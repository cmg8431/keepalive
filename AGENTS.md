# AI Agent Guidelines

## Commit Message Convention

```
<type>(<scope>): <description>
```

| Type | Description |
|------|-------------|
| `feat` | New feature |
| `fix` | Bug fix |
| `refactor` | Code refactoring (no behavior change) |
| `docs` | Documentation only |
| `test` | Adding or updating tests |
| `chore` | Maintenance tasks |
| `perf` | Performance improvements |

Never add Claude/AI attribution (`Co-Authored-By`, "Generated with", etc.) to commits or PRs.

## Architecture Overview

keepalive is an **all-Rust cargo workspace**. One binary (`keepalive`) serves as daemon, client, and hook entrypoint.

- **`crates/keepalive-core`** — pure logic, no macOS APIs, no I/O beyond config loading. `SessionTable` (ref-counted TTL wake holds), `policy::evaluate` (wake/sleep decision), `Config`.
- **`crates/keepalive-daemon`** — tokio unix-socket server (JSON lines protocol), IOKit `IOPMAssertion` FFI (RAII: drop = release), `pmset -g batt` parsing.
- **`crates/keepalive-cli`** — clap CLI. `keepalive daemon` runs the daemon; everything else is a socket client.

### Core Loop

```
hooks (UserPromptSubmit/PostToolUse → acquire, Stop/SessionEnd → release)
  → CLI → unix socket → SessionTable
  → every poll tick: prune expired → read power → policy::evaluate
  → StayAwake: hold IOPMAssertion / AllowSleep: drop it (+ clear table if a guard tripped)
```

## Key Design Decisions

- **Safety guards always beat wake holds.** Battery floor and max-hold cap force-release even mid-task, and clear the session table so the guard can't re-arm next tick.
- **TTL on every hold.** A crashed agent that never sends `release` expires in `default_ttl_secs`. `PostToolUse` renews the TTL during long turns.
- **RAII assertions.** `WakeAssertion::drop` releases the IOPM assertion — a daemon crash can never leave the Mac permanently wired.
- **Hook invocations always exit 0.** A broken keepalive must never fail an agent turn.
- **Injected clocks.** Core takes `now: Instant` as a parameter — tests never sleep.
- **cfg-gated FFI.** `power/iokit.rs` is macOS-only; `power/stub.rs` keeps the workspace compiling elsewhere. CI checks and tests on `macos-latest`.
- **`anyhow::Result`** for all fallible functions.

## Adding a New Safety Guard

1. Add the signal to `PolicyInput` and a variant to `SleepReason` in `core/src/policy.rs`
2. Add the check in `evaluate()` (order matters: cheapest/most-critical first) + a test
3. Feed the signal in `Daemon::tick()` in `daemon/src/server.rs`

## Adding a New CLI Command

1. Add a variant to `Commands` in `cli/src/main.rs` and a match arm in `run()`
2. If it needs the daemon: add a `Request` variant + `handle()` arm in `daemon/src/server.rs`

## Code Conventions

- Rust edition 2024, workspace dependencies in root `Cargo.toml`
- No comments unless the logic is non-obvious
- `cargo fmt` + `cargo clippy --workspace -- -D warnings` must pass before commit
