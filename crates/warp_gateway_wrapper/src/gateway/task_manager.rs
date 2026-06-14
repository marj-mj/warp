use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;
use crate::utils::TaskId;

/// Manages active tasks with cancellation support
pub struct TaskManager {
    tasks: Arc<RwLock<HashMap<TaskId, TaskHandle>>>,
}

/// Handle for a running task
pub struct TaskHandle {
    /// Cancellation token for this task
    pub cancellation_token: CancellationToken,
    /// Channel for progress updates
    pub progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
}

/// Progress update from a task
#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub task_id: TaskId,
    pub progress: f32,  // 0.0 to 1.0
    pub message: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Register a new task and get its cancellation token and progress sender
    pub async fn register_task(&self, task_id: TaskId) -> (CancellationToken, mpsc::UnboundedReceiver<ProgressUpdate>) {
        let cancellation_token = CancellationToken::new();
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();
        
        let handle = TaskHandle {
            cancellation_token: cancellation_token.clone(),
            progress_tx,
        };
        
        self.tasks.write().await.insert(task_id, handle);
        
        (cancellation_token, progress_rx)
    }
    
    /// Unregister a task when it completes
    pub async fn unregister_task(&self, task_id: &TaskId) {
        self.tasks.write().await.remove(task_id);
    }
    
    /// Cancel a specific task
    pub async fn cancel_task(&self, task_id: &TaskId) -> bool {
        let tasks = self.tasks.read().await;
        
        if let Some(handle) = tasks.get(task_id) {
            handle.cancellation_token.cancel();
            true
        } else {
            false
        }
    }
    
    /// Cancel all tasks
    pub async fn cancel_all(&self) {
        let tasks = self.tasks.read().await;
        
        for handle in tasks.values() {
            handle.cancellation_token.cancel();
        }
    }
    
    /// Get the number of active tasks
    pub async fn active_count(&self) -> usize {
        self.tasks.read().await.len()
    }
    
    /// Check if a task is active
    pub async fn is_active(&self, task_id: &TaskId) -> bool {
        self.tasks.read().await.contains_key(task_id)
    }
    
    /// Send progress update for a task
    pub async fn send_progress(&self, update: ProgressUpdate) -> bool {
        let tasks = self.tasks.read().await;
        
        if let Some(handle) = tasks.get(&update.task_id) {
            handle.progress_tx.send(update).is_ok()
        } else {
            false
        }
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_task_registration() {
        let manager = TaskManager::new();
        let task_id = TaskId::new();
        
        let (token, _rx) = manager.register_task(task_id.clone()).await;
        
        assert!(manager.is_active(&task_id).await);
        assert!(!token.is_cancelled());
        
        manager.unregister_task(&task_id).await;
        assert!(!manager.is_active(&task_id).await);
    }
    
    #[tokio::test]
    async fn test_task_cancellation() {
        let manager = TaskManager::new();
        let task_id = TaskId::new();
        
        let (token, _rx) = manager.register_task(task_id.clone()).await;
        
        assert!(!token.is_cancelled());
        
        let cancelled = manager.cancel_task(&task_id).await;
        assert!(cancelled);
        assert!(token.is_cancelled());
    }
    
    #[tokio::test]
    async fn test_cancel_nonexistent_task() {
        let manager = TaskManager::new();
        let task_id = TaskId::new();
        
        let cancelled = manager.cancel_task(&task_id).await;
        assert!(!cancelled);
    }
    
    #[tokio::test]
    async fn test_cancel_all() {
        let manager = TaskManager::new();
        
        let task1 = TaskId::new();
        let task2 = TaskId::new();
        
        let (token1, _) = manager.register_task(task1.clone()).await;
        let (token2, _) = manager.register_task(task2.clone()).await;
        
        assert_eq!(manager.active_count().await, 2);
        
        manager.cancel_all().await;
        
        assert!(token1.is_cancelled());
        assert!(token2.is_cancelled());
    }
    
    #[tokio::test]
    async fn test_progress_updates() {
        let manager = TaskManager::new();
        let task_id = TaskId::new();
        
        let (_token, mut rx) = manager.register_task(task_id.clone()).await;
        
        let update = ProgressUpdate {
            task_id: task_id.clone(),
            progress: 0.5,
            message: Some("Half done".to_string()),
            metadata: None,
        };
        
        assert!(manager.send_progress(update.clone()).await);
        
        let received = rx.recv().await.unwrap();
        assert_eq!(received.task_id, task_id);
        assert_eq!(received.progress, 0.5);
        assert_eq!(received.message, Some("Half done".to_string()));
    }
}
