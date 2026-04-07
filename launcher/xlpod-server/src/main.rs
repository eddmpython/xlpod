//! xlpod loopback HTTPS server (Phase 1.1a — `/health` only).
//!
//! Authoritative API spec: `proto/xlpod.openapi.yaml`.
//! Threat model: `docs/threat-model.md`.

mod bind;
mod routes;
mod tls;

use std::process::ExitCode;

use crate::bind::{addr_v4, LAUNCHER_VERSION, PORT, PROTO};

#[tokio::main]
async fn main() -> ExitCode {
    let paths = tls::TlsPaths::from_env_or_default();

    eprintln!("xlpod-server v{LAUNCHER_VERSION} (proto {PROTO})");
    eprintln!("  cert: {}", paths.cert.display());
    eprintln!("  key:  {}", paths.key.display());

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

    let app = routes::router();
    let addr = addr_v4();
    eprintln!("listening on https://{addr} (loopback only)");
    eprintln!("try: curl --cacert <mkcert-rootCA> https://127.0.0.1:{PORT}/health");

    if let Err(e) = axum_server::bind_rustls(addr, tls_config)
        .serve(app.into_make_service())
        .await
    {
        eprintln!("error: server exited: {e}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
