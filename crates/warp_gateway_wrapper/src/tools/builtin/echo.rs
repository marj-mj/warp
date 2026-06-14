use async_trait::async_trait;
use serde_json::{json, Value};
use crate::tools::traits::{Tool, ToolResult, ToolContext};

pub struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    
    fn description(&self) -> &str {
        "Echoes back the input message"
    }
    
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message to echo back"
                }
            },
            "required": ["message"]
        })
    }
    
    async fn execute(&self, context: ToolContext, parameters: Value) -> ToolResult<Value> {
        let message = parameters
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                crate::protocol::GatewayError::InvalidParameters(
                    "Missing or invalid 'message' parameter".to_string()
                )
            })?;
        
        Ok(json!({
            "echoed": message,
            "task_id": context.task_id.to_string()
        }))
    }
}
