use crate::clamshell;
use crate::heartbeat::Heartbeat;
use crate::notify;
use crate::power::{self, SmcReader, WakeAssertion};
use crate::sessions::{ManagedSessionInfo, SessionManager};
use anyhow::{Context, Result};
use keepalive_core::config::{Config, socket_path};
use keepalive_core::policy::{self, CutoutKind, CutoutLatch, Decision, PolicyInput, SleepReason};
use keepalive_core::session::SessionTable;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub(crate) enum Request {
    Acquire {
        id: String,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        ttl_secs: Option<u64>,
        #[serde(default)]
        label: Option<String>,
    },
    Release {
        id: String,
    },
    Hold {
        #[serde(default)]
        minutes: Option<u64>,
    },
    Clear,
    Status,
    Shutdown,
    Run {
        dir: String,
        #[serde(default)]
        command: Option<String>,
        #[serde(default)]
        name: Option<String>,
    },
    Sessions,
    Kill {
        name: String,
    },
}

#[derive(Serialize)]
struct SessionInfo {
    id: String,
    source: String,
    label: Option<String>,
    active_secs: u64,
    expires_in_secs: u64,
}

#[derive(Serialize)]
struct StatusResponse {
    ok: bool,
    awake: bool,
    sessions: Vec<SessionInfo>,
    battery_percent: Option<u8>,
    on_ac_power: bool,
    temperature_celsius: Option<f64>,
    lid_closed: bool,
    cutout_latched: bool,
    clamshell_active: bool,
    managed: Vec<ManagedSessionInfo>,
}

pub(crate) struct Daemon {
    config: Config,
    table: SessionTable,
    assertion: Option<WakeAssertion>,
    held_since: Option<Instant>,
    smc: Option<SmcReader>,
    latch: CutoutLatch,
    clamshell_ok: bool,
    clamshell_active: bool,
    clamshell_pushed_at: Option<Instant>,
    managed: SessionManager,
    heartbeat: Option<Heartbeat>,
    last_tick: Option<Instant>,
}

const CLAMSHELL_REPUSH: Duration = Duration::from_secs(60);

impl Daemon {
    fn new(config: Config) -> Self {
        let smc = SmcReader::new();
        if smc.is_none() {
            log("SMC unavailable — thermal cutout disabled");
        }
        let passwordless = clamshell::passwordless_available();
        let clamshell_ok = config.clamshell && passwordless;
        if config.clamshell && !clamshell_ok {
            log(
                "clamshell requested but passwordless pmset is not configured — run: sudo keepalive clamshell-setup (then restart the daemon)",
            );
        }
        let heartbeat = if config.heartbeat_minutes > 0 && !passwordless {
            log(
                "heartbeat wake requested but pmset scheduling needs the sudoers rule — run: sudo keepalive clamshell-setup",
            );
            None
        } else {
            Heartbeat::new(config.heartbeat_minutes, &config.ntfy_topic)
        };
        Self {
            config,
            table: SessionTable::default(),
            assertion: None,
            held_since: None,
            smc,
            latch: CutoutLatch::default(),
            clamshell_ok,
            clamshell_active: false,
            clamshell_pushed_at: None,
            managed: SessionManager::default(),
            heartbeat,
            last_tick: None,
        }
    }

    /// Re-pushed periodically while blocking: heals a failed pmset or a
    /// kernel reset across sleep/wake, at the cost of one fork per minute
    /// only while an agent is held.
    fn clamshell_sync(&mut self, blocked: bool, now: Instant) {
        if !self.clamshell_ok {
            return;
        }
        let due = self
            .clamshell_pushed_at
            .is_none_or(|t| now - t >= CLAMSHELL_REPUSH);
        if blocked && (!self.clamshell_active || due) {
            match clamshell::set_blocked(true) {
                Ok(()) => {
                    if !self.clamshell_active {
                        log("clamshell sleep disabled (lid-close safe)");
                    }
                    self.clamshell_active = true;
                    self.clamshell_pushed_at = Some(now);
                }
                Err(e) => log(&format!("clamshell enable failed: {e:#}")),
            }
        } else if !blocked && self.clamshell_active {
            match clamshell::set_blocked(false) {
                Ok(()) => log("clamshell sleep restored"),
                Err(e) => log(&format!("clamshell disable failed: {e:#}")),
            }
            self.clamshell_active = false;
            self.clamshell_pushed_at = None;
        }
    }

    fn policy_input(&self, now: Instant) -> PolicyInput {
        let status = power::read_power_status();
        let lid_closed = power::lid_closed();
        // Temperature is only read while it can matter (holding or latched):
        // no SMC traffic from an idle daemon.
        let temperature_celsius = if self.assertion.is_some() || self.latch.is_latched() {
            self.smc.as_ref().and_then(SmcReader::temperature)
        } else {
            None
        };
        PolicyInput {
            active_sessions: self.table.active_count(),
            battery_percent: status.battery_percent,
            on_ac_power: status.on_ac_power,
            held_for: self.held_since.map_or(Duration::ZERO, |t| now - t),
            temperature_celsius,
            lid_closed,
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        // A large gap between ticks means the machine slept: check whether
        // the phone asked for a wake while we were gone.
        let gap_threshold = Duration::from_secs(self.config.poll_secs.max(1) * 3 + 30);
        let woke_from_sleep = self
            .last_tick
            .is_some_and(|t| now.saturating_duration_since(t) > gap_threshold);
        self.last_tick = Some(now);
        if woke_from_sleep && let Some(hb) = &self.heartbeat {
            log("woke from sleep — polling wake mailbox");
            if hb.wake_requested() {
                self.table.acquire(
                    "remote-wake",
                    "phone",
                    Some("remote wake"),
                    Duration::from_secs(1800),
                    now,
                );
                log("remote wake requested — holding 30 minutes");
                notify::push(
                    &self.config.ntfy_topic,
                    "keepalive",
                    "Mac is awake and holding for 30 minutes",
                );
            }
        }
        for event in self.managed.monitor(now) {
            log(&event);
            notify::push(&self.config.ntfy_topic, "keepalive session", &event);
        }
        // Live managed sessions renew their wake holds every tick; the TTL
        // is a backstop for daemon stalls, not the liveness signal.
        let managed_ttl = Duration::from_secs((self.config.poll_secs * 4).max(60));
        for name in self.managed.running() {
            self.table.acquire(
                &format!("managed:{name}"),
                "managed",
                Some(&name),
                managed_ttl,
                now,
            );
        }
        let pruned = self.table.prune_expired(now);
        if pruned > 0 {
            log(&format!("pruned {pruned} expired session(s)"));
        }
        let input = self.policy_input(now);
        if self.latch.is_latched() {
            if self.latch.try_clear(&self.config, &input) {
                log("safety cutout latch cleared — holds accepted again");
            } else {
                self.release_assertion("cutout latched");
                return;
            }
        }
        match policy::evaluate(&self.config, &input) {
            Decision::StayAwake => {
                if self.assertion.is_none() {
                    self.assertion = WakeAssertion::new("keepalive: agent session active");
                    self.held_since = Some(now);
                    log("wake assertion acquired");
                }
                self.clamshell_sync(true, now);
            }
            Decision::AllowSleep(reason) => {
                let was_awake = self.assertion.is_some();
                self.release_assertion(&format!("{reason:?}"));
                // Safety guards evict the sessions that caused the hold and
                // latch until conditions recover past hysteresis, so a
                // tripped guard can't re-arm on the next hook ping.
                let topic = self.config.ntfy_topic.clone();
                match reason {
                    SleepReason::NoActiveSessions => {
                        if was_awake {
                            notify::push(
                                &topic,
                                "keepalive",
                                "Work finished — letting the Mac sleep",
                            );
                        }
                    }
                    SleepReason::ThermalCutout(t) => {
                        self.latch.trip(CutoutKind::Thermal);
                        self.table.clear();
                        notify::push(
                            &topic,
                            "keepalive safety cutout",
                            &format!("Thermal cutout at {t:.0}C (lid closed) — forcing sleep"),
                        );
                    }
                    SleepReason::BatteryBelowFloor(p) => {
                        self.latch.trip(CutoutKind::LowBattery);
                        self.table.clear();
                        notify::push(
                            &topic,
                            "keepalive safety cutout",
                            &format!("Battery at {p}% — forcing sleep"),
                        );
                    }
                    SleepReason::MaxHoldExceeded => {
                        self.table.clear();
                        if was_awake {
                            notify::push(
                                &topic,
                                "keepalive",
                                "Max hold duration reached — letting the Mac sleep",
                            );
                        }
                    }
                }
                // About to let the machine sleep: make sure a heartbeat wake
                // is on the calendar so the phone can still reach us.
                if let Some(hb) = &mut self.heartbeat
                    && let Err(e) = hb.ensure_scheduled(now)
                {
                    log(&format!("heartbeat scheduling failed, disabling: {e:#}"));
                    self.heartbeat = None;
                }
            }
        }
    }

    pub(crate) fn config(&self) -> &Config {
        &self.config
    }

    /// Live-apply a changed config (setup wizard): notification topic and
    /// heartbeat take effect without a daemon restart.
    pub(crate) fn reload_config(&mut self, config: Config) {
        let passwordless = clamshell::passwordless_available();
        self.clamshell_ok = config.clamshell && passwordless;
        self.heartbeat = if config.heartbeat_minutes > 0 && !passwordless {
            None
        } else {
            Heartbeat::new(config.heartbeat_minutes, &config.ntfy_topic)
        };
        self.config = config;
        log("config reloaded");
    }

    fn release_assertion(&mut self, reason: &str) {
        if self.assertion.take().is_some() {
            self.held_since = None;
            log(&format!("wake assertion released ({reason})"));
        }
        self.clamshell_sync(false, Instant::now());
    }

    pub(crate) fn handle(&mut self, req: Request) -> serde_json::Value {
        let now = Instant::now();
        let response = match req {
            Request::Acquire {
                id,
                source,
                ttl_secs,
                label,
            } => {
                let ttl = ttl_secs.map_or(self.config.default_ttl(), Duration::from_secs);
                let source = source.unwrap_or_else(|| "unknown".to_string());
                self.table.acquire(&id, &source, label.as_deref(), ttl, now);
                serde_json::json!({ "ok": true })
            }
            Request::Release { id } => {
                let released = self.table.release(&id);
                serde_json::json!({ "ok": true, "released": released })
            }
            Request::Hold { minutes } => {
                let ttl = Duration::from_secs(minutes.unwrap_or(60) * 60);
                self.table
                    .acquire("manual", "manual", Some("manual hold"), ttl, now);
                serde_json::json!({ "ok": true })
            }
            Request::Clear => {
                self.table.clear();
                serde_json::json!({ "ok": true })
            }
            Request::Shutdown => serde_json::json!({ "ok": true }),
            Request::Run { dir, command, name } => {
                let cmd = command.unwrap_or_else(|| "claude".to_string());
                match self.managed.spawn(std::path::PathBuf::from(dir), cmd, name) {
                    Ok(name) => serde_json::json!({ "ok": true, "name": name }),
                    Err(e) => serde_json::json!({ "ok": false, "error": format!("{e:#}") }),
                }
            }
            Request::Sessions => {
                return serde_json::json!({ "ok": true, "managed": self.managed.list() });
            }
            Request::Kill { name } => match self.managed.kill(&name) {
                Ok(found) => {
                    self.table.release(&format!("managed:{name}"));
                    serde_json::json!({ "ok": true, "found": found })
                }
                Err(e) => serde_json::json!({ "ok": false, "error": format!("{e:#}") }),
            },
            Request::Status => {
                let status = power::read_power_status();
                let sessions = self
                    .table
                    .iter()
                    .map(|(id, s)| SessionInfo {
                        id: id.clone(),
                        source: s.source.clone(),
                        label: s.label.clone(),
                        active_secs: now.saturating_duration_since(s.acquired_at).as_secs(),
                        expires_in_secs: s.expires_at.saturating_duration_since(now).as_secs(),
                    })
                    .collect();
                return serde_json::to_value(StatusResponse {
                    ok: true,
                    awake: self.assertion.is_some(),
                    sessions,
                    battery_percent: status.battery_percent,
                    on_ac_power: status.on_ac_power,
                    temperature_celsius: self.smc.as_ref().and_then(SmcReader::temperature),
                    lid_closed: power::lid_closed(),
                    cutout_latched: self.latch.is_latched(),
                    clamshell_active: self.clamshell_active,
                    managed: self.managed.list(),
                })
                .unwrap_or_else(|_| serde_json::json!({ "ok": false }));
            }
        };
        self.tick();
        response
    }
}

fn log(msg: &str) {
    println!("[keepalived] {msg}");
}

pub async fn serve() -> Result<()> {
    let config = Config::load();
    clamshell::reconcile_on_startup();
    let path = socket_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).context("creating socket directory")?;
    }
    if std::os::unix::net::UnixStream::connect(&path).is_ok() {
        anyhow::bail!("daemon already running at {}", path.display());
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).context("binding daemon socket")?;
    log(&format!("listening on {}", path.display()));

    let daemon = Arc::new(Mutex::new(Daemon::new(config.clone())));
    tokio::spawn(crate::http::serve_http(Arc::clone(&daemon), config.clone()));
    let mut poll = tokio::time::interval(Duration::from_secs(config.poll_secs.max(1)));
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("registering SIGTERM handler")?;

    loop {
        tokio::select! {
            _ = poll.tick() => {
                daemon.lock().unwrap().tick();
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("accepting connection")?;
                let daemon = Arc::clone(&daemon);
                tokio::spawn(async move {
                    let _ = handle_connection(stream, daemon).await;
                });
            }
            _ = tokio::signal::ctrl_c() => {
                shutdown(&daemon, &path);
                return Ok(());
            }
            _ = sigterm.recv() => {
                shutdown(&daemon, &path);
                return Ok(());
            }
        }
    }
}

/// Every exit path must restore normal sleep: the IOPM assertion dies with
/// the process, but disablesleep would persist.
fn shutdown(daemon: &Arc<Mutex<Daemon>>, path: &std::path::Path) {
    log("shutting down");
    daemon.lock().unwrap().release_assertion("shutdown");
    let _ = std::fs::remove_file(path);
}

async fn handle_connection(stream: UnixStream, daemon: Arc<Mutex<Daemon>>) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut line = String::new();
    BufReader::new(read_half).read_line(&mut line).await?;
    let parsed = serde_json::from_str::<Request>(&line);
    let shutdown = matches!(parsed, Ok(Request::Shutdown));
    let response = match parsed {
        Ok(req) => daemon.lock().unwrap().handle(req),
        Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }),
    };
    write_half
        .write_all(format!("{response}\n").as_bytes())
        .await?;
    if shutdown {
        log("shutdown requested");
        daemon.lock().unwrap().release_assertion("shutdown request");
        let _ = std::fs::remove_file(socket_path());
        std::process::exit(0);
    }
    Ok(())
}
