//! Integration test: per-identity tool authorization through the agent loop.
//!
//! The mock provider requests the `echo` tool (a "basic" tool that any
//! authenticated identity may use). A non-privileged identity without shell or
//! filesystem permissions should still complete an echo-driven run, confirming
//! that basic tools are not blocked while sensitive ones would be.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::timeout;
use warp_gateway_wrapper::http::auth::Identity;
use warp_gateway_wrapper::http::types::{AgentConfigSnapshot, SpawnAgentRequest, UserQueryMode};
use warp_gateway_wrapper::tools::builtin::EchoTool;
use warp_gateway_wrapper::{GatewayEngine, SSEEvent, ToolRegistry};

fn request(prompt: &str) -> SpawnAgentRequest {
    SpawnAgentRequest {
        prompt: Some(prompt.to_string()),
        mode: UserQueryMode::Normal,
        config: Some(AgentConfigSnapshot {
            model: Some("gpt-4.1".to_string()),
            ..Default::default()
        }),
        agent_identity_uid: Some("limited".to_string()),
        attachments: Vec::new(),
        initial_snapshot_token: None,
        parent_run_id: None,
    }
}

fn limited_identity() -> Identity {
    Identity {
        uid: "limited".to_string(),
        is_privileged: false,
        permissions: HashSet::new(),
    }
}

#[tokio::test]
async fn basic_tool_allowed_for_limited_identity() {
    std::env::remove_var("OPENAI_API_KEY");
    std::env::remove_var("MANAGED_GATEWAY_TOKEN");

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    let engine = GatewayEngine::new(registry);
    let stream_manager = engine.stream_manager();

    let (task_id, _run_id) = match engine
        .spawn_agent_with_identity(request("hello"), limited_identity())
        .await
    {
        warp_gateway_wrapper::gateway::engine::SpawnOutcome::Spawned { task_id, run_id } => {
            (task_id, run_id)
        }
        warp_gateway_wrapper::gateway::engine::SpawnOutcome::AtCapacity => {
            panic!("unexpected at_capacity")
        }
    };

    let mut receiver = stream_manager.subscribe(&task_id).await.expect("channel");

    let mut saw_tool_completed = false;
    let mut saw_complete = false;
    loop {
        let event = match timeout(Duration::from_secs(5), receiver.recv()).await {
            Ok(Ok(event)) => event,
            Ok(Err(_)) => break,
            Err(_) => panic!("timed out"),
        };
        match event.event {
            SSEEvent::ToolCallCompleted { tool_name, .. } => {
                assert_eq!(tool_name, "echo");
                saw_tool_completed = true;
            }
            SSEEvent::ToolCallFailed { error, .. } => {
                panic!("basic echo tool should not be blocked: {error}");
            }
            SSEEvent::Complete { .. } => {
                saw_complete = true;
                break;
            }
            _ => {}
        }
    }

    assert!(
        saw_tool_completed,
        "echo (basic) should run for limited identity"
    );
    assert!(saw_complete);
}
