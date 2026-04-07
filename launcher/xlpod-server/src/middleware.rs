//! Five-check middleware stack + audit wrapper.
//!
//! Order (outermost first):
//!   1. `audit_wrap`        — records *every* request, including rejections
//!   2. `host_guard`        — DNS-rebinding defense (Host header)
//!   3. `origin_guard`      — origin allow-list (Origin header)
//!   4. `bearer_guard`      — bearer token validation
//!   5. `scope_guard`       — required scopes (per route, via extension)
//!   6. (handler)
//!
//! Rate limiting happens *inside* `bearer_guard` once we have a token id,
//! so anonymous floods on `/health` and `/auth/handshake` are limited by
//! origin/host (not per-token). A separate per-IP limiter could be added
//! later but is overkill for a loopback-only server.
//!
//! Each guard returns `Err(ApiError)` which the framework converts to a
//! JSON error matching `proto/xlpod.openapi.yaml#/components/schemas/Error`.

use std::time::Instant;

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap},
    middleware::Next,
    response::Response,
};

use crate::{
    audit::{now_ms, AuditEntry},
    auth::{Scope, TokenRecord, TokenStore},
    config::{allowed_hosts, ALLOWED_ORIGINS},
    error::ApiError,
    state::AppState,
};

// ---- 0. audit wrapper -----------------------------------------------------

pub async fn audit_wrap(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let start = Instant::now();
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let origin = header_str(request.headers(), header::ORIGIN).map(str::to_owned);
    let host = header_str(request.headers(), header::HOST).map(str::to_owned);
    let token_id = request
        .extensions()
        .get::<TokenIdExt>()
        .map(|t| t.0.clone());
    // We need to peek at the request, so move on:
    let response = next.run(request).await;
    let latency_ms = start.elapsed().as_millis() as u64;
    let entry = AuditEntry {
        ts_ms: now_ms(),
        actor: "user",
        token_id: response
            .extensions()
            .get::<TokenIdExt>()
            .map(|t| t.0.clone())
            .or(token_id),
        method: method.as_str(),
        path: &path,
        status: response.status().as_u16(),
        origin: origin.as_deref(),
        host: host.as_deref(),
        latency_ms,
    };
    state.audit.write(&entry).await;
    response
}

// ---- 1. host guard --------------------------------------------------------

pub async fn host_guard(request: Request, next: Next) -> Result<Response, ApiError> {
    let host = header_str(request.headers(), header::HOST).ok_or(ApiError::HostNotAllowed)?;
    let permitted = allowed_hosts();
    if !permitted.iter().any(|p| p == host) {
        return Err(ApiError::HostNotAllowed);
    }
    Ok(next.run(request).await)
}

// ---- 2. origin guard ------------------------------------------------------

pub async fn origin_guard(request: Request, next: Next) -> Result<Response, ApiError> {
    // The Origin header is absent on non-CORS requests (e.g. curl). We
    // require it for every route on this server because the *entire*
    // legitimate caller set is browser-style (xlwings Lite + future
    // browser-side python clients via fetch). curl-from-localhost callers
    // can supply `-H 'Origin: https://addin.xlwings.org'` for testing.
    let origin = header_str(request.headers(), header::ORIGIN).ok_or(ApiError::OriginNotAllowed)?;
    if !ALLOWED_ORIGINS.contains(&origin) {
        return Err(ApiError::OriginNotAllowed);
    }
    Ok(next.run(request).await)
}

// ---- 3. bearer guard + per-token rate limit -------------------------------

#[derive(Clone)]
pub struct TokenIdExt(pub String);

#[derive(Clone)]
#[allow(dead_code)] // .0 is consumed by per-route scope guards (Phase 1.x)
pub struct TokenRecordExt(pub TokenRecord);

pub async fn bearer_guard(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = bearer_token(request.headers()).ok_or(ApiError::Unauthorized)?;
    let record = state.tokens.lookup(&token)?;
    let token_id = TokenStore::id_of(&token);
    state.limiter.check(&token_id)?;
    request.extensions_mut().insert(TokenIdExt(token_id.clone()));
    request.extensions_mut().insert(TokenRecordExt(record));
    let mut response = next.run(request).await;
    // Propagate so the audit layer (outer) can record token_id even when
    // the handler short-circuits.
    response.extensions_mut().insert(TokenIdExt(token_id));
    Ok(response)
}

// ---- 4. scope guard (constructor returns a per-route middleware fn) -------

/// Build a scope-checking middleware for a specific required scope set.
/// Used as `axum::middleware::from_fn(require_scopes(&[Scope::FsRead]))`.
#[allow(dead_code)] // wired up by per-route guards in Phase 1.x
pub fn require_scopes(required: &'static [Scope]) -> impl Clone + Fn(Request, Next) -> ScopeFuture {
    move |request: Request, next: Next| {
        let required = required;
        Box::pin(async move {
            let record = request
                .extensions()
                .get::<TokenRecordExt>()
                .ok_or(ApiError::Unauthorized)?
                .0
                .clone();
            for need in required {
                if !record.scopes.iter().any(|s| s == need) {
                    return Err(ApiError::ScopeDenied);
                }
            }
            Ok::<Response, ApiError>(next.run(request).await)
        })
    }
}

#[allow(dead_code)] // paired with require_scopes above
pub type ScopeFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response, ApiError>> + Send>>;

// ---- helpers --------------------------------------------------------------

fn header_str(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let auth = header_str(headers, header::AUTHORIZATION)?;
    let stripped = auth.strip_prefix("Bearer ")?;
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_owned())
    }
}

