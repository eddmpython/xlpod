//! HTTP + WebSocket routes.
//!
//! Phase 1.2: `/health`, `/auth/handshake`, `/launcher/version`, `/ws`.
//! Authoritative schemas live in `proto/xlpod.openapi.yaml`.

use std::path::PathBuf;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension, Query, State,
    },
    http::HeaderMap,
    middleware::{from_fn, from_fn_with_state},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

use crate::{
    auth::Scope,
    bind::{LAUNCHER_VERSION, PROTO},
    config::TOKEN_TTL_SECS,
    consent::ConsentRequest,
    error::ApiError,
    fs_read::{canonicalize_roots, read_under_roots},
    middleware::{
        audit_wrap, bearer_guard, host_guard, origin_guard, require_excel_com, require_fs_read,
        require_run_python, TokenRecordExt,
    },
    python_worker::ExecResult,
    state::AppState,
};

// ---- /health --------------------------------------------------------------

#[derive(Serialize)]
struct Health {
    status: &'static str,
    launcher: &'static str,
    proto: u32,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        launcher: LAUNCHER_VERSION,
        proto: PROTO,
    })
}

// ---- /auth/handshake ------------------------------------------------------

#[derive(Deserialize)]
struct HandshakeRequest {
    requested_scopes: Vec<Scope>,
    #[serde(default)]
    fs_roots: Vec<String>,
}

#[derive(Serialize)]
struct HandshakeResponse {
    token: String,
    granted_scopes: Vec<Scope>,
    granted_fs_roots: Vec<String>,
    expires_in: u64,
}

async fn handshake(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<HandshakeRequest>,
) -> Result<Json<HandshakeResponse>, ApiError> {
    if body.requested_scopes.iter().any(|s| s.is_reserved()) {
        return Err(ApiError::ReservedScope);
    }
    let wants_fs = body
        .requested_scopes
        .iter()
        .any(|s| matches!(s, Scope::FsRead | Scope::FsWrite));
    let granted_roots: Vec<PathBuf> = if wants_fs {
        let canon = canonicalize_roots(&body.fs_roots);
        if canon.is_empty() {
            // fs:* without any usable root is a programming error: the
            // token would carry the scope but be unable to do anything
            // with it. Reject so the client knows to widen its request.
            return Err(ApiError::BadRequest);
        }
        canon
    } else {
        Vec::new()
    };

    // Phase 4: ask the user (or the configured ConsentBackend) to
    // approve this handshake before any token is minted. Empty scope
    // sets are passed through unchallenged because the resulting token
    // can only call the public probes.
    if !body.requested_scopes.is_empty() {
        let origin = headers
            .get(axum::http::header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let approved = state
            .consent
            .request(ConsentRequest {
                origin,
                scopes: body.requested_scopes.clone(),
                fs_roots: granted_roots.clone(),
            })
            .await;
        if !approved {
            return Err(ApiError::ConsentDenied);
        }
    }

    let granted = body.requested_scopes.clone();
    let (token, _record) = state.tokens.issue(granted.clone(), granted_roots.clone());
    Ok(Json(HandshakeResponse {
        token,
        granted_scopes: granted,
        granted_fs_roots: granted_roots
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
        expires_in: TOKEN_TTL_SECS,
    }))
}

// ---- /launcher/version ----------------------------------------------------

#[derive(Serialize)]
struct Version {
    launcher: &'static str,
    proto: u32,
}

async fn launcher_version() -> Json<Version> {
    Json(Version {
        launcher: LAUNCHER_VERSION,
        proto: PROTO,
    })
}

// ---- /fs/read -------------------------------------------------------------

#[derive(Deserialize)]
struct FsReadParams {
    path: String,
}

#[derive(Serialize)]
struct FileContent {
    path: String,
    size: u64,
    encoding: &'static str,
    content: String,
}

async fn fs_read(
    Extension(token): Extension<TokenRecordExt>,
    Query(params): Query<FsReadParams>,
) -> Result<Json<FileContent>, ApiError> {
    let requested = PathBuf::from(&params.path);
    let result = read_under_roots(&requested, &token.0.fs_roots)?;
    let size = result.bytes.len() as u64;
    let encoded = BASE64.encode(&result.bytes);
    Ok(Json(FileContent {
        path: result.canonical.display().to_string(),
        size,
        encoding: "base64",
        content: encoded,
    }))
}

// ---- /run/python ----------------------------------------------------------

#[derive(Deserialize)]
struct RunPythonRequest {
    code: String,
}

async fn run_python(
    State(state): State<AppState>,
    Json(body): Json<RunPythonRequest>,
) -> Result<Json<ExecResult>, ApiError> {
    let result = state.worker.exec(&body.code).await?;
    Ok(Json(result))
}

// ---- /excel/* -------------------------------------------------------------

async fn excel_workbooks(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let resp = state
        .worker
        .excel_call("excel_workbooks", serde_json::json!({}))
        .await?;
    let workbooks = resp
        .get("workbooks")
        .cloned()
        .unwrap_or(serde_json::json!([]));
    Ok(Json(serde_json::json!({ "workbooks": workbooks })))
}

#[derive(Deserialize)]
struct RangeReadRequest {
    workbook: String,
    sheet: String,
    range: String,
}

async fn excel_range_read(
    State(state): State<AppState>,
    Json(body): Json<RangeReadRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let params = serde_json::json!({
        "workbook": body.workbook,
        "sheet": body.sheet,
        "range": body.range,
    });
    let resp = state.worker.excel_call("excel_range_read", params).await?;
    let address = resp
        .get("address")
        .cloned()
        .unwrap_or(serde_json::json!(""));
    let values = resp.get("values").cloned().unwrap_or(serde_json::json!([]));
    Ok(Json(serde_json::json!({
        "address": address,
        "values": values,
    })))
}

// ---- /ws ------------------------------------------------------------------

async fn ws_upgrade(
    State(_state): State<AppState>,
    ws: WebSocketUpgrade,
    axum::extract::Extension(token): axum::extract::Extension<TokenRecordExt>,
) -> Response {
    let _ = token; // referenced so the extension is required
    ws.on_upgrade(handle_ws)
}

async fn handle_ws(socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Ping(p) => {
                let _ = sink.send(Message::Pong(p)).await;
            }
            Message::Text(t) => {
                let _ = sink.send(Message::Text(t)).await;
            }
            Message::Binary(b) => {
                let _ = sink.send(Message::Binary(b)).await;
            }
            Message::Close(_) => break,
            Message::Pong(_) => {}
        }
    }
}

// ---- router assembly ------------------------------------------------------

pub fn router(state: AppState) -> Router {
    // Public routes: /health (no Origin/Host enforcement so curl-style
    // liveness checks work) and /auth/handshake (Origin + Host enforced).
    let public_open = Router::new().route("/health", get(health));

    let public_origin = Router::new()
        .route("/auth/handshake", post(handshake))
        .route_layer(from_fn(origin_guard))
        .route_layer(from_fn_with_state(state.clone(), host_guard));

    let authed = Router::new()
        .route("/launcher/version", get(launcher_version))
        .route("/ws", get(ws_upgrade))
        .route_layer(from_fn_with_state(state.clone(), bearer_guard))
        .route_layer(from_fn(origin_guard))
        .route_layer(from_fn_with_state(state.clone(), host_guard));

    // /fs/read: bearer + fs:read scope. Same outer guards (origin/host)
    // as the rest of the authenticated tree, plus a route_layer that
    // enforces the scope. route_layer applies inside-out, so the order
    // here is exactly: host -> origin -> bearer -> require_fs_read ->
    // handler.
    let fs = Router::new()
        .route("/fs/read", get(fs_read))
        .route_layer(from_fn(require_fs_read))
        .route_layer(from_fn_with_state(state.clone(), bearer_guard))
        .route_layer(from_fn(origin_guard))
        .route_layer(from_fn_with_state(state.clone(), host_guard));

    // /run/python: same gating, different scope.
    let run = Router::new()
        .route("/run/python", post(run_python))
        .route_layer(from_fn(require_run_python))
        .route_layer(from_fn_with_state(state.clone(), bearer_guard))
        .route_layer(from_fn(origin_guard))
        .route_layer(from_fn_with_state(state.clone(), host_guard));

    // /excel/*: requires excel:com.
    let excel = Router::new()
        .route("/excel/workbooks", get(excel_workbooks))
        .route("/excel/range/read", post(excel_range_read))
        .route_layer(from_fn(require_excel_com))
        .route_layer(from_fn_with_state(state.clone(), bearer_guard))
        .route_layer(from_fn(origin_guard))
        .route_layer(from_fn_with_state(state.clone(), host_guard));

    Router::new()
        .merge(public_open)
        .merge(public_origin)
        .merge(authed)
        .merge(fs)
        .merge(run)
        .merge(excel)
        .layer(from_fn_with_state(state.clone(), audit_wrap))
        .with_state(state)
}
