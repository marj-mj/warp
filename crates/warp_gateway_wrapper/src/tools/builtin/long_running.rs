use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};
use crate::tools::traits::{Tool, ToolResult, ToolContext};

pub struct LongRunningTool;

#[async_trait]
impl Tool for LongRunningTool {
    fn name(&self) -> &str {
        "long_running"
    }
    
    fn description(&self) -> &str {
        "Simulates a long-running operation with cancellation support"
    }
    
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "duration_ms": {
                    "type": "integer",
                    "description": "How long to run for (in milliseconds)",
                    "minimum": 0,
                    "maximum": 60000
                }
            },
            "required": ["duration_ms"]
        })
    }
    
    async fn execute(&self, context: ToolContext, parameters: Value) -> ToolResult<Value> {
        let duration_ms = parameters
            .get("duration_ms")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                crate::protocol::GatewayError::InvalidParameters(
                    "Missing or invalid 'duration_ms' parameter".to_string()
                )
            })?;
        
        // Break the wait into smaller chunks to check for cancellation
        let total_duration = Duration::from_millis(duration_ms);
        let check_interval = Duration::from_millis(100);
        let mut elapsed = Duration::ZERO;
        
        while elapsed < total_duration {
            // Check for cancellation
            if context.is_cancelled() {
                return Err(crate::protocol::GatewayError::Cancelled(
                    format!("Task cancelled after {:?}", elapsed)
                ));
            }
            
            let remaining = total_duration - elapsed;
            let sleep_duration = std::cmp::min(check_interval, remaining);
            
            sleep(sleep_duration).await;
            elapsed += sleep_duration;
        }
        
        Ok(json!({
            "completed": true,
            "duration_ms": duration_ms,
            "task_id": context.task_id.to_string()
        }))
    }
}
