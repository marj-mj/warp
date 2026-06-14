use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

use crate::gateway::GatewayEngine;
use crate::http::auth::{AuthConfig, Authenticator};
use crate::http::handlers::{
    auth_middleware, cancel_task_handler, get_task_status_handler, health_handler,
    list_tasks_handler, spawn_agent_handler, stream_handler, AppState,
};

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
        }
    }
}

pub struct Server {
    config: ServerConfig,
    gateway: Arc<GatewayEngine>,
    authenticator: Arc<Authenticator>,
}

impl Server {
    /// Create a server with authentication disabled (local development).
    pub fn new(config: ServerConfig, gateway: Arc<GatewayEngine>) -> Self {
        Self::with_auth(config, gateway, AuthConfig::disabled())
    }

    /// Create a server with the given authentication configuration.
    pub fn with_auth(
        config: ServerConfig,
        gateway: Arc<GatewayEngine>,
        auth_config: AuthConfig,
    ) -> Self {
        Self {
            config,
            gateway,
            authenticator: Arc::new(Authenticator::new(auth_config)),
        }
    }

    pub fn build_router(&self) -> Router {
        let state = AppState {
            gateway: self.gateway.clone(),
            authenticator: self.authenticator.clone(),
        };

        // Authenticated agent routes. The middleware injects the resolved
        // Identity into request extensions for the handlers.
        let agent_routes = Router::new()
            .route("/agent/run", post(spawn_agent_handler))
            .route("/agent/stream/{task_id}", get(stream_handler))
            .route("/agent/cancel", post(cancel_task_handler))
            .route("/agent/task/{task_id}", get(get_task_status_handler))
            .route("/agent/tasks", get(list_tasks_handler))
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ));

        Router::new()
            .merge(agent_routes)
            // Health check is intentionally unauthenticated.
            .route("/health", get(health_handler))
            // Enable CORS for local development.
            .layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            )
            .with_state(state)
    }

    pub async fn run(self) -> Result<(), std::io::Error> {
        let addr = format!("{}:{}", self.config.host, self.config.port)
            .parse::<SocketAddr>()
            .expect("Invalid socket address");

        let auth_status = if self.authenticator.is_enabled() {
            "enabled"
        } else {
            "DISABLED (local dev)"
        };

        let router = self.build_router();

        tracing::info!(
            %addr,
            auth = auth_status,
            "agent server listening (POST /agent/run, GET /agent/stream/{{task_id}}, \
             POST /agent/cancel, GET /agent/task/{{task_id}}, GET /agent/tasks, GET /health)"
        );

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, router).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolRegistry;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_endpoint() {
        let registry = ToolRegistry::new();
        let gateway = Arc::new(GatewayEngine::new(registry));
        let config = ServerConfig::default();

        let server = Server::new(config, gateway);
        let app = server.build_router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_agent_run_requires_auth_when_enabled() {
        let registry = ToolRegistry::new();
        let gateway = Arc::new(GatewayEngine::new(registry));
        let server = Server::with_auth(
            ServerConfig::default(),
            gateway,
            AuthConfig::single_token("secret", "u1", true),
        );
        let app = server.build_router();

        // No Authorization header -> 401.
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/agent/run")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("{\"prompt\":\"hi\"}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_agent_run_accepts_valid_token() {
        let registry = ToolRegistry::new();
        let gateway = Arc::new(GatewayEngine::new(registry));
        let server = Server::with_auth(
            ServerConfig::default(),
            gateway,
            AuthConfig::single_token("secret", "u1", true),
        );
        let app = server.build_router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/agent/run")
                    .method("POST")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer secret")
                    .body(axum::body::Body::from("{\"prompt\":\"hi\"}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
