//! Menu bar companion: always-visible wake state plus one-click controls.
//! A thin shell over the daemon socket — no policy lives here.

#![allow(unexpected_cfgs)]

use anyhow::Result;
use keepalive_core::config::{Config, socket_path};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::TrayIconBuilder;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};

const POLL: Duration = Duration::from_secs(5);

fn request(req: &serde_json::Value) -> Result<serde_json::Value> {
    let mut stream = UnixStream::connect(socket_path())?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    writeln!(stream, "{req}")?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    Ok(serde_json::from_str(&line)?)
}

struct View {
    title: &'static str,
    detail: String,
}

fn read_state() -> View {
    match request(&serde_json::json!({ "cmd": "status" })) {
        Ok(s) => {
            let awake = s["awake"].as_bool().unwrap_or(false);
            let holds = s["sessions"].as_array().map_or(0, Vec::len);
            let temp = s["temperature_celsius"]
                .as_f64()
                .map_or(String::new(), |t| format!(" · {t:.0}°C"));
            let battery = s["battery_percent"]
                .as_u64()
                .map_or(String::new(), |b| format!(" · {b}%"));
            if s["cutout_latched"].as_bool().unwrap_or(false) {
                View {
                    title: "☂",
                    detail: format!("Safety cutout latched{temp}{battery}"),
                }
            } else if awake {
                let clam = if s["clamshell_active"].as_bool().unwrap_or(false) {
                    " · lid-safe"
                } else {
                    ""
                };
                View {
                    title: "☀",
                    detail: format!("Awake · {holds} hold(s){clam}{temp}{battery}"),
                }
            } else {
                View {
                    title: "☾",
                    detail: format!("Sleeping normally{battery}"),
                }
            }
        }
        Err(_) => View {
            title: "☾",
            detail: "Daemon not running".to_string(),
        },
    }
}

fn main() {
    let mut event_loop = EventLoopBuilder::new().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let menu = Menu::new();
    let status_line = MenuItem::new("Connecting…", false, None);
    let hold_1h = MenuItem::new("Hold awake 1 hour", true, None);
    let hold_3h = MenuItem::new("Hold awake 3 hours", true, None);
    let let_sleep = MenuItem::new("Let it sleep", true, None);
    let dashboard = MenuItem::new("Open dashboard", true, None);
    let quit = MenuItem::new("Quit keepalive menu", true, None);
    let _ = menu.append_items(&[
        &status_line,
        &PredefinedMenuItem::separator(),
        &hold_1h,
        &hold_3h,
        &let_sleep,
        &PredefinedMenuItem::separator(),
        &dashboard,
        &PredefinedMenuItem::separator(),
        &quit,
    ]);

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_title("☾")
        .build()
        .expect("failed to create tray icon");

    let menu_events = MenuEvent::receiver();
    let web_port = Config::load().web_port;
    let mut next_poll = Instant::now();

    event_loop.run(move |_event, _target, control_flow| {
        if Instant::now() >= next_poll {
            let view = read_state();
            tray.set_title(Some(view.title));
            status_line.set_text(view.detail);
            next_poll = Instant::now() + POLL;
        }
        *control_flow = ControlFlow::WaitUntil(next_poll);

        while let Ok(event) = menu_events.try_recv() {
            let id = event.id();
            if id == hold_1h.id() {
                let _ = request(&serde_json::json!({ "cmd": "hold", "minutes": 60 }));
            } else if id == hold_3h.id() {
                let _ = request(&serde_json::json!({ "cmd": "hold", "minutes": 180 }));
            } else if id == let_sleep.id() {
                let _ = request(&serde_json::json!({ "cmd": "clear" }));
            } else if id == dashboard.id() {
                let _ = std::process::Command::new("open")
                    .arg(format!("http://127.0.0.1:{web_port}/"))
                    .spawn();
            } else if id == quit.id() {
                *control_flow = ControlFlow::Exit;
            }
            next_poll = Instant::now();
        }
    });
}
