use serde::{Deserialize, Serialize};

/// Mode in which the user query is executed, mirroring OZ `UserQueryMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserQueryMode {
    /// Standard single-agent execution.
    #[default]
    Normal,
    /// Planning mode (produce a plan before acting).
    Plan,
    /// Orchestration mode (coordinate multiple agents).
    Orchestrate,
}

/// A file attached to the request, made available to the agent's conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAttachment {
    /// Logical name of the attachment (e.g. file name shown to the user).
    pub name: String,
    /// Optional path relative to the agent working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Optional MIME type (e.g. "text/plain").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Optional inline content for the attachment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Request to spawn an agent, compatible with the OZ `SpawnAgentRequest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnAgentRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Execution mode (normal / plan / orchestrate).
    #[serde(default)]
    pub mode: UserQueryMode,
    /// Configuration snapshot (model, environment, sampling settings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<AgentConfigSnapshot>,
    /// Authentication / identity of the requesting agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_identity_uid: Option<String>,
    /// Files attached to the request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<FileAttachment>,
    /// Snapshot token used for local -> cloud handoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_snapshot_token: Option<String>,
    /// When this is a child agent, the run id of the parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
}

/// Snapshot of the agent configuration, compatible with OZ `AgentConfigSnapshot`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentConfigSnapshot {
    /// Model identifier (e.g. "gpt-4.1", "claude-3-5-sonnet").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Harness that runs the agent (oz / claude / opencode / gemini / codex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// Environment id (e.g. Docker environment for remote execution).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    /// Sampling temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Maximum tokens to generate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Optional system prompt override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Any additional provider-specific settings, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnAgentResponse {
    pub task_id: String,
    pub run_id: String,
    pub at_capacity: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelTaskRequest {
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelTaskResponse {
    pub success: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTaskStatusRequest {
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatusResponse {
    pub task_id: String,
    pub status: TaskStatus,
    pub progress: Option<f32>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl SpawnAgentRequest {
    pub fn model(&self) -> Option<&str> {
        self.config.as_ref()?.model.as_deref()
    }

    pub fn environment_id(&self) -> Option<&str> {
        self.config.as_ref()?.environment_id.as_deref()
    }

    pub fn system_prompt(&self) -> Option<&str> {
        self.config.as_ref()?.system_prompt.as_deref()
    }

    pub fn temperature(&self) -> Option<f32> {
        self.config.as_ref()?.temperature
    }

    pub fn max_tokens(&self) -> Option<u32> {
        self.config.as_ref()?.max_tokens
    }

    pub fn harness(&self) -> Option<&str> {
        self.config.as_ref()?.harness.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_minimal_request() {
        // Only a prompt: mode defaults to normal, everything else empty.
        let json = r#"{ "prompt": "hello" }"#;
        let request: SpawnAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.prompt.as_deref(), Some("hello"));
        assert_eq!(request.mode, UserQueryMode::Normal);
        assert!(request.attachments.is_empty());
        assert!(request.config.is_none());
    }

    #[test]
    fn deserializes_full_oz_payload() {
        let json = r#"{
            "prompt": "do the thing",
            "mode": "orchestrate",
            "config": {
                "model": "claude-3-5-sonnet",
                "environment_id": "env-123",
                "temperature": 0.4,
                "max_tokens": 2048,
                "system_prompt": "be terse",
                "custom_flag": true
            },
            "agent_identity_uid": "user-42",
            "attachments": [
                { "name": "a.txt", "path": "docs/a.txt", "mime_type": "text/plain", "content": "x" }
            ],
            "initial_snapshot_token": "snap-abc",
            "parent_run_id": "run-parent"
        }"#;
        let request: SpawnAgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.mode, UserQueryMode::Orchestrate);
        assert_eq!(request.model(), Some("claude-3-5-sonnet"));
        assert_eq!(request.environment_id(), Some("env-123"));
        assert_eq!(request.system_prompt(), Some("be terse"));
        assert_eq!(request.temperature(), Some(0.4));
        assert_eq!(request.max_tokens(), Some(2048));
        assert_eq!(request.agent_identity_uid.as_deref(), Some("user-42"));
        assert_eq!(request.attachments.len(), 1);
        assert_eq!(request.attachments[0].name, "a.txt");
        assert_eq!(request.initial_snapshot_token.as_deref(), Some("snap-abc"));
        assert_eq!(request.parent_run_id.as_deref(), Some("run-parent"));
        // Unknown config fields are preserved in `extra`.
        let extra = request.config.as_ref().unwrap().extra.as_ref().unwrap();
        assert_eq!(extra["custom_flag"], true);
    }

    #[test]
    fn mode_round_trips() {
        for (mode, text) in [
            (UserQueryMode::Normal, "\"normal\""),
            (UserQueryMode::Plan, "\"plan\""),
            (UserQueryMode::Orchestrate, "\"orchestrate\""),
        ] {
            assert_eq!(serde_json::to_string(&mode).unwrap(), text);
            let parsed: UserQueryMode = serde_json::from_str(text).unwrap();
            assert_eq!(parsed, mode);
        }
    }
}
