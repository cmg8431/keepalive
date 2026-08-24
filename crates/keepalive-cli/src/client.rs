use anyhow::{Context, Result};
use keepalive_core::config::socket_path;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub fn request(req: &serde_json::Value) -> Result<serde_json::Value> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path)
        .with_context(|| format!("daemon not reachable at {}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    writeln!(stream, "{req}")?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    serde_json::from_str(&line).context("invalid daemon response")
}

/// Hooks call this path: if the daemon isn't running yet, spawn it detached
/// and retry briefly. Must stay fast — an agent turn is blocked on us.
pub fn request_autostart(req: &serde_json::Value) -> Result<serde_json::Value> {
    if let Ok(res) = request(req) {
        return Ok(res);
    }
    let exe = std::env::current_exe().context("resolving own binary path")?;
    std::process::Command::new(exe)
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawning daemon")?;
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(res) = request(req) {
            return Ok(res);
        }
    }
    request(req)
}
