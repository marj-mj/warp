//! Deterministic offline provider used when no API credentials are available.
//!
//! The mock provider keeps the gateway fully functional without network access.
//! Its behavior is intentionally simple and predictable so it can be relied upon
//! in tests and local development:
//!
//! 1. On the first turn, if a tool named `echo` is available and the conversation
//!    has not yet produced a tool result, it requests a single `echo` tool call
//!    using the latest user prompt.
//! 2. Once a tool result is present (or no tools are available), it returns a
//!    final assistant turn that summarizes what happened.

use async_trait::async_trait;
use serde_json::json;

use crate::protocol::GatewayError;

use super::{AssistantTurn, ChatMessage, CompletionRequest, LlmProvider, Role, ToolCall};

#[derive(Debug, Default, Clone)]
pub struct MockProvider;

impl MockProvider {
    pub fn new() -> Self {
        Self
    }

    fn latest_user_prompt(messages: &[ChatMessage]) -> String {
        messages
            .iter()
            .rev()
            .find(|message| message.role == Role::User)
            .map(|message| message.content.clone())
            .unwrap_or_default()
    }

    fn has_tool_result(messages: &[ChatMessage]) -> bool {
        messages.iter().any(|message| message.role == Role::Tool)
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    async fn step(&self, request: &CompletionRequest) -> Result<AssistantTurn, GatewayError> {
        let prompt = Self::latest_user_prompt(&request.messages);
        let echo_available = request.tools.iter().any(|tool| tool.name == "echo");
        let already_used_tool = Self::has_tool_result(&request.messages);

        // First turn with an echo tool available: request a tool call so the
        // full agent loop (tool execution + feedback) is exercised offline.
        if echo_available && !already_used_tool {
            let tool_call = ToolCall {
                id: "mock-call-1".to_string(),
                name: "echo".to_string(),
                arguments: json!({
                    "message": if prompt.is_empty() {
                        "mock provider initialization".to_string()
                    } else {
                        prompt.clone()
                    }
                }),
            };
            return Ok(AssistantTurn {
                content: "Using the echo tool to process the request.".to_string(),
                tool_calls: vec![tool_call],
            });
        }

        // Final turn: summarize the outcome.
        let tool_summary = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::Tool)
            .map(|message| message.content.clone());

        let content = match tool_summary {
            Some(result) => format!(
                "[mock] Completed request '{}'. Tool result: {}",
                prompt, result
            ),
            None => format!(
                "[mock] Completed request '{}' without tool usage.",
                prompt
            ),
        };

        Ok(AssistantTurn {
            content,
            tool_calls: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ToolDefinition;

    fn echo_tool_def() -> ToolDefinition {
        ToolDefinition {
            name: "echo".to_string(),
            description: "echo".to_string(),
            parameters_schema: json!({}),
        }
    }

    #[tokio::test]
    async fn requests_echo_on_first_turn() {
        let provider = MockProvider::new();
        let request = CompletionRequest {
            model: Some("gpt-4".to_string()),
            messages: vec![ChatMessage::user("hello")],
            tools: vec![echo_tool_def()],
            temperature: None,
            max_tokens: None,
        };

        let turn = provider.step(&request).await.unwrap();
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "echo");
        assert_eq!(turn.tool_calls[0].arguments["message"], "hello");
    }

    #[tokio::test]
    async fn finalizes_after_tool_result() {
        let provider = MockProvider::new();
        let request = CompletionRequest {
            model: None,
            messages: vec![
                ChatMessage::user("hello"),
                ChatMessage::assistant_with_tool_calls(
                    "",
                    vec![ToolCall {
                        id: "mock-call-1".to_string(),
                        name: "echo".to_string(),
                        arguments: json!({"message": "hello"}),
                    }],
                ),
                ChatMessage::tool_result("mock-call-1", "{\"echoed\":\"hello\"}"),
            ],
            tools: vec![echo_tool_def()],
            temperature: None,
            max_tokens: None,
        };

        let turn = provider.step(&request).await.unwrap();
        assert!(turn.is_final());
        assert!(turn.content.contains("hello"));
    }

    #[tokio::test]
    async fn finalizes_without_tools() {
        let provider = MockProvider::new();
        let request = CompletionRequest {
            model: None,
            messages: vec![ChatMessage::user("just answer")],
            tools: vec![],
            temperature: None,
            max_tokens: None,
        };

        let turn = provider.step(&request).await.unwrap();
        assert!(turn.is_final());
    }
}
