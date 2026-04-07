//! xlpod loopback HTTPS server.
//!
//! Phase 1.2: 5-check security stack + token issuance + WebSocket.
//! Authoritative API spec: `proto/xlpod.openapi.yaml`.
//! Threat model: `docs/threat-model.md`.

mod audit;
mod auth;
mod bind;
mod config;
mod error;
mod middleware;
mod rate_limit;
mod routes;
mod state;
mod tls;

use std::{process::ExitCode, sync::Arc};

use crate::{
    audit::AuditLog,
    auth::TokenStore,
    bind::{addr_v4, LAUNCHER_VERSION, PROTO},
    rate_limit::RateLimiter,
    state::AppState,
};

#[tokio::main]
async fn main() -> ExitCode {
    let paths = tls::TlsPaths::from_env_or_default();
    let audit_path = std::env::var_os("XLPOD_AUDIT_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(config::default_audit_path);

    eprintln!("xlpod-server v{LAUNCHER_VERSION} (proto {PROTO})");
    eprintln!("  cert:  {}", paths.cert.display());
    eprintln!("  key:   {}", paths.key.display());
    eprintln!("  audit: {}", audit_path.display());

    let tls_config = match tls::load(&paths).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to load TLS material: {e}");
            eprintln!(
                "hint: set XLPOD_TLS_CERT/XLPOD_TLS_KEY, or run the Phase 0 \
                 mkcert step to populate .certs/"
            );
            return ExitCode::from(2);
        }
    };

    let audit = match AuditLog::open(audit_path).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: failed to open audit log: {e}");
            return ExitCode::from(3);
        }
    };

    let state = AppState {
        tokens: Arc::new(TokenStore::new()),
        limiter: Arc::new(RateLimiter::new()),
        audit,
    };

    let app = routes::router(state);
    let addr = addr_v4();
    eprintln!("listening on https://{addr} (loopback only)");

    if let Err(e) = axum_server::bind_rustls(addr, tls_config)
        .serve(app.into_make_service())
        .await
    {
        eprintln!("error: server exited: {e}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
