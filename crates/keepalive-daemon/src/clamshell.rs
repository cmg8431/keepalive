//! Lid-closed sleep control. Only `pmset -a disablesleep` verifiably keeps a
//! displayless, lid-closed Mac awake (the cleaner IOPMrootDomain paths return
//! success but don't work — verified on-device by adrafinil). It needs root,
//! obtained via a scoped passwordless sudoers rule installed by
//! `sudo keepalive clamshell-setup`.
//!
//! disablesleep is a persistent power pref that outlives this process, so
//! every path out of "blocked" must clear it: release edge, daemon startup
//! reconcile (marker file), and signal-triggered shutdown.

use anyhow::{Context, Result, bail};
use keepalive_core::config::data_dir;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const PMSET_TIMEOUT: Duration = Duration::from_secs(10);

fn marker_path() -> PathBuf {
    data_dir().join("disablesleep.marker")
}

/// Runs a command with a hard timeout; pmset can wedge and this is called
/// under the daemon's state lock.
fn run_with_timeout(program: &str, args: &[&str], timeout: Duration) -> Result<()> {
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning {program}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            let mut err = String::new();
            if let Some(mut pipe) = child.stderr.take() {
                use std::io::Read;
                let _ = pipe.read_to_string(&mut err);
            }
            bail!("{program} {} failed: {}", args.join(" "), err.trim());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            bail!("{program} timed out after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// True when the passwordless sudoers rule is usable (`sudo -n` succeeds).
pub fn passwordless_available() -> bool {
    std::process::Command::new("/usr/bin/sudo")
        .args(["-n", "/usr/bin/pmset", "-g"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn set_blocked(blocked: bool) -> Result<()> {
    let value = if blocked { "1" } else { "0" };
    run_with_timeout(
        "/usr/bin/sudo",
        &["-n", "/usr/bin/pmset", "-a", "disablesleep", value],
        PMSET_TIMEOUT,
    )?;
    let marker = marker_path();
    if blocked {
        if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&marker, b"1");
    } else {
        let _ = std::fs::remove_file(&marker);
    }
    Ok(())
}

pub fn is_blocked_marker_set() -> bool {
    marker_path().exists()
}

/// Startup reconcile: if a previous daemon died while holding disablesleep,
/// clear it. Runs before any policy decision.
pub fn reconcile_on_startup() {
    if is_blocked_marker_set() {
        match set_blocked(false) {
            Ok(()) => eprintln!("[keepalived] cleared stale disablesleep from a previous run"),
            Err(e) => eprintln!("[keepalived] failed to clear stale disablesleep: {e:#}"),
        }
    }
}
