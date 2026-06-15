//! The built-in `oz` harness: the gateway's native agent loop.
//!
//! Drives an LLM provider, executes requested tool calls through the gateway,
//! feeds results back, and streams progress/messages/tool events over SSE until
//! the provider returns a final turn (or limits/cancellation stop the loop).

use async_trait::async_trait;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::gateway::task_manager::ProgressUpdate;
use crate::gateway::GatewayEngine;
use crate::http::auth::Identity;
use crate::http::sse::SSEEvent;
use crate::http::types::FileAttachment;
use crate::providers::{
    resolve_provider, ChatMessage, CompletionRequest, ProviderFamily, ProviderSettings, ToolCall,
};
use crate::tools::ToolContext;
use crate::utils::TaskId;

use super::{Harness, HarnessContext, HarnessType};

/// Maximum number of provider <-> tool round trips before the loop stops.
const MAX_TURNS: usize = 8;

const SYSTEM_PROMPT: &str = "You are a Warp gateway agent. Use the available tools when they help \
you fulfill the user's request, then provide a concise final answer.";

#[derive(Debug, Default, Clone)]
pub struct OzHarness;

impl OzHarness {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Harness for OzHarness {
    fn harness_type(&self) -> HarnessType {
        HarnessType::Oz
    }

    async fn run(&self, ctx: HarnessContext) {
        let HarnessContext {
            task_id,
            run_id,
            request,
            identity,
            engine,
            cancellation_token,
        } = ctx;

        let stream_manager = engine.stream_manager();
        let task_manager = engine.task_manager();

        let _ = stream_manager
            .send_event(
                &task_id,
                SSEEvent::StreamInit {
                    run_id: run_id.clone(),
                    task_id: task_id.to_string(),
                },
            )
            .await;

        let model = request.model().map(str::to_string);
        let family = ProviderFamily::from_model(model.as_deref());
        let settings = ProviderSettings::from_env(family);
        let provider = resolve_provider(model.as_deref(), settings);

        let _ = task_manager
            .send_progress(ProgressUpdate {
                task_id: task_id.clone(),
                progress: 0.1,
                message: Some("Session initialized".to_string()),
                metadata: Some(json!({
                    "model": model,
                    "mode": request.mode,
                    "harness": "oz",
                    "provider": provider.name(),
                    "provider_family": family.as_str(),
                })),
            })
            .await;

        if cancellation_token.is_cancelled() {
            engine
                .fail_execution(
                    &task_id,
                    "cancelled",
                    "Task cancelled before execution started".to_string(),
                )
                .await;
            engine.finish_stream(&task_id).await;
            return;
        }

        let prompt = request.prompt.clone().unwrap_or_default();
        let tools = engine.list_tool_definitions().await;

        let _ = stream_manager
            .send_event(
                &task_id,
                SSEEvent::Message {
                    task_id: task_id.to_string(),
                    role: "assistant".to_string(),
                    content: format!(
                        "Starting agent session with provider '{}' and {} available tool(s).",
                        provider.name(),
                        tools.len()
                    ),
                },
            )
            .await;

        // Seed the conversation. A config-provided system prompt overrides the default.
        let system_prompt = request
            .system_prompt()
            .map(str::to_string)
            .unwrap_or_else(|| SYSTEM_PROMPT.to_string());
        let mut messages = vec![ChatMessage::system(system_prompt)];

        // Surface any attachments to the model as a system-context message.
        if !request.attachments.is_empty() {
            messages.push(ChatMessage::system(format_attachments(
                &request.attachments,
            )));
        }

        messages.push(ChatMessage::user(if prompt.is_empty() {
            format!("(no prompt provided; run_id={})", run_id)
        } else {
            prompt.clone()
        }));

        let temperature = request.temperature();
        let max_tokens = request.max_tokens();

        let mut final_content = String::new();

        for turn in 0..MAX_TURNS {
            if cancellation_token.is_cancelled() {
                let _ = stream_manager
                    .send_event(
                        &task_id,
                        SSEEvent::Cancelled {
                            task_id: task_id.to_string(),
                            reason: Some("Task cancelled during execution".to_string()),
                        },
                    )
                    .await;
                engine
                    .fail_execution(
                        &task_id,
                        "cancelled",
                        "Task cancelled during execution".to_string(),
                    )
                    .await;
                engine.finish_stream(&task_id).await;
                return;
            }

            let progress = 0.2 + (turn as f32 / MAX_TURNS as f32) * 0.6;
            let _ = task_manager
                .send_progress(ProgressUpdate {
                    task_id: task_id.clone(),
                    progress,
                    message: Some(format!("Provider turn {}", turn + 1)),
                    metadata: None,
                })
                .await;

            let completion_request = CompletionRequest {
                model: model.clone(),
                messages: messages.clone(),
                tools: tools.clone(),
                temperature,
                max_tokens,
            };

            let assistant_turn = match provider.step(&completion_request).await {
                Ok(turn) => turn,
                Err(err) => {
                    let code = error_code(&err);
                    let _ = stream_manager
                        .send_event(
                            &task_id,
                            SSEEvent::Error {
                                task_id: task_id.to_string(),
                                code: code.to_string(),
                                message: err.to_string(),
                            },
                        )
                        .await;
                    engine.fail_execution(&task_id, code, err.to_string()).await;
                    engine.finish_stream(&task_id).await;
                    return;
                }
            };

            if !assistant_turn.content.is_empty() {
                stream_assistant_message(&stream_manager, &task_id, &assistant_turn.content).await;
            }

            // Final turn: no tool calls requested.
            if assistant_turn.is_final() {
                final_content = assistant_turn.content;
                break;
            }

            // Record the assistant's tool-call request in history.
            messages.push(ChatMessage::assistant_with_tool_calls(
                assistant_turn.content.clone(),
                assistant_turn.tool_calls.clone(),
            ));

            // Execute each requested tool call and feed back the results.
            for tool_call in &assistant_turn.tool_calls {
                let result_message =
                    execute_tool_call(&engine, &identity, &task_id, &cancellation_token, tool_call)
                        .await;
                messages.push(result_message);
            }

            if turn == MAX_TURNS - 1 {
                final_content =
                    "Reached maximum number of agent turns without a final answer.".to_string();
            }
        }

        let final_result = json!({
            "harness": "oz",
            "provider": provider.name(),
            "model": model,
            "prompt": prompt,
            "response": final_content,
        });

        let _ = stream_manager
            .send_event(
                &task_id,
                SSEEvent::Complete {
                    task_id: task_id.to_string(),
                    result: Some(final_result.clone()),
                },
            )
            .await;

        engine.complete_execution(&task_id, final_result).await;
        engine.finish_stream(&task_id).await;
    }
}

/// Execute a single tool call, emit the appropriate SSE events, and return a
/// `Role::Tool` message capturing the result (or error) for the next turn.
async fn execute_tool_call(
    engine: &GatewayEngine,
    identity: &Identity,
    task_id: &TaskId,
    cancellation_token: &CancellationToken,
    tool_call: &ToolCall,
) -> ChatMessage {
    let stream_manager = engine.stream_manager();

    let _ = stream_manager
        .send_event(
            task_id,
            SSEEvent::ToolCallStarted {
                task_id: task_id.to_string(),
                tool_name: tool_call.name.clone(),
                parameters: tool_call.arguments.clone(),
            },
        )
        .await;

    // Authorization check: reject tool calls the identity is not allowed to make.
    if !identity.can_use_tool(&tool_call.name) {
        let message = format!(
            "identity '{}' is not authorized to use tool '{}'",
            identity.uid, tool_call.name
        );
        let _ = stream_manager
            .send_event(
                task_id,
                SSEEvent::ToolCallFailed {
                    task_id: task_id.to_string(),
                    tool_name: tool_call.name.clone(),
                    error: message.clone(),
                },
            )
            .await;
        return ChatMessage::tool_result(
            tool_call.id.clone(),
            json!({ "error": message }).to_string(),
        );
    }

    let context = ToolContext::new(task_id.clone(), cancellation_token.clone());
    let execution = engine
        .execute_named_tool(context, &tool_call.name, tool_call.arguments.clone())
        .await;

    match execution {
        Ok(result) => {
            let _ = stream_manager
                .send_event(
                    task_id,
                    SSEEvent::ToolCallCompleted {
                        task_id: task_id.to_string(),
                        tool_name: tool_call.name.clone(),
                        result: result.clone(),
                    },
                )
                .await;
            ChatMessage::tool_result(tool_call.id.clone(), result.to_string())
        }
        Err(err) => {
            let _ = stream_manager
                .send_event(
                    task_id,
                    SSEEvent::ToolCallFailed {
                        task_id: task_id.to_string(),
                        tool_name: tool_call.name.clone(),
                        error: err.to_string(),
                    },
                )
                .await;
            ChatMessage::tool_result(
                tool_call.id.clone(),
                json!({ "error": err.to_string() }).to_string(),
            )
        }
    }
}

/// Stream an assistant message as a sequence of `MessageDelta` events followed
/// by a terminal `Message` event with the full content.
async fn stream_assistant_message(
    stream_manager: &std::sync::Arc<crate::http::stream_manager::StreamManager>,
    task_id: &TaskId,
    content: &str,
) {
    for chunk in chunk_text(content) {
        let _ = stream_manager
            .send_event(
                task_id,
                SSEEvent::MessageDelta {
                    task_id: task_id.to_string(),
                    role: "assistant".to_string(),
                    delta: chunk,
                },
            )
            .await;
    }

    let _ = stream_manager
        .send_event(
            task_id,
            SSEEvent::Message {
                task_id: task_id.to_string(),
                role: "assistant".to_string(),
                content: content.to_string(),
            },
        )
        .await;
}

/// Split text into delta chunks on whitespace boundaries, preserving trailing
/// whitespace with each word so concatenating deltas reproduces the original.
fn chunk_text(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if ch.is_whitespace() {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Render attachments into a compact textual context block for the model.
fn format_attachments(attachments: &[FileAttachment]) -> String {
    let mut out = String::from("The user attached the following file(s):\n");
    for (index, attachment) in attachments.iter().enumerate() {
        out.push_str(&format!("\n[{}] {}", index + 1, attachment.name));
        if let Some(path) = &attachment.path {
            out.push_str(&format!(" (path: {path})"));
        }
        if let Some(mime) = &attachment.mime_type {
            out.push_str(&format!(" [{mime}]"));
        }
        if let Some(content) = &attachment.content {
            const MAX: usize = 4000;
            let body = if content.len() > MAX {
                format!("{}... (truncated)", &content[..MAX])
            } else {
                content.clone()
            };
            out.push_str(&format!("\n---\n{body}\n---"));
        }
    }
    out
}

fn error_code(err: &crate::protocol::GatewayError) -> &'static str {
    match err {
        crate::protocol::GatewayError::Cancelled(_) => "cancelled",
        crate::protocol::GatewayError::InvalidParameters(_) => "invalid_parameters",
        crate::protocol::GatewayError::ToolNotFound(_) => "tool_not_found",
        crate::protocol::GatewayError::ExecutionFailed(_) => "execution_failed",
        crate::protocol::GatewayError::Timeout => "timeout",
        crate::protocol::GatewayError::PermissionDenied(_) => "permission_denied",
        crate::protocol::GatewayError::IOError(_) => "io_error",
        crate::protocol::GatewayError::SerializationError(_) => "serialization_error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_text_reconstructs_original() {
        let text = "hello world from oz";
        let chunks = chunk_text(text);
        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn chunk_text_handles_empty() {
        assert!(chunk_text("").is_empty());
    }

    #[test]
    fn format_attachments_includes_names() {
        let attachments = vec![FileAttachment {
            name: "a.txt".to_string(),
            path: Some("docs/a.txt".to_string()),
            mime_type: Some("text/plain".to_string()),
            content: Some("hi".to_string()),
        }];
        let rendered = format_attachments(&attachments);
        assert!(rendered.contains("a.txt"));
        assert!(rendered.contains("docs/a.txt"));
        assert!(rendered.contains("hi"));
    }
}
