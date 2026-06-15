use std::sync::Arc;
use warp_gateway_wrapper::tools::builtin::EchoTool;
use warp_gateway_wrapper::{GatewayEngine, Server, ServerConfig, ToolRegistry};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tool registry and register built-in tools
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));

    // Create gateway engine
    let gateway = Arc::new(GatewayEngine::new(registry));

    // Configure server
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 8080,
    };

    // Start server
    let server = Server::new(config, gateway);

    println!("Starting Gateway HTTP server...");
    server.run().await?;

    Ok(())
}
