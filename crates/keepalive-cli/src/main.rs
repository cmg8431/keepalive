mod client;
mod install;

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
        /// Read the session id from a Claude Code hook payload on stdin
        #[arg(long)]
        from_hook: bool,
    },
    /// Release a session's wake hold
    Release {
        #[arg(long, conflicts_with = "from_hook")]
        id: Option<String>,
        #[arg(long)]
        from_hook: bool,
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
        } => {
            let (id, source) = resolve_session(id, source, from_hook)?;
            let mut req = serde_json::json!({ "cmd": "acquire", "id": id, "source": source });
            if let Some(ttl) = ttl_secs {
                req["ttl_secs"] = ttl.into();
            }
            client::request_autostart(&req)?;
            Ok(())
        }
        Commands::Release { id, from_hook } => {
            let (id, _) = resolve_session(id, String::new(), from_hook)?;
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
    }
}

fn resolve_session(
    id: Option<String>,
    source: String,
    from_hook: bool,
) -> Result<(String, String)> {
    if from_hook {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        let payload: serde_json::Value = serde_json::from_str(&input)?;
        let id = payload["session_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("hook payload has no session_id"))?
            .to_string();
        return Ok((id, "claude-code".to_string()));
    }
    let id = id.ok_or_else(|| anyhow::anyhow!("--id is required (or use --from-hook)"))?;
    Ok((id, source))
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
