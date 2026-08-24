//! Phone/browser control surface. Served on localhost plus the Tailscale
//! interface when present — never a public interface, so reachability itself
//! is the authentication boundary (the tailnet is the user's private network).

use crate::server::{Daemon, Request};
use axum::extract::State;
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
        .route("/api/notify-test", post(api_notify_test))
        .route("/api/setup", get(api_setup))
        .route("/api/setup/provider", post(api_setup_provider))
        .route("/api/setup/ntfy", post(api_setup_ntfy))
        .fallback(static_assets)
        .with_state(state);

    bind(app.clone(), SocketAddr::from(([127, 0, 0, 1], port)));
    // The Tailscale interface can appear at any time (setup wizard installs
    // it live), so keep watching and bind as soon as it exists.
    tokio::spawn(async move {
        let mut bound: Option<IpAddr> = None;
        loop {
            if let Some(ip) = crate::connect::tailscale_ip()
                && bound != Some(ip)
            {
                bind(app.clone(), SocketAddr::new(ip, port));
                bound = Some(ip);
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
                if let Err(e) = axum::serve(listener, app).await {
                    eprintln!("[keepalived] dashboard server on {addr} failed: {e}");
                }
            }
            Err(e) => eprintln!("[keepalived] dashboard bind {addr} failed: {e}"),
        }
    });
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
    use std::io::Read;
    let mut buf = [0u8; 6];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut buf);
    }
    let hex: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    format!("keepalive-{hex}")
}

async fn api_setup(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = state.daemon.lock().unwrap().config().clone();
    let ts_ip = crate::connect::tailscale_ip();
    let dashboard_url = ts_ip.map(|ip| format!("http://{ip}:{}", config.web_port));
    let ntfy_url =
        (!config.ntfy_topic.is_empty()).then(|| format!("https://ntfy.sh/{}", config.ntfy_topic));
    Json(serde_json::json!({
        "ok": true,
        "providers": crate::connect::providers(),
        "ntfy_topic": config.ntfy_topic,
        "heartbeat_minutes": config.heartbeat_minutes,
        "clamshell_ready": crate::clamshell::passwordless_available(),
        "dashboard_url": dashboard_url,
        "dashboard_qr": dashboard_url.as_deref().and_then(qr_svg),
        "ntfy_url": ntfy_url,
        "ntfy_qr": ntfy_url.as_deref().and_then(qr_svg),
    }))
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
}

async fn api_hold(
    State(state): State<AppState>,
    Json(body): Json<HoldBody>,
) -> Json<serde_json::Value> {
    Json(ask(
        &state,
        Request::Hold {
            minutes: body.minutes,
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

/// Remote spawning is an RCE surface by construction, so it is triply
/// constrained: tailnet-only reachability, an explicit directory allowlist,
/// and a fixed command — the dashboard can only ever start `claude`.
async fn api_spawn(
    State(state): State<AppState>,
    Json(body): Json<SpawnBody>,
) -> (StatusCode, Json<serde_json::Value>) {
    let projects = state.daemon.lock().unwrap().config().projects.clone();
    if projects.is_empty() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "ok": false,
                "error": "no allowlisted projects — add `projects = [\"/path/to/repo\"]` to the config"
            })),
        );
    }
    let requested = std::path::Path::new(&body.dir);
    let Ok(canonical) = requested.canonicalize() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "directory does not exist" })),
        );
    };
    let allowed = projects.iter().any(|p| {
        std::path::Path::new(p)
            .canonicalize()
            .is_ok_and(|allow| canonical.starts_with(allow))
    });
    if !allowed {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "ok": false, "error": "directory is not in the allowlist" })),
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
