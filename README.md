<h1 align="center">keepalive</h1>

<p align="center">
  <strong>Keeps your Mac awake while agents work — and lets it sleep when it's safer to.</strong>
  <br />
  Agent-aware wake holds, hard safety guards, and (soon) session revival you can trigger from your phone.
</p>

---

> **Status: early development.** The core loop works; lid-closed sleep control, the menu bar app, and remote management are tracked in [issues](https://github.com/cmg8431/keepalive/issues).

## The Problem

You kick off a long Claude Code run, close the lid, and walk away. Either the Mac falls asleep and the agent dies mid-task — or you wire it awake with `caffeinate`/Amphetamine and it stays hot in your bag at 3 a.m. long after the work finished.

Remote-control tools (Claude Code Remote Control, Happy) gave you a phone. Nobody keeps the machine alive underneath them.

## The Solution

**keepalive** holds your Mac awake *only while an agent is actually working*, and always yields to safety:

- **Agent-aware.** Claude Code hooks acquire a wake hold when a turn starts and release it when the turn stops. No sessions → normal sleep, immediately.
- **Reference-counted.** Overlapping sessions stack; sleep unblocks when the last one releases. Holds carry a TTL, so a crashed agent can never wire the machine forever.
- **Safety guards win.** Battery below the floor (default 30%, on battery power) or a hold exceeding the hard time cap (default 8h) force-releases everything — even mid-task.
- **Manual holds too.** `keepalive hold --minutes 90` for a long download; `keepalive sleep` to clear everything.

## Install

```bash
git clone https://github.com/cmg8431/keepalive
cd keepalive
cargo install --path crates/keepalive-cli
keepalive hooks   # prints the Claude Code hooks snippet
```

The daemon starts automatically on the first `acquire` — no launchd setup needed yet (tracked in issues).

## Usage

```bash
keepalive status              # daemon state, active sessions, battery
keepalive hold --minutes 90   # manual wake hold, no agent needed
keepalive sleep               # release everything, restore normal sleep
keepalive daemon              # run the daemon in the foreground
```

## Architecture

```
Claude Code hooks ──▶ keepalive CLI ──▶ unix socket ──▶ keepalived
                     (acquire/release)                  │
                                                        ├─ SessionTable (ref-counted, TTL)
                                                        ├─ Policy (battery floor, max hold)
                                                        └─ IOPMAssertion (IOKit FFI, RAII)
```

| Crate | Role |
|---|---|
| `keepalive-core` | Pure logic: session table, wake/sleep policy, config. Fully unit-tested, no macOS APIs. |
| `keepalive-daemon` | Tokio unix-socket server, IOKit power assertion FFI, `pmset` power status. |
| `keepalive-cli` | Single `keepalive` binary: daemon runner + client commands + hook entrypoints. |

## Configuration

`~/.config/keepalive/config.toml` (all optional):

```toml
battery_floor_percent = 30
max_hold_hours = 8
default_ttl_secs = 900
poll_secs = 15
```

## Roadmap

Phase 1: lid-closed sleep control (privileged helper), thermal cutout, launchd, menu bar app.
Phase 2: tmux session persistence + `claude --resume` auto-revival, phone status page over Tailscale, ntfy push, heartbeat wake.
Phase 3: multi-agent hook installer, MCP tool, signed/notarized distribution.

See [issues](https://github.com/cmg8431/keepalive/issues) for details.

## License

MIT
