//! Managed agent sessions: spawned detached inside tmux so a dropped remote
//! connection never kills them, monitored each tick, and revived with
//! `claude --continue` (conversation context restored) if the process dies.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

const MAX_RESPAWNS: u32 = 3;
/// A session alive this long after a respawn is considered healthy again.
const RESPAWN_RESET: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    Abandoned,
}

#[derive(Debug)]
pub struct ManagedSession {
    pub dir: PathBuf,
    pub cmd: String,
    pub status: SessionStatus,
    pub respawn_count: u32,
    pub spawned_at: Instant,
    attention: AttentionState,
}

impl ManagedSession {
    fn new(dir: PathBuf, cmd: String) -> Self {
        Self {
            dir,
            cmd,
            status: SessionStatus::Running,
            respawn_count: 0,
            spawned_at: Instant::now(),
            attention: AttentionState::default(),
        }
    }
}

/// Tracks whether the agent inside a session looks parked on a question.
/// The screen not changing across ticks while the agent's "working" marker is
/// absent is the signal; one notification per pause, reset when work resumes.
#[derive(Debug, Default)]
struct AttentionState {
    pane_hash: u64,
    stable_ticks: u32,
    waiting: bool,
    notified: bool,
    /// The agent has visibly worked at least once. A session idling at its
    /// welcome prompt is "waiting" on the dashboard but not push-worthy —
    /// the person who just spawned it knows it's there.
    was_busy: bool,
    /// When the pane last changed — the "active 2m ago" indicator.
    last_change: Option<Instant>,
}

#[derive(Serialize)]
pub struct ManagedSessionInfo {
    pub name: String,
    pub dir: String,
    pub cmd: String,
    pub status: SessionStatus,
    pub respawn_count: u32,
    pub waiting: bool,
    /// Seconds since the terminal last changed; None until first observed.
    pub idle_secs: Option<u64>,
}

#[derive(Default)]
pub struct SessionManager {
    sessions: HashMap<String, ManagedSession>,
    counter: u32,
}

/// launchd starts the daemon with a minimal PATH, so Homebrew's tmux has to
/// be found by absolute path rather than lookup.
pub fn tmux_bin() -> &'static str {
    [
        "/opt/homebrew/bin/tmux",
        "/usr/local/bin/tmux",
        "/usr/bin/tmux",
    ]
    .into_iter()
    .find(|p| std::path::Path::new(p).exists())
    .unwrap_or("tmux")
}

fn tmux(args: &[&str]) -> Result<std::process::Output> {
    Command::new(tmux_bin())
        .args(args)
        .output()
        .context("running tmux — is it installed?")
}

fn tmux_alive(name: &str) -> bool {
    let target = format!("={name}");
    tmux(&["has-session", "-t", &target]).is_ok_and(|o| o.status.success())
}

fn tmux_spawn(name: &str, dir: &std::path::Path, cmd: &str) -> Result<()> {
    // Login shell so agents installed via nvm/homebrew resolve despite
    // launchd's minimal PATH.
    let wrapped = format!("exec /bin/zsh -lc {}", shell_quote(cmd));
    let dir_str = dir.to_string_lossy();
    let out = tmux(&["new-session", "-d", "-s", name, "-c", &dir_str, &wrapped])?;
    if !out.status.success() {
        bail!(
            "tmux new-session failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Raw pane text for the attention heuristic; None if tmux can't answer.
fn pane_text(name: &str) -> Option<String> {
    let target = format!("={name}:");
    let out = tmux(&["capture-pane", "-p", "-t", &target]).ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn tmux_attached(name: &str) -> bool {
    let target = format!("={name}:");
    tmux(&[
        "display-message",
        "-p",
        "-t",
        &target,
        "#{session_attached}",
    ])
    .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() != "0")
}

/// Waiting detection only makes sense for interactive agents that pause on
/// questions; a build script sitting quiet is just a quiet build script.
fn is_interactive_agent(cmd: &str) -> bool {
    let base = cmd.split_whitespace().next().unwrap_or("");
    let base = base.rsplit('/').next().unwrap_or(base);
    matches!(base, "claude" | "codex" | "gemini" | "aider" | "amp" | "opencode")
}

/// Claude Code (and codex) print this while a turn is running; its absence
/// plus a frozen screen is the strongest "parked on a question" signal
/// available from outside the process.
fn looks_busy(pane: &str) -> bool {
    pane.contains("esc to interrupt") || pane.contains("Esc to interrupt")
}

fn hash_pane(pane: &str) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    pane.hash(&mut h);
    h.finish()
}

/// The revival command restores conversation context for claude-family
/// commands; anything else is restarted as-is.
fn revival_cmd(cmd: &str) -> String {
    let base = cmd.split_whitespace().next().unwrap_or("");
    if base.ends_with("claude") && !cmd.contains("--continue") && !cmd.contains("--resume") {
        format!("{cmd} --continue")
    } else {
        cmd.to_string()
    }
}

impl SessionManager {
    /// Re-adopt `ka-*` tmux sessions that outlived the daemon.
    ///
    /// The manager only lives in memory, so a daemon restart used to orphan
    /// every running agent: tmux kept them alive but the dashboard could no
    /// longer show or open them. Reconciling at startup keeps the promise that
    /// these sessions survive anything short of the machine going down.
    pub fn adopt_existing(&mut self) -> usize {
        let Ok(out) = tmux(&[
            "list-sessions",
            "-F",
            "#{session_name}|#{pane_current_path}",
        ]) else {
            return 0;
        };
        if !out.status.success() {
            return 0;
        }
        let mut adopted = 0;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let (name, dir) = line.split_once('|').unwrap_or((line, ""));
            let Some(suffix) = name.strip_prefix("ka-") else {
                continue;
            };
            if self.sessions.contains_key(name) {
                continue;
            }
            if let Ok(n) = suffix.parse::<u32>() {
                self.counter = self.counter.max(n);
            }
            self.sessions.insert(
                name.to_string(),
                // The original command is gone with the old daemon; revival
                // uses the same default the spawn path does.
                ManagedSession::new(
                    PathBuf::from(if dir.is_empty() { "/" } else { dir }),
                    "claude".to_string(),
                ),
            );
            adopted += 1;
        }
        adopted
    }

    pub fn spawn(&mut self, dir: PathBuf, cmd: String, name: Option<String>) -> Result<String> {
        if !dir.is_dir() {
            bail!("not a directory: {}", dir.display());
        }
        let name = match name {
            Some(n) if !n.is_empty() => format!("ka-{n}"),
            _ => {
                self.counter += 1;
                format!("ka-{}", self.counter)
            }
        };
        if self.sessions.contains_key(&name) || tmux_alive(&name) {
            bail!("session {name} already exists");
        }
        tmux_spawn(&name, &dir, &cmd)?;
        self.sessions
            .insert(name.clone(), ManagedSession::new(dir, cmd));
        Ok(name)
    }

    /// Last visible lines of a managed session's pane, for a remote glance at
    /// what the agent is doing. Only sessions this manager spawned are
    /// readable — arbitrary tmux targets stay out of reach of the dashboard.
    pub fn tail(&self, name: &str) -> Result<String> {
        self.assert_known(name)?;
        // capture-pane takes a pane target: "=name:" is the exact-matched
        // session's active window/pane (bare "=name" is not a valid pane ref).
        let target = format!("={name}:");
        let out = tmux(&["capture-pane", "-p", "-t", &target])?;
        if !out.status.success() {
            bail!(
                "tmux capture-pane failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut lines: Vec<&str> = text.lines().collect();
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        let start = lines.len().saturating_sub(25);
        Ok(lines[start..].join("\n"))
    }

    /// Every remote-control entry point funnels through this: the dashboard
    /// may only ever address sessions this manager spawned, never an arbitrary
    /// tmux target that happens to exist on the machine.
    fn assert_known(&self, name: &str) -> Result<()> {
        if !self.sessions.contains_key(name) {
            bail!("unknown session: {name}");
        }
        Ok(())
    }

    /// Type into a running agent from the phone. `text` is sent literally
    /// (`-l`), so a payload can never be reinterpreted as a tmux key name;
    /// named keys go through [`Self::send_key`] instead.
    pub fn send_text(&self, name: &str, text: &str) -> Result<()> {
        self.assert_known(name)?;
        let target = format!("={name}:");
        let out = tmux(&["send-keys", "-t", &target, "-l", "--", text])?;
        if !out.status.success() {
            bail!(
                "tmux send-keys failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    /// Named keys the phone keyboard cannot produce (Enter, Escape, arrows,
    /// C-c). Restricted to a fixed vocabulary so the dashboard can never pass
    /// through an arbitrary tmux key spec.
    pub fn send_key(&self, name: &str, key: &str) -> Result<()> {
        self.assert_known(name)?;
        const ALLOWED: &[&str] = &[
            "Enter", "Escape", "Tab", "Space", "BSpace", "Up", "Down", "Left", "Right", "C-c",
            "C-d", "C-r", "C-u", "C-l",
        ];
        if !ALLOWED.contains(&key) {
            bail!("unsupported key: {key}");
        }
        let target = format!("={name}:");
        let out = tmux(&["send-keys", "-t", &target, key])?;
        if !out.status.success() {
            bail!(
                "tmux send-keys failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    /// One frame of a session's screen: the rendered pane plus where the
    /// cursor sits, which is everything the browser needs to redraw it.
    ///
    /// This is deliberately *not* a tmux client. Attaching a real client
    /// (`tmux attach` on a PTY, even into a grouped session) was measured to
    /// take the entire tmux server down with it when the client goes away —
    /// killing every agent session on the machine, which is the exact opposite
    /// of what this daemon exists to do. Polling the pane can't: `capture-pane`
    /// and `send-keys` are read/write operations on a session, never a client.
    pub fn snapshot(&self, name: &str) -> Result<Snapshot> {
        self.assert_known(name)?;
        let target = format!("={name}:");
        // -e keeps colour/attribute escapes. No -J: joining wrapped rows makes
        // lines longer than the pane, and the viewer then wraps them somewhere
        // else — the screen has to arrive exactly as tmux drew it, row by row.
        let out = tmux(&["capture-pane", "-p", "-e", "-t", &target])?;
        if !out.status.success() {
            bail!(
                "tmux capture-pane failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        // capture-pane separates rows with a bare LF. A terminal treats that as
        // "down one row, keep the column", so every row would start where the
        // previous one ended — the staircase. Rows need an explicit CR.
        let screen = String::from_utf8_lossy(&out.stdout)
            .lines()
            .collect::<Vec<_>>()
            .join("\r\n");
        let meta = tmux(&[
            "display-message",
            "-p",
            "-t",
            &target,
            "#{cursor_x},#{cursor_y},#{pane_width},#{pane_height}",
        ])?;
        let meta = String::from_utf8_lossy(&meta.stdout);
        let mut parts = meta.trim().split(',').map(|v| v.parse().unwrap_or(0));
        Ok(Snapshot {
            screen,
            cursor_x: parts.next().unwrap_or(0),
            cursor_y: parts.next().unwrap_or(0),
            cols: parts.next().unwrap_or(80).max(1),
            rows: parts.next().unwrap_or(24).max(1),
        })
    }

    /// Forwards raw bytes from the browser's keyboard, hex-encoded so arrow
    /// keys, control characters and UTF-8 all survive the trip intact.
    /// Match the tmux window to the viewer's terminal. Without this the pane
    /// keeps whatever geometry it was born with and the browser renders a
    /// differently-wrapped copy of the same screen — the "broken" look.
    pub fn resize(&self, name: &str, cols: u16, rows: u16) -> Result<()> {
        self.assert_known(name)?;
        // A viewer must never reshape a window someone is sitting in front of:
        // resizing an attached session changes what the person at the real
        // terminal sees. Detached sessions have no such owner, so those we size.
        if self.attached(name) {
            return Ok(());
        }
        let cols = cols.clamp(20, 500);
        let rows = rows.clamp(5, 200);
        let target = format!("={name}:");
        let out = tmux(&[
            "resize-window",
            "-t",
            &target,
            "-x",
            &cols.to_string(),
            "-y",
            &rows.to_string(),
        ])?;
        if !out.status.success() {
            bail!(
                "tmux resize-window failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    /// Whether a real tmux client is looking at this session right now.
    fn attached(&self, name: &str) -> bool {
        tmux_attached(name)
    }

    pub fn send_raw(&self, name: &str, bytes: &[u8]) -> Result<()> {
        self.assert_known(name)?;
        if bytes.is_empty() {
            return Ok(());
        }
        let target = format!("={name}:");
        let mut args = vec!["send-keys".to_string(), "-H".to_string()];
        args.push("-t".to_string());
        args.push(target);
        args.extend(bytes.iter().map(|b| format!("{b:02x}")));
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = tmux(&borrowed)?;
        if !out.status.success() {
            bail!(
                "tmux send-keys failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    pub fn kill(&mut self, name: &str) -> Result<bool> {
        let existed = self.sessions.remove(name).is_some();
        if tmux_alive(name) {
            let target = format!("={name}");
            tmux(&["kill-session", "-t", &target])?;
            return Ok(true);
        }
        Ok(existed)
    }

    /// Returns `(session name, log line)` for everything that changed this
    /// tick. The name travels with the message so a notification can link
    /// straight to the session it is about.
    pub fn monitor(&mut self, now: Instant) -> Vec<(String, String)> {
        let mut events = Vec::new();
        for (name, session) in &mut self.sessions {
            if session.status != SessionStatus::Running {
                continue;
            }
            if tmux_alive(name) {
                if session.respawn_count > 0 && now - session.spawned_at >= RESPAWN_RESET {
                    session.respawn_count = 0;
                }
                if let Some(event) = watch_attention(name, session) {
                    events.push(event);
                }
                continue;
            }
            if session.respawn_count >= MAX_RESPAWNS {
                session.status = SessionStatus::Abandoned;
                events.push((
                    name.clone(),
                    format!(
                        "session {name} died {MAX_RESPAWNS} times — abandoned (kill it to clean up)"
                    ),
                ));
                continue;
            }
            let cmd = revival_cmd(&session.cmd);
            match tmux_spawn(name, &session.dir, &cmd) {
                Ok(()) => {
                    session.respawn_count += 1;
                    session.spawned_at = now;
                    events.push((
                        name.clone(),
                        format!(
                            "session {name} died — revived with `{cmd}` (attempt {}/{MAX_RESPAWNS})",
                            session.respawn_count
                        ),
                    ));
                }
                Err(e) => {
                    session.status = SessionStatus::Abandoned;
                    events.push((
                        name.clone(),
                        format!("session {name} revival failed: {e:#}"),
                    ));
                }
            }
        }
        events
    }

    /// The managed session working in `dir`, if any — how a hook event
    /// (which only knows its cwd) finds its way back to a tmux session.
    pub fn find_by_dir(&self, dir: &str) -> Option<String> {
        let dir = std::path::Path::new(dir);
        self.sessions
            .iter()
            .filter(|(_, s)| s.status == SessionStatus::Running && dir.starts_with(&s.dir))
            // Deepest match wins when projects nest.
            .max_by_key(|(_, s)| s.dir.components().count())
            .map(|(name, _)| name.clone())
    }

    /// Mark a session as waiting right now — a hook told us, no heuristic
    /// needed. The pane watcher clears it as soon as the agent moves again.
    pub fn flag_waiting(&mut self, name: &str) {
        if let Some(s) = self.sessions.get_mut(name) {
            s.attention.waiting = true;
            s.attention.notified = true;
        }
    }

    /// Names of sessions currently alive — these renew wake holds each tick.
    pub fn running(&self) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(name, s)| s.status == SessionStatus::Running && tmux_alive(name))
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub fn list(&self) -> Vec<ManagedSessionInfo> {
        self.sessions
            .iter()
            .map(|(name, s)| ManagedSessionInfo {
                name: name.clone(),
                dir: s.dir.to_string_lossy().into_owned(),
                cmd: s.cmd.clone(),
                status: s.status,
                respawn_count: s.respawn_count,
                waiting: s.attention.waiting,
                idle_secs: s.attention.last_change.map(|t| t.elapsed().as_secs()),
            })
            .collect()
    }
}

/// One attention check for one live session. Returns a notification-worthy
/// event exactly once per pause: two consecutive frozen ticks (~30s) with no
/// busy marker. Someone attached in a local terminal is already looking, so
/// no push for them.
fn watch_attention(name: &str, session: &mut ManagedSession) -> Option<(String, String)> {
    let att = &mut session.attention;
    let pane = pane_text(name)?;
    let hash = hash_pane(&pane);
    let changed = hash != att.pane_hash;
    att.pane_hash = hash;
    if changed {
        att.last_change = Some(Instant::now());
    }
    // Every session gets the activity clock; only interactive agents get the
    // waiting heuristic — a quiet build script is just a quiet build script.
    if !is_interactive_agent(&session.cmd) {
        return None;
    }
    if changed || looks_busy(&pane) {
        att.was_busy = att.was_busy || looks_busy(&pane);
        att.stable_ticks = 0;
        att.waiting = false;
        att.notified = false;
        return None;
    }
    att.stable_ticks += 1;
    if att.stable_ticks < 2 || att.notified {
        return None;
    }
    att.waiting = true;
    att.notified = true;
    if !att.was_busy || tmux_attached(name) {
        return None;
    }
    Some((
        name.to_string(),
        format!("session {name} is waiting for your input"),
    ))
}

#[derive(Serialize, PartialEq, Eq, Clone)]
pub struct Snapshot {
    pub screen: String,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub cols: u16,
    pub rows: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revival_appends_continue_for_claude() {
        assert_eq!(revival_cmd("claude"), "claude --continue");
        assert_eq!(
            revival_cmd("claude --model opus"),
            "claude --model opus --continue"
        );
        assert_eq!(revival_cmd("claude --continue"), "claude --continue");
        assert_eq!(revival_cmd("codex"), "codex");
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn attention_only_watches_interactive_agents() {
        assert!(is_interactive_agent("claude"));
        assert!(is_interactive_agent("/usr/local/bin/claude --model opus"));
        assert!(is_interactive_agent("codex"));
        assert!(!is_interactive_agent("npm run build"));
        assert!(!is_interactive_agent("sleep 999"));
    }

    #[test]
    fn busy_marker_detected() {
        assert!(looks_busy("Cogitating… (esc to interrupt)"));
        assert!(!looks_busy("❯ waiting at the prompt"));
    }
}
