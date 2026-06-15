//! CLI delegate harness.
//!
//! Delegates a run to an external agent CLI (`claude`, `opencode`, `gemini`,
//! `codex`). The harness spawns the CLI with the prompt, streams its stdout to
//! the client as assistant messages, and maps the process exit status to a
//! terminal `Complete` / `Error` SSE event.
//!
//! The CLI binary and base arguments can be overridden per harness via
//! environment variables so tests (and custom installs) can point at a
//! different executable:
//!
//! - command: `WARP_GATEWAY_<HARNESS>_CMD`  (e.g. `WARP_GATEWAY_CLAUDE_CMD`)
//! - args:    `WARP_GATEWAY_<HARNESS>_ARGS` (space-separated, prepended)

use async_trait::async_trait;
use serde_json::json;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::gateway::task_manager::ProgressUpdate;
use crate::http::sse::SSEEvent;

use super::{Harness, HarnessContext, HarnessType};

pub struct CliHarness {
    harness_type: HarnessType,
}

impl CliHarness {
    pub fn new(harness_type: HarnessType) -> Self {
        Self { harness_type }
    }

    /// Resolve the command + base args for this harness, honoring env overrides.
    fn resolve_command(&self) -> (String, Vec<String>) {
        let default_cmd = self
            .harness_type
            .cli_command()
            .unwrap_or("false") // Oz never reaches here; "false" exits non-zero.
            .to_string();

        let upper = self.harness_type.as_str().to_ascii_uppercase();
        let cmd = std::env::var(format!("WARP_GATEWAY_{upper}_CMD")).unwrap_or(default_cmd);

        let mut args: Vec<String> = std::env::var(format!("WARP_GATEWAY_{upper}_ARGS"))
            .ok()
            .map(|raw| raw.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();

        // Common convention: `-p <prompt>` for a non-interactive print run. Only
        // appended when no explicit args were configured via env.
        if args.is_empty() {
            args.push("-p".to_string());
        }

        (cmd, args)
    }
}

#[async_trait]
impl Harness for CliHarness {
    fn harness_type(&self) -> HarnessType {
        self.harness_type
    }

    async fn run(&self, ctx: HarnessContext) {
        let HarnessContext {
            task_id,
            run_id,
            request,
            identity: _identity,
            engine,
            cancellation_token,
        } = ctx;

        let stream_manager = engine.stream_manager();
        let task_manager = engine.task_manager();
        let harness_name = self.harness_type.as_str();

        let _ = stream_manager
            .send_event(
                &task_id,
                SSEEvent::StreamInit {
                    run_id: run_id.clone(),
                    task_id: task_id.to_string(),
                },
            )
            .await;

        let (command, mut args) = self.resolve_command();
        let prompt = request.prompt.clone().unwrap_or_default();
        args.push(prompt.clone());

        let _ = task_manager
            .send_progress(ProgressUpdate {
                task_id: task_id.clone(),
                progress: 0.1,
                message: Some(format!("Delegating to '{harness_name}' CLI")),
                metadata: Some(json!({
                    "harness": harness_name,
                    "command": command,
                })),
            })
            .await;

        if cancellation_token.is_cancelled() {
            engine
                .fail_execution(
                    &task_id,
                    "cancelled",
                    "Task cancelled before execution".to_string(),
                )
                .await;
            engine.finish_stream(&task_id).await;
            return;
        }

        let mut child = match Command::new(&command)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                let message = format!("failed to launch '{command}' CLI: {err}");
                let _ = stream_manager
                    .send_event(
                        &task_id,
                        SSEEvent::Error {
                            task_id: task_id.to_string(),
                            code: "harness_launch_failed".to_string(),
                            message: message.clone(),
                        },
                    )
                    .await;
                engine
                    .fail_execution(&task_id, "harness_launch_failed", message)
                    .await;
                engine.finish_stream(&task_id).await;
                return;
            }
        };

        // Stream stdout line by line as assistant messages.
        let stdout = child.stdout.take();
        let mut collected = String::new();
        if let Some(stdout) = stdout {
            let mut reader = BufReader::new(stdout).lines();
            loop {
                tokio::select! {
                    biased;

                    _ = cancellation_token.cancelled() => {
                        let _ = child.kill().await;
                        let _ = stream_manager
                            .send_event(
                                &task_id,
                                SSEEvent::Cancelled {
                                    task_id: task_id.to_string(),
                                    reason: Some("Task cancelled during CLI execution".to_string()),
                                },
                            )
                            .await;
                        engine
                            .fail_execution(&task_id, "cancelled", "Task cancelled during execution".to_string())
                            .await;
                        engine.finish_stream(&task_id).await;
                        return;
                    }

                    line = reader.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                collected.push_str(&line);
                                collected.push('\n');
                                let _ = stream_manager
                                    .send_event(
                                        &task_id,
                                        SSEEvent::Message {
                                            task_id: task_id.to_string(),
                                            role: "assistant".to_string(),
                                            content: line,
                                        },
                                    )
                                    .await;
                            }
                            Ok(None) => break, // EOF
                            Err(_) => break,
                        }
                    }
                }
            }
        }

        // Wait for the process and read any stderr for diagnostics.
        let status = child.wait().await;
        let stderr = match child.stderr.take() {
            Some(mut stderr) => {
                use tokio::io::AsyncReadExt;
                let mut buf = String::new();
                let _ = stderr.read_to_string(&mut buf).await;
                buf
            }
            None => String::new(),
        };

        match status {
            Ok(status) if status.success() => {
                let final_result = json!({
                    "harness": harness_name,
                    "command": command,
                    "prompt": prompt,
                    "response": collected.trim_end(),
                    "exit_code": status.code(),
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
            }
            Ok(status) => {
                let message = format!(
                    "'{harness_name}' CLI exited with status {:?}: {}",
                    status.code(),
                    stderr.trim()
                );
                let _ = stream_manager
                    .send_event(
                        &task_id,
                        SSEEvent::Error {
                            task_id: task_id.to_string(),
                            code: "harness_failed".to_string(),
                            message: message.clone(),
                        },
                    )
                    .await;
                engine
                    .fail_execution(&task_id, "harness_failed", message)
                    .await;
            }
            Err(err) => {
                let message = format!("failed to wait for '{harness_name}' CLI: {err}");
                let _ = stream_manager
                    .send_event(
                        &task_id,
                        SSEEvent::Error {
                            task_id: task_id.to_string(),
                            code: "harness_failed".to_string(),
                            message: message.clone(),
                        },
                    )
                    .await;
                engine
                    .fail_execution(&task_id, "harness_failed", message)
                    .await;
            }
        }

        engine.finish_stream(&task_id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_command_uses_default() {
        // Ensure no env override is set for this harness.
        std::env::remove_var("WARP_GATEWAY_CLAUDE_CMD");
        std::env::remove_var("WARP_GATEWAY_CLAUDE_ARGS");
        let harness = CliHarness::new(HarnessType::Claude);
        let (cmd, args) = harness.resolve_command();
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["-p".to_string()]);
    }

    #[test]
    fn resolve_command_honors_env_override() {
        std::env::set_var("WARP_GATEWAY_CODEX_CMD", "my-codex");
        std::env::set_var("WARP_GATEWAY_CODEX_ARGS", "run --quiet");
        let harness = CliHarness::new(HarnessType::Codex);
        let (cmd, args) = harness.resolve_command();
        assert_eq!(cmd, "my-codex");
        assert_eq!(args, vec!["run".to_string(), "--quiet".to_string()]);
        std::env::remove_var("WARP_GATEWAY_CODEX_CMD");
        std::env::remove_var("WARP_GATEWAY_CODEX_ARGS");
    }
}
