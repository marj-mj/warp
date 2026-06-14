//! System information tool.
//!
//! Reports basic host information (OS, CPU count, memory) via the `sysinfo`
//! crate. Read-only and safe; classified as a basic tool.

use async_trait::async_trait;
use serde_json::{json, Value};
use sysinfo::System;

use crate::tools::traits::{Tool, ToolContext, ToolResult};

pub struct SystemInfoTool;

#[async_trait]
impl Tool for SystemInfoTool {
    fn name(&self) -> &str {
        "system_info"
    }

    fn description(&self) -> &str {
        "Report host system information: OS name/version, kernel, CPU count, and \
         total/used/available memory."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, context: ToolContext, _parameters: Value) -> ToolResult<Value> {
        // sysinfo is synchronous and may briefly block; run it on a blocking
        // thread so we don't stall the async runtime.
        let info = tokio::task::spawn_blocking(|| {
            let mut system = System::new();
            system.refresh_memory();
            system.refresh_cpu_usage();

            json!({
                "os_name": System::name(),
                "os_version": System::os_version(),
                "kernel_version": System::kernel_version(),
                "host_name": System::host_name(),
                "cpu_count": system.cpus().len(),
                "total_memory_bytes": system.total_memory(),
                "used_memory_bytes": system.used_memory(),
                "available_memory_bytes": system.available_memory(),
            })
        })
        .await
        .map_err(|err| {
            crate::protocol::GatewayError::ExecutionFailed(format!(
                "failed to collect system info: {err}"
            ))
        })?;

        let mut info = info;
        if let Value::Object(map) = &mut info {
            map.insert("task_id".to_string(), json!(context.task_id.to_string()));
        }
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::TaskId;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn reports_system_info() {
        let tool = SystemInfoTool;
        let ctx = ToolContext::new(TaskId::new(), CancellationToken::new());
        let result = tool.execute(ctx, json!({})).await.unwrap();
        // cpu_count should be at least 1 on any real host.
        assert!(result["cpu_count"].as_u64().unwrap() >= 1);
        assert!(result.get("total_memory_bytes").is_some());
    }
}
