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
    ai::{
        dispatch::{self, DispatchCtx},
        provider::ProviderError,
        tools as ai_tools,
        types::{ChatMessage, ChatRequest, ChatResponse, ContentBlock, Role, StopReason, Usage},
    },
    auth::Scope,
    bind::{LAUNCHER_VERSION, PROTO},
    config::TOKEN_TTL_SECS,
    consent::ConsentRequest,
    error::ApiError,
    fs_read::{canonicalize_roots, read_under_roots},
    middleware::{
        audit_wrap, bearer_guard, host_guard, origin_guard, require_ai_provider_call,
        require_bundle_read, require_bundle_write, require_excel_com, require_fs_read,
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
    // Phase 8: ai:* scopes are no longer reserved. The handshake
    // accepts them and the consent dialog (Phase 4 mechanism) is what
    // actually gates access. The `is_reserved` helper now returns
    // false for every Scope, but the call site is preserved so a
    // future scope re-marking is one-line.
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

// ---- /bundle/* ------------------------------------------------------------

#[derive(Deserialize)]
struct BundlePathRequest {
    path: String,
}

#[derive(Deserialize)]
struct BundleWriteRequest {
    path: String,
    payload: serde_json::Value,
}

fn ensure_path_under_roots(
    requested: &str,
    token: &TokenRecordExt,
) -> Result<std::path::PathBuf, ApiError> {
    let canon = std::fs::canonicalize(requested).map_err(|_| ApiError::PathNotFound)?;
    if !token.0.fs_roots.iter().any(|root| canon.starts_with(root)) {
        return Err(ApiError::ForbiddenPath);
    }
    Ok(canon)
}

fn map_worker_bundle_error(error_code: &str) -> ApiError {
    match error_code {
        "bundle_not_found" => ApiError::BundleNotFound,
        "bundle_too_large" => ApiError::BundleTooLarge,
        "bundle_corrupt" => ApiError::BundleCorrupt,
        "bundle_schema_mismatch" => ApiError::BundleSchemaMismatch,
        "path_not_found" => ApiError::PathNotFound,
        _ => ApiError::Internal,
    }
}

async fn bundle_read_route(
    State(state): State<AppState>,
    Extension(token): Extension<TokenRecordExt>,
    Json(body): Json<BundlePathRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let canonical = ensure_path_under_roots(&body.path, &token)?;
    let resp = state
        .worker
        .call(
            "bundle_read",
            serde_json::json!({"path": canonical.to_string_lossy()}),
        )
        .await?;
    let ok = resp
        .get("ok")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !ok {
        let code = resp
            .get("error_code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        return Err(map_worker_bundle_error(code));
    }
    let payload = resp
        .get("payload")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    Ok(Json(payload))
}

async fn bundle_write_route(
    State(state): State<AppState>,
    Extension(token): Extension<TokenRecordExt>,
    Json(body): Json<BundleWriteRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let canonical = ensure_path_under_roots(&body.path, &token)?;
    let approved = state
        .consent
        .request(ConsentRequest {
            origin: format!("bundle-write://{}", canonical.to_string_lossy()),
            scopes: vec![Scope::BundleWrite],
            fs_roots: vec![],
        })
        .await;
    if !approved {
        return Err(ApiError::ConsentDenied);
    }
    let resp = state
        .worker
        .call(
            "bundle_write",
            serde_json::json!({
                "path": canonical.to_string_lossy(),
                "payload": body.payload,
            }),
        )
        .await?;
    let ok = resp
        .get("ok")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !ok {
        let code = resp
            .get("error_code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        return Err(map_worker_bundle_error(code));
    }
    Ok(Json(serde_json::json!({"ok": true})))
}

// ---- /ai/* ----------------------------------------------------------------

#[derive(Deserialize)]
struct OpenSessionRequest {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Serialize)]
struct OpenSessionResponse {
    session_id: String,
    provider: String,
    model: String,
    granted_scopes: Vec<Scope>,
    opened_ms: u128,
}

async fn ai_open_session(
    State(state): State<AppState>,
    Extension(token): Extension<TokenRecordExt>,
    Json(body): Json<OpenSessionRequest>,
) -> Result<Json<OpenSessionResponse>, ApiError> {
    let provider = body.provider.unwrap_or_else(|| "anthropic".to_string());
    let model = body
        .model
        .unwrap_or_else(|| crate::ai::anthropic::DEFAULT_MODEL.to_string());

    // Intersection: the AI's "internal bearer scopes" are the
    // user's token scopes restricted to the ones a tool registry
    // entry requires. The model never gains scopes the user did not
    // hold.
    let user_scopes: Vec<Scope> = token.0.scopes.clone();
    let registry = ai_tools::builtin_tools();
    let mut granted: Vec<Scope> = registry
        .iter()
        .map(|t| t.required_scope)
        .filter(|s| user_scopes.contains(s))
        .collect();
    granted.sort_by_key(|s| format!("{s:?}"));
    granted.dedup();

    // Phase 8: the internal bearer is the same token id as the user
    // token (we do not mint a separate token store entry yet — that
    // is a clean refactor for Phase 9 once trust windows arrive).
    let session = state.ai.sessions.open(
        provider.clone(),
        model.clone(),
        "internal".to_string(),
        granted.clone(),
        token.0.fs_roots.clone(),
    );

    Ok(Json(OpenSessionResponse {
        session_id: session.id.to_string(),
        provider,
        model,
        granted_scopes: granted,
        opened_ms: session.opened_ms,
    }))
}

async fn ai_chat(
    State(state): State<AppState>,
    Json(body): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, ApiError> {
    if state.ai.cost.over_budget() {
        return Err(ApiError::AiBudgetExceeded);
    }
    let session = state.ai.sessions.get(body.session_id)?;

    let provider = state
        .ai
        .providers
        .get(&session.provider)
        .ok_or(ApiError::AiProviderUnconfigured)?;

    let tools = ai_tools::builtin_tools();

    // Append the new user messages to the session before calling
    // the provider, so the transcript is the full multi-turn flow.
    state
        .ai
        .sessions
        .append_messages(session.id, body.messages.clone())?;

    let mut history = state.ai.sessions.get(session.id)?.messages.clone();
    let mut final_message = ChatMessage {
        role: Role::Assistant,
        ts_ms: None,
        content: vec![],
    };
    let mut final_stop = StopReason::EndTurn;
    let mut final_usage = Usage::default();

    // Phase 8: bounded tool-use loop. Cap at 8 round trips so a
    // confused model can't pin the worker forever.
    const MAX_TURNS: usize = 8;
    for _ in 0..MAX_TURNS {
        let turn = provider
            .chat(&history, &tools, body.max_tokens)
            .await
            .map_err(|e| match e {
                ProviderError::Unconfigured => ApiError::AiProviderUnconfigured,
                _ => ApiError::AiProviderUpstream,
            })?;

        let assistant_message = turn.message.clone();
        history.push(assistant_message.clone());
        state
            .ai
            .sessions
            .append_messages(session.id, vec![assistant_message.clone()])?;

        // Find any tool_use blocks the model emitted; dispatch each
        // and feed the results back as a `tool` role message.
        let tool_uses: Vec<(String, String, serde_json::Value)> = assistant_message
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, name, input } => {
                    Some((id.clone(), name.clone(), input.clone()))
                }
                _ => None,
            })
            .collect();

        // Record cost on every provider turn (Phase 9). Failures
        // here do not break the chat — the worst case is a missing
        // ledger line that the next call replaces.
        let _ = state
            .ai
            .cost
            .record(&session.provider, &session.model, &turn.usage)
            .await;

        if tool_uses.is_empty() {
            final_message = assistant_message;
            final_stop = turn.stop_reason;
            final_usage = turn.usage;
            break;
        }

        let ctx = DispatchCtx {
            state: &state,
            ai_consent: &state.ai.consent,
            trust_windows: &state.ai.trust_windows,
            session: &session,
            plan_only: body.plan_only,
        };
        let mut tool_results = Vec::new();
        for (tu_id, name, input) in tool_uses {
            let result = dispatch::execute_tool_use(&ctx, &tu_id, &name, &input).await;
            tool_results.push(result);
        }
        let tool_message = ChatMessage {
            role: Role::Tool,
            ts_ms: None,
            content: tool_results,
        };
        history.push(tool_message.clone());
        state
            .ai
            .sessions
            .append_messages(session.id, vec![tool_message])?;
        final_stop = turn.stop_reason;
        final_usage = turn.usage;
    }

    Ok(Json(ChatResponse {
        session_id: session.id,
        message: final_message,
        stop_reason: final_stop,
        usage: final_usage,
    }))
}

#[derive(Serialize)]
struct ProviderStatus {
    name: String,
    has_key: bool,
}

#[derive(Serialize)]
struct ProvidersResponse {
    providers: Vec<ProviderStatus>,
}

async fn ai_list_providers(
    State(state): State<AppState>,
) -> Result<Json<ProvidersResponse>, ApiError> {
    let providers = vec![ProviderStatus {
        name: "anthropic".to_string(),
        has_key: state
            .ai
            .keychain
            .read("anthropic_api_key")
            .map(|v| v.is_some())
            .unwrap_or(false),
    }];
    Ok(Json(ProvidersResponse { providers }))
}

#[derive(Deserialize)]
struct SetKeyRequest {
    provider: String,
    key: String,
}

async fn ai_set_provider_key(
    State(state): State<AppState>,
    Json(body): Json<SetKeyRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if body.provider != "anthropic" {
        return Err(ApiError::BadRequest);
    }
    let approved = state
        .ai
        .consent
        .request(ConsentRequest {
            origin: format!("ai-key://{}", body.provider),
            scopes: vec![],
            fs_roots: vec![],
        })
        .await;
    if !approved {
        return Err(ApiError::ConsentDenied);
    }
    state
        .ai
        .keychain
        .write(&format!("{}_api_key", body.provider), &body.key)
        .map_err(|_| ApiError::Internal)?;
    Ok(Json(serde_json::json!({"ok": true})))
}

#[derive(Deserialize)]
struct DeleteKeyParams {
    provider: String,
}

async fn ai_delete_provider_key(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<DeleteKeyParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let approved = state
        .ai
        .consent
        .request(ConsentRequest {
            origin: format!("ai-key-delete://{}", params.provider),
            scopes: vec![],
            fs_roots: vec![],
        })
        .await;
    if !approved {
        return Err(ApiError::ConsentDenied);
    }
    state
        .ai
        .keychain
        .delete(&format!("{}_api_key", params.provider))
        .map_err(|_| ApiError::Internal)?;
    Ok(Json(serde_json::json!({"ok": true})))
}

#[derive(Serialize)]
struct ToolsResponse {
    tools: Vec<crate::ai::types::ToolSpec>,
}

async fn ai_list_tools() -> Json<ToolsResponse> {
    Json(ToolsResponse {
        tools: ai_tools::builtin_tools(),
    })
}

#[derive(Serialize)]
struct SessionHistoryResponse {
    session_id: String,
    messages: Vec<ChatMessage>,
}

async fn ai_session_history(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<SessionHistoryResponse>, ApiError> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| ApiError::BadRequest)?;
    let session = state.ai.sessions.get(uuid)?;
    Ok(Json(SessionHistoryResponse {
        session_id: session.id.to_string(),
        messages: session.messages,
    }))
}

async fn ai_cost_today(State(state): State<AppState>) -> Json<crate::ai::cost::CostRollup> {
    Json(state.ai.cost.rollup())
}

#[derive(Deserialize)]
struct OpenTrustWindowRequest {
    session_id: String,
    tools: Vec<String>,
    duration_secs: u64,
}

async fn ai_open_trust_window(
    State(state): State<AppState>,
    Json(body): Json<OpenTrustWindowRequest>,
) -> Result<Json<crate::ai::trust_window::TrustWindow>, ApiError> {
    let session_id = uuid::Uuid::parse_str(&body.session_id).map_err(|_| ApiError::BadRequest)?;
    // Confirm session exists before bothering the user with a dialog.
    let _ = state.ai.sessions.get(session_id)?;
    let approved = state
        .ai
        .consent
        .request(ConsentRequest {
            origin: format!(
                "ai-trust://{}?{}s",
                body.tools.join(","),
                body.duration_secs
            ),
            scopes: vec![],
            fs_roots: vec![],
        })
        .await;
    if !approved {
        return Err(ApiError::AiConsentDenied);
    }
    let win = state
        .ai
        .trust_windows
        .open(session_id, body.tools, body.duration_secs);
    Ok(Json(win))
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

    // /bundle/*: read = bundle:read scope, write = bundle:write
    // scope. Both check the path is under the token's fs_roots
    // before letting the worker touch the file.
    let bundle_read = Router::new()
        .route("/bundle/read", post(bundle_read_route))
        .route_layer(from_fn(require_bundle_read))
        .route_layer(from_fn_with_state(state.clone(), bearer_guard))
        .route_layer(from_fn(origin_guard))
        .route_layer(from_fn_with_state(state.clone(), host_guard));
    let bundle_write = Router::new()
        .route("/bundle/write", post(bundle_write_route))
        .route_layer(from_fn(require_bundle_write))
        .route_layer(from_fn_with_state(state.clone(), bearer_guard))
        .route_layer(from_fn(origin_guard))
        .route_layer(from_fn_with_state(state.clone(), host_guard));

    // /ai/*: requires ai:provider:call. Tool dispatch inside the
    // chat handler additionally checks per-tool scopes against the
    // session's intersection.
    let ai = Router::new()
        .route("/ai/chat", post(ai_chat))
        .route("/ai/session", post(ai_open_session))
        .route("/ai/session/:id/history", get(ai_session_history))
        .route("/ai/providers", get(ai_list_providers))
        .route("/ai/providers/key", post(ai_set_provider_key))
        .route(
            "/ai/providers/key",
            axum::routing::delete(ai_delete_provider_key),
        )
        .route("/ai/tools", get(ai_list_tools))
        .route("/ai/cost/today", get(ai_cost_today))
        .route("/ai/consent/window", post(ai_open_trust_window))
        .route_layer(from_fn(require_ai_provider_call))
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
        .merge(bundle_read)
        .merge(bundle_write)
        .merge(ai)
        .layer(from_fn_with_state(state.clone(), audit_wrap))
        .with_state(state)
}
