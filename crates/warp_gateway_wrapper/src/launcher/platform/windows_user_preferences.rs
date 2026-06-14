#[cfg(not(target_os = "windows"))]
compile_error!("warp_gateway_wrapper currently supports Windows only.");

use std::io;

use anyhow::{Context, Result};
use serde::Serialize;
use windows_registry::{Key, CURRENT_USER};

const WARP_REGISTRY_BASE_PATH: &str = "Software\\Warp.dev\\";

pub struct WindowsUserPreferences {
    app_key_path: String,
}

impl WindowsUserPreferences {
    pub fn new(app_name: &str) -> Self {
        Self {
            app_key_path: format!("{WARP_REGISTRY_BASE_PATH}{app_name}"),
        }
    }

    pub fn write_json<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let serialized = serde_json::to_string(value).context("failed to serialize preference")?;
        self.registry_key()?
            .set_string(key, serialized.as_str())
            .map_err(io::Error::from)
            .with_context(|| format!("failed to write registry value '{}'", key))?;
        Ok(())
    }

    fn registry_key(&self) -> Result<Key> {
        CURRENT_USER
            .create(self.app_key_path.clone())
            .map_err(io::Error::from)
            .with_context(|| format!("failed to open registry key {}", self.app_key_path))
    }
}
