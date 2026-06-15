//! Proxy controller — async, owned by Tauri''s tokio runtime.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::oneshot;
use warp_gateway_wrapper::proxy::{ProxyConfig, ProxyServer};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProxyStatus {
    Stopped,
    Running { addr: String },
    Error { message: String },
}

struct RunningInstance {
    shutdown: oneshot::Sender<()>,
    join: tokio::task::JoinHandle<()>,
}

pub struct ProxyController {
    status: Arc<Mutex<ProxyStatus>>,
    instance: Mutex<Option<RunningInstance>>,
}

impl ProxyController {
    pub fn new() -> Self {
        Self {
            status: Arc::new(Mutex::new(ProxyStatus::Stopped)),
            instance: Mutex::new(None),
        }
    }

    pub fn status(&self) -> ProxyStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn is_running(&self) -> bool {
        self.instance.lock().unwrap().is_some()
    }

    pub async fn start(&self, host: &str, port: u16, config: ProxyConfig) -> ProxyStatus {
        if self.is_running() {
            return self.status();
        }

        let server = match ProxyServer::new(host, port, config) {
            Ok(s) => s,
            Err(err) => {
                let status = ProxyStatus::Error {
                    message: format!("invalid bind address: {err}"),
                };
                *self.status.lock().unwrap() = status.clone();
                return status;
            }
        };

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let (ready_tx, ready_rx) = oneshot::channel::<Result<std::net::SocketAddr, String>>();
        let status = self.status.clone();

        let join = tokio::spawn(async move {
            let result = server
                .run_with_shutdown_signal(
                    async move { let _ = shutdown_rx.await; },
                    Some(ready_tx),
                )
                .await;
            match result {
                Ok(()) => *status.lock().unwrap() = ProxyStatus::Stopped,
                Err(err) => {
                    *status.lock().unwrap() = ProxyStatus::Error {
                        message: err.to_string(),
                    };
                }
            }
        });

        let new_status = match ready_rx.await {
            Ok(Ok(addr)) => {
                let s = ProxyStatus::Running { addr: addr.to_string() };
                *self.status.lock().unwrap() = s.clone();
                *self.instance.lock().unwrap() = Some(RunningInstance { shutdown: shutdown_tx, join });
                s
            }
            Ok(Err(message)) => {
                let s = ProxyStatus::Error { message };
                *self.status.lock().unwrap() = s.clone();
                s
            }
            Err(_) => {
                let s = ProxyStatus::Error {
                    message: "proxy exited before reporting readiness".to_string(),
                };
                *self.status.lock().unwrap() = s.clone();
                s
            }
        };
        new_status
    }

    pub async fn stop(&self) {
        let instance = self.instance.lock().unwrap().take();
        if let Some(instance) = instance {
            let _ = instance.shutdown.send(());
            let _ = instance.join.await;
            *self.status.lock().unwrap() = ProxyStatus::Stopped;
        }
    }
}

impl Default for ProxyController {
    fn default() -> Self { Self::new() }
}
