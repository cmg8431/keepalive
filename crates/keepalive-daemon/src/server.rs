use crate::power::{self, WakeAssertion};
use anyhow::{Context, Result};
use keepalive_core::config::{Config, socket_path};
use keepalive_core::policy::{self, Decision, PolicyInput};
use keepalive_core::session::SessionTable;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request {
    Acquire {
        id: String,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        ttl_secs: Option<u64>,
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
}

#[derive(Serialize)]
struct SessionInfo {
    id: String,
    source: String,
    expires_in_secs: u64,
}

#[derive(Serialize)]
struct StatusResponse {
    ok: bool,
    awake: bool,
    sessions: Vec<SessionInfo>,
    battery_percent: Option<u8>,
    on_ac_power: bool,
}

struct Daemon {
    config: Config,
    table: SessionTable,
    assertion: Option<WakeAssertion>,
    held_since: Option<Instant>,
}

impl Daemon {
    fn new(config: Config) -> Self {
        Self {
            config,
            table: SessionTable::default(),
            assertion: None,
            held_since: None,
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let pruned = self.table.prune_expired(now);
        if pruned > 0 {
            log(&format!("pruned {pruned} expired session(s)"));
        }
        let status = power::read_power_status();
        let input = PolicyInput {
            active_sessions: self.table.active_count(),
            battery_percent: status.battery_percent,
            on_ac_power: status.on_ac_power,
            held_for: self.held_since.map_or(Duration::ZERO, |t| now - t),
        };
        match policy::evaluate(&self.config, &input) {
            Decision::StayAwake => {
                if self.assertion.is_none() {
                    self.assertion = WakeAssertion::new("keepalive: agent session active");
                    self.held_since = Some(now);
                    log("wake assertion acquired");
                }
            }
            Decision::AllowSleep(reason) => {
                if self.assertion.take().is_some() {
                    self.held_since = None;
                    log(&format!("wake assertion released ({reason:?})"));
                }
                // Safety guards also evict the sessions that caused the hold,
                // so a tripped guard can't re-arm on the next tick.
                if !matches!(
                    reason,
                    keepalive_core::policy::SleepReason::NoActiveSessions
                ) {
                    self.table.clear();
                }
            }
        }
    }

    fn handle(&mut self, req: Request) -> serde_json::Value {
        let now = Instant::now();
        let response = match req {
            Request::Acquire {
                id,
                source,
                ttl_secs,
            } => {
                let ttl = ttl_secs.map_or(self.config.default_ttl(), Duration::from_secs);
                let source = source.unwrap_or_else(|| "unknown".to_string());
                self.table.acquire(&id, &source, ttl, now);
                serde_json::json!({ "ok": true })
            }
            Request::Release { id } => {
                let released = self.table.release(&id);
                serde_json::json!({ "ok": true, "released": released })
            }
            Request::Hold { minutes } => {
                let ttl = Duration::from_secs(minutes.unwrap_or(60) * 60);
                self.table.acquire("manual", "manual", ttl, now);
                serde_json::json!({ "ok": true })
            }
            Request::Clear => {
                self.table.clear();
                serde_json::json!({ "ok": true })
            }
            Request::Status => {
                let status = power::read_power_status();
                let sessions = self
                    .table
                    .iter()
                    .map(|(id, s)| SessionInfo {
                        id: id.clone(),
                        source: s.source.clone(),
                        expires_in_secs: s.expires_at.saturating_duration_since(now).as_secs(),
                    })
                    .collect();
                return serde_json::to_value(StatusResponse {
                    ok: true,
                    awake: self.assertion.is_some(),
                    sessions,
                    battery_percent: status.battery_percent,
                    on_ac_power: status.on_ac_power,
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
    let path = socket_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).context("creating socket directory")?;
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).context("binding daemon socket")?;
    log(&format!("listening on {}", path.display()));

    let daemon = Arc::new(Mutex::new(Daemon::new(config.clone())));
    let mut poll = tokio::time::interval(Duration::from_secs(config.poll_secs.max(1)));

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
                log("shutting down");
                let _ = std::fs::remove_file(&path);
                return Ok(());
            }
        }
    }
}

async fn handle_connection(stream: UnixStream, daemon: Arc<Mutex<Daemon>>) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut line = String::new();
    BufReader::new(read_half).read_line(&mut line).await?;
    let response = match serde_json::from_str::<Request>(&line) {
        Ok(req) => daemon.lock().unwrap().handle(req),
        Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }),
    };
    write_half
        .write_all(format!("{response}\n").as_bytes())
        .await?;
    Ok(())
}
