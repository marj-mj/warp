use std::fmt;

#[derive(Debug, Clone)]
pub enum GatewayError {
    ToolNotFound(String),
    InvalidParameters(String),
    ExecutionFailed(String),
    Timeout,
    Cancelled(String),
    PermissionDenied(String),
    IOError(String),
    SerializationError(String),
}

impl fmt::Display for GatewayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ToolNotFound(name) => write!(f, "Tool not found: {}", name),
            Self::InvalidParameters(msg) => write!(f, "Invalid parameters: {}", msg),
            Self::ExecutionFailed(msg) => write!(f, "Execution failed: {}", msg),
            Self::Timeout => write!(f, "Task timed out"),
            Self::Cancelled(msg) => write!(f, "Task was cancelled: {}", msg),
            Self::PermissionDenied(msg) => write!(f, "Permission denied: {}", msg),
            Self::IOError(msg) => write!(f, "IO error: {}", msg),
            Self::SerializationError(msg) => write!(f, "Serialization error: {}", msg),
        }
    }
}

impl std::error::Error for GatewayError {}

impl From<std::io::Error> for GatewayError {
    fn from(err: std::io::Error) -> Self {
        Self::IOError(err.to_string())
    }
}

impl From<serde_json::Error> for GatewayError {
    fn from(err: serde_json::Error) -> Self {
        Self::SerializationError(err.to_string())
    }
}
