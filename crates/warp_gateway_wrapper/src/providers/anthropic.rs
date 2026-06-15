//! Anthropic Messages API provider.
//!
//! Talks to `{base_url}/messages` using the Anthropic native schema. Tool calling
//! uses Anthropic `tool_use` content blocks on output and `tool_result` blocks on
//! input. The gateway's flat [`ChatMessage`] history is translated into Anthropic
//! `messages` (with the system prompt hoisted into the top-level `system` field).

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::protocol::{GatewayError, ToolDefinition};

use super::{AssistantTurn, ChatMessage, CompletionRequest, LlmProvider, Role, ToolCall};

const DEFAULT_MODEL: &str = "claude-3-5-sonnet-latest";
const DEFAULT_MAX_TOKENS: u32 = 4096;
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        Self {
            api_key: api_key.into(),
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Split the conversation into the top-level system prompt and the message
    /// array Anthropic expects. Tool-call/result messages become content blocks.
    fn encode(messages: &[ChatMessage]) -> (Option<String>, Vec<Value>) {
        let mut system = None;
        let mut out: Vec<Value> = Vec::new();

        for message in messages {
            match message.role {
                Role::System => {
                    // Concatenate multiple system messages.
                    system = Some(match system.take() {
                        Some(existing) => format!("{existing}\n\n{}", message.content),
                        None => message.content.clone(),
                    });
                }
                Role::User => {
                    out.push(json!({
                        "role": "user",
                        "content": [{ "type": "text", "text": message.content }],
                    }));
                }
                Role::Assistant => {
                    let mut blocks: Vec<Value> = Vec::new();
                    if !message.content.is_empty() {
                        blocks.push(json!({ "type": "text", "text": message.content }));
                    }
                    for call in &message.tool_calls {
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": call.id,
                            "name": call.name,
                            "input": call.arguments,
                        }));
                    }
                    if blocks.is_empty() {
                        blocks.push(json!({ "type": "text", "text": "" }));
                    }
                    out.push(json!({ "role": "assistant", "content": blocks }));
                }
                Role::Tool => {
                    // Anthropic models tool results as user-role tool_result blocks.
                    let tool_use_id = message.tool_call_id.clone().unwrap_or_default();
                    out.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": message.content,
                        }],
                    }));
                }
            }
        }

        (system, out)
    }

    fn encode_tools(tools: &[ToolDefinition]) -> Vec<Value> {
        tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters_schema,
                })
            })
            .collect()
    }

    fn parse_response(body: &Value) -> Result<AssistantTurn, GatewayError> {
        let content = body
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                GatewayError::ExecutionFailed(
                    "Anthropic response missing content array".to_string(),
                )
            })?;

        let mut text = String::new();
        let mut tool_calls = Vec::new();

        for block in content {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(value) = block.get("text").and_then(Value::as_str) {
                        text.push_str(value);
                    }
                }
                Some("tool_use") => {
                    let id = block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let arguments = block.get("input").cloned().unwrap_or_else(|| json!({}));
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
                _ => {}
            }
        }

        Ok(AssistantTurn {
            content: text,
            tool_calls,
        })
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn step(&self, request: &CompletionRequest) -> Result<AssistantTurn, GatewayError> {
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let (system, messages) = Self::encode(&request.messages);

        let mut payload = json!({
            "model": model,
            "max_tokens": request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            "messages": messages,
        });

        if let Some(system) = system {
            payload["system"] = json!(system);
        }
        if !request.tools.is_empty() {
            payload["tools"] = Value::Array(Self::encode_tools(&request.tools));
        }
        if let Some(temperature) = request.temperature {
            payload["temperature"] = json!(temperature);
        }

        let url = format!("{}/messages", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&payload)
            .send()
            .await
            .map_err(|err| {
                GatewayError::ExecutionFailed(format!("request to {url} failed: {err}"))
            })?;

        let status = response.status();
        let body_text = response.text().await.map_err(|err| {
            GatewayError::ExecutionFailed(format!("failed to read response body: {err}"))
        })?;

        if !status.is_success() {
            return Err(GatewayError::ExecutionFailed(format!(
                "Anthropic API returned status {status}: {body_text}"
            )));
        }

        let body: Value = serde_json::from_str(&body_text).map_err(|err| {
            GatewayError::SerializationError(format!("invalid JSON in Anthropic response: {err}"))
        })?;

        Self::parse_response(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hoists_system_prompt() {
        let messages = vec![ChatMessage::system("be helpful"), ChatMessage::user("hi")];
        let (system, encoded) = AnthropicProvider::encode(&messages);
        assert_eq!(system.as_deref(), Some("be helpful"));
        assert_eq!(encoded.len(), 1);
        assert_eq!(encoded[0]["role"], "user");
    }

    #[test]
    fn encodes_tool_result_as_user_block() {
        let messages = vec![ChatMessage::tool_result("call_1", "{\"ok\":true}")];
        let (_system, encoded) = AnthropicProvider::encode(&messages);
        assert_eq!(encoded[0]["role"], "user");
        assert_eq!(encoded[0]["content"][0]["type"], "tool_result");
        assert_eq!(encoded[0]["content"][0]["tool_use_id"], "call_1");
    }

    #[test]
    fn parses_text_and_tool_use() {
        let body = json!({
            "content": [
                { "type": "text", "text": "let me check" },
                { "type": "tool_use", "id": "tu_1", "name": "echo", "input": {"message": "hi"} }
            ]
        });
        let turn = AnthropicProvider::parse_response(&body).unwrap();
        assert_eq!(turn.content, "let me check");
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].id, "tu_1");
        assert_eq!(turn.tool_calls[0].name, "echo");
        assert_eq!(turn.tool_calls[0].arguments["message"], "hi");
    }

    #[test]
    fn parses_final_text_turn() {
        let body = json!({ "content": [{ "type": "text", "text": "done" }] });
        let turn = AnthropicProvider::parse_response(&body).unwrap();
        assert!(turn.is_final());
        assert_eq!(turn.content, "done");
    }
}
