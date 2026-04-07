//! Live Anthropic Messages API implementation of [`Provider`].
//!
//! This is a thin hand-rolled HTTP client — we deliberately do not
//! pull in `anthropic-sdk` because (a) the dependency surface is
//! large for a single endpoint and (b) keeping the wire types in
//! one ~150-line file makes it trivial to update when Anthropic
//! changes the API shape.
//!
//! Phase 8 implements the non-streaming `POST
//! https://api.anthropic.com/v1/messages` endpoint with `tool_use`
//! blocks. SSE streaming, prompt caching, and citations are all
//! Phase 9+.
//!
//! The API key is fetched from the keychain on every call (cheap —
//! `CredReadW` is microseconds), so a key rotation through
//! `DELETE /ai/providers/key` + `POST /ai/providers/key` takes
//! effect on the very next request without restarting the launcher.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::ai::keychain::Keychain;
use crate::ai::provider::{Provider, ProviderError, ProviderTurn};
use crate::ai::types::{ChatMessage, ContentBlock, Role, StopReason, ToolSpec, Usage};

pub const DEFAULT_MODEL: &str = "claude-opus-4-6";
pub const ANTHROPIC_API_VERSION: &str = "2023-06-01";
pub const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";

pub struct AnthropicProvider {
    keychain: Arc<dyn Keychain>,
    http: reqwest::Client,
    model: String,
    api_url: String,
}

impl AnthropicProvider {
    pub fn new(keychain: Arc<dyn Keychain>) -> Self {
        Self {
            keychain,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            model: DEFAULT_MODEL.to_string(),
            api_url: ANTHROPIC_MESSAGES_URL.to_string(),
        }
    }

    /// Test seam: override the upstream URL (point at a local mock).
    #[allow(dead_code)]
    pub fn with_api_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &'static str {
        "anthropic"
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        max_tokens: Option<u32>,
    ) -> Result<ProviderTurn, ProviderError> {
        let key = self
            .keychain
            .read("anthropic_api_key")
            .map_err(|_| ProviderError::Unconfigured)?
            .ok_or(ProviderError::Unconfigured)?;

        let body = WireRequest::from_internal(messages, tools, &self.model, max_tokens);

        let resp = self
            .http
            .post(&self.api_url)
            .header("x-api-key", &key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Upstream(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Upstream(format!(
                "{status}: {}",
                truncate_for_log(&body_text)
            )));
        }

        let parsed: WireResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Decode(e.to_string()))?;

        Ok(parsed.into_internal())
    }
}

fn truncate_for_log(s: &str) -> String {
    const MAX: usize = 400;
    if s.len() > MAX {
        format!("{}…", &s[..MAX])
    } else {
        s.to_string()
    }
}

// ---- wire types (Anthropic Messages API v1) -------------------------------

#[derive(Debug, Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
}

impl<'a> WireRequest<'a> {
    fn from_internal(
        messages: &'a [ChatMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        max_tokens: Option<u32>,
    ) -> Self {
        Self {
            model,
            max_tokens: max_tokens.unwrap_or(1024),
            messages: messages.iter().map(WireMessage::from_internal).collect(),
            tools: tools.iter().map(WireTool::from_internal).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: Vec<WireContent<'a>>,
}

impl<'a> WireMessage<'a> {
    fn from_internal(msg: &'a ChatMessage) -> Self {
        let role = match msg.role {
            Role::System => "user", // Anthropic uses system at top level; demote inline
            Role::User | Role::Tool => "user",
            Role::Assistant => "assistant",
        };
        Self {
            role,
            content: msg.content.iter().map(WireContent::from_internal).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum WireContent<'a> {
    #[serde(rename = "text")]
    Text { text: &'a str },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: &'a str,
        name: &'a str,
        input: &'a serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: &'a str,
        content: String,
    },
}

impl<'a> WireContent<'a> {
    fn from_internal(b: &'a ContentBlock) -> Self {
        match b {
            ContentBlock::Text { text } => WireContent::Text { text },
            ContentBlock::ToolUse { id, name, input } => WireContent::ToolUse { id, name, input },
            ContentBlock::ToolResult {
                tool_use_id,
                output,
                ..
            } => WireContent::ToolResult {
                tool_use_id,
                content: output.to_string(),
            },
        }
    }
}

#[derive(Debug, Serialize)]
struct WireTool<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a serde_json::Value,
}

impl<'a> WireTool<'a> {
    fn from_internal(t: &'a ToolSpec) -> Self {
        Self {
            name: &t.name,
            description: &t.description,
            input_schema: &t.input_schema,
        }
    }
}

#[derive(Debug, Deserialize)]
struct WireResponse {
    #[serde(default)]
    content: Vec<WireRespBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireRespBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Default, Deserialize)]
struct WireUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

impl WireResponse {
    fn into_internal(self) -> ProviderTurn {
        let content: Vec<ContentBlock> = self
            .content
            .into_iter()
            .map(|b| match b {
                WireRespBlock::Text { text } => ContentBlock::Text { text },
                WireRespBlock::ToolUse { id, name, input } => {
                    ContentBlock::ToolUse { id, name, input }
                }
            })
            .collect();
        let stop_reason = match self.stop_reason.as_deref() {
            Some("max_tokens") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };
        let usage = self
            .usage
            .map(|u| Usage {
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                cached_input_tokens: None,
            })
            .unwrap_or_default();
        ProviderTurn {
            message: ChatMessage {
                role: Role::Assistant,
                ts_ms: None,
                content,
            },
            stop_reason,
            usage,
        }
    }
}
