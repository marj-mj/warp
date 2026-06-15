//! Harness abstraction.
//!
//! A *harness* is the execution backend that actually runs an agent for a task.
//! Mirrors the OZ harness model:
//!
//! - `oz`       — built-in agent loop (LLM provider + gateway tools). Default.
//! - `claude`   — delegate to the `claude` CLI.
//! - `opencode` — delegate to the `opencode` CLI.
//! - `gemini`   — delegate to the `gemini` CLI.
//! - `codex`    — delegate to the `codex` CLI.
//!
//! The gateway session resolves a harness from the request config and hands it a
//! [`HarnessContext`]; the harness drives the run and emits SSE events through
//! the engine's stream manager.

pub mod cli;
pub mod oz;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::gateway::GatewayEngine;
use crate::http::auth::Identity;
use crate::http::types::SpawnAgentRequest;
use crate::utils::TaskId;

pub use cli::CliHarness;
pub use oz::OzHarness;

/// The set of supported harness backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessType {
    Oz,
    Claude,
    Opencode,
    Gemini,
    Codex,
}

impl HarnessType {
    /// Parse a harness identifier string. Unknown / empty values fall back to Oz.
    pub fn from_str_or_default(value: Option<&str>) -> Self {
        match value.map(str::trim).unwrap_or_default() {
            "claude" => Self::Claude,
            "opencode" => Self::Opencode,
            "gemini" => Self::Gemini,
            "codex" => Self::Codex,
            // "oz", "", or anything unrecognized -> default built-in harness.
            _ => Self::Oz,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Oz => "oz",
            Self::Claude => "claude",
            Self::Opencode => "opencode",
            Self::Gemini => "gemini",
            Self::Codex => "codex",
        }
    }

    /// The CLI executable name for delegate harnesses (`None` for the built-in
    /// `oz` harness).
    pub fn cli_command(&self) -> Option<&'static str> {
        match self {
            Self::Oz => None,
            Self::Claude => Some("claude"),
            Self::Opencode => Some("opencode"),
            Self::Gemini => Some("gemini"),
            Self::Codex => Some("codex"),
        }
    }
}

/// Everything a harness needs to execute one task run.
pub struct HarnessContext {
    pub task_id: TaskId,
    pub run_id: String,
    pub request: SpawnAgentRequest,
    pub identity: Identity,
    pub engine: GatewayEngine,
    pub cancellation_token: CancellationToken,
}

/// A backend that runs an agent for a single task.
#[async_trait]
pub trait Harness: Send + Sync {
    fn harness_type(&self) -> HarnessType;

    /// Execute the run to completion, emitting SSE events via the engine. The
    /// harness is responsible for terminal `Complete`/`Error`/`Cancelled` events
    /// and for calling `engine.finish_stream`.
    async fn run(&self, ctx: HarnessContext);
}

/// Resolve a concrete harness for a request based on its configured harness type.
pub fn resolve_harness(request: &SpawnAgentRequest) -> Box<dyn Harness> {
    let harness_type = HarnessType::from_str_or_default(request.harness());
    match harness_type {
        HarnessType::Oz => Box::new(OzHarness::new()),
        other => Box::new(CliHarness::new(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_harness_types() {
        assert_eq!(
            HarnessType::from_str_or_default(Some("oz")),
            HarnessType::Oz
        );
        assert_eq!(
            HarnessType::from_str_or_default(Some("claude")),
            HarnessType::Claude
        );
        assert_eq!(
            HarnessType::from_str_or_default(Some("opencode")),
            HarnessType::Opencode
        );
        assert_eq!(
            HarnessType::from_str_or_default(Some("gemini")),
            HarnessType::Gemini
        );
        assert_eq!(
            HarnessType::from_str_or_default(Some("codex")),
            HarnessType::Codex
        );
    }

    #[test]
    fn unknown_or_empty_falls_back_to_oz() {
        assert_eq!(HarnessType::from_str_or_default(None), HarnessType::Oz);
        assert_eq!(HarnessType::from_str_or_default(Some("")), HarnessType::Oz);
        assert_eq!(
            HarnessType::from_str_or_default(Some("bogus")),
            HarnessType::Oz
        );
    }

    #[test]
    fn cli_command_mapping() {
        assert_eq!(HarnessType::Oz.cli_command(), None);
        assert_eq!(HarnessType::Claude.cli_command(), Some("claude"));
        assert_eq!(HarnessType::Codex.cli_command(), Some("codex"));
    }
}
