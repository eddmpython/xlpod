//! TLS configuration loading.
//!
//! Phase 1.1a: read a PEM cert + key from disk (the mkcert-issued
//! `.certs/probe-cert.pem` from Phase 0). Phase 1.3 will replace this with
//! an embedded local CA that issues a fresh server cert at every launcher
//! start.
//!
//! Plain HTTP is **never** supported. xlwings Lite enforces
//! `upgrade-insecure-requests` (Phase 0 measurement), and the entire reason
//! we exist is to be reachable from there. A plain-HTTP code path would be
//! unreachable from the only legitimate caller and is intentionally absent.

use std::path::PathBuf;

use axum_server::tls_rustls::RustlsConfig;

#[derive(Debug, Clone)]
pub struct TlsPaths {
    pub cert: PathBuf,
    pub key: PathBuf,
}

impl TlsPaths {
    /// Resolve from environment, falling back to the repo's `.certs/`
    /// directory used by the Phase 0 probe.
    pub fn from_env_or_default() -> Self {
        let cert = std::env::var_os("XLPOD_TLS_CERT")
            .map(PathBuf::from)
            .unwrap_or_else(|| repo_default("probe-cert.pem"));
        let key = std::env::var_os("XLPOD_TLS_KEY")
            .map(PathBuf::from)
            .unwrap_or_else(|| repo_default("probe-key.pem"));
        Self { cert, key }
    }
}

fn repo_default(name: &str) -> PathBuf {
    // launcher/xlpod-server/src/ -> launcher/xlpod-server -> launcher -> repo
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_default();
    repo_root.join(".certs").join(name)
}

pub async fn load(paths: &TlsPaths) -> Result<RustlsConfig, std::io::Error> {
    if !paths.cert.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("TLS cert not found at {}", paths.cert.display()),
        ));
    }
    if !paths.key.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("TLS key not found at {}", paths.key.display()),
        ));
    }
    RustlsConfig::from_pem_file(&paths.cert, &paths.key).await
}
