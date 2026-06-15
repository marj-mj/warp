//! Filesystem operations tool with path confinement.
//!
//! Supports `read`, `write`, `list`, and `delete` actions. All paths are resolved
//! against a configured root directory and rejected if they escape it, preventing
//! directory traversal. The default root is the current working directory.

use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::protocol::GatewayError;
use crate::tools::traits::{Tool, ToolContext, ToolResult};

pub struct FilesystemTool {
    root: PathBuf,
}

impl Default for FilesystemTool {
    fn default() -> Self {
        Self::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

impl FilesystemTool {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        // Best-effort canonicalization; fall back to the raw path when the root
        // does not yet exist so the tool can still be constructed.
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        Self { root }
    }

    /// Normalize a path lexically (without touching the filesystem) so traversal
    /// can be checked even for files that do not exist yet.
    fn normalize(path: &Path) -> PathBuf {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::ParentDir => {
                    normalized.pop();
                }
                Component::CurDir => {}
                other => normalized.push(other.as_os_str()),
            }
        }
        normalized
    }

    /// Resolve a caller-supplied relative path against the root, rejecting any
    /// path that escapes the root directory.
    fn resolve(&self, requested: &str) -> Result<PathBuf, GatewayError> {
        let requested_path = Path::new(requested);
        if requested_path.is_absolute() {
            return Err(GatewayError::InvalidParameters(
                "absolute paths are not allowed; use a path relative to the root".to_string(),
            ));
        }

        let joined = Self::normalize(&self.root.join(requested_path));
        if !joined.starts_with(&self.root) {
            return Err(GatewayError::InvalidParameters(format!(
                "path '{requested}' escapes the allowed root directory"
            )));
        }
        Ok(joined)
    }

    async fn read(&self, path: PathBuf) -> ToolResult<Value> {
        let contents = tokio::fs::read_to_string(&path).await.map_err(|err| {
            GatewayError::IOError(format!("failed to read {}: {err}", path.display()))
        })?;
        Ok(json!({
            "action": "read",
            "path": path.display().to_string(),
            "contents": contents,
        }))
    }

    async fn write(&self, path: PathBuf, contents: &str) -> ToolResult<Value> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|err| {
                GatewayError::IOError(format!("failed to create {}: {err}", parent.display()))
            })?;
        }
        tokio::fs::write(&path, contents).await.map_err(|err| {
            GatewayError::IOError(format!("failed to write {}: {err}", path.display()))
        })?;
        Ok(json!({
            "action": "write",
            "path": path.display().to_string(),
            "bytes_written": contents.len(),
        }))
    }

    async fn list(&self, path: PathBuf) -> ToolResult<Value> {
        let mut entries = tokio::fs::read_dir(&path).await.map_err(|err| {
            GatewayError::IOError(format!("failed to list {}: {err}", path.display()))
        })?;

        let mut items = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|err| GatewayError::IOError(format!("failed to read entry: {err}")))?
        {
            let file_type = entry.file_type().await.ok();
            items.push(json!({
                "name": entry.file_name().to_string_lossy().to_string(),
                "is_dir": file_type.map(|t| t.is_dir()).unwrap_or(false),
            }));
        }

        Ok(json!({
            "action": "list",
            "path": path.display().to_string(),
            "entries": items,
        }))
    }

    async fn delete(&self, path: PathBuf) -> ToolResult<Value> {
        let metadata = tokio::fs::metadata(&path).await.map_err(|err| {
            GatewayError::IOError(format!("failed to stat {}: {err}", path.display()))
        })?;

        if metadata.is_dir() {
            tokio::fs::remove_dir_all(&path).await.map_err(|err| {
                GatewayError::IOError(format!("failed to delete dir {}: {err}", path.display()))
            })?;
        } else {
            tokio::fs::remove_file(&path).await.map_err(|err| {
                GatewayError::IOError(format!("failed to delete file {}: {err}", path.display()))
            })?;
        }

        Ok(json!({
            "action": "delete",
            "path": path.display().to_string(),
            "deleted": true,
        }))
    }
}

#[async_trait]
impl Tool for FilesystemTool {
    fn name(&self) -> &str {
        "filesystem"
    }

    fn description(&self) -> &str {
        "Read, write, list, or delete files within a confined root directory. \
         Paths must be relative to the root; traversal outside it is rejected."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["read", "write", "list", "delete"],
                    "description": "The filesystem operation to perform"
                },
                "path": {
                    "type": "string",
                    "description": "Path relative to the configured root directory"
                },
                "contents": {
                    "type": "string",
                    "description": "Contents to write (required for the 'write' action)"
                }
            },
            "required": ["action", "path"]
        })
    }

    async fn execute(&self, context: ToolContext, parameters: Value) -> ToolResult<Value> {
        if context.is_cancelled() {
            return Err(GatewayError::Cancelled(
                "Filesystem operation cancelled".to_string(),
            ));
        }

        let action = parameters
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                GatewayError::InvalidParameters("Missing 'action' parameter".to_string())
            })?;

        let requested = parameters
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                GatewayError::InvalidParameters("Missing 'path' parameter".to_string())
            })?;

        let resolved = self.resolve(requested)?;

        match action {
            "read" => self.read(resolved).await,
            "write" => {
                let contents = parameters
                    .get("contents")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        GatewayError::InvalidParameters(
                            "'write' action requires a 'contents' parameter".to_string(),
                        )
                    })?;
                self.write(resolved, contents).await
            }
            "list" => self.list(resolved).await,
            "delete" => self.delete(resolved).await,
            other => Err(GatewayError::InvalidParameters(format!(
                "unknown action '{other}'"
            ))),
        }
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

    fn temp_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wgw-fs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn write_then_read_roundtrip() {
        let root = temp_root();
        let tool = FilesystemTool::new(&root);

        let write = tool
            .execute(
                context(),
                json!({ "action": "write", "path": "notes/hello.txt", "contents": "hi there" }),
            )
            .await
            .unwrap();
        assert_eq!(write["action"], "write");

        let read = tool
            .execute(
                context(),
                json!({ "action": "read", "path": "notes/hello.txt" }),
            )
            .await
            .unwrap();
        assert_eq!(read["contents"], "hi there");

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn list_returns_entries() {
        let root = temp_root();
        let tool = FilesystemTool::new(&root);
        tool.execute(
            context(),
            json!({ "action": "write", "path": "a.txt", "contents": "x" }),
        )
        .await
        .unwrap();

        let list = tool
            .execute(context(), json!({ "action": "list", "path": "." }))
            .await
            .unwrap();
        let entries = list["entries"].as_array().unwrap();
        assert!(entries.iter().any(|e| e["name"] == "a.txt"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn delete_removes_file() {
        let root = temp_root();
        let tool = FilesystemTool::new(&root);
        tool.execute(
            context(),
            json!({ "action": "write", "path": "gone.txt", "contents": "x" }),
        )
        .await
        .unwrap();

        tool.execute(context(), json!({ "action": "delete", "path": "gone.txt" }))
            .await
            .unwrap();

        let read = tool
            .execute(context(), json!({ "action": "read", "path": "gone.txt" }))
            .await;
        assert!(read.is_err());

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn rejects_traversal() {
        let root = temp_root();
        let tool = FilesystemTool::new(&root);
        let err = tool
            .execute(
                context(),
                json!({ "action": "read", "path": "../../etc/passwd" }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::InvalidParameters(_)));
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn rejects_absolute_path() {
        let root = temp_root();
        let tool = FilesystemTool::new(&root);
        let abs = if cfg!(windows) {
            "C:\\Windows\\system.ini"
        } else {
            "/etc/passwd"
        };
        let err = tool
            .execute(context(), json!({ "action": "read", "path": abs }))
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::InvalidParameters(_)));
        std::fs::remove_dir_all(&root).ok();
    }
}
