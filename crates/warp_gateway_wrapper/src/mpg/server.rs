//! Managed Provider Gateway HTTP server.
//!
//! Routes:
//! - `GET  /healthz`               – debug info
//! - `GET  /v1/healthz`            – debug info (alias used by some clients)
//! - `GET  /v1/models`             – advertised model list
//! - `POST /v1/chat/completions`   – Chat Completions endpoint
//!
//! Optional Bearer auth (`WARP_MANAGED_PROVIDER_GATEWAY_AUTH_TOKEN`); empty
//! token means open access (suitable for `127.0.0.1` deployments).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

use super::config::GatewayConfig;
use super::openai_chat::forward_chat_completions;

/// Shared state for the gateway: configuration + a reusable HTTP client.
pub struct MpgState {
    pub config: GatewayConfig,
    pub client: reqwest::Client,
    pub duplicate_guard: super::duplicate_guard::DuplicateGuard,
}

impl MpgState {
    pub fn new(config: GatewayConfig) -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client");
        let duplicate_guard =
            super::duplicate_guard::DuplicateGuard::new(config.duplicate_window_secs);
        Self {
            config,
            client,
            duplicate_guard,
        }
    }
}

pub struct MpgServer {
    addr: SocketAddr,
    state: Arc<MpgState>,
}

impl MpgServer {
    pub fn new(host: &str, port: u16, config: GatewayConfig) -> Result<Self, std::io::Error> {
        let addr: SocketAddr = format!("{host}:{port}")
            .parse()
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
        Ok(Self {
            addr,
            state: Arc::new(MpgState::new(config)),
        })
    }

    pub fn build_router(&self) -> Router {
        let state = self.state.clone();
        // Authenticated routes require the configured Bearer token (if any).
        let api_routes = Router::new()
            .route("/v1/healthz", get(healthz_handler))
            .route("/v1/models", get(models_handler))
            .route("/v1/chat/completions", post(chat_handler))
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ));

        Router::new()
            // Public health check (no auth) so probes can hit the gateway easily.
            .route("/healthz", get(healthz_handler))
            .merge(api_routes)
            .with_state(state)
    }

    /// The configured bind address.
    pub fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }

    pub async fn run(self) -> Result<(), std::io::Error> {
        let listener = tokio::net::TcpListener::bind(self.addr).await?;
        let bound_addr = listener.local_addr().unwrap_or(self.addr);
        self.log_listening(bound_addr);
        axum::serve(listener, self.build_router()).await
    }

    /// Run until shutdown resolves. Used to embed the gateway in another
    /// process (e.g. the launcher UI) with graceful stop.
    pub async fn run_with_shutdown<F>(self, shutdown: F) -> Result<(), std::io::Error>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.run_with_shutdown_signal(shutdown, None).await
    }

    /// Run until shutdown resolves and optionally report whether the listener
    /// bound successfully before serving requests.
    pub async fn run_with_shutdown_signal<F>(
        self,
        shutdown: F,
        ready_tx: Option<tokio::sync::oneshot::Sender<Result<SocketAddr, String>>>,
    ) -> Result<(), std::io::Error>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        match tokio::net::TcpListener::bind(self.addr).await {
            Ok(listener) => {
                let bound_addr = listener.local_addr().unwrap_or(self.addr);
                if let Some(ready_tx) = ready_tx {
                    let _ = ready_tx.send(Ok(bound_addr));
                }
                self.run_with_listener_and_shutdown(listener, shutdown)
                    .await
            }
            Err(err) => {
                if let Some(ready_tx) = ready_tx {
                    let _ = ready_tx.send(Err(err.to_string()));
                }
                Err(err)
            }
        }
    }

    /// Serve using an already-bound listener. This lets embedders reserve a
    /// fallback port atomically before spawning the server task.
    pub async fn run_with_listener_and_shutdown<F>(
        self,
        listener: tokio::net::TcpListener,
        shutdown: F,
    ) -> Result<(), std::io::Error>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let bound_addr = listener.local_addr().unwrap_or(self.addr);
        self.log_listening(bound_addr);
        axum::serve(listener, self.build_router())
            .with_graceful_shutdown(shutdown)
            .await
    }

    fn log_listening(&self, bound_addr: SocketAddr) {
        let auth_state = if self.state.config.auth_token.is_empty() {
            "open"
        } else {
            "configured"
        };
        let upstream_auth = if self.state.config.provider.resolved_api_key().is_some() {
            "configured"
        } else {
            "missing"
        };
        tracing::info!(
            addr = %bound_addr,
            upstream = %self.state.config.provider.base_url,
            adapter = self.state.config.provider.adapter.as_str(),
            wire_api = self.state.config.provider.wire_api.as_str(),
            auth = auth_state,
            upstream_auth,
            "managed provider gateway listening"
        );
    }
}

/// Authentication middleware. When `auth_token` is non-empty, requests must
/// present `Authorization: Bearer <token>`; otherwise all requests are allowed.
async fn auth_middleware(
    State(state): State<Arc<MpgState>>,
    request: Request,
    next: Next,
) -> Response {
    let expected = state.config.auth_token.trim();
    if expected.is_empty() {
        return next.run(request).await;
    }

    let authorized = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            let value = value.trim();
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
                .map(str::trim)
        })
        .map(|token| token == expected)
        .unwrap_or(false);

    if authorized {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "missing or invalid Authorization").into_response()
    }
}

/// `GET /healthz` (and `/v1/healthz`): debug summary mirroring `warp-oss`.
async fn healthz_handler(State(state): State<Arc<MpgState>>) -> Json<Value> {
    let config = &state.config;
    let provider = &config.provider;
    Json(json!({
        "status": "ok",
        "adapter": provider.adapter.as_str(),
        "compatibility_group": provider.adapter.compatibility_group(),
        "wire_api": provider.wire_api.as_str(),
        "default_model": provider.model,
        "base_url": provider.base_url,
        "tools": if config.disable_tools { "stripped" } else { "forwarded" },
        "mcp": if config.disable_mcp { "disabled" } else { "enabled" },
        "duplicate_request_window_secs": config.duplicate_window_secs,
        "force_model_config_key": config.force_model_config_key,
    }))
}

/// `GET /v1/models`: advertise the configured model so Warp can populate its
/// custom-endpoint model list.
async fn models_handler(State(state): State<Arc<MpgState>>) -> Json<Value> {
    let provider = &state.config.provider;
    let model_id = provider
        .model
        .clone()
        .unwrap_or_else(|| "default".to_string());
    Json(json!({
        "object": "list",
        "data": [{
            "id": model_id,
            "object": "model",
            "created": 0,
            "owned_by": provider.name,
        }]
    }))
}

/// `POST /v1/chat/completions`: dispatch to the configured adapter.
async fn chat_handler(State(state): State<Arc<MpgState>>, body: axum::Json<Value>) -> Response {
    use super::config::Adapter;
    use super::duplicate_guard::DuplicateGuard;

    // Duplicate-request guard: suppress identical requests within the window.
    if state.duplicate_guard.is_enabled() {
        let fingerprint = DuplicateGuard::fingerprint(&body.0);
        if state.duplicate_guard.check_and_record(fingerprint) {
            let model = body
                .0
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("default");
            tracing::info!(
                model,
                "managed provider gateway suppressed duplicate request"
            );
            return Json(DuplicateGuard::synthetic_response(model)).into_response();
        }
    }

    match state.config.provider.adapter {
        Adapter::OpenaiChat | Adapter::BridgeOpenai => forward_chat_completions(state, body).await,
        Adapter::OpenaiResponses => {
            // Apply the same safe-mode strip, then convert chat <-> responses.
            let (processed, info) = super::openai_chat::apply_safe_mode(body.0, &state.config);
            if let Some(info) = &info {
                tracing::info!(
                    tools = info.tools,
                    tool_choice = info.tool_choice,
                    parallel_tool_calls = info.parallel_tool_calls,
                    "managed provider gateway stripped upstream tool fields"
                );
            }
            super::openai_responses::forward_responses(state, processed).await
        }
        Adapter::AnthropicMessages => {
            let (processed, _info) = super::openai_chat::apply_safe_mode(body.0, &state.config);
            super::native::forward_anthropic(state, processed).await
        }
        Adapter::GeminiGenerateContent => {
            let (processed, _info) = super::openai_chat::apply_safe_mode(body.0, &state.config);
            super::native::forward_gemini(state, processed).await
        }
    }
}
