//! Connectivity providers: how the phone reaches this Mac. A small registry
//! so new providers (Cloudflare Tunnel, ZeroTier, ...) can be added without
//! touching the setup flow — the dashboard renders whatever this lists.

use serde::Serialize;
use std::net::IpAddr;
use std::process::Command;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderState {
    NotInstalled,
    Installing,
    /// Installed but not logged in / not connected yet.
    WaitingForLogin,
    Connected {
        ip: String,
    },
    Failed {
        error: String,
    },
}

#[derive(Serialize)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub recommended: bool,
    pub state: ProviderState,
}

static INSTALL_STATE: Mutex<Option<ProviderState>> = Mutex::new(None);

const TAILSCALE_PATHS: [&str; 3] = [
    "/usr/local/bin/tailscale",
    "/opt/homebrew/bin/tailscale",
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
];

fn tailscale_bin() -> Option<&'static str> {
    TAILSCALE_PATHS
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
}

pub fn tailscale_ip() -> Option<IpAddr> {
    if let Some(bin) = tailscale_bin()
        && let Ok(out) = Command::new(bin).args(["ip", "-4"]).output()
        && out.status.success()
        && let Some(line) = String::from_utf8_lossy(&out.stdout).lines().next()
        && let Ok(ip) = line.trim().parse()
    {
        return Some(ip);
    }
    // Fallback: scan interfaces for a CGNAT (100.64.0.0/10) address.
    let out = Command::new("ifconfig").output().ok()?;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(rest) = line.trim().strip_prefix("inet ")
            && let Some(addr) = rest.split_whitespace().next()
            && let Ok(IpAddr::V4(v4)) = addr.parse::<IpAddr>()
        {
            let o = v4.octets();
            if o[0] == 100 && (64..128).contains(&o[1]) {
                return Some(IpAddr::V4(v4));
            }
        }
    }
    None
}

fn tailscale_state() -> ProviderState {
    if let Some(ip) = tailscale_ip() {
        return ProviderState::Connected { ip: ip.to_string() };
    }
    if let Some(s) = INSTALL_STATE.lock().unwrap().clone()
        && matches!(s, ProviderState::Installing | ProviderState::Failed { .. })
    {
        return s;
    }
    if tailscale_bin().is_some() {
        ProviderState::WaitingForLogin
    } else {
        ProviderState::NotInstalled
    }
}

pub fn providers() -> Vec<ProviderInfo> {
    vec![
        ProviderInfo {
            id: "tailscale",
            label: "Tailscale",
            description: "Private network between your Mac and phone. Free, end-to-end encrypted, works from anywhere. Log in with the same account on both devices.",
            recommended: true,
            state: tailscale_state(),
        },
        ProviderInfo {
            id: "local",
            label: "This Mac only",
            description: "No remote access. The dashboard stays on localhost; you can enable a provider later.",
            recommended: false,
            state: ProviderState::Connected {
                ip: "127.0.0.1".into(),
            },
        },
    ]
}

/// Kick off provider setup. Long-running work (brew install) happens on a
/// detached thread; the dashboard polls state. Returns instruction text.
pub fn begin(id: &str) -> Result<String, String> {
    match id {
        "tailscale" => begin_tailscale(),
        "local" => {
            Ok("Nothing to set up — the dashboard is available on this Mac at localhost.".into())
        }
        other => Err(format!("unknown provider: {other}")),
    }
}

fn begin_tailscale() -> Result<String, String> {
    if tailscale_ip().is_some() {
        return Ok("Already connected. Install the Tailscale app on your phone and log in with the same account.".into());
    }
    if tailscale_bin().is_some() {
        let _ = Command::new("open").args(["-a", "Tailscale"]).status();
        return Ok("Tailscale opened on the Mac — log in there, then install the Tailscale app on your phone with the same account.".into());
    }
    let mut guard = INSTALL_STATE.lock().unwrap();
    if matches!(*guard, Some(ProviderState::Installing)) {
        return Ok("Still installing Tailscale via Homebrew…".into());
    }
    *guard = Some(ProviderState::Installing);
    drop(guard);
    std::thread::spawn(|| {
        let result = Command::new("brew")
            .args(["install", "--cask", "tailscale"])
            .output();
        let state = match result {
            Ok(out) if out.status.success() => {
                let _ = Command::new("open").args(["-a", "Tailscale"]).status();
                ProviderState::WaitingForLogin
            }
            Ok(out) => ProviderState::Failed {
                error: format!(
                    "brew install failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                        .lines()
                        .last()
                        .unwrap_or("unknown")
                ),
            },
            Err(e) => ProviderState::Failed {
                error: format!("could not run brew (is Homebrew installed?): {e}"),
            },
        };
        *INSTALL_STATE.lock().unwrap() = Some(state);
    });
    Ok("Installing Tailscale via Homebrew — this can take a couple of minutes…".into())
}
