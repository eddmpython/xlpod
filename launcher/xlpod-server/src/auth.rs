//! Tokens and scopes.
//!
//! Mirrors `proto/xlpod.openapi.yaml#/components/schemas/Scope` and the
//! `bearerAuth` security scheme. Tokens are 256-bit OS CSPRNG values,
//! hex-encoded (64 chars). They live in process memory only — never
//! written to disk, never logged in full. The audit log records only
//! `token_id` (the first 8 chars of the hex), which is enough for
//! correlation but not for impersonation.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::RwLock,
    time::{Duration, Instant},
};

use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::{config::TOKEN_TTL_SECS, error::ApiError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    #[serde(rename = "fs:read")]
    FsRead,
    #[serde(rename = "fs:write")]
    FsWrite,
    #[serde(rename = "run:python")]
    RunPython,
    #[serde(rename = "excel:com")]
    ExcelCom,
    // Reserved for Phase 6+ (see plan §7.5). Currently rejected by the
    // launcher: any handshake that requests one of these returns
    // `reserved_scope`.
    #[serde(rename = "ai:provider:call")]
    AiProviderCall,
    #[serde(rename = "ai:codegen:write")]
    AiCodegenWrite,
    #[serde(rename = "ai:exec:python")]
    AiExecPython,
    #[serde(rename = "bundle:read")]
    BundleRead,
    #[serde(rename = "bundle:write")]
    BundleWrite,
}

impl Scope {
    /// Phase 8 activated all three `ai:*` scopes. The launcher no
    /// longer rejects them at handshake; the consent dialog branches
    /// on `ConsentKind::AiHandshake` instead. Kept as a function so a
    /// future scope can be re-marked reserved without re-plumbing.
    #[allow(dead_code)]
    pub const fn is_reserved(self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // `created` consumed by future expiry/audit code
pub struct TokenRecord {
    pub scopes: Vec<Scope>,
    /// Canonicalized filesystem roots the token is allowed to read
    /// from. Empty unless the handshake requested `fs:read` and
    /// supplied at least one valid root.
    pub fs_roots: Vec<PathBuf>,
    pub created: Instant,
    pub expires: Instant,
}

#[derive(Debug, Default)]
pub struct TokenStore {
    inner: RwLock<HashMap<String, TokenRecord>>,
}

impl TokenStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Generate a new token bound to a (sanitized) scope set and an
    /// already-canonicalized fs root list. Reserved scopes are rejected
    /// before reaching this function.
    pub fn issue(&self, scopes: Vec<Scope>, fs_roots: Vec<PathBuf>) -> (String, TokenRecord) {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        let now = Instant::now();
        let record = TokenRecord {
            scopes,
            fs_roots,
            created: now,
            expires: now + Duration::from_secs(TOKEN_TTL_SECS),
        };
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.insert(token.clone(), record.clone());
        (token, record)
    }

    /// Look up a token, refusing if it has expired. Returns a *clone* of
    /// the record so the caller does not hold the lock across an await.
    pub fn lookup(&self, token: &str) -> Result<TokenRecord, ApiError> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let rec = guard.get(token).ok_or(ApiError::Unauthorized)?;
        if Instant::now() >= rec.expires {
            return Err(ApiError::Unauthorized);
        }
        Ok(rec.clone())
    }

    /// First 8 hex chars of a token, suitable for audit log correlation.
    pub fn id_of(token: &str) -> String {
        token.chars().take(8).collect()
    }
}
