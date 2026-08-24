use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Below this battery percentage (on battery power) sleep is always allowed.
    pub battery_floor_percent: u8,
    /// Hard cap on how long a single continuous wake hold may last.
    pub max_hold_hours: u64,
    /// Sessions that stop renewing are dropped after this TTL.
    pub default_ttl_secs: u64,
    pub poll_secs: u64,
    /// Lid-closed CPU temperature at which all holds are force-released.
    pub thermal_threshold_celsius: f64,
    /// Keep working with the lid closed (needs `sudo keepalive clamshell-setup`).
    pub clamshell: bool,
    /// Dashboard port; served on localhost plus the Tailscale interface if present.
    pub web_port: u16,
    /// ntfy.sh topic for push notifications (empty = disabled).
    pub ntfy_topic: String,
    /// Directories the dashboard may spawn new agent sessions in.
    pub projects: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            battery_floor_percent: 30,
            max_hold_hours: 8,
            default_ttl_secs: 900,
            poll_secs: 15,
            thermal_threshold_celsius: 80.0,
            clamshell: true,
            web_port: 7757,
            ntfy_topic: String::new(),
            projects: Vec::new(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("keepalive/config.toml"))
    }

    pub fn max_hold(&self) -> Duration {
        Duration::from_secs(self.max_hold_hours * 3600)
    }

    pub fn default_ttl(&self) -> Duration {
        Duration::from_secs(self.default_ttl_secs)
    }
}

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("keepalive")
}

pub fn socket_path() -> PathBuf {
    data_dir().join("daemon.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = Config::default();
        assert_eq!(c.battery_floor_percent, 30);
        assert_eq!(c.max_hold(), Duration::from_secs(8 * 3600));
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let c: Config = toml::from_str("battery_floor_percent = 20").unwrap();
        assert_eq!(c.battery_floor_percent, 20);
        assert_eq!(c.max_hold_hours, 8);
    }
}
