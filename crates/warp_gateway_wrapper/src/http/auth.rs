//! Authentication and authorization for the gateway HTTP surface.
//!
//! The gateway accepts a Bearer token (or an `x-agent-identity-uid` header) on
//! every `/agent/*` request. Tokens are mapped to an [`Identity`], which carries
//! the set of tool permissions granted to the caller. Sensitive built-in tools
//! (shell, filesystem) are gated behind explicit permissions.
//!
//! Auth can be disabled for local development via [`AuthConfig::disabled`], in
//! which case every request is treated as a fully-privileged anonymous identity.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Permission required to invoke a class of tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermission {
    /// Run shell commands (`run_shell_command`).
    Shell,
    /// Read/write/list/delete files (`filesystem`).
    Filesystem,
    /// Make outbound network requests.
    Network,
    /// Any other (non-sensitive) tool.
    Basic,
}

impl ToolPermission {
    /// Map a tool name to the permission it requires.
    pub fn for_tool(tool_name: &str) -> Self {
        match tool_name {
            "run_shell_command" => Self::Shell,
            "filesystem" => Self::Filesystem,
            "http_request" => Self::Network,
            _ => Self::Basic,
        }
    }
}

/// An authenticated caller and the permissions it holds.
#[derive(Debug, Clone)]
pub struct Identity {
    /// Stable identifier for the caller (token subject or identity uid).
    pub uid: String,
    /// Whether this identity may invoke any tool regardless of `permissions`.
    pub is_privileged: bool,
    /// Explicitly granted tool permissions.
    pub permissions: HashSet<ToolPermission>,
}

impl Identity {
    /// An anonymous, fully-privileged identity used when auth is disabled.
    pub fn anonymous() -> Self {
        Self {
            uid: "anonymous".to_string(),
            is_privileged: true,
            permissions: HashSet::new(),
        }
    }

    /// Whether this identity is allowed to invoke the named tool.
    pub fn can_use_tool(&self, tool_name: &str) -> bool {
        if self.is_privileged {
            return true;
        }
        let required = ToolPermission::for_tool(tool_name);
        // Basic tools are always allowed for any authenticated identity.
        required == ToolPermission::Basic || self.permissions.contains(&required)
    }
}

/// A configured token and the identity it grants.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenEntry {
    pub token: String,
    pub uid: String,
    #[serde(default)]
    pub privileged: bool,
    #[serde(default)]
    pub permissions: Vec<ToolPermission>,
}

/// Authentication configuration for the gateway.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    /// When false, all requests bypass auth as the anonymous privileged identity.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Valid tokens and their granted identities.
    #[serde(default)]
    pub tokens: Vec<TokenEntry>,
}

fn default_true() -> bool {
    true
}

impl Default for AuthConfig {
    fn default() -> Self {
        // Secure by default: enabled with no tokens means every request is rejected
        // until tokens are configured. Use `disabled()` for local development.
        Self {
            enabled: true,
            tokens: Vec::new(),
        }
    }
}

impl AuthConfig {
    /// Auth turned off; every request maps to an anonymous privileged identity.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            tokens: Vec::new(),
        }
    }

    /// Build a single-token config granting the given permissions.
    pub fn single_token(token: impl Into<String>, uid: impl Into<String>, privileged: bool) -> Self {
        Self {
            enabled: true,
            tokens: vec![TokenEntry {
                token: token.into(),
                uid: uid.into(),
                privileged,
                permissions: Vec::new(),
            }],
        }
    }
}

/// Runtime authenticator built from [`AuthConfig`].
#[derive(Clone)]
pub struct Authenticator {
    enabled: bool,
    tokens: Arc<HashMap<String, Identity>>,
}

/// Result of an authentication attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum AuthError {
    /// No credentials were supplied.
    MissingCredentials,
    /// Credentials were supplied but did not match any known token.
    InvalidCredentials,
}

impl Authenticator {
    pub fn new(config: AuthConfig) -> Self {
        let mut tokens = HashMap::new();
        for entry in config.tokens {
            let identity = Identity {
                uid: entry.uid,
                is_privileged: entry.privileged,
                permissions: entry.permissions.into_iter().collect(),
            };
            tokens.insert(entry.token, identity);
        }
        Self {
            enabled: config.enabled,
            tokens: Arc::new(tokens),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Authenticate a request given the raw `Authorization` header value and an
    /// optional `x-agent-identity-uid` header.
    pub fn authenticate(
        &self,
        authorization: Option<&str>,
        identity_uid: Option<&str>,
    ) -> Result<Identity, AuthError> {
        if !self.enabled {
            return Ok(Identity::anonymous());
        }

        // Prefer a Bearer token; fall back to a raw identity-uid header that
        // matches a configured token value.
        let presented = extract_bearer(authorization).or(identity_uid);

        let Some(token) = presented else {
            return Err(AuthError::MissingCredentials);
        };

        self.tokens
            .get(token)
            .cloned()
            .ok_or(AuthError::InvalidCredentials)
    }
}

/// Extract the token from an `Authorization: Bearer <token>` header value.
fn extract_bearer(authorization: Option<&str>) -> Option<&str> {
    let value = authorization?;
    let trimmed = value.trim();
    // Case-insensitive scheme match.
    let rest = trimmed.strip_prefix("Bearer ").or_else(|| trimmed.strip_prefix("bearer "))?;
    let token = rest.trim();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_auth_yields_privileged_anonymous() {
        let auth = Authenticator::new(AuthConfig::disabled());
        let identity = auth.authenticate(None, None).unwrap();
        assert!(identity.is_privileged);
        assert!(identity.can_use_tool("run_shell_command"));
    }

    #[test]
    fn missing_credentials_rejected_when_enabled() {
        let auth = Authenticator::new(AuthConfig::single_token("secret", "u1", true));
        assert_eq!(auth.authenticate(None, None).unwrap_err(), AuthError::MissingCredentials);
    }

    #[test]
    fn invalid_token_rejected() {
        let auth = Authenticator::new(AuthConfig::single_token("secret", "u1", true));
        assert_eq!(
            auth.authenticate(Some("Bearer wrong"), None).unwrap_err(),
            AuthError::InvalidCredentials
        );
    }

    #[test]
    fn valid_bearer_token_accepted() {
        let auth = Authenticator::new(AuthConfig::single_token("secret", "u1", true));
        let identity = auth.authenticate(Some("Bearer secret"), None).unwrap();
        assert_eq!(identity.uid, "u1");
        assert!(identity.is_privileged);
    }

    #[test]
    fn identity_uid_header_fallback() {
        let auth = Authenticator::new(AuthConfig::single_token("secret", "u1", false));
        let identity = auth.authenticate(None, Some("secret")).unwrap();
        assert_eq!(identity.uid, "u1");
    }

    #[test]
    fn non_privileged_identity_blocked_from_shell() {
        let config = AuthConfig {
            enabled: true,
            tokens: vec![TokenEntry {
                token: "t".to_string(),
                uid: "limited".to_string(),
                privileged: false,
                permissions: vec![ToolPermission::Filesystem],
            }],
        };
        let auth = Authenticator::new(config);
        let identity = auth.authenticate(Some("Bearer t"), None).unwrap();
        assert!(identity.can_use_tool("echo")); // basic always allowed
        assert!(identity.can_use_tool("filesystem")); // granted
        assert!(!identity.can_use_tool("run_shell_command")); // not granted
    }

    #[test]
    fn bearer_scheme_is_case_insensitive() {
        assert_eq!(extract_bearer(Some("bearer abc")), Some("abc"));
        assert_eq!(extract_bearer(Some("Bearer  abc ")), Some("abc"));
        assert_eq!(extract_bearer(Some("Basic abc")), None);
        assert_eq!(extract_bearer(Some("Bearer ")), None);
    }
}
