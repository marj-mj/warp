//! `openai_responses` adapter.
//!
//! Converts an incoming OpenAI Chat Completions request into an OpenAI
//! Responses (`/responses`) request, forwards it upstream, then converts the
//! Responses output back into the Chat Completions shape Warp expects.
//!
//! Streaming responses are translated from Responses SSE events into Chat
//! Completion `chat.completion.chunk` SSE frames. Text is extracted via a
//! fallback chain so providers that only emit text at the `done`/`completed`
//! stage still surface content:
//!   response.output_text.delta
//!     -> response.output_text.done
//!     -> response.content_part.done
//!     -> response.completed (response.output[*])

use serde_json::{json, Value};

use std::sync::Arc;

use axum::{
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;

use super::server::MpgState;

/// Forward a (already safe-mode-processed) Chat Completions body to the upstream
/// Responses API and convert the result back to Chat Completions shape.
pub async fn forward_responses(state: Arc<MpgState>, chat_body: Value) -> Response {
    let stream_requested = chat_body
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let model = chat_body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("default")
        .to_string();

    let responses_body = chat_to_responses_request(&chat_body);
    let url = format!(
        "{}/responses",
        state.config.provider.base_url.trim_end_matches('/')
    );

    let mut request = state.client.post(&url);
    if let Some(key) = state.config.provider.resolved_api_key() {
        request = request.bearer_auth(key);
    }
    request = request
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&responses_body);

    let upstream = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            tracing::warn!(error = %err, url = %url, "mpg responses upstream request failed");
            return (
                StatusCode::BAD_GATEWAY,
                format!("upstream request failed: {err}"),
            )
                .into_response();
        }
    };

    let status = upstream.status();
    if !status.is_success() {
        let body = upstream.text().await.unwrap_or_default();
        tracing::warn!(status = status.as_u16(), "mpg responses upstream error");
        let code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        return (code, body).into_response();
    }

    if stream_requested {
        stream_responses_to_chat(upstream, model).await
    } else {
        match upstream.json::<Value>().await {
            Ok(body) => {
                let text = extract_text_from_responses_json(&body);
                Json(responses_json_to_chat(&text, &model)).into_response()
            }
            Err(err) => (
                StatusCode::BAD_GATEWAY,
                format!("failed to read upstream JSON: {err}"),
            )
                .into_response(),
        }
    }
}

/// Translate an upstream Responses SSE stream into a Chat Completion SSE stream.
async fn stream_responses_to_chat(upstream: reqwest::Response, model: String) -> Response {
    let byte_stream = upstream.bytes_stream();
    let chat_stream = async_stream::stream! {
        let mut buffer = String::new();
        let mut stats = SseConvertStats::default();
        futures_util::pin_mut!(byte_stream);
        while let Some(chunk) = byte_stream.next().await {
            let chunk = match chunk {
                Ok(bytes) => bytes,
                Err(err) => {
                    yield Err(std::io::Error::new(std::io::ErrorKind::Other, err));
                    return;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE events (separated by a blank line).
            while let Some(idx) = buffer.find("\n\n") {
                let raw_event = buffer[..idx].to_string();
                buffer.drain(..idx + 2);
                for line in raw_event.lines() {
                    let line = line.trim_start();
                    let Some(data) = line.strip_prefix("data:") else { continue; };
                    let data = data.trim();
                    if data == "[DONE]" { continue; }
                    let Ok(event) = serde_json::from_str::<Value>(data) else { continue; };
                    if let Some(delta) = responses_event_to_delta(&event, &mut stats) {
                        let frame = chat_chunk(&delta, &model);
                        yield Ok(format!("data: {}\n\n", frame));
                    }
                }
            }
        }

        // Terminal chunk + DONE sentinel.
        let done = chat_chunk_done(&model);
        yield Ok(format!("data: {}\n\n", done));
        yield Ok("data: [DONE]\n\n".to_string());
        tracing::info!(
            events = stats.events,
            content_delta_events = stats.content_delta_events,
            fallback_content_events = stats.fallback_content_events,
            saw_tool_call = stats.saw_tool_call,
            "responses SSE adapter converted stream"
        );
    };

    let body = Body::from_stream(chat_stream);
    let mut response = Response::new(body);
    response
        .headers_mut()
        .insert("content-type", "text/event-stream".parse().unwrap());
    response
}
/// Convert a Chat Completions request body into a Responses request body.
///
/// - `messages` -> `input` (array of {role, content:[{type:input_text|output_text,text}]})
/// - `model`, `temperature`, `max_tokens`(->`max_output_tokens`), `stream` carried over.
/// - tool fields are dropped here (safe-mode strip happens before this).
pub fn chat_to_responses_request(chat: &Value) -> Value {
    let mut out = json!({});

    if let Some(model) = chat.get("model") {
        out["model"] = model.clone();
    }

    // Build the `input` array from chat messages.
    let mut input = Vec::new();
    if let Some(messages) = chat.get("messages").and_then(Value::as_array) {
        for message in messages {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user");
            let text = message
                .get("content")
                .map(content_to_text)
                .unwrap_or_default();
            // Responses uses `input_text` for user/system, `output_text` for assistant.
            let part_type = if role == "assistant" {
                "output_text"
            } else {
                "input_text"
            };
            input.push(json!({
                "role": role,
                "content": [{ "type": part_type, "text": text }],
            }));
        }
    }
    out["input"] = Value::Array(input);

    if let Some(stream) = chat.get("stream") {
        out["stream"] = stream.clone();
    }
    if let Some(temp) = chat.get("temperature") {
        out["temperature"] = temp.clone();
    }
    if let Some(max) = chat.get("max_tokens").and_then(Value::as_u64) {
        out["max_output_tokens"] = json!(max);
    }

    out
}

/// Flatten a Chat `content` field (string or array of parts) into plain text.
fn content_to_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => {
            let mut text = String::new();
            for part in parts {
                if let Some(value) = part.get("text").and_then(Value::as_str) {
                    text.push_str(value);
                } else if let Some(value) = part.as_str() {
                    text.push_str(value);
                }
            }
            text
        }
        _ => String::new(),
    }
}

/// Extract assistant text from a non-streaming Responses JSON body using the
/// fallback chain. Returns the concatenated text (possibly empty).
pub fn extract_text_from_responses_json(body: &Value) -> String {
    // Preferred: top-level `output_text` convenience field (some providers include it).
    if let Some(text) = body.get("output_text").and_then(Value::as_str) {
        if !text.is_empty() {
            return text.to_string();
        }
    }

    // Fallback: walk `output[*].content[*].text` for output_text parts.
    if let Some(output) = body.get("output").and_then(Value::as_array) {
        let mut collected = String::new();
        for item in output {
            if let Some(content) = item.get("content").and_then(Value::as_array) {
                for part in content {
                    let is_text = part
                        .get("type")
                        .and_then(Value::as_str)
                        .map(|t| t.contains("text"))
                        .unwrap_or(false);
                    if is_text {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            collected.push_str(text);
                        }
                    }
                }
            }
        }
        if !collected.is_empty() {
            return collected;
        }
    }

    String::new()
}

/// Wrap assistant text into a non-streaming Chat Completion JSON body.
pub fn responses_json_to_chat(text: &str, model: &str) -> Value {
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

/// A single chat.completion.chunk frame carrying a content delta.
pub fn chat_chunk(delta: &str, model: &str) -> Value {
    json!({
        "id": "chatcmpl-mpg",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": { "content": delta },
            "finish_reason": Value::Null
        }]
    })
}

/// The terminal chunk (finish_reason=stop).
pub fn chat_chunk_done(model: &str) -> Value {
    json!({
        "id": "chatcmpl-mpg",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "stop"
        }]
    })
}

/// Statistics from converting a Responses SSE stream (for diagnostics/logging).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SseConvertStats {
    pub events: usize,
    pub content_delta_events: usize,
    pub fallback_content_events: usize,
    pub saw_tool_call: bool,
}

/// Parse one Responses SSE `data:` JSON payload and return any text delta it
/// carries, applying the fallback chain. Updates `stats`.
///
/// Returns `Some(text)` when a text fragment should be emitted as a chat chunk.
pub fn responses_event_to_delta(event: &Value, stats: &mut SseConvertStats) -> Option<String> {
    stats.events += 1;
    let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");

    match event_type {
        // Primary streaming text path.
        "response.output_text.delta" => {
            if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                stats.content_delta_events += 1;
                return Some(delta.to_string());
            }
            None
        }
        // Fallback: full text at the done stage.
        "response.output_text.done" => {
            if let Some(text) = event.get("text").and_then(Value::as_str) {
                stats.fallback_content_events += 1;
                return Some(text.to_string());
            }
            None
        }
        // Fallback: content part completed carrying a text part.
        "response.content_part.done" => {
            let text = event
                .get("part")
                .and_then(|part| part.get("text"))
                .and_then(Value::as_str);
            if let Some(text) = text {
                stats.fallback_content_events += 1;
                return Some(text.to_string());
            }
            None
        }
        // Fallback: the whole response at completion.
        "response.completed" => {
            let response = event.get("response")?;
            let text = extract_text_from_responses_json(response);
            if !text.is_empty() {
                stats.fallback_content_events += 1;
                return Some(text);
            }
            None
        }
        // Track tool-call activity for diagnostics.
        other if other.contains("function_call") || other.contains("tool") => {
            stats.saw_tool_call = true;
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_chat_messages_to_input() {
        let chat = json!({
            "model": "gpt-5.5",
            "messages": [
                {"role": "system", "content": "be terse"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"}
            ],
            "max_tokens": 256,
            "stream": true
        });
        let req = chat_to_responses_request(&chat);
        assert_eq!(req["model"], "gpt-5.5");
        assert_eq!(req["stream"], true);
        assert_eq!(req["max_output_tokens"], 256);
        assert_eq!(req["input"][0]["role"], "system");
        assert_eq!(req["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(req["input"][2]["content"][0]["type"], "output_text");
        assert_eq!(req["input"][1]["content"][0]["text"], "hi");
    }

    #[test]
    fn extracts_text_from_output_array() {
        let body = json!({
            "output": [{
                "content": [{ "type": "output_text", "text": "answer" }]
            }]
        });
        assert_eq!(extract_text_from_responses_json(&body), "answer");
    }

    #[test]
    fn extracts_top_level_output_text() {
        let body = json!({ "output_text": "quick" });
        assert_eq!(extract_text_from_responses_json(&body), "quick");
    }

    #[test]
    fn delta_event_primary_path() {
        let mut stats = SseConvertStats::default();
        let event = json!({ "type": "response.output_text.delta", "delta": "Hel" });
        assert_eq!(
            responses_event_to_delta(&event, &mut stats).as_deref(),
            Some("Hel")
        );
        assert_eq!(stats.content_delta_events, 1);
    }

    #[test]
    fn delta_event_done_fallback() {
        let mut stats = SseConvertStats::default();
        let event = json!({ "type": "response.output_text.done", "text": "full text" });
        assert_eq!(
            responses_event_to_delta(&event, &mut stats).as_deref(),
            Some("full text")
        );
        assert_eq!(stats.fallback_content_events, 1);
    }

    #[test]
    fn delta_event_content_part_done_fallback() {
        let mut stats = SseConvertStats::default();
        let event = json!({
            "type": "response.content_part.done",
            "part": { "type": "output_text", "text": "part text" }
        });
        assert_eq!(
            responses_event_to_delta(&event, &mut stats).as_deref(),
            Some("part text")
        );
    }

    #[test]
    fn delta_event_completed_fallback() {
        let mut stats = SseConvertStats::default();
        let event = json!({
            "type": "response.completed",
            "response": { "output": [{ "content": [{ "type": "output_text", "text": "final" }] }] }
        });
        assert_eq!(
            responses_event_to_delta(&event, &mut stats).as_deref(),
            Some("final")
        );
    }

    #[test]
    fn tracks_tool_calls() {
        let mut stats = SseConvertStats::default();
        let event = json!({ "type": "response.function_call_arguments.delta" });
        assert!(responses_event_to_delta(&event, &mut stats).is_none());
        assert!(stats.saw_tool_call);
    }
}
