//! Integration tests for the transparent Warp proxy.
//!
//! Spins up a fake "upstream" axum server in-process, points the proxy at it,
//! and exercises the forward path: header override (Authorization + Host),
//! query/body preservation, and streaming response (SSE) relay.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    body::Body,
    extract::{Query, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, get},
    Router,
};
use tokio::net::TcpListener;
use warp_gateway_wrapper::proxy::{ProxyConfig, ProxyServer};

#[derive(Default, Clone)]
struct CapturedRequest {
    method: String,
    path: String,
    query: String,
    auth: Option<String>,
    body: Vec<u8>,
}

#[derive(Clone, Default)]
struct UpstreamState {
    last: Arc<Mutex<Option<CapturedRequest>>>,
}

async fn capture_handler(State(state): State<UpstreamState>, req: Request) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let auth = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = axum::body::to_bytes(req.into_body(), 1024 * 1024)
        .await
        .unwrap_or_default()
        .to_vec();

    *state.last.lock().unwrap() = Some(CapturedRequest {
        method,
        path,
        query,
        auth,
        body,
    });

    (StatusCode::OK, "captured").into_response()
}

async fn echo_query(Query(params): Query<std::collections::HashMap<String, String>>) -> String {
    params
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

async fn sse_handler() -> impl IntoResponse {
    // Emit three SSE frames slowly so the proxy has to relay them as a stream.
    let body = Body::from_stream(futures_util::stream::iter([
        Ok::<_, std::io::Error>("data: one\n\n".to_string()),
        Ok::<_, std::io::Error>("data: two\n\n".to_string()),
        Ok::<_, std::io::Error>("data: three\n\n".to_string()),
    ]));
    let mut response = Response::new(body);
    response
        .headers_mut()
        .insert("content-type", "text/event-stream".parse().unwrap());
    response
}

/// Spawn an upstream HTTP server on an ephemeral port and return its base URL
/// plus a handle to the captured-request slot.
async fn start_upstream() -> (String, Arc<Mutex<Option<CapturedRequest>>>) {
    let state = UpstreamState::default();
    let captured = state.last.clone();
    let app = Router::new()
        .route("/echo-query", get(echo_query))
        .route("/sse", get(sse_handler))
        .fallback(any(capture_handler))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), captured)
}

/// Spawn the proxy on an ephemeral port targeting the given upstream HTTP root
/// + Bearer token, and return its base URL.
async fn start_proxy(upstream_http: String, oz_token: &str) -> String {
    let mut config = ProxyConfig::default();
    config.upstream_http = upstream_http;
    config.upstream_ws_rtc = None;
    config.upstream_ws_sessions = None;
    config.oz_token = oz_token.to_string();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    drop(listener); // ProxyServer will rebind; we just used this to grab a free port.

    let server = ProxyServer::new("127.0.0.1", addr.port(), config).unwrap();
    tokio::spawn(async move {
        let _ = server.run().await;
    });
    // Give the server a moment to start listening.
    tokio::time::sleep(Duration::from_millis(50)).await;
    format!("http://{addr}")
}

#[tokio::test]
async fn forwards_request_with_token_override() {
    let (upstream_url, captured) = start_upstream().await;
    let proxy_url = start_proxy(upstream_url, "oz-secret").await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{proxy_url}/agent/run?foo=bar"))
        .header("authorization", "Bearer client-token")
        .body("hello".to_string())
        .send()
        .await
        .expect("proxy request");
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "captured");

    let captured = captured.lock().unwrap().clone().expect("captured");
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.path, "/agent/run");
    assert_eq!(captured.query, "foo=bar");
    // Authorization must have been replaced with the configured OZ token.
    assert_eq!(captured.auth.as_deref(), Some("Bearer oz-secret"));
    assert_eq!(captured.body, b"hello");
}

#[tokio::test]
async fn forwards_query_and_status() {
    let (upstream_url, _captured) = start_upstream().await;
    let proxy_url = start_proxy(upstream_url, "tok").await;

    let body = reqwest::get(format!("{proxy_url}/echo-query?a=1&b=2"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let mut parts: Vec<&str> = body.split('&').collect();
    parts.sort();
    assert_eq!(parts, vec!["a=1", "b=2"]);
}

#[tokio::test]
async fn streams_sse_response() {
    let (upstream_url, _captured) = start_upstream().await;
    let proxy_url = start_proxy(upstream_url, "tok").await;

    let response = reqwest::get(format!("{proxy_url}/sse")).await.unwrap();
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    let body = response.text().await.unwrap();
    assert!(body.contains("data: one"));
    assert!(body.contains("data: two"));
    assert!(body.contains("data: three"));
}

#[tokio::test]
async fn passes_through_authorization_when_no_token_configured() {
    let (upstream_url, captured) = start_upstream().await;
    let proxy_url = start_proxy(upstream_url, "").await;

    let client = reqwest::Client::new();
    let _ = client
        .get(format!("{proxy_url}/whatever"))
        .header("authorization", "Bearer original")
        .send()
        .await
        .unwrap();

    let captured = captured.lock().unwrap().clone().expect("captured");
    assert_eq!(captured.auth.as_deref(), Some("Bearer original"));
}

// Silence unused-imports warnings when only some tests are compiled.
#[allow(dead_code)]
fn _silence() {
    let _ = HeaderMap::new();
}
