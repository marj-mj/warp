//! Gateway controller: owns a tokio runtime and runs the Managed Provider
//! Gateway in-process, with start/stop and observable status.
//!
//! The egui UI thread stays responsive; all async work happens on a dedicated
//! multi-thread tokio runtime owned by the controller.

use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use warp_gateway_wrapper::mpg::{GatewayConfig, MpgServer};

/// Observable gateway status, shared with the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    #[allow(dead_code)]
    pub fn label(&self) -> String {
        match self {
            Self::Stopped => "stopped".to_string(),
            Self::Running { addr } => format!("running @ {addr}"),
            Self::Error { message } => format!("error: {message}"),
        }
    }
}

/// Handle to a running gateway instance: the shutdown trigger + join handle.
struct RunningInstance {
    shutdown: oneshot::Sender<()>,
    join: tokio::task::JoinHandle<()>,
}

pub struct GatewayController {
    runtime: Runtime,
    status: Arc<Mutex<GatewayStatus>>,
    instance: Option<RunningInstance>,
}

impl GatewayController {
    pub fn new() -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        Ok(Self {
            runtime,
            status: Arc::new(Mutex::new(GatewayStatus::Stopped)),
            instance: None,
        })
    }

    /// Handle to the controller's tokio runtime, for spawning auxiliary work
    /// (e.g. provider probes) without blocking the UI thread.
    pub fn runtime_handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }

    /// Current status snapshot.
    pub fn status(&self) -> GatewayStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn is_running(&self) -> bool {
        self.instance.is_some()
    }

    /// Start the gateway with the given host/port/config. No-op if already running.
    pub fn start(&mut self, host: &str, port: u16, config: GatewayConfig) {
        if self.instance.is_some() {
            return;
        }

        let server = match MpgServer::new(host, port, config) {
            Ok(server) => server,
            Err(err) => {
                *self.status.lock().unwrap() = GatewayStatus::Error {
                    message: format!("invalid bind address: {err}"),
                };
                return;
            }
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let (ready_tx, ready_rx) = oneshot::channel::<Result<std::net::SocketAddr, String>>();
        let status = self.status.clone();

        // Spawn the server task on the controller's runtime. Catch the run
        // result so a bind/runtime failure surfaces as an Error status.
        let join = self.runtime.spawn(async move {
            let result = server
                .run_with_shutdown_signal(
                    async move {
                    let _ = shutdown_rx.await;
                    },
                    Some(ready_tx),
                )
                .await;
            match result {
                Ok(()) => {
                    *status.lock().unwrap() = GatewayStatus::Stopped;
                }
                Err(err) => {
                    *status.lock().unwrap() = GatewayStatus::Error {
                        message: err.to_string(),
                    };
                }
            }
        });

        match self.runtime.block_on(async { ready_rx.await }) {
            Ok(Ok(addr)) => {
                *self.status.lock().unwrap() = GatewayStatus::Running {
                    addr: addr.to_string(),
                };
                self.instance = Some(RunningInstance {
                    shutdown: shutdown_tx,
                    join,
                });
            }
            Ok(Err(message)) => {
                *self.status.lock().unwrap() = GatewayStatus::Error { message };
            }
            Err(_) => {
                *self.status.lock().unwrap() = GatewayStatus::Error {
                    message: "gateway exited before reporting readiness".to_string(),
                };
            }
        }
    }

    /// Stop the running gateway, waiting for graceful shutdown. No-op if stopped.
    pub fn stop(&mut self) {
        if let Some(instance) = self.instance.take() {
            // Signal shutdown; ignore error if the task already finished.
            let _ = instance.shutdown.send(());
            // Wait for the server task to wind down so the port is freed.
            let _ = self.runtime.block_on(instance.join);
            *self.status.lock().unwrap() = GatewayStatus::Stopped;
        }
    }
}

impl Drop for GatewayController {
    fn drop(&mut self) {
        // Ensure the server is stopped before the runtime is dropped.
        self.stop();
    }
}

#[cfg(test)]
#[path = "controller_tests.rs"]
mod tests;
