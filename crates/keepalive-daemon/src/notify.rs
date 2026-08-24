//! Push notifications via ntfy.sh, fired through curl on a detached thread —
//! a slow network must never stall the daemon's policy loop.

pub fn push(topic: &str, title: &str, message: &str) {
    push_with_click(topic, title, message, None);
}

/// A notification about a specific session carries a tap target, so the push
/// itself is the way back in: tapping it opens that session's terminal on the
/// phone instead of dropping you on a dashboard you then have to navigate.
pub fn push_with_click(topic: &str, title: &str, message: &str, click: Option<String>) {
    if topic.is_empty() {
        return;
    }
    let url = format!("https://ntfy.sh/{topic}");
    let title = title.to_string();
    let message = message.to_string();
    std::thread::spawn(move || {
        let mut cmd = std::process::Command::new("curl");
        cmd.args(["-s", "-m", "10", "-H", &format!("Title: {title}")]);
        if let Some(click) = click {
            cmd.args(["-H", &format!("Click: {click}")]);
        }
        cmd.args(["-d", &message, &url]);
        let _ = cmd
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

/// Base URL a phone on the tailnet can reach this daemon at, used to build
/// notification tap targets. Prefers the MagicDNS name over a raw IP so a
/// saved notification keeps working if the tailnet reassigns addresses.
///
/// Resolving it shells out to `tailscale`, and this runs from the policy tick,
/// so the answer is cached — the address changes about as often as the machine
/// joins a new tailnet.
pub fn dashboard_base(port: u16) -> Option<String> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    const TTL: Duration = Duration::from_secs(300);
    static CACHE: Mutex<Option<(Instant, Option<String>)>> = Mutex::new(None);

    let mut cache = CACHE.lock().unwrap();
    if let Some((at, value)) = cache.as_ref()
        && at.elapsed() < TTL
    {
        return value.clone();
    }
    let resolved = match crate::connect::magic_dns_name() {
        Some(name) if crate::connect::https_active(port) => Some(format!("https://{name}")),
        Some(name) => Some(format!("http://{name}:{port}")),
        None => crate::connect::tailscale_ip().map(|ip| format!("http://{ip}:{port}")),
    };
    *cache = Some((Instant::now(), resolved.clone()));
    resolved
}
