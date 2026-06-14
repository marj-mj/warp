use std::sync::Arc;
use warp_gateway_wrapper::{GatewayEngine, ToolRegistry};
use warp_gateway_wrapper::tools::builtin::EchoTool;
use warp_gateway_wrapper::protocol::{GatewayRequest, GatewayResponse, ExecutionStatus};
use warp_gateway_wrapper::utils::TaskId;
use serde_json::json;

#[tokio::test]
async fn test_echo_tool_execution() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    
    let engine = GatewayEngine::new(registry);
    
    let task_id = TaskId::new();
    let request = GatewayRequest::Execute {
        task_id: task_id.clone(),
        tool_name: "echo".to_string(),
        parameters: json!({"message": "Hello, World!"}),
    };
    
    let response = engine.handle_request(request).await;
    
    match response {
        GatewayResponse::ToolResult { task_id: resp_id, result, status } => {
            assert_eq!(resp_id, task_id);
            assert_eq!(status, ExecutionStatus::Success);
            assert_eq!(result["echoed"], "Hello, World!");
        }
        _ => panic!("Expected ToolResult response"),
    }
}

#[tokio::test]
async fn test_list_tools() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    
    let engine = GatewayEngine::new(registry);
    
    let request = GatewayRequest::ListTools;
    let response = engine.handle_request(request).await;
    
    match response {
        GatewayResponse::ToolsList { tools } => {
            assert_eq!(tools.len(), 1);
            assert_eq!(tools[0].name, "echo");
        }
        _ => panic!("Expected ToolsList response"),
    }
}

#[tokio::test]
async fn test_tool_not_found() {
    let registry = ToolRegistry::new();
    let engine = GatewayEngine::new(registry);
    
    let task_id = TaskId::new();
    let request = GatewayRequest::Execute {
        task_id: task_id.clone(),
        tool_name: "nonexistent".to_string(),
        parameters: json!({}),
    };
    
    let response = engine.handle_request(request).await;
    
    match response {
        GatewayResponse::Error { task_id: resp_id, code, .. } => {
            assert_eq!(resp_id, Some(task_id));
            assert_eq!(code, "tool_not_found");
        }
        _ => panic!("Expected Error response"),
    }
}
