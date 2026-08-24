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
    config: Arc<Config>,
}

pub async fn serve_http(daemon: Arc<Mutex<Daemon>>, config: Config) {
    let port = config.web_port;
    let state = AppState {
        daemon,
        config: Arc::new(config),
    };
    let app = Router::new()
        .route("/api/status", get(api_status))
        .route("/api/events", get(api_events))
        .route("/api/hold", post(api_hold))
        .route("/api/sleep", post(api_sleep))
        .route("/api/spawn", post(api_spawn))
        .route("/api/kill", post(api_kill))
        .fallback(static_assets)
        .with_state(state);

    let mut addrs = vec![SocketAddr::from(([127, 0, 0, 1], port))];
    if let Some(ip) = tailscale_ip() {
        addrs.push(SocketAddr::new(ip, port));
    }
    for addr in addrs {
        let app = app.clone();
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
}

/// The Mac's own Tailscale address (100.64.0.0/10). Tries the tailscale CLI,
/// then falls back to scanning interfaces.
fn tailscale_ip() -> Option<IpAddr> {
    for bin in [
        "/usr/local/bin/tailscale",
        "/opt/homebrew/bin/tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ] {
        if let Ok(out) = std::process::Command::new(bin).args(["ip", "-4"]).output()
            && out.status.success()
            && let Some(line) = String::from_utf8_lossy(&out.stdout).lines().next()
            && let Ok(ip) = line.trim().parse::<IpAddr>()
        {
            return Some(ip);
        }
    }
    let out = std::process::Command::new("ifconfig").output().ok()?;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("inet ")
            && let Some(addr) = rest.split_whitespace().next()
            && let Ok(IpAddr::V4(v4)) = addr.parse::<IpAddr>()
            && is_cgnat(v4)
        {
            return Some(IpAddr::V4(v4));
        }
    }
    None
}

fn is_cgnat(ip: std::net::Ipv4Addr) -> bool {
    // 100.64.0.0/10
    let o = ip.octets();
    o[0] == 100 && (64..128).contains(&o[1])
}

fn ask(state: &AppState, req: Request) -> serde_json::Value {
    state.daemon.lock().unwrap().handle(req)
}

async fn api_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut status = ask(&state, Request::Status);
    status["projects"] = serde_json::json!(state.config.projects);
    Json(status)
}

async fn api_events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let stream =
        tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(Duration::from_secs(3)))
            .map(move |_| {
                let mut status = ask(&state, Request::Status);
                status["projects"] = serde_json::json!(state.config.projects);
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
    if state.config.projects.is_empty() {
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
    let allowed = state.config.projects.iter().any(|p| {
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
