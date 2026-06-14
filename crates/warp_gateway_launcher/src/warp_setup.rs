//! Warp setup helpers: detect a Warp install, build a custom-endpoint config to
//! paste into Warp, spawn Warp pointed at the local gateway, and manage an
//! optional cloudflared tunnel.
//!
//! IMPORTANT: nothing here modifies Warp's files or secure storage. We only
//! detect (read-only), spawn processes, and build text for the user to paste.
//! This keeps Warp updatable.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};

use serde_json::json;

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
    candidates.push(PathBuf::from("/Applications/Warp.app/Contents/MacOS/stable"));
    #[cfg(target_os = "linux")]
    {
        candidates.push(PathBuf::from("/usr/bin/warp-terminal"));
        candidates.push(PathBuf::from("/usr/local/bin/warp-terminal"));
    }

    candidates
}

/// Detect an installed Warp binary, returning the first existing candidate.
pub fn detect_warp() -> Option<PathBuf> {
    warp_binary_candidates().into_iter().find(|path| path.is_file())
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
    let normalized = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();

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

/// Launch Warp normally (no env override). Used by "Launch all" since the
/// Managed Provider Gateway flow relies on a custom endpoint configured inside
/// Warp, not an env var.
pub fn spawn_warp(warp: &PathBuf) -> Result<(), String> {
    Command::new(warp)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("failed to launch Warp: {err}"))
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
pub fn start_cloudflared_tunnel(cloudflared: &PathBuf, port: u16) -> Result<CloudflaredTunnel, String> {
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
