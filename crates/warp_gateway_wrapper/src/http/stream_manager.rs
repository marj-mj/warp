use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};

use crate::http::sse::{SSEEvent, StreamEnvelope};
use crate::utils::TaskId;

/// Per-task stream state: the broadcast sender, a monotonic event counter, and a
/// bounded history buffer used to replay missed events on reconnect.
struct TaskStream {
    sender: broadcast::Sender<StreamEnvelope>,
    next_id: AtomicU64,
    history: RwLock<VecDeque<StreamEnvelope>>,
    history_capacity: usize,
}

impl TaskStream {
    fn new(channel_capacity: usize, history_capacity: usize) -> Self {
        let (sender, _rx) = broadcast::channel(channel_capacity);
        Self {
            sender,
            next_id: AtomicU64::new(1),
            history: RwLock::new(VecDeque::with_capacity(history_capacity)),
            history_capacity,
        }
    }
}

/// Manages SSE broadcast channels and replay history for active tasks.
pub struct StreamManager {
    channels: Arc<RwLock<HashMap<TaskId, Arc<TaskStream>>>>,
    channel_capacity: usize,
    history_capacity: usize,
}

impl StreamManager {
    pub fn new(channel_capacity: usize) -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            channel_capacity,
            // Keep a generous history so a brief disconnect can be fully replayed.
            history_capacity: channel_capacity.max(256),
        }
    }

    /// Create a new broadcast channel for a task.
    pub async fn create_channel(&self, task_id: TaskId) -> broadcast::Sender<StreamEnvelope> {
        let stream = Arc::new(TaskStream::new(
            self.channel_capacity,
            self.history_capacity,
        ));
        let sender = stream.sender.clone();
        self.channels.write().await.insert(task_id, stream);
        sender
    }

    /// Subscribe to a task's live event stream.
    pub async fn subscribe(&self, task_id: &TaskId) -> Option<broadcast::Receiver<StreamEnvelope>> {
        let channels = self.channels.read().await;
        channels
            .get(task_id)
            .map(|stream| stream.sender.subscribe())
    }

    /// Send an event to a task's stream, assigning it the next sequence id and
    /// recording it in the replay history. Returns the assigned id.
    pub async fn send_event(&self, task_id: &TaskId, event: SSEEvent) -> Result<u64, String> {
        let stream = {
            let channels = self.channels.read().await;
            channels
                .get(task_id)
                .cloned()
                .ok_or_else(|| format!("No stream found for task {}", task_id))?
        };

        let id = stream.next_id.fetch_add(1, Ordering::SeqCst);
        let envelope = StreamEnvelope::new(id, event);

        // Record in history (bounded ring buffer) before broadcasting.
        {
            let mut history = stream.history.write().await;
            if history.len() == stream.history_capacity {
                history.pop_front();
            }
            history.push_back(envelope.clone());
        }

        // A send error only means there are no live subscribers; the event is
        // still retained in history for later replay, so treat it as success.
        let _ = stream.sender.send(envelope);
        Ok(id)
    }

    /// Return all buffered events with an id strictly greater than `last_id`.
    /// Used to replay events a reconnecting client missed (Last-Event-ID).
    pub async fn replay_since(&self, task_id: &TaskId, last_id: u64) -> Vec<StreamEnvelope> {
        let stream = {
            let channels = self.channels.read().await;
            channels.get(task_id).cloned()
        };
        let Some(stream) = stream else {
            return Vec::new();
        };
        let history = stream.history.read().await;
        history
            .iter()
            .filter(|envelope| envelope.id > last_id)
            .cloned()
            .collect()
    }

    /// Remove a channel when a task completes.
    pub async fn remove_channel(&self, task_id: &TaskId) {
        self.channels.write().await.remove(task_id);
    }

    /// Count of active streams.
    pub async fn active_count(&self) -> usize {
        self.channels.read().await.len()
    }

    /// Whether a stream exists for the task.
    pub async fn has_stream(&self, task_id: &TaskId) -> bool {
        self.channels.read().await.contains_key(task_id)
    }
}

impl Default for StreamManager {
    fn default() -> Self {
        Self::new(100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_and_subscribe() {
        let manager = StreamManager::new(10);
        let task_id = TaskId::new();

        manager.create_channel(task_id.clone()).await;
        assert!(manager.has_stream(&task_id).await);

        let mut rx = manager.subscribe(&task_id).await.unwrap();

        manager
            .send_event(
                &task_id,
                SSEEvent::Progress {
                    task_id: task_id.to_string(),
                    progress: 0.5,
                    message: None,
                    metadata: None,
                },
            )
            .await
            .unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received.id, 1);
        match received.event {
            SSEEvent::Progress { progress, .. } => assert_eq!(progress, 0.5),
            _ => panic!("Wrong event type"),
        }
    }

    #[tokio::test]
    async fn test_send_event_assigns_increasing_ids() {
        let manager = StreamManager::new(10);
        let task_id = TaskId::new();
        manager.create_channel(task_id.clone()).await;

        let id1 = manager.send_event(&task_id, SSEEvent::Ping).await.unwrap();
        let id2 = manager.send_event(&task_id, SSEEvent::Ping).await.unwrap();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[tokio::test]
    async fn test_replay_since() {
        let manager = StreamManager::new(10);
        let task_id = TaskId::new();
        manager.create_channel(task_id.clone()).await;

        for _ in 0..5 {
            manager.send_event(&task_id, SSEEvent::Ping).await.unwrap();
        }

        // Replay everything after id 2 -> ids 3,4,5.
        let replayed = manager.replay_since(&task_id, 2).await;
        let ids: Vec<u64> = replayed.iter().map(|envelope| envelope.id).collect();
        assert_eq!(ids, vec![3, 4, 5]);

        // Replay after the last id -> nothing.
        assert!(manager.replay_since(&task_id, 5).await.is_empty());
    }

    #[tokio::test]
    async fn test_remove_channel() {
        let manager = StreamManager::new(10);
        let task_id = TaskId::new();

        manager.create_channel(task_id.clone()).await;
        assert_eq!(manager.active_count().await, 1);

        manager.remove_channel(&task_id).await;
        assert_eq!(manager.active_count().await, 0);
        assert!(!manager.has_stream(&task_id).await);
    }

    #[tokio::test]
    async fn test_subscribe_nonexistent() {
        let manager = StreamManager::new(10);
        let task_id = TaskId::new();
        assert!(manager.subscribe(&task_id).await.is_none());
    }
}
