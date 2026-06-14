use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::ValueEnum;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::{RegKey, HKEY};

#[derive(Debug, Deserialize)]
pub struct WrapperConfig {
    pub warp: WarpConfig,
    pub endpoint: EndpointConfig,
    #[serde(default)]
    pub gateway: Option<GatewayConfig>,
    #[serde(skip)]
    config_dir: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct WarpBinaryDiagnostics {
    pub app_name: String,
    pub requested: Option<PathBuf>,
    pub selected: Option<PathBuf>,
    pub candidates: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
pub struct GatewayCommandDiagnostics {
    pub requested: Option<PathBuf>,
    pub selected: Option<PathBuf>,
    pub working_dir: Option<PathBuf>,
    pub search_dirs: Vec<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub struct WarpConfig {
    pub binary: Option<PathBuf>,
    #[serde(default)]
    pub args: Vec<String>,
    pub app_id: Option<String>,
    pub channel: Option<WarpChannelPreset>,
    pub state_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum WarpChannelPreset {
    Stable,
    Preview,
    Dev,
    Local,
    Oss,
    Integration,
}

impl WarpChannelPreset {
    pub fn as_config_value(self) -> &'static str {
        match self {
            WarpChannelPreset::Stable => "stable",
            WarpChannelPreset::Preview => "preview",
            WarpChannelPreset::Dev => "dev",
            WarpChannelPreset::Local => "local",
            WarpChannelPreset::Oss => "oss",
            WarpChannelPreset::Integration => "integration",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EndpointConfig {
    pub name: String,
    pub url: String,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub models: Vec<EndpointModelConfig>,
    pub preferred_model_slug: Option<String>,
    pub preferred_model_config_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EndpointModelConfig {
    pub slug: String,
    pub alias: Option<String>,
    pub config_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GatewayConfig {
    pub command: Option<PathBuf>,
    #[serde(default)]
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub healthcheck_url: Option<String>,
    pub startup_timeout_secs: Option<u64>,
}

impl WrapperConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)?;
        let mut config: Self =
            toml::from_str(&contents).context("config file is not valid TOML")?;
        config.config_dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Ok(config)
    }

    pub fn resolve_warp_binary(&self) -> Result<PathBuf> {
        self.diagnose_warp_binary()?
            .selected
            .with_context(|| {
                format!(
                    "could not find an installed Warp binary for app '{}'; set warp.binary explicitly",
                    self.warp
                        .resolved_app_name()
                        .unwrap_or_else(|_| "<unknown>".to_string())
                )
            })
    }

    pub fn resolve_gateway_command(&self) -> Result<Option<PathBuf>> {
        let Some(gateway) = self.gateway.as_ref() else {
            return Ok(None);
        };
        let diagnostics = self.diagnose_gateway_command()?.context("missing gateway config")?;
        if let Some(selected) = diagnostics.selected {
            return Ok(Some(selected));
        }

        anyhow::bail!(
            "gateway.command is {}and no managed gateway binary was found",
            if gateway.command.is_some() {
                "set but did not resolve; "
            } else {
                "not set; "
            }
        );
    }

    pub fn resolve_gateway_working_dir(&self) -> Result<Option<PathBuf>> {
        let Some(gateway) = self.gateway.as_ref() else {
            return Ok(None);
        };

        if let Some(working_dir) = gateway.working_dir.as_ref() {
            return Ok(Some(self.resolve_support_path(working_dir)));
        }

        let Some(command) = self.resolve_gateway_command()? else {
            return Ok(None);
        };

        Ok(command.parent().map(Path::to_path_buf))
    }

    pub fn diagnose_warp_binary(&self) -> Result<WarpBinaryDiagnostics> {
        let app_name = self.warp.resolved_app_name()?;
        let requested = self
            .warp
            .binary
            .as_ref()
            .map(|binary| self.resolve_support_path(binary));
        let candidates = match requested.clone() {
            Some(binary) => vec![binary],
            None => discover_warp_binaries(&app_name),
        };
        let selected = candidates.iter().find(|path| path.is_file()).cloned();
        Ok(WarpBinaryDiagnostics {
            app_name,
            requested,
            selected,
            candidates,
        })
    }

    pub fn diagnose_gateway_command(&self) -> Result<Option<GatewayCommandDiagnostics>> {
        let Some(gateway) = self.gateway.as_ref() else {
            return Ok(None);
        };

        let search_dirs = self.support_base_dirs()?;
        let requested = gateway
            .command
            .as_ref()
            .map(|command| self.resolve_support_path(command));
        let candidates = match requested.clone() {
            Some(command) => vec![command],
            None => search_dirs
                .iter()
                .flat_map(|base_dir| {
                    implicit_gateway_binary_names()
                        .into_iter()
                        .map(move |name| base_dir.join(name))
                })
                .collect(),
        };
        let selected = candidates.iter().find(|path| path.is_file()).cloned();
        let working_dir = if let Some(working_dir) = gateway.working_dir.as_ref() {
            Some(self.resolve_support_path(working_dir))
        } else {
            selected
                .as_ref()
                .and_then(|command| command.parent().map(Path::to_path_buf))
        };
        Ok(Some(GatewayCommandDiagnostics {
            requested,
            selected,
            working_dir,
            search_dirs,
        }))
    }

    fn resolve_support_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            return path.to_path_buf();
        }

        for base_dir in self.support_base_dirs().unwrap_or_default() {
            let candidate = base_dir.join(path);
            if candidate.exists() {
                return candidate;
            }
        }

        self.config_dir.join(path)
    }

    fn support_base_dirs(&self) -> Result<Vec<PathBuf>> {
        let mut dirs = Vec::new();
        dirs.push(current_exe_dir()?);
        if !dirs.iter().any(|existing| existing == &self.config_dir) {
            dirs.push(self.config_dir.clone());
        }
        Ok(dirs)
    }
}

impl WarpConfig {
    pub fn resolved_app_id(&self) -> Result<String> {
        if let Some(app_id) = self
            .app_id
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return Ok(app_id.to_string());
        }

        let Some(channel) = self.channel else {
            anyhow::bail!("warp.app_id or warp.channel must be set");
        };

        let app_id = match channel {
            WarpChannelPreset::Stable => "dev.warp.Warp",
            WarpChannelPreset::Preview => "dev.warp.WarpPreview",
            WarpChannelPreset::Dev => "dev.warp.WarpDev",
            WarpChannelPreset::Local => "dev.warp.WarpLocal",
            WarpChannelPreset::Oss => "dev.warp.WarpOss",
            WarpChannelPreset::Integration => "dev.warp.WarpIntegration",
        };
        Ok(app_id.to_string())
    }

    pub fn resolved_app_name(&self) -> Result<String> {
        Ok(app_name_for_app_id(&self.resolved_app_id()?).to_string())
    }
}

fn discover_warp_binaries(app_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    candidates.extend(uninstall_registry_binaries(app_name));
    for install_dir in install_dirs(app_name) {
        for binary_name in binary_names(app_name) {
            candidates.push(install_dir.join(binary_name));
        }
    }
    candidates
}

fn install_dirs(app_name: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(base_dirs) = BaseDirs::new() {
        dirs.push(base_dirs.data_local_dir().join("Programs").join(app_name));
    }

    for env_var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(value) = env::var_os(env_var) {
            dirs.push(PathBuf::from(value).join(app_name));
        }
    }

    dirs
}

fn implicit_gateway_binary_names() -> [&'static str; 2] {
    ["managed-gateway.exe", "managed_gateway.exe"]
}

fn uninstall_registry_binaries(app_name: &str) -> Vec<PathBuf> {
    const SCOPES: [(HKEY, &str); 3] = [
        (
            HKEY_LOCAL_MACHINE,
            "SOFTWARE\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        ),
        (
            HKEY_LOCAL_MACHINE,
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        ),
        (
            HKEY_CURRENT_USER,
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        ),
    ];

    let mut candidates = Vec::new();
    for (scope, key) in SCOPES {
        let uninstall_key = match RegKey::predef(scope).open_subkey(key) {
            Ok(key) => key,
            Err(_) => continue,
        };

        for application_id in uninstall_key.enum_keys().flatten() {
            let Ok(application_info) = uninstall_key.open_subkey(&application_id) else {
                continue;
            };
            let Some(display_name) = application_info.get_value::<String, _>("DisplayName").ok()
            else {
                continue;
            };
            if !is_target_warp_display_name(app_name, &display_name) {
                continue;
            }

            candidates.extend(uninstall_entry_executable_paths(&application_info, app_name));
        }
    }
    candidates
}

fn uninstall_entry_executable_paths(application_info: &RegKey, app_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(display_icon_path) = parse_display_icon_path(application_info) {
        candidates.push(display_icon_path);
    }

    if let Ok(install_location) = application_info.get_value::<String, _>("InstallLocation") {
        let install_location = PathBuf::from(install_location);
        for binary_name in binary_names(app_name) {
            candidates.push(install_location.join(binary_name));
        }
    }

    candidates
}

fn parse_display_icon_path(application_info: &RegKey) -> Option<PathBuf> {
    let display_icon = application_info.get_value::<String, _>("DisplayIcon").ok()?;
    let path = display_icon
        .rsplit_once(',')
        .map(|(path, _)| path)
        .unwrap_or(display_icon.as_str())
        .replace('"', "");
    Some(PathBuf::from(path))
}

fn is_target_warp_display_name(app_name: &str, display_name: &str) -> bool {
    let app_name = normalize_name(app_name);
    let display_name = normalize_name(display_name);
    display_name == app_name || display_name.starts_with(&app_name)
}

fn binary_names(app_name: &str) -> Vec<String> {
    let mut names = vec!["warp.exe".to_string()];
    let app_specific = format!("{app_name}.exe");
    if !names.iter().any(|name| name.eq_ignore_ascii_case(&app_specific)) {
        names.push(app_specific);
    }
    names
}

fn app_name_for_app_id(app_id: &str) -> &str {
    app_id
        .rsplit('.')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(app_id)
}

fn normalize_name(value: &str) -> String {
    value
        .chars()
        .filter(|char| char.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn current_exe_dir() -> Result<PathBuf> {
    let current_exe = env::current_exe().context("failed to resolve wrapper executable path")?;
    current_exe
        .parent()
        .map(Path::to_path_buf)
        .context("wrapper executable path has no parent directory")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_names_include_generic_and_app_specific_names() {
        assert_eq!(
            binary_names("WarpPreview"),
            vec!["warp.exe".to_string(), "WarpPreview.exe".to_string()]
        );
        assert_eq!(binary_names("Warp"), vec!["warp.exe".to_string()]);
    }

    #[test]
    fn app_name_for_app_id_returns_last_segment() {
        assert_eq!(app_name_for_app_id("dev.warp.Warp"), "Warp");
        assert_eq!(app_name_for_app_id("dev.warp.WarpPreview"), "WarpPreview");
    }

    #[test]
    fn normalize_name_ignores_spacing_and_case() {
        assert_eq!(normalize_name("Warp Preview"), "warppreview");
        assert_eq!(normalize_name("Warp (User)"), "warpuser");
    }

    #[test]
    fn display_name_match_allows_suffixes() {
        assert!(is_target_warp_display_name("Warp", "Warp"));
        assert!(is_target_warp_display_name("Warp", "Warp (User)"));
        assert!(is_target_warp_display_name("WarpPreview", "Warp Preview"));
        assert!(is_target_warp_display_name(
            "WarpPreview",
            "Warp Preview (Machine-Wide)"
        ));
        assert!(!is_target_warp_display_name("WarpPreview", "Warp"));
    }

    #[test]
    fn implicit_gateway_binary_names_match_supported_filenames() {
        assert_eq!(
            implicit_gateway_binary_names(),
            ["managed-gateway.exe", "managed_gateway.exe"]
        );
    }
}
