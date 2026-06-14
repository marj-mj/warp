//! WebSocket relay for the transparent Warp proxy.
//!
//! Accepts WebSocket upgrades on the proxy and opens a corresponding upstream
//! connection (typically `wss://rtc.app.warp.dev/...`). Frames are forwarded
//! in both directions until either side closes.
//!
//! The upstream URL is built by joining the configured WS root (RTC by
//! default; sessions if the path looks like a session-sharing endpoint) with
//! the original path/query the client sent. The upstream `Authorization`
//! header is set from the configured OZ token.

use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{HeaderMap, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as TungMessage};

use super::server::ProxyState;

#[derive(Clone, Debug, Default)]
pub struct UpstreamWsHeaders {
    authorization: Option<HeaderValue>,
    cookie: Option<HeaderValue>,
    sec_websocket_protocol: Option<HeaderValue>,
    user_agent: Option<HeaderValue>,
}

impl UpstreamWsHeaders {
    pub fn from_request_headers(headers: &HeaderMap) -> Self {
        Self {
            authorization: headers.get(axum::http::header::AUTHORIZATION).cloned(),
            cookie: headers.get(axum::http::header::COOKIE).cloned(),
            sec_websocket_protocol: headers
                .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)
                .cloned(),
            user_agent: headers.get(axum::http::header::USER_AGENT).cloned(),
        }
    }
}

/// Axum handler for the proxy WebSocket route. Upgrades the client connection
/// and spawns a relay against the upstream Warp WS endpoint.
pub async fn ws_handler(
    State(state): State<Arc<ProxyState>>,
    ws: WebSocketUpgrade,
    uri: Uri,
    upstream_headers: UpstreamWsHeaders,
) -> Response {
    let upstream_url = match build_upstream_ws_url(&state, &uri) {
        Ok(url) => url,
        Err(message) => {
            tracing::warn!(error = %message, "ws upstream URL build failed");
            return (StatusCode::BAD_GATEWAY, message).into_response();
        }
    };

    ws.on_upgrade(move |socket| async move {
        if let Err(err) = relay(socket, upstream_url, state, upstream_headers).await {
            tracing::warn!(error = %err, "ws relay terminated with error");
        }
    })
}

/// Pick the right upstream WS root for the incoming path. Anything looking like
/// session sharing routes to the sessions root; everything else uses RTC.
fn build_upstream_ws_url(state: &ProxyState, uri: &Uri) -> Result<String, String> {
    let path = uri.path();
    let path_and_query = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(path);

    let root = if path.contains("session") {
        state
            .config
            .upstream_ws_sessions
            .as_deref()
            .or(state.config.upstream_ws_rtc.as_deref())
    } else {
        state.config.upstream_ws_rtc.as_deref()
    };

    let root = root
        .ok_or_else(|| "no upstream WebSocket URL configured".to_string())?
        .trim_end_matches('/');
    Ok(format!("{root}{path_and_query}"))
}

async fn relay(
    client_ws: WebSocket,
    upstream_url: String,
    state: Arc<ProxyState>,
    upstream_headers: UpstreamWsHeaders,
) -> Result<(), String> {
    let mut request = upstream_url
        .as_str()
        .into_client_request()
        .map_err(|err| format!("invalid upstream URL: {err}"))?;

    apply_upstream_headers(request.headers_mut(), &state.config.oz_token, &upstream_headers)?;

    let (upstream_ws, response) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|err| format!("upstream connect failed: {err}"))?;
    tracing::info!(
        upstream = %upstream_url,
        status = response.status().as_u16(),
        "ws upstream connected"
    );

    let (mut upstream_sink, mut upstream_stream) = upstream_ws.split();
    let (mut client_sink, mut client_stream) = client_ws.split();

    // Client -> upstream
    let c2u = async {
        while let Some(message) = client_stream.next().await {
            let message = message.map_err(|err| format!("client recv: {err}"))?;
            let Some(frame) = axum_to_tungstenite(message) else {
                continue;
            };
            upstream_sink
                .send(frame)
                .await
                .map_err(|err| format!("upstream send: {err}"))?;
        }
        Ok::<(), String>(())
    };

    // Upstream -> client
    let u2c = async {
        while let Some(message) = upstream_stream.next().await {
            let message = message.map_err(|err| format!("upstream recv: {err}"))?;
            let Some(frame) = tungstenite_to_axum(message) else {
                continue;
            };
            client_sink
                .send(frame)
                .await
                .map_err(|err| format!("client send: {err}"))?;
        }
        Ok::<(), String>(())
    };

    // Run both directions concurrently; finish when either side completes.
    tokio::select! {
        result = c2u => result,
        result = u2c => result,
    }
}

/// Convert an axum `Message` into a tungstenite `Message`. Returns `None` for
/// frames we should silently ignore (e.g. raw pongs, which are managed at the
/// transport layer).
fn axum_to_tungstenite(message: AxumMessage) -> Option<TungMessage> {
    match message {
        AxumMessage::Text(text) => Some(TungMessage::Text(text.to_string().into())),
        AxumMessage::Binary(bytes) => Some(TungMessage::Binary(bytes.to_vec().into())),
        AxumMessage::Ping(payload) => Some(TungMessage::Ping(payload.to_vec().into())),
        AxumMessage::Pong(payload) => Some(TungMessage::Pong(payload.to_vec().into())),
        AxumMessage::Close(close) => Some(TungMessage::Close(close.map(|frame| {
            tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: frame.code.into(),
                reason: frame.reason.to_string().into(),
            }
        }))),
    }
}

fn tungstenite_to_axum(message: TungMessage) -> Option<AxumMessage> {
    match message {
        TungMessage::Text(text) => Some(AxumMessage::Text(text.to_string().into())),
        TungMessage::Binary(bytes) => Some(AxumMessage::Binary(bytes.to_vec().into())),
        TungMessage::Ping(payload) => Some(AxumMessage::Ping(payload.to_vec().into())),
        TungMessage::Pong(payload) => Some(AxumMessage::Pong(payload.to_vec().into())),
        TungMessage::Close(close) => Some(AxumMessage::Close(close.map(|frame| {
            axum::extract::ws::CloseFrame {
                code: frame.code.into(),
                reason: frame.reason.to_string().into(),
            }
        }))),
        // Raw frames are not surfaced by the axum side; ignore.
        TungMessage::Frame(_) => None,
    }
}

fn apply_upstream_headers(
    headers: &mut HeaderMap,
    override_token: &str,
    upstream_headers: &UpstreamWsHeaders,
) -> Result<(), String> {
    if !override_token.is_empty() {
        let value = HeaderValue::from_str(&format!("Bearer {override_token}"))
            .map_err(|err| format!("invalid OZ token header: {err}"))?;
        headers.insert(axum::http::header::AUTHORIZATION, value);
    } else if let Some(value) = &upstream_headers.authorization {
        headers.insert(axum::http::header::AUTHORIZATION, value.clone());
    }

    if let Some(value) = &upstream_headers.cookie {
        headers.insert(axum::http::header::COOKIE, value.clone());
    }
    if let Some(value) = &upstream_headers.sec_websocket_protocol {
        headers.insert(axum::http::header::SEC_WEBSOCKET_PROTOCOL, value.clone());
    }
    if let Some(value) = &upstream_headers.user_agent {
        headers.insert(axum::http::header::USER_AGENT, value.clone());
    }

    Ok(())
}

#[cfg(test)]
#[path = "ws_tests.rs"]
mod tests;
