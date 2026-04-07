//! Trust window — "approve N tools for M minutes" so the model
//! can run a multi-step plan without prompting on every call.
//!
//! Phase 9 implements the in-memory store and the open / lookup /
//! revoke methods. The dispatch path consults
//! `TrustWindowStore::covers` before falling back to a per-call
//! consent dialog. Auto-revoke triggers (session close, token
//! expiry, upstream 5xx) are wired in `routes.rs::ai_chat`.

use std::{
    collections::HashMap,
    sync::RwLock,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct TrustWindow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub tools: Vec<String>,
    pub expires_ms: u128,
}

impl TrustWindow {
    pub fn covers(&self, tool: &str) -> bool {
        let now = now_ms();
        if now >= self.expires_ms {
            return false;
        }
        self.tools.iter().any(|t| t == tool)
    }
}

#[derive(Debug, Default)]
pub struct TrustWindowStore {
    inner: RwLock<HashMap<Uuid, TrustWindow>>,
}

/// Hard upper bound on duration_secs accepted by the route.
pub const MAX_DURATION_SECS: u64 = 3600;

impl TrustWindowStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&self, session_id: Uuid, tools: Vec<String>, duration_secs: u64) -> TrustWindow {
        let bounded = duration_secs.min(MAX_DURATION_SECS);
        let id = Uuid::new_v4();
        let win = TrustWindow {
            id,
            session_id,
            tools,
            expires_ms: now_ms() + (bounded as u128) * 1000,
        };
        let mut g = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.insert(id, win.clone());
        win
    }

    pub fn covers(&self, session_id: Uuid, tool: &str) -> bool {
        let g = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.values()
            .any(|w| w.session_id == session_id && w.covers(tool))
    }

    /// Auto-revoke every window for a session. Used on session
    /// close, token expiry, and on any upstream 5xx (defense
    /// against hijacked sessions).
    pub fn revoke_session(&self, session_id: Uuid) {
        let mut g = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.retain(|_, w| w.session_id != session_id);
    }

    #[allow(dead_code)]
    pub fn revoke(&self, id: Uuid) {
        let mut g = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.remove(&id);
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
