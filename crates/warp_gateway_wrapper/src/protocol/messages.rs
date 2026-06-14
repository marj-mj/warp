use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::utils::TaskId;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum GatewayRequest {
    Execute {
        task_id: TaskId,
        tool_name: String,
        parameters: Value,
    },
    CancelTask {
        task_id: TaskId,
    },
    ListTools,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum GatewayResponse {
    ToolResult {
        task_id: TaskId,
        result: Value,
        status: ExecutionStatus,
    },
    Error {
        task_id: Option<TaskId>,
        code: String,
        message: String,
    },
    ToolsList {
        tools: Vec<ToolDefinition>,
    },
    Ack {
        task_id: TaskId,
    },
    Progress {
        task_id: TaskId,
        data: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionStatus {
    Success,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters_schema: Value,
}
