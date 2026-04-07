//! xlpod loopback HTTPS server — library entry point.
//!
//! The same crate produces a `xlpod-server` binary (for direct dev use) and
//! a library that the future tray launcher and the integration tests
//! consume. The binary stays a thin shell over `serve()`; everything
//! testable lives here.

pub mod audit;
pub mod auth;
pub mod bind;
#[cfg(windows)]
pub mod ca;
pub mod config;
pub mod error;
pub mod middleware;
pub mod rate_limit;
pub mod routes;
pub mod state;
pub mod tls;

pub use routes::router as make_app;

use std::{path::PathBuf, sync::Arc};

/// Inputs to `serve()` — kept as a struct so the binary and the future
/// tray launcher can both build it from their own argument parsers.
#[derive(Debug, Clone)]
pub struct ServeOptions {
    pub tls: tls::TlsPaths,
    pub audit_path: PathBuf,
}

impl ServeOptions {
    pub fn from_env() -> Self {
        Self {
            tls: tls::TlsPaths::from_env_or_default(),
            audit_path: std::env::var_os("XLPOD_AUDIT_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(config::default_audit_path),
        }
    }
}

#[derive(Debug)]
pub enum ServeError {
    Tls(std::io::Error),
    Audit(std::io::Error),
    Bind(std::io::Error),
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServeError::Tls(e) => write!(f, "tls: {e}"),
            ServeError::Audit(e) => write!(f, "audit: {e}"),
            ServeError::Bind(e) => write!(f, "bind: {e}"),
        }
    }
}

impl std::error::Error for ServeError {}

/// Single source of truth for spinning up the loopback HTTPS server.
/// Both the standalone `xlpod-server` binary and the tray-hosted
/// `xlpod-launcher` call this function.
pub async fn serve(opts: ServeOptions) -> Result<(), ServeError> {
    let tls_config = tls::load(&opts.tls).await.map_err(ServeError::Tls)?;
    let audit_log = audit::AuditLog::open(opts.audit_path)
        .await
        .map_err(ServeError::Audit)?;
    let state = state::AppState {
        tokens: Arc::new(auth::TokenStore::new()),
        limiter: Arc::new(rate_limit::RateLimiter::new()),
        audit: audit_log,
        allowed_hosts: Arc::new(config::allowed_hosts().to_vec()),
    };
    let app = make_app(state);
    let addr = bind::addr_v4();
    axum_server::bind_rustls(addr, tls_config)
        .serve(app.into_make_service())
        .await
        .map_err(ServeError::Bind)
}
