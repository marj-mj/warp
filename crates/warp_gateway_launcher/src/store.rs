//! Persistent provider store.
//!
//! Providers are saved to `%APPDATA%/WarpGatewayLauncher/providers.json`
//! (platform config dir). For safety, the upstream API key is NEVER written to
//! disk: only an optional `env_key` reference is persisted. The actual key is
//! entered at runtime and kept in memory.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use warp_gateway_wrapper::mpg::{Adapter, ProviderConfig, WireApi};

/// A persisted provider entry (no secret material).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredProvider {
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub wire_api: WireApi,
    #[serde(default)]
    pub adapter: Adapter,
    /// Name of an environment variable holding the API key. The key value
    /// itself is never stored.
    #[serde(default)]
    pub env_key: Option<String>,
}

impl StoredProvider {
    /// Build a runtime ProviderConfig, attaching the in-memory api key if given.
    pub fn to_config(&self, api_key: Option<String>) -> ProviderConfig {
        ProviderConfig {
            name: self.name.clone(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            wire_api: self.wire_api,
            adapter: self.adapter,
            api_key,
            env_key: self.env_key.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderStore {
    #[serde(default)]
    pub providers: Vec<StoredProvider>,
}

impl ProviderStore {
    /// Resolve the on-disk path: `<config_dir>/WarpGatewayLauncher/providers.json`.
    pub fn config_path() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|dirs| {
            dirs.config_dir()
                .join("WarpGatewayLauncher")
                .join("providers.json")
        })
    }

    /// Load the store from disk; returns an empty store if the file is missing
    /// or unreadable (with the error surfaced to the caller as `Err`).
    pub fn load() -> Result<Self, String> {
        let Some(path) = Self::config_path() else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        serde_json::from_str(&raw).map_err(|err| format!("invalid providers.json: {err}"))
    }

    /// Persist the store to disk, creating the parent directory if needed.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::config_path() else {
            return Err("could not resolve a config directory".to_string());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|err| format!("failed to serialize providers: {err}"))?;
        std::fs::write(&path, json)
            .map_err(|err| format!("failed to write {}: {err}", path.display()))
    }

    /// Insert or replace a provider by name (case-insensitive).
    pub fn upsert(&mut self, provider: StoredProvider) {
        if let Some(existing) = self
            .providers
            .iter_mut()
            .find(|p| p.name.eq_ignore_ascii_case(&provider.name))
        {
            *existing = provider;
        } else {
            self.providers.push(provider);
        }
    }

    /// Remove a provider by index, if valid.
    pub fn remove(&mut self, index: usize) {
        if index < self.providers.len() {
            self.providers.remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_replaces_same_name() {
        let mut store = ProviderStore::default();
        store.upsert(StoredProvider {
            name: "P".into(),
            base_url: "https://a".into(),
            model: None,
            wire_api: WireApi::Chat,
            adapter: Adapter::OpenaiChat,
            env_key: None,
        });
        store.upsert(StoredProvider {
            name: "p".into(),
            base_url: "https://b".into(),
            model: None,
            wire_api: WireApi::Chat,
            adapter: Adapter::OpenaiChat,
            env_key: None,
        });
        assert_eq!(store.providers.len(), 1);
        assert_eq!(store.providers[0].base_url, "https://b");
    }

    #[test]
    fn to_config_attaches_runtime_key() {
        let provider = StoredProvider {
            name: "P".into(),
            base_url: "https://a".into(),
            model: Some("m".into()),
            wire_api: WireApi::Responses,
            adapter: Adapter::OpenaiResponses,
            env_key: None,
        };
        let config = provider.to_config(Some("secret".into()));
        assert_eq!(config.api_key.as_deref(), Some("secret"));
        assert_eq!(config.base_url, "https://a");
    }

    #[test]
    fn roundtrip_json_omits_no_fields() {
        let store = ProviderStore {
            providers: vec![StoredProvider {
                name: "P".into(),
                base_url: "https://a".into(),
                model: None,
                wire_api: WireApi::Chat,
                adapter: Adapter::OpenaiChat,
                env_key: Some("MY_KEY".into()),
            }],
        };
        let json = serde_json::to_string(&store).unwrap();
        // api_key must never appear in serialized output.
        assert!(!json.contains("api_key"));
        let parsed: ProviderStore = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.providers.len(), 1);
        assert_eq!(parsed.providers[0].env_key.as_deref(), Some("MY_KEY"));
    }
}
