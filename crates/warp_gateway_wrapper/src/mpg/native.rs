//! Native provider adapters: Anthropic Messages and Google Gemini.
//!
//! These convert an incoming OpenAI Chat Completions request into the
//! provider's native wire format, call upstream, and convert the response back
//! into Chat Completions shape. To keep the conversion robust, native adapters
//! request a non-streaming upstream response and return a single Chat
//! Completion (Warp renders it the same way). Streaming can be layered on later.

use std::sync::Arc;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use super::server::MpgState;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u64 = 4096;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Flatten a Chat `content` field (string or array of parts) into plain text.
fn content_to_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| part.as_str().map(str::to_string))
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Wrap assistant text into a non-streaming Chat Completion body.
fn chat_completion(text: &str, model: &str) -> Value {
    json!({
        "id": "chatcmpl-mpg",
        "object": "chat.completion",
        "created": 0,
        "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": text },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 }
    })
}

// ---------------------------------------------------------------------------
// Anthropic Messages
// ---------------------------------------------------------------------------

/// Convert a Chat Completions body into an Anthropic Messages request.
pub fn chat_to_anthropic(chat: &Value) -> Value {
    let model = chat
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("claude-3-5-sonnet-latest");
    let max_tokens = chat
        .get("max_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_TOKENS);

    let mut system = String::new();
    let mut messages = Vec::new();
    if let Some(items) = chat.get("messages").and_then(Value::as_array) {
        for message in items {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user");
            let text = message
                .get("content")
                .map(content_to_text)
                .unwrap_or_default();
            match role {
                "system" => {
                    if !system.is_empty() {
                        system.push_str("\n\n");
                    }
                    system.push_str(&text);
                }
                "assistant" => messages.push(json!({
                    "role": "assistant",
                    "content": [{ "type": "text", "text": text }]
                })),
                _ => messages.push(json!({
                    "role": "user",
                    "content": [{ "type": "text", "text": text }]
                })),
            }
        }
    }

    let mut out = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": messages,
    });
    if !system.is_empty() {
        out["system"] = json!(system);
    }
    if let Some(temp) = chat.get("temperature") {
        out["temperature"] = temp.clone();
    }
    out
}

/// Extract assistant text from an Anthropic Messages response.
pub fn anthropic_to_text(body: &Value) -> String {
    body.get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// Forward a Chat request to Anthropic `/messages` and convert the response.
pub async fn forward_anthropic(state: Arc<MpgState>, chat_body: Value) -> Response {
    let model = chat_body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("default")
        .to_string();
    let body = chat_to_anthropic(&chat_body);
    let url = format!(
        "{}/messages",
        state.config.provider.base_url.trim_end_matches('/')
    );

    let mut request = state
        .client
        .post(&url)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    if let Some(key) = state.config.provider.resolved_api_key() {
        request = request.header("x-api-key", key);
    }
    request = request.json(&body);

    match request.send().await {
        Ok(response) => {
            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                let code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                return (code, text).into_response();
            }
            match response.json::<Value>().await {
                Ok(value) => {
                    Json(chat_completion(&anthropic_to_text(&value), &model)).into_response()
                }
                Err(err) => {
                    (StatusCode::BAD_GATEWAY, format!("invalid JSON: {err}")).into_response()
                }
            }
        }
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            format!("upstream request failed: {err}"),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Google Gemini
// ---------------------------------------------------------------------------

/// Convert a Chat Completions body into a Gemini `generateContent` request.
pub fn chat_to_gemini(chat: &Value) -> Value {
    let mut system = String::new();
    let mut contents = Vec::new();
    if let Some(items) = chat.get("messages").and_then(Value::as_array) {
        for message in items {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user");
            let text = message
                .get("content")
                .map(content_to_text)
                .unwrap_or_default();
            match role {
                "system" => {
                    if !system.is_empty() {
                        system.push_str("\n\n");
                    }
                    system.push_str(&text);
                }
                "assistant" => contents.push(json!({
                    "role": "model",
                    "parts": [{ "text": text }]
                })),
                _ => contents.push(json!({
                    "role": "user",
                    "parts": [{ "text": text }]
                })),
            }
        }
    }

    let mut out = json!({ "contents": contents });
    if !system.is_empty() {
        out["systemInstruction"] = json!({ "parts": [{ "text": system }] });
    }
    let mut generation_config = json!({});
    if let Some(temp) = chat.get("temperature") {
        generation_config["temperature"] = temp.clone();
    }
    if let Some(max) = chat.get("max_tokens").and_then(Value::as_u64) {
        generation_config["maxOutputTokens"] = json!(max);
    }
    if generation_config
        .as_object()
        .map(|m| !m.is_empty())
        .unwrap_or(false)
    {
        out["generationConfig"] = generation_config;
    }
    out
}

/// Extract assistant text from a Gemini `generateContent` response.
pub fn gemini_to_text(body: &Value) -> String {
    body.get("candidates")
        .and_then(|candidates| candidates.get(0))
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// Forward a Chat request to Gemini `generateContent` and convert the response.
pub async fn forward_gemini(state: Arc<MpgState>, chat_body: Value) -> Response {
    let model = chat_body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("gemini-1.5-flash")
        .to_string();
    let body = chat_to_gemini(&chat_body);
    let base = state.config.provider.base_url.trim_end_matches('/');
    let key = state.config.provider.resolved_api_key().unwrap_or_default();
    let url = format!("{base}/models/{model}:generateContent?key={key}");

    match state
        .client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                let code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                return (code, text).into_response();
            }
            match response.json::<Value>().await {
                Ok(value) => Json(chat_completion(&gemini_to_text(&value), &model)).into_response(),
                Err(err) => {
                    (StatusCode::BAD_GATEWAY, format!("invalid JSON: {err}")).into_response()
                }
            }
        }
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            format!("upstream request failed: {err}"),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_hoists_system_and_maps_roles() {
        let chat = json!({
            "model": "claude-3-5-sonnet",
            "messages": [
                {"role": "system", "content": "be terse"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"}
            ],
            "max_tokens": 100
        });
        let req = chat_to_anthropic(&chat);
        assert_eq!(req["system"], "be terse");
        assert_eq!(req["max_tokens"], 100);
        assert_eq!(req["messages"][0]["role"], "user");
        assert_eq!(req["messages"][1]["role"], "assistant");
        assert_eq!(req["messages"][0]["content"][0]["text"], "hi");
    }

    #[test]
    fn anthropic_extracts_text_blocks() {
        let body = json!({
            "content": [
                {"type": "text", "text": "Hello "},
                {"type": "text", "text": "world"}
            ]
        });
        assert_eq!(anthropic_to_text(&body), "Hello world");
    }

    #[test]
    fn gemini_maps_assistant_to_model() {
        let chat = json!({
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "assistant", "content": "prior"},
                {"role": "user", "content": "now"}
            ]
        });
        let req = chat_to_gemini(&chat);
        assert_eq!(req["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(req["contents"][0]["role"], "model");
        assert_eq!(req["contents"][1]["role"], "user");
    }

    #[test]
    fn gemini_extracts_text() {
        let body = json!({
            "candidates": [{
                "content": { "parts": [{ "text": "answer" }] }
            }]
        });
        assert_eq!(gemini_to_text(&body), "answer");
    }
}
