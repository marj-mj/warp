#[cfg(test)]
mod http_adapter_tests {
    use std::sync::Arc;
    use warp_gateway_wrapper::tools::builtin::EchoTool;
    use warp_gateway_wrapper::{GatewayEngine, HttpAdapter, ToolRegistry};

    #[tokio::test]
    async fn test_http_adapter_creation() {
        // Initialize tool registry
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));

        // Create gateway engine
        let engine = GatewayEngine::new(registry);

        // Create HTTP adapter
        let adapter = HttpAdapter::new(engine);

        // Just verify it can create a router
        let _router = adapter.router();
    }
}
