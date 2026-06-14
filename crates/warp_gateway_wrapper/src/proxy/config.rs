//! Configuration for the transparent Warp proxy.
//!
//! The proxy sits between a Warp client (configured via `WARP_SERVER_ROOT_URL`
//! / `WARP_WS_SERVER_URL` to point at the proxy) and the upstream Warp backend.
//! It forwards requests byte-for-byte while overriding the `Host` and
//! `Authorization` headers so a single configured Warp/OZ token is used.

use serde::{Deserialize, Serialize};

/// Known Warp deployment channels. Mirrors the per-channel server URLs in
/// `crates/warp_core/src/channel/config.rs` so users can pick a preset instead
/// of typing URLs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WarpChannel {
    Production,
    Staging,
    Dev,
}

impl WarpChannel {
    /// HTTP root URL for this channel (used for REST/GraphQL).
    pub fn http_root(self) -> &'static str {
        match self {
            Self::Production => "https://app.warp.dev",
            Self::Staging => "https://staging.warp.dev",
            Self::Dev => "https://dev.warp.dev",
        }
    }

    /// WebSocket root URL for the RTC server (real-time updates).
    pub fn ws_rtc(self) -> &'static str {
        match self {
            Self::Production => "wss://rtc.app.warp.dev",
            Self::Staging => "wss://rtc.staging.warp.dev",
            Self::Dev => "wss://rtc.dev.warp.dev",
        }
    }

    /// WebSocket root URL for session sharing.
    pub fn ws_sessions(self) -> &'static str {
        match self {
            Self::Production => "wss://sessions.app.warp.dev",
            Self::Staging => "wss://sessions.staging.warp.dev",
            Self::Dev => "wss://sessions.dev.warp.dev",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Staging => "staging",
            Self::Dev => "dev",
        }
    }
}

/// Runtime configuration for the proxy.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// HTTP upstream root (e.g. `https://app.warp.dev`). All HTTP requests are
    /// forwarded to this host with their original path/query preserved.
    pub upstream_http: String,
    /// WebSocket upstream root for RTC traffic (e.g. `wss://rtc.app.warp.dev`).
    pub upstream_ws_rtc: Option<String>,
    /// WebSocket upstream root for session sharing.
    pub upstream_ws_sessions: Option<String>,
    /// Bearer token attached to every forwarded request (Warp/OZ identity).
    /// Empty means do not override the incoming Authorization header.
    pub oz_token: String,
    /// When true, the proxy refuses requests that do not present a valid
    /// gateway-side token (uses the existing Phase 9 Authenticator). Off by
    /// default for local-only deployments.
    pub require_gateway_auth: bool,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self::for_channel(WarpChannel::Production)
    }
}

impl ProxyConfig {
    /// Build a config targeting the given channel using its standard URLs.
    pub fn for_channel(channel: WarpChannel) -> Self {
        Self {
            upstream_http: channel.http_root().to_string(),
            upstream_ws_rtc: Some(channel.ws_rtc().to_string()),
            upstream_ws_sessions: Some(channel.ws_sessions().to_string()),
            oz_token: String::new(),
            require_gateway_auth: false,
        }
    }

    /// Read the OZ/Warp token from `WARP_GATEWAY_OZ_TOKEN` (preferred) or the
    /// generic `OZ_TOKEN` / `WARP_TOKEN` variables. Empty/missing -> no token.
    pub fn token_from_env() -> String {
        for var in ["WARP_GATEWAY_OZ_TOKEN", "OZ_TOKEN", "WARP_TOKEN"] {
            if let Ok(value) = std::env::var(var) {
                if !value.trim().is_empty() {
                    return value;
                }
            }
        }
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_presets() {
        assert_eq!(WarpChannel::Production.http_root(), "https://app.warp.dev");
        assert_eq!(WarpChannel::Staging.ws_rtc(), "wss://rtc.staging.warp.dev");
        assert_eq!(WarpChannel::Dev.ws_sessions(), "wss://sessions.dev.warp.dev");
    }

    #[test]
    fn for_channel_populates_all_urls() {
        let config = ProxyConfig::for_channel(WarpChannel::Production);
        assert_eq!(config.upstream_http, "https://app.warp.dev");
        assert!(config.upstream_ws_rtc.is_some());
        assert!(config.upstream_ws_sessions.is_some());
        assert!(config.oz_token.is_empty());
    }
}
