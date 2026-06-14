//! Provider capability probe.
//!
//! Sends minimal requests to an upstream provider to determine which wire APIs
//! and features it supports, then recommends an adapter / compatibility group /
//! wire API along with safe-mode defaults. Only HTTP 2xx counts as a pass.

use serde_json::{json, Value};

/// Outcome of probing a single capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    Pass,
    Fail,
    NotTested,
}

impl ProbeOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::NotTested => "not_tested",
        }
    }
}

/// Classification of an upstream error for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Diagnostic {
    Ok,
    Auth,
    RateLimit,
    ServerError,
    Network,
}

impl Diagnostic {
    /// Classify an HTTP status code into a diagnostic bucket.
    pub fn from_status(status: u16) -> Self {
        match status {
            200..=299 => Self::Ok,
            401 | 403 => Self::Auth,
            429 => Self::RateLimit,
            500..=599 => Self::ServerError,
            _ => Self::ServerError,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Auth => "auth",
            Self::RateLimit => "rate_limit",
            Self::ServerError => "server",
            Self::Network => "network",
        }
    }
}

/// Full capability matrix for a provider.
#[derive(Debug, Clone)]
pub struct CapabilityMatrix {
    pub chat_non_stream: ProbeOutcome,
    pub chat_streaming: ProbeOutcome,
    pub chat_tools: ProbeOutcome,
    pub responses_non_stream: ProbeOutcome,
    pub responses_streaming: ProbeOutcome,
    pub responses_tools: ProbeOutcome,
    pub diagnostics: Vec<(String, Diagnostic)>,
}

impl Default for CapabilityMatrix {
    fn default() -> Self {
        Self {
            chat_non_stream: ProbeOutcome::NotTested,
            chat_streaming: ProbeOutcome::NotTested,
            chat_tools: ProbeOutcome::NotTested,
            responses_non_stream: ProbeOutcome::NotTested,
            responses_streaming: ProbeOutcome::NotTested,
            responses_tools: ProbeOutcome::NotTested,
            diagnostics: Vec::new(),
        }
    }
}

/// Recommendation derived from a capability matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recommendation {
    pub adapter: &'static str,
    pub compatibility_group: &'static str,
    pub wire_api: &'static str,
    pub confidence: &'static str,
    pub safe_mode_defaults: SafeModeDefaults,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeModeDefaults {
    pub disable_tools: bool,
    pub disable_mcp: bool,
}

impl CapabilityMatrix {
    /// Derive a recommended configuration from the observed capabilities.
    pub fn recommend(&self) -> Recommendation {
        let chat_ok =
            matches!(self.chat_non_stream, ProbeOutcome::Pass) || matches!(self.chat_streaming, ProbeOutcome::Pass);
        let responses_ok = matches!(self.responses_non_stream, ProbeOutcome::Pass)
            || matches!(self.responses_streaming, ProbeOutcome::Pass);

        // Prefer chat completions when available (simpler, no conversion);
        // fall back to responses; default to chat with low confidence otherwise.
        if chat_ok {
            let tools_ok = matches!(self.chat_tools, ProbeOutcome::Pass);
            Recommendation {
                adapter: "openai_chat",
                compatibility_group: "openai_compatible_chat",
                wire_api: "chat",
                confidence: if responses_ok { "high" } else { "medium" },
                safe_mode_defaults: SafeModeDefaults {
                    // Only forward tools when the provider proved it supports them.
                    disable_tools: !tools_ok,
                    disable_mcp: true,
                },
            }
        } else if responses_ok {
            let tools_ok = matches!(self.responses_tools, ProbeOutcome::Pass);
            Recommendation {
                adapter: "openai_responses",
                compatibility_group: "openai_compatible_responses",
                wire_api: "responses",
                confidence: "medium",
                safe_mode_defaults: SafeModeDefaults {
                    disable_tools: !tools_ok,
                    disable_mcp: true,
                },
            }
        } else {
            Recommendation {
                adapter: "openai_chat",
                compatibility_group: "openai_compatible_chat",
                wire_api: "chat",
                confidence: "low",
                safe_mode_defaults: SafeModeDefaults {
                    disable_tools: true,
                    disable_mcp: true,
                },
            }
        }
    }

    /// Render the matrix + recommendation as JSON for CLI output.
    pub fn to_json(&self) -> Value {
        let rec = self.recommend();
        json!({
            "capabilities": {
                "chat_completions": {
                    "non_stream": self.chat_non_stream.as_str(),
                    "streaming": self.chat_streaming.as_str(),
                    "tools": self.chat_tools.as_str(),
                },
                "responses": {
                    "non_stream": self.responses_non_stream.as_str(),
                    "streaming": self.responses_streaming.as_str(),
                    "tools": self.responses_tools.as_str(),
                }
            },
            "diagnostics": self.diagnostics.iter().map(|(label, diag)| json!({
                "check": label,
                "diagnostic": diag.as_str(),
            })).collect::<Vec<_>>(),
            "recommendation": {
                "adapter": rec.adapter,
                "compatibility_group": rec.compatibility_group,
                "wire_api": rec.wire_api,
                "confidence": rec.confidence,
                "safe_mode_defaults": {
                    "disable_tools": rec.safe_mode_defaults.disable_tools,
                    "disable_mcp": rec.safe_mode_defaults.disable_mcp,
                }
            }
        })
    }
}

/// Probe a provider's chat + responses capabilities. `api_key` may be empty.
pub async fn probe_provider(base_url: &str, api_key: Option<&str>) -> CapabilityMatrix {
    let client = reqwest::Client::new();
    let base = base_url.trim_end_matches('/');
    let mut matrix = CapabilityMatrix::default();

    // --- Chat Completions: non-stream ---
    let chat_url = format!("{base}/chat/completions");
    let chat_body = json!({
        "model": "probe",
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 1
    });
    let (outcome, diag) = probe_once(&client, &chat_url, api_key, &chat_body).await;
    matrix.chat_non_stream = outcome;
    matrix.diagnostics.push(("chat_completions.non_stream".to_string(), diag));

    // --- Chat Completions: streaming ---
    let mut chat_stream_body = chat_body.clone();
    chat_stream_body["stream"] = json!(true);
    let (outcome, diag) = probe_once(&client, &chat_url, api_key, &chat_stream_body).await;
    matrix.chat_streaming = outcome;
    matrix.diagnostics.push(("chat_completions.streaming".to_string(), diag));

    // --- Chat Completions: tools ---
    let mut chat_tools_body = chat_body.clone();
    chat_tools_body["tools"] = json!([{
        "type": "function",
        "function": { "name": "noop", "parameters": {"type": "object", "properties": {}} }
    }]);
    let (outcome, diag) = probe_once(&client, &chat_url, api_key, &chat_tools_body).await;
    matrix.chat_tools = outcome;
    matrix.diagnostics.push(("chat_completions.tools".to_string(), diag));

    // --- Responses: non-stream ---
    let responses_url = format!("{base}/responses");
    let responses_body = json!({
        "model": "probe",
        "input": [{"role": "user", "content": [{"type": "input_text", "text": "ping"}]}],
        "max_output_tokens": 1
    });
    let (outcome, diag) = probe_once(&client, &responses_url, api_key, &responses_body).await;
    matrix.responses_non_stream = outcome;
    matrix.diagnostics.push(("responses.non_stream".to_string(), diag));

    // --- Responses: streaming ---
    let mut responses_stream_body = responses_body.clone();
    responses_stream_body["stream"] = json!(true);
    let (outcome, diag) = probe_once(&client, &responses_url, api_key, &responses_stream_body).await;
    matrix.responses_streaming = outcome;
    matrix.diagnostics.push(("responses.streaming".to_string(), diag));

    // --- Responses: tools ---
    let mut responses_tools_body = responses_body.clone();
    responses_tools_body["tools"] = json!([{
        "type": "function",
        "name": "noop",
        "parameters": {"type": "object", "properties": {}}
    }]);
    let (outcome, diag) = probe_once(&client, &responses_url, api_key, &responses_tools_body).await;
    matrix.responses_tools = outcome;
    matrix.diagnostics.push(("responses.tools".to_string(), diag));

    matrix
}

/// Send one probe request; only 2xx is a pass.
async fn probe_once(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    body: &Value,
) -> (ProbeOutcome, Diagnostic) {
    let mut request = client.post(url).json(body);
    if let Some(key) = api_key {
        if !key.is_empty() {
            request = request.bearer_auth(key);
        }
    }
    match request.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let diag = Diagnostic::from_status(status);
            let outcome = if (200..=299).contains(&status) {
                ProbeOutcome::Pass
            } else {
                ProbeOutcome::Fail
            };
            (outcome, diag)
        }
        Err(_) => (ProbeOutcome::Fail, Diagnostic::Network),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_classification() {
        assert_eq!(Diagnostic::from_status(200), Diagnostic::Ok);
        assert_eq!(Diagnostic::from_status(401), Diagnostic::Auth);
        assert_eq!(Diagnostic::from_status(403), Diagnostic::Auth);
        assert_eq!(Diagnostic::from_status(429), Diagnostic::RateLimit);
        assert_eq!(Diagnostic::from_status(500), Diagnostic::ServerError);
    }

    #[test]
    fn recommends_chat_when_chat_passes() {
        let mut matrix = CapabilityMatrix::default();
        matrix.chat_non_stream = ProbeOutcome::Pass;
        matrix.chat_tools = ProbeOutcome::Pass;
        let rec = matrix.recommend();
        assert_eq!(rec.adapter, "openai_chat");
        assert_eq!(rec.wire_api, "chat");
        assert!(!rec.safe_mode_defaults.disable_tools); // tools proven
    }

    #[test]
    fn recommends_responses_when_only_responses_passes() {
        let mut matrix = CapabilityMatrix::default();
        matrix.responses_non_stream = ProbeOutcome::Pass;
        let rec = matrix.recommend();
        assert_eq!(rec.adapter, "openai_responses");
        assert_eq!(rec.wire_api, "responses");
        assert!(rec.safe_mode_defaults.disable_tools); // tools not proven
    }

    #[test]
    fn low_confidence_when_nothing_passes() {
        let matrix = CapabilityMatrix::default();
        let rec = matrix.recommend();
        assert_eq!(rec.confidence, "low");
        assert!(rec.safe_mode_defaults.disable_tools);
    }

    #[test]
    fn json_output_has_recommendation() {
        let mut matrix = CapabilityMatrix::default();
        matrix.chat_non_stream = ProbeOutcome::Pass;
        let value = matrix.to_json();
        assert_eq!(value["recommendation"]["adapter"], "openai_chat");
        assert_eq!(value["capabilities"]["chat_completions"]["non_stream"], "pass");
    }
}
