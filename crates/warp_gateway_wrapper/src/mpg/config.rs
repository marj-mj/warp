//! Configuration for the Managed Provider Gateway (MPG).
//!
//! MPG is a local OpenAI-compatible server that lets a Warp client (configured
//! via a custom endpoint pointing at this gateway) forward Agent requests to
//! any upstream provider. Mirrors the design of the `warp-oss` Managed
//! Provider Gateway: a normalized Chat Completions front, with adapters that
//! convert to/from the upstream's actual wire format.

use serde::{Deserialize, Serialize};

/// What wire format the upstream speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WireApi {
    /// OpenAI Chat Completions (`/chat/completions`). Default.
    #[default]
    Chat,
    /// OpenAI Responses (`/responses`). Future Phase MPG-B.
    Responses,
}

impl WireApi {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Responses => "responses",
        }
    }
}

/// Adapter selected for an upstream. Determines the request/response shape
/// conversion. Currently only `openai_chat` is implemented in Phase A.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Adapter {
    /// Forward as OpenAI Chat Completions, no conversion.
    #[default]
    OpenaiChat,
    /// Convert Chat Completions <-> OpenAI Responses (future Phase B).
    OpenaiResponses,
    /// Fallback: pass through chat-compatible behaviour without conversion.
    BridgeOpenai,
    /// Convert Chat Completions -> Anthropic Messages.
    AnthropicMessages,
    /// Convert Chat Completions -> Gemini generateContent.
    GeminiGenerateContent,
}

impl Adapter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenaiChat => "openai_chat",
            Self::OpenaiResponses => "openai_responses",
            Self::BridgeOpenai => "bridge_openai",
            Self::AnthropicMessages => "anthropic_messages",
            Self::GeminiGenerateContent => "gemini_generate_content",
        }
    }

    pub fn compatibility_group(self) -> &'static str {
        match self {
            Self::OpenaiChat | Self::BridgeOpenai => "openai_compatible_chat",
            Self::OpenaiResponses => "openai_compatible_responses",
            Self::AnthropicMessages => "anthropic_messages",
            Self::GeminiGenerateContent => "gemini_generate_content",
        }
    }
}

/// One upstream provider entry. Modeled after the JSON preset used by the
/// `warp-oss` `script/provider_gateway.providers.json`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    /// Display name (e.g. "NROUTER").
    pub name: String,
    /// Upstream base URL (e.g. "https://api.example.com/v1"). Trailing slash optional.
    pub base_url: String,
    /// Default model to advertise via `/v1/models` and use when the request omits one.
    #[serde(default)]
    pub model: Option<String>,
    /// Upstream wire format.
    #[serde(default)]
    pub wire_api: WireApi,
    /// Adapter selection.
    #[serde(default)]
    pub adapter: Adapter,
    /// Optional API key. If `env_key` is also set, the env value wins.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Name of an environment variable to read the API key from.
    #[serde(default)]
    pub env_key: Option<String>,
}

impl ProviderConfig {
    /// Resolve the actual API key, preferring the env var if set.
    pub fn resolved_api_key(&self) -> Option<String> {
        if let Some(env_name) = &self.env_key {
            if let Ok(value) = std::env::var(env_name) {
                if !value.trim().is_empty() {
                    return Some(value);
                }
            }
        }
        self.api_key.as_ref().and_then(|key| {
            let trimmed = key.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    }
}

/// Top-level MPG runtime config.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub provider: ProviderConfig,
    /// Optional Bearer token clients must present to use the gateway. Empty means open access.
    pub auth_token: String,
    /// When true, strip `tools`, `tool_choice`, `parallel_tool_calls` before forwarding.
    pub disable_tools: bool,
    /// When true, log/respect the disable_mcp flag (gateway has no MCP itself; this is informational).
    pub disable_mcp: bool,
    /// Sliding window for the duplicate-request guard (seconds). 0 disables.
    pub duplicate_window_secs: u64,
    /// Optional model_config_key the gateway should report as "forced" in /healthz.
    pub force_model_config_key: Option<String>,
}

impl GatewayConfig {
    pub fn new(provider: ProviderConfig) -> Self {
        Self {
            provider,
            auth_token: String::new(),
            disable_tools: true,
            disable_mcp: true,
            duplicate_window_secs: 60,
            force_model_config_key: None,
        }
    }

    /// Read overlay knobs from environment variables. Mirrors the
    /// `WARP_MANAGED_PROVIDER_GATEWAY_*` names used by `warp-oss`.
    pub fn apply_env_overrides(&mut self) {
        if let Ok(value) = std::env::var("WARP_MANAGED_PROVIDER_GATEWAY_AUTH_TOKEN") {
            self.auth_token = value;
        }
        self.disable_tools =
            env_bool("WARP_MANAGED_PROVIDER_GATEWAY_DISABLE_TOOLS", self.disable_tools);
        self.disable_mcp =
            env_bool("WARP_MANAGED_PROVIDER_GATEWAY_DISABLE_MCP", self.disable_mcp);
        if let Ok(value) = std::env::var("WARP_MANAGED_PROVIDER_GATEWAY_DUPLICATE_REQUEST_WINDOW_SECS") {
            if let Ok(parsed) = value.parse() {
                self.duplicate_window_secs = parsed;
            }
        }
        if let Ok(value) = std::env::var("WARP_MANAGED_PROVIDER_GATEWAY_FORCE_MODEL_CONFIG_KEY") {
            if !value.trim().is_empty() {
                self.force_model_config_key = Some(value);
            }
        }

        // Upstream overrides win over JSON preset values.
        if let Ok(value) = std::env::var("WARP_MANAGED_PROVIDER_UPSTREAM_BASE_URL") {
            if !value.trim().is_empty() {
                self.provider.base_url = value;
            }
        }
        if let Ok(value) = std::env::var("WARP_MANAGED_PROVIDER_UPSTREAM_API_KEY") {
            if !value.trim().is_empty() {
                self.provider.api_key = Some(value);
                self.provider.env_key = None;
            }
        }
        if let Ok(value) = std::env::var("WARP_MANAGED_PROVIDER_UPSTREAM_WIRE_API") {
            self.provider.wire_api = match value.to_ascii_lowercase().as_str() {
                "responses" => WireApi::Responses,
                _ => WireApi::Chat,
            };
        }
        if let Ok(value) = std::env::var("WARP_MANAGED_PROVIDER_UPSTREAM_ADAPTER") {
            self.provider.adapter = match value.as_str() {
                "openai_responses" => Adapter::OpenaiResponses,
                "bridge_openai" => Adapter::BridgeOpenai,
                "anthropic_messages" => Adapter::AnthropicMessages,
                "gemini_generate_content" => Adapter::GeminiGenerateContent,
                _ => Adapter::OpenaiChat,
            };
        }
    }
}

/// JSON shape for `provider_gateway.providers.json` style files.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProvidersFile {
    pub providers: Vec<ProviderConfig>,
}

impl ProvidersFile {
    /// Load from disk and find a provider by `name` (case-insensitive).
    pub fn pick(path: &std::path::Path, name: &str) -> Result<ProviderConfig, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let parsed: ProvidersFile = serde_json::from_str(&raw)
            .map_err(|err| format!("invalid providers JSON: {err}"))?;
        let lowered = name.to_ascii_lowercase();
        parsed
            .providers
            .into_iter()
            .find(|provider| provider.name.eq_ignore_ascii_case(&lowered))
            .ok_or_else(|| format!("provider '{name}' not found in {}", path.display()))
    }
}

fn env_bool(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_resolves_api_key_from_env() {
        std::env::set_var("MPG_TEST_KEY", "abc");
        let provider = ProviderConfig {
            name: "p".into(),
            base_url: "https://x".into(),
            model: None,
            wire_api: WireApi::Chat,
            adapter: Adapter::OpenaiChat,
            api_key: None,
            env_key: Some("MPG_TEST_KEY".into()),
        };
        assert_eq!(provider.resolved_api_key().as_deref(), Some("abc"));
        std::env::remove_var("MPG_TEST_KEY");
    }

    #[test]
    fn env_overrides_replace_upstream() {
        let mut config = GatewayConfig::new(ProviderConfig {
            name: "p".into(),
            base_url: "https://orig".into(),
            model: None,
            wire_api: WireApi::Chat,
            adapter: Adapter::OpenaiChat,
            api_key: None,
            env_key: None,
        });
        std::env::set_var("WARP_MANAGED_PROVIDER_UPSTREAM_BASE_URL", "https://new");
        std::env::set_var("WARP_MANAGED_PROVIDER_UPSTREAM_WIRE_API", "responses");
        std::env::set_var("WARP_MANAGED_PROVIDER_UPSTREAM_ADAPTER", "openai_responses");
        config.apply_env_overrides();
        std::env::remove_var("WARP_MANAGED_PROVIDER_UPSTREAM_BASE_URL");
        std::env::remove_var("WARP_MANAGED_PROVIDER_UPSTREAM_WIRE_API");
        std::env::remove_var("WARP_MANAGED_PROVIDER_UPSTREAM_ADAPTER");
        assert_eq!(config.provider.base_url, "https://new");
        assert_eq!(config.provider.wire_api, WireApi::Responses);
        assert_eq!(config.provider.adapter, Adapter::OpenaiResponses);
    }

    #[test]
    fn adapter_groups_correctly() {
        assert_eq!(Adapter::OpenaiChat.compatibility_group(), "openai_compatible_chat");
        assert_eq!(
            Adapter::OpenaiResponses.compatibility_group(),
            "openai_compatible_responses"
        );
    }
}
