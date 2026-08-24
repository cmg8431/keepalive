//! A live terminal for the phone, built out of `capture-pane` frames rather
//! than a tmux client.
//!
//! The obvious implementation — spawn `tmux attach` on a PTY and pipe it to
//! xterm.js — was built first and rejected: when the browser goes away and the
//! attached client dies, tmux takes the **whole server** down with it, killing
//! every agent session on the machine. (Measured on tmux 3.7c: a `sleep`
//! session survives, any job-control shell does not; detaching the client
//! first does not avoid it.) For a daemon whose entire job is keeping agent
//! work alive, a viewer that can kill the work is not a viewer.
//!
//! So the browser polls the rendered screen and posts keystrokes back. It
//! costs a redraw per frame instead of a byte stream, but nothing it does can
//! outlive or destabilise the session it is looking at.

use crate::server::{Daemon, Request};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Fast enough that typing feels attached, slow enough that a phone on cellular
/// and an idle agent both stay cheap. Frames are only sent when the screen
/// actually changed, so a quiet agent costs one `capture-pane` per tick.
pub const FRAME_INTERVAL: Duration = Duration::from_millis(350);

/// Produces the next frame for a session, or `None` when it is unchanged.
pub fn next_frame(
    daemon: &Arc<Mutex<Daemon>>,
    name: &str,
    last: &mut Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    let frame = daemon.lock().unwrap().handle(Request::Snapshot {
        name: name.to_string(),
    });
    if last.as_ref() == Some(&frame) {
        return None;
    }
    *last = Some(frame.clone());
    Some(frame)
}
