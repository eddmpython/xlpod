//! HTTP + WebSocket routes.
//!
//! Phase 1.2: `/health`, `/auth/handshake`, `/launcher/version`, `/ws`.
//! Authoritative schemas live in `proto/xlpod.openapi.yaml`.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    middleware::{from_fn, from_fn_with_state},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

use crate::{
    auth::Scope,
    bind::{LAUNCHER_VERSION, PROTO},
    config::TOKEN_TTL_SECS,
    error::ApiError,
    middleware::{audit_wrap, bearer_guard, host_guard, origin_guard, TokenRecordExt},
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
}

#[derive(Serialize)]
struct HandshakeResponse {
    token: String,
    granted_scopes: Vec<Scope>,
    expires_in: u64,
}

async fn handshake(
    State(state): State<AppState>,
    Json(body): Json<HandshakeRequest>,
) -> Result<Json<HandshakeResponse>, ApiError> {
    if body.requested_scopes.iter().any(|s| s.is_reserved()) {
        return Err(ApiError::ReservedScope);
    }
    // Phase 1.2: grant exactly what was requested. Phase 1.3 will route
    // through a tray consent dialog and may downgrade.
    let granted = body.requested_scopes.clone();
    let (token, _record) = state.tokens.issue(granted.clone());
    Ok(Json(HandshakeResponse {
        token,
        granted_scopes: granted,
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

    Router::new()
        .merge(public_open)
        .merge(public_origin)
        .merge(authed)
        .layer(from_fn_with_state(state.clone(), audit_wrap))
        .with_state(state)
}
