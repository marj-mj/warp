#[cfg(test)]
mod http_tests {
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tower::ServiceExt;
    use warp_gateway_wrapper::{GatewayEngine, SSEEvent, Server, ServerConfig, ToolRegistry};

    fn create_test_server() -> Server {
        let registry = ToolRegistry::new();
        let gateway = Arc::new(GatewayEngine::new(registry));
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0, // Random port for testing
        };
        Server::new(config, gateway)
    }

    #[tokio::test]
    async fn test_health_check() {
        let server = create_test_server();
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
    async fn test_spawn_agent_endpoint() {
        let server = create_test_server();
        let app = server.build_router();

        let request_body = serde_json::json!({
            "prompt": "Test prompt",
            "config": {
                "model": "gpt-4"
            }
        });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/agent/run")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(request_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_sse_event_formatting() {
        let event = SSEEvent::Progress {
            task_id: "test-123".to_string(),
            progress: 0.5,
            message: Some("In progress".to_string()),
            metadata: None,
        };

        let formatted = event.to_sse_message();
        assert!(formatted.starts_with("data: "));
        assert!(formatted.ends_with("\n\n"));
        assert!(formatted.contains("\"type\":\"progress\""));
        assert!(formatted.contains("\"progress\":0.5"));
    }

    #[tokio::test]
    async fn test_stream_manager() {
        use warp_gateway_wrapper::http::stream_manager::StreamManager;
        use warp_gateway_wrapper::utils::TaskId;

        let manager = StreamManager::new(10);
        let task_id = TaskId::new();

        // Create channel
        manager.create_channel(task_id.clone()).await;
        assert!(manager.has_stream(&task_id).await);

        // Subscribe
        let mut rx = manager.subscribe(&task_id).await.unwrap();

        // Send event via the manager so it gets a sequence id + history entry.
        let event = SSEEvent::StreamInit {
            run_id: "run-123".to_string(),
            task_id: task_id.to_string(),
        };
        manager.send_event(&task_id, event).await.unwrap();

        // Receive event (wrapped in a StreamEnvelope carrying the id).
        let received = rx.recv().await.unwrap();
        assert_eq!(received.id, 1);
        assert!(matches!(received.event, SSEEvent::StreamInit { .. }));

        // Remove channel
        manager.remove_channel(&task_id).await;
        assert!(!manager.has_stream(&task_id).await);
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_task() {
        let server = create_test_server();
        let app = server.build_router();

        let request_body = serde_json::json!({
            "task_id": "nonexistent-task-id"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/agent/cancel")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(request_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return OK even for nonexistent tasks
        assert_eq!(response.status(), StatusCode::OK);
    }
}
