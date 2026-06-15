use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::gateway::session::{AgentSession, ExecutionRecord, ExecutionState};
use crate::gateway::task_manager::{ProgressUpdate, TaskManager};
use crate::http::sse::SSEEvent;
use crate::http::types::SpawnAgentRequest;
use crate::tools::{ToolContext, ToolRegistry};
use crate::utils::TaskId;

/// Tunable limits and lifecycle settings for the gateway.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// Maximum number of concurrently running tasks. New spawns are rejected
    /// (reported as t_capacity) once this many tasks are running.
    pub max_concurrent_tasks: usize,
    /// How long (seconds) a terminal execution record is retained before the
    /// cleanup reaper removes it.
    pub completed_ttl_secs: u64,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 16,
            completed_ttl_secs: 3600,
        }
    }
}

/// Result of attempting to spawn an agent.
#[derive(Debug, Clone)]
pub enum SpawnOutcome {
    /// The agent was spawned; carries its task and run identifiers.
    Spawned { task_id: TaskId, run_id: String },
    /// The gateway was at its concurrent-task limit and rejected the spawn.
    AtCapacity,
}

#[derive(Clone)]
pub struct GatewayEngine {
    config: Arc<GatewayConfig>,
    registry: Arc<RwLock<ToolRegistry>>,
    task_manager: Arc<TaskManager>,
    stream_manager: Arc<crate::http::stream_manager::StreamManager>,
    executions: Arc<RwLock<HashMap<TaskId, ExecutionRecord>>>,
}

impl GatewayEngine {
    pub async fn handle_request(
        &self,
        request: crate::protocol::GatewayRequest,
    ) -> crate::protocol::GatewayResponse {
        match request {
            crate::protocol::GatewayRequest::Execute {
                task_id,
                tool_name,
                parameters,
            } => {
                self.execute_tool_request(task_id, tool_name, parameters)
                    .await
            }
            crate::protocol::GatewayRequest::ListTools => {
                let registry = self.registry.read().await;
                crate::protocol::GatewayResponse::ToolsList {
                    tools: registry.tool_definitions(),
                }
            }
            crate::protocol::GatewayRequest::CancelTask { task_id } => {
                let cancelled = self.task_manager.cancel_task(&task_id).await;
                if cancelled {
                    crate::protocol::GatewayResponse::ToolResult {
                        task_id,
                        status: crate::protocol::ExecutionStatus::Cancelled,
                        result: serde_json::json!({ "cancelled": true }),
                    }
                } else {
                    crate::protocol::GatewayResponse::Error {
                        task_id: Some(task_id),
                        code: "task_not_found".to_string(),
                        message: "Task not found or already completed".to_string(),
                    }
                }
            }
            crate::protocol::GatewayRequest::Shutdown => {
                self.task_manager.cancel_all().await;
                crate::protocol::GatewayResponse::Error {
                    task_id: None,
                    code: "shutdown".to_string(),
                    message: "Gateway shutting down".to_string(),
                }
            }
        }
    }

    async fn execute_tool_request(
        &self,
        task_id: TaskId,
        tool_name: String,
        parameters: serde_json::Value,
    ) -> crate::protocol::GatewayResponse {
        let (cancellation_token, _progress_rx) =
            self.task_manager.register_task(task_id.clone()).await;
        let context = ToolContext::new(task_id.clone(), cancellation_token);

        match self
            .execute_named_tool(context, &tool_name, parameters)
            .await
        {
            Ok(result) => {
                self.task_manager.unregister_task(&task_id).await;
                crate::protocol::GatewayResponse::ToolResult {
                    task_id,
                    status: crate::protocol::ExecutionStatus::Success,
                    result,
                }
            }
            Err(err) => {
                self.task_manager.unregister_task(&task_id).await;
                let code = match err {
                    crate::protocol::GatewayError::ToolNotFound(_) => "tool_not_found",
                    crate::protocol::GatewayError::InvalidParameters(_) => "invalid_parameters",
                    crate::protocol::GatewayError::ExecutionFailed(_) => "execution_failed",
                    crate::protocol::GatewayError::Timeout => "timeout",
                    crate::protocol::GatewayError::Cancelled(_) => "cancelled",
                    crate::protocol::GatewayError::PermissionDenied(_) => "permission_denied",
                    crate::protocol::GatewayError::IOError(_) => "io_error",
                    crate::protocol::GatewayError::SerializationError(_) => "serialization_error",
                };

                crate::protocol::GatewayResponse::Error {
                    task_id: Some(task_id),
                    code: code.to_string(),
                    message: err.to_string(),
                }
            }
        }
    }
    pub fn new(registry: ToolRegistry) -> Self {
        Self::with_config(registry, GatewayConfig::default())
    }

    pub fn with_config(registry: ToolRegistry, config: GatewayConfig) -> Self {
        Self {
            config: Arc::new(config),
            registry: Arc::new(RwLock::new(registry)),
            task_manager: Arc::new(TaskManager::new()),
            stream_manager: Arc::new(crate::http::stream_manager::StreamManager::default()),
            executions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn config(&self) -> &GatewayConfig {
        &self.config
    }

    /// Whether the gateway is at its concurrent-task limit.
    pub async fn at_capacity(&self) -> bool {
        self.task_manager.active_count().await >= self.config.max_concurrent_tasks
    }

    pub fn task_manager(&self) -> Arc<TaskManager> {
        self.task_manager.clone()
    }

    pub fn stream_manager(&self) -> Arc<crate::http::stream_manager::StreamManager> {
        self.stream_manager.clone()
    }

    pub async fn spawn_agent(&self, request: SpawnAgentRequest) -> SpawnOutcome {
        // Default to an anonymous privileged identity (used when auth is disabled
        // or when spawning internally).
        self.spawn_agent_with_identity(request, crate::http::auth::Identity::anonymous())
            .await
    }

    pub async fn spawn_agent_with_identity(
        &self,
        request: SpawnAgentRequest,
        identity: crate::http::auth::Identity,
    ) -> SpawnOutcome {
        // Reject new work when at the concurrent-task limit.
        if self.at_capacity().await {
            tracing::warn!(
                max = self.config.max_concurrent_tasks,
                "rejecting spawn: gateway at capacity"
            );
            return SpawnOutcome::AtCapacity;
        }

        let task_id = TaskId::new();
        let run_id = uuid::Uuid::new_v4().to_string();

        let (cancellation_token, progress_rx) =
            self.task_manager.register_task(task_id.clone()).await;
        self.stream_manager.create_channel(task_id.clone()).await;

        self.set_execution_record(ExecutionRecord::new(
            task_id.clone(),
            run_id.clone(),
            ExecutionState::Running,
        ))
        .await;

        // Capture log fields before equest/identity are moved into the session.
        let identity_uid = identity.uid.clone();
        let harness_label =
            crate::harness::HarnessType::from_str_or_default(request.harness()).as_str();
        let session = AgentSession::new(task_id.clone(), run_id.clone(), request, identity);
        let engine = self.clone();
        let progress_engine = engine.clone();
        tokio::spawn(async move {
            progress_engine.forward_progress(progress_rx).await;
        });

        tracing::info!(
            task_id = %task_id,
            run_id = %run_id,
            identity = %identity_uid,
            harness = harness_label,
            "spawned agent"
        );

        tokio::spawn(async move {
            session.run(engine, cancellation_token).await;
        });

        SpawnOutcome::Spawned { task_id, run_id }
    }

    async fn forward_progress(
        &self,
        mut progress_rx: tokio::sync::mpsc::UnboundedReceiver<ProgressUpdate>,
    ) {
        while let Some(update) = progress_rx.recv().await {
            let _ = self
                .stream_manager
                .send_event(
                    &update.task_id,
                    SSEEvent::Progress {
                        task_id: update.task_id.to_string(),
                        progress: update.progress,
                        message: update.message,
                        metadata: update.metadata,
                    },
                )
                .await;

            self.update_progress(&update.task_id, Some(update.progress))
                .await;
        }
    }

    pub async fn execute_named_tool(
        &self,
        context: ToolContext,
        tool_name: &str,
        parameters: serde_json::Value,
    ) -> Result<serde_json::Value, crate::protocol::GatewayError> {
        let tool = {
            let registry = self.registry.read().await;
            registry.get(tool_name)?
        };

        tool.execute(context, parameters).await
    }

    pub async fn list_tool_names(&self) -> Vec<String> {
        let registry = self.registry.read().await;
        registry.list_tools()
    }

    pub async fn list_tool_definitions(&self) -> Vec<crate::protocol::ToolDefinition> {
        let registry = self.registry.read().await;
        registry.tool_definitions()
    }

    pub async fn set_execution_record(&self, record: ExecutionRecord) {
        self.executions
            .write()
            .await
            .insert(record.task_id.clone(), record);
    }

    pub async fn get_execution_record(&self, task_id: &TaskId) -> Option<ExecutionRecord> {
        self.executions.read().await.get(task_id).cloned()
    }

    /// Snapshot of all known execution records (running and terminal).
    pub async fn list_executions(&self) -> Vec<ExecutionRecord> {
        self.executions.read().await.values().cloned().collect()
    }

    /// Count of currently running tasks.
    pub async fn running_count(&self) -> usize {
        self.task_manager.active_count().await
    }

    /// Remove terminal execution records whose completed_at is older than the
    /// configured TTL. Returns the number of records reaped.
    pub async fn cleanup_expired(&self) -> usize {
        let ttl = self.config.completed_ttl_secs;
        let now = crate::gateway::session::now_epoch_secs();
        let mut executions = self.executions.write().await;
        let before = executions.len();
        executions.retain(|_, record| match record.completed_at {
            Some(completed_at) => now.saturating_sub(completed_at) < ttl,
            None => true, // keep running tasks
        });
        before - executions.len()
    }

    /// Spawn a background task that periodically reaps expired records. The
    /// interval is derived from the TTL (checked at least once a minute).
    pub fn start_cleanup_reaper(&self) -> tokio::task::JoinHandle<()> {
        let engine = self.clone();
        let interval_secs = engine.config.completed_ttl_secs.clamp(1, 60);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                ticker.tick().await;
                let reaped = engine.cleanup_expired().await;
                if reaped > 0 {
                    tracing::debug!(reaped, "reaped expired execution records");
                }
            }
        })
    }

    pub async fn update_progress(&self, task_id: &TaskId, progress: Option<f32>) {
        if let Some(record) = self.executions.write().await.get_mut(task_id) {
            record.progress = progress;
        }
    }

    pub async fn complete_execution(&self, task_id: &TaskId, result: serde_json::Value) {
        if let Some(record) = self.executions.write().await.get_mut(task_id) {
            record.state = ExecutionState::Completed;
            record.progress = Some(1.0);
            record.result = Some(result);
            record.error = None;
            record.completed_at = Some(crate::gateway::session::now_epoch_secs());
        }
        self.task_manager.unregister_task(task_id).await;
    }

    pub async fn fail_execution(&self, task_id: &TaskId, code: &str, message: String) {
        if let Some(record) = self.executions.write().await.get_mut(task_id) {
            record.state = if code == "cancelled" {
                ExecutionState::Cancelled
            } else {
                ExecutionState::Failed
            };
            record.error = Some(message.clone());
            record.completed_at = Some(crate::gateway::session::now_epoch_secs());
        }

        let _ = self
            .stream_manager
            .send_event(
                task_id,
                SSEEvent::Error {
                    task_id: task_id.to_string(),
                    code: code.to_string(),
                    message,
                },
            )
            .await;

        self.task_manager.unregister_task(task_id).await;
    }

    pub async fn finish_stream(&self, task_id: &TaskId) {
        self.stream_manager.remove_channel(task_id).await;
    }
}
