//! Phase 10: capacity & lifecycle tests.

use std::time::Duration;

use tokio::time::{sleep, timeout};
use warp_gateway_wrapper::gateway::engine::{GatewayConfig, SpawnOutcome};
use warp_gateway_wrapper::http::types::{AgentConfigSnapshot, SpawnAgentRequest, UserQueryMode};
use warp_gateway_wrapper::{ExecutionState, GatewayEngine, SSEEvent, ToolRegistry};

fn never_finishing_request() -> SpawnAgentRequest {
    // Point the harness at a long-running CLI command so the task stays "Running"
    // long enough to observe at_capacity. The CLI harness is portable here.
    if cfg!(windows) {
        std::env::set_var("WARP_GATEWAY_CLAUDE_CMD", "ping");
        std::env::set_var("WARP_GATEWAY_CLAUDE_ARGS", "-n 30 127.0.0.1");
    } else {
        std::env::set_var("WARP_GATEWAY_CLAUDE_CMD", "sleep");
        std::env::set_var("WARP_GATEWAY_CLAUDE_ARGS", "30");
    }
    SpawnAgentRequest {
        prompt: Some("hold".to_string()),
        mode: UserQueryMode::Normal,
        config: Some(AgentConfigSnapshot {
            harness: Some("claude".to_string()),
            ..Default::default()
        }),
        agent_identity_uid: None,
        attachments: Vec::new(),
        initial_snapshot_token: None,
        parent_run_id: None,
    }
}

#[tokio::test]
async fn rejects_spawn_when_at_capacity() {
    let engine = GatewayEngine::with_config(
        ToolRegistry::new(),
        GatewayConfig {
            max_concurrent_tasks: 1,
            completed_ttl_secs: 60,
        },
    );

    let first = engine.spawn_agent(never_finishing_request()).await;
    let task_id = match first {
        SpawnOutcome::Spawned { task_id, .. } => task_id,
        SpawnOutcome::AtCapacity => panic!("first spawn should succeed"),
    };

    // Second spawn while the first is still running -> at capacity.
    assert!(engine.at_capacity().await);
    let second = engine.spawn_agent(never_finishing_request()).await;
    assert!(matches!(second, SpawnOutcome::AtCapacity));

    // Cancel the first task to free capacity, then verify a new spawn succeeds.
    engine.task_manager().cancel_task(&task_id).await;
    // Wait for the harness to react to cancellation.
    let stream_manager = engine.stream_manager();
    if let Some(mut rx) = stream_manager.subscribe(&task_id).await {
        let _ = timeout(Duration::from_secs(5), async {
            while let Ok(envelope) = rx.recv().await {
                if matches!(envelope.event, SSEEvent::Cancelled { .. } | SSEEvent::Complete { .. } | SSEEvent::Error { .. }) {
                    break;
                }
            }
        })
        .await;
    }
    // Give the spawn task a moment to unregister.
    for _ in 0..20 {
        if !engine.at_capacity().await {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(!engine.at_capacity().await, "capacity should free after cancellation");

    let third = engine.spawn_agent(never_finishing_request()).await;
    assert!(matches!(third, SpawnOutcome::Spawned { .. }));

    std::env::remove_var("WARP_GATEWAY_CLAUDE_CMD");
    std::env::remove_var("WARP_GATEWAY_CLAUDE_ARGS");
}

#[tokio::test]
async fn list_executions_includes_spawned_tasks() {
    let engine = GatewayEngine::new(ToolRegistry::new());
    let outcome = engine
        .spawn_agent(SpawnAgentRequest {
            prompt: Some("hi".to_string()),
            mode: UserQueryMode::Normal,
            config: None,
            agent_identity_uid: None,
            attachments: Vec::new(),
            initial_snapshot_token: None,
            parent_run_id: None,
        })
        .await;
    let task_id = match outcome {
        SpawnOutcome::Spawned { task_id, .. } => task_id,
        SpawnOutcome::AtCapacity => panic!("unexpected at_capacity"),
    };

    let listed = engine.list_executions().await;
    assert!(listed.iter().any(|record| record.task_id == task_id));
}

#[tokio::test]
async fn cleanup_expired_removes_terminal_records() {
    // TTL of 0 means everything terminal is immediately reapable.
    let engine = GatewayEngine::with_config(
        ToolRegistry::new(),
        GatewayConfig {
            max_concurrent_tasks: 4,
            completed_ttl_secs: 0,
        },
    );

    let outcome = engine
        .spawn_agent(SpawnAgentRequest {
            prompt: Some("answer please".to_string()),
            mode: UserQueryMode::Normal,
            config: None,
            agent_identity_uid: None,
            attachments: Vec::new(),
            initial_snapshot_token: None,
            parent_run_id: None,
        })
        .await;
    let task_id = match outcome {
        SpawnOutcome::Spawned { task_id, .. } => task_id,
        SpawnOutcome::AtCapacity => panic!("unexpected at_capacity"),
    };

    // Wait for the run to finish (mock provider, no tools -> immediate complete).
    let stream_manager = engine.stream_manager();
    if let Some(mut rx) = stream_manager.subscribe(&task_id).await {
        let _ = timeout(Duration::from_secs(5), async {
            while let Ok(envelope) = rx.recv().await {
                if matches!(envelope.event, SSEEvent::Complete { .. }) {
                    break;
                }
            }
        })
        .await;
    }

    // Poll until the record is marked terminal.
    let mut state = ExecutionState::Running;
    for _ in 0..40 {
        if let Some(record) = engine.get_execution_record(&task_id).await {
            state = record.state.clone();
            if record.is_terminal() {
                break;
            }
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(matches!(state, ExecutionState::Completed | ExecutionState::Failed | ExecutionState::Cancelled));

    let reaped = engine.cleanup_expired().await;
    assert!(reaped >= 1, "should reap at least the completed record");
    assert!(engine.get_execution_record(&task_id).await.is_none());
}
