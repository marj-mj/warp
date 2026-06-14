//! egui one-screen dashboard launcher for the Managed Provider Gateway.

use eframe::egui;
use std::time::{Duration, Instant};
use warp_gateway_wrapper::mpg::probe::CapabilityMatrix;
use warp_gateway_wrapper::mpg::{Adapter, GatewayConfig, WireApi};
use warp_gateway_wrapper::proxy::{ProxyConfig, WarpChannel};

use crate::controller::{GatewayController, GatewayStatus};
use crate::log_buffer::LogBuffer;
use crate::probe::PendingProbe;
use crate::proxy_controller::{ProxyController, ProxyStatus};
use crate::store::{ProviderStore, StoredProvider};
use crate::warp_setup::ServerUrlOverrideSupport;

#[derive(Debug, Clone, PartialEq, Eq)]
enum LaunchPhase {
    Idle,
    StartingGateway,
    StartingTunnel,
    WaitingPublicUrl,
    SpawningWarp,
    Done,
    Failed(String),
}

impl LaunchPhase {
    fn label(&self) -> String {
        match self {
            Self::Idle => "Ready".into(),
            Self::StartingGateway => "1/4 Starting gateway…".into(),
            Self::StartingTunnel => "2/4 Starting tunnel…".into(),
            Self::WaitingPublicUrl => "3/4 Waiting for public URL…".into(),
            Self::SpawningWarp => "4/4 Opening Warp…".into(),
            Self::Done => "Running — paste the endpoint into Warp if you haven't.".into(),
            Self::Failed(msg) => format!("Failed: {msg}"),
        }
    }
    fn is_active(&self) -> bool {
        matches!(
            self,
            Self::StartingGateway | Self::StartingTunnel | Self::WaitingPublicUrl | Self::SpawningWarp
        )
    }
    fn progress(&self) -> f32 {
        match self {
            Self::Idle => 0.0,
            Self::StartingGateway => 0.15,
            Self::StartingTunnel => 0.4,
            Self::WaitingPublicUrl => 0.6,
            Self::SpawningWarp => 0.85,
            Self::Done => 1.0,
            Self::Failed(_) => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceMode {
    ManagedProvider,
    WarpProxy,
}

impl SurfaceMode {
    fn label(self) -> &'static str {
        match self {
            Self::ManagedProvider => "Managed Provider",
            Self::WarpProxy => "Warp/OZ Proxy",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::ManagedProvider => "Gateway wrapper with a public HTTPS endpoint for Warp.",
            Self::WarpProxy => "Transparent proxy for override-capable Warp builds.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceTab {
    Configure,
    Runtime,
    Session,
    Diagnostics,
}

impl WorkspaceTab {
    fn label(self) -> &'static str {
        match self {
            Self::Configure => "Configure",
            Self::Runtime => "Runtime",
            Self::Session => "Session",
            Self::Diagnostics => "Diagnostics",
        }
    }
}

struct ProviderForm {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    env_key: String,
    wire_api: WireApi,
    adapter: Adapter,
}

impl Default for ProviderForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            base_url: String::new(),
            model: String::new(),
            api_key: String::new(),
            env_key: String::new(),
            wire_api: WireApi::Chat,
            adapter: Adapter::OpenaiChat,
        }
    }
}

impl ProviderForm {
    fn from_stored(s: &StoredProvider) -> Self {
        Self {
            name: s.name.clone(),
            base_url: s.base_url.clone(),
            model: s.model.clone().unwrap_or_default(),
            api_key: String::new(),
            env_key: s.env_key.clone().unwrap_or_default(),
            wire_api: s.wire_api,
            adapter: s.adapter,
        }
    }
    fn to_stored(&self) -> StoredProvider {
        StoredProvider {
            name: self.name.trim().to_string(),
            base_url: self.base_url.trim().to_string(),
            model: nullable(&self.model),
            wire_api: self.wire_api,
            adapter: self.adapter,
            env_key: nullable(&self.env_key),
        }
    }
    fn is_valid(&self) -> bool {
        !self.name.trim().is_empty() && !self.base_url.trim().is_empty()
    }
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
}

fn nullable(v: &str) -> Option<String> {
    let t = v.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

const PUBLIC_URL_TIMEOUT: Duration = Duration::from_secs(30);

struct GatewayForm {
    host: String,
    port: u16,
    disable_tools: bool,
    disable_mcp: bool,
    duplicate_window_secs: u64,
    force_model_config_key: String,
}

impl Default for GatewayForm {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8787,
            disable_tools: true,
            disable_mcp: true,
            duplicate_window_secs: 60,
            force_model_config_key: String::new(),
        }
    }
}

struct ProxyForm {
    host: String,
    port: u16,
    channel: WarpChannel,
    oz_token: String,
}

impl Default for ProxyForm {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8788,
            channel: WarpChannel::Production,
            oz_token: String::new(),
        }
    }
}

impl ProxyForm {
    fn build_config(&self) -> ProxyConfig {
        let mut config = ProxyConfig::for_channel(self.channel);
        config.oz_token = self.resolved_token();
        config
    }

    fn resolved_token(&self) -> String {
        let token = self.oz_token.trim();
        if !token.is_empty() {
            token.to_string()
        } else {
            ProxyConfig::token_from_env()
        }
    }

    fn auth_label(&self) -> &'static str {
        if self.oz_token.trim().is_empty() {
            "pass-through"
        } else {
            "override"
        }
    }
}

pub struct LauncherApp {
    controller: GatewayController,
    proxy_controller: ProxyController,
    active_mode: SurfaceMode,
    active_tab: WorkspaceTab,
    store: ProviderStore,
    selected: Option<usize>,
    form: ProviderForm,
    gateway: GatewayForm,
    proxy: ProxyForm,
    editing: bool,
    pending_probe: Option<PendingProbe>,
    last_matrix: Option<CapabilityMatrix>,
    last_message: String,
    tunnel: Option<crate::warp_setup::CloudflaredTunnel>,
    public_url: Option<String>,
    waiting_for_public_url_since: Option<Instant>,
    launch_phase: LaunchPhase,
    launch_started_gateway: bool,
    launch_started_tunnel: bool,
    warp_path: Option<std::path::PathBuf>,
    cloudflared_path: Option<std::path::PathBuf>,
    log_buffer: LogBuffer,
    show_advanced: bool,
    show_logs: bool,
}

impl LauncherApp {
    pub fn new(
        controller: GatewayController,
        proxy_controller: ProxyController,
        log_buffer: LogBuffer,
    ) -> Self {
        let store = ProviderStore::load().unwrap_or_default();
        let selected = if store.providers.is_empty() { None } else { Some(0) };
        let form = selected
            .and_then(|i| store.providers.get(i))
            .map(ProviderForm::from_stored)
            .unwrap_or_default();
        let editing = store.providers.is_empty();
        Self {
            controller,
            proxy_controller,
            active_mode: SurfaceMode::ManagedProvider,
            active_tab: WorkspaceTab::Configure,
            store,
            selected,
            form,
            gateway: GatewayForm::default(),
            proxy: ProxyForm::default(),
            editing,
            pending_probe: None,
            last_matrix: None,
            last_message: String::new(),
            tunnel: None,
            public_url: None,
            waiting_for_public_url_since: None,
            launch_phase: LaunchPhase::Idle,
            launch_started_gateway: false,
            launch_started_tunnel: false,
            warp_path: crate::warp_setup::detect_warp(),
            cloudflared_path: crate::warp_setup::detect_cloudflared(),
            log_buffer,
            show_advanced: false,
            show_logs: false,
        }
    }

    fn build_config(&self) -> GatewayConfig {
        let provider = self.form.to_stored().to_config(self.form.resolved_api_key());
        let mut c = GatewayConfig::new(provider);
        c.disable_tools = self.gateway.disable_tools;
        c.disable_mcp = self.gateway.disable_mcp;
        c.duplicate_window_secs = self.gateway.duplicate_window_secs;
        let f = self.gateway.force_model_config_key.trim();
        c.force_model_config_key = if f.is_empty() { None } else { Some(f.to_string()) };
        c
    }

    fn build_proxy_config(&self) -> ProxyConfig {
        self.proxy.build_config()
    }

    fn endpoint_url(&self) -> String {
        match &self.public_url {
            Some(p) => format!("{p}/v1"),
            None => match self.controller.status() {
                GatewayStatus::Running { addr } => format!("http://{addr}/v1"),
                _ => format!("http://{}:{}/v1", self.gateway.host, self.gateway.port),
            },
        }
    }

    fn proxy_http_root(&self) -> String {
        match self.proxy_controller.status() {
            ProxyStatus::Running { addr } => format!("http://{addr}"),
            _ => format!("http://{}:{}", self.proxy.host, self.proxy.port),
        }
    }

    fn proxy_ws_root(&self) -> String {
        let http_root = self.proxy_http_root();
        if let Some(rest) = http_root.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = http_root.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            http_root
        }
    }

    fn warp_override_support(&self) -> Option<ServerUrlOverrideSupport> {
        self.warp_path
            .as_deref()
            .map(crate::warp_setup::detect_server_url_override_support)
    }

    fn proxy_launch_blocker(&self) -> Option<&'static str> {
        let Some(path) = self.warp_path.as_deref() else {
            return Some("Warp not found — install from Advanced");
        };
        match crate::warp_setup::detect_server_url_override_support(path) {
            ServerUrlOverrideSupport::Unsupported => {
                Some("Installed Warp build ignores server URL overrides")
            }
            ServerUrlOverrideSupport::Supported | ServerUrlOverrideSupport::Unknown => None,
        }
    }

    fn start_proxy(&mut self) {
        if self.proxy_controller.is_running() {
            return;
        }
        let config = self.build_proxy_config();
        self.proxy_controller
            .start(&self.proxy.host, self.proxy.port, config);
        match self.proxy_controller.status() {
            ProxyStatus::Running { ref addr } => {
                self.last_message = format!("Proxy listening at http://{addr}");
            }
            ProxyStatus::Error { ref message } => {
                self.last_message = format!("Proxy failed: {message}");
            }
            ProxyStatus::Stopped => {}
        }
    }

    fn launch_warp_via_proxy(&mut self) {
        if let Some(message) = self.proxy_launch_blocker() {
            self.last_message = message.into();
            return;
        }
        if !self.proxy_controller.is_running() {
            self.start_proxy();
        }
        match self.proxy_controller.status() {
            ProxyStatus::Running { .. } => {}
            ProxyStatus::Error { message } => {
                self.last_message = format!("Proxy failed: {message}");
                return;
            }
            ProxyStatus::Stopped => {
                self.last_message = "Proxy is not running.".into();
                return;
            }
        }

        let Some(warp) = self.warp_path.clone() else {
            self.last_message = "Warp not found — install from Advanced".into();
            return;
        };
        let http_root = self.proxy_http_root();
        let ws_root = self.proxy_ws_root();
        match crate::warp_setup::spawn_warp_with_proxy_urls(&warp, &http_root, &ws_root, &ws_root)
        {
            Ok(()) => {
                self.last_message =
                    "Warp opened with proxy overrides. Keep the proxy running while Warp is active."
                        .into();
            }
            Err(err) => self.last_message = err,
        }
    }

    fn try_start_tunnel(&mut self, owned_by_launch: bool) -> Result<(), String> {
        if self.tunnel.is_some() {
            return Ok(());
        }
        let Some(p) = self.cloudflared_path.clone() else {
            return Err("cloudflared not found".into());
        };
        match crate::warp_setup::start_cloudflared_tunnel(&p, self.gateway.port) {
            Ok(t) => {
                self.tunnel = Some(t);
                self.public_url = None;
                if owned_by_launch {
                    self.launch_started_tunnel = true;
                }
                Ok(())
            }
            Err(e) => {
                self.last_message = e.clone();
                Err(e)
            }
        }
    }

    fn start_probe(&mut self) {
        let h = self.controller.runtime_handle();
        self.pending_probe = Some(PendingProbe::spawn(
            &h,
            self.form.base_url.trim().to_string(),
            self.form.resolved_api_key(),
        ));
        self.last_message = "Probing…".into();
    }

    fn poll(&mut self) {
        if let Some(p) = self.pending_probe.as_mut() {
            if let Some(m) = p.poll() {
                self.last_matrix = Some(m);
                self.pending_probe = None;
                self.last_message = "Probe finished.".into();
            }
        }
        if self.public_url.is_none() {
            let mut tunnel_exit_message = None;
            if let Some(tunnel) = self.tunnel.as_mut() {
                if let Some(url) = tunnel.try_url() {
                    self.public_url = Some(url.clone());
                    self.waiting_for_public_url_since = None;
                    self.last_message = format!("Public URL ready: {url}");
                } else {
                    match tunnel.try_exit_status() {
                        Ok(Some(status)) => {
                            let detail = status
                                .code()
                                .map(|code| format!("exit code {code}"))
                                .unwrap_or_else(|| "terminated by signal".to_string());
                            tunnel_exit_message =
                                Some(format!("cloudflared exited before publishing a URL ({detail})"));
                        }
                        Ok(None) => {}
                        Err(err) => tunnel_exit_message = Some(err),
                    }
                }
            }

            if let Some(message) = tunnel_exit_message {
                self.tunnel = None;
                self.public_url = None;
                self.waiting_for_public_url_since = None;
                self.last_message = message.clone();
                if matches!(
                    self.launch_phase,
                    LaunchPhase::StartingTunnel | LaunchPhase::WaitingPublicUrl
                ) {
                    self.fail_launch(message);
                }
            }
        }
    }

    fn fail_launch(&mut self, message: impl Into<String>) {
        self.waiting_for_public_url_since = None;
        self.launch_phase = LaunchPhase::Failed(message.into());
    }

    fn release_launch_ownership(&mut self) {
        self.waiting_for_public_url_since = None;
        self.launch_started_gateway = false;
        self.launch_started_tunnel = false;
    }

    fn cancel_launch(&mut self) {
        if self.launch_started_tunnel {
            self.tunnel = None;
            self.public_url = None;
        }
        if self.launch_started_gateway {
            self.controller.stop();
        }
        self.release_launch_ownership();
        self.launch_phase = LaunchPhase::Idle;
        self.last_message = "Launch cancelled.".into();
    }

    fn reset_launch_state(&mut self) {
        if matches!(self.launch_phase, LaunchPhase::Failed(_)) {
            if self.launch_started_tunnel {
                self.tunnel = None;
                self.public_url = None;
            }
            if self.launch_started_gateway {
                self.controller.stop();
            }
        }
        self.release_launch_ownership();
        self.launch_phase = LaunchPhase::Idle;
    }

    fn prepare_launch_attempt(&mut self) {
        if matches!(self.launch_phase, LaunchPhase::Failed(_)) {
            self.reset_launch_state();
        }
        self.release_launch_ownership();
        self.launch_phase = LaunchPhase::StartingGateway;
    }

    fn copy_endpoint_config(&mut self) {
        let url = self.endpoint_url();
        let model = if self.form.model.trim().is_empty() {
            "default-model".to_string()
        } else {
            self.form.model.trim().to_string()
        };
        let json = crate::warp_setup::endpoint_config_json(self.form.name.trim(), &url, &model);
        copy(&json);
        self.last_message = "Endpoint config copied — paste into Warp Settings.".into();
    }

    fn copy_endpoint_url(&mut self) {
        let url = self.endpoint_url();
        copy(&url);
        self.last_message = "URL copied.".into();
    }

    fn step_launch(&mut self) {
        match self.launch_phase.clone() {
            LaunchPhase::Idle | LaunchPhase::Done | LaunchPhase::Failed(_) => {}
            LaunchPhase::StartingGateway => {
                if !self.form.is_valid() {
                    self.fail_launch("provider needs Name + Base URL");
                    return;
                }
                if !self.controller.is_running() {
                    let c = self.build_config();
                    self.launch_started_gateway = true;
                    self.controller.start(&self.gateway.host, self.gateway.port, c);
                }
                self.launch_phase = match self.controller.status() {
                    GatewayStatus::Running { .. } => LaunchPhase::StartingTunnel,
                    GatewayStatus::Error { message } => {
                        self.launch_started_gateway = false;
                        LaunchPhase::Failed(format!("gateway: {message}"))
                    }
                    _ => LaunchPhase::StartingGateway,
                };
            }
            LaunchPhase::StartingTunnel => {
                if self.tunnel.is_none() {
                    if self.cloudflared_path.is_none() {
                        self.fail_launch("cloudflared not found — install from Advanced");
                        return;
                    }
                    if let Err(message) = self.try_start_tunnel(true) {
                        self.fail_launch(message);
                        return;
                    }
                }
                if self.public_url.is_some() {
                    self.waiting_for_public_url_since = None;
                    self.launch_phase = LaunchPhase::SpawningWarp;
                } else {
                    self.waiting_for_public_url_since.get_or_insert_with(Instant::now);
                    self.launch_phase = LaunchPhase::WaitingPublicUrl;
                }
            }
            LaunchPhase::WaitingPublicUrl => {
                if self.public_url.is_some() {
                    self.waiting_for_public_url_since = None;
                    self.launch_phase = LaunchPhase::SpawningWarp;
                } else if self
                    .waiting_for_public_url_since
                    .is_some_and(|started_at| started_at.elapsed() >= PUBLIC_URL_TIMEOUT)
                {
                    self.fail_launch("timed out waiting for the cloudflared public URL");
                } else {
                    self.waiting_for_public_url_since.get_or_insert_with(Instant::now);
                }
            }
            LaunchPhase::SpawningWarp => {
                let Some(w) = self.warp_path.clone() else {
                    self.fail_launch("Warp not found — install from Advanced");
                    return;
                };
                match crate::warp_setup::spawn_warp(&w) {
                    Ok(()) => {
                        self.launch_phase = LaunchPhase::Done;
                        self.last_message = "Warp opened. Copy config JSON then paste into Warp Settings.".into();
                        self.release_launch_ownership();
                    }
                    Err(e) => self.fail_launch(e),
                }
            }
        }
    }

    fn persist(&mut self) {
        if let Err(e) = self.store.save() {
            self.last_message = format!("Save failed: {e}");
        }
    }
}

impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        self.step_launch();

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(crate::theme::BG).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    crate::theme::card_frame().show(ui, |ui| self.header_card(ui));
                    crate::theme::card_frame().show(ui, |ui| self.mode_switch_card(ui));
                    crate::theme::card_frame().show(ui, |ui| self.workflow_card(ui));
                    crate::theme::card_frame().show(ui, |ui| self.workspace_card(ui));
                });
            });

        ctx.request_repaint_after(std::time::Duration::from_millis(400));
    }
}

impl LauncherApp {
    fn header_card(&self, ui: &mut egui::Ui) {
        let full_width = ui.available_width();
        ui.set_min_width(full_width);
        if full_width >= 920.0 {
            let status_width = 340.0;
            let summary_width = (full_width - status_width - 12.0).max(280.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(summary_width);
                    ui.label(
                        egui::RichText::new("Warp Gateway Launcher")
                            .size(22.0)
                            .strong()
                            .color(crate::theme::TEXT),
                    );
                    ui.label(
                        egui::RichText::new(self.active_mode.subtitle())
                            .color(crate::theme::TEXT_WEAK)
                            .size(12.5),
                    );
                });
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                    ui.set_width(status_width);
                    self.status_row(ui);
                });
            });
        } else {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("Warp Gateway Launcher")
                        .size(22.0)
                        .strong()
                        .color(crate::theme::TEXT),
                );
                ui.label(
                    egui::RichText::new(self.active_mode.subtitle())
                        .color(crate::theme::TEXT_WEAK)
                        .size(12.5),
                );
            });
            ui.add_space(8.0);
            self.status_row(ui);
        }
        if !self.last_message.is_empty() {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(&self.last_message)
                    .color(crate::theme::TEXT_WEAK)
                    .size(12.5),
            );
        }
    }

    fn mode_switch_card(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("Connection Mode")
                    .size(12.0)
                    .strong()
                    .color(crate::theme::TEXT_WEAK),
            );
            ui.label(
                egui::RichText::new(self.active_mode.subtitle())
                    .size(12.0)
                    .color(crate::theme::TEXT_WEAK),
            );
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for mode in [SurfaceMode::ManagedProvider, SurfaceMode::WarpProxy] {
                let selected = self.active_mode == mode;
                let button = egui::Button::new(
                    egui::RichText::new(mode.label())
                        .size(13.0)
                        .color(if selected {
                            egui::Color32::WHITE
                        } else {
                            crate::theme::TEXT_WEAK
                        }),
                )
                .fill(if selected {
                    crate::theme::ACCENT.linear_multiply(0.2)
                } else {
                    crate::theme::SURFACE_ALT
                })
                .stroke(egui::Stroke::new(
                    1.0,
                    if selected {
                        crate::theme::ACCENT
                    } else {
                        crate::theme::BORDER
                    },
                ))
                .min_size(egui::vec2(168.0, 34.0));
                if ui.add(button).clicked() {
                    self.active_mode = mode;
                    self.active_tab = WorkspaceTab::Configure;
                }
            }
        });
    }

    fn workflow_card(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("Workflow")
                .size(12.0)
                .strong()
                .color(crate::theme::TEXT_WEAK),
        );
        ui.add_space(8.0);

        match self.active_mode {
            SurfaceMode::ManagedProvider => {
                let steps = [
                    ("1", "Provider preset ready", self.form.is_valid()),
                    ("2", "Gateway is listening", self.controller.is_running()),
                    ("3", "Public HTTPS endpoint ready", self.public_url.is_some()),
                ];
                workflow_steps(ui, &steps);

                let (color, note) = match &self.launch_phase {
                    LaunchPhase::Failed(message) => (crate::theme::ERR, message.clone()),
                    LaunchPhase::Done => (
                        crate::theme::OK,
                        "Warp is open. Copy the endpoint JSON into Warp Settings.".into(),
                    ),
                    _ if !self.form.is_valid() => (
                        crate::theme::WARN,
                        "Pick or save a provider preset first.".into(),
                    ),
                    _ if !self.controller.is_running() => (
                        crate::theme::INFO,
                        "Start the gateway flow to bind the local wrapper.".into(),
                    ),
                    _ if self.public_url.is_none() => (
                        crate::theme::WARN,
                        "Wait for cloudflared to publish an HTTPS endpoint.".into(),
                    ),
                    _ => (
                        crate::theme::OK,
                        "Copy config JSON from Custom Endpoint and paste it into Warp.".into(),
                    ),
                };

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("Next")
                            .size(11.5)
                            .strong()
                            .color(crate::theme::TEXT_WEAK),
                    );
                    ui.colored_label(color, note);
                });
                ui.add_space(10.0);
                self.workflow_actions(ui);
            }
            SurfaceMode::WarpProxy => {
                let steps = [
                    ("1", "Warp binary detected", self.warp_path.is_some()),
                    ("2", "Override launch allowed", self.proxy_launch_blocker().is_none()),
                    ("3", "Local proxy is running", self.proxy_controller.is_running()),
                ];
                workflow_steps(ui, &steps);

                let (color, note) = match self.proxy_controller.status() {
                    ProxyStatus::Error { message } => (crate::theme::ERR, message),
                    _ => match self.proxy_launch_blocker() {
                        Some(message) => (crate::theme::WARN, message.into()),
                        None if !self.proxy_controller.is_running() => (
                            crate::theme::INFO,
                            "Start the proxy, then open Warp with overrides.".into(),
                        ),
                        None => (
                            crate::theme::OK,
                            "Open Warp with proxy overrides and keep this proxy running.".into(),
                        ),
                    },
                };

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("Next")
                            .size(11.5)
                            .strong()
                            .color(crate::theme::TEXT_WEAK),
                    );
                    ui.colored_label(color, note);
                });
                ui.add_space(10.0);
                self.workflow_actions(ui);
            }
        }
    }

    fn workflow_actions(&mut self, ui: &mut egui::Ui) {
        match self.active_mode {
            SurfaceMode::ManagedProvider => {
                let busy = self.launch_phase.is_active();
                let can_start = self.form.is_valid() && !busy;
                ui.horizontal_wrapped(|ui| {
                    let btn = egui::Button::new(
                        egui::RichText::new("Start gateway flow")
                            .size(14.0)
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(if can_start {
                        crate::theme::ACCENT
                    } else {
                        crate::theme::SURFACE_HI
                    })
                    .min_size(egui::vec2(196.0, 38.0));
                    if ui.add_enabled(can_start, btn).clicked() {
                        self.prepare_launch_attempt();
                    }
                    if busy {
                        if ui.add(danger_button("Cancel")).clicked() {
                            self.cancel_launch();
                        }
                    }
                    if matches!(self.launch_phase, LaunchPhase::Done | LaunchPhase::Failed(_))
                        && ui.add(subtle_button("Reset")).clicked()
                    {
                        self.reset_launch_state();
                    }
                    if self.public_url.is_some()
                        && ui.add(subtle_button("Copy config JSON")).clicked()
                    {
                        self.copy_endpoint_config();
                    }
                });
                ui.add_space(6.0);
                self.workspace_tab_strip(ui);
            }
            SurfaceMode::WarpProxy => {
                let running = self.proxy_controller.is_running();
                ui.horizontal_wrapped(|ui| {
                    if running {
                        if ui.add(danger_button("Stop proxy")).clicked() {
                            self.proxy_controller.stop();
                            self.last_message = "Proxy stopped.".into();
                        }
                    } else if ui.add(secondary_button("Start proxy")).clicked() {
                        self.start_proxy();
                    }

                    let launch_label = if running {
                        "Open Warp with proxy"
                    } else {
                        "Start proxy + open Warp"
                    };
                    let open = egui::Button::new(
                        egui::RichText::new(launch_label)
                            .size(14.0)
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(if self.proxy_launch_blocker().is_none() {
                        crate::theme::ACCENT
                    } else {
                        crate::theme::SURFACE_HI
                    })
                    .min_size(egui::vec2(196.0, 38.0));
                    if ui
                        .add_enabled(self.proxy_launch_blocker().is_none(), open)
                        .clicked()
                    {
                        self.launch_warp_via_proxy();
                    }
                });
                ui.add_space(6.0);
                self.workspace_tab_strip(ui);
            }
        }
    }

    fn workspace_card(&mut self, ui: &mut egui::Ui) {
        match (self.active_mode, self.active_tab) {
            (SurfaceMode::ManagedProvider, WorkspaceTab::Configure) => self.provider_card(ui),
            (SurfaceMode::ManagedProvider, WorkspaceTab::Runtime) => self.environment_card(ui),
            (SurfaceMode::ManagedProvider, WorkspaceTab::Session) => {
                self.launch_card(ui);
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);
                self.endpoint_card(ui);
            }
            (SurfaceMode::ManagedProvider, WorkspaceTab::Diagnostics) => self.advanced_card(ui),
            (SurfaceMode::WarpProxy, WorkspaceTab::Configure) => self.proxy_card(ui),
            (SurfaceMode::WarpProxy, WorkspaceTab::Runtime) => self.environment_card(ui),
            (SurfaceMode::WarpProxy, WorkspaceTab::Session) => self.proxy_overview_card(ui),
            (SurfaceMode::WarpProxy, WorkspaceTab::Diagnostics) => self.advanced_card(ui),
        }
    }

    fn workspace_tab_strip(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for tab in [
                WorkspaceTab::Configure,
                WorkspaceTab::Runtime,
                WorkspaceTab::Session,
                WorkspaceTab::Diagnostics,
            ] {
                let selected = self.active_tab == tab;
                let button = egui::Button::new(
                    egui::RichText::new(tab.label())
                        .color(if selected {
                            egui::Color32::WHITE
                        } else {
                            crate::theme::TEXT_WEAK
                        })
                        .size(12.5),
                )
                .fill(if selected {
                    crate::theme::ACCENT.linear_multiply(0.22)
                } else {
                    crate::theme::SURFACE_ALT
                })
                .stroke(egui::Stroke::new(
                    1.0,
                    if selected {
                        crate::theme::ACCENT
                    } else {
                        crate::theme::BORDER
                    },
                ))
                .min_size(egui::vec2(108.0, 34.0));
                if ui.add(button).clicked() {
                    self.active_tab = tab;
                }
            }
        });
    }

    fn environment_card(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Environment")
                    .size(12.0)
                    .strong()
                    .color(crate::theme::TEXT_WEAK),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.active_mode == SurfaceMode::ManagedProvider && self.cloudflared_path.is_none()
                {
                    if ui.add(secondary_button("Install cloudflared")).clicked() {
                        match crate::warp_setup::install_cloudflared_via_winget() {
                            Ok(()) => {
                                self.last_message =
                                    "cloudflared installer launched. Re-detect after install.".into();
                            }
                            Err(err) => self.last_message = err,
                        }
                    }
                }
                if self.warp_path.is_none() && ui.add(secondary_button("Install Warp")).clicked() {
                    match crate::warp_setup::install_warp_via_winget() {
                        Ok(()) => {
                            self.last_message =
                                "Warp installer launched. Re-detect after install.".into();
                        }
                        Err(err) => self.last_message = err,
                    }
                }
                if ui.add(subtle_button("Re-detect")).clicked() {
                    self.warp_path = crate::warp_setup::detect_warp();
                    self.cloudflared_path = crate::warp_setup::detect_cloudflared();
                    self.last_message = "Binary detection refreshed.".into();
                }
            });
        });
        ui.add_space(8.0);
        self.status_row(ui);
        ui.add_space(10.0);

        ui.label(
            egui::RichText::new("Warp binary")
                .size(12.0)
                .strong()
                .color(crate::theme::TEXT_WEAK),
        );
        match &self.warp_path {
            Some(path) => {
                ui.label(egui::RichText::new(path.display().to_string()).monospace());
                if self.active_mode == SurfaceMode::WarpProxy {
                    match self.warp_override_support() {
                        Some(ServerUrlOverrideSupport::Supported) => {
                            ui.colored_label(crate::theme::OK, "Supports server URL overrides.");
                        }
                        Some(ServerUrlOverrideSupport::Unsupported) => {
                            ui.colored_label(
                                crate::theme::WARN,
                                "This Warp build ignores WARP_*SERVER_URL overrides.",
                            );
                        }
                        Some(ServerUrlOverrideSupport::Unknown) | None => {
                            ui.colored_label(
                                crate::theme::INFO,
                                "Override support could not be confirmed from the binary path.",
                            );
                        }
                    }
                } else {
                    ui.colored_label(
                        crate::theme::INFO,
                        "Used to open Warp after the public endpoint is ready.",
                    );
                }
            }
            None => {
                ui.colored_label(crate::theme::ERR, "Not detected");
            }
        }

        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("cloudflared")
                .size(11.5)
                .strong()
                .color(crate::theme::TEXT_WEAK),
        );
        match &self.cloudflared_path {
            Some(path) => {
                ui.label(egui::RichText::new(path.display().to_string()).monospace());
                if self.active_mode == SurfaceMode::ManagedProvider {
                    ui.colored_label(
                        crate::theme::INFO,
                        "Used to publish the HTTPS endpoint Warp will accept.",
                    );
                }
            }
            None => {
                let message = if self.active_mode == SurfaceMode::ManagedProvider {
                    "Not detected. Required for the public HTTPS endpoint."
                } else {
                    "Not detected. Optional in proxy mode."
                };
                ui.colored_label(crate::theme::WARN, message);
            }
        }

        if self.active_mode == SurfaceMode::ManagedProvider {
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new("Gateway root")
                    .size(11.5)
                    .strong()
                    .color(crate::theme::TEXT_WEAK),
            );
            let local_root = match self.controller.status() {
                GatewayStatus::Running { addr } => format!("http://{addr}/v1"),
                _ => format!("http://{}:{}/v1", self.gateway.host, self.gateway.port),
            };
            ui.label(egui::RichText::new(local_root).monospace());
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Public endpoint")
                    .size(11.5)
                    .strong()
                    .color(crate::theme::TEXT_WEAK),
            );
            match &self.public_url {
                Some(url) => ui.label(egui::RichText::new(format!("{url}/v1")).monospace()),
                None => ui.colored_label(crate::theme::TEXT_WEAK, "Not published yet"),
            };
        }
    }

    fn proxy_overview_card(&self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("Proxy Session")
                .size(12.0)
                .strong()
                .color(crate::theme::TEXT_WEAK),
        );
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("HTTP root")
                .size(11.5)
                .strong()
                .color(crate::theme::TEXT_WEAK),
        );
        ui.label(egui::RichText::new(self.proxy_http_root()).monospace());
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("WebSocket root")
                .size(11.5)
                .strong()
                .color(crate::theme::TEXT_WEAK),
        );
        ui.label(egui::RichText::new(self.proxy_ws_root()).monospace());
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("Upstream")
                    .size(11.5)
                    .strong()
                    .color(crate::theme::TEXT_WEAK),
            );
            ui.label(match self.proxy.channel {
                WarpChannel::Production => "production",
                WarpChannel::Staging => "staging",
                WarpChannel::Dev => "dev",
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("Auth mode")
                    .size(11.5)
                    .strong()
                    .color(crate::theme::TEXT_WEAK),
            );
            ui.label(self.proxy.auth_label());
        });
        if self.proxy.oz_token.trim().is_empty() {
            ui.colored_label(
                crate::theme::INFO,
                "Without an override token, the proxy forwards Warp's original credentials.",
            );
        }
    }

    fn status_row(&self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            status_chip(ui, "Gateway", self.controller.is_running(), crate::theme::OK);
            status_chip(ui, "Proxy", self.proxy_controller.is_running(), crate::theme::INFO);
            status_chip(ui, "Public URL", self.public_url.is_some(), crate::theme::WARN);
            status_chip(ui, "Warp", self.warp_path.is_some(), crate::theme::ACCENT);
        });
    }

    fn provider_card(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Provider Preset").size(12.0).strong().color(crate::theme::TEXT_WEAK));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(subtle_button(if self.editing { "Close" } else { "Edit" }))
                    .clicked()
                {
                    self.editing = !self.editing;
                }
                if ui.add(secondary_button("+ New")).clicked() {
                    self.form = ProviderForm::default();
                    self.selected = None;
                    self.editing = true;
                    self.last_matrix = None;
                }
            });
        });
        ui.add_space(6.0);

        if !self.store.providers.is_empty() {
            ui.label(
                egui::RichText::new("Saved presets")
                    .size(11.5)
                    .strong()
                    .color(crate::theme::TEXT_WEAK),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let mut pick = None;
                for (i, p) in self.store.providers.iter().enumerate() {
                    let label = if p.name.is_empty() { "(unnamed)".into() } else { p.name.clone() };
                    if ui.selectable_label(self.selected == Some(i), label).clicked() {
                        pick = Some(i);
                    }
                }
                if let Some(i) = pick {
                    self.selected = Some(i);
                    self.form = ProviderForm::from_stored(&self.store.providers[i]);
                    self.editing = false;
                    self.last_matrix = None;
                }
            });
            ui.add_space(6.0);
        }

        if self.editing {
            self.provider_form(ui);
        } else if self.selected.is_some() {
            let credential = if !self.form.env_key.trim().is_empty() {
                format!("env: {}", self.form.env_key.trim())
            } else if self.form.api_key.trim().is_empty() {
                "not set".to_string()
            } else {
                "inline key".to_string()
            };
            egui::Grid::new("provider_summary")
                .num_columns(2)
                .spacing([10.0, 7.0])
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("Base URL")
                            .size(11.5)
                            .strong()
                            .color(crate::theme::TEXT_WEAK),
                    );
                    ui.label(egui::RichText::new(self.form.base_url.trim()).monospace());
                    ui.end_row();

                    ui.label(
                        egui::RichText::new("Model")
                            .size(11.5)
                            .strong()
                            .color(crate::theme::TEXT_WEAK),
                    );
                    ui.label(if self.form.model.trim().is_empty() {
                        "(default)"
                    } else {
                        self.form.model.trim()
                    });
                    ui.end_row();

                    ui.label(
                        egui::RichText::new("Adapter")
                            .size(11.5)
                            .strong()
                            .color(crate::theme::TEXT_WEAK),
                    );
                    ui.label(self.form.adapter.as_str());
                    ui.end_row();

                    ui.label(
                        egui::RichText::new("Wire API")
                            .size(11.5)
                            .strong()
                            .color(crate::theme::TEXT_WEAK),
                    );
                    ui.label(self.form.wire_api.as_str());
                    ui.end_row();

                    ui.label(
                        egui::RichText::new("Credential")
                            .size(11.5)
                            .strong()
                            .color(crate::theme::TEXT_WEAK),
                    );
                    ui.label(credential);
                    ui.end_row();
                });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                let can_probe = self.pending_probe.is_none() && !self.form.base_url.trim().is_empty();
                if ui.add_enabled(can_probe, secondary_button("Probe")).clicked() {
                    self.start_probe();
                }
                if self.pending_probe.is_some() {
                    ui.spinner();
                }
                if ui.add(subtle_button("Edit preset")).clicked() {
                    self.editing = true;
                }
            });
            if let Some(m) = &self.last_matrix {
                let rec = m.recommend();
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Probe: {} / {} ({})",
                        rec.adapter, rec.wire_api, rec.confidence
                    ))
                    .color(crate::theme::TEXT_WEAK),
                );
            }
        }
    }

    fn provider_form(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("pf").num_columns(2).spacing([10.0, 7.0]).show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut self.form.name);
            ui.end_row();
            ui.label("Base URL");
            ui.vertical(|ui| {
                ui.text_edit_singleline(&mut self.form.base_url);
                if !self.form.base_url.trim().is_empty() && needs_hint(&self.form.base_url) {
                    ui.colored_label(crate::theme::WARN, "Tip: base URL usually ends with /v1");
                }
            });
            ui.end_row();
            ui.label("Model");
            ui.text_edit_singleline(&mut self.form.model);
            ui.end_row();
            ui.label("API key");
            ui.add(egui::TextEdit::singleline(&mut self.form.api_key).password(true));
            ui.end_row();
            ui.label("Env key");
            ui.text_edit_singleline(&mut self.form.env_key);
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(self.form.is_valid(), secondary_button("Save")).clicked() {
                self.store.upsert(self.form.to_stored());
                self.selected = self.store.providers.iter().position(|p| p.name.eq_ignore_ascii_case(&self.form.name));
                self.persist();
                self.editing = false;
                self.last_message = "Saved.".into();
            }
            let pe = self.pending_probe.is_none() && !self.form.base_url.trim().is_empty();
            if ui.add_enabled(pe, secondary_button("Probe")).clicked() {
                self.start_probe();
            }
            if self.pending_probe.is_some() {
                ui.spinner();
            }
            if self.selected.is_some() && ui.add(danger_button("Delete")).clicked() {
                if let Some(i) = self.selected {
                    self.store.remove(i);
                    self.selected = if self.store.providers.is_empty() { None } else { Some(i.min(self.store.providers.len()-1)) };
                    self.form = self.selected.and_then(|j| self.store.providers.get(j)).map(ProviderForm::from_stored).unwrap_or_default();
                    self.persist();
                }
            }
        });
        if let Some(m) = &self.last_matrix {
            let rec = m.recommend();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("→ {} / {} ({})", rec.adapter, rec.wire_api, rec.confidence)).color(crate::theme::TEXT_WEAK));
                if ui.add(subtle_button("Apply")).clicked() {
                    self.form.adapter = match rec.adapter {
                        "openai_responses" => Adapter::OpenaiResponses,
                        "anthropic_messages" => Adapter::AnthropicMessages,
                        "gemini_generate_content" => Adapter::GeminiGenerateContent,
                        _ => Adapter::OpenaiChat,
                    };
                    self.form.wire_api = if rec.wire_api == "responses" { WireApi::Responses } else { WireApi::Chat };
                    self.gateway.disable_tools = rec.safe_mode_defaults.disable_tools;
                    self.gateway.disable_mcp = rec.safe_mode_defaults.disable_mcp;
                    self.last_message = "Applied recommendation.".into();
                }
            });
        }
    }

    fn launch_card(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("Gateway Session")
                .size(12.0)
                .strong()
                .color(crate::theme::TEXT_WEAK),
        );
        ui.add_space(8.0);
        let busy = self.launch_phase.is_active();
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("Gateway")
                    .size(11.5)
                    .strong()
                    .color(crate::theme::TEXT_WEAK),
            );
            ui.label(match self.controller.status() {
                GatewayStatus::Running { addr } => format!("http://{addr}/v1"),
                GatewayStatus::Error { ref message } => format!("error: {message}"),
                GatewayStatus::Stopped => format!("http://{}:{}/v1", self.gateway.host, self.gateway.port),
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("Tunnel")
                    .size(11.5)
                    .strong()
                    .color(crate::theme::TEXT_WEAK),
            );
            ui.label(match &self.public_url {
                Some(url) => format!("{url}/v1"),
                None if self.tunnel.is_some() => "waiting for public URL".to_string(),
                None => "not started".to_string(),
            });
        });
        ui.add_space(6.0);
        if busy || matches!(self.launch_phase, LaunchPhase::Done) {
            ui.add(egui::ProgressBar::new(self.launch_phase.progress()).desired_height(8.0).fill(crate::theme::ACCENT));
        }
        ui.add_space(6.0);
        let c = match &self.launch_phase {
            LaunchPhase::Done => crate::theme::OK,
            LaunchPhase::Failed(_) => crate::theme::ERR,
            LaunchPhase::Idle => crate::theme::TEXT_WEAK,
            _ => crate::theme::WARN,
        };
        ui.colored_label(c, self.launch_phase.label());
    }

    fn endpoint_card(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Custom Endpoint").size(12.0).strong().color(crate::theme::TEXT_WEAK));
        ui.add_space(6.0);
        if self.public_url.is_none() {
            ui.colored_label(crate::theme::WARN, "Warp rejects local/HTTP URLs. Start the gateway flow to mint an HTTPS endpoint.");
        }
        let url = self.endpoint_url();
        ui.add_space(4.0);
        ui.label(egui::RichText::new(&url).monospace());
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(subtle_button("Copy config JSON")).clicked() {
                self.copy_endpoint_config();
            }
            if ui.add(subtle_button("Copy URL")).clicked() {
                self.copy_endpoint_url();
            }
        });
    }

    fn proxy_card(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("Proxy Settings")
                .size(12.0)
                .strong()
                .color(crate::theme::TEXT_WEAK),
        );
        ui.add_space(6.0);

        egui::Grid::new("proxy").num_columns(2).spacing([10.0, 7.0]).show(ui, |ui| {
            ui.label("Channel");
            egui::ComboBox::from_id_salt("proxy_channel")
                .selected_text(match self.proxy.channel {
                    WarpChannel::Production => "production",
                    WarpChannel::Staging => "staging",
                    WarpChannel::Dev => "dev",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.proxy.channel,
                        WarpChannel::Production,
                        "production",
                    );
                    ui.selectable_value(
                        &mut self.proxy.channel,
                        WarpChannel::Staging,
                        "staging",
                    );
                    ui.selectable_value(&mut self.proxy.channel, WarpChannel::Dev, "dev");
                });
            ui.end_row();

            ui.label("Host");
            ui.add_enabled(
                !self.proxy_controller.is_running(),
                egui::TextEdit::singleline(&mut self.proxy.host),
            );
            ui.end_row();

            ui.label("Port");
            ui.add_enabled(
                !self.proxy_controller.is_running(),
                egui::DragValue::new(&mut self.proxy.port).range(1..=65535),
            );
            ui.end_row();

            ui.label("OZ token");
            ui.add(egui::TextEdit::singleline(&mut self.proxy.oz_token).password(true));
            ui.end_row();

            ui.label("Auth");
            ui.label(
                egui::RichText::new(self.proxy.auth_label()).color(crate::theme::TEXT_WEAK),
            );
            ui.end_row();
        });

        ui.add_space(8.0);
        ui.label(egui::RichText::new(self.proxy_http_root()).monospace());
        ui.label(egui::RichText::new(self.proxy_ws_root()).monospace().color(crate::theme::TEXT_WEAK));
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(if self.proxy_controller.is_running() {
                "Workflow holds the primary proxy actions."
            } else {
                "Adjust settings here, then use Workflow to start the proxy."
            })
            .color(crate::theme::TEXT_WEAK),
        );

        if let Some(support) = self.warp_override_support() {
            match support {
                ServerUrlOverrideSupport::Supported => {
                    ui.colored_label(crate::theme::OK, "Detected override-capable Warp build.");
                }
                ServerUrlOverrideSupport::Unsupported => {
                    ui.colored_label(
                        crate::theme::WARN,
                        "Installed Stable/Preview/Oss Warp ignores proxy override env vars.",
                    );
                }
                ServerUrlOverrideSupport::Unknown => {
                    ui.colored_label(
                        crate::theme::TEXT_WEAK,
                        "Warp channel could not be classified; proxy launch depends on override support.",
                    );
                }
            }
        }

        if let ProxyStatus::Error { message } = self.proxy_controller.status() {
            ui.colored_label(crate::theme::ERR, message);
        }
    }

    fn advanced_card(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(egui::RichText::new("Diagnostics").color(crate::theme::TEXT_WEAK)).default_open(false).show(ui, |ui| {
            let running = self.controller.is_running();
            egui::Grid::new("adv").num_columns(2).spacing([10.0,7.0]).show(ui, |ui| {
                ui.label("Port");
                ui.add_enabled(!running, egui::DragValue::new(&mut self.gateway.port).range(1..=65535));
                ui.end_row();
                ui.label("Adapter");
                egui::ComboBox::from_id_salt("ad").selected_text(self.form.adapter.as_str()).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.form.adapter, Adapter::OpenaiChat, "openai_chat");
                    ui.selectable_value(&mut self.form.adapter, Adapter::OpenaiResponses, "openai_responses");
                    ui.selectable_value(&mut self.form.adapter, Adapter::AnthropicMessages, "anthropic_messages");
                    ui.selectable_value(&mut self.form.adapter, Adapter::GeminiGenerateContent, "gemini_generate_content");
                });
                ui.end_row();
                ui.label("Wire API");
                egui::ComboBox::from_id_salt("wa").selected_text(self.form.wire_api.as_str()).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.form.wire_api, WireApi::Chat, "chat");
                    ui.selectable_value(&mut self.form.wire_api, WireApi::Responses, "responses");
                });
                ui.end_row();
                ui.label("Dup window (s)");
                ui.add(egui::DragValue::new(&mut self.gateway.duplicate_window_secs).range(0..=3600));
                ui.end_row();
                ui.label("force_model_key");
                ui.text_edit_singleline(&mut self.gateway.force_model_config_key);
                ui.end_row();
            });
            ui.checkbox(&mut self.gateway.disable_tools, "disable_tools (safe mode)");
            ui.checkbox(&mut self.gateway.disable_mcp, "disable_mcp");
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if running {
                    if ui.add(danger_button("Stop gateway")).clicked() {
                        self.controller.stop();
                        self.launch_started_gateway = false;
                    }
                } else if ui.add(secondary_button("Start gateway")).clicked() {
                    let c = self.build_config();
                    self.controller.start(&self.gateway.host, self.gateway.port, c);
                }
                if self.tunnel.is_some() {
                    if ui.add(danger_button("Stop tunnel")).clicked() {
                        self.tunnel = None;
                        self.public_url = None;
                        self.waiting_for_public_url_since = None;
                        self.launch_started_tunnel = false;
                    }
                } else if ui
                    .add_enabled(self.cloudflared_path.is_some(), secondary_button("Start tunnel"))
                    .clicked()
                {
                    let _ = self.try_start_tunnel(false);
                }
                if ui.add(subtle_button("Re-detect")).clicked() {
                    self.warp_path = crate::warp_setup::detect_warp();
                    self.cloudflared_path = crate::warp_setup::detect_cloudflared();
                    self.last_message = "Binary detection refreshed.".into();
                }
            });

            ui.add_space(4.0);
            egui::CollapsingHeader::new(egui::RichText::new("Logs").color(crate::theme::TEXT_WEAK)).default_open(false).show(ui, |ui| {
                if ui.add(subtle_button("Clear")).clicked() { self.log_buffer.clear(); }
                let lines = self.log_buffer.snapshot();
                egui::ScrollArea::vertical().id_salt("logs").max_height(150.0).stick_to_bottom(true).show(ui, |ui| {
                    if lines.is_empty() { ui.weak("(no logs yet)"); }
                    else { for l in &lines { ui.monospace(l); } }
                });
            });
        });
        let _ = (&mut self.show_advanced, &mut self.show_logs);
    }
}

fn status_chip(ui: &mut egui::Ui, label: &str, on: bool, on_color: egui::Color32) {
    let fill = if on {
        on_color.linear_multiply(0.18)
    } else {
        crate::theme::SURFACE_ALT
    };
    let stroke = if on { on_color } else { crate::theme::BORDER };
    egui::Frame::default()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::symmetric(8.0, 4.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let dot = if on { on_color } else { egui::Color32::from_gray(96) };
                ui.label(egui::RichText::new("●").color(dot).size(10.5));
                ui.label(
                    egui::RichText::new(label)
                        .size(11.5)
                        .color(crate::theme::TEXT_WEAK),
                );
            });
        });
}

fn secondary_button(label: &'static str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(label)
            .size(12.5)
            .color(crate::theme::TEXT),
    )
    .fill(crate::theme::SURFACE_ALT)
    .stroke(egui::Stroke::new(1.0, crate::theme::BORDER))
    .min_size(egui::vec2(0.0, 32.0))
}

fn subtle_button(label: &'static str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(label)
            .size(12.5)
            .color(crate::theme::TEXT_WEAK),
    )
    .fill(egui::Color32::TRANSPARENT)
    .stroke(egui::Stroke::new(1.0, crate::theme::BORDER))
    .min_size(egui::vec2(0.0, 32.0))
}

fn danger_button(label: &'static str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(label)
            .size(12.5)
            .color(crate::theme::ERR),
    )
    .fill(crate::theme::ERR.linear_multiply(0.08))
    .stroke(egui::Stroke::new(1.0, crate::theme::ERR.linear_multiply(0.8)))
    .min_size(egui::vec2(0.0, 32.0))
}

fn workflow_steps(ui: &mut egui::Ui, steps: &[(&str, &str, bool)]) {
    if ui.available_width() >= 720.0 {
        ui.columns(steps.len(), |columns| {
            for (column, (index, label, done)) in columns.into_iter().zip(steps.iter().copied()) {
                workflow_step(column, index, label, done);
            }
        });
    } else {
        for (index, label, done) in steps.iter().copied() {
            workflow_step(ui, index, label, done);
            ui.add_space(6.0);
        }
    }
}

fn workflow_step(ui: &mut egui::Ui, index: &str, label: &str, done: bool) {
    let accent = if done {
        crate::theme::OK
    } else {
        crate::theme::BORDER
    };
    egui::Frame::default()
        .fill(if done {
            crate::theme::OK.linear_multiply(0.1)
        } else {
            crate::theme::SURFACE_ALT
        })
        .stroke(egui::Stroke::new(1.0, accent))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_min_height(72.0);
            ui.horizontal(|ui| {
                egui::Frame::default()
                    .fill(if done {
                        crate::theme::OK.linear_multiply(0.2)
                    } else {
                        crate::theme::SURFACE
                    })
                    .stroke(egui::Stroke::new(1.0, accent))
                    .rounding(egui::Rounding::same(8.0))
                    .inner_margin(egui::Margin::symmetric(9.0, 6.0))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(index)
                                .strong()
                                .color(crate::theme::TEXT),
                        );
                    });
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(label).color(crate::theme::TEXT));
                    ui.label(
                        egui::RichText::new(if done { "Ready" } else { "Pending" })
                            .size(11.5)
                            .color(if done {
                                crate::theme::OK
                            } else {
                                crate::theme::TEXT_WEAK
                            }),
                    );
                });
            });
        });
}

fn needs_hint(url: &str) -> bool {
    let t = url.trim().trim_end_matches('/');
    if t.is_empty() { return false; }
    let after = t.split_once("://").map(|(_, r)| r).unwrap_or(t);
    !after.contains('/')
}

fn copy(text: &str) {
    match arboard::Clipboard::new() {
        Ok(mut c) => { let _ = c.set_text(text.to_string()); }
        Err(e) => tracing::warn!(error = %e, "clipboard unavailable"),
    }
}
