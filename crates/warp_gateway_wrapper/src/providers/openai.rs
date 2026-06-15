//! OpenAI-compatible chat completions provider.
//!
//! Works against the OpenAI API or any OpenAI-compatible endpoint (including a
//! managed gateway) by POSTing to `{base_url}/chat/completions`. Tool calling
//! follows the OpenAI `tools` / `tool_calls` schema.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::protocol::{GatewayError, ToolDefinition};

use super::{AssistantTurn, ChatMessage, CompletionRequest, LlmProvider, Role, ToolCall};

const DEFAULT_MODEL: &str = "gpt-4o-mini";

pub struct OpenAiProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        Self {
            api_key: api_key.into(),
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    fn role_str(role: Role) -> &'static str {
        match role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }

    fn encode_messages(messages: &[ChatMessage]) -> Vec<Value> {
        messages
            .iter()
            .map(|message| {
                let mut obj = json!({
                    "role": Self::role_str(message.role),
                    "content": message.content,
                });

                if !message.tool_calls.is_empty() {
                    let calls: Vec<Value> = message
                        .tool_calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": call.arguments.to_string(),
                                }
                            })
                        })
                        .collect();
                    obj["tool_calls"] = Value::Array(calls);
                }

                if let Some(tool_call_id) = &message.tool_call_id {
                    obj["tool_call_id"] = json!(tool_call_id);
                }

                obj
            })
            .collect()
    }

    fn encode_tools(tools: &[ToolDefinition]) -> Vec<Value> {
        tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters_schema,
                    }
                })
            })
            .collect()
    }

    fn parse_response(body: &Value) -> Result<AssistantTurn, GatewayError> {
        let message = body
            .get("choices")
            .and_then(|choices| choices.get(0))
            .and_then(|choice| choice.get("message"))
            .ok_or_else(|| {
                GatewayError::ExecutionFailed(
                    "OpenAI response missing choices[0].message".to_string(),
                )
            })?;

        let content = message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let mut tool_calls = Vec::new();
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
            for (index, call) in calls.iter().enumerate() {
                let function = call.get("function");
                let name = function
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();

                let raw_args = function
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let arguments =
                    serde_json::from_str::<Value>(raw_args).unwrap_or_else(|_| json!({}));

                let id = call
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("call-{index}"));

                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
        }

        Ok(AssistantTurn {
            content,
            tool_calls,
        })
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn step(&self, request: &CompletionRequest) -> Result<AssistantTurn, GatewayError> {
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());

        let mut payload = json!({
            "model": model,
            "messages": Self::encode_messages(&request.messages),
        });

        if !request.tools.is_empty() {
            payload["tools"] = Value::Array(Self::encode_tools(&request.tools));
            payload["tool_choice"] = json!("auto");
        }
        if let Some(temperature) = request.temperature {
            payload["temperature"] = json!(temperature);
        }
        if let Some(max_tokens) = request.max_tokens {
            payload["max_tokens"] = json!(max_tokens);
        }

        let url = format!("{}/chat/completions", self.base_url);
        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
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
                "OpenAI API returned status {status}: {body_text}"
            )));
        }

        let body: Value = serde_json::from_str(&body_text).map_err(|err| {
            GatewayError::SerializationError(format!("invalid JSON in OpenAI response: {err}"))
        })?;

        Self::parse_response(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_trailing_slash_in_base_url() {
        let provider = OpenAiProvider::new("sk", "https://example.com/v1/");
        assert_eq!(provider.base_url, "https://example.com/v1");
    }

    #[test]
    fn parses_plain_text_response() {
        let body = json!({
            "choices": [{
                "message": { "role": "assistant", "content": "hello there" }
            }]
        });
        let turn = OpenAiProvider::parse_response(&body).unwrap();
        assert_eq!(turn.content, "hello there");
        assert!(turn.is_final());
    }

    #[test]
    fn parses_tool_call_response() {
        let body = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": {
                            "name": "echo",
                            "arguments": "{\"message\":\"hi\"}"
                        }
                    }]
                }
            }]
        });
        let turn = OpenAiProvider::parse_response(&body).unwrap();
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].id, "call_abc");
        assert_eq!(turn.tool_calls[0].name, "echo");
        assert_eq!(turn.tool_calls[0].arguments["message"], "hi");
    }

    #[test]
    fn errors_on_missing_choices() {
        let body = json!({ "error": "nope" });
        assert!(OpenAiProvider::parse_response(&body).is_err());
    }
}
