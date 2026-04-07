// Tests legitimately use unwrap/expect on setup paths.
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Integration tests for the xlpod-server HTTP surface.
//!
//! These tests run the router over plain HTTP on a random loopback port,
//! so they exercise every middleware (Origin, Host, Bearer, Audit, Rate
//! limit, Reserved-scope rejection) without depending on the TLS layer
//! (which is provided by axum-server in the binary). The TLS path is
//! covered by the manual smoke test in the Phase 1.2 commit message.

use std::sync::Arc;

use reqwest::{Client, StatusCode};
use serde_json::json;
use tempfile::TempDir;
use xlpod_server::{
    audit::AuditLog, auth::TokenStore, make_app, rate_limit::RateLimiter, state::AppState,
};

const ORIGIN: &str = "https://addin.xlwings.org";

struct Harness {
    base: String,
    host_header: String,
    _audit_dir: TempDir,
}

async fn spawn() -> Harness {
    let dir = tempfile::tempdir().expect("tempdir");
    let audit = AuditLog::open(dir.path().join("audit.log"))
        .await
        .expect("audit");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let state = AppState {
        tokens: Arc::new(TokenStore::new()),
        limiter: Arc::new(RateLimiter::new()),
        audit,
        allowed_hosts: Arc::new(vec![format!("{addr}")]),
    };
    let app = make_app(state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Harness {
        base: format!("http://{addr}"),
        host_header: format!("{}", addr),
        _audit_dir: dir,
    }
}

fn client() -> Client {
    Client::builder().build().expect("client")
}

async fn handshake(h: &Harness, scopes: serde_json::Value) -> reqwest::Response {
    client()
        .post(format!("{}/auth/handshake", h.base))
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .json(&json!({ "requested_scopes": scopes }))
        .send()
        .await
        .expect("send")
}

#[tokio::test]
async fn health_is_open() {
    let h = spawn().await;
    let resp = client()
        .get(format!("{}/health", h.base))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["proto"], 1);
}

#[tokio::test]
async fn handshake_rejects_unknown_origin() {
    let h = spawn().await;
    let resp = client()
        .post(format!("{}/auth/handshake", h.base))
        .header("Origin", "https://evil.example")
        .header("Host", &h.host_header)
        .json(&json!({ "requested_scopes": ["fs:read"] }))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["code"], "origin_not_allowed");
}

#[tokio::test]
async fn handshake_rejects_bad_host() {
    let h = spawn().await;
    let resp = client()
        .post(format!("{}/auth/handshake", h.base))
        .header("Origin", ORIGIN)
        .header("Host", "evil.com")
        .json(&json!({ "requested_scopes": ["fs:read"] }))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["code"], "host_not_allowed");
}

#[tokio::test]
async fn handshake_rejects_reserved_scopes() {
    let h = spawn().await;
    let resp = handshake(&h, json!(["ai:provider:call"])).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["code"], "reserved_scope");
}

#[tokio::test]
async fn handshake_issues_token() {
    let h = spawn().await;
    let resp = handshake(&h, json!(["fs:read", "run:python"])).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("json");
    let token = body["token"].as_str().expect("token string");
    assert_eq!(token.len(), 64, "token must be 32 bytes hex-encoded");
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(body["expires_in"], 3600);
}

#[tokio::test]
async fn version_requires_token() {
    let h = spawn().await;
    let resp = client()
        .get(format!("{}/launcher/version", h.base))
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["code"], "unauthorized");
}

#[tokio::test]
async fn version_with_valid_token() {
    let h = spawn().await;
    let hs: serde_json::Value = handshake(&h, json!(["fs:read"]))
        .await
        .json()
        .await
        .expect("json");
    let token = hs["token"].as_str().expect("token");
    let resp = client()
        .get(format!("{}/launcher/version", h.base))
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["proto"], 1);
}

#[tokio::test]
async fn version_with_unknown_token() {
    let h = spawn().await;
    let resp = client()
        .get(format!("{}/launcher/version", h.base))
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .header(
            "Authorization",
            "Bearer 0000000000000000000000000000000000000000000000000000000000000000",
        )
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn version_with_bad_host_after_auth() {
    let h = spawn().await;
    let hs: serde_json::Value = handshake(&h, json!(["fs:read"]))
        .await
        .json()
        .await
        .expect("json");
    let token = hs["token"].as_str().expect("token");
    let resp = client()
        .get(format!("{}/launcher/version", h.base))
        .header("Origin", ORIGIN)
        .header("Host", "evil.com")
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["code"], "host_not_allowed");
}
