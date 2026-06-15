//! Warp setup helpers: detect a Warp install, build a custom-endpoint config to
//! paste into Warp, spawn Warp pointed at the local gateway, and manage an
//! optional cloudflared tunnel.
//!
//! Warp's custom endpoint is updated through its Windows secure-storage format.
//! This keeps the launcher independent from Warp's source and installed binary.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};

use serde_json::json;

const API_KEYS_STORAGE_KEY: &str = "AiApiKeys";
const EMPTY_GATEWAY_API_KEY: &str = "warp-gateway";

/// On Windows, prevent a child process from spawning its own console window.
/// No-op on other platforms.
#[allow(unused_variables)]
fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW
        cmd.creation_flags(0x0800_0000);
    }
}

/// Candidate Warp executable names across channels (Windows-focused; the launcher
/// targets Windows first, matching the wrapper's platform support).
fn warp_binary_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(base) = directories::BaseDirs::new() {
        let programs = base.data_local_dir().join("Programs");
        for app in ["Warp", "WarpPreview", "WarpDev"] {
            candidates.push(programs.join(app).join("warp.exe"));
            candidates.push(programs.join(app).join(format!("{app}.exe")));
        }
    }
    for env_var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(value) = std::env::var_os(env_var) {
            let base = PathBuf::from(value);
            for app in ["Warp", "WarpPreview"] {
                candidates.push(base.join(app).join("warp.exe"));
            }
        }
    }
    // Non-Windows common locations (best-effort).
    #[cfg(target_os = "macos")]
    candidates.push(PathBuf::from(
        "/Applications/Warp.app/Contents/MacOS/stable",
    ));
    #[cfg(target_os = "linux")]
    {
        candidates.push(PathBuf::from("/usr/bin/warp-terminal"));
        candidates.push(PathBuf::from("/usr/local/bin/warp-terminal"));
    }

    candidates
}

/// Detect an installed Warp binary, returning the first existing candidate.
pub fn detect_warp() -> Option<PathBuf> {
    warp_binary_candidates()
        .into_iter()
        .find(|path| path.is_file())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerUrlOverrideSupport {
    Supported,
    Unsupported,
    Unknown,
}

/// Best-effort classification of whether the detected Warp build is likely to
/// honor the `WARP_*SERVER_URL` overrides needed by proxy mode.
pub fn detect_server_url_override_support(path: &Path) -> ServerUrlOverrideSupport {
    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();

    if normalized.contains("warpdev")
        || normalized.contains("warplocal")
        || normalized.contains("warpintegration")
        || normalized.contains("/target/debug/")
        || normalized.contains("/target/release/")
        || normalized.ends_with("/dev")
        || normalized.ends_with("/local")
        || normalized.ends_with("/integration")
    {
        return ServerUrlOverrideSupport::Supported;
    }

    if normalized.contains("warppreview")
        || normalized.contains("/programs/warp/")
        || normalized.contains("/warp.app/contents/macos/stable")
        || normalized.contains("/warp.app/contents/macos/preview")
        || normalized.ends_with("/usr/bin/warp-terminal")
        || normalized.ends_with("/usr/local/bin/warp-terminal")
        || normalized.contains("warp-oss")
    {
        return ServerUrlOverrideSupport::Unsupported;
    }

    ServerUrlOverrideSupport::Unknown
}

/// Launch `winget install Warp.Warp` in a detached console. Returns an error
/// string if the process could not be spawned.
pub fn install_warp_via_winget() -> Result<(), String> {
    spawn_detached(
        "winget",
        &[
            "install",
            "--id",
            "Warp.Warp",
            "--accept-source-agreements",
            "--accept-package-agreements",
        ],
    )
}

/// Build the custom-endpoint config JSON for the user to paste into Warp.
///
/// Uses the `@gateway` name marker so Warp recognizes it as a managed-provider
/// source endpoint (matching the `warp-oss` convention).
pub fn endpoint_config_json(name: &str, endpoint_url: &str, model: &str) -> String {
    let display_name = if name.trim().is_empty() {
        "@gateway Managed Provider Gateway".to_string()
    } else if name.starts_with("@gateway") {
        name.to_string()
    } else {
        format!("@gateway {name}")
    };

    let value = json!({
        "name": display_name,
        "url": endpoint_url,
        "api_key": "",
        "models": [{
            "name": model,
            "config_key": model,
        }]
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

fn managed_endpoint_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        "@gateway Managed Provider Gateway".to_string()
    } else if name.starts_with("@gateway") {
        name.to_string()
    } else {
        format!("@gateway {name}")
    }
}

#[cfg(windows)]
fn warp_storage_location(warp: &Path) -> Result<PathBuf, String> {
    let normalized = warp
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let (application, services): (&str, &[&str]) = if normalized.contains("warppreview") {
        (
            "WarpPreview",
            &["dev.warp.WarpPreview", "dev.warp.Warp-Preview"],
        )
    } else if normalized.contains("warpdev") {
        ("WarpDev", &["dev.warp.WarpDev", "dev.warp.Warp-Dev"])
    } else {
        ("Warp", &["dev.warp.Warp"])
    };
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA is not available".to_string())?;
    let storage_dir = local.join("warp").join(application).join("data");
    Ok(services
        .iter()
        .map(|service| storage_dir.join(format!("{service}-{API_KEYS_STORAGE_KEY}")))
        .find(|path| path.is_file())
        .unwrap_or_else(|| storage_dir.join(format!("{}-{API_KEYS_STORAGE_KEY}", services[0]))))
}

#[cfg(windows)]
fn decrypt_warp_storage(encrypted_bytes: Vec<u8>) -> Result<String, String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    let mut encrypted_bytes = encrypted_bytes;
    let encrypted_blob = CRYPT_INTEGER_BLOB {
        cbData: encrypted_bytes.len() as u32,
        pbData: encrypted_bytes.as_mut_ptr(),
    };
    let mut decrypted_blob = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &encrypted_blob,
            None,
            None,
            None,
            None,
            0,
            &mut decrypted_blob,
        )
        .map_err(|err| format!("failed to decrypt Warp API keys: {err}"))?;
        let bytes =
            std::slice::from_raw_parts(decrypted_blob.pbData, decrypted_blob.cbData as usize)
                .to_vec();
        LocalFree(Some(HLOCAL(decrypted_blob.pbData.cast())));
        String::from_utf8(bytes).map_err(|err| format!("Warp API keys are not UTF-8: {err}"))
    }
}

#[cfg(windows)]
fn encrypt_warp_storage(plaintext: &str) -> Result<Vec<u8>, String> {
    use windows::core::BSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};

    let mut plaintext = plaintext.as_bytes().to_vec();
    let plaintext_blob = CRYPT_INTEGER_BLOB {
        cbData: plaintext.len() as u32,
        pbData: plaintext.as_mut_ptr(),
    };
    let mut encrypted_blob = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &plaintext_blob,
            &BSTR::from(API_KEYS_STORAGE_KEY),
            None,
            None,
            None,
            0,
            &mut encrypted_blob,
        )
        .map_err(|err| format!("failed to encrypt Warp API keys: {err}"))?;
        let bytes =
            std::slice::from_raw_parts(encrypted_blob.pbData, encrypted_blob.cbData as usize)
                .to_vec();
        LocalFree(Some(HLOCAL(encrypted_blob.pbData.cast())));
        Ok(bytes)
    }
}

/// Upsert the launcher's custom endpoint in Warp's Windows DPAPI storage.
///
/// Existing provider keys and unrelated custom endpoints are preserved.
#[cfg(windows)]
pub fn upsert_warp_gateway_endpoint(
    warp: &Path,
    name: &str,
    endpoint_url: &str,
    api_key: Option<&str>,
    model: &str,
) -> Result<(), String> {
    let storage_file = warp_storage_location(warp)?;
    let mut keys: serde_json::Value = if storage_file.is_file() {
        let encrypted = std::fs::read(&storage_file)
            .map_err(|err| format!("failed to read Warp API keys: {err}"))?;
        let json = decrypt_warp_storage(encrypted)?;
        serde_json::from_str(&json)
            .map_err(|err| format!("failed to parse Warp API keys: {err}"))?
    } else {
        json!({})
    };

    let name = managed_endpoint_name(name);
    let model = if model.trim().is_empty() {
        "default-model"
    } else {
        model.trim()
    };
    let endpoint = json!({
        "name": name,
        "url": endpoint_url.trim().trim_end_matches('/'),
        "api_key": api_key
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .unwrap_or(EMPTY_GATEWAY_API_KEY)
            .to_string(),
        "models": [{
            "name": model,
            "alias": null,
            "config_key": model,
        }],
    });
    let root = keys
        .as_object_mut()
        .ok_or_else(|| "Warp API keys payload is not a JSON object".to_string())?;
    let endpoints = root
        .entry("custom_endpoints")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| "Warp custom_endpoints payload is not a JSON array".to_string())?;
    if let Some(existing) = endpoints
        .iter_mut()
        .find(|existing| existing.get("name").and_then(|value| value.as_str()) == Some(&name))
    {
        *existing = endpoint;
    } else {
        endpoints.push(endpoint);
    }

    let json = serde_json::to_string(&keys)
        .map_err(|err| format!("failed to serialize Warp API keys: {err}"))?;
    let encrypted = encrypt_warp_storage(&json)?;
    if let Some(parent) = storage_file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create Warp data directory: {err}"))?;
    }
    std::fs::write(&storage_file, encrypted)
        .map_err(|err| format!("failed to update Warp API keys: {err}"))
}

#[cfg(not(windows))]
pub fn upsert_warp_gateway_endpoint(
    _warp: &Path,
    _name: &str,
    _endpoint_url: &str,
    _api_key: Option<&str>,
    _model: &str,
) -> Result<(), String> {
    Err("automatic Warp endpoint configuration is currently supported on Windows only".to_string())
}

/// Spawn Warp with `WARP_SERVER_ROOT_URL` pointed at the proxy (only relevant
/// for the transparent-proxy mode, not MPG). Read-only env injection; no source
/// changes.
#[allow(dead_code)] // wired by the proxy mode UI later
pub fn spawn_warp_with_server_url(warp: &PathBuf, server_root_url: &str) -> Result<(), String> {
    let ws_server_url = if let Some(rest) = server_root_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = server_root_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        server_root_url.to_string()
    };
    spawn_warp_with_proxy_urls(warp, server_root_url, &ws_server_url, &ws_server_url)
}

/// Launch Warp with the local proxy roots injected via environment variables.
/// This remains read-only with respect to Warp's install and settings.
pub fn spawn_warp_with_proxy_urls(
    warp: &PathBuf,
    server_root_url: &str,
    ws_server_url: &str,
    session_sharing_server_url: &str,
) -> Result<(), String> {
    Command::new(warp)
        .env("WARP_SERVER_ROOT_URL", server_root_url)
        .env("WARP_WS_SERVER_URL", ws_server_url)
        .env(
            "WARP_SESSION_SHARING_SERVER_URL",
            session_sharing_server_url,
        )
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("failed to launch Warp: {err}"))
}

/// Launch Warp normally after the launcher has updated its endpoint storage.
pub fn spawn_warp(warp: &PathBuf) -> Result<(), String> {
    Command::new(warp)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("failed to launch Warp: {err}"))
}

#[cfg(windows)]
pub fn is_warp_running(warp: &Path) -> Result<bool, String> {
    let executable = warp
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Warp executable path has no file name".to_string())?;
    let mut command = Command::new("tasklist");
    command.args(["/FI", &format!("IMAGENAME eq {executable}"), "/NH"]);
    no_window(&mut command);
    let output = command
        .output()
        .map_err(|err| format!("failed to inspect running Warp processes: {err}"))?;
    if !output.status.success() {
        return Err("failed to inspect running Warp processes".to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    Ok(stdout.contains(&executable.to_ascii_lowercase()))
}

#[cfg(not(windows))]
pub fn is_warp_running(_warp: &Path) -> Result<bool, String> {
    Ok(false)
}

/// Resolve a cloudflared binary: explicit override, PATH, or known install dirs.
pub fn detect_cloudflared() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("WARP_MANAGED_PROVIDER_GATEWAY_CLOUDFLARED_BIN") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
    }

    // Known Windows install locations.
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local);
        candidates.push(base.join("Microsoft\\WinGet\\Links\\cloudflared.exe"));
        candidates.push(base.join("Microsoft\\WindowsApps\\cloudflared.exe"));
    }
    for env_var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(value) = std::env::var_os(env_var) {
            let base = PathBuf::from(value);
            candidates.push(base.join("Cloudflare\\cloudflared\\cloudflared.exe"));
            candidates.push(base.join("cloudflared\\cloudflared.exe"));
        }
    }

    // Common macOS / Linux locations (Homebrew, system bins).
    for path in [
        "/opt/homebrew/bin/cloudflared", // Apple Silicon Homebrew
        "/usr/local/bin/cloudflared",    // Intel Homebrew / manual
        "/usr/bin/cloudflared",          // Linux package
    ] {
        candidates.push(PathBuf::from(path));
    }

    // Fall back to PATH lookup via which/where is overkill here; the explicit
    // env override covers custom installs.
    candidates.into_iter().find(|path| path.is_file())
}

/// A running cloudflared tunnel. Dropping it kills the child process.
pub struct CloudflaredTunnel {
    child: std::process::Child,
    pub url_rx: Receiver<String>,
}

impl CloudflaredTunnel {
    /// Non-blocking: returns the captured public URL once available.
    pub fn try_url(&self) -> Option<String> {
        self.url_rx.try_recv().ok()
    }

    /// Non-blocking: returns the child exit status once the process finishes.
    pub fn try_exit_status(&mut self) -> Result<Option<std::process::ExitStatus>, String> {
        self.child
            .try_wait()
            .map_err(|err| format!("failed to query cloudflared status: {err}"))
    }
}

impl Drop for CloudflaredTunnel {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// Spawn `cloudflared tunnel --url http://127.0.0.1:<port>`, capturing output
/// to extract the public trycloudflare.com URL.
pub fn start_cloudflared_tunnel(
    cloudflared: &PathBuf,
    port: u16,
) -> Result<CloudflaredTunnel, String> {
    let url = format!("http://127.0.0.1:{port}");
    let mut command = Command::new(cloudflared);
    command
        .args(["tunnel", "--url", &url])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|err| format!("failed to start cloudflared: {err}"))?;

    let (tx, rx) = mpsc::channel::<String>();
    if let Some(stderr) = child.stderr.take() {
        let tx = tx.clone();
        std::thread::spawn(move || scan_for_url(stderr, tx));
    }
    if let Some(stdout) = child.stdout.take() {
        let tx = tx.clone();
        std::thread::spawn(move || scan_for_url(stdout, tx));
    }
    Ok(CloudflaredTunnel { child, url_rx: rx })
}

fn scan_for_url<R: std::io::Read>(stream: R, tx: std::sync::mpsc::Sender<String>) {
    let reader = BufReader::new(stream);
    for line in reader.lines().map_while(Result::ok) {
        if let Some(url) = extract_trycloudflare_url(&line) {
            let _ = tx.send(url);
            return;
        }
    }
}

/// Extract a `https://<sub>.trycloudflare.com` URL from a log line.
pub fn extract_trycloudflare_url(line: &str) -> Option<String> {
    let idx = line.find("https://")?;
    let rest = &line[idx..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '|')
        .unwrap_or(rest.len());
    let candidate = &rest[..end];
    if candidate.contains("trycloudflare.com") {
        Some(candidate.trim_end_matches('/').to_string())
    } else {
        None
    }
}

/// Install cloudflared via winget.
pub fn install_cloudflared_via_winget() -> Result<(), String> {
    spawn_detached(
        "winget",
        &[
            "install",
            "--id",
            "Cloudflare.cloudflared",
            "--accept-source-agreements",
            "--accept-package-agreements",
        ],
    )
}

fn spawn_detached(program: &str, args: &[&str]) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args);
    no_window(&mut command);
    command
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("failed to spawn {program}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_config_adds_gateway_marker() {
        let json = endpoint_config_json("NROUTER", "http://127.0.0.1:8787/v1", "gpt-5.5");
        assert!(json.contains("@gateway NROUTER"));
        assert!(json.contains("http://127.0.0.1:8787/v1"));
        assert!(json.contains("gpt-5.5"));
    }

    #[test]
    fn endpoint_config_keeps_existing_marker() {
        let json = endpoint_config_json("@gateway Custom", "http://x/v1", "m");
        // Should not double-prefix.
        assert!(!json.contains("@gateway @gateway"));
    }

    #[test]
    fn endpoint_config_default_name_when_empty() {
        let json = endpoint_config_json("", "http://x/v1", "m");
        assert!(json.contains("@gateway Managed Provider Gateway"));
    }

    #[test]
    fn extract_url_from_banner() {
        let line = "|  https://happy-cat-1234.trycloudflare.com                          |";
        assert_eq!(
            extract_trycloudflare_url(line).as_deref(),
            Some("https://happy-cat-1234.trycloudflare.com")
        );
    }

    #[test]
    fn extract_url_ignores_non_trycloudflare() {
        assert!(extract_trycloudflare_url("https://example.com/path").is_none());
        assert!(extract_trycloudflare_url("no url here").is_none());
    }

    #[test]
    fn warp_candidates_non_empty() {
        // At least the platform default locations should be generated.
        assert!(!warp_binary_candidates().is_empty());
    }

    #[test]
    fn detect_override_support_marks_dev_and_release_paths() {
        assert_eq!(
            detect_server_url_override_support(Path::new(
                "C:/Users/test/AppData/Local/Programs/WarpDev/warp.exe"
            )),
            ServerUrlOverrideSupport::Supported
        );
        assert_eq!(
            detect_server_url_override_support(Path::new(
                "C:/Users/test/AppData/Local/Programs/Warp/warp.exe"
            )),
            ServerUrlOverrideSupport::Unsupported
        );
    }
}
