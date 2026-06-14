use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;

use crate::gateway::GatewayEngine;
use crate::protocol::{GatewayRequest, GatewayResponse};

/// HTTP adapter providing REST and WebSocket access to the gateway
pub struct HttpAdapter {
    engine: Arc<GatewayEngine>,
}

impl HttpAdapter {
    pub fn new(engine: GatewayEngine) -> Self {
        Self {
            engine: Arc::new(engine),
        }
    }

    /// Create the axum router with all routes
    pub fn router(&self) -> Router {
        Router::new()
            .route("/health", get(health_check))
            .route("/api/execute", post(execute_request))
            .route("/api/tools", get(list_tools))
            .route("/ws", get(ws_handler))
            .with_state(Arc::clone(&self.engine))
    }

    /// Run the HTTP server on the specified address
    pub async fn serve(self, addr: std::net::SocketAddr) -> anyhow::Result<()> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!(%addr, "legacy HTTP adapter listening");
        
        axum::serve(listener, self.router()).await?;
        
        Ok(())
    }
}

/// Health check endpoint
async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "warp-gateway-wrapper"
    }))
}

/// Execute a gateway request via REST
async fn execute_request(
    State(engine): State<Arc<GatewayEngine>>,
    Json(request): Json<GatewayRequest>,
) -> Result<Json<GatewayResponse>, ApiError> {
    let response = engine.handle_request(request).await;
    Ok(Json(response))
}

/// List available tools
async fn list_tools(
    State(engine): State<Arc<GatewayEngine>>,
) -> Result<Json<GatewayResponse>, ApiError> {
    let response = engine.handle_request(GatewayRequest::ListTools).await;
    Ok(Json(response))
}

/// WebSocket handler for bidirectional communication
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(engine): State<Arc<GatewayEngine>>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, engine))
}

/// Handle WebSocket connection
async fn handle_socket(socket: WebSocket, engine: Arc<GatewayEngine>) {
    let (mut sender, mut receiver) = socket.split();

    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                // Parse the request
                let request: GatewayRequest = match serde_json::from_str(&text) {
                    Ok(req) => req,
                    Err(e) => {
                        let error_response = GatewayResponse::Error {
                            task_id: None,
                            code: "PARSE_ERROR".to_string(),
                            message: format!("Failed to parse request: {}", e),
                        };
                        
                        if let Ok(response_json) = serde_json::to_string(&error_response) {
                            let _ = sender.send(Message::Text(response_json.into())).await;
                        }
                        continue;
                    }
                };

                // Handle the request
                let response = engine.handle_request(request).await;

                // Send the response
                if let Ok(response_json) = serde_json::to_string(&response) {
                    if sender.send(Message::Text(response_json.into())).await.is_err() {
                        break;
                    }
                }
            }
            Ok(Message::Close(_)) => break,
            Err(e) => {
                tracing::warn!(error = %e, "websocket error");
                break;
            }
            _ => {} // Ignore ping/pong/binary
        }
    }
}

/// Custom error type for API responses
struct ApiError {
    code: StatusCode,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({
            "error": self.message
        }));
        
        (self.code, body).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError {
            code: StatusCode::INTERNAL_SERVER_ERROR,
            message: err.to_string(),
        }
    }
}
