//! HTTP request forwarding (REST / GraphQL / SSE) for the transparent proxy.
//!
//! Forwards an incoming request to the upstream Warp backend byte-for-byte,
//! overriding the `Host` and `Authorization` headers. The response (including
//! streaming `text/event-stream` bodies) is relayed back unchanged.

use std::sync::Arc;
use std::time::Instant;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;

use super::server::ProxyState;

/// Hop-by-hop headers that must not be forwarded (per RFC 7230 ยง6.1) plus
/// `host`/`authorization` which we set explicitly.
const STRIPPED_REQUEST_HEADERS: &[&str] = &[
    "host",
    "authorization",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

const STRIPPED_RESPONSE_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
    // reqwest decompresses the body, so the original content-length/encoding
    // would be wrong; let the framework recompute.
    "content-length",
    "content-encoding",
];

/// Axum fallback handler: forward any request to the upstream backend.
pub async fn forward_handler(
    State(state): State<Arc<ProxyState>>,
    request: Request<Body>,
) -> Response {
    match forward(state, request).await {
        Ok(response) => response,
        Err(message) => {
            tracing::warn!(error = %message, "proxy forward failed");
            (StatusCode::BAD_GATEWAY, format!("proxy error: {message}")).into_response()
        }
    }
}

async fn forward(state: Arc<ProxyState>, request: Request<Body>) -> Result<Response, String> {
    let started = Instant::now();
    let (parts, body) = request.into_parts();
    let method = parts.method.clone();
    let uri = parts.uri.clone();

    let upstream_url = build_upstream_url(&state.config.upstream_http, &uri)?;

    // Buffer the request body. Agent requests are small protobuf/JSON payloads;
    // streaming uploads are not expected on this control plane.
    let body_bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|err| format!("failed to read request body: {err}"))?;

    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .map_err(|err| format!("invalid method: {err}"))?;

    let mut builder = state.client.request(reqwest_method, &upstream_url);
    builder = builder.headers(forward_request_headers(&parts.headers));

    // Override the Authorization header with the configured token.
    if !state.config.oz_token.is_empty() {
        builder = builder.bearer_auth(&state.config.oz_token);
    } else if let Some(value) = parts.headers.get(axum::http::header::AUTHORIZATION) {
        // No override configured: preserve the client's original credentials.
        builder = builder.header(reqwest::header::AUTHORIZATION, value.as_bytes());
    }

    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes);
    }

    let upstream_response = builder
        .send()
        .await
        .map_err(|err| format!("upstream request failed: {err}"))?;

    let status = upstream_response.status();
    let response = build_client_response(upstream_response)?;

    tracing::info!(
        method = %method,
        path = uri.path(),
        status = status.as_u16(),
        latency_ms = started.elapsed().as_millis() as u64,
        "proxied request"
    );

    Ok(response)
}

/// Combine the upstream root with the incoming path + query.
fn build_upstream_url(upstream_root: &str, uri: &Uri) -> Result<String, String> {
    let root = upstream_root.trim_end_matches('/');
    let path_and_query = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or_else(|| uri.path());
    Ok(format!("{root}{path_and_query}"))
}

/// Copy request headers, dropping hop-by-hop and explicitly-managed ones.
fn forward_request_headers(headers: &HeaderMap) -> reqwest::header::HeaderMap {
    let mut out = reqwest::header::HeaderMap::new();
    for (name, value) in headers {
        if STRIPPED_REQUEST_HEADERS.contains(&name.as_str()) {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()),
            reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            out.append(name, value);
        }
    }
    out
}

/// Convert the upstream reqwest response into an axum streaming response,
/// preserving status, headers (minus hop-by-hop), and a streamed body so SSE
/// works.
fn build_client_response(upstream: reqwest::Response) -> Result<Response, String> {
    let status = StatusCode::from_u16(upstream.status().as_u16())
        .map_err(|err| format!("invalid upstream status: {err}"))?;

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

    // Stream the body so server-sent events are relayed incrementally.
    let stream = upstream
        .bytes_stream()
        .map(|chunk| chunk.map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err)));
    let body = Body::from_stream(stream);

    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    Ok(response)
}

/// Helper used by tests and callers needing the method type alias.
pub type ProxyMethod = Method;
