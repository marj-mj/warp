pub mod auth;
pub mod server;
pub mod handlers;
pub mod types;
pub mod sse;
pub mod stream_manager;

pub use server::{Server, ServerConfig};
pub use types::{
    AgentConfigSnapshot, FileAttachment, SpawnAgentRequest, SpawnAgentResponse, TaskStatus,
    UserQueryMode,
};
pub use sse::SSEEvent;
pub use auth::{AuthConfig, Authenticator, Identity, ToolPermission};
pub use stream_manager::StreamManager;
