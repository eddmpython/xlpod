//! Append-only JSONL audit log.
//!
//! Every request that reaches the launcher (including 4xx rejections) is
//! recorded. Sensitive fields (full bearer tokens, AI provider keys) MUST
//! NOT be written here. We log only `token_id` (first 8 hex chars).
//!
//! Format: one JSON object per line, UTF-8, LF terminator. Stable enough
//! for `jq`/`grep` and forward-compatible (consumers must ignore unknown
//! fields).

use std::{
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tokio::{
    fs::{File, OpenOptions},
    io::AsyncWriteExt,
    sync::Mutex,
};

#[derive(Debug, Serialize)]
pub struct AuditEntry<'a> {
    /// Unix epoch milliseconds.
    pub ts_ms: u128,
    /// `"user"` for human-driven calls; `"ai:<provider>:<model>"` once
    /// Phase 6 lands. Phase 1 always emits `"user"`.
    pub actor: &'a str,
    /// First 8 chars of the bearer token, or `null` for unauthenticated
    /// routes (`/health`, `/auth/handshake`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
    pub method: &'a str,
    pub path: &'a str,
    pub status: u16,
    pub origin: Option<&'a str>,
    pub host: Option<&'a str>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone)]
pub struct AuditLog {
    file: Arc<Mutex<File>>,
    path: PathBuf,
}

impl AuditLog {
    pub async fn open(path: PathBuf) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            path,
        })
    }

    #[allow(dead_code)] // surfaced via tray status in Phase 1.1b
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub async fn write(&self, entry: &AuditEntry<'_>) {
        let Ok(mut line) = serde_json::to_vec(entry) else {
            // Serialization of a small fixed struct cannot fail in
            // practice; if it does we drop the entry rather than panic.
            return;
        };
        line.push(b'\n');
        let mut guard = self.file.lock().await;
        // Best-effort: a failure to write the audit log MUST NOT cascade
        // into the request handling. We swallow IO errors here and rely
        // on a future health check to surface them.
        let _ = guard.write_all(&line).await;
        let _ = guard.flush().await;
    }
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
