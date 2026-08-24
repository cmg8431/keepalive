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
}

#[derive(Serialize)]
pub struct ManagedSessionInfo {
    pub name: String,
    pub dir: String,
    pub cmd: String,
    pub status: SessionStatus,
    pub respawn_count: u32,
}

#[derive(Default)]
pub struct SessionManager {
    sessions: HashMap<String, ManagedSession>,
    counter: u32,
}

fn tmux(args: &[&str]) -> Result<std::process::Output> {
    Command::new("tmux")
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
        self.sessions.insert(
            name.clone(),
            ManagedSession {
                dir,
                cmd,
                status: SessionStatus::Running,
                respawn_count: 0,
                spawned_at: Instant::now(),
            },
        );
        Ok(name)
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

    /// Returns log lines describing what changed this tick.
    pub fn monitor(&mut self, now: Instant) -> Vec<String> {
        let mut events = Vec::new();
        for (name, session) in &mut self.sessions {
            if session.status != SessionStatus::Running {
                continue;
            }
            if tmux_alive(name) {
                if session.respawn_count > 0 && now - session.spawned_at >= RESPAWN_RESET {
                    session.respawn_count = 0;
                }
                continue;
            }
            if session.respawn_count >= MAX_RESPAWNS {
                session.status = SessionStatus::Abandoned;
                events.push(format!(
                    "session {name} died {MAX_RESPAWNS} times — abandoned (kill it to clean up)"
                ));
                continue;
            }
            let cmd = revival_cmd(&session.cmd);
            match tmux_spawn(name, &session.dir, &cmd) {
                Ok(()) => {
                    session.respawn_count += 1;
                    session.spawned_at = now;
                    events.push(format!(
                        "session {name} died — revived with `{cmd}` (attempt {}/{MAX_RESPAWNS})",
                        session.respawn_count
                    ));
                }
                Err(e) => {
                    session.status = SessionStatus::Abandoned;
                    events.push(format!("session {name} revival failed: {e:#}"));
                }
            }
        }
        events
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
            })
            .collect()
    }
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
}
