//! egui application: tabbed control panel for the Managed Provider Gateway.
//!
//! Phase L-A added the window shell + in-process gateway controller.
//! Phase L-B adds: persistent provider list (`%APPDATA%/WarpGatewayLauncher`),
//! a CRUD form, a Probe button (capability matrix + recommendation), and an
//! "active provider" selection used to start the gateway.

use eframe::egui;
use warp_gateway_wrapper::mpg::probe::CapabilityMatrix;
use warp_gateway_wrapper::mpg::{Adapter, GatewayConfig, WireApi};

use crate::controller::{GatewayController, GatewayStatus};
use crate::log_buffer::LogBuffer;
use crate::probe::PendingProbe;
use crate::store::{ProviderStore, StoredProvider};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Provider,
    Gateway,
    WarpSetup,
}

/// Editable provider fields. Mirrors `StoredProvider` plus an in-memory api key
/// that is never written to disk.
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
    fn from_stored(stored: &StoredProvider) -> Self {
        Self {
            name: stored.name.clone(),
            base_url: stored.base_url.clone(),
            model: stored.model.clone().unwrap_or_default(),
            api_key: String::new(),
            env_key: stored.env_key.clone().unwrap_or_default(),
            wire_api: stored.wire_api,
            adapter: stored.adapter,
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

    /// Resolve the runtime API key, preferring the in-memory field, falling
    /// back to the env_key reference.
    fn resolved_api_key(&self) -> Option<String> {
        if !self.api_key.trim().is_empty() {
            return Some(self.api_key.clone());
        }
        let env_name = self.env_key.trim();
        if env_name.is_empty() {
            return None;
        }
        std::env::var(env_name).ok().filter(|value| !value.trim().is_empty())
    }
}

fn nullable(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Gateway-side knobs (host/port + safe mode flags).
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
            host: "127.0.0.1".to_string(),
            port: 8787,
            disable_tools: true,
            disable_mcp: true,
            duplicate_window_secs: 60,
            force_model_config_key: String::new(),
        }
    }
}

pub struct LauncherApp {
    tab: Tab,
    controller: GatewayController,

    store: ProviderStore,
    selected: Option<usize>,
    form: ProviderForm,
    gateway: GatewayForm,

    pending_probe: Option<PendingProbe>,
    last_matrix: Option<CapabilityMatrix>,
    last_message: String,
    log_buffer: LogBuffer,
    warp_path: Option<std::path::PathBuf>,
    cloudflared_path: Option<std::path::PathBuf>,
    tunnel: Option<crate::warp_setup::CloudflaredTunnel>,
    public_url: Option<String>,
}

impl LauncherApp {
    pub fn new(controller: GatewayController, log_buffer: LogBuffer) -> Self {
        let store = ProviderStore::load().unwrap_or_default();
        let selected = if store.providers.is_empty() { None } else { Some(0) };
        let form = selected
            .and_then(|i| store.providers.get(i))
            .map(ProviderForm::from_stored)
            .unwrap_or_default();
        Self {
            tab: Tab::Provider,
            controller,
            store,
            selected,
            form,
            gateway: GatewayForm::default(),
            pending_probe: None,
            last_matrix: None,
            last_message: String::new(),
            warp_path: crate::warp_setup::detect_warp(),
            cloudflared_path: crate::warp_setup::detect_cloudflared(),
            tunnel: None,
            public_url: None,
            log_buffer,
        }
    }

    /// Build the runtime gateway config from the active form + gateway flags.
    fn build_gateway_config(&self) -> GatewayConfig {
        let stored = self.form.to_stored();
        let provider = stored.to_config(self.form.resolved_api_key());
        let mut config = GatewayConfig::new(provider);
        config.disable_tools = self.gateway.disable_tools;
        config.disable_mcp = self.gateway.disable_mcp;
        config.duplicate_window_secs = self.gateway.duplicate_window_secs;
        let force = self.gateway.force_model_config_key.trim();
        config.force_model_config_key = if force.is_empty() { None } else { Some(force.to_string()) };
        config
    }

    fn provider_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Provider");

        ui.horizontal(|ui| {
            // Provider list on the left
            ui.vertical(|ui| {
                ui.set_min_width(180.0);
                ui.label("Saved providers");
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        let mut clicked: Option<usize> = None;
                        for (idx, provider) in self.store.providers.iter().enumerate() {
                            let label = if provider.name.is_empty() {
                                format!("(unnamed #{idx})")
                            } else {
                                provider.name.clone()
                            };
                            if ui
                                .selectable_label(self.selected == Some(idx), label)
                                .clicked()
                            {
                                clicked = Some(idx);
                            }
                        }
                        if let Some(idx) = clicked {
                            self.selected = Some(idx);
                            self.form = ProviderForm::from_stored(&self.store.providers[idx]);
                            self.last_matrix = None;
                        }
                    });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("+ New").clicked() {
                        self.form = ProviderForm::default();
                        self.selected = None;
                        self.last_matrix = None;
                    }
                    let can_delete = self.selected.is_some();
                    if ui
                        .add_enabled(can_delete, egui::Button::new("Delete"))
                        .clicked()
                    {
                        if let Some(idx) = self.selected {
                            self.store.remove(idx);
                            self.selected = if self.store.providers.is_empty() {
                                None
                            } else {
                                Some(idx.min(self.store.providers.len() - 1))
                            };
                            if let Some(i) = self.selected {
                                self.form = ProviderForm::from_stored(&self.store.providers[i]);
                            } else {
                                self.form = ProviderForm::default();
                            }
                            self.persist_store();
                        }
                    }
                });
            });

            ui.separator();

            // Form on the right
            ui.vertical(|ui| self.provider_form_ui(ui));
        });
    }

    fn provider_form_ui(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("provider_form")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.form.name);
                ui.end_row();

                ui.label("Base URL");
                ui.vertical(|ui| {
                    ui.text_edit_singleline(&mut self.form.base_url);
                    if needs_path_hint(&self.form.base_url) {
                        ui.colored_label(
                            egui::Color32::from_rgb(0xd8, 0x9b, 0x2b),
                            "⚠ Base URL thường cần path, vd kết thúc bằng /v1 (gateway tự thêm /chat/completions).",
                        );
                    }
                });
                ui.end_row();

                ui.label("Model");
                ui.text_edit_singleline(&mut self.form.model);
                ui.end_row();

                ui.label("API key (memory)");
                ui.add(egui::TextEdit::singleline(&mut self.form.api_key).password(true));
                ui.end_row();

                ui.label("Env key");
                ui.text_edit_singleline(&mut self.form.env_key);
                ui.end_row();

                ui.label("Wire API");
                egui::ComboBox::from_id_salt("wire_api")
                    .selected_text(self.form.wire_api.as_str())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.form.wire_api, WireApi::Chat, "chat");
                        ui.selectable_value(&mut self.form.wire_api, WireApi::Responses, "responses");
                    });
                ui.end_row();

                ui.label("Adapter");
                egui::ComboBox::from_id_salt("adapter")
                    .selected_text(self.form.adapter.as_str())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.form.adapter, Adapter::OpenaiChat, "openai_chat");
                        ui.selectable_value(&mut self.form.adapter, Adapter::OpenaiResponses, "openai_responses");
                        ui.selectable_value(&mut self.form.adapter, Adapter::AnthropicMessages, "anthropic_messages");
                        ui.selectable_value(&mut self.form.adapter, Adapter::GeminiGenerateContent, "gemini_generate_content");
                    });
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let can_save = self.form.is_valid();
            if ui
                .add_enabled(can_save, egui::Button::new("Save"))
                .clicked()
            {
                self.store.upsert(self.form.to_stored());
                // Re-sync selection to point at the just-saved entry.
                self.selected = self
                    .store
                    .providers
                    .iter()
                    .position(|p| p.name.eq_ignore_ascii_case(&self.form.name));
                self.persist_store();
                self.last_message = "Saved.".to_string();
            }

            let probe_enabled = self.pending_probe.is_none() && !self.form.base_url.trim().is_empty();
            if ui
                .add_enabled(probe_enabled, egui::Button::new("Probe"))
                .clicked()
            {
                self.start_probe();
            }
            if self.pending_probe.is_some() {
                ui.spinner();
                ui.label("probing…");
            }
        });

        ui.add_space(8.0);
        if !self.last_message.is_empty() {
            ui.label(&self.last_message);
        }

        if let Some(matrix) = &self.last_matrix {
            ui.add_space(8.0);
            ui.separator();
            ui.label("Capability matrix:");
            let cap = |outcome: warp_gateway_wrapper::mpg::probe::ProbeOutcome| outcome.as_str();
            ui.label(format!(
                "  chat: non_stream={} streaming={} tools={}",
                cap(matrix.chat_non_stream),
                cap(matrix.chat_streaming),
                cap(matrix.chat_tools)
            ));
            ui.label(format!(
                "  responses: non_stream={} streaming={} tools={}",
                cap(matrix.responses_non_stream),
                cap(matrix.responses_streaming),
                cap(matrix.responses_tools)
            ));
            let rec = matrix.recommend();
            ui.label(format!(
                "Recommendation: adapter={} wire_api={} confidence={}",
                rec.adapter, rec.wire_api, rec.confidence
            ));
            if ui.button("Apply recommendation").clicked() {
                self.form.adapter = match rec.adapter {
                    "openai_chat" => Adapter::OpenaiChat,
                    "openai_responses" => Adapter::OpenaiResponses,
                    "anthropic_messages" => Adapter::AnthropicMessages,
                    "gemini_generate_content" => Adapter::GeminiGenerateContent,
                    _ => self.form.adapter,
                };
                self.form.wire_api = match rec.wire_api {
                    "responses" => WireApi::Responses,
                    _ => WireApi::Chat,
                };
                self.gateway.disable_tools = rec.safe_mode_defaults.disable_tools;
                self.gateway.disable_mcp = rec.safe_mode_defaults.disable_mcp;
                self.last_message = "Applied recommendation.".to_string();
            }
        }
    }

    fn start_probe(&mut self) {
        let handle = self.controller.runtime_handle();
        let probe = PendingProbe::spawn(
            &handle,
            self.form.base_url.trim().to_string(),
            self.form.resolved_api_key(),
        );
        self.pending_probe = Some(probe);
        self.last_message = "Probing upstream…".to_string();
    }

    fn poll_probe(&mut self) {
        if let Some(probe) = self.pending_probe.as_mut() {
            if let Some(matrix) = probe.poll() {
                self.last_matrix = Some(matrix);
                self.pending_probe = None;
                self.last_message = "Probe finished.".to_string();
            }
        }
    }

    fn poll_tunnel(&mut self) {
        if self.public_url.is_none() {
            if let Some(tunnel) = &self.tunnel {
                if let Some(url) = tunnel.try_url() {
                    self.public_url = Some(url.clone());
                    self.last_message = format!("Public URL sẵn sàng: {url}");
                }
            }
        }
    }

    fn persist_store(&mut self) {
        if let Err(err) = self.store.save() {
            self.last_message = format!("Save failed: {err}");
        }
    }

    fn gateway_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Gateway");
        let status = self.controller.status();
        ui.horizontal(|ui| {
            ui.label("Status:");
            let color = if status.is_running() {
                egui::Color32::from_rgb(0x3c, 0xb3, 0x71)
            } else {
                egui::Color32::GRAY
            };
            ui.colored_label(color, status.label());
        });
        ui.add_space(8.0);

        let running = self.controller.is_running();

        egui::Grid::new("gateway_form")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Host");
                ui.add_enabled(!running, egui::TextEdit::singleline(&mut self.gateway.host));
                ui.end_row();

                ui.label("Port");
                ui.add_enabled(
                    !running,
                    egui::DragValue::new(&mut self.gateway.port).range(1..=65535),
                );
                ui.end_row();
            });

        ui.add_space(4.0);
        ui.checkbox(&mut self.gateway.disable_tools, "disable_tools (safe mode)");
        ui.checkbox(&mut self.gateway.disable_mcp, "disable_mcp");
        ui.horizontal(|ui| {
            ui.label("duplicate_window_secs");
            ui.add(egui::DragValue::new(&mut self.gateway.duplicate_window_secs).range(0..=3600));
        });
        ui.horizontal(|ui| {
            ui.label("force_model_config_key");
            ui.text_edit_singleline(&mut self.gateway.force_model_config_key);
        });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if !running {
                let can_start = self.form.is_valid();
                if ui
                    .add_enabled(can_start, egui::Button::new("Start gateway"))
                    .clicked()
                {
                    let config = self.build_gateway_config();
                    self.controller
                        .start(&self.gateway.host, self.gateway.port, config);
                    self.last_message = "Started gateway.".to_string();
                }
                if !can_start {
                    ui.label("Configure a provider first.");
                }
            } else if ui.button("Stop gateway").clicked() {
                self.controller.stop();
                self.last_message = "Stopped gateway.".to_string();
            }
        });

        if let GatewayStatus::Running { addr } = &status {
            ui.add_space(8.0);
            let endpoint = format!("http://{addr}/v1");
            let health = format!("http://{addr}/healthz");
            ui.horizontal(|ui| {
                ui.label("Endpoint for Warp:");
                ui.monospace(&endpoint);
                if ui.small_button("Copy").clicked() {
                    copy_to_clipboard(&endpoint);
                    self.last_message = "Endpoint URL copied to clipboard.".to_string();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Health:");
                ui.monospace(&health);
                if ui.small_button("Copy").clicked() {
                    copy_to_clipboard(&health);
                    self.last_message = "Health URL copied to clipboard.".to_string();
                }
            });
        }

        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.heading("Logs");
            if ui.small_button("Clear").clicked() {
                self.log_buffer.clear();
            }
        });
        let lines = self.log_buffer.snapshot();
        egui::ScrollArea::vertical()
            .max_height(180.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if lines.is_empty() {
                    ui.label("(no log lines yet)");
                } else {
                    for line in &lines {
                        ui.monospace(line);
                    }
                }
            });
    }

    fn warp_setup_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Warp setup");
        ui.label("Trỏ Warp vào gateway mà không sửa source Warp.");
        ui.add_space(8.0);

        // --- Warp detection / install ---
        ui.group(|ui| {
            ui.label(egui::RichText::new("Warp install").strong());
            match &self.warp_path {
                Some(path) => {
                    ui.horizontal(|ui| {
                        ui.label("Detected:");
                        ui.monospace(path.display().to_string());
                    });
                }
                None => {
                    ui.label("Warp not detected in standard locations.");
                }
            }
            ui.horizontal(|ui| {
                if ui.button("Re-detect").clicked() {
                    self.warp_path = crate::warp_setup::detect_warp();
                    self.last_message = "Re-scanned for Warp.".to_string();
                }
                if ui.button("Install via winget").clicked() {
                    match crate::warp_setup::install_warp_via_winget() {
                        Ok(()) => self.last_message = "Started winget install for Warp.".to_string(),
                        Err(err) => self.last_message = err,
                    }
                }
            });
        });

        ui.add_space(8.0);

        // --- Copy custom-endpoint config ---
        ui.group(|ui| {
            ui.label(egui::RichText::new("Custom endpoint config").strong());
            // Warp rejects http/local URLs; a public HTTPS tunnel URL is required.
            let endpoint_url = match &self.public_url {
                Some(public) => format!("{public}/v1"),
                None => match self.controller.status() {
                    GatewayStatus::Running { addr } => format!("http://{addr}/v1"),
                    _ => format!("http://{}:{}/v1", self.gateway.host, self.gateway.port),
                },
            };
            if self.public_url.is_none() {
                ui.colored_label(
                    egui::Color32::from_rgb(0xd8, 0x9b, 0x2b),
                    "⚠ Warp từ chối URL local/HTTP. Hãy Start tunnel (Cloudflared) bên dưới để có URL HTTPS công khai trước khi paste.",
                );
            }
            let model = self
                .form
                .model
                .trim()
                .to_string()
                .if_empty("default-model");
            let json = crate::warp_setup::endpoint_config_json(
                self.form.name.trim(),
                &endpoint_url,
                &model,
            );
            ui.label("Paste the following into Warp Settings -> Custom endpoints:");
            egui::ScrollArea::vertical()
                .max_height(140.0)
                .show(ui, |ui| {
                    ui.monospace(&json);
                });
            ui.horizontal(|ui| {
                if ui.button("Copy JSON").clicked() {
                    copy_to_clipboard(&json);
                    self.last_message = "Endpoint config copied.".to_string();
                }
                if ui.button("Copy URL").clicked() {
                    copy_to_clipboard(&endpoint_url);
                    self.last_message = "Endpoint URL copied.".to_string();
                }
            });
            ui.label("Tip: name starts with @gateway so Warp marks it as a managed provider source.");
        });

        ui.add_space(8.0);

        // --- Spawn Warp pointed at the proxy mode (only useful for transparent proxy) ---
        ui.group(|ui| {
            ui.label(egui::RichText::new("Launch Warp pointed at proxy (optional)").strong());
            ui.label(
                "For the transparent-proxy mode (proxy subcommand) Warp can be \
                 launched with WARP_SERVER_ROOT_URL set. For the Managed Provider \
                 Gateway flow, paste the JSON above instead.",
            );
            let can_spawn = self.warp_path.is_some();
            if ui
                .add_enabled(can_spawn, egui::Button::new("Spawn Warp with proxy URL"))
                .clicked()
            {
                if let Some(path) = self.warp_path.clone() {
                    let url = match self.controller.status() {
                        GatewayStatus::Running { addr } => format!("http://{addr}"),
                        _ => format!("http://{}:{}", self.gateway.host, self.gateway.port),
                    };
                    match crate::warp_setup::spawn_warp_with_server_url(&path, &url) {
                        Ok(()) => self.last_message = format!("Spawned Warp with WARP_SERVER_ROOT_URL={url}"),
                        Err(err) => self.last_message = err,
                    }
                }
            }
        });

        ui.add_space(8.0);

        // --- Cloudflared tunnel ---
        ui.group(|ui| {
            ui.label(egui::RichText::new("Cloudflared tunnel (optional)").strong());
            match &self.cloudflared_path {
                Some(path) => {
                    ui.horizontal(|ui| {
                        ui.label("Detected:");
                        ui.monospace(path.display().to_string());
                    });
                }
                None => {
                    ui.label("cloudflared not detected.");
                }
            }
            ui.horizontal(|ui| {
                if ui.button("Re-detect").clicked() {
                    self.cloudflared_path = crate::warp_setup::detect_cloudflared();
                    self.last_message = "Re-scanned for cloudflared.".to_string();
                }
                if ui.button("Install via winget").clicked() {
                    match crate::warp_setup::install_cloudflared_via_winget() {
                        Ok(()) => self.last_message = "Started winget install for cloudflared.".to_string(),
                        Err(err) => self.last_message = err,
                    }
                }
                let can_run = self.cloudflared_path.is_some() && self.tunnel.is_none();
                if ui
                    .add_enabled(can_run, egui::Button::new("Start tunnel"))
                    .clicked()
                {
                    if let Some(path) = self.cloudflared_path.clone() {
                        match crate::warp_setup::start_cloudflared_tunnel(&path, self.gateway.port) {
                            Ok(tunnel) => {
                                self.tunnel = Some(tunnel);
                                self.public_url = None;
                                self.last_message = "Tunnel đang khởi động; chờ public URL…".to_string();
                            }
                            Err(err) => self.last_message = err,
                        }
                    }
                }
                if self.tunnel.is_some() && ui.button("Stop tunnel").clicked() {
                    self.tunnel = None;
                    self.public_url = None;
                    self.last_message = "Đã dừng tunnel.".to_string();
                }
            });

            match &self.public_url {
                Some(url) => {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Public URL:");
                        ui.monospace(url);
                        if ui.small_button("Copy /v1").clicked() {
                            copy_to_clipboard(&format!("{url}/v1"));
                            self.last_message = "Public endpoint URL copied.".to_string();
                        }
                    });
                }
                None if self.tunnel.is_some() => {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Đang chờ cloudflared cấp public URL…");
                    });
                }
                None => {}
            }
        });
    }
}

impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Pull probe results before drawing so the UI reflects them this frame.
        self.poll_probe();
        self.poll_tunnel();

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Provider, "Provider");
                ui.selectable_value(&mut self.tab, Tab::Gateway, "Gateway");
                ui.selectable_value(&mut self.tab, Tab::WarpSetup, "Warp setup");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Provider => self.provider_tab(ui),
            Tab::Gateway => self.gateway_tab(ui),
            Tab::WarpSetup => self.warp_setup_tab(ui),
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}


/// Best-effort clipboard write; logs and ignores errors so a missing
/// clipboard backend can never crash the UI.
fn copy_to_clipboard(text: &str) {
    match arboard::Clipboard::new() {
        Ok(mut clipboard) => {
            if let Err(err) = clipboard.set_text(text.to_string()) {
                tracing::warn!(error = %err, "clipboard write failed");
            }
        }
        Err(err) => tracing::warn!(error = %err, "clipboard unavailable"),
    }
}

/// Tiny helper: replace an empty string with a default.
trait IfEmpty {
    fn if_empty(self, default: &str) -> String;
}
impl IfEmpty for String {
    fn if_empty(self, default: &str) -> String {
        if self.is_empty() { default.to_string() } else { self }
    }
}

/// Heuristic: warn when a base URL has no path segment (likely missing /v1).
fn needs_path_hint(base_url: &str) -> bool {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return false;
    }
    // Strip scheme, then check if anything path-like remains after the host.
    let after_scheme = trimmed.split_once("://").map(|(_, rest)| rest).unwrap_or(trimmed);
    !after_scheme.contains('/')
}