use crate::protocol::{GatewayError, ToolDefinition};
use crate::utils::TaskId;
use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub type ToolResult<T> = Result<T, GatewayError>;

/// Context passed to tool execution
pub struct ToolContext {
    pub task_id: TaskId,
    pub cancellation_token: CancellationToken,
}

impl ToolContext {
    pub fn new(task_id: TaskId, cancellation_token: CancellationToken) -> Self {
        Self {
            task_id,
            cancellation_token,
        }
    }

    /// Check if this task has been cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancellation_token.is_cancelled()
    }

    /// Get a future that completes when the task is cancelled
    pub fn cancelled(&self) -> tokio_util::sync::WaitForCancellationFuture<'_> {
        self.cancellation_token.cancelled()
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;

    /// Execute the tool with cancellation support
    async fn execute(&self, context: ToolContext, parameters: Value) -> ToolResult<Value>;

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters_schema: self.parameters_schema(),
        }
    }
}
