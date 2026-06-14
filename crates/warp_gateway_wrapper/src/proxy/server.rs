//! Proxy HTTP server: a single fallback router that forwards every request to
//! the upstream Warp backend.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;

use super::config::ProxyConfig;
use super::http::forward_handler;
use super::ws::ws_handler;

/// Shared state for proxy handlers: configured upstream + a reusable HTTP
/// client.
pub struct ProxyState {
    pub config: ProxyConfig,
    pub client: reqwest::Client,
}

impl ProxyState {
    pub fn new(config: ProxyConfig) -> Self {
        // Disable redirect following so redirects are passed through to the
        // client unchanged (preserves correct origin / cookie semantics).
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client");
        Self { config, client }
    }
}

pub struct ProxyServer {
    addr: SocketAddr,
    state: Arc<ProxyState>,
}

impl ProxyServer {
    pub fn new(host: &str, port: u16, config: ProxyConfig) -> Result<Self, std::io::Error> {
        let addr: SocketAddr = format!("{host}:{port}")
            .parse()
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
        Ok(Self {
            addr,
            state: Arc::new(ProxyState::new(config)),
        })
    }

    /// Build the axum router. Every path falls through to the forward handler.
    pub fn build_router(&self) -> Router {
        // Requests with an Upgrade: websocket header are dispatched to the
        // WebSocket relay; everything else falls through to the HTTP forwarder.
        Router::new()
            .fallback(forward_handler)
            .layer(middleware::from_fn_with_state(
                self.state.clone(),
                ws_dispatch_middleware,
            ))
            .with_state(self.state.clone())
    }

    pub async fn run(self) -> Result<(), std::io::Error> {
        let listener = tokio::net::TcpListener::bind(self.addr).await?;
        let token_status = if self.state.config.oz_token.is_empty() {
            "no-override"
        } else {
            "configured"
        };
        tracing::info!(
            addr = %self.addr,
            upstream = %self.state.config.upstream_http,
            oz_token = token_status,
            "transparent Warp proxy listening"
        );
        axum::serve(listener, self.build_router()).await
    }
}


/// If the incoming request is a WebSocket upgrade, route it to the WS handler;
/// otherwise pass through to the HTTP forwarder.
async fn ws_dispatch_middleware(
    State(state): State<std::sync::Arc<ProxyState>>,
    request: Request,
    next: Next,
) -> Response {
    use axum::extract::ws::WebSocketUpgrade;
    use axum::extract::FromRequestParts;

    let is_upgrade = request
        .headers()
        .get(axum::http::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    if !is_upgrade {
        return next.run(request).await;
    }

    let (mut parts, _body) = request.into_parts();
    let uri = parts.uri.clone();
    match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(upgrade) => ws_handler(State(state), upgrade, uri).await,
        Err(rejection) => rejection.into_response(),
    }
}