//! Tauri command surface Ã¢â‚¬â€ the bridge between the React UI and the Rust
//! gateway/proxy controllers, provider store, and launch flow.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::controller::GatewayStatus;
use crate::events::emit_phase;
use crate::probe::{run_probe, ProbeResult};
use crate::proxy_controller::ProxyStatus;
use crate::state::{AppState, LaunchPhase, ToolPaths};
use crate::store::{ProviderStore, StoredProvider};
use warp_gateway_wrapper::mpg::{Adapter, GatewayConfig, ProviderConfig, WireApi};
use warp_gateway_wrapper::proxy::{ProxyConfig, WarpChannel};

const PUBLIC_URL_TIMEOUT: Duration = Duration::from_secs(30);

type CmdResult<T> = Result<T, String>;

/// Serializable provider form coming from the frontend. Mirrors the egui
/// `ProviderForm`, but secrets are passed explicitly and never persisted.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderInput {
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub env_key: String,
    #[serde(default)]
    pub wire_api: String,
    #[serde(default)]
    pub adapter: String,
}

impl ProviderInput {
    fn wire_api(&self) -> WireApi {
        match self.wire_api.as_str() {
            "responses" => WireApi::Responses,
            _ => WireApi::Chat,
        }
    }

    fn adapter(&self) -> Adapter {
        match self.adapter.as_str() {
            "openai_responses" => Adapter::OpenaiResponses,
            "bridge_openai" => Adapter::BridgeOpenai,
            "anthropic_messages" => Adapter::AnthropicMessages,
            "gemini_generate_content" => Adapter::GeminiGenerateContent,
            _ => Adapter::OpenaiChat,
        }
    }

    fn to_stored(&self) -> StoredProvider {
        StoredProvider {
            name: self.name.trim().to_string(),
            base_url: self.base_url.trim().to_string(),
            model: nullable(&self.model),
            wire_api: self.wire_api(),
            adapter: self.adapter(),
            env_key: nullable(&self.env_key),
        }
    }

    /// Resolve the runtime API key: explicit value wins, otherwise read env_key.
    fn resolved_api_key(&self) -> Option<String> {
        if !self.api_key.trim().is_empty() {
            return Some(self.api_key.clone());
        }
        let env = self.env_key.trim();
        if env.is_empty() {
            return None;
        }
        std::env::var(env).ok().filter(|v| !v.trim().is_empty())
    }

    fn is_valid(&self) -> bool {
        !self.name.trim().is_empty() && !self.base_url.trim().is_empty()
    }

    fn to_provider_config(&self) -> ProviderConfig {
        self.to_stored().to_config(self.resolved_api_key())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayOptions {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_gateway_port")]
    pub port: u16,
    #[serde(default = "default_true")]
    pub disable_tools: bool,
    #[serde(default = "default_true")]
    pub disable_mcp: bool,
    #[serde(default = "default_dup_window")]
    pub duplicate_window_secs: u64,
    #[serde(default)]
    pub force_model_config_key: String,
}

impl Default for GatewayOptions {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_gateway_port(),
            disable_tools: true,
            disable_mcp: true,
            duplicate_window_secs: default_dup_window(),
            force_model_config_key: String::new(),
        }
    }
}

impl GatewayOptions {
    fn build_config(&self, provider: &ProviderInput) -> GatewayConfig {
        let mut c = GatewayConfig::new(provider.to_provider_config());
        c.disable_tools = self.disable_tools;
        c.disable_mcp = self.disable_mcp;
        c.duplicate_window_secs = self.duplicate_window_secs;
        let f = self.force_model_config_key.trim();
        c.force_model_config_key = if f.is_empty() {
            None
        } else {
            Some(f.to_string())
        };
        c
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProxyOptions {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_proxy_port")]
    pub port: u16,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub oz_token: String,
}

impl ProxyOptions {
    fn channel(&self) -> WarpChannel {
        match self.channel.as_str() {
            "staging" => WarpChannel::Staging,
            "dev" => WarpChannel::Dev,
            _ => WarpChannel::Production,
        }
    }

    fn build_config(&self) -> ProxyConfig {
        let mut config = ProxyConfig::for_channel(self.channel());
        let token = self.oz_token.trim();
        config.oz_token = if token.is_empty() {
            ProxyConfig::token_from_env()
        } else {
            token.to_string()
        };
        config
    }
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}
fn default_gateway_port() -> u16 {
    8787
}
fn default_proxy_port() -> u16 {
    8788
}
fn default_true() -> bool {
    true
}
fn default_dup_window() -> u64 {
    60
}

fn nullable(v: &str) -> Option<String> {
    let t = v.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

// ---------------------------------------------------------------------------
// Provider store commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_providers() -> CmdResult<Vec<StoredProvider>> {
    Ok(ProviderStore::load().unwrap_or_default().providers)
}

#[tauri::command]
pub fn save_provider(provider: ProviderInput) -> CmdResult<Vec<StoredProvider>> {
    if !provider.is_valid() {
        return Err("provider needs a name and base URL".to_string());
    }
    let mut store = ProviderStore::load().unwrap_or_default();
    store.upsert(provider.to_stored());
    store.save()?;
    Ok(store.providers)
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteProviderResult {
    pub providers: Vec<StoredProvider>,
    pub gateway_stopped: bool,
    pub tunnel_stopped: bool,
    pub warp_endpoint_removed: bool,
    pub warp_running: bool,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub async fn delete_provider(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    name: String,
) -> CmdResult<DeleteProviderResult> {
    let mut store = ProviderStore::load().unwrap_or_default();
    let mut removed_provider = None;
    if let Some(index) = store
        .providers
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(&name))
    {
        removed_provider = Some(store.providers[index].clone());
        store.remove(index);
        store.save()?;
    }

    let mut warnings = Vec::new();
    let mut gateway_stopped = false;
    let mut tunnel_stopped = false;
    let mut warp_endpoint_removed = false;
    let mut warp_running = false;

    if let Some(provider) = removed_provider {
        {
            let mut launch = state.launch.lock().await;
            if launch.tunnel.is_some() || launch.public_url.is_some() {
                launch.tunnel = None;
                launch.public_url = None;
                tunnel_stopped = true;
            }
            launch.started_tunnel = false;
            launch.started_gateway = false;
            launch.cancel_requested = false;
            launch.phase = LaunchPhase::Idle;
            emit_phase(&app, &launch.phase);
        }

        let gateway = state.gateway.lock().await;
        if gateway.is_running() {
            gateway.stop(&app).await;
            gateway_stopped = true;
        }
        drop(gateway);

        let warp = { state.tools.lock().await.warp.clone() };
        if let Some(warp) = warp {
            let warp = std::path::PathBuf::from(warp);
            match crate::warp_setup::is_warp_running(&warp) {
                Ok(running) => {
                    warp_running = running;
                    if running {
                        warnings.push(
                            "Warp is running. Restart Warp for the custom provider removal to apply."
                                .to_string(),
                        );
                    }
                }
                Err(err) => warnings.push(err),
            }
            match crate::warp_setup::remove_warp_gateway_endpoint(&warp, &provider.name) {
                Ok(removed) => warp_endpoint_removed = removed,
                Err(err) => warnings.push(err),
            }
        } else {
            warnings.push("Warp not found; custom provider cleanup was skipped.".to_string());
        }
    }

    Ok(DeleteProviderResult {
        providers: store.providers,
        gateway_stopped,
        tunnel_stopped,
        warp_endpoint_removed,
        warp_running,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Gateway commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn gateway_status(state: State<'_, Arc<AppState>>) -> CmdResult<GatewayStatus> {
    Ok(state.gateway.lock().await.status())
}

#[tauri::command]
pub async fn start_gateway(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    provider: ProviderInput,
    options: GatewayOptions,
) -> CmdResult<GatewayStatus> {
    if !provider.is_valid() {
        return Err("provider needs a name and base URL".to_string());
    }
    let config = options.build_config(&provider);
    let gateway = state.gateway.lock().await;
    gateway
        .start(&options.host, options.port, config, &app)
        .await
}

#[tauri::command]
pub async fn stop_gateway(app: AppHandle, state: State<'_, Arc<AppState>>) -> CmdResult<()> {
    state.gateway.lock().await.stop(&app).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Proxy commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn proxy_status(state: State<'_, Arc<AppState>>) -> CmdResult<ProxyStatus> {
    Ok(state.proxy.lock().await.status())
}

#[tauri::command]
pub async fn start_proxy(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    options: ProxyOptions,
) -> CmdResult<ProxyStatus> {
    let config = options.build_config();
    let status = state
        .proxy
        .lock()
        .await
        .start(&options.host, options.port, config)
        .await;
    let _ = app.emit("proxy://status", &status);
    Ok(status)
}

#[tauri::command]
pub async fn stop_proxy(app: AppHandle, state: State<'_, Arc<AppState>>) -> CmdResult<()> {
    state.proxy.lock().await.stop().await;
    let _ = app.emit("proxy://status", ProxyStatus::Stopped);
    Ok(())
}

// ---------------------------------------------------------------------------
// Launch flow
// ---------------------------------------------------------------------------

/// Run the full guided launch: start gateway -> start tunnel -> wait for public
/// URL -> spawn Warp. Emits `launch://phase` at each transition. This is an
/// async driver replacing the per-frame egui state machine.
#[tauri::command]
pub async fn start_launch_flow(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    provider: ProviderInput,
    options: GatewayOptions,
) -> CmdResult<()> {
    if !provider.is_valid() {
        return Err("provider needs a name and base URL".to_string());
    }

    // Reset launch bookkeeping.
    {
        let mut launch = state.launch.lock().await;
        launch.cancel_requested = false;
        launch.started_gateway = false;
        launch.started_tunnel = false;
        launch.tunnel = None;
        launch.public_url = None;
        launch.phase = LaunchPhase::StartingGateway;
        emit_phase(&app, &launch.phase);
    }

    // 1) Gateway. Restart with the current provider form so in-memory
    // credentials and provider edits are applied to this launch.
    let gateway_port = {
        let gateway = state.gateway.lock().await;
        if gateway.is_running() {
            gateway.stop(&app).await;
        }
        let config = options.build_config(&provider);
        match gateway
            .start(&options.host, options.port, config, &app)
            .await
        {
            Ok(GatewayStatus::Running { addr }) => {
                state.launch.lock().await.started_gateway = true;
                addr.parse::<std::net::SocketAddr>()
                    .map(|addr| addr.port())
                    .map_err(|err| format!("gateway returned invalid address '{addr}': {err}"))?
            }
            Ok(other) => {
                return fail(&app, &state, format!("gateway: {other:?}")).await;
            }
            Err(message) => {
                return fail(&app, &state, format!("gateway: {message}")).await;
            }
        }
    };

    if cancelled(&state).await {
        return cancel_cleanup(&app, &state).await;
    }

    // 2) Tunnel.
    {
        let mut launch = state.launch.lock().await;
        launch.phase = LaunchPhase::StartingTunnel;
        emit_phase(&app, &launch.phase);
        if launch.tunnel.is_none() {
            let cloudflared = {
                let tools = state.tools.lock().await;
                tools.cloudflared.clone()
            };
            let Some(cloudflared) = cloudflared else {
                drop(launch);
                return fail(&app, &state, "cloudflared not found - install it first").await;
            };
            match crate::warp_setup::start_cloudflared_tunnel(
                &std::path::PathBuf::from(cloudflared),
                gateway_port,
            ) {
                Ok(tunnel) => {
                    launch.tunnel = Some(tunnel);
                    launch.public_url = None;
                    launch.started_tunnel = true;
                }
                Err(err) => {
                    drop(launch);
                    return fail(&app, &state, err).await;
                }
            }
        }
        launch.phase = LaunchPhase::WaitingPublicUrl;
        emit_phase(&app, &launch.phase);
    }

    // 3) Wait for public URL (polled by the background poller, surfaced here).
    let started = std::time::Instant::now();
    let public_url = loop {
        if cancelled(&state).await {
            return cancel_cleanup(&app, &state).await;
        }
        let url = { state.launch.lock().await.public_url.clone() };
        if let Some(url) = url {
            break url;
        }
        if started.elapsed() >= PUBLIC_URL_TIMEOUT {
            return fail(
                &app,
                &state,
                "timed out waiting for the cloudflared public URL",
            )
            .await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    };

    // 4) Spawn Warp.
    {
        let mut launch = state.launch.lock().await;
        launch.phase = LaunchPhase::SpawningWarp;
        emit_phase(&app, &launch.phase);
    }
    let warp = {
        let tools = state.tools.lock().await;
        tools.warp.clone()
    };
    let Some(warp) = warp else {
        return fail(&app, &state, "Warp not found - install it first").await;
    };
    let endpoint_url = format!("{public_url}/v1");
    let warp = std::path::PathBuf::from(warp);
    match crate::warp_setup::is_warp_running(&warp) {
        Ok(true) => {
            return fail(
                &app,
                &state,
                "Warp is running. Exit Warp completely, then start the gateway flow again so the endpoint update can be loaded safely.",
            )
            .await;
        }
        Ok(false) => {}
        Err(err) => return fail(&app, &state, err).await,
    }
    if let Err(err) = crate::warp_setup::upsert_warp_gateway_endpoint(
        &warp,
        &provider.name,
        &endpoint_url,
        provider.resolved_api_key().as_deref(),
        provider.model.trim(),
    ) {
        return fail(&app, &state, err).await;
    }
    match crate::warp_setup::spawn_warp(&warp) {
        Ok(()) => {
            let mut launch = state.launch.lock().await;
            launch.phase = LaunchPhase::Done;
            launch.started_gateway = false;
            launch.started_tunnel = false;
            emit_phase(&app, &launch.phase);
            Ok(())
        }
        Err(err) => fail(&app, &state, err).await,
    }
}

async fn cancelled(state: &State<'_, Arc<AppState>>) -> bool {
    state.launch.lock().await.cancel_requested
}

async fn fail(
    app: &AppHandle,
    state: &State<'_, Arc<AppState>>,
    message: impl Into<String>,
) -> CmdResult<()> {
    let message = message.into();
    let mut launch = state.launch.lock().await;
    launch.phase = LaunchPhase::Failed(message.clone());
    emit_phase(app, &launch.phase);
    Err(message)
}

#[tauri::command]
pub async fn cancel_launch(app: AppHandle, state: State<'_, Arc<AppState>>) -> CmdResult<()> {
    state.launch.lock().await.cancel_requested = true;
    cancel_cleanup(&app, &state).await
}

async fn cancel_cleanup(app: &AppHandle, state: &State<'_, Arc<AppState>>) -> CmdResult<()> {
    let (started_gateway, started_tunnel) = {
        let mut launch = state.launch.lock().await;
        let g = launch.started_gateway;
        let t = launch.started_tunnel;
        if t {
            launch.tunnel = None;
            launch.public_url = None;
        }
        (g, t)
    };
    if started_gateway {
        state.gateway.lock().await.stop(app).await;
    }
    let mut launch = state.launch.lock().await;
    launch.started_gateway = false;
    launch.started_tunnel = false;
    launch.phase = LaunchPhase::Idle;
    let _ = started_tunnel;
    emit_phase(app, &launch.phase);
    Ok(())
}

#[tauri::command]
pub async fn reset_launch(app: AppHandle, state: State<'_, Arc<AppState>>) -> CmdResult<()> {
    cancel_cleanup(&app, &state).await
}

#[tauri::command]
pub async fn launch_phase(state: State<'_, Arc<AppState>>) -> CmdResult<LaunchPhase> {
    Ok(state.launch.lock().await.phase.clone())
}

// ---------------------------------------------------------------------------
// Probe
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn start_probe(
    app: AppHandle,
    base_url: String,
    api_key: Option<String>,
) -> CmdResult<ProbeResult> {
    let result = run_probe(base_url, api_key).await;
    let _ = app.emit("probe://result", &result);
    Ok(result)
}

// ---------------------------------------------------------------------------
// Tunnel (manual control, outside the guided flow)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn start_tunnel(state: State<'_, Arc<AppState>>, port: u16) -> CmdResult<()> {
    let mut launch = state.launch.lock().await;
    if launch.tunnel.is_some() {
        return Ok(());
    }
    let cloudflared = {
        let tools = state.tools.lock().await;
        tools.cloudflared.clone()
    };
    let Some(cloudflared) = cloudflared else {
        return Err("cloudflared not found".to_string());
    };
    let tunnel =
        crate::warp_setup::start_cloudflared_tunnel(&std::path::PathBuf::from(cloudflared), port)?;
    launch.tunnel = Some(tunnel);
    launch.public_url = None;
    Ok(())
}

#[tauri::command]
pub async fn stop_tunnel(state: State<'_, Arc<AppState>>) -> CmdResult<()> {
    let mut launch = state.launch.lock().await;
    launch.tunnel = None;
    launch.public_url = None;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tool detection + install
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn detect_tools(state: State<'_, Arc<AppState>>) -> CmdResult<ToolPaths> {
    let tools = ToolPaths {
        warp: crate::warp_setup::detect_warp().map(|p| p.display().to_string()),
        cloudflared: crate::warp_setup::detect_cloudflared().map(|p| p.display().to_string()),
    };
    *state.tools.lock().await = tools.clone();
    Ok(tools)
}

#[tauri::command]
pub fn install_warp() -> CmdResult<()> {
    crate::warp_setup::install_warp_via_winget()
}

#[tauri::command]
pub fn install_cloudflared() -> CmdResult<()> {
    crate::warp_setup::install_cloudflared_via_winget()
}

// ---------------------------------------------------------------------------
// Endpoint helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct EndpointInfo {
    pub url: String,
    pub public: bool,
}

#[tauri::command]
pub async fn endpoint_url(
    state: State<'_, Arc<AppState>>,
    host: String,
    port: u16,
) -> CmdResult<EndpointInfo> {
    let launch = state.launch.lock().await;
    if let Some(public) = &launch.public_url {
        return Ok(EndpointInfo {
            url: format!("{public}/v1"),
            public: true,
        });
    }
    let gateway = state.gateway.lock().await;
    let url = match gateway.status() {
        GatewayStatus::Running { addr } => format!("http://{addr}/v1"),
        _ => format!("http://{host}:{port}/v1"),
    };
    Ok(EndpointInfo { url, public: false })
}

#[tauri::command]
pub async fn endpoint_config_json(
    state: State<'_, Arc<AppState>>,
    name: String,
    model: String,
    host: String,
    port: u16,
) -> CmdResult<String> {
    let info = endpoint_url(state, host, port).await?;
    let model = if model.trim().is_empty() {
        "default-model".to_string()
    } else {
        model.trim().to_string()
    };
    Ok(crate::warp_setup::endpoint_config_json(
        name.trim(),
        &info.url,
        &model,
    ))
}

// ---------------------------------------------------------------------------
// Proxy override-support detection
// ---------------------------------------------------------------------------

/// Classify whether the detected Warp build is likely to honor the
/// `WARP_*SERVER_URL` overrides that proxy mode depends on. Returns one of
/// "supported" | "unsupported" | "unknown" | "missing".
#[tauri::command]
pub async fn warp_override_support(state: State<'_, Arc<AppState>>) -> CmdResult<String> {
    use crate::warp_setup::{detect_server_url_override_support, ServerUrlOverrideSupport};
    let warp = { state.tools.lock().await.warp.clone() };
    let Some(warp) = warp else {
        return Ok("missing".to_string());
    };
    let label = match detect_server_url_override_support(&std::path::PathBuf::from(warp)) {
        ServerUrlOverrideSupport::Supported => "supported",
        ServerUrlOverrideSupport::Unsupported => "unsupported",
        ServerUrlOverrideSupport::Unknown => "unknown",
    };
    Ok(label.to_string())
}

/// Launch Warp with proxy override env vars pointed at the local proxy.
#[tauri::command]
pub async fn launch_warp_via_proxy(
    state: State<'_, Arc<AppState>>,
    http_root: String,
    ws_root: String,
) -> CmdResult<()> {
    let warp = { state.tools.lock().await.warp.clone() };
    let Some(warp) = warp else {
        return Err("Warp not found - install it first".to_string());
    };
    crate::warp_setup::spawn_warp_with_proxy_urls(
        &std::path::PathBuf::from(warp),
        &http_root,
        &ws_root,
        &ws_root,
    )
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn log_snapshot(state: State<'_, Arc<AppState>>) -> CmdResult<Vec<String>> {
    Ok(state.log_buffer.snapshot())
}

#[tauri::command]
pub fn clear_logs(state: State<'_, Arc<AppState>>) -> CmdResult<()> {
    state.log_buffer.clear();
    Ok(())
}
