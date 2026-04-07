//! Provider trait + ProviderRegistry + FakeProvider for tests.
//!
//! A `Provider` is the network-facing relay to a remote AI service.
//! Phase 8 ships [`crate::ai::anthropic::AnthropicProvider`] as the
//! single live impl, plus [`FakeProvider`] for deterministic tests.
//! OpenAI, Ollama, and others are explicitly out of scope for the
//! Phase 7–13 arc (see plan).
//!
//! The trait is intentionally narrow: one `chat` async method that
//! takes the current message history + the tool list and returns
//! either an `assistant` message containing text and/or tool_use
//! blocks, or an error. The launcher loops on this method (running
//! tool calls between turns) until the model emits a non-tool-use
//! response.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde::Serialize;

use crate::ai::types::{ChatMessage, StopReason, ToolSpec, Usage};

#[derive(Debug)]
pub enum ProviderError {
    Unconfigured,
    Upstream(String),
    Decode(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Unconfigured => write!(f, "no API key for provider"),
            ProviderError::Upstream(s) => write!(f, "upstream provider error: {s}"),
            ProviderError::Decode(s) => write!(f, "could not decode provider response: {s}"),
        }
    }
}

impl std::error::Error for ProviderError {}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderTurn {
    pub message: ChatMessage,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

#[async_trait]
pub trait Provider: Send + Sync + 'static {
    fn id(&self) -> &'static str;

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        max_tokens: Option<u32>,
    ) -> Result<ProviderTurn, ProviderError>;
}

#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn Provider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, provider: Arc<dyn Provider>) {
        self.providers.insert(provider.id().to_string(), provider);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(id).cloned()
    }
}

// ---------- FakeProvider for tests -----------------------------------------

/// A scriptable provider used by integration tests. Each call to
/// `chat` pops the next pre-recorded `ProviderTurn` from a queue.
/// Panics if the queue is empty (the test was wrong about the
/// number of turns).
pub struct FakeProvider {
    queue: tokio::sync::Mutex<Vec<ProviderTurn>>,
}

impl FakeProvider {
    pub fn new(turns: Vec<ProviderTurn>) -> Self {
        // Reverse so pop() gives us the first turn.
        let mut q = turns;
        q.reverse();
        Self {
            queue: tokio::sync::Mutex::new(q),
        }
    }
}

#[async_trait]
impl Provider for FakeProvider {
    fn id(&self) -> &'static str {
        "fake"
    }

    async fn chat(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
        _max_tokens: Option<u32>,
    ) -> Result<ProviderTurn, ProviderError> {
        let mut q = self.queue.lock().await;
        q.pop()
            .ok_or_else(|| ProviderError::Upstream("FakeProvider queue exhausted".into()))
    }
}
