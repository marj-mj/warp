//! Proxy controller: owns a tokio runtime and runs the transparent Warp proxy
//! in-process, with start/stop and observable status.

use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use warp_gateway_wrapper::proxy::{ProxyConfig, ProxyServer};

#[derive(Debug, Clone, PartialEq, Eq)]
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
    runtime: Runtime,
    status: Arc<Mutex<ProxyStatus>>,
    instance: Option<RunningInstance>,
}

impl ProxyController {
    pub fn new() -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        Ok(Self {
            runtime,
            status: Arc::new(Mutex::new(ProxyStatus::Stopped)),
            instance: None,
        })
    }

    pub fn status(&self) -> ProxyStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn is_running(&self) -> bool {
        self.instance.is_some()
    }

    pub fn start(&mut self, host: &str, port: u16, config: ProxyConfig) {
        if self.instance.is_some() {
            return;
        }

        let server = match ProxyServer::new(host, port, config) {
            Ok(server) => server,
            Err(err) => {
                *self.status.lock().unwrap() = ProxyStatus::Error {
                    message: format!("invalid bind address: {err}"),
                };
                return;
            }
        };

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let (ready_tx, ready_rx) = oneshot::channel::<Result<std::net::SocketAddr, String>>();
        let status = self.status.clone();

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
                    *status.lock().unwrap() = ProxyStatus::Stopped;
                }
                Err(err) => {
                    *status.lock().unwrap() = ProxyStatus::Error {
                        message: err.to_string(),
                    };
                }
            }
        });

        match self.runtime.block_on(async { ready_rx.await }) {
            Ok(Ok(addr)) => {
                *self.status.lock().unwrap() = ProxyStatus::Running {
                    addr: addr.to_string(),
                };
                self.instance = Some(RunningInstance {
                    shutdown: shutdown_tx,
                    join,
                });
            }
            Ok(Err(message)) => {
                *self.status.lock().unwrap() = ProxyStatus::Error { message };
            }
            Err(_) => {
                *self.status.lock().unwrap() = ProxyStatus::Error {
                    message: "proxy exited before reporting readiness".to_string(),
                };
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(instance) = self.instance.take() {
            let _ = instance.shutdown.send(());
            let _ = self.runtime.block_on(instance.join);
            *self.status.lock().unwrap() = ProxyStatus::Stopped;
        }
    }
}

impl Drop for ProxyController {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[path = "proxy_controller_tests.rs"]
mod tests;
