//! xlpod loopback HTTPS server — library entry point.
//!
//! The same crate produces a `xlpod-server` binary (for direct dev use) and
//! a library that the future tray launcher and the integration tests
//! consume. The binary stays a thin shell over `serve()`; everything
//! testable lives here.

pub mod ai;
pub mod audit;
pub mod auth;
pub mod bind;
#[cfg(windows)]
pub mod ca;
pub mod config;
pub mod consent;
pub mod error;
pub mod fs_read;
pub mod middleware;
pub mod python_worker;
pub mod rate_limit;
pub mod routes;
pub mod state;
pub mod tls;

pub use routes::router as make_app;

use std::{path::PathBuf, sync::Arc};

use crate::ai::cost::{CostLedger, DEFAULT_DAILY_BUDGET_MICROS};
use crate::ai::keychain::{InMemoryKeychain, Keychain};
use crate::ai::{anthropic::AnthropicProvider, provider::ProviderRegistry, AiState};
use crate::consent::{AutoApproveConsent, ConsentBackend};

/// Inputs to `serve()` — kept as a struct so the binary and the future
/// tray launcher can both build it from their own argument parsers.
#[derive(Clone)]
pub struct ServeOptions {
    pub tls: tls::TlsPaths,
    pub audit_path: PathBuf,
    pub consent: Arc<dyn ConsentBackend>,
    /// Keychain backend for AI provider keys. Production wires this
    /// to `WindowsCredentialKeychain` on Windows; tests inject
    /// `InMemoryKeychain`. Default for the standalone dev binary is
    /// `InMemoryKeychain` so manual runs without the keychain UAC
    /// prompt still work — keys set during a single run are dropped
    /// at exit.
    pub keychain: Arc<dyn Keychain>,
    /// Path to the AI cost ledger JSONL file. Default lives next to
    /// the audit log under `%LOCALAPPDATA%/xlpod/cost.jsonl`.
    pub cost_path: PathBuf,
    /// Daily AI spend cap in micro-USD ($5/day default). Set to 0
    /// to disable. The launcher reads `XLPOD_DAILY_BUDGET_MICROS`
    /// from env and falls back to this.
    pub daily_budget_micros: u64,
}

impl ServeOptions {
    /// Default options for the standalone `xlpod-server` dev binary:
    /// reads TLS material from env or `.certs/`, audit log under
    /// `%LOCALAPPDATA%`, and `AutoApproveConsent` so manual smoke
    /// tests do not block on a dialog. The tray launcher constructs
    /// `ServeOptions` directly with `MessageBoxConsent` and the
    /// real Windows keychain instead.
    pub fn from_env() -> Self {
        let audit_path = std::env::var_os("XLPOD_AUDIT_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(config::default_audit_path);
        let cost_path = std::env::var_os("XLPOD_COST_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                audit_path
                    .parent()
                    .map(|p| p.join("cost.jsonl"))
                    .unwrap_or_else(|| PathBuf::from("cost.jsonl"))
            });
        let daily_budget_micros = std::env::var("XLPOD_DAILY_BUDGET_MICROS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_DAILY_BUDGET_MICROS);
        Self {
            tls: tls::TlsPaths::from_env_or_default(),
            audit_path,
            consent: Arc::new(AutoApproveConsent),
            keychain: Arc::new(InMemoryKeychain::new()),
            cost_path,
            daily_budget_micros,
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
    // Phase 8 added `reqwest` as a non-dev dependency for the
    // Anthropic provider; by default it pulls in rustls' `ring`
    // backend while `axum-server` uses `aws-lc-rs`. Both backends
    // visible to a single rustls crate instance is a process-wide
    // panic at handshake time. Pin one explicitly here, before any
    // TLS work happens. This is best-effort: if a previous call
    // already installed a provider in the same process the result
    // is `Err(())`, which we deliberately ignore.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let tls_config = tls::load(&opts.tls).await.map_err(ServeError::Tls)?;
    let audit_log = audit::AuditLog::open(opts.audit_path)
        .await
        .map_err(ServeError::Audit)?;
    let mut providers = ProviderRegistry::new();
    providers.register(Arc::new(AnthropicProvider::new(opts.keychain.clone())));
    let cost_ledger = CostLedger::open(opts.cost_path.clone(), opts.daily_budget_micros)
        .await
        .map_err(ServeError::Audit)?;
    let ai = AiState::new(
        Arc::new(providers),
        opts.keychain.clone(),
        opts.consent.clone(),
        cost_ledger,
    );
    let state = state::AppState {
        tokens: Arc::new(auth::TokenStore::new()),
        limiter: Arc::new(rate_limit::RateLimiter::new()),
        audit: audit_log,
        allowed_hosts: Arc::new(config::allowed_hosts().to_vec()),
        consent: opts.consent,
        worker: python_worker::PythonWorker::new(),
        ai,
    };
    let app = make_app(state);
    let addr = bind::addr_v4();
    axum_server::bind_rustls(addr, tls_config)
        .serve(app.into_make_service())
        .await
        .map_err(ServeError::Bind)
}
