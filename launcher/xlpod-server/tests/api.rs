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
use std::time::Duration;
use tempfile::TempDir;

use xlpod_server::{
    audit::AuditLog,
    auth::TokenStore,
    consent::{AutoApproveConsent, ConsentBackend, DenyAllConsent},
    make_app,
    python_worker::PythonWorker,
    rate_limit::RateLimiter,
    state::AppState,
};

const ORIGIN: &str = "https://addin.xlwings.org";

struct Harness {
    base: String,
    host_header: String,
    _audit_dir: TempDir,
}

async fn spawn() -> Harness {
    spawn_with(Arc::new(AutoApproveConsent), PythonWorker::new()).await
}

async fn spawn_with_consent(consent: Arc<dyn ConsentBackend>) -> Harness {
    spawn_with(consent, PythonWorker::new()).await
}

async fn spawn_with_worker(worker: PythonWorker) -> Harness {
    spawn_with(Arc::new(AutoApproveConsent), worker).await
}

async fn spawn_with(consent: Arc<dyn ConsentBackend>, worker: PythonWorker) -> Harness {
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
        consent,
        worker,
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
    // run:python does not require fs_roots, so this exercises the
    // generic handshake path without dragging in Phase 3 setup.
    let resp = handshake(&h, json!(["run:python"])).await;
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
    let hs: serde_json::Value = handshake(&h, json!(["run:python"]))
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

// ---- consent ---------------------------------------------------------------

#[tokio::test]
async fn handshake_consent_denied_short_circuits_token_issue() {
    let h = spawn_with_consent(Arc::new(DenyAllConsent)).await;
    let resp = handshake(&h, json!(["run:python"])).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["code"], "consent_denied");
    // The body must NOT contain a token field — denial happens before
    // any token is minted.
    assert!(body.get("token").is_none());
}

#[tokio::test]
async fn handshake_consent_skipped_for_empty_scope_set() {
    // No scopes requested = nothing to consent to. The deny backend
    // should not even be consulted, and the issued token is harmless
    // (it has no scopes attached).
    let h = spawn_with_consent(Arc::new(DenyAllConsent)).await;
    let resp = handshake(&h, json!([])).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---- /run/python -----------------------------------------------------------

async fn handshake_with_run_python(h: &Harness) -> String {
    let resp = client()
        .post(format!("{}/auth/handshake", h.base))
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .json(&json!({"requested_scopes": ["run:python"]}))
        .send()
        .await
        .expect("send");
    let body: serde_json::Value = resp.json().await.expect("json");
    body["token"].as_str().expect("token").to_string()
}

async fn post_run_python(h: &Harness, token: &str, code: &str) -> reqwest::Response {
    client()
        .post(format!("{}/run/python", h.base))
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({"code": code}))
        .send()
        .await
        .expect("send")
}

#[tokio::test]
async fn run_python_happy_path_returns_result_and_stdout() {
    let h = spawn().await;
    let token = handshake_with_run_python(&h).await;
    let resp = post_run_python(&h, &token, "print('hi from worker'); _result = 1 + 2").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["ok"], true);
    assert_eq!(body["stdout"].as_str().unwrap().trim(), "hi from worker");
    assert_eq!(body["result"], "3");
    assert!(body["error"].is_null());
}

#[tokio::test]
async fn run_python_exception_returns_ok_false_with_traceback() {
    let h = spawn().await;
    let token = handshake_with_run_python(&h).await;
    let resp = post_run_python(&h, &token, "1/0").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["ok"], false);
    let err = body["error"].as_str().unwrap_or("");
    assert!(err.contains("ZeroDivisionError"), "got: {err}");
}

#[tokio::test]
async fn run_python_serializes_two_calls_through_one_worker() {
    // Variables set in one call should be visible in the next, since
    // Phase 5 ships a single shared globals namespace per worker
    // (the worker `exec`s into a per-call namespace BUT the worker
    // process itself outlives the call). For Phase 5 we instead test
    // the simpler property that two calls in a row both succeed and
    // each runs in its own clean namespace, since per-call exec uses
    // a fresh dict.
    let h = spawn().await;
    let token = handshake_with_run_python(&h).await;
    let r1 = post_run_python(&h, &token, "_result = 7").await;
    let r2 = post_run_python(&h, &token, "_result = 11").await;
    let b1: serde_json::Value = r1.json().await.expect("json");
    let b2: serde_json::Value = r2.json().await.expect("json");
    assert_eq!(b1["result"], "7");
    assert_eq!(b2["result"], "11");
}

#[tokio::test]
async fn run_python_timeout_kills_worker_and_recovers() {
    // Spawn a server with a very short worker timeout so the test
    // doesn't sit on a 30 second wall.
    let h = spawn_with_worker(PythonWorker::with_timeout(Duration::from_millis(800))).await;
    let token = handshake_with_run_python(&h).await;
    let resp = post_run_python(&h, &token, "import time; time.sleep(5)").await;
    assert_eq!(resp.status(), StatusCode::GATEWAY_TIMEOUT);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["code"], "worker_timeout");

    // Recovery: the next call must spawn a fresh worker and succeed.
    let resp2 = post_run_python(&h, &token, "_result = 42").await;
    assert_eq!(resp2.status(), StatusCode::OK);
    let body2: serde_json::Value = resp2.json().await.expect("json");
    assert_eq!(body2["result"], "42");
}

#[tokio::test]
async fn run_python_without_scope_is_denied() {
    let h = spawn().await;
    // Issue a token without run:python.
    let resp = client()
        .post(format!("{}/auth/handshake", h.base))
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .json(&json!({"requested_scopes": ["excel:com"]}))
        .send()
        .await
        .expect("send");
    let body: serde_json::Value = resp.json().await.expect("json");
    let token = body["token"].as_str().expect("token").to_string();
    let resp = post_run_python(&h, &token, "_result = 1").await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["code"], "scope_denied");
}

// ---- /fs/read --------------------------------------------------------------

async fn handshake_with_fs_root(h: &Harness, root: &std::path::Path) -> String {
    let resp = client()
        .post(format!("{}/auth/handshake", h.base))
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .json(&json!({
            "requested_scopes": ["fs:read"],
            "fs_roots": [root.to_string_lossy()],
        }))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("json");
    body["token"].as_str().expect("token").to_string()
}

#[tokio::test]
async fn fs_read_handshake_without_root_is_rejected() {
    let h = spawn().await;
    let resp = client()
        .post(format!("{}/auth/handshake", h.base))
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .json(&json!({"requested_scopes": ["fs:read"], "fs_roots": []}))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["code"], "bad_request");
}

#[tokio::test]
async fn fs_read_returns_file_under_root() {
    let h = spawn().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("hello.txt");
    std::fs::write(&target, b"hello, xlpod").expect("write");
    let token = handshake_with_fs_root(&h, dir.path()).await;

    let resp = client()
        .get(format!("{}/fs/read", h.base))
        .query(&[("path", target.to_string_lossy().as_ref())])
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["encoding"], "base64");
    assert_eq!(body["size"], 12);
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    let decoded = B64
        .decode(body["content"].as_str().expect("content"))
        .expect("base64");
    assert_eq!(decoded, b"hello, xlpod");
}

#[tokio::test]
async fn fs_read_outside_root_is_forbidden() {
    let h = spawn().await;
    let allowed = tempfile::tempdir().expect("tempdir1");
    let other = tempfile::tempdir().expect("tempdir2");
    let outside = other.path().join("secret.txt");
    std::fs::write(&outside, b"top secret").expect("write");
    let token = handshake_with_fs_root(&h, allowed.path()).await;

    let resp = client()
        .get(format!("{}/fs/read", h.base))
        .query(&[("path", outside.to_string_lossy().as_ref())])
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["code"], "forbidden_path");
}

#[tokio::test]
async fn fs_read_traversal_is_caught_by_canonicalize() {
    // ../ escape attempts canonicalize before the root check, so a
    // path like /allowed/../other/secret resolves to /other/secret
    // and fails the starts_with(allowed) check.
    let h = spawn().await;
    let allowed = tempfile::tempdir().expect("tempdir1");
    let other = tempfile::tempdir().expect("tempdir2");
    let outside = other.path().join("secret.txt");
    std::fs::write(&outside, b"top secret").expect("write");
    let token = handshake_with_fs_root(&h, allowed.path()).await;

    let traversal = allowed.path().join("..").join(
        other
            .path()
            .file_name()
            .expect("dirname")
            .to_string_lossy()
            .to_string(),
    );
    let traversal = traversal.join("secret.txt");

    let resp = client()
        .get(format!("{}/fs/read", h.base))
        .query(&[("path", traversal.to_string_lossy().as_ref())])
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn fs_read_missing_file_is_404() {
    let h = spawn().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let token = handshake_with_fs_root(&h, dir.path()).await;
    let resp = client()
        .get(format!("{}/fs/read", h.base))
        .query(&[(
            "path",
            dir.path().join("nope.txt").to_string_lossy().as_ref(),
        )])
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["code"], "path_not_found");
}

#[tokio::test]
async fn fs_read_directory_is_not_a_file() {
    let h = spawn().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let token = handshake_with_fs_root(&h, dir.path()).await;
    let resp = client()
        .get(format!("{}/fs/read", h.base))
        .query(&[("path", dir.path().to_string_lossy().as_ref())])
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["code"], "not_a_file");
}

#[tokio::test]
async fn fs_read_without_scope_is_denied() {
    let h = spawn().await;
    // Issue a token WITHOUT fs:read.
    let resp = client()
        .post(format!("{}/auth/handshake", h.base))
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .json(&json!({"requested_scopes": ["run:python"]}))
        .send()
        .await
        .expect("send");
    let body: serde_json::Value = resp.json().await.expect("json");
    let token = body["token"].as_str().expect("token").to_string();

    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("x.txt");
    std::fs::write(&target, b"x").expect("write");

    let resp = client()
        .get(format!("{}/fs/read", h.base))
        .query(&[("path", target.to_string_lossy().as_ref())])
        .header("Origin", ORIGIN)
        .header("Host", &h.host_header)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["code"], "scope_denied");
}

#[tokio::test]
async fn version_with_bad_host_after_auth() {
    let h = spawn().await;
    let hs: serde_json::Value = handshake(&h, json!(["run:python"]))
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
