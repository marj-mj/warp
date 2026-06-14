//! Cross-platform shell command execution tool.
//!
//! Runs a command via the platform shell (`cmd /C` on Windows, `sh -c`
//! elsewhere), capturing stdout, stderr, and the exit code. Execution races
//! against both the task cancellation token and an optional timeout; the child
//! process is killed if either fires.

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::timeout;

use crate::protocol::GatewayError;
use crate::tools::traits::{Tool, ToolContext, ToolResult};

/// Hard ceiling on command runtime regardless of the requested timeout.
const MAX_TIMEOUT_MS: u64 = 600_000;
/// Default timeout applied when the caller does not specify one.
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub struct ShellTool;

impl ShellTool {
    fn build_command(command: &str, cwd: Option<&str>, env: Option<&Value>) -> Command {
        let mut cmd = if cfg!(windows) {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg(command);
            cmd
        } else {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(command);
            cmd
        };

        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }

        if let Some(Value::Object(map)) = env {
            for (key, value) in map {
                if let Some(value) = value.as_str() {
                    cmd.env(key, value);
                }
            }
        }

        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "run_shell_command"
    }

    fn description(&self) -> &str {
        "Execute a shell command and capture stdout, stderr, and exit code. \
         Supports an optional working directory, environment overrides, and a timeout."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command line to execute"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory for the command (optional)"
                },
                "env": {
                    "type": "object",
                    "description": "Environment variable overrides as string key/value pairs",
                    "additionalProperties": { "type": "string" }
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Maximum runtime in milliseconds (default 30000, max 600000)",
                    "minimum": 0,
                    "maximum": 600000
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, context: ToolContext, parameters: Value) -> ToolResult<Value> {
        let command = parameters
            .get("command")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                GatewayError::InvalidParameters("Missing or empty 'command' parameter".to_string())
            })?;

        let cwd = parameters.get("cwd").and_then(Value::as_str);
        let env = parameters.get("env");
        let timeout_ms = parameters
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        let child = Self::build_command(command, cwd, env)
            .spawn()
            .map_err(|err| GatewayError::ExecutionFailed(format!("failed to spawn command: {err}")))?;

        let wait_for_output = async {
            child.wait_with_output().await.map_err(|err| {
                GatewayError::ExecutionFailed(format!("failed to wait for command: {err}"))
            })
        };

        let output = tokio::select! {
            biased;

            _ = context.cancelled() => {
                return Err(GatewayError::Cancelled(
                    "Shell command cancelled before completion".to_string(),
                ));
            }

            result = timeout(Duration::from_millis(timeout_ms), wait_for_output) => {
                match result {
                    Ok(output) => output?,
                    Err(_) => {
                        return Err(GatewayError::Timeout);
                    }
                }
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code();

        Ok(json!({
            "command": command,
            "exit_code": exit_code,
            "success": output.status.success(),
            "stdout": stdout,
            "stderr": stderr,
            "task_id": context.task_id.to_string()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::TaskId;
    use tokio_util::sync::CancellationToken;

    fn context() -> ToolContext {
        ToolContext::new(TaskId::new(), CancellationToken::new())
    }

    #[tokio::test]
    async fn runs_simple_command() {
        let tool = ShellTool;
        // `echo hello` works on both cmd and sh.
        let result = tool
            .execute(context(), json!({ "command": "echo hello" }))
            .await
            .unwrap();
        assert_eq!(result["success"], true);
        assert_eq!(result["exit_code"], 0);
        assert!(result["stdout"].as_str().unwrap().contains("hello"));
    }

    #[tokio::test]
    async fn missing_command_is_invalid() {
        let tool = ShellTool;
        let err = tool.execute(context(), json!({})).await.unwrap_err();
        assert!(matches!(err, GatewayError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn reports_nonzero_exit() {
        let tool = ShellTool;
        let command = if cfg!(windows) { "exit 3" } else { "exit 3" };
        let result = tool
            .execute(context(), json!({ "command": command }))
            .await
            .unwrap();
        assert_eq!(result["success"], false);
        assert_eq!(result["exit_code"], 3);
    }

    #[tokio::test]
    async fn times_out_long_command() {
        let tool = ShellTool;
        let command = if cfg!(windows) {
            // ping with delay is a portable-ish sleep on Windows.
            "ping -n 10 127.0.0.1 >NUL"
        } else {
            "sleep 5"
        };
        let err = tool
            .execute(context(), json!({ "command": command, "timeout_ms": 200 }))
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::Timeout));
    }

    #[tokio::test]
    async fn cancels_running_command() {
        let tool = ShellTool;
        let token = CancellationToken::new();
        let ctx = ToolContext::new(TaskId::new(), token.clone());
        token.cancel();
        let command = if cfg!(windows) { "ping -n 5 127.0.0.1 >NUL" } else { "sleep 5" };
        let err = tool
            .execute(ctx, json!({ "command": command, "timeout_ms": 5000 }))
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::Cancelled(_)));
    }
}
