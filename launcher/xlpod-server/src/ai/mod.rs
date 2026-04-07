//! AI bridge — Phase 8.
//!
//! This module relays chat completion requests to a remote AI
//! provider, dispatches the model's tool calls back through the
//! launcher's own routes (so the existing 5-check security stack
//! runs on every AI-driven action), and stores transcripts in a
//! session store keyed by UUID.
//!
//! Design summary (full plan in `~/.claude/plans/stateless-sparking-newt.md`
//! Phase 8):
//!
//! - **Provider trait** ([`provider::Provider`]) abstracts the
//!   network call. Phase 8 ships only [`anthropic::AnthropicProvider`];
//!   tests inject [`provider::FakeProvider`].
//! - **Tool registry** ([`tools::builtin_tools`]) is a static list
//!   that exposes existing routes (`/fs/read`, `/excel/range/read`,
//!   `/run/python`, ...) as MCP-style tools. The model receives the
//!   JSON Schema for each tool's input and emits `tool_use` blocks
//!   the launcher executes.
//! - **Dispatch** ([`dispatch::execute_tool_use`]) translates a
//!   `tool_use` into a normal `ToolResult` by calling the matching
//!   route through the same router an HTTP client would hit. The
//!   internal bearer is minted at session-open time and carries the
//!   *intersection* of the user's scopes and the tools the user
//!   chose to expose; the model cannot escalate.
//! - **Sessions** ([`session::SessionStore`]) hold message history,
//!   the internal bearer id, and the consent context.
//! - **Keychain** ([`keychain::Keychain`]) stores API keys outside
//!   the audit log; Windows Credential Manager via `windows-sys`
//!   on production, an in-memory fake in tests.
//! - **Consent** is enforced per mutating tool call by reusing the
//!   existing [`crate::consent::ConsentBackend`] machinery; Phase 8
//!   asks every time, Phase 9 adds trust windows.

pub mod anthropic;
pub mod dispatch;
pub mod keychain;
pub mod provider;
pub mod session;
pub mod tools;
pub mod types;

use std::sync::Arc;

use crate::consent::ConsentBackend;

#[derive(Clone)]
pub struct AiState {
    pub providers: Arc<provider::ProviderRegistry>,
    pub sessions: Arc<session::SessionStore>,
    pub keychain: Arc<dyn keychain::Keychain>,
    pub consent: Arc<dyn ConsentBackend>,
}

impl AiState {
    pub fn new(
        providers: Arc<provider::ProviderRegistry>,
        keychain: Arc<dyn keychain::Keychain>,
        consent: Arc<dyn ConsentBackend>,
    ) -> Self {
        Self {
            providers,
            sessions: Arc::new(session::SessionStore::new()),
            keychain,
            consent,
        }
    }
}
