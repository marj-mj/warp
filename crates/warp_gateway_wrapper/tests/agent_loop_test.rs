//! End-to-end test of the agent loop driven by the offline mock provider.

use std::sync::Arc;
use std::time::Duration;

use tokio::time::timeout;
use warp_gateway_wrapper::http::types::{AgentConfigSnapshot, SpawnAgentRequest, UserQueryMode};
use warp_gateway_wrapper::tools::builtin::EchoTool;
use warp_gateway_wrapper::{GatewayEngine, SSEEvent, ToolRegistry};

fn spawn_request(prompt: &str, model: Option<&str>) -> SpawnAgentRequest {
    SpawnAgentRequest {
        prompt: Some(prompt.to_string()),
        mode: UserQueryMode::Normal,
        config: model.map(|model| AgentConfigSnapshot {
            model: Some(model.to_string()),
            ..Default::default()
        }),
        agent_identity_uid: None,
        attachments: Vec::new(),
        initial_snapshot_token: None,
        parent_run_id: None,
    }
}

#[tokio::test]
async fn agent_loop_runs_tool_and_completes() {
    // Ensure no live provider is selected so the deterministic mock is used.
    std::env::remove_var("OPENAI_API_KEY");
    std::env::remove_var("MANAGED_GATEWAY_TOKEN");

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    let engine = GatewayEngine::new(registry);

    let stream_manager = engine.stream_manager();
    let request = spawn_request("hello agent", Some("gpt-4.1"));

    let (task_id, run_id) = match engine.spawn_agent(request).await {
        warp_gateway_wrapper::gateway::engine::SpawnOutcome::Spawned { task_id, run_id } => (task_id, run_id),
        warp_gateway_wrapper::gateway::engine::SpawnOutcome::AtCapacity => panic!("unexpected at_capacity"),
    };
    assert!(!run_id.is_empty());

    let mut receiver = stream_manager
        .subscribe(&task_id)
        .await
        .expect("stream channel should exist for spawned task");

    let mut saw_stream_init = false;
    let mut saw_tool_started = false;
    let mut saw_tool_completed = false;
    let mut saw_complete = false;
    let mut final_response = String::new();

    // Collect events until completion or timeout.
    loop {
        let event = match timeout(Duration::from_secs(5), receiver.recv()).await {
            Ok(Ok(event)) => event,
            Ok(Err(_)) => break, // channel closed
            Err(_) => panic!("timed out waiting for agent events"),
        };

        match event.event {
            SSEEvent::StreamInit { .. } => saw_stream_init = true,
            SSEEvent::ToolCallStarted { tool_name, .. } => {
                assert_eq!(tool_name, "echo");
                saw_tool_started = true;
            }
            SSEEvent::ToolCallCompleted { tool_name, .. } => {
                assert_eq!(tool_name, "echo");
                saw_tool_completed = true;
            }
            SSEEvent::Complete { result, .. } => {
                saw_complete = true;
                if let Some(result) = result {
                    final_response = result["response"].as_str().unwrap_or_default().to_string();
                    assert_eq!(result["provider"], "mock");
                }
                break;
            }
            SSEEvent::Error { message, .. } => panic!("unexpected error event: {message}"),
            _ => {}
        }
    }

    assert!(saw_stream_init, "should emit StreamInit");
    assert!(saw_tool_started, "should start the echo tool");
    assert!(saw_tool_completed, "should complete the echo tool");
    assert!(saw_complete, "should emit Complete");
    assert!(final_response.contains("hello agent"), "final response should mention the prompt");

    let record = engine
        .get_execution_record(&task_id)
        .await
        .expect("execution record should exist");
    assert_eq!(
        record.state,
        warp_gateway_wrapper::ExecutionState::Completed
    );
}

#[tokio::test]
async fn agent_loop_completes_without_tools() {
    std::env::remove_var("OPENAI_API_KEY");
    std::env::remove_var("MANAGED_GATEWAY_TOKEN");

    // No tools registered: provider should answer directly.
    let engine = GatewayEngine::new(ToolRegistry::new());
    let stream_manager = engine.stream_manager();

    let (task_id, _run_id) = match engine.spawn_agent(spawn_request("just answer", None)).await {
        warp_gateway_wrapper::gateway::engine::SpawnOutcome::Spawned { task_id, run_id } => (task_id, run_id),
        warp_gateway_wrapper::gateway::engine::SpawnOutcome::AtCapacity => panic!("unexpected at_capacity"),
    };
    let mut receiver = stream_manager.subscribe(&task_id).await.expect("channel");

    let mut saw_complete = false;
    loop {
        let event = match timeout(Duration::from_secs(5), receiver.recv()).await {
            Ok(Ok(event)) => event,
            Ok(Err(_)) => break,
            Err(_) => panic!("timed out"),
        };
        if let SSEEvent::Complete { .. } = event.event {
            saw_complete = true;
            break;
        }
        if let SSEEvent::ToolCallStarted { .. } = event.event {
            panic!("no tool should be called when none are registered");
        }
    }
    assert!(saw_complete);
}

#[tokio::test]
async fn agent_loop_streams_message_deltas() {
    std::env::remove_var("OPENAI_API_KEY");
    std::env::remove_var("MANAGED_GATEWAY_TOKEN");

    let engine = GatewayEngine::new(ToolRegistry::new());
    let stream_manager = engine.stream_manager();

    let (task_id, _run_id) = match engine.spawn_agent(spawn_request("stream please", None)).await {
        warp_gateway_wrapper::gateway::engine::SpawnOutcome::Spawned { task_id, run_id } => (task_id, run_id),
        warp_gateway_wrapper::gateway::engine::SpawnOutcome::AtCapacity => panic!("unexpected at_capacity"),
    };
    let mut receiver = stream_manager.subscribe(&task_id).await.expect("channel");

    let mut delta_count = 0usize;
    let mut reconstructed = String::new();
    let mut saw_message_after_deltas = false;

    loop {
        let envelope = match timeout(Duration::from_secs(5), receiver.recv()).await {
            Ok(Ok(envelope)) => envelope,
            Ok(Err(_)) => break,
            Err(_) => panic!("timed out"),
        };
        match envelope.event {
            SSEEvent::MessageDelta { delta, .. } => {
                delta_count += 1;
                reconstructed.push_str(&delta);
            }
            SSEEvent::Message { content, .. } => {
                // The terminal Message content must equal the concatenated deltas.
                if delta_count > 0 {
                    saw_message_after_deltas = true;
                    assert_eq!(reconstructed, content);
                }
            }
            SSEEvent::Complete { .. } => break,
            _ => {}
        }
    }

    assert!(delta_count > 0, "should emit at least one MessageDelta");
    assert!(saw_message_after_deltas, "deltas should be followed by a full Message");
}
