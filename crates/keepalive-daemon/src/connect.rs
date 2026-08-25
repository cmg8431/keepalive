//! Connectivity providers: how the phone reaches this Mac. A small registry
//! so new providers (Cloudflare Tunnel, ZeroTier, ...) can be added without
//! touching the setup flow — the dashboard renders whatever this lists.

use serde::Serialize;
use std::net::IpAddr;
use std::os::unix::fs::PermissionsExt;
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

/// The Mac's address on the local network (192.168/10.x/172.16-31), for
/// same-Wi-Fi access without any VPN. Real network interfaces (en0/en1) are
/// asked first so a VM bridge or container network can't shadow the Wi-Fi
/// address; CGNAT is excluded — that's the tailnet, handled separately.
pub fn lan_ip() -> Option<IpAddr> {
    for iface in ["en0", "en1"] {
        if let Ok(out) = Command::new("ipconfig").args(["getifaddr", iface]).output()
            && out.status.success()
            && let Ok(IpAddr::V4(v4)) = String::from_utf8_lossy(&out.stdout).trim().parse()
            && v4.is_private()
        {
            return Some(IpAddr::V4(v4));
        }
    }
    let out = Command::new("ifconfig").output().ok()?;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(rest) = line.trim().strip_prefix("inet ")
            && let Some(addr) = rest.split_whitespace().next()
            && let Ok(IpAddr::V4(v4)) = addr.parse::<IpAddr>()
            && v4.is_private()
        {
            return Some(IpAddr::V4(v4));
        }
    }
    None
}

/// The machine's MagicDNS name (`macbookpro.tailXXXX.ts.net`), which is both
/// nicer to type than an IP and the only name a TLS certificate can be issued
/// for.
pub fn magic_dns_name() -> Option<String> {
    let bin = tailscale_bin()?;
    let out = Command::new(bin).args(["status", "--json"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let name = value["Self"]["DNSName"].as_str()?.trim_end_matches('.');
    (!name.is_empty()).then(|| name.to_string())
}

/// Whether `tailscale serve` is already fronting the dashboard on 443.
pub fn https_active(port: u16) -> bool {
    let Some(bin) = tailscale_bin() else {
        return false;
    };
    let Ok(out) = Command::new(bin).args(["serve", "status"]).output() else {
        return false;
    };
    out.status.success() && String::from_utf8_lossy(&out.stdout).contains(&format!("{port}"))
}

/// Puts the dashboard behind `https://<magic-dns-name>` — no port, a real
/// certificate, and a name that survives the tailnet handing out a different
/// IP. Still tailnet-only: `serve` (unlike `funnel`) never faces the internet.
///
/// Certificate issuance requires HTTPS to be enabled once for the tailnet, and
/// only an admin can do that in the web console, so a failure here is
/// surfaced verbatim rather than flattened into "failed".
pub fn enable_https(port: u16) -> Result<String, String> {
    let bin = tailscale_bin().ok_or("Tailscale is not installed")?;
    let name = magic_dns_name().ok_or("MagicDNS name unavailable — is Tailscale logged in?")?;
    let out = Command::new(bin)
        .args(["serve", "--bg", "--https=443", &port.to_string()])
        .output()
        .map_err(|e| format!("running tailscale serve: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        let detail = if err.is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            err.to_string()
        };
        return Err(if detail.to_lowercase().contains("https") {
            format!(
                "{detail}\n\nEnable HTTPS certificates once at \
                 https://login.tailscale.com/admin/dns, then try again."
            )
        } else {
            detail
        });
    }
    Ok(format!("https://{name}"))
}

/// Removes the `serve` mapping, dropping back to `http://<ip>:<port>`.
pub fn disable_https(port: u16) -> Result<(), String> {
    let bin = tailscale_bin().ok_or("Tailscale is not installed")?;
    Command::new(bin)
        .args(["serve", "--https=443", "off", &port.to_string()])
        .output()
        .map_err(|e| format!("running tailscale serve: {e}"))?;
    Ok(())
}

fn tailscale_state() -> ProviderState {
    if let Some(ip) = tailscale_ip() {
        return ProviderState::Connected { ip: ip.to_string() };
    }
    // The binary appearing supersedes whatever the install left behind —
    // otherwise a cancelled or unobservable install latches "Installing".
    if tailscale_bin().is_some() {
        *INSTALL_STATE.lock().unwrap() = None;
        return ProviderState::WaitingForLogin;
    }
    if let Some(s) = INSTALL_STATE.lock().unwrap().clone()
        && matches!(s, ProviderState::Installing | ProviderState::Failed { .. })
    {
        return s;
    }
    ProviderState::NotInstalled
}

/// Which agents currently have keepalive hooks wired, for the setup
/// checklist. Config files, not process state: an agent that has never run
/// still counts as connected once `keepalive install` touched its config.
pub fn hooked_agents() -> Vec<serde_json::Value> {
    const AGENTS: [(&str, &str); 4] = [
        ("claude-code", ".claude/settings.json"),
        ("codex", ".codex/hooks.json"),
        ("cursor", ".cursor/hooks.json"),
        ("gemini-cli", ".gemini/settings.json"),
    ];
    let home = dirs::home_dir().unwrap_or_default();
    AGENTS
        .into_iter()
        .map(|(name, rel)| {
            let installed = std::fs::read_to_string(home.join(rel))
                .is_ok_and(|text| text.contains("keepalive"));
            serde_json::json!({ "name": name, "installed": installed })
        })
        .collect()
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

const BREW_PATHS: [&str; 2] = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"];
const DOWNLOAD_URL: &str = "https://tailscale.com/download/macos";

fn brew_bin() -> Option<&'static str> {
    BREW_PATHS
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
}

/// The cask installs a pkg, which needs an admin password — impossible to
/// answer from a launchd daemon with no TTY. Hand the install to a Terminal
/// window the user can actually type into instead of failing silently.
fn open_brew_installer(brew: &str) -> Result<(), String> {
    let script = keepalive_core::config::data_dir().join("install-tailscale.command");
    if let Some(dir) = script.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let body = format!(
        "#!/bin/sh\n\
         echo 'keepalive: installing Tailscale with Homebrew.'\n\
         echo 'macOS will ask for your password — the installer needs it.'\n\
         echo\n\
         {brew} install --cask tailscale-app || {brew} install --cask tailscale || exit 1\n\
         open -a Tailscale\n\
         echo\n\
         echo 'Installed. Log in to Tailscale, then go back to keepalive.'\n"
    );
    std::fs::write(&script, body).map_err(|e| format!("writing installer script: {e}"))?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("marking installer executable: {e}"))?;
    Command::new("open")
        .args(["-a", "Terminal"])
        .arg(&script)
        .status()
        .map_err(|e| format!("opening Terminal: {e}"))?;
    Ok(())
}

fn begin_tailscale() -> Result<String, String> {
    if tailscale_ip().is_some() {
        return Ok("Already connected. Install the Tailscale app on your phone and log in with the same account.".into());
    }
    if tailscale_bin().is_some() {
        let _ = Command::new("open").args(["-a", "Tailscale"]).status();
        return Ok("Tailscale opened on the Mac — log in there, then install the Tailscale app on your phone with the same account.".into());
    }
    let Some(brew) = brew_bin() else {
        let _ = Command::new("open").arg(DOWNLOAD_URL).status();
        return Ok(format!(
            "Homebrew was not found, so the download page is open in your browser ({DOWNLOAD_URL}). Install Tailscale, then press Connect again."
        ));
    };
    if let Err(e) = open_brew_installer(brew) {
        *INSTALL_STATE.lock().unwrap() = Some(ProviderState::Failed { error: e.clone() });
        let _ = Command::new("open").arg(DOWNLOAD_URL).status();
        return Err(format!("{e} — opened {DOWNLOAD_URL} instead"));
    }
    *INSTALL_STATE.lock().unwrap() = Some(ProviderState::Installing);
    Ok("Installing Tailscale in a Terminal window — enter your macOS password there. This page updates itself when it finishes.".into())
}
