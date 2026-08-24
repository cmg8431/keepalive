//! Minimal MCP stdio server: lets an agent deliberately hold the Mac awake
//! past its own turn ("long build running, hold 40 minutes") and inspect
//! daemon state. Line-delimited JSON-RPC 2.0, tools only.

use crate::client;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

pub fn serve() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(method) = msg["method"].as_str() else {
            continue;
        };
        let id = msg["id"].clone();
        // Notifications (no id) get no response.
        if id.is_null() {
            continue;
        }
        let result = match method {
            "initialize" => json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "keepalive", "version": env!("CARGO_PKG_VERSION") }
            }),
            "ping" => json!({}),
            "tools/list" => tools_list(),
            "tools/call" => tools_call(&msg["params"]),
            _ => {
                respond(
                    &mut stdout,
                    &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("method not found: {method}") } }),
                )?;
                continue;
            }
        };
        respond(
            &mut stdout,
            &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        )?;
    }
    Ok(())
}

fn respond(stdout: &mut impl Write, msg: &Value) -> Result<()> {
    writeln!(stdout, "{msg}")?;
    stdout.flush()?;
    Ok(())
}

fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "keepalive_hold",
                "description": "Keep the Mac awake for N minutes even after this agent turn ends. Use before kicking off long builds, deploys, or downloads that outlive the conversation turn.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "minutes": { "type": "integer", "description": "How long to hold the Mac awake (default 60)" }
                    }
                }
            },
            {
                "name": "keepalive_release",
                "description": "Release the manual wake hold placed by keepalive_hold.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "keepalive_status",
                "description": "Current keepalive state: awake holds, battery, temperature, lid, managed sessions.",
                "inputSchema": { "type": "object", "properties": {} }
            }
        ]
    })
}

fn tools_call(params: &Value) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];
    let outcome = match name {
        "keepalive_hold" => {
            let minutes = args["minutes"].as_u64().unwrap_or(60);
            client::request_autostart(&json!({ "cmd": "hold", "minutes": minutes }))
                .map(|_| format!("Holding the Mac awake for {minutes} minutes."))
        }
        "keepalive_release" => client::request(&json!({ "cmd": "release", "id": "manual" }))
            .map(|_| "Manual hold released.".to_string()),
        "keepalive_status" => client::request(&json!({ "cmd": "status" }))
            .map(|status| serde_json::to_string_pretty(&status).unwrap_or_default()),
        other => Err(anyhow::anyhow!("unknown tool: {other}")),
    };
    match outcome {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Err(e) => {
            json!({ "content": [{ "type": "text", "text": format!("error: {e:#}") }], "isError": true })
        }
    }
}
