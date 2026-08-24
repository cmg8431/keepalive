//! Heartbeat wake: a sleeping Mac vanishes from the network, so before sleep
//! is allowed the daemon schedules a periodic RTC wake (`pmset schedule
//! wake`, via the same sudoers rule as clamshell). Each brief wake, the tick
//! loop notices the time gap and polls the ntfy topic for a pending "wake"
//! message from the phone; if present, a real hold is taken and managed
//! sessions come back. Otherwise the Mac just drifts back to sleep.

use anyhow::{Context, Result, bail};
use std::process::Command;
use std::time::{Duration, Instant};

pub struct Heartbeat {
    interval: Duration,
    topic: String,
    scheduled_at: Option<Instant>,
}

impl Heartbeat {
    pub fn new(interval_minutes: u64, topic: &str) -> Option<Self> {
        if interval_minutes == 0 || topic.is_empty() {
            return None;
        }
        Some(Self {
            interval: Duration::from_secs(interval_minutes * 60),
            topic: topic.to_string(),
            scheduled_at: None,
        })
    }

    /// Ensure a wake is scheduled roughly one interval out. pmset schedule
    /// entries auto-expire after firing, so we only re-schedule once the
    /// previous one is in the past.
    pub fn ensure_scheduled(&mut self, now: Instant) -> Result<()> {
        if self
            .scheduled_at
            .is_some_and(|t| now.saturating_duration_since(t) < self.interval)
        {
            return Ok(());
        }
        let minutes = (self.interval.as_secs() / 60).max(1);
        let date = Command::new("date")
            .args([format!("-v+{minutes}M"), "+%m/%d/%y %H:%M:%S".to_string()])
            .output()
            .context("running date")?;
        if !date.status.success() {
            bail!("date failed");
        }
        let when = String::from_utf8_lossy(&date.stdout).trim().to_string();
        let out = Command::new("/usr/bin/sudo")
            .args(["-n", "/usr/bin/pmset", "schedule", "wake", &when])
            .output()
            .context("running pmset schedule wake")?;
        if !out.status.success() {
            bail!(
                "pmset schedule wake failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        self.scheduled_at = Some(now);
        Ok(())
    }

    /// Poll the ntfy mailbox for a "wake" message published since roughly the
    /// last heartbeat window. Strict match so our own status pushes on the
    /// same topic can never self-trigger.
    pub fn wake_requested(&self) -> bool {
        let window_min = self.interval.as_secs() / 60 + 10;
        let url = format!(
            "https://ntfy.sh/{}/json?poll=1&since={window_min}m",
            self.topic
        );
        let Ok(out) = Command::new("curl").args(["-s", "-m", "8", &url]).output() else {
            return false;
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .any(is_wake_message)
    }
}

fn is_wake_message(v: serde_json::Value) -> bool {
    v["event"].as_str() == Some("message")
        && v["message"]
            .as_str()
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("wake"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_without_topic_or_interval() {
        assert!(Heartbeat::new(0, "topic").is_none());
        assert!(Heartbeat::new(20, "").is_none());
        assert!(Heartbeat::new(20, "topic").is_some());
    }

    #[test]
    fn wake_message_matching_is_strict() {
        let wake = serde_json::json!({"event": "message", "message": "WAKE"});
        let noise = serde_json::json!({"event": "message", "message": "Work finished, letting the Mac sleep"});
        let keepalive = serde_json::json!({"event": "keepalive"});
        assert!(is_wake_message(wake));
        assert!(!is_wake_message(noise));
        assert!(!is_wake_message(keepalive));
    }
}
