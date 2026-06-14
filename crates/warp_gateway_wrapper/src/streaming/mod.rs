// Streaming support for long-running tasks
// TODO: Implement SSE or WebSocket streaming

use tokio::sync::mpsc;
use crate::protocol::GatewayResponse;

pub type ProgressSender = mpsc::Sender<GatewayResponse>;
pub type ProgressReceiver = mpsc::Receiver<GatewayResponse>;

pub fn create_progress_channel(buffer_size: usize) -> (ProgressSender, ProgressReceiver) {
    mpsc::channel(buffer_size)
}
