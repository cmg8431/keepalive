//! `keepalive setup`: opens the dashboard's Settings tab, which hosts the
//! guided GUI setup (connectivity provider, notifications, language).

use crate::client;
use anyhow::Result;
use keepalive_core::config::Config;

pub fn run() -> Result<()> {
    // Make sure the daemon is up so the page loads.
    let _ = client::request_autostart(&serde_json::json!({ "cmd": "status" }))?;
    let url = format!("http://127.0.0.1:{}/#settings", Config::load().web_port);
    std::process::Command::new("open").arg(&url).spawn()?;
    println!("setup opened: {url}");
    Ok(())
}
