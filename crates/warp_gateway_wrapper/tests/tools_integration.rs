//! Integration tests for the shell and filesystem built-in tools executed
//! through the GatewayEngine request path.

use std::sync::Arc;

use serde_json::json;
use warp_gateway_wrapper::protocol::{ExecutionStatus, GatewayRequest, GatewayResponse};
use warp_gateway_wrapper::tools::builtin::{FilesystemTool, ShellTool};
use warp_gateway_wrapper::utils::TaskId;
use warp_gateway_wrapper::{GatewayEngine, ToolRegistry};

fn engine_with_tools(root: &std::path::Path) -> GatewayEngine {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ShellTool));
    registry.register(Arc::new(FilesystemTool::new(root)));
    GatewayEngine::new(registry)
}

fn temp_root() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wgw-int-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn shell_tool_via_engine() {
    let root = temp_root();
    let engine = engine_with_tools(&root);

    let response = engine
        .handle_request(GatewayRequest::Execute {
            task_id: TaskId::new(),
            tool_name: "run_shell_command".to_string(),
            parameters: json!({ "command": "echo integration" }),
        })
        .await;

    match response {
        GatewayResponse::ToolResult { status, result, .. } => {
            assert_eq!(status, ExecutionStatus::Success);
            assert_eq!(result["success"], true);
            assert!(result["stdout"].as_str().unwrap().contains("integration"));
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn filesystem_tool_via_engine() {
    let root = temp_root();
    let engine = engine_with_tools(&root);

    let write = engine
        .handle_request(GatewayRequest::Execute {
            task_id: TaskId::new(),
            tool_name: "filesystem".to_string(),
            parameters: json!({ "action": "write", "path": "data.txt", "contents": "payload" }),
        })
        .await;
    assert!(matches!(
        write,
        GatewayResponse::ToolResult {
            status: ExecutionStatus::Success,
            ..
        }
    ));

    let read = engine
        .handle_request(GatewayRequest::Execute {
            task_id: TaskId::new(),
            tool_name: "filesystem".to_string(),
            parameters: json!({ "action": "read", "path": "data.txt" }),
        })
        .await;

    match read {
        GatewayResponse::ToolResult { result, .. } => {
            assert_eq!(result["contents"], "payload");
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn filesystem_traversal_is_rejected_via_engine() {
    let root = temp_root();
    let engine = engine_with_tools(&root);

    let response = engine
        .handle_request(GatewayRequest::Execute {
            task_id: TaskId::new(),
            tool_name: "filesystem".to_string(),
            parameters: json!({ "action": "read", "path": "../secret.txt" }),
        })
        .await;

    match response {
        GatewayResponse::Error { code, .. } => assert_eq!(code, "invalid_parameters"),
        other => panic!("expected Error, got {other:?}"),
    }

    std::fs::remove_dir_all(&root).ok();
}
