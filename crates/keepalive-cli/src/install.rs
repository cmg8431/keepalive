use crate::client;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

const LAUNCHD_LABEL: &str = "com.cmg8431.keepalive";
const HOOK_EVENTS: [(&str, &str); 4] = [
    ("UserPromptSubmit", "acquire"),
    ("PostToolUse", "acquire"),
    ("Stop", "release"),
    ("SessionEnd", "release"),
];

pub fn install() -> Result<()> {
    let bin = std::env::current_exe()?
        .canonicalize()
        .context("resolving binary path")?;
    install_launch_agent(&bin)?;
    install_claude_hooks(&bin)?;
    println!("keepalive installed:");
    println!("  daemon: LaunchAgent {LAUNCHD_LABEL} (starts at login, restarts on crash)");
    println!("  hooks:  registered in {}", settings_path().display());
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("gui/{}/{LAUNCHD_LABEL}", uid()?)])
        .output();
    let plist = plist_path()?;
    let _ = std::fs::remove_file(&plist);
    let _ = client::request(&serde_json::json!({ "cmd": "shutdown" }));
    remove_claude_hooks()?;
    println!("keepalive uninstalled: LaunchAgent removed, hooks cleaned up");
    Ok(())
}

fn uid() -> Result<String> {
    let out = Command::new("id")
        .arg("-u")
        .output()
        .context("running id -u")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn plist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home directory")?;
    Ok(home.join(format!("Library/LaunchAgents/{LAUNCHD_LABEL}.plist")))
}

fn install_launch_agent(bin: &std::path::Path) -> Result<()> {
    let log_dir = dirs::data_local_dir()
        .context("no data directory")?
        .join("keepalive");
    std::fs::create_dir_all(&log_dir)?;
    let log = log_dir.join("daemon.log");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LAUNCHD_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{bin}</string>
        <string>daemon</string>
    </array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>StandardOutPath</key><string>{log}</string>
    <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        bin = bin.display(),
        log = log.display(),
    );
    let path = plist_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }

    // Hand over from any previously running daemon (autostarted or old install).
    let uid = uid()?;
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("gui/{uid}/{LAUNCHD_LABEL}")])
        .output();
    let _ = client::request(&serde_json::json!({ "cmd": "shutdown" }));

    std::fs::write(&path, plist).context("writing LaunchAgent plist")?;
    let out = Command::new("launchctl")
        .args(["bootstrap", &format!("gui/{uid}")])
        .arg(&path)
        .output()
        .context("running launchctl bootstrap")?;
    if !out.status.success() {
        anyhow::bail!(
            "launchctl bootstrap failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn settings_path() -> PathBuf {
    let config_dir = std::env::var("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".claude"));
    config_dir.join("settings.json")
}

fn is_ours(hook: &serde_json::Value) -> bool {
    hook["command"]
        .as_str()
        .is_some_and(|c| c.contains("keepalive"))
}

fn install_claude_hooks(bin: &std::path::Path) -> Result<()> {
    let path = settings_path();
    let mut settings: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid JSON; fix it first", path.display()))?,
        Err(_) => serde_json::json!({}),
    };
    if !settings["hooks"].is_object() {
        settings["hooks"] = serde_json::json!({});
    }
    for (event, verb) in HOOK_EVENTS {
        let groups = &mut settings["hooks"][event];
        if !groups.is_array() {
            *groups = serde_json::json!([]);
        }
        let already = groups.as_array().unwrap().iter().any(|g| {
            g["hooks"]
                .as_array()
                .is_some_and(|hs| hs.iter().any(is_ours))
        });
        if !already {
            groups.as_array_mut().unwrap().push(serde_json::json!({
                "hooks": [{
                    "type": "command",
                    "command": format!("{} {verb} --from-hook", bin.display()),
                    "timeout": 5
                }]
            }));
        }
    }
    write_settings(&path, &settings)
}

fn remove_claude_hooks() -> Result<()> {
    let path = settings_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let Ok(mut settings) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(());
    };
    let Some(hooks) = settings["hooks"].as_object_mut() else {
        return Ok(());
    };
    for (_, groups) in hooks.iter_mut() {
        if let Some(arr) = groups.as_array_mut() {
            for group in arr.iter_mut() {
                if let Some(hs) = group["hooks"].as_array_mut() {
                    hs.retain(|h| !is_ours(h));
                }
            }
            arr.retain(|g| g["hooks"].as_array().is_none_or(|hs| !hs.is_empty()));
        }
    }
    write_settings(&path, &settings)
}

fn write_settings(path: &std::path::Path, settings: &serde_json::Value) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if path.exists() {
        let backup = path.with_extension("json.keepalive-backup");
        if !backup.exists() {
            let _ = std::fs::copy(path, &backup);
        }
    }
    std::fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(settings)?),
    )
    .with_context(|| format!("writing {}", path.display()))
}
