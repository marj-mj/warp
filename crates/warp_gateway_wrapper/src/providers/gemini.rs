//! Google Gemini `generateContent` provider.
//!
//! Talks to `{base_url}/models/{model}:generateContent?key=...`. Tools are sent as
//! `functionDeclarations`; the model replies with `functionCall` parts and the
//! gateway feeds results back as `functionResponse` parts. The gateway's flat
//! [`ChatMessage`] history is translated into Gemini `contents` (with the system
//! prompt placed in `systemInstruction`).

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::protocol::{GatewayError, ToolDefinition};

use super::{AssistantTurn, ChatMessage, CompletionRequest, LlmProvider, Role, ToolCall};

const DEFAULT_MODEL: &str = "gemini-1.5-flash";
const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

pub struct GeminiProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl GeminiProvider {
    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        let base_url = if base_url.trim().is_empty() {
            DEFAULT_BASE_URL.to_string()
        } else {
            base_url.trim_end_matches('/').to_string()
        };
        Self {
            api_key: api_key.into(),
            base_url,
            client: reqwest::Client::new(),
        }
    }

    /// Build (systemInstruction, contents) from the conversation.
    fn encode(messages: &[ChatMessage]) -> (Option<Value>, Vec<Value>) {
        let mut system: Option<String> = None;
        let mut contents: Vec<Value> = Vec::new();

        for message in messages {
            match message.role {
                Role::System => {
                    system = Some(match system.take() {
                        Some(existing) => format!("{existing}\n\n{}", message.content),
                        None => message.content.clone(),
                    });
                }
                Role::User => {
                    contents.push(json!({
                        "role": "user",
                        "parts": [{ "text": message.content }],
                    }));
                }
                Role::Assistant => {
                    let mut parts: Vec<Value> = Vec::new();
                    if !message.content.is_empty() {
                        parts.push(json!({ "text": message.content }));
                    }
                    for call in &message.tool_calls {
                        parts.push(json!({
                            "functionCall": {
                                "name": call.name,
                                "args": call.arguments,
                            }
                        }));
                    }
                    if parts.is_empty() {
                        parts.push(json!({ "text": "" }));
                    }
                    contents.push(json!({ "role": "model", "parts": parts }));
                }
                Role::Tool => {
                    // Gemini expects function results parsed back into a structured
                    // response object; fall back to wrapping raw text if needed.
                    let response_value = serde_json::from_str::<Value>(&message.content)
                        .unwrap_or_else(|_| json!({ "result": message.content }));
                    contents.push(json!({
                        "role": "user",
                        "parts": [{
                            "functionResponse": {
                                "name": message.tool_call_id.clone().unwrap_or_default(),
                                "response": response_value,
                            }
                        }],
                    }));
                }
            }
        }

        let system_instruction = system.map(|text| json!({ "parts": [{ "text": text }] }));
        (system_instruction, contents)
    }

    fn encode_tools(tools: &[ToolDefinition]) -> Value {
        let declarations: Vec<Value> = tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters_schema,
                })
            })
            .collect();
        json!([{ "functionDeclarations": declarations }])
    }

    fn parse_response(body: &Value) -> Result<AssistantTurn, GatewayError> {
        let parts = body
            .get("candidates")
            .and_then(|candidates| candidates.get(0))
            .and_then(|candidate| candidate.get("content"))
            .and_then(|content| content.get("parts"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                GatewayError::ExecutionFailed(
                    "Gemini response missing candidates[0].content.parts".to_string(),
                )
            })?;

        let mut text = String::new();
        let mut tool_calls = Vec::new();
        let mut call_index = 0usize;

        for part in parts {
            if let Some(value) = part.get("text").and_then(Value::as_str) {
                text.push_str(value);
            }
            if let Some(function_call) = part.get("functionCall") {
                let name = function_call
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let arguments = function_call.get("args").cloned().unwrap_or_else(|| json!({}));
                // Gemini calls have no id; synthesize a stable one keyed on name.
                let id = format!("{name}-{call_index}");
                call_index += 1;
                tool_calls.push(ToolCall { id, name, arguments });
            }
        }

        Ok(AssistantTurn {
            content: text,
            tool_calls,
        })
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn name(&self) -> &str {
        "google"
    }

    async fn step(&self, request: &CompletionRequest) -> Result<AssistantTurn, GatewayError> {
        let model = request.model.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let (system_instruction, contents) = Self::encode(&request.messages);

        let mut payload = json!({ "contents": contents });
        if let Some(system_instruction) = system_instruction {
            payload["systemInstruction"] = system_instruction;
        }
        if !request.tools.is_empty() {
            payload["tools"] = Self::encode_tools(&request.tools);
        }

        let mut generation_config = json!({});
        if let Some(temperature) = request.temperature {
            generation_config["temperature"] = json!(temperature);
        }
        if let Some(max_tokens) = request.max_tokens {
            generation_config["maxOutputTokens"] = json!(max_tokens);
        }
        if generation_config.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
            payload["generationConfig"] = generation_config;
        }

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, model, self.api_key
        );
        let response = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|err| GatewayError::ExecutionFailed(format!("request to Gemini failed: {err}")))?;

        let status = response.status();
        let body_text = response
            .text()
            .await
            .map_err(|err| GatewayError::ExecutionFailed(format!("failed to read response body: {err}")))?;

        if !status.is_success() {
            return Err(GatewayError::ExecutionFailed(format!(
                "Gemini API returned status {status}: {body_text}"
            )));
        }

        let body: Value = serde_json::from_str(&body_text).map_err(|err| {
            GatewayError::SerializationError(format!("invalid JSON in Gemini response: {err}"))
        })?;

        Self::parse_response(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_default_base_url_when_empty() {
        let provider = GeminiProvider::new("k", "");
        assert_eq!(provider.base_url, DEFAULT_BASE_URL);
    }

    #[test]
    fn encodes_system_instruction_and_contents() {
        let messages = vec![
            ChatMessage::system("be concise"),
            ChatMessage::user("hi"),
        ];
        let (system, contents) = GeminiProvider::encode(&messages);
        assert_eq!(system.unwrap()["parts"][0]["text"], "be concise");
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
    }

    #[test]
    fn maps_assistant_role_to_model() {
        let messages = vec![ChatMessage::assistant("hello")];
        let (_system, contents) = GeminiProvider::encode(&messages);
        assert_eq!(contents[0]["role"], "model");
    }

    #[test]
    fn parses_function_call() {
        let body = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "text": "calling" },
                        { "functionCall": { "name": "echo", "args": {"message": "hi"} } }
                    ]
                }
            }]
        });
        let turn = GeminiProvider::parse_response(&body).unwrap();
        assert_eq!(turn.content, "calling");
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "echo");
        assert_eq!(turn.tool_calls[0].arguments["message"], "hi");
    }

    #[test]
    fn parses_final_text() {
        let body = json!({
            "candidates": [{ "content": { "parts": [{ "text": "done" }] } }]
        });
        let turn = GeminiProvider::parse_response(&body).unwrap();
        assert!(turn.is_final());
        assert_eq!(turn.content, "done");
    }
}
