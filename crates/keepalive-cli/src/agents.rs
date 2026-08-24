//! Hook installers for agents beyond Claude Code. Formats verified against
//! each agent's real config shape:
//! - Codex: nested matcher-group JSON in ~/.codex/hooks.json; the flat form
//!   is silently ignored, and extra marker keys risk strict-parse rejection,
//!   so ownership is detected by command substring instead.
//! - Cursor: flat JSON in ~/.cursor/hooks.json (version: 1, lowercase
//!   events); holds carry a TTL because Cursor is one long-lived process.
//! - Gemini CLI: nested JSON in ~/.gemini/settings.json, session-scoped.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct AgentReport {
    pub name: &'static str,
    pub detail: String,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

fn is_ours(hook: &Value) -> bool {
    hook["command"].as_str().is_some_and(|c| {
        c.contains("keepalive") && (c.contains(" acquire") || c.contains(" release"))
    })
}

/// Missing file is Ok(None); an unparseable file is an error and must never
/// be overwritten (it may be jsonc or mid-edit).
fn read_json(path: &Path) -> Result<Option<Value>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(serde_json::from_str(&text).with_context(|| {
            format!("{} is not valid JSON; fix it first", path.display())
        })?)),
        Err(_) => Ok(None),
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if path.exists() {
        let backup = path.with_extension("json.keepalive-backup");
        if !backup.exists() {
            let _ = std::fs::copy(path, &backup);
        }
    }
    std::fs::write(path, format!("{}\n", serde_json::to_string_pretty(value)?))
        .with_context(|| format!("writing {}", path.display()))
}

fn command(bin: &Path, verb: &str, tool: &str, ttl: Option<u64>) -> String {
    let mut cmd = format!("{} {verb} --from-hook --tool {tool}", bin.display());
    if let Some(ttl) = ttl {
        cmd.push_str(&format!(" --ttl-secs {ttl}"));
    }
    cmd
}

fn nested_group(cmd: &str, marker: bool) -> Value {
    let mut hook = json!({ "type": "command", "command": cmd });
    if marker {
        hook["_keepalive"] = json!(true);
    }
    json!({ "hooks": [hook] })
}

fn nested_install(config: &mut Value, events: &[(&str, String)], marker: bool) {
    if !config["hooks"].is_object() {
        config["hooks"] = json!({});
    }
    for (event, cmd) in events {
        let groups = &mut config["hooks"][*event];
        if !groups.is_array() {
            *groups = json!([]);
        }
        let arr = groups.as_array_mut().unwrap();
        let already = arr.iter().any(|g| {
            g["hooks"]
                .as_array()
                .is_some_and(|hs| hs.iter().any(is_ours))
        });
        if !already {
            arr.push(nested_group(cmd, marker));
        }
    }
}

fn nested_uninstall(config: &mut Value) {
    let Some(hooks) = config["hooks"].as_object_mut() else {
        return;
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
    hooks.retain(|_, groups| groups.as_array().is_none_or(|a| !a.is_empty()));
}

fn codex_install(bin: &Path) -> Result<String> {
    let dir = home().join(".codex");
    if !dir.is_dir() {
        bail!("not detected");
    }
    let path = dir.join("hooks.json");
    let mut config = read_json(&path)?.unwrap_or_else(|| json!({}));
    // Codex deserializes handlers strictly: no marker key.
    nested_install(
        &mut config,
        &[
            (
                "UserPromptSubmit",
                command(bin, "acquire", "codex", Some(7200)),
            ),
            ("Stop", command(bin, "release", "codex", None)),
        ],
        false,
    );
    write_json(&path, &config)?;
    Ok(format!("hooks in {}", path.display()))
}

fn cursor_install(bin: &Path) -> Result<String> {
    let dir = home().join(".cursor");
    if !dir.is_dir() && !Path::new("/Applications/Cursor.app").exists() {
        bail!("not detected");
    }
    let path = dir.join("hooks.json");
    let mut config = read_json(&path)?.unwrap_or_else(|| json!({ "version": 1 }));
    if !config["hooks"].is_object() {
        config["hooks"] = json!({});
    }
    // Canonicalize: strip ours everywhere, then re-add (flat shape).
    cursor_uninstall_entries(&mut config);
    let entries = [
        (
            "beforeSubmitPrompt",
            command(bin, "acquire", "cursor", Some(3600)),
        ),
        ("stop", command(bin, "release", "cursor", None)),
    ];
    for (event, cmd) in entries {
        let groups = &mut config["hooks"][event];
        if !groups.is_array() {
            *groups = json!([]);
        }
        groups
            .as_array_mut()
            .unwrap()
            .push(json!({ "command": cmd, "_keepalive": true }));
    }
    write_json(&path, &config)?;
    Ok(format!("hooks in {}", path.display()))
}

fn cursor_uninstall_entries(config: &mut Value) {
    if let Some(hooks) = config["hooks"].as_object_mut() {
        for (_, groups) in hooks.iter_mut() {
            if let Some(arr) = groups.as_array_mut() {
                arr.retain(|entry| !is_ours(entry));
            }
        }
        hooks.retain(|_, groups| groups.as_array().is_none_or(|a| !a.is_empty()));
    }
}

fn gemini_install(bin: &Path) -> Result<String> {
    let dir = home().join(".gemini");
    if !dir.is_dir() {
        bail!("not detected");
    }
    let path = dir.join("settings.json");
    let mut config = read_json(&path)?.unwrap_or_else(|| json!({}));
    // Gemini sessions can run long between events, so the acquire TTL is
    // generous; SessionEnd releases explicitly.
    nested_install(
        &mut config,
        &[
            (
                "SessionStart",
                command(bin, "acquire", "gemini-cli", Some(14400)),
            ),
            ("SessionEnd", command(bin, "release", "gemini-cli", None)),
        ],
        true,
    );
    write_json(&path, &config)?;
    Ok(format!("hooks in {}", path.display()))
}

fn uninstall_file(path: &Path, flat: bool) -> Result<String> {
    let Some(mut config) = read_json(path).unwrap_or(None) else {
        bail!("nothing installed");
    };
    if flat {
        cursor_uninstall_entries(&mut config);
    } else {
        nested_uninstall(&mut config);
    }
    write_json(path, &config)?;
    Ok("cleaned".to_string())
}

pub fn install_all(bin: &Path) -> Vec<AgentReport> {
    let results = [
        ("codex", codex_install(bin)),
        ("cursor", cursor_install(bin)),
        ("gemini-cli", gemini_install(bin)),
    ];
    results
        .into_iter()
        .map(|(name, res)| AgentReport {
            name,
            detail: match res {
                Ok(d) => d,
                Err(e) => format!("skipped ({e:#})"),
            },
        })
        .collect()
}

pub fn uninstall_all() -> Vec<AgentReport> {
    let results = [
        (
            "codex",
            uninstall_file(&home().join(".codex/hooks.json"), false),
        ),
        (
            "cursor",
            uninstall_file(&home().join(".cursor/hooks.json"), true),
        ),
        (
            "gemini-cli",
            uninstall_file(&home().join(".gemini/settings.json"), false),
        ),
    ];
    results
        .into_iter()
        .map(|(name, res)| AgentReport {
            name,
            detail: match res {
                Ok(d) => d,
                Err(e) => format!("skipped ({e:#})"),
            },
        })
        .collect()
}
