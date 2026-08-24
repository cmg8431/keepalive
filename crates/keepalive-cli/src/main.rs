mod agents;
mod clamshell_setup;
mod client;
mod install;
mod mcp;
mod setup;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::io::Read;

#[derive(Parser)]
#[command(
    name = "keepalive",
    version,
    about = "Keeps your Mac awake while agents work — and lets it sleep when it's safer to."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the daemon in the foreground
    Daemon,
    /// Register (or renew) a wake hold for a session
    Acquire {
        #[arg(long, conflicts_with = "from_hook")]
        id: Option<String>,
        #[arg(long, default_value = "cli")]
        source: String,
        #[arg(long)]
        ttl_secs: Option<u64>,
        /// Read the session id from an agent hook payload on stdin
        #[arg(long)]
        from_hook: bool,
        /// Which agent this hook belongs to (used as source and id fallback)
        #[arg(long)]
        tool: Option<String>,
    },
    /// Release a session's wake hold
    Release {
        #[arg(long, conflicts_with = "from_hook")]
        id: Option<String>,
        #[arg(long)]
        from_hook: bool,
        #[arg(long)]
        tool: Option<String>,
    },
    /// Keep the Mac awake manually, no agent needed
    Hold {
        #[arg(long, default_value_t = 60)]
        minutes: u64,
    },
    /// Release every hold and let the Mac sleep normally
    Sleep,
    /// Show daemon state, sessions, and power status
    Status,
    /// Print the Claude Code hooks snippet for manual installation
    Hooks,
    /// Install the LaunchAgent (login autostart) and Claude Code hooks
    Install,
    /// Remove the LaunchAgent and clean up hooks
    Uninstall,
    /// Install the passwordless pmset rule for lid-closed wake (run with sudo)
    ClamshellSetup,
    /// Remove the passwordless pmset rule (run with sudo)
    ClamshellRemove,
    /// Start an agent in a managed tmux session (survives disconnects, auto-revives)
    Run {
        /// Project directory (defaults to the current directory)
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
        /// Command to run (defaults to claude)
        #[arg(long, default_value = "claude")]
        cmd: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// List managed sessions
    Sessions,
    /// Kill a managed session
    Kill { name: String },
    /// Run as an MCP stdio server (hold/release/status tools for agents)
    Mcp,
    /// Open the guided setup (phone access, notifications, language)
    Setup,
}

fn main() {
    let cli = Cli::parse();
    let from_hook = matches!(
        &cli.command,
        Commands::Acquire {
            from_hook: true,
            ..
        } | Commands::Release {
            from_hook: true,
            ..
        }
    );
    if let Err(e) = run(cli) {
        // Hook invocations must never fail an agent turn.
        if from_hook {
            std::process::exit(0);
        }
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Daemon => keepalive_daemon::run(),
        Commands::Acquire {
            id,
            source,
            ttl_secs,
            from_hook,
            tool,
        } => {
            let (id, source, label) = resolve_session(id, source, tool, from_hook)?;
            let mut req = serde_json::json!({ "cmd": "acquire", "id": id, "source": source });
            if let Some(ttl) = ttl_secs {
                req["ttl_secs"] = ttl.into();
            }
            if let Some(label) = label {
                req["label"] = label.into();
            }
            client::request_autostart(&req)?;
            Ok(())
        }
        Commands::Release {
            id,
            from_hook,
            tool,
        } => {
            let (id, _, _) = resolve_session(id, String::new(), tool, from_hook)?;
            client::request(&serde_json::json!({ "cmd": "release", "id": id }))?;
            Ok(())
        }
        Commands::Hold { minutes } => {
            client::request_autostart(&serde_json::json!({ "cmd": "hold", "minutes": minutes }))?;
            println!("holding wake for {minutes} minute(s)");
            Ok(())
        }
        Commands::Sleep => {
            client::request(&serde_json::json!({ "cmd": "clear" }))?;
            println!("all holds released — normal sleep restored");
            Ok(())
        }
        Commands::Status => {
            let res = client::request(&serde_json::json!({ "cmd": "status" }))?;
            println!("{}", serde_json::to_string_pretty(&res)?);
            Ok(())
        }
        Commands::Hooks => {
            print_hooks_snippet();
            Ok(())
        }
        Commands::Install => install::install(),
        Commands::Uninstall => install::uninstall(),
        Commands::Mcp => mcp::serve(),
        Commands::Setup => setup::run(),
        Commands::ClamshellSetup => clamshell_setup::setup(),
        Commands::ClamshellRemove => clamshell_setup::remove(),
        Commands::Run { dir, cmd, name } => {
            let dir = match dir {
                Some(d) => d,
                None => std::env::current_dir()?,
            };
            let dir = dir.canonicalize()?;
            let res = client::request_autostart(&serde_json::json!({
                "cmd": "run",
                "dir": dir.to_string_lossy(),
                "command": cmd,
                "name": name,
            }))?;
            print_result(&res);
            Ok(())
        }
        Commands::Sessions => {
            let res = client::request(&serde_json::json!({ "cmd": "sessions" }))?;
            println!("{}", serde_json::to_string_pretty(&res)?);
            Ok(())
        }
        Commands::Kill { name } => {
            let res = client::request(&serde_json::json!({ "cmd": "kill", "name": name }))?;
            print_result(&res);
            Ok(())
        }
    }
}

fn print_result(res: &serde_json::Value) {
    if res["ok"].as_bool().unwrap_or(false) {
        if let Some(name) = res["name"].as_str() {
            println!("started managed session {name} (attach: tmux attach -t {name})");
        } else {
            println!("ok");
        }
    } else {
        eprintln!("error: {}", res["error"].as_str().unwrap_or("unknown"));
    }
}

fn resolve_session(
    id: Option<String>,
    source: String,
    tool: Option<String>,
    from_hook: bool,
) -> Result<(String, String, Option<String>)> {
    if from_hook {
        let tool = tool.unwrap_or_else(|| "claude-code".to_string());
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        // Agents differ in payload shape; fall back to a per-tool id so
        // acquire and release still pair up without a session id.
        let payload = serde_json::from_str::<serde_json::Value>(&input).ok();
        let id = payload
            .as_ref()
            .and_then(|p| p["session_id"].as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{tool}-session"));
        // The project directory name is what makes the hold recognizable
        // in the dashboard, so a UUID never has to stand alone.
        let label = payload
            .as_ref()
            .and_then(|p| p["cwd"].as_str())
            .and_then(|cwd| {
                std::path::Path::new(cwd)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            });
        return Ok((id, tool, label));
    }
    let id = id.ok_or_else(|| anyhow::anyhow!("--id is required (or use --from-hook)"))?;
    Ok((id, source, None))
}

fn print_hooks_snippet() {
    println!(
        r#"Add to ~/.claude/settings.json (or use the plugin/ directory in this repo):

{{
  "hooks": {{
    "UserPromptSubmit": [{{ "hooks": [{{ "type": "command", "command": "keepalive acquire --from-hook" }}] }}],
    "PostToolUse": [{{ "hooks": [{{ "type": "command", "command": "keepalive acquire --from-hook" }}] }}],
    "Stop": [{{ "hooks": [{{ "type": "command", "command": "keepalive release --from-hook" }}] }}],
    "SessionEnd": [{{ "hooks": [{{ "type": "command", "command": "keepalive release --from-hook" }}] }}]
  }}
}}"#
    );
}
