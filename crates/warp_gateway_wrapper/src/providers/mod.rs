//! LLM provider abstraction layer.
//!
//! A provider turns a conversation (system/user/assistant/tool messages) plus a
//! set of available tools into the next assistant turn. The agent loop in
//! `gateway::session` drives providers: it executes any requested tool calls,
//! feeds the results back, and repeats until the provider returns a turn with
//! no tool calls (the final answer).

pub mod anthropic;
pub mod gemini;
pub mod mock;
pub mod openai;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::protocol::{GatewayError, ToolDefinition};

pub use anthropic::AnthropicProvider;
pub use gemini::GeminiProvider;
pub use mock::MockProvider;
pub use openai::OpenAiProvider;

/// Role of a message in a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A single tool call requested by the assistant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    /// Stable identifier used to correlate the call with its result.
    pub id: String,
    /// Name of the tool to invoke.
    pub name: String,
    /// JSON arguments for the tool.
    pub arguments: Value,
}

/// A message in the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Tool calls requested by an assistant message (empty otherwise).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// For `Role::Tool` messages, the id of the call this result answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self::text(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::text(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::text(Role::Assistant, content)
    }

    pub fn text(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// Build an assistant message that requests tool calls.
    pub fn assistant_with_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
        }
    }

    /// Build a tool-result message answering a specific call.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// A request for the next assistant turn.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

/// The assistant turn returned by a provider.
#[derive(Debug, Clone)]
pub struct AssistantTurn {
    /// Natural-language content (may be empty when only tool calls are returned).
    pub content: String,
    /// Tool calls requested by the assistant; empty means this is the final turn.
    pub tool_calls: Vec<ToolCall>,
}

impl AssistantTurn {
    pub fn is_final(&self) -> bool {
        self.tool_calls.is_empty()
    }
}

/// Abstraction over an LLM backend.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Human-readable provider name, e.g. "openai" or "mock".
    fn name(&self) -> &str;

    /// Produce the next assistant turn for the given conversation.
    async fn step(&self, request: &CompletionRequest) -> Result<AssistantTurn, GatewayError>;
}

/// High-level provider families inferred from a model identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFamily {
    OpenAi,
    Anthropic,
    Google,
    Custom,
}

impl ProviderFamily {
    pub fn from_model(model: Option<&str>) -> Self {
        let Some(model) = model else {
            return Self::OpenAi;
        };
        let normalized = model.to_ascii_lowercase();
        if normalized.contains("claude") {
            Self::Anthropic
        } else if normalized.contains("gemini") {
            Self::Google
        } else if normalized.contains("gpt")
            || normalized.starts_with("o1")
            || normalized.starts_with("o3")
            || normalized.starts_with("o4")
        {
            Self::OpenAi
        } else {
            Self::Custom
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Google => "google",
            Self::Custom => "custom",
        }
    }
}

/// Connection settings used to construct a live provider.
#[derive(Debug, Clone, Default)]
pub struct ProviderSettings {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

impl ProviderSettings {
    /// Read provider settings from the process environment.
    ///
    /// Recognizes, in order of preference, a managed-gateway token/URL and the
    /// standard `OPENAI_API_KEY` / `OPENAI_BASE_URL` variables.
    pub fn from_env(family: ProviderFamily) -> Self {
        let mut settings = Self::default();

        // Managed gateway (OpenAI-compatible) takes precedence when present.
        if let Ok(token) = std::env::var("MANAGED_GATEWAY_TOKEN") {
            if !token.trim().is_empty() {
                settings.api_key = Some(token);
            }
        }
        if let Ok(url) = std::env::var("MANAGED_GATEWAY_URL") {
            if !url.trim().is_empty() {
                settings.base_url = Some(url);
            }
        }

        let (key_var, url_var) = match family {
            ProviderFamily::Anthropic => ("ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL"),
            ProviderFamily::Google => ("GEMINI_API_KEY", "GEMINI_BASE_URL"),
            _ => ("OPENAI_API_KEY", "OPENAI_BASE_URL"),
        };

        if settings.api_key.is_none() {
            if let Ok(key) = std::env::var(key_var) {
                if !key.trim().is_empty() {
                    settings.api_key = Some(key);
                }
            }
        }
        if settings.base_url.is_none() {
            if let Ok(url) = std::env::var(url_var) {
                if !url.trim().is_empty() {
                    settings.base_url = Some(url);
                }
            }
        }

        settings
    }
}

/// Resolve a concrete provider for a model.
///
/// When the relevant API key is configured an OpenAI-compatible live provider is
/// returned; otherwise a deterministic offline [`MockProvider`] is used so the
/// gateway remains functional without network access or credentials.
pub fn resolve_provider(model: Option<&str>, settings: ProviderSettings) -> Arc<dyn LlmProvider> {
    let family = ProviderFamily::from_model(model);

    let Some(api_key) = settings.api_key.clone() else {
        return Arc::new(MockProvider::new());
    };

    // An explicit base_url implies an OpenAI-compatible gateway regardless of the
    // model family, so it takes precedence over native provider selection.
    if settings.base_url.is_some()
        && matches!(family, ProviderFamily::OpenAi | ProviderFamily::Custom)
    {
        let base_url = settings.base_url.clone().unwrap();
        return Arc::new(OpenAiProvider::new(api_key, base_url));
    }

    match family {
        ProviderFamily::Anthropic => {
            let base_url = settings
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());
            Arc::new(AnthropicProvider::new(api_key, base_url))
        }
        ProviderFamily::Google => {
            let base_url = settings.base_url.clone().unwrap_or_default();
            Arc::new(GeminiProvider::new(api_key, base_url))
        }
        ProviderFamily::OpenAi | ProviderFamily::Custom => {
            let base_url = settings
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            Arc::new(OpenAiProvider::new(api_key, base_url))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_inference() {
        assert_eq!(
            ProviderFamily::from_model(Some("gpt-4.1")),
            ProviderFamily::OpenAi
        );
        assert_eq!(
            ProviderFamily::from_model(Some("claude-3-5-sonnet")),
            ProviderFamily::Anthropic
        );
        assert_eq!(
            ProviderFamily::from_model(Some("gemini-1.5-pro")),
            ProviderFamily::Google
        );
        assert_eq!(
            ProviderFamily::from_model(Some("llama-3")),
            ProviderFamily::Custom
        );
        assert_eq!(ProviderFamily::from_model(None), ProviderFamily::OpenAi);
    }

    #[test]
    fn resolves_to_mock_without_key() {
        let provider = resolve_provider(Some("gpt-4.1"), ProviderSettings::default());
        assert_eq!(provider.name(), "mock");
    }

    #[test]
    fn resolves_to_openai_with_key() {
        let settings = ProviderSettings {
            api_key: Some("sk-test".to_string()),
            base_url: None,
        };
        let provider = resolve_provider(Some("gpt-4.1"), settings);
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn resolves_to_anthropic_with_key() {
        let settings = ProviderSettings {
            api_key: Some("sk-ant".to_string()),
            base_url: None,
        };
        let provider = resolve_provider(Some("claude-3-5-sonnet"), settings);
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn resolves_to_gemini_with_key() {
        let settings = ProviderSettings {
            api_key: Some("g-key".to_string()),
            base_url: None,
        };
        let provider = resolve_provider(Some("gemini-1.5-pro"), settings);
        assert_eq!(provider.name(), "google");
    }

    #[test]
    fn explicit_base_url_uses_openai_compatible_for_custom_model() {
        let settings = ProviderSettings {
            api_key: Some("token".to_string()),
            base_url: Some("http://127.0.0.1:8787/v1".to_string()),
        };
        let provider = resolve_provider(Some("llama-3"), settings);
        assert_eq!(provider.name(), "openai");
    }
}
