//! Managed Provider Gateway (compat shim for Warp Agent).
//!
//! Faithfully replicates the local-server design from `warp-oss`:
//! a Chat Completions HTTP endpoint that Warp can be pointed at via the
//! built-in custom-endpoint mechanism. No Warp source changes are required.

pub mod config;
pub mod duplicate_guard;
pub mod native;
pub mod openai_chat;
pub mod openai_responses;
pub mod probe;
pub mod server;

pub use config::{Adapter, GatewayConfig, ProviderConfig, WireApi};
pub use server::MpgServer;
