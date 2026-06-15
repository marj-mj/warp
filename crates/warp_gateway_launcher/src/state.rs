//! Shared application state for the Tauri backend.
//!
//! Holds the gateway + proxy controllers, the provider store, detected tool
//! paths, and the in-flight launch state. All mutable state lives behind async
//! Mutexes so Tauri command handlers can access it concurrently.

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::Mutex;

use crate::controller::GatewayController;
use crate::log_buffer::LogBuffer;
use crate::proxy_controller::ProxyController;
use crate::warp_setup::CloudflaredTunnel;

/// Phase of the guided "launch everything" flow. Mirrors the old egui state
/// machine but is now serialized to the frontend over the `launch://phase`
/// event channel.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum LaunchPhase {
    Idle,
    StartingGateway,
    StartingTunnel,
    WaitingPublicUrl,
    SpawningWarp,
    Done,
    PendingWarpRestart,
    Failed(String),
}

impl LaunchPhase {
    pub fn progress(&self) -> f32 {
        match self {
            Self::Idle => 0.0,
            Self::StartingGateway => 0.2,
            Self::StartingTunnel => 0.45,
            Self::WaitingPublicUrl => 0.7,
            Self::SpawningWarp => 0.9,
            Self::Done => 1.0,
            Self::PendingWarpRestart => 1.0,
            Self::Failed(_) => 0.0,
        }
    }
}

/// Detected external tooling, refreshed on demand.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ToolPaths {
    pub warp: Option<String>,
    pub cloudflared: Option<String>,
}

#[derive(Default)]
pub struct LaunchState {
    pub phase: LaunchPhase,
    pub public_url: Option<String>,
    pub tunnel: Option<CloudflaredTunnel>,
    pub started_gateway: bool,
    pub started_tunnel: bool,
    pub cancel_requested: bool,
}

impl Default for LaunchPhase {
    fn default() -> Self {
        Self::Idle
    }
}

/// Top-level shared state, stored in Tauri's managed state.
pub struct AppState {
    pub gateway: Mutex<GatewayController>,
    pub proxy: Mutex<ProxyController>,
    pub launch: Mutex<LaunchState>,
    pub tools: Mutex<ToolPaths>,
    pub log_buffer: LogBuffer,
}

impl AppState {
    pub fn new(log_buffer: LogBuffer) -> Arc<Self> {
        let tools = ToolPaths {
            warp: crate::warp_setup::detect_warp().map(|p| p.display().to_string()),
            cloudflared: crate::warp_setup::detect_cloudflared().map(|p| p.display().to_string()),
        };
        Arc::new(Self {
            gateway: Mutex::new(GatewayController::new()),
            proxy: Mutex::new(ProxyController::new()),
            launch: Mutex::new(LaunchState::default()),
            tools: Mutex::new(tools),
            log_buffer,
        })
    }
}

