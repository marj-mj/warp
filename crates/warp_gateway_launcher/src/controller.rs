//! Gateway controller: async start/stop on top of Tauri's tokio runtime.
//!
//! Emits `gateway://status` events whenever the status changes so the React UI
//! can react without polling. Generic over the Tauri runtime so it can be
//! exercised with `tauri::test::MockRuntime` in tests.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use warp_gateway_wrapper::mpg::{GatewayConfig, MpgServer};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GatewayStatus {
    Stopped,
    Running { addr: String },
    Error { message: String },
}

impl GatewayStatus {
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

struct RunningInstance {
    shutdown: oneshot::Sender<()>,
    join: JoinHandle<()>,
}

pub struct GatewayController {
    status: Arc<Mutex<GatewayStatus>>,
    instance: Mutex<Option<RunningInstance>>,
}

impl GatewayController {
    pub fn new() -> Self {
        Self {
            status: Arc::new(Mutex::new(GatewayStatus::Stopped)),
            instance: Mutex::new(None),
        }
    }

    pub fn status(&self) -> GatewayStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn is_running(&self) -> bool {
        self.instance.lock().unwrap().is_some()
    }

    fn set_status<R: Runtime>(&self, app: &AppHandle<R>, new_status: GatewayStatus) {
        *self.status.lock().unwrap() = new_status.clone();
        let _ = app.emit("gateway://status", &new_status);
    }

    /// Bind the gateway and run it on the Tauri tokio runtime.
    pub async fn start<R: Runtime>(
        &self,
        host: &str,
        port: u16,
        config: GatewayConfig,
        app: &AppHandle<R>,
    ) -> Result<GatewayStatus, String> {
        if self.instance.lock().unwrap().is_some() {
            return Ok(self.status());
        }

        let server = MpgServer::new(host, port, config)
            .map_err(|err| format!("invalid bind address: {err}"))?;
        let requested_addr = server.addr();
        let listener = match tokio::net::TcpListener::bind(requested_addr).await {
            Ok(listener) => listener,
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {
                let fallback_addr = std::net::SocketAddr::new(requested_addr.ip(), 0);
                let listener =
                    tokio::net::TcpListener::bind(fallback_addr)
                        .await
                        .map_err(|fallback_err| {
                            format!(
                                "port {port} is in use and no fallback port could be allocated: \
                             {fallback_err}"
                            )
                        })?;
                let bound_addr = listener.local_addr().unwrap_or(fallback_addr);
                tracing::warn!(
                    requested = %requested_addr,
                    fallback = %bound_addr,
                    "gateway port is in use; using an automatically allocated port"
                );
                listener
            }
            Err(err) => return Err(err.to_string()),
        };
        let bound_addr = listener
            .local_addr()
            .map_err(|err| format!("failed to read gateway bind address: {err}"))?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let status_arc = self.status.clone();
        let app_for_task = app.clone();
        let join = tokio::spawn(async move {
            let result = server
                .run_with_listener_and_shutdown(listener, async move {
                    let _ = shutdown_rx.await;
                })
                .await;
            let new_status = match result {
                Ok(()) => GatewayStatus::Stopped,
                Err(err) => GatewayStatus::Error {
                    message: err.to_string(),
                },
            };
            *status_arc.lock().unwrap() = new_status.clone();
            let _ = app_for_task.emit("gateway://status", &new_status);
        });

        let status = GatewayStatus::Running {
            addr: bound_addr.to_string(),
        };
        *self.instance.lock().unwrap() = Some(RunningInstance {
            shutdown: shutdown_tx,
            join,
        });
        self.set_status(app, status.clone());
        Ok(status)
    }

    pub async fn stop<R: Runtime>(&self, app: &AppHandle<R>) {
        let instance = self.instance.lock().unwrap().take();
        if let Some(instance) = instance {
            let _ = instance.shutdown.send(());
            let _ = instance.join.await;
            self.set_status(app, GatewayStatus::Stopped);
        }
    }
}

impl Default for GatewayController {
    fn default() -> Self {
        Self::new()
    }
}

