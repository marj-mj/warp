//! Async probe runner for the launcher.
//!
//! Runs `mpg::probe::probe_provider` on the controller's tokio runtime and
//! delivers the result back to the UI thread via a oneshot channel, so the UI
//! never blocks while a probe is in flight.

use tokio::sync::oneshot;
use warp_gateway_wrapper::mpg::probe::{probe_provider, CapabilityMatrix};

/// A probe in progress: holds the receiver the UI polls each frame.
pub struct PendingProbe {
    rx: oneshot::Receiver<CapabilityMatrix>,
}

impl PendingProbe {
    /// Spawn a probe on `handle`. `api_key` may be empty.
    pub fn spawn(
        handle: &tokio::runtime::Handle,
        base_url: String,
        api_key: Option<String>,
    ) -> Self {
        let (tx, rx) = oneshot::channel();
        handle.spawn(async move {
            let matrix = probe_provider(&base_url, api_key.as_deref()).await;
            let _ = tx.send(matrix);
        });
        Self { rx }
    }

    /// Non-blocking poll: returns the matrix once the probe completes.
    pub fn poll(&mut self) -> Option<CapabilityMatrix> {
        match self.rx.try_recv() {
            Ok(matrix) => Some(matrix),
            Err(_) => None,
        }
    }
}
