//! `openai_chat` adapter: forward an incoming OpenAI Chat Completions request
//! to an upstream that speaks the same wire format. No body conversion needed;
//! the gateway only swaps `Authorization`, optionally strips tool-related
//! fields (safe mode), and streams the upstream response back to the client.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;
use serde_json::Value;

use super::config::GatewayConfig;
use super::server::MpgState;

const STRIPPED_RESPONSE_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
    // reqwest decompresses, so original lengths/encodings are wrong
    "content-length",
    "content-encoding",
];

/// Strip tool-related fields from a Chat Completions request body when the
/// gateway runs in safe mode. Mirrors the `warp-oss` Managed Provider Gateway
/// behavior: provider compatibility wins over Agent's tool plumbing.
pub fn apply_safe_mode(mut body: Value, config: &GatewayConfig) -> (Value, Option<SafeModeInfo>) {
    if !config.disable_tools {
        return (body, None);
    }
    let mut info = SafeModeInfo::default();
    if let Some(object) = body.as_object_mut() {
        if let Some(tools) = object.remove("tools") {
            info.tools = tools.as_array().map(|array| array.len()).unwrap_or(0);
        }
        if object.remove("tool_choice").is_some() {
            info.tool_choice = true;
        }
        if object.remove("parallel_tool_calls").is_some() {
            info.parallel_tool_calls = true;
        }
    }
    (body, Some(info))
}

#[derive(Debug, Default, Clone)]
pub struct SafeModeInfo {
    pub tools: usize,
    pub tool_choice: bool,
    pub parallel_tool_calls: bool,
}

/// Forward a Chat Completions request to the upstream and stream the response back.
pub async fn forward_chat_completions(
    state: Arc<MpgState>,
    Json(body): Json<Value>,
) -> Response {
    let (body, safe_info) = apply_safe_mode(body, &state.config);
    if let Some(info) = &safe_info {
        tracing::info!(
            tools = info.tools,
            tool_choice = info.tool_choice,
            parallel_tool_calls = info.parallel_tool_calls,
            "managed provider gateway stripped upstream tool fields"
        );
    }

    let stream_requested = body
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let url = format!(
        "{}/chat/completions",
        state.config.provider.base_url.trim_end_matches('/')
    );

    let mut request = state.client.post(&url);
    if let Some(key) = state.config.provider.resolved_api_key() {
        request = request.bearer_auth(key);
    }
    request = request.header(reqwest::header::CONTENT_TYPE, "application/json");
    request = request.json(&body);

    let upstream = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            tracing::warn!(error = %err, url = %url, "mpg upstream request failed");
            return (
                StatusCode::BAD_GATEWAY,
                format!("upstream request failed: {err}"),
            )
                .into_response();
        }
    };

    let status = match StatusCode::from_u16(upstream.status().as_u16()) {
        Ok(status) => status,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("invalid upstream status: {err}"),
            )
                .into_response()
        }
    };

    // Surface upstream errors clearly: a 404 here usually means the base_url is
    // missing a path segment (e.g. /v1) or the provider only supports a
    // different wire API. Buffer + log the body so it is not silently relayed.
    if !status.is_success() {
        let body = upstream.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(300).collect();
        tracing::warn!(
            status = status.as_u16(),
            url = %url,
            body = %snippet,
            "mpg upstream returned error status"
        );
        let mut response = Response::new(Body::from(body));
        *response.status_mut() = status;
        return response;
    }

    let mut headers = HeaderMap::new();
    for (name, value) in upstream.headers() {
        if STRIPPED_RESPONSE_HEADERS.contains(&name.as_str()) {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_str().as_bytes()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            headers.append(name, value);
        }
    }

    // For streaming responses, relay chunk-by-chunk so SSE works.
    let body_stream = upstream.bytes_stream().map(|chunk| {
        chunk.map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
    });
    let body = Body::from_stream(body_stream);

    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    tracing::info!(
        status = status.as_u16(),
        stream = stream_requested,
        adapter = "openai_chat",
        "mpg forwarded chat completions request"
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dummy_config(disable_tools: bool) -> GatewayConfig {
        let mut cfg = GatewayConfig::new(super::super::config::ProviderConfig {
            name: "p".into(),
            base_url: "https://x".into(),
            model: None,
            wire_api: super::super::config::WireApi::Chat,
            adapter: super::super::config::Adapter::OpenaiChat,
            api_key: None,
            env_key: None,
        });
        cfg.disable_tools = disable_tools;
        cfg
    }

    #[test]
    fn safe_mode_strips_tool_fields() {
        let body = json!({
            "model": "gpt-4",
            "messages": [],
            "tools": [{"type": "function"}, {"type": "function"}],
            "tool_choice": "auto",
            "parallel_tool_calls": false
        });
        let (out, info) = apply_safe_mode(body, &dummy_config(true));
        let info = info.unwrap();
        assert_eq!(info.tools, 2);
        assert!(info.tool_choice);
        assert!(info.parallel_tool_calls);
        assert!(out.get("tools").is_none());
        assert!(out.get("tool_choice").is_none());
        assert!(out.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn safe_mode_off_preserves_body() {
        let body = json!({"tools": [], "tool_choice": "auto"});
        let (out, info) = apply_safe_mode(body.clone(), &dummy_config(false));
        assert!(info.is_none());
        assert_eq!(out, body);
    }
}
