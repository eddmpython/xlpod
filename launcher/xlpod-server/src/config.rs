//! Compile-time security configuration.
//!
//! Mirrors `info.x-xlpod-allowed-origins` and the loopback host constraints
//! declared in `proto/xlpod.openapi.yaml`. Phase 1.4 CI will diff these
//! values against the spec and fail the build on drift.

use crate::bind::PORT;

/// Origins authorized to call the launcher. Phase 0 measurement
/// (2026-04-07) confirmed `https://addin.xlwings.org` as the sole
/// xlwings Lite production origin. Wildcards are NOT honoured.
pub const ALLOWED_ORIGINS: &[&str] = &["https://addin.xlwings.org"];

/// Permitted `Host` header values. DNS rebinding defense:
/// the launcher rejects requests whose Host is anything other than the
/// literal loopback string for our bind.
pub fn allowed_hosts() -> [String; 2] {
    [format!("127.0.0.1:{PORT}"), format!("[::1]:{PORT}")]
}

/// Per-token rate limit. Mirrors `info.x-xlpod-rate-limit.per_token_rps`.
pub const RATE_LIMIT_PER_SEC: u32 = 100;

/// Maximum bytes the `/fs/read` route will return in a single response.
/// Phase 3 = 10 MiB. Larger files require a streaming follow-up route.
pub const FS_READ_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// Token lifetime. A new token is issued at every launcher start; this
/// caps how long a single token survives even if the launcher keeps
/// running. Phase 1.2 default; tray consent flow may shorten in Phase 1.3.
pub const TOKEN_TTL_SECS: u64 = 3600;

/// Default audit log path under `%LOCALAPPDATA%\xlpod\audit.log`.
/// Override with `XLPOD_AUDIT_PATH`.
pub fn default_audit_path() -> std::path::PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("xlpod").join("audit.log")
}
