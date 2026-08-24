use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Session {
    pub source: String,
    /// Human-meaningful context (project directory name), so the UI never
    /// has to show a bare UUID.
    pub label: Option<String>,
    pub acquired_at: Instant,
    pub expires_at: Instant,
}

/// Reference-counted wake holds. Sleep is only blocked while at least one
/// session is alive; overlapping sessions stack and unblock together.
#[derive(Debug, Default)]
pub struct SessionTable {
    sessions: HashMap<String, Session>,
}

impl SessionTable {
    /// A renewal (same id) extends the TTL but keeps the original
    /// acquired_at, so "active for Nm" stays truthful.
    pub fn acquire(
        &mut self,
        id: &str,
        source: &str,
        label: Option<&str>,
        ttl: Duration,
        now: Instant,
    ) {
        match self.sessions.get_mut(id) {
            Some(existing) => {
                existing.expires_at = now + ttl;
                if let Some(l) = label {
                    existing.label = Some(l.to_string());
                }
            }
            None => {
                self.sessions.insert(
                    id.to_string(),
                    Session {
                        source: source.to_string(),
                        label: label.map(str::to_string),
                        acquired_at: now,
                        expires_at: now + ttl,
                    },
                );
            }
        }
    }

    pub fn release(&mut self, id: &str) -> bool {
        self.sessions.remove(id).is_some()
    }

    pub fn clear(&mut self) {
        self.sessions.clear();
    }

    pub fn prune_expired(&mut self, now: Instant) -> usize {
        let before = self.sessions.len();
        self.sessions.retain(|_, s| s.expires_at > now);
        before - self.sessions.len()
    }

    pub fn active_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Session)> {
        self.sessions.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(900);

    #[test]
    fn acquire_release_roundtrip() {
        let mut t = SessionTable::default();
        let now = Instant::now();
        t.acquire("a", "claude-code", None, TTL, now);
        assert_eq!(t.active_count(), 1);
        assert!(t.release("a"));
        assert_eq!(t.active_count(), 0);
        assert!(!t.release("a"));
    }

    #[test]
    fn re_acquire_renews_ttl() {
        let mut t = SessionTable::default();
        let now = Instant::now();
        t.acquire("a", "claude-code", None, TTL, now);
        t.acquire(
            "a",
            "claude-code",
            None,
            TTL,
            now + Duration::from_secs(800),
        );
        assert_eq!(t.prune_expired(now + Duration::from_secs(1000)), 0);
        assert_eq!(t.active_count(), 1);
    }

    #[test]
    fn expired_sessions_are_pruned() {
        let mut t = SessionTable::default();
        let now = Instant::now();
        t.acquire("a", "claude-code", None, TTL, now);
        t.acquire("b", "manual", None, TTL * 2, now);
        assert_eq!(t.prune_expired(now + TTL), 1);
        assert_eq!(t.active_count(), 1);
    }

    #[test]
    fn overlapping_sessions_stack() {
        let mut t = SessionTable::default();
        let now = Instant::now();
        t.acquire("a", "claude-code", None, TTL, now);
        t.acquire("b", "codex", None, TTL, now);
        t.release("a");
        assert_eq!(t.active_count(), 1);
    }
}
