//! HTTP network request tool.
//!
//! Performs an outbound HTTP request and returns the status, headers, and body.
//! Includes a basic SSRF guard that rejects requests to loopback, private, and
//! link-local hosts (including the cloud metadata address) unless explicitly
//! allowed. This is a defense-in-depth measure, not a complete SSRF mitigation.

use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::time::timeout;

use crate::protocol::GatewayError;
use crate::tools::traits::{Tool, ToolContext, ToolResult};

const DEFAULT_TIMEOUT_MS: u64 = 15_000;
const MAX_TIMEOUT_MS: u64 = 120_000;
/// Cap the response body captured into the result to keep payloads bounded.
const MAX_BODY_BYTES: usize = 256 * 1024;

pub struct NetworkTool {
    client: reqwest::Client,
    /// When true, requests to private/loopback addresses are allowed (e.g. for
    /// talking to a local managed gateway). Off by default.
    allow_private: bool,
}

impl Default for NetworkTool {
    fn default() -> Self {
        Self::new(false)
    }
}

impl NetworkTool {
    pub fn new(allow_private: bool) -> Self {
        Self {
            client: reqwest::Client::new(),
            allow_private,
        }
    }

    /// Allow requests to private/loopback hosts (use with care).
    pub fn allowing_private() -> Self {
        Self::new(true)
    }

    /// Reject obviously-internal targets to provide a basic SSRF guard.
    fn check_host(&self, url: &url::Url) -> Result<(), GatewayError> {
        if self.allow_private {
            return Ok(());
        }

        let scheme = url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(GatewayError::InvalidParameters(format!(
                "unsupported URL scheme '{scheme}' (only http/https allowed)"
            )));
        }

        let host = url.host_str().ok_or_else(|| {
            GatewayError::InvalidParameters("URL has no host".to_string())
        })?;

        // Block obvious internal hostnames.
        let lowered = host.to_ascii_lowercase();
        if lowered == "localhost" || lowered.ends_with(".localhost") {
            return Err(GatewayError::PermissionDenied(
                "requests to localhost are not allowed".to_string(),
            ));
        }

        // If the host parses as an IP, reject loopback/private/link-local ranges.
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_blocked_ip(&ip) {
                return Err(GatewayError::PermissionDenied(format!(
                    "requests to internal address {ip} are not allowed"
                )));
            }
        }

        Ok(())
    }
}

/// Whether an IP falls in a range that should be blocked by the SSRF guard.
fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                // Cloud metadata endpoint.
                || v4.octets() == [169, 254, 169, 254]
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

#[async_trait]
impl Tool for NetworkTool {
    fn name(&self) -> &str {
        "http_request"
    }

    fn description(&self) -> &str {
        "Perform an outbound HTTP request (GET/POST/PUT/PATCH/DELETE/HEAD) and \
         return the status, headers, and (bounded) body. Internal/loopback hosts \
         are blocked."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The absolute http(s) URL" },
                "method": {
                    "type": "string",
                    "description": "HTTP method (default GET)",
                    "enum": ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"]
                },
                "headers": {
                    "type": "object",
                    "description": "Request headers as string key/value pairs",
                    "additionalProperties": { "type": "string" }
                },
                "body": { "type": "string", "description": "Optional request body" },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Request timeout in milliseconds (default 15000, max 120000)",
                    "minimum": 0,
                    "maximum": 120000
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, context: ToolContext, parameters: Value) -> ToolResult<Value> {
        if context.is_cancelled() {
            return Err(GatewayError::Cancelled("Request cancelled".to_string()));
        }

        let url_str = parameters
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| GatewayError::InvalidParameters("Missing 'url' parameter".to_string()))?;

        let url = url::Url::parse(url_str)
            .map_err(|err| GatewayError::InvalidParameters(format!("invalid URL: {err}")))?;
        self.check_host(&url)?;

        let method_str = parameters
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_ascii_uppercase();
        let method = reqwest::Method::from_bytes(method_str.as_bytes())
            .map_err(|_| GatewayError::InvalidParameters(format!("invalid method '{method_str}'")))?;

        let timeout_ms = parameters
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        let mut builder = self.client.request(method, url);

        if let Some(Value::Object(headers)) = parameters.get("headers") {
            for (key, value) in headers {
                if let Some(value) = value.as_str() {
                    builder = builder.header(key, value);
                }
            }
        }

        if let Some(body) = parameters.get("body").and_then(Value::as_str) {
            builder = builder.body(body.to_string());
        }

        let response = match timeout(Duration::from_millis(timeout_ms), builder.send()).await {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                return Err(GatewayError::ExecutionFailed(format!("request failed: {err}")))
            }
            Err(_) => return Err(GatewayError::Timeout),
        };

        let status = response.status();
        let mut headers = serde_json::Map::new();
        for (name, value) in response.headers() {
            if let Ok(value) = value.to_str() {
                headers.insert(name.to_string(), json!(value));
            }
        }

        let full_body = response
            .text()
            .await
            .map_err(|err| GatewayError::ExecutionFailed(format!("failed to read body: {err}")))?;
        let truncated = full_body.len() > MAX_BODY_BYTES;
        let body = if truncated {
            full_body.chars().take(MAX_BODY_BYTES).collect::<String>()
        } else {
            full_body
        };

        Ok(json!({
            "status": status.as_u16(),
            "success": status.is_success(),
            "headers": Value::Object(headers),
            "body": body,
            "truncated": truncated,
            "task_id": context.task_id.to_string(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::TaskId;
    use tokio_util::sync::CancellationToken;

    fn ctx() -> ToolContext {
        ToolContext::new(TaskId::new(), CancellationToken::new())
    }

    #[test]
    fn blocks_internal_ips() {
        assert!(is_blocked_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip(&"10.0.0.5".parse().unwrap()));
        assert!(is_blocked_ip(&"192.168.1.1".parse().unwrap()));
        assert!(is_blocked_ip(&"169.254.169.254".parse().unwrap()));
        assert!(!is_blocked_ip(&"8.8.8.8".parse().unwrap()));
    }

    #[tokio::test]
    async fn rejects_localhost() {
        let tool = NetworkTool::default();
        let err = tool
            .execute(ctx(), json!({ "url": "http://localhost:8080/x" }))
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::PermissionDenied(_)));
    }

    #[tokio::test]
    async fn rejects_private_ip() {
        let tool = NetworkTool::default();
        let err = tool
            .execute(ctx(), json!({ "url": "http://192.168.0.1/" }))
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::PermissionDenied(_)));
    }

    #[tokio::test]
    async fn rejects_non_http_scheme() {
        let tool = NetworkTool::default();
        let err = tool
            .execute(ctx(), json!({ "url": "ftp://example.com/" }))
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn missing_url_is_invalid() {
        let tool = NetworkTool::default();
        let err = tool.execute(ctx(), json!({})).await.unwrap_err();
        assert!(matches!(err, GatewayError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn allow_private_permits_localhost_host_check() {
        // With allow_private, the host check passes (the actual request may still
        // fail to connect, which is fine — we only assert the guard is bypassed).
        let tool = NetworkTool::allowing_private();
        let url = url::Url::parse("http://127.0.0.1:0/").unwrap();
        assert!(tool.check_host(&url).is_ok());
    }
}
