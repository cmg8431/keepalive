//! Where the dashboard is allowed to start an agent session.
//!
//! The config allowlist used to be the only source, which meant the phone's
//! "new session" button answered FORBIDDEN until you hand-edited a TOML on the
//! laptop — the one setup step you cannot perform from the device the feature
//! exists for. Agent hooks already run *inside* the project directory and send
//! their cwd along with every hold, so the daemon learns the real list for
//! free and the allowlist seeds itself from actual use.
//!
//! Trusting a learned directory grants no capability the machine did not
//! already have: it is a directory where an agent has already run under this
//! user, and spawning is fixed to the `claude` command regardless.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

const MAX_RECENT: usize = 24;

#[derive(Serialize, Deserialize, Clone)]
pub struct RecentProject {
    /// Absolute, canonicalized path.
    pub dir: String,
    /// Directory basename — what the UI shows.
    pub name: String,
    /// Which agent was seen working here.
    pub source: String,
    pub last_seen: u64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Recents {
    projects: VecDeque<RecentProject>,
}

fn store_path() -> PathBuf {
    keepalive_core::config::data_dir().join("projects.json")
}

/// The home directory as the filesystem reports it. Paths coming from
/// elsewhere are canonicalized before comparison, so home has to be too or a
/// symlinked home (`/tmp` -> `/private/tmp`, `/home` -> `/Users`, ...) makes
/// every prefix check fail.
fn canonical_home() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.canonicalize().unwrap_or(home))
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Recents {
    pub fn load() -> Self {
        std::fs::read_to_string(store_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let path = store_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    /// Records a directory an agent is working in. Re-seeing a directory
    /// refreshes it and moves it to the front rather than duplicating.
    pub fn record(&mut self, dir: &str, source: &str) {
        let Ok(canonical) = Path::new(dir).canonicalize() else {
            return;
        };
        if !canonical.is_dir() {
            return;
        }
        // Home itself is where a shell starts, not a project; recording it
        // would trust the whole tree through the prefix check below.
        if canonical_home().is_some_and(|h| h == canonical) {
            return;
        }
        let dir = canonical.to_string_lossy().into_owned();
        let name = canonical
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.clone());
        self.projects.retain(|p| p.dir != dir);
        self.projects.push_front(RecentProject {
            dir,
            name,
            source: source.to_string(),
            last_seen: now_epoch(),
        });
        while self.projects.len() > MAX_RECENT {
            self.projects.pop_back();
        }
        self.save();
    }

    /// Drops a learned directory. Does not touch the config allowlist.
    pub fn forget(&mut self, dir: &str) -> bool {
        let before = self.projects.len();
        self.projects.retain(|p| p.dir != dir);
        if self.projects.len() != before {
            self.save();
            return true;
        }
        false
    }

    pub fn list(&self) -> Vec<RecentProject> {
        let mut out: Vec<RecentProject> = self
            .projects
            .iter()
            .filter(|p| Path::new(&p.dir).is_dir())
            .cloned()
            .collect();
        out.sort_by_key(|p| std::cmp::Reverse(p.last_seen));
        out
    }

    pub fn contains(&self, dir: &Path) -> bool {
        self.projects
            .iter()
            .any(|p| Path::new(&p.dir).canonicalize().is_ok_and(|d| d == dir))
    }
}

/// A spawn target is acceptable if it sits inside a configured allowlist entry
/// or is itself a directory an agent has already worked in.
pub fn is_allowed(allowlist: &[String], recents: &Recents, dir: &Path) -> bool {
    let in_allowlist = allowlist.iter().any(|p| {
        Path::new(p)
            .canonicalize()
            .is_ok_and(|allow| dir.starts_with(allow))
    });
    in_allowlist || recents.contains(dir)
}

#[derive(Serialize)]
pub struct BrowseEntry {
    pub name: String,
    pub dir: String,
    pub is_repo: bool,
}

#[derive(Serialize)]
pub struct Browse {
    pub dir: String,
    pub parent: Option<String>,
    pub entries: Vec<BrowseEntry>,
}

/// Directory picker for adding a project from the phone, so a path never has
/// to be typed on a touch keyboard. Confined to the home directory: browsing
/// is read-only, but there is no reason to expose the whole filesystem.
pub fn browse(path: Option<&str>) -> Result<Browse, String> {
    let home = canonical_home().ok_or("no home directory")?;
    let requested = match path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => home.clone(),
    };
    let dir = requested
        .canonicalize()
        .map_err(|_| "directory does not exist".to_string())?;
    if !dir.starts_with(&home) {
        return Err("outside the home directory".to_string());
    }
    if !dir.is_dir() {
        return Err("not a directory".to_string());
    }
    let mut entries: Vec<BrowseEntry> = std::fs::read_dir(&dir)
        .map_err(|e| format!("reading directory: {e}"))?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| {
            let p = e.path();
            BrowseEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                is_repo: p.join(".git").exists(),
                dir: p.to_string_lossy().into_owned(),
            }
        })
        .collect();
    // Repositories first, then alphabetical: the thing you are looking for is
    // almost always a repo.
    entries.sort_by(|a, b| b.is_repo.cmp(&a.is_repo).then(a.name.cmp(&b.name)));
    Ok(Browse {
        dir: dir.to_string_lossy().into_owned(),
        parent: dir
            .parent()
            .filter(|p| p.starts_with(&home) || *p == home)
            .map(|p| p.to_string_lossy().into_owned()),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recents_with(dirs: &[&str]) -> Recents {
        Recents {
            projects: dirs
                .iter()
                .map(|d| RecentProject {
                    dir: (*d).to_string(),
                    name: String::new(),
                    source: "test".to_string(),
                    last_seen: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn allowlist_matches_by_prefix_but_recents_match_exactly() {
        let repo = std::env::current_dir().unwrap().canonicalize().unwrap();
        let parent = repo.parent().unwrap().to_string_lossy().into_owned();

        // An allowlisted parent covers everything beneath it...
        let by_prefix = vec![parent];
        assert!(is_allowed(&by_prefix, &Recents::default(), &repo));

        // ...but a learned directory only vouches for itself, so an agent
        // having run in one repo never unlocks its siblings.
        let learned = recents_with(&[&repo.to_string_lossy()]);
        assert!(is_allowed(&[], &learned, &repo));
        assert!(!is_allowed(&[], &learned, repo.parent().unwrap()));
    }

    #[test]
    fn nothing_is_allowed_without_a_source() {
        let repo = std::env::current_dir().unwrap().canonicalize().unwrap();
        assert!(!is_allowed(&[], &Recents::default(), &repo));
    }

    #[test]
    fn browse_refuses_paths_outside_home() {
        assert!(browse(Some("/etc")).is_err());
    }

    #[test]
    fn browse_defaults_to_home_and_has_no_parent_there() {
        let Some(home) = canonical_home() else { return };
        let browsed = browse(None).expect("home is browsable");
        assert_eq!(browsed.dir, home.to_string_lossy());
        assert!(browsed.parent.is_none(), "home must be the ceiling");
    }
}
