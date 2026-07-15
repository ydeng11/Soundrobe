//! Shared task progress and cancellation registry for auto-tag/audit workflows.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgress {
    pub task_id: String,
    pub status: TaskStatus,
    pub progress: u64,
    pub total: u64,
    pub message: String,
    pub result: Value,
}

#[derive(Debug)]
struct TaskEntry {
    progress: TaskProgress,
    cancelled: Arc<AtomicBool>,
}

#[derive(Debug, Default)]
pub struct TaskRegistry {
    counter: AtomicU64,
    tasks: Mutex<HashMap<String, TaskEntry>>,
}

impl TaskRegistry {
    pub fn create(&self, prefix: &str, total: u64, message: impl Into<String>) -> String {
        let counter = self.counter.fetch_add(1, Ordering::AcqRel) + 1;
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let task_id = format!("{prefix}-{counter}-{millis}");
        let entry = TaskEntry {
            progress: TaskProgress {
                task_id: task_id.clone(),
                status: TaskStatus::Running,
                progress: 0,
                total,
                message: message.into(),
                result: Value::Null,
            },
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.insert(task_id.clone(), entry);
        } else {
            tracing::error!("task registry mutex poisoned; task was not recorded");
        }
        task_id
    }

    pub fn get(&self, task_id: &str) -> Option<TaskProgress> {
        self.tasks
            .lock()
            .ok()
            .and_then(|tasks| tasks.get(task_id).map(|entry| entry.progress.clone()))
    }

    pub fn cancellation(&self, task_id: &str) -> Option<Arc<AtomicBool>> {
        self.tasks
            .lock()
            .ok()
            .and_then(|tasks| tasks.get(task_id).map(|entry| Arc::clone(&entry.cancelled)))
    }

    pub fn update(&self, task_id: &str, progress: u64, message: impl Into<String>) -> bool {
        let Ok(mut tasks) = self.tasks.lock() else {
            return false;
        };
        let Some(entry) = tasks.get_mut(task_id) else {
            return false;
        };
        if entry.progress.status != TaskStatus::Running {
            return false;
        }
        entry.progress.progress = progress;
        entry.progress.message = message.into();
        true
    }

    pub fn finish(
        &self,
        task_id: &str,
        status: TaskStatus,
        message: impl Into<String>,
        result: Value,
    ) -> bool {
        let Ok(mut tasks) = self.tasks.lock() else {
            return false;
        };
        let Some(entry) = tasks.get_mut(task_id) else {
            return false;
        };
        entry.progress.status = status;
        entry.progress.message = message.into();
        entry.progress.result = result;
        if status == TaskStatus::Completed {
            entry.progress.progress = entry.progress.total;
        }
        if status == TaskStatus::Cancelled {
            entry.cancelled.store(true, Ordering::Release);
        }
        true
    }

    pub fn cancel(&self, task_id: &str) {
        let Ok(mut tasks) = self.tasks.lock() else {
            tracing::error!("task registry mutex poisoned; cancellation unavailable");
            return;
        };
        if let Some(entry) = tasks.get_mut(task_id) {
            entry.cancelled.store(true, Ordering::Release);
            entry.progress.status = TaskStatus::Cancelled;
            entry.progress.message = "Cancelled".to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_tracks_updates_completion_and_unknowns() {
        let registry = TaskRegistry::default();
        assert_eq!(registry.get("missing"), None);
        registry.cancel("missing");
        let id = registry.create("auto-tag", 9, "Starting...");
        assert!(id.starts_with("auto-tag-1-"));
        assert_eq!(registry.get(&id).unwrap().status, TaskStatus::Running);
        assert!(registry.update(&id, 3, "Looking up"));
        assert_eq!(registry.get(&id).unwrap().progress, 3);
        assert!(registry.finish(
            &id,
            TaskStatus::Completed,
            "Done",
            serde_json::json!({"ok": true})
        ));
        let done = registry.get(&id).unwrap();
        assert_eq!(done.progress, 9);
        assert_eq!(done.status, TaskStatus::Completed);
        assert!(!registry.update(&id, 4, "late"));
    }

    #[test]
    fn cancellation_updates_status_and_shared_token() {
        let registry = TaskRegistry::default();
        let id = registry.create("audit", 4, "Starting...");
        let token = registry.cancellation(&id).unwrap();
        assert!(!token.load(Ordering::Acquire));
        registry.cancel(&id);
        assert!(token.load(Ordering::Acquire));
        let progress = registry.get(&id).unwrap();
        assert_eq!(progress.status, TaskStatus::Cancelled);
        assert_eq!(progress.message, "Cancelled");
    }

    #[test]
    fn ids_are_monotonic_even_with_same_timestamp() {
        let registry = TaskRegistry::default();
        let first = registry.create("task", 1, "one");
        let second = registry.create("task", 1, "two");
        assert!(first.starts_with("task-1-"));
        assert!(second.starts_with("task-2-"));
        assert_ne!(first, second);
    }
}
