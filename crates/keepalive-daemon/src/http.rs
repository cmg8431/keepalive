//! Phone/browser control surface. Served on localhost plus the Tailscale
//! interface when present — never a public interface, so reachability itself
//! is the authentication boundary (the tailnet is the user's private network).

use crate::server::{Daemon, Request};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use keepalive_core::config::Config;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_stream::StreamExt;

#[derive(rust_embed::Embed)]
#[folder = "../../web/dist"]
struct Assets;

#[derive(Clone)]
struct AppState {
    daemon: Arc<Mutex<Daemon>>,
}

pub async fn serve_http(daemon: Arc<Mutex<Daemon>>, config: Config) {
    let port = config.web_port;
    let state = AppState { daemon };
    let app = Router::new()
        .route("/api/status", get(api_status))
        .route("/api/events", get(api_events))
        .route("/api/hold", post(api_hold))
        .route("/api/release", post(api_release))
        .route("/api/sleep", post(api_sleep))
        .route("/api/spawn", post(api_spawn))
        .route("/api/kill", post(api_kill))
        .route("/api/tail", post(api_tail))
        .route("/api/send", post(api_send))
        .route("/api/terminal/{name}/stream", get(api_terminal_stream))
        .route("/api/terminal/{name}/input", post(api_terminal_input))
        .route("/api/terminal/{name}/resize", post(api_terminal_resize))
        .route("/api/projects", get(api_projects))
        .route("/api/projects/add", post(api_projects_add))
        .route("/api/projects/remove", post(api_projects_remove))
        .route("/api/browse", get(api_browse))
        .route("/api/notify-test", post(api_notify_test))
        .route("/api/open-browser", post(api_open_browser))
        .route("/api/setup", get(api_setup))
        .route("/api/setup/provider", post(api_setup_provider))
        .route("/api/setup/ntfy", post(api_setup_ntfy))
        .route("/api/setup/https", post(api_setup_https))
        .route("/api/setup/lan", post(api_setup_lan))
        .route("/api/setup/hooks", post(api_setup_hooks))
        .fallback(static_assets)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            lan_guard,
        ))
        .with_state(state.clone());

    bind(app.clone(), SocketAddr::from(([127, 0, 0, 1], port)));
    // Interfaces come and go while the daemon runs (Tailscale login, Wi-Fi
    // hops, LAN access toggled on): keep watching and bind whatever exists.
    tokio::spawn(async move {
        let mut bound_ts: Option<IpAddr> = None;
        let mut bound_lan: Option<IpAddr> = None;
        loop {
            if let Some(ip) = crate::connect::tailscale_ip()
                && bound_ts != Some(ip)
            {
                bind(app.clone(), SocketAddr::new(ip, port));
                bound_ts = Some(ip);
            }
            let lan_on = !state.daemon.lock().unwrap().config().lan_key.is_empty();
            if lan_on
                && let Some(ip) = crate::connect::lan_ip()
                && bound_lan != Some(ip)
            {
                bind(app.clone(), SocketAddr::new(ip, port));
                bound_lan = Some(ip);
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
}

fn bind(app: Router, addr: SocketAddr) {
    tokio::spawn(async move {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                println!("[keepalived] dashboard on http://{addr}");
                if let Err(e) = axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                {
                    eprintln!("[keepalived] dashboard server on {addr} failed: {e}");
                }
            }
            Err(e) => eprintln!("[keepalived] dashboard bind {addr} failed: {e}"),
        }
    });
}

/// Reachability is the auth boundary for localhost and the tailnet, but the
/// LAN is shared territory (cafe Wi-Fi, office networks), so LAN peers must
/// present the pairing key once — from the QR deep link — and get a cookie.
/// The key never grants more than the tailnet already has.
async fn lan_guard(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<SocketAddr>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let trusted = match peer.ip() {
        IpAddr::V4(v4) => {
            v4.is_loopback() || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
        }
        IpAddr::V6(v6) => v6.is_loopback(),
    };
    if trusted {
        return next.run(req).await;
    }
    let key = state.daemon.lock().unwrap().config().lan_key.clone();
    if key.is_empty() {
        return (StatusCode::FORBIDDEN, "LAN access is not enabled").into_response();
    }
    let from_cookie = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|c| {
            c.split(';')
                .any(|pair| pair.trim().strip_prefix("ka=") == Some(key.as_str()))
        });
    let from_query = req
        .uri()
        .query()
        .is_some_and(|q| q.split('&').any(|pair| pair.strip_prefix("k=") == Some(&key)));
    if !from_cookie && !from_query {
        return (StatusCode::FORBIDDEN, "pairing required — scan the QR from the Mac")
            .into_response();
    }
    let mut response = next.run(req).await;
    if from_query && !from_cookie {
        // First visit through the QR link: persist the pairing in a cookie so
        // every later request (and the home-screen PWA) just works.
        let cookie = format!("ka={key}; Path=/; Max-Age=31536000; SameSite=Lax");
        if let Ok(value) = header::HeaderValue::from_str(&cookie) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

fn qr_svg(data: &str) -> Option<String> {
    let code = qrcode::QrCode::new(data).ok()?;
    Some(
        code.render::<qrcode::render::svg::Color>()
            .min_dimensions(180, 180)
            .quiet_zone(true)
            .build(),
    )
}

fn random_topic() -> String {
    format!("keepalive-{}", random_hex(6))
}

fn random_key() -> String {
    random_hex(16)
}

fn random_hex(bytes: usize) -> String {
    use std::io::Read;
    let mut buf = vec![0u8; bytes];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut buf);
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// The menu bar panel has no address bar, so it asks the daemon to hand the
/// dashboard to the real browser.
async fn api_open_browser(State(state): State<AppState>) -> Json<serde_json::Value> {
    let port = state.daemon.lock().unwrap().config().web_port;
    let _ = std::process::Command::new("open")
        .arg(format!("http://127.0.0.1:{port}/"))
        .spawn();
    Json(serde_json::json!({ "ok": true }))
}

async fn api_setup(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = state.daemon.lock().unwrap().config().clone();
    let ts_ip = crate::connect::tailscale_ip();
    let magic_dns = crate::connect::magic_dns_name();
    let https_on = crate::connect::https_active(config.web_port);
    // Best address first: a certificate-backed name beats a MagicDNS name with
    // a port, which beats a raw IP. The QR encodes whichever one applies, so
    // pointing a phone camera at it is always the shortest path in.
    let dashboard_url = match (&magic_dns, ts_ip) {
        (Some(name), _) if https_on => Some(format!("https://{name}")),
        (Some(name), _) => Some(format!("http://{name}:{}", config.web_port)),
        (None, Some(ip)) => Some(format!("http://{ip}:{}", config.web_port)),
        (None, None) => None,
    };
    let ntfy_url =
        (!config.ntfy_topic.is_empty()).then(|| format!("https://ntfy.sh/{}", config.ntfy_topic));
    // The LAN QR carries the pairing key: scanning it is the whole handshake.
    let lan_url = (!config.lan_key.is_empty())
        .then(crate::connect::lan_ip)
        .flatten()
        .map(|ip| format!("http://{ip}:{}/?k={}", config.web_port, config.lan_key));
    Json(serde_json::json!({
        "ok": true,
        "providers": crate::connect::providers(),
        "hooks": crate::connect::hooked_agents(),
        "projects": config.projects,
        "ntfy_topic": config.ntfy_topic,
        "heartbeat_minutes": config.heartbeat_minutes,
        "clamshell_ready": crate::clamshell::passwordless_available(),
        "dashboard_url": dashboard_url,
        "dashboard_qr": dashboard_url.as_deref().and_then(qr_svg),
        "magic_dns": magic_dns,
        "https_enabled": https_on,
        "lan_enabled": !config.lan_key.is_empty(),
        "lan_url": lan_url,
        "lan_qr": lan_url.as_deref().and_then(qr_svg),
        "ntfy_url": ntfy_url,
        "ntfy_qr": ntfy_url.as_deref().and_then(qr_svg),
    }))
}

#[derive(Deserialize)]
struct HttpsBody {
    enable: bool,
}

async fn api_setup_https(
    State(state): State<AppState>,
    Json(body): Json<HttpsBody>,
) -> Json<serde_json::Value> {
    let port = state.daemon.lock().unwrap().config().web_port;
    if !body.enable {
        return match crate::connect::disable_https(port) {
            Ok(()) => Json(serde_json::json!({ "ok": true })),
            Err(e) => Json(serde_json::json!({ "ok": false, "error": e })),
        };
    }
    match crate::connect::enable_https(port) {
        Ok(url) => Json(serde_json::json!({ "ok": true, "url": url })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e })),
    }
}

#[derive(Deserialize)]
struct LanBody {
    enable: bool,
}

/// Same-Wi-Fi access: mint (or drop) the pairing key. The bind watcher picks
/// up the LAN interface within seconds of the key existing.
async fn api_setup_lan(
    State(state): State<AppState>,
    Json(body): Json<LanBody>,
) -> Json<serde_json::Value> {
    let mut config = state.daemon.lock().unwrap().config().clone();
    if body.enable {
        if config.lan_key.is_empty() {
            config.lan_key = random_key();
        }
    } else {
        config.lan_key.clear();
    }
    if let Err(e) = config.save() {
        return Json(serde_json::json!({ "ok": false, "error": format!("saving config: {e}") }));
    }
    state.daemon.lock().unwrap().reload_config(config.clone());
    Json(serde_json::json!({ "ok": true, "enabled": !config.lan_key.is_empty() }))
}

/// One tap instead of "open a terminal and run keepalive install": the daemon
/// is the CLI binary, so it can re-run its own installer for agent hooks.
async fn api_setup_hooks(State(_state): State<AppState>) -> Json<serde_json::Value> {
    let Ok(bin) = std::env::current_exe() else {
        return Json(serde_json::json!({ "ok": false, "error": "cannot locate the keepalive binary" }));
    };
    match std::process::Command::new(bin)
        .args(["install", "--hooks-only"])
        .output()
    {
        Ok(out) if out.status.success() => Json(serde_json::json!({ "ok": true })),
        Ok(out) => Json(serde_json::json!({
            "ok": false,
            "error": String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": format!("running installer: {e}") })),
    }
}

#[derive(Deserialize)]
struct ProviderBody {
    id: String,
}

async fn api_setup_provider(
    State(_state): State<AppState>,
    Json(body): Json<ProviderBody>,
) -> Json<serde_json::Value> {
    match crate::connect::begin(&body.id) {
        Ok(message) => Json(serde_json::json!({ "ok": true, "message": message })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e })),
    }
}

#[derive(Deserialize)]
struct NtfyBody {
    enable: bool,
}

async fn api_setup_ntfy(
    State(state): State<AppState>,
    Json(body): Json<NtfyBody>,
) -> Json<serde_json::Value> {
    let mut config = state.daemon.lock().unwrap().config().clone();
    if body.enable {
        if config.ntfy_topic.is_empty() {
            config.ntfy_topic = random_topic();
        }
        if config.heartbeat_minutes == 0 {
            config.heartbeat_minutes = 20;
        }
    } else {
        config.ntfy_topic.clear();
        config.heartbeat_minutes = 0;
    }
    if let Err(e) = config.save() {
        return Json(serde_json::json!({ "ok": false, "error": format!("saving config: {e}") }));
    }
    state.daemon.lock().unwrap().reload_config(config.clone());
    Json(serde_json::json!({ "ok": true, "ntfy_topic": config.ntfy_topic }))
}

fn ask(state: &AppState, req: Request) -> serde_json::Value {
    state.daemon.lock().unwrap().handle(req)
}

fn status_with_projects(state: &AppState) -> serde_json::Value {
    let mut daemon = state.daemon.lock().unwrap();
    let mut status = daemon.handle(Request::Status);
    let config = daemon.config();
    status["projects"] = serde_json::json!(config.projects);
    // Guard thresholds so the dashboard can show live margins, not just raw
    // readings — "82% (floor 30%)" answers "when would it sleep?" at a glance.
    status["battery_floor_percent"] = serde_json::json!(config.battery_floor_percent);
    status["thermal_threshold_celsius"] = serde_json::json!(config.thermal_threshold_celsius);
    status["max_hold_hours"] = serde_json::json!(config.max_hold_hours);
    status
}

async fn api_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(status_with_projects(&state))
}

async fn api_events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let stream =
        tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(Duration::from_secs(3)))
            .map(move |_| {
                let status = status_with_projects(&state);
                Ok(Event::default().data(status.to_string()))
            });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[derive(Deserialize)]
struct HoldBody {
    #[serde(default)]
    minutes: Option<u64>,
    #[serde(default)]
    forever: bool,
}

async fn api_hold(
    State(state): State<AppState>,
    Json(body): Json<HoldBody>,
) -> Json<serde_json::Value> {
    Json(ask(
        &state,
        Request::Hold {
            minutes: body.minutes,
            forever: body.forever,
        },
    ))
}

#[derive(Deserialize)]
struct ReleaseBody {
    id: String,
}

async fn api_release(
    State(state): State<AppState>,
    Json(body): Json<ReleaseBody>,
) -> Json<serde_json::Value> {
    Json(ask(&state, Request::Release { id: body.id }))
}

async fn api_sleep(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(ask(&state, Request::Clear))
}

#[derive(Deserialize)]
struct SpawnBody {
    dir: String,
    #[serde(default)]
    name: Option<String>,
}

/// Remote spawning is an RCE surface by construction, so it stays triply
/// constrained: tailnet-only reachability, a known-projects check, and a fixed
/// command — the dashboard can only ever start `claude`.
///
/// "Known" is the config allowlist plus directories an agent has already run
/// in (learned from hooks). The learned half is what makes the phone usable
/// out of the box, and it grants nothing new: those are directories where the
/// same agent already ran under the same user.
async fn api_spawn(
    State(state): State<AppState>,
    Json(body): Json<SpawnBody>,
) -> (StatusCode, Json<serde_json::Value>) {
    let requested = std::path::Path::new(&body.dir);
    let Ok(canonical) = requested.canonicalize() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "directory does not exist" })),
        );
    };
    let allowed = {
        let daemon = state.daemon.lock().unwrap();
        crate::projects::is_allowed(&daemon.config().projects, daemon.recents(), &canonical)
    };
    if !allowed {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "ok": false,
                "error": "unknown project — add it from Projects first"
            })),
        );
    }
    let res = ask(
        &state,
        Request::Run {
            dir: canonical.to_string_lossy().into_owned(),
            command: None,
            name: body.name,
        },
    );
    (StatusCode::OK, Json(res))
}

#[derive(Deserialize)]
struct SendBody {
    name: String,
    /// Literal text to type. Mutually usable with `key`; text goes first.
    #[serde(default)]
    text: Option<String>,
    /// A named key from the fixed vocabulary (Enter, Escape, C-c, ...).
    #[serde(default)]
    key: Option<String>,
}

/// Answering the agent without a full terminal: the quick-reply bar and the
/// "type a message" field both land here.
async fn api_send(
    State(state): State<AppState>,
    Json(body): Json<SendBody>,
) -> Json<serde_json::Value> {
    if let Some(text) = body.text.filter(|t| !t.is_empty()) {
        let res = ask(
            &state,
            Request::SendText {
                name: body.name.clone(),
                text,
            },
        );
        if !res["ok"].as_bool().unwrap_or(false) {
            return Json(res);
        }
    }
    match body.key.filter(|k| !k.is_empty()) {
        Some(key) => Json(ask(
            &state,
            Request::SendKey {
                name: body.name,
                key,
            },
        )),
        None => Json(serde_json::json!({ "ok": true })),
    }
}

/// Streams the session's screen as it changes. Unknown names fail up front so
/// the browser gets a readable error instead of an event stream of failures.
async fn api_terminal_stream(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> axum::response::Response {
    if !state.daemon.lock().unwrap().manages(&name) {
        return (StatusCode::NOT_FOUND, format!("unknown session: {name}")).into_response();
    }
    let daemon = Arc::clone(&state.daemon);
    let mut last: Option<serde_json::Value> = None;
    let stream = tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(
        crate::terminal::FRAME_INTERVAL,
    ))
    .filter_map(move |_| {
        crate::terminal::next_frame(&daemon, &name, &mut last).map(|frame| {
            Ok::<Event, std::convert::Infallible>(Event::default().data(frame.to_string()))
        })
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

#[derive(Deserialize)]
struct TerminalInput {
    /// Hex-encoded keyboard bytes, so control and escape sequences survive.
    hex: String,
}

async fn api_terminal_input(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<TerminalInput>,
) -> Json<serde_json::Value> {
    Json(ask(
        &state,
        Request::SendRaw {
            name,
            hex: body.hex,
        },
    ))
}

#[derive(Deserialize)]
struct TerminalSize {
    cols: u16,
    rows: u16,
}

async fn api_terminal_resize(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<TerminalSize>,
) -> Json<serde_json::Value> {
    Json(ask(
        &state,
        Request::Resize {
            name,
            cols: body.cols,
            rows: body.rows,
        },
    ))
}

async fn api_projects(State(state): State<AppState>) -> Json<serde_json::Value> {
    let daemon = state.daemon.lock().unwrap();
    Json(serde_json::json!({
        "ok": true,
        "allowlist": daemon.config().projects,
        "recent": daemon.recents().list(),
    }))
}

#[derive(Deserialize)]
struct ProjectBody {
    dir: String,
}

/// Pins a directory into the config allowlist, so it survives even after it
/// ages out of the learned list.
async fn api_projects_add(
    State(state): State<AppState>,
    Json(body): Json<ProjectBody>,
) -> Json<serde_json::Value> {
    let Ok(canonical) = std::path::Path::new(&body.dir).canonicalize() else {
        return Json(serde_json::json!({ "ok": false, "error": "directory does not exist" }));
    };
    if !canonical.is_dir() {
        return Json(serde_json::json!({ "ok": false, "error": "not a directory" }));
    }
    let dir = canonical.to_string_lossy().into_owned();
    let mut config = state.daemon.lock().unwrap().config().clone();
    if !config.projects.contains(&dir) {
        config.projects.push(dir.clone());
    }
    if let Err(e) = config.save() {
        return Json(serde_json::json!({ "ok": false, "error": format!("saving config: {e}") }));
    }
    state.daemon.lock().unwrap().reload_config(config);
    Json(serde_json::json!({ "ok": true, "dir": dir }))
}

async fn api_projects_remove(
    State(state): State<AppState>,
    Json(body): Json<ProjectBody>,
) -> Json<serde_json::Value> {
    let mut config = state.daemon.lock().unwrap().config().clone();
    config.projects.retain(|p| p != &body.dir);
    if let Err(e) = config.save() {
        return Json(serde_json::json!({ "ok": false, "error": format!("saving config: {e}") }));
    }
    let mut daemon = state.daemon.lock().unwrap();
    daemon.reload_config(config);
    // Also drop it from the learned list, or it would immediately reappear as
    // a spawnable project and the removal would look like it did nothing.
    daemon.forget_recent(&body.dir);
    Json(serde_json::json!({ "ok": true }))
}

#[derive(Deserialize)]
struct BrowseQuery {
    #[serde(default)]
    path: Option<String>,
}

async fn api_browse(Query(q): Query<BrowseQuery>) -> Json<serde_json::Value> {
    match crate::projects::browse(q.path.as_deref()) {
        Ok(browse) => {
            let mut value = serde_json::to_value(browse).unwrap_or_default();
            value["ok"] = serde_json::json!(true);
            Json(value)
        }
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e })),
    }
}

#[derive(Deserialize)]
struct KillBody {
    name: String,
}

async fn api_kill(
    State(state): State<AppState>,
    Json(body): Json<KillBody>,
) -> Json<serde_json::Value> {
    Json(ask(&state, Request::Kill { name: body.name }))
}

async fn api_tail(
    State(state): State<AppState>,
    Json(body): Json<KillBody>,
) -> Json<serde_json::Value> {
    Json(ask(&state, Request::Tail { name: body.name }))
}

async fn api_notify_test(State(state): State<AppState>) -> Json<serde_json::Value> {
    let topic = state.daemon.lock().unwrap().config().ntfy_topic.clone();
    if topic.is_empty() {
        return Json(serde_json::json!({ "ok": false, "error": "notifications are not enabled" }));
    }
    crate::notify::push(&topic, "keepalive", "Test notification — the pipe works");
    Json(serde_json::json!({ "ok": true }))
}

async fn static_assets(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Assets::get(path).or_else(|| Assets::get("index.html")) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                content.data.into_owned(),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
