<h1 align="center">keepalive</h1>

<p align="center">
  <strong>Keeps your Mac awake while agents work — and lets it sleep when it's safer to.</strong>
  <br />
  Close the lid, walk away, and manage everything from your phone.
</p>

---

> **Status: functional pre-release.** Every feature below works and is exercised end to end; signing/notarization and Homebrew distribution are tracked in [issues](https://github.com/cmg8431/keepalive/issues).

## What it does

You kick off a long Claude Code run, close the lid, and walk away. Either the Mac sleeps and the agent dies mid-task, or you wire it awake with `caffeinate` and it stays hot in your bag at 3 a.m. Remote-control tools (Claude Code Remote Control, Happy) gave you a phone — nobody keeps the machine alive underneath them. keepalive does:

- **Agent-aware wake holds.** Hooks for Claude Code, Codex, Cursor, and Gemini CLI acquire a hold when a turn starts and release it when it stops. No sessions → normal sleep, immediately. Holds are reference-counted with TTLs, so a crashed agent can never wire the machine forever.
- **Lid-closed (clamshell) operation.** A scoped passwordless-sudo rule lets the daemon toggle `pmset disablesleep` — the only mechanism that verifiably keeps a displayless, lid-closed Mac awake. Cleared on every exit path (release, startup reconcile, SIGTERM), so it can never strand.
- **Safety guards always win.** Battery floor (default 30% on battery), lid-closed thermal cutout (raw SMC reads, default 80°C), and a hard max-hold cap (8h) force-release everything mid-task, then latch with hysteresis so the still-running agent can't oscillate the machine back awake.
- **Sessions that survive and revive.** `keepalive run` starts an agent inside a managed tmux session: disconnects can't kill it, and if the process dies it comes back via `claude --continue` with its conversation intact.
- **A phone dashboard.** The daemon serves a React dashboard on localhost and your Tailscale interface only: live status (SSE), hold/sleep buttons, session list, and one-tap spawning of new agent sessions in allowlisted project directories.
- **Heartbeat wake.** Before allowing sleep, the daemon schedules periodic RTC wakes; each brief wake it polls an ntfy topic. Send "wake" from your phone and the Mac comes back and holds for 30 minutes.
- **ntfy push notifications.** Work finished, safety cutout fired, session revived — pushed to your phone.
- **MCP tool.** Agents can call `keepalive_hold` to keep the Mac awake past their own turn ("long build running, hold 40 minutes").
- **Menu bar companion.** `keepalive-menubar` shows ☾/☀/☂ state with one-click holds.

## Install

```bash
git clone https://github.com/cmg8431/keepalive
cd keepalive
cargo install --path crates/keepalive-cli
keepalive install                 # LaunchAgent + hooks for detected agents
sudo keepalive clamshell-setup    # optional: lid-closed wake + heartbeat scheduling
```

`keepalive install` registers the daemon as a LaunchAgent (starts at login, restarts on crash) and wires hooks for every detected agent. `keepalive uninstall` reverses everything.

## Usage

```bash
keepalive status                  # holds, battery, temperature, lid, sessions
keepalive hold --minutes 90       # manual hold, no agent needed
keepalive sleep                   # release everything
keepalive run --dir ~/proj        # start claude in a managed tmux session
keepalive sessions                # list managed sessions
keepalive kill ka-1               # stop one
keepalive mcp                     # MCP stdio server (for agent config)
keepalive-menubar &               # menu bar companion
```

Dashboard: `http://localhost:7757` (or `http://<tailscale-ip>:7757` from your phone).

## Configuration

`~/.config/keepalive/config.toml` (all optional):

```toml
battery_floor_percent = 30        # force sleep below this, on battery
max_hold_hours = 8                # hard cap on any continuous hold
default_ttl_secs = 900            # holds expire unless renewed
poll_secs = 15
thermal_threshold_celsius = 80.0  # lid-closed cutout
clamshell = true                  # use pmset disablesleep when set up
web_port = 7757
ntfy_topic = ""                   # your ntfy.sh topic; enables push + heartbeat
heartbeat_minutes = 0             # 0 = off; e.g. 20 wakes every 20 min while asleep
projects = []                     # dirs the dashboard may spawn sessions in
```

To wake a sleeping Mac from your phone: `heartbeat_minutes = 20`, set `ntfy_topic`, then publish `wake` to that topic (ntfy app or `curl -d wake ntfy.sh/<topic>`).

## Architecture

```
agent hooks / MCP ──▶ keepalive CLI ──▶ unix socket ─┐
phone browser ──▶ Tailscale ──▶ axum dashboard ──────┤
menu bar app ────────────────────────────────────────┴─▶ keepalived
                                                          ├─ SessionTable (ref-counted TTL holds)
                                                          ├─ Policy: battery floor · thermal cutout · max hold · hysteresis latch
                                                          ├─ IOPMAssertion (IOKit FFI, RAII)
                                                          ├─ clamshell: sudo -n pmset disablesleep (scoped sudoers)
                                                          ├─ tmux session manager + claude --continue revival
                                                          ├─ heartbeat: pmset schedule wake + ntfy mailbox
                                                          └─ ntfy push
```

| Crate | Role |
|---|---|
| `keepalive-core` | Pure logic: session table, policy, cutout latch, config. No macOS APIs, fully unit-tested. |
| `keepalive-daemon` | Unix-socket server, IOKit/SMC FFI, clamshell, tmux sessions, axum dashboard, heartbeat. |
| `keepalive-cli` | The `keepalive` binary: daemon runner, client commands, hook entrypoints, installers, MCP. |
| `keepalive-app` | `keepalive-menubar`: tray companion (tray-icon, no policy). |
| `web/` | Vite/React dashboard, statically built and embedded into the daemon binary. |

## Security posture

- The dashboard binds only to localhost and the Tailscale interface — tailnet reachability is the boundary; nothing is ever exposed publicly.
- Remote spawning is triply constrained: tailnet-only, canonicalized directory allowlist (empty = disabled), fixed `claude` command.
- The sudoers rule is scoped to exact `pmset` subcommands, staged inside `/etc/sudoers.d`, and visudo-validated before and after install.
- `disablesleep` is cleared on release, on daemon startup (marker reconcile), and on SIGTERM/shutdown, and re-pushed every 60s while held to heal kernel resets.

## License

MIT
