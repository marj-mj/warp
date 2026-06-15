//! Background event pump.
//!
//! Tauri commands trigger work; this poller bridges the synchronous cloudflared
//! tunnel (which exposes a blocking `try_url` / `try_exit_status`) into async
//! Tauri events. It runs every 500ms while the app is alive.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};

use crate::state::{AppState, LaunchPhase};

/// Emit the current launch phase + progress to the frontend.
pub fn emit_phase(app: &AppHandle, phase: &LaunchPhase) {
    let payload = serde_json::json!({
        "phase": phase,
        "progress": phase.progress(),
    });
    let _ = app.emit("launch://phase", payload);
}

/// Spawn the background poller. Safe to call once during setup.
pub fn spawn_launch_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<Arc<AppState>>().inner().clone();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            poll_once(&app, &state).await;
        }
    });
}

async fn poll_once(app: &AppHandle, state: &Arc<AppState>) {
    let mut launch = state.launch.lock().await;
    if launch.public_url.is_some() || launch.tunnel.is_none() {
        // Nothing to poll, but still surface a tunnel exit if it died.
        if let Some(tunnel) = launch.tunnel.as_mut() {
            if let Ok(Some(status)) = tunnel.try_exit_status() {
                let detail = status
                    .code()
                    .map(|code| format!("exit code {code}"))
                    .unwrap_or_else(|| "terminated by signal".to_string());
                let message = format!("cloudflared exited before publishing a URL ({detail})");
                launch.tunnel = None;
                launch.public_url = None;
                if matches!(
                    launch.phase,
                    LaunchPhase::StartingTunnel | LaunchPhase::WaitingPublicUrl
                ) {
                    launch.phase = LaunchPhase::Failed(message.clone());
                    emit_phase(app, &launch.phase);
                }
                let _ = app.emit("tunnel://error", message);
            }
        }
        return;
    }

    if let Some(tunnel) = launch.tunnel.as_ref() {
        if let Some(url) = tunnel.try_url() {
            launch.public_url = Some(url.clone());
            let _ = app.emit("tunnel://public-url", &url);
        }
    }
}
