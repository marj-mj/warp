//! Integration test: the CLI delegate harness runs an external command and
//! streams its stdout. Uses an env override to point the `claude` harness at a
//! portable echo command so the test does not depend on a real CLI install.

use std::time::Duration;

use tokio::time::timeout;
use warp_gateway_wrapper::http::types::{AgentConfigSnapshot, SpawnAgentRequest, UserQueryMode};
use warp_gateway_wrapper::{GatewayEngine, SSEEvent, ToolRegistry};

fn cli_request(prompt: &str, harness: &str) -> SpawnAgentRequest {
    SpawnAgentRequest {
        prompt: Some(prompt.to_string()),
        mode: UserQueryMode::Normal,
        config: Some(AgentConfigSnapshot {
            harness: Some(harness.to_string()),
            ..Default::default()
        }),
        agent_identity_uid: None,
        attachments: Vec::new(),
        initial_snapshot_token: None,
        parent_run_id: None,
    }
}

#[tokio::test]
async fn cli_harness_streams_command_output() {
    // Point the `claude` harness at a portable echo command.
    if cfg!(windows) {
        std::env::set_var("WARP_GATEWAY_CLAUDE_CMD", "cmd");
        std::env::set_var("WARP_GATEWAY_CLAUDE_ARGS", "/C echo");
    } else {
        std::env::set_var("WARP_GATEWAY_CLAUDE_CMD", "echo");
        std::env::set_var("WARP_GATEWAY_CLAUDE_ARGS", " ");
    }

    let engine = GatewayEngine::new(ToolRegistry::new());
    let stream_manager = engine.stream_manager();

    let (task_id, _run_id) = match engine
        .spawn_agent(cli_request("HELLO_FROM_CLI", "claude"))
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

    let mut saw_message = false;
    let mut completed = false;
    let mut response = String::new();

    loop {
        let envelope = match timeout(Duration::from_secs(10), receiver.recv()).await {
            Ok(Ok(envelope)) => envelope,
            Ok(Err(_)) => break,
            Err(_) => panic!("timed out waiting for CLI harness events"),
        };
        match envelope.event {
            SSEEvent::Message { content, .. } => {
                if content.contains("HELLO_FROM_CLI") {
                    saw_message = true;
                }
            }
            SSEEvent::Complete { result, .. } => {
                completed = true;
                if let Some(result) = result {
                    assert_eq!(result["harness"], "claude");
                    response = result["response"].as_str().unwrap_or_default().to_string();
                }
                break;
            }
            SSEEvent::Error { message, .. } => panic!("CLI harness errored: {message}"),
            _ => {}
        }
    }

    std::env::remove_var("WARP_GATEWAY_CLAUDE_CMD");
    std::env::remove_var("WARP_GATEWAY_CLAUDE_ARGS");

    assert!(
        saw_message,
        "should stream the echoed line as an assistant message"
    );
    assert!(completed, "should emit Complete");
    assert!(
        response.contains("HELLO_FROM_CLI"),
        "final response should include CLI output"
    );
}

#[tokio::test]
async fn cli_harness_reports_launch_failure() {
    // Point at a command that does not exist.
    std::env::set_var(
        "WARP_GATEWAY_GEMINI_CMD",
        "definitely-not-a-real-binary-xyz",
    );
    std::env::set_var("WARP_GATEWAY_GEMINI_ARGS", " ");

    let engine = GatewayEngine::new(ToolRegistry::new());
    let stream_manager = engine.stream_manager();

    let (task_id, _run_id) = match engine.spawn_agent(cli_request("hi", "gemini")).await {
        warp_gateway_wrapper::gateway::engine::SpawnOutcome::Spawned { task_id, run_id } => {
            (task_id, run_id)
        }
        warp_gateway_wrapper::gateway::engine::SpawnOutcome::AtCapacity => {
            panic!("unexpected at_capacity")
        }
    };
    let mut receiver = stream_manager.subscribe(&task_id).await.expect("channel");

    let mut saw_error = false;
    loop {
        let envelope = match timeout(Duration::from_secs(10), receiver.recv()).await {
            Ok(Ok(envelope)) => envelope,
            Ok(Err(_)) => break,
            Err(_) => panic!("timed out"),
        };
        match envelope.event {
            SSEEvent::Error { code, .. } => {
                assert_eq!(code, "harness_launch_failed");
                saw_error = true;
                break;
            }
            SSEEvent::Complete { .. } => panic!("should not complete when launch fails"),
            _ => {}
        }
    }

    std::env::remove_var("WARP_GATEWAY_GEMINI_CMD");
    std::env::remove_var("WARP_GATEWAY_GEMINI_ARGS");

    assert!(saw_error, "should emit a launch failure error");
}
