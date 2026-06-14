use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::gateway::GatewayEngine;
use crate::harness::{resolve_harness, HarnessContext};
use crate::http::types::{SpawnAgentRequest, TaskStatus};
use crate::utils::TaskId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl ExecutionState {
    pub fn as_task_status(&self) -> TaskStatus {
        match self {
            Self::Running => TaskStatus::Running,
            Self::Completed => TaskStatus::Completed,
            Self::Failed => TaskStatus::Failed,
            Self::Cancelled => TaskStatus::Cancelled,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub task_id: TaskId,
    pub run_id: String,
    pub state: ExecutionState,
    pub progress: Option<f32>,
    pub result: Option<Value>,
    pub error: Option<String>,
    /// Unix-epoch seconds at which the task reached a terminal state. None
    /// while running; set on completion/failure/cancellation and used for TTL
    /// based cleanup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
}

impl ExecutionRecord {
    pub fn new(task_id: TaskId, run_id: String, state: ExecutionState) -> Self {
        Self {
            task_id,
            run_id,
            state,
            progress: Some(0.0),
            result: None,
            error: None,
            completed_at: None,
        }
    }

    /// Whether this record is in a terminal (non-running) state.
    pub fn is_terminal(&self) -> bool {
        !matches!(self.state, ExecutionState::Running)
    }
}

/// Current unix-epoch time in seconds.
pub fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A single agent run. The session resolves the configured harness and delegates
/// execution to it; the harness owns the run loop and SSE emission.
#[derive(Clone)]
pub struct AgentSession {
    task_id: TaskId,
    run_id: String,
    request: SpawnAgentRequest,
    identity: crate::http::auth::Identity,
}

impl AgentSession {
    pub fn new(
        task_id: TaskId,
        run_id: String,
        request: SpawnAgentRequest,
        identity: crate::http::auth::Identity,
    ) -> Self {
        Self {
            task_id,
            run_id,
            request,
            identity,
        }
    }

    pub async fn run(self, engine: GatewayEngine, cancellation_token: CancellationToken) {
        let harness = resolve_harness(&self.request);
        let ctx = HarnessContext {
            task_id: self.task_id,
            run_id: self.run_id,
            request: self.request,
            identity: self.identity,
            engine,
            cancellation_token,
        };
        harness.run(ctx).await;
    }
}
