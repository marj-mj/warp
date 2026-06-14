use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SSEEvent {
    StreamInit {
        run_id: String,
        task_id: String,
    },
    Progress {
        task_id: String,
        progress: f32,
        message: Option<String>,
        metadata: Option<serde_json::Value>,
    },
    ToolCallStarted {
        task_id: String,
        tool_name: String,
        parameters: serde_json::Value,
    },
    ToolCallCompleted {
        task_id: String,
        tool_name: String,
        result: serde_json::Value,
    },
    ToolCallFailed {
        task_id: String,
        tool_name: String,
        error: String,
    },
    /// Incremental chunk of an assistant message (token-level streaming).
    MessageDelta {
        task_id: String,
        role: String,
        /// The newly produced text fragment to append.
        delta: String,
    },
    /// A complete assistant message (terminates a sequence of MessageDelta).
    Message {
        task_id: String,
        content: String,
        role: String,
    },
    Complete {
        task_id: String,
        result: Option<serde_json::Value>,
    },
    Error {
        task_id: String,
        code: String,
        message: String,
    },
    Cancelled {
        task_id: String,
        reason: Option<String>,
    },
    Ping,
}

impl SSEEvent {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// The SSE `event:` name for this event (derived from the `type` tag).
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::StreamInit { .. } => "stream_init",
            Self::Progress { .. } => "progress",
            Self::ToolCallStarted { .. } => "tool_call_started",
            Self::ToolCallCompleted { .. } => "tool_call_completed",
            Self::ToolCallFailed { .. } => "tool_call_failed",
            Self::MessageDelta { .. } => "message_delta",
            Self::Message { .. } => "message",
            Self::Complete { .. } => "complete",
            Self::Error { .. } => "error",
            Self::Cancelled { .. } => "cancelled",
            Self::Ping => "ping",
        }
    }

    /// Render this event as a raw SSE message block (`data:` only, no id/event).
    pub fn to_sse_message(&self) -> String {
        format!("data: {}\n\n", self.to_json())
    }

    /// Render this event as a full SSE block including `id:` and `event:` lines,
    /// which is what an EventSource client needs for named events and
    /// `Last-Event-ID` based resumption.
    pub fn to_sse_block(&self, id: u64) -> String {
        format!(
            "id: {}\nevent: {}\ndata: {}\n\n",
            id,
            self.event_name(),
            self.to_json()
        )
    }

    pub fn ping() -> Self {
        Self::Ping
    }

    pub fn task_id(&self) -> Option<&str> {
        match self {
            Self::StreamInit { task_id, .. }
            | Self::Progress { task_id, .. }
            | Self::ToolCallStarted { task_id, .. }
            | Self::ToolCallCompleted { task_id, .. }
            | Self::ToolCallFailed { task_id, .. }
            | Self::MessageDelta { task_id, .. }
            | Self::Message { task_id, .. }
            | Self::Complete { task_id, .. }
            | Self::Error { task_id, .. }
            | Self::Cancelled { task_id, .. } => Some(task_id),
            Self::Ping => None,
        }
    }
}

/// An event paired with its monotonic sequence id within a task stream.
///
/// The id is used both for the SSE `id:` field and for `Last-Event-ID` based
/// replay after a client reconnects.
#[derive(Debug, Clone)]
pub struct StreamEnvelope {
    pub id: u64,
    pub event: SSEEvent,
}

impl StreamEnvelope {
    pub fn new(id: u64, event: SSEEvent) -> Self {
        Self { id, event }
    }

    pub fn to_sse_block(&self) -> String {
        self.event.to_sse_block(self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_message_format() {
        let event = SSEEvent::Progress {
            task_id: "test-123".to_string(),
            progress: 0.5,
            message: Some("Working...".to_string()),
            metadata: None,
        };

        let message = event.to_sse_message();
        assert!(message.starts_with("data: "));
        assert!(message.ends_with("\n\n"));
        assert!(message.contains("\"type\":\"progress\""));
    }

    #[test]
    fn test_sse_block_includes_id_and_event() {
        let event = SSEEvent::MessageDelta {
            task_id: "t1".to_string(),
            role: "assistant".to_string(),
            delta: "Hel".to_string(),
        };
        let block = event.to_sse_block(7);
        assert!(block.starts_with("id: 7\n"));
        assert!(block.contains("event: message_delta\n"));
        assert!(block.contains("\"delta\":\"Hel\""));
        assert!(block.ends_with("\n\n"));
    }

    #[test]
    fn test_event_names() {
        assert_eq!(SSEEvent::Ping.event_name(), "ping");
        assert_eq!(
            SSEEvent::Complete { task_id: "t".into(), result: None }.event_name(),
            "complete"
        );
    }

    #[test]
    fn test_extract_task_id() {
        let event = SSEEvent::Complete {
            task_id: "task-456".to_string(),
            result: None,
        };

        assert_eq!(event.task_id(), Some("task-456"));

        let ping = SSEEvent::Ping;
        assert_eq!(ping.task_id(), None);
    }
}
