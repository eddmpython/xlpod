//! xlpod-server binary entry point.
//!
//! Thin shell over `xlpod_server::make_app`. The launcher GUI binary in
//! Phase 1.1b will reuse the same library entry point and add a tray.
//!
//! Authoritative API spec: `proto/xlpod.openapi.yaml`.
//! Threat model: `docs/threat-model.md`.

use std::{process::ExitCode, sync::Arc};

use xlpod_server::{
    audit::AuditLog,
    auth::TokenStore,
    bind::{addr_v4, LAUNCHER_VERSION, PROTO},
    config, make_app,
    rate_limit::RateLimiter,
    state::AppState,
    tls,
};

#[tokio::main]
async fn main() -> ExitCode {
    eprintln!("xlpod-server v{LAUNCHER_VERSION} (proto {PROTO})");

    // Phase 1.3 status: rcgen-based local CA generation lives in
    // `xlpod_server::ca` and is verified end-to-end through the cert
    // material it produces, but the silent Win32 trust install path
    // (CertAddEncodedCertificateToStore) triggers a Windows confirmation
    // dialog on first run. Wiring that into a tray-driven user-consent
    // flow is Phase 1.4. Until then we keep using the mkcert-issued
    // material under `.certs/`, which the user installed once during
    // Phase 0. Operators can override with XLPOD_TLS_CERT/KEY.
    let paths = tls::TlsPaths::from_env_or_default();

    let audit_path = std::env::var_os("XLPOD_AUDIT_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(config::default_audit_path);

    eprintln!("  cert:  {}", paths.cert.display());
    eprintln!("  key:   {}", paths.key.display());
    eprintln!("  audit: {}", audit_path.display());

    let tls_config = match tls::load(&paths).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to load TLS material: {e}");
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
        allowed_hosts: Arc::new(config::allowed_hosts().to_vec()),
    };

    let app = make_app(state);
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
