use warp_gateway_wrapper::gateway::GatewayEngine;
use warp_gateway_wrapper::tools::ToolRegistry;
use warp_gateway_wrapper::protocol::{GatewayRequest, GatewayResponse, ExecutionStatus};
use warp_gateway_wrapper::tools::builtin::LongRunningTool;
use warp_gateway_wrapper::utils::TaskId;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_task_cancellation_integration() {
    // Set up registry with long-running tool
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(LongRunningTool));
    
    let engine = GatewayEngine::new(registry);
    let task_id = TaskId::new();
    
    // Start a long-running task
    let engine_clone = engine.clone();
    let task_id_clone = task_id.clone();
    
    let execute_handle = tokio::spawn(async move {
        let request = GatewayRequest::Execute {
            task_id: task_id_clone,
            tool_name: "long_running".to_string(),
            parameters: serde_json::json!({
                "duration_ms": 5000
            }),
        };
        
        engine_clone.handle_request(request).await
    });
    
    // Wait a bit then cancel the task
    sleep(Duration::from_millis(100)).await;
    
    let cancel_request = GatewayRequest::CancelTask {
        task_id: task_id.clone(),
    };
    
    let cancel_response = engine.handle_request(cancel_request).await;
    
    // Verify cancellation was acknowledged
    match cancel_response {
        GatewayResponse::ToolResult { status, .. } => {
            assert_eq!(status, ExecutionStatus::Cancelled);
        }
        _ => panic!("Expected ToolResult with Cancelled status"),
    }
    
    // Wait for the task to complete
    let execution_response = execute_handle.await.unwrap();
    
    // Verify the task was actually cancelled
    match execution_response {
        GatewayResponse::Error { code, .. } => {
            assert_eq!(code, "cancelled");
        }
        _ => panic!("Expected Error response with cancelled code"),
    }
}

#[tokio::test]
async fn test_cancel_nonexistent_task() {
    let registry = ToolRegistry::new();
    let engine = GatewayEngine::new(registry);
    
    let task_id = TaskId::new();
    
    let request = GatewayRequest::CancelTask {
        task_id: task_id.clone(),
    };
    
    let response = engine.handle_request(request).await;
    
    match response {
        GatewayResponse::Error { code, .. } => {
            assert_eq!(code, "task_not_found");
        }
        _ => panic!("Expected Error response"),
    }
}

#[tokio::test]
async fn test_shutdown_cancels_all_tasks() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(LongRunningTool));
    
    let engine = GatewayEngine::new(registry);
    
    // Start multiple long-running tasks
    let task1 = TaskId::new();
    let task2 = TaskId::new();
    
    let engine_clone1 = engine.clone();
    let task1_clone = task1.clone();
    let handle1 = tokio::spawn(async move {
        let request = GatewayRequest::Execute {
            task_id: task1_clone,
            tool_name: "long_running".to_string(),
            parameters: serde_json::json!({
                "duration_ms": 5000
            }),
        };
        engine_clone1.handle_request(request).await
    });
    
    let engine_clone2 = engine.clone();
    let task2_clone = task2.clone();
    let handle2 = tokio::spawn(async move {
        let request = GatewayRequest::Execute {
            task_id: task2_clone,
            tool_name: "long_running".to_string(),
            parameters: serde_json::json!({
                "duration_ms": 5000
            }),
        };
        engine_clone2.handle_request(request).await
    });
    
    // Wait a bit then shutdown
    sleep(Duration::from_millis(100)).await;
    
    let shutdown_request = GatewayRequest::Shutdown;
    engine.handle_request(shutdown_request).await;
    
    // Wait for tasks to complete
    let response1 = handle1.await.unwrap();
    let response2 = handle2.await.unwrap();
    
    // Both should be cancelled
    match response1 {
        GatewayResponse::Error { code, .. } => {
            assert_eq!(code, "cancelled");
        }
        _ => panic!("Expected Error response with cancelled code"),
    }
    
    match response2 {
        GatewayResponse::Error { code, .. } => {
            assert_eq!(code, "cancelled");
        }
        _ => panic!("Expected Error response with cancelled code"),
    }
}
