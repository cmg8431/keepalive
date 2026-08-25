//! Menu bar companion: clicking the tray icon toggles a Figma-style panel
//! (borderless webview rendering the dashboard in panel mode). The icon
//! title mirrors daemon state; right-click gives a small utility menu.

#![allow(unexpected_cfgs)]

use anyhow::Result;
use keepalive_core::config::{Config, socket_path};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};
use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tao::window::WindowBuilder;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIconBuilder, TrayIconEvent};

const POLL: Duration = Duration::from_secs(5);
const PANEL_WIDTH: f64 = 420.0;
const PANEL_HEIGHT: f64 = 640.0;

fn request(req: &serde_json::Value) -> Result<serde_json::Value> {
    let mut stream = UnixStream::connect(socket_path())?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    writeln!(stream, "{req}")?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    Ok(serde_json::from_str(&line)?)
}

fn tray_title() -> &'static str {
    match request(&serde_json::json!({ "cmd": "status" })) {
        Ok(s) if s["cutout_latched"].as_bool().unwrap_or(false) => "☂",
        // An agent parked on a question outranks plain "awake" — that's the
        // moment the menu bar should pull the eye.
        Ok(s)
            if s["managed"]
                .as_array()
                .is_some_and(|m| m.iter().any(|x| x["waiting"].as_bool().unwrap_or(false))) =>
        {
            "✳"
        }
        Ok(s) if s["awake"].as_bool().unwrap_or(false) => "☀",
        _ => "☾",
    }
}

fn main() {
    let mut event_loop = EventLoopBuilder::new().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let web_port = Config::load().web_port;
    let panel_url = format!("http://127.0.0.1:{web_port}/?panel=1");

    let window = WindowBuilder::new()
        .with_title("keepalive")
        .with_inner_size(LogicalSize::new(PANEL_WIDTH, PANEL_HEIGHT))
        .with_decorations(false)
        .with_resizable(false)
        .with_always_on_top(true)
        .with_transparent(true)
        .with_visible(false)
        .build(&event_loop)
        .expect("failed to create panel window");

    let _webview = wry::WebViewBuilder::new()
        .with_url(&panel_url)
        .with_transparent(true)
        .build(&window)
        .expect("failed to create webview");

    // Right-click is the "I know what I want" path: the holds people actually
    // reach for, plus the escape hatch, without opening the panel at all.
    let menu = Menu::new();
    let hold_30 = MenuItem::new("Keep awake 30 min", true, None);
    let hold_1h = MenuItem::new("Keep awake 1 hour", true, None);
    let hold_3h = MenuItem::new("Keep awake 3 hours", true, None);
    let sleep_now = MenuItem::new("Let it sleep now", true, None);
    let open_browser = MenuItem::new("Open dashboard in browser", true, None);
    let quit = MenuItem::new("Quit keepalive menu", true, None);
    let _ = menu.append_items(&[
        &hold_30,
        &hold_1h,
        &hold_3h,
        &PredefinedMenuItem::separator(),
        &sleep_now,
        &PredefinedMenuItem::separator(),
        &open_browser,
        &quit,
    ]);

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_title("☾")
        .build()
        .expect("failed to create tray icon");

    let menu_events = MenuEvent::receiver();
    let tray_events = TrayIconEvent::receiver();
    let mut next_poll = Instant::now();

    event_loop.run(move |event, _target, control_flow| {
        if let Event::WindowEvent {
            event: WindowEvent::Focused(false),
            ..
        } = event
        {
            window.set_visible(false);
        }

        if Instant::now() >= next_poll {
            tray.set_title(Some(tray_title()));
            next_poll = Instant::now() + POLL;
        }
        *control_flow = ControlFlow::WaitUntil(next_poll);

        while let Ok(event) = tray_events.try_recv() {
            if let TrayIconEvent::Click {
                position,
                button: tray_icon::MouseButton::Left,
                button_state: tray_icon::MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                if window.is_visible() {
                    window.set_visible(false);
                } else {
                    let scale = window.scale_factor();
                    let width = PANEL_WIDTH * scale;
                    let x = (position.x - width / 2.0).max(8.0 * scale);
                    let y = rect.position.y + f64::from(rect.size.height);
                    window.set_outer_position(PhysicalPosition::new(x, y + 4.0 * scale));
                    window.set_visible(true);
                    window.set_focus();
                }
            }
        }

        while let Ok(event) = menu_events.try_recv() {
            let id = event.id();
            let hold = [
                (hold_30.id(), 30u64),
                (hold_1h.id(), 60),
                (hold_3h.id(), 180),
            ]
            .into_iter()
            .find(|(menu_id, _)| id == menu_id)
            .map(|(_, minutes)| minutes);
            if let Some(minutes) = hold {
                let _ = request(&serde_json::json!({ "cmd": "hold", "minutes": minutes }));
                tray.set_title(Some(tray_title()));
            } else if id == sleep_now.id() {
                let _ = request(&serde_json::json!({ "cmd": "clear" }));
                tray.set_title(Some(tray_title()));
            } else if id == open_browser.id() {
                let _ = std::process::Command::new("open")
                    .arg(format!("http://127.0.0.1:{web_port}/"))
                    .spawn();
            } else if id == quit.id() {
                *control_flow = ControlFlow::Exit;
            }
        }
    });
}
