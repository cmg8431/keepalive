//! Installs the scoped passwordless-sudo rule that lets the daemon toggle
//! `pmset disablesleep` (lid-closed wake) and schedule wakes (heartbeat).
//! Staged inside /etc/sudoers.d itself — never /tmp, which is world-writable
//! and invites symlink games on a root-privileged write.

use anyhow::{Context, Result, bail};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const SUDOERS_PATH: &str = "/etc/sudoers.d/keepalive";
const STAGING_PATH: &str = "/etc/sudoers.d/.keepalive.staging";

fn require_root() -> Result<()> {
    let out = Command::new("id")
        .arg("-u")
        .output()
        .context("running id -u")?;
    if String::from_utf8_lossy(&out.stdout).trim() != "0" {
        bail!("this command needs root: sudo keepalive clamshell-setup");
    }
    Ok(())
}

fn invoking_user() -> Result<String> {
    let user = std::env::var("SUDO_USER")
        .context("SUDO_USER not set — run via sudo, not as a root login")?;
    if user.is_empty()
        || user == "root"
        || !user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        bail!("refusing to write a sudoers rule for user {user:?}");
    }
    Ok(user)
}

pub fn setup() -> Result<()> {
    require_root()?;
    let user = invoking_user()?;
    let content = format!(
        "# Installed by keepalive (sudo keepalive clamshell-remove to undo)\n\
         {user} ALL=(root) NOPASSWD: /usr/bin/pmset -a disablesleep 1, /usr/bin/pmset -a disablesleep 0, /usr/bin/pmset -g, /usr/bin/pmset schedule wake *, /usr/bin/pmset schedule cancelall\n"
    );
    std::fs::write(STAGING_PATH, content).context("writing staging file")?;
    std::fs::set_permissions(STAGING_PATH, std::fs::Permissions::from_mode(0o440))
        .context("setting staging permissions")?;

    let check = Command::new("/usr/sbin/visudo")
        .args(["-cf", STAGING_PATH])
        .output()
        .context("running visudo -cf")?;
    if !check.status.success() {
        let _ = std::fs::remove_file(STAGING_PATH);
        bail!(
            "generated rule failed visudo validation: {}",
            String::from_utf8_lossy(&check.stderr).trim()
        );
    }
    std::fs::rename(STAGING_PATH, SUDOERS_PATH).context("installing sudoers rule")?;

    let full = Command::new("/usr/sbin/visudo")
        .arg("-c")
        .output()
        .context("running visudo -c")?;
    if !full.status.success() {
        let _ = std::fs::remove_file(SUDOERS_PATH);
        bail!(
            "system sudoers check failed after install; rule removed: {}",
            String::from_utf8_lossy(&full.stderr).trim()
        );
    }

    println!("clamshell rule installed for {user} at {SUDOERS_PATH}");
    println!("the daemon can now keep the Mac awake with the lid closed;");
    println!("restart it to pick this up: keepalive install (or relaunch the daemon)");
    Ok(())
}

pub fn remove() -> Result<()> {
    require_root()?;
    if Path::new(SUDOERS_PATH).exists() {
        std::fs::remove_file(SUDOERS_PATH).context("removing sudoers rule")?;
        println!("removed {SUDOERS_PATH}");
    } else {
        println!("nothing to remove — {SUDOERS_PATH} does not exist");
    }
    Ok(())
}
