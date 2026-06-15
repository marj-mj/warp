//! Probe runner: async wrapper over `mpg::probe::probe_provider`, returning a
//! serializable summary for the frontend.

use serde::Serialize;
use warp_gateway_wrapper::mpg::probe::{probe_provider, CapabilityMatrix};

#[derive(Debug, Clone, Serialize)]
pub struct ProbeResult {
    pub adapter: String,
    pub wire_api: String,
    pub compatibility_group: String,
    pub confidence: String,
    pub disable_tools: bool,
    pub disable_mcp: bool,
    pub chat_non_stream: String,
    pub chat_streaming: String,
    pub chat_tools: String,
    pub responses_non_stream: String,
    pub responses_streaming: String,
    pub responses_tools: String,
}

impl ProbeResult {
    fn from_matrix(matrix: &CapabilityMatrix) -> Self {
        let rec = matrix.recommend();
        Self {
            adapter: rec.adapter.to_string(),
            wire_api: rec.wire_api.to_string(),
            compatibility_group: rec.compatibility_group.to_string(),
            confidence: rec.confidence.to_string(),
            disable_tools: rec.safe_mode_defaults.disable_tools,
            disable_mcp: rec.safe_mode_defaults.disable_mcp,
            chat_non_stream: matrix.chat_non_stream.as_str().to_string(),
            chat_streaming: matrix.chat_streaming.as_str().to_string(),
            chat_tools: matrix.chat_tools.as_str().to_string(),
            responses_non_stream: matrix.responses_non_stream.as_str().to_string(),
            responses_streaming: matrix.responses_streaming.as_str().to_string(),
            responses_tools: matrix.responses_tools.as_str().to_string(),
        }
    }
}

/// Run a capability probe against `base_url`. `api_key` may be empty.
pub async fn run_probe(base_url: String, api_key: Option<String>) -> ProbeResult {
    let matrix = probe_provider(&base_url, api_key.as_deref()).await;
    ProbeResult::from_matrix(&matrix)
}
