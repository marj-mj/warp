//! Integration tests for the Managed Provider Gateway.
//!
//! Spins up a fake OpenAI-compatible upstream in-process, points the gateway
//! at it, and exercises: /healthz, /v1/models, /v1/chat/completions forward
//! (including SSE streaming), Bearer auth gating, and the disable_tools strip.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::routing::{get, post as axum_post};
use axum::{
    body::Body,
    extract::{Json, Request, State},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use warp_gateway_wrapper::mpg::{
    Adapter, GatewayConfig as MpgGatewayConfig, MpgServer, ProviderConfig, WireApi,
};

#[derive(Default, Clone)]
struct UpstreamCapture {
    last_body: Arc<Mutex<Option<Value>>>,
    last_auth: Arc<Mutex<Option<String>>>,
}

async fn capture_chat(State(state): State<UpstreamCapture>, request: Request) -> Response {
    let auth = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    *state.last_auth.lock().unwrap() = auth;

    let body_bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
        .await
        .unwrap_or_default();
    let body: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    let stream_requested = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    *state.last_body.lock().unwrap() = Some(body);

    if stream_requested {
        // Emit a couple of SSE chunks plus the [DONE] sentinel.
        let frames: Vec<Result<String, std::io::Error>> = vec![
            Ok("data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n".to_string()),
            Ok("data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n".to_string()),
            Ok("data: [DONE]\n\n".to_string()),
        ];
        let stream = futures_util::stream::iter(frames);
        let mut response = Response::new(Body::from_stream(stream));
        response
            .headers_mut()
            .insert("content-type", "text/event-stream".parse().unwrap());
        response
    } else {
        Json(json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "model": "gpt-test",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "ok"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
        }))
        .into_response()
    }
}

async fn start_upstream() -> (String, UpstreamCapture) {
    let state = UpstreamCapture::default();
    let app = Router::new()
        .route("/v1/chat/completions", post(capture_chat))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}/v1"), state)
}

async fn start_gateway(upstream_url: String, auth_token: &str, disable_tools: bool) -> String {
    let mut cfg = MpgGatewayConfig::new(ProviderConfig {
        name: "test".into(),
        base_url: upstream_url,
        model: Some("gpt-test".into()),
        wire_api: WireApi::Chat,
        adapter: Adapter::OpenaiChat,
        api_key: Some("upstream-secret".into()),
        env_key: None,
    });
    cfg.auth_token = auth_token.to_string();
    cfg.disable_tools = disable_tools;
    cfg.disable_mcp = true;
    cfg.duplicate_window_secs = 0;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    drop(listener);
    let server = MpgServer::new("127.0.0.1", addr.port(), cfg).unwrap();
    tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    format!("http://{addr}")
}

#[tokio::test]
async fn healthz_reports_config() {
    let (upstream, _capture) = start_upstream().await;
    let gateway = start_gateway(upstream.clone(), "", false).await;
    let body: Value = reqwest::get(format!("{gateway}/healthz"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["adapter"], "openai_chat");
    assert_eq!(body["compatibility_group"], "openai_compatible_chat");
    assert_eq!(body["wire_api"], "chat");
    assert_eq!(body["default_model"], "gpt-test");
    assert_eq!(body["base_url"], upstream);
}

#[tokio::test]
async fn models_lists_default_model() {
    let (upstream, _capture) = start_upstream().await;
    let gateway = start_gateway(upstream, "", false).await;
    let body: Value = reqwest::get(format!("{gateway}/v1/models"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["object"], "list");
    assert_eq!(body["data"][0]["id"], "gpt-test");
    assert_eq!(body["data"][0]["owned_by"], "test");
}

#[tokio::test]
async fn chat_forwards_request_with_upstream_auth() {
    let (upstream, capture) = start_upstream().await;
    let gateway = start_gateway(upstream, "", false).await;

    let payload = json!({
        "model": "gpt-test",
        "messages": [{"role": "user", "content": "hi"}]
    });
    let response: Value = reqwest::Client::new()
        .post(format!("{gateway}/v1/chat/completions"))
        .json(&payload)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(response["choices"][0]["message"]["content"], "ok");

    let captured_body = capture.last_body.lock().unwrap().clone().expect("body");
    assert_eq!(captured_body["model"], "gpt-test");
    assert_eq!(captured_body["messages"][0]["content"], "hi");

    let captured_auth = capture.last_auth.lock().unwrap().clone();
    // The gateway should rewrite Authorization to the configured upstream key.
    assert_eq!(captured_auth.as_deref(), Some("Bearer upstream-secret"));
}

#[tokio::test]
async fn safe_mode_strips_tool_fields() {
    let (upstream, capture) = start_upstream().await;
    let gateway = start_gateway(upstream, "", true).await;

    let payload = json!({
        "model": "gpt-test",
        "messages": [{"role": "user", "content": "hi"}],
        "tools": [{"type": "function"}],
        "tool_choice": "auto",
        "parallel_tool_calls": false
    });
    let _ = reqwest::Client::new()
        .post(format!("{gateway}/v1/chat/completions"))
        .json(&payload)
        .send()
        .await
        .unwrap();

    let captured = capture.last_body.lock().unwrap().clone().expect("body");
    assert!(captured.get("tools").is_none(), "tools should be stripped");
    assert!(captured.get("tool_choice").is_none());
    assert!(captured.get("parallel_tool_calls").is_none());
}

#[tokio::test]
async fn streaming_response_is_relayed() {
    let (upstream, _capture) = start_upstream().await;
    let gateway = start_gateway(upstream, "", false).await;

    let response = reqwest::Client::new()
        .post(format!("{gateway}/v1/chat/completions"))
        .json(&json!({
            "model": "gpt-test",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    let body = response.text().await.unwrap();
    assert!(body.contains("Hello"));
    assert!(body.contains("world"));
    assert!(body.contains("[DONE]"));
}

#[tokio::test]
async fn auth_token_gates_protected_endpoints() {
    let (upstream, _capture) = start_upstream().await;
    let gateway = start_gateway(upstream, "gateway-secret", false).await;

    // /healthz is public.
    let public = reqwest::get(format!("{gateway}/healthz")).await.unwrap();
    assert_eq!(public.status(), 200);

    // Models requires auth.
    let unauth = reqwest::get(format!("{gateway}/v1/models")).await.unwrap();
    assert_eq!(unauth.status(), 401);

    let authed = reqwest::Client::new()
        .get(format!("{gateway}/v1/models"))
        .header("authorization", "Bearer gateway-secret")
        .send()
        .await
        .unwrap();
    assert_eq!(authed.status(), 200);
}

// ── Adapters: responses, anthropic, gemini ─────────────────────────────────

/// Start a multi-endpoint upstream that serves responses / anthropic / gemini.
async fn start_multi_upstream() -> String {
    async fn responses_handler() -> Response {
        // Non-stream Responses body with output_text in the output array.
        Json(json!({
            "output": [{ "content": [{ "type": "output_text", "text": "resp-answer" }] }]
        }))
        .into_response()
    }
    async fn anthropic_handler() -> Response {
        Json(json!({
            "content": [{ "type": "text", "text": "claude-answer" }]
        }))
        .into_response()
    }
    async fn gemini_handler() -> Response {
        Json(json!({
            "candidates": [{ "content": { "parts": [{ "text": "gemini-answer" }] } }]
        }))
        .into_response()
    }

    let app = Router::new()
        .route("/v1/responses", axum_post(responses_handler))
        .route("/v1/messages", axum_post(anthropic_handler))
        .route("/v1/models/{model}", axum_post(gemini_handler))
        .fallback(get(|| async { "ok" }));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/v1")
}

async fn start_gateway_with_adapter(upstream_url: String, adapter: Adapter) -> String {
    let mut cfg = MpgGatewayConfig::new(ProviderConfig {
        name: "test".into(),
        base_url: upstream_url,
        model: Some("m".into()),
        wire_api: WireApi::Chat,
        adapter,
        api_key: Some("k".into()),
        env_key: None,
    });
    cfg.auth_token = String::new();
    cfg.disable_tools = true;
    cfg.duplicate_window_secs = 0;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    drop(listener);
    let server = MpgServer::new("127.0.0.1", addr.port(), cfg).unwrap();
    tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    format!("http://{addr}")
}

async fn chat_via(gateway: &str) -> Value {
    reqwest::Client::new()
        .post(format!("{gateway}/v1/chat/completions"))
        .json(&json!({ "model": "m", "messages": [{"role": "user", "content": "hi"}] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn responses_adapter_converts_to_chat() {
    let upstream = start_multi_upstream().await;
    let gateway = start_gateway_with_adapter(upstream, Adapter::OpenaiResponses).await;
    let body = chat_via(&gateway).await;
    assert_eq!(body["choices"][0]["message"]["content"], "resp-answer");
    assert_eq!(body["object"], "chat.completion");
}

#[tokio::test]
async fn anthropic_adapter_converts_to_chat() {
    let upstream = start_multi_upstream().await;
    let gateway = start_gateway_with_adapter(upstream, Adapter::AnthropicMessages).await;
    let body = chat_via(&gateway).await;
    assert_eq!(body["choices"][0]["message"]["content"], "claude-answer");
}

#[tokio::test]
async fn gemini_adapter_converts_to_chat() {
    let upstream = start_multi_upstream().await;
    let gateway = start_gateway_with_adapter(upstream, Adapter::GeminiGenerateContent).await;
    let body = chat_via(&gateway).await;
    assert_eq!(body["choices"][0]["message"]["content"], "gemini-answer");
}

#[tokio::test]
async fn duplicate_guard_suppresses_second_identical_request() {
    let (upstream, _capture) = start_upstream().await;
    // Build a gateway with a non-zero duplicate window.
    let mut cfg = MpgGatewayConfig::new(ProviderConfig {
        name: "test".into(),
        base_url: upstream,
        model: Some("gpt-test".into()),
        wire_api: WireApi::Chat,
        adapter: Adapter::OpenaiChat,
        api_key: Some("k".into()),
        env_key: None,
    });
    cfg.duplicate_window_secs = 60;
    cfg.disable_tools = false;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    drop(listener);
    let server = MpgServer::new("127.0.0.1", addr.port(), cfg).unwrap();
    tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let gateway = format!("http://{addr}");

    let payload = json!({ "model": "gpt-test", "messages": [{"role": "user", "content": "dup"}] });
    let first: Value = reqwest::Client::new()
        .post(format!("{gateway}/v1/chat/completions"))
        .json(&payload)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // First goes upstream -> content "ok".
    assert_eq!(first["choices"][0]["message"]["content"], "ok");

    let second: Value = reqwest::Client::new()
        .post(format!("{gateway}/v1/chat/completions"))
        .json(&payload)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // Second within window -> synthetic suppressed response.
    assert_eq!(second["x_managed_gateway"]["duplicate_suppressed"], true);
}
