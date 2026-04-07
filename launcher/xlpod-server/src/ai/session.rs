//! AI session store.
//!
//! A `Session` ties a UUID to:
//!   - the provider + model the user picked at session-open time
//!   - the *internal bearer* — a real `TokenStore` token whose
//!     scopes are the intersection of (the calling user's scopes)
//!     and (the AI tools the user chose to expose at open time)
//!   - the running message history
//!   - the timestamp it was opened
//!
//! Sessions live in process memory only — never persisted to disk.
//! On launcher restart they're gone, exactly like bearer tokens.
//! Phase 10 will serialize the *messages* into the workbook custom
//! XML part so a workbook can carry the conversation across launcher
//! restarts; the internal bearer is never serialized (it's always
//! re-minted on the next session-open).

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::RwLock,
    time::{SystemTime, UNIX_EPOCH},
};

use uuid::Uuid;

use crate::ai::types::ChatMessage;
use crate::auth::Scope;
use crate::error::ApiError;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: Uuid,
    pub provider: String,
    pub model: String,
    pub internal_bearer: String,
    pub granted_scopes: Vec<Scope>,
    /// Inherited from the calling user token — Phase 8 dispatch uses
    /// these directly when the AI calls fs_read. The intersection
    /// rule still holds: the AI cannot read paths outside the user's
    /// approved roots.
    pub fs_roots: Vec<PathBuf>,
    pub messages: Vec<ChatMessage>,
    pub opened_ms: u128,
}

impl Session {
    pub fn fs_roots_for_dispatch(&self) -> Vec<PathBuf> {
        self.fs_roots.clone()
    }
}

#[derive(Debug, Default)]
pub struct SessionStore {
    inner: RwLock<HashMap<Uuid, Session>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(
        &self,
        provider: String,
        model: String,
        internal_bearer: String,
        granted_scopes: Vec<Scope>,
        fs_roots: Vec<PathBuf>,
    ) -> Session {
        let id = Uuid::new_v4();
        let session = Session {
            id,
            provider,
            model,
            internal_bearer,
            granted_scopes,
            fs_roots,
            messages: Vec::new(),
            opened_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        };
        let mut g = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.insert(id, session.clone());
        session
    }

    pub fn get(&self, id: Uuid) -> Result<Session, ApiError> {
        let g = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.get(&id).cloned().ok_or(ApiError::AiSessionNotFound)
    }

    pub fn append_messages(
        &self,
        id: Uuid,
        new_messages: Vec<ChatMessage>,
    ) -> Result<(), ApiError> {
        let mut g = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let session = g.get_mut(&id).ok_or(ApiError::AiSessionNotFound)?;
        session.messages.extend(new_messages);
        Ok(())
    }
}
