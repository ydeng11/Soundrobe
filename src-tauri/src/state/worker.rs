use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use time::OffsetDateTime;

const WORKER_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS worker_items (
  path TEXT PRIMARY KEY,
  trigger TEXT NOT NULL,
  status TEXT NOT NULL,
  run_id TEXT,
  reason TEXT,
  authority TEXT,
  updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS review_flags (
  path TEXT PRIMARY KEY,
  reason TEXT NOT NULL,
  run_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  reviewed_at INTEGER
);
"#;

#[derive(Clone)]
pub struct WorkerStore {
    path: PathBuf,
    inner: Arc<Mutex<Option<Connection>>>,
}

impl WorkerStore {
    pub fn new_in(data_dir: PathBuf) -> Self {
        Self {
            path: data_dir.join("worker.db"),
            inner: Arc::new(Mutex::new(None)),
        }
    }

    pub fn initialize(&self) -> bool {
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };
        if guard.is_some() {
            return true;
        }
        if let Some(parent) = self.path.parent() {
            if fs::create_dir_all(parent).is_err() {
                return false;
            }
        }
        let Ok(connection) = Connection::open(&self.path) else {
            return false;
        };
        if connection.execute_batch(WORKER_SCHEMA).is_err() {
            return false;
        }
        *guard = Some(connection);
        true
    }

    pub fn enqueue(&self, paths: &[PathBuf], trigger: &str) -> Result<Vec<PathBuf>, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let mut accepted = Vec::new();
        for path in paths {
            let path = storage_key(path);
            let existing: Option<String> = connection
                .query_row(
                    "SELECT status FROM worker_items WHERE path = ?1",
                    [&path],
                    |row| row.get(0),
                )
                .ok();
            if matches!(existing.as_deref(), Some("pending" | "running")) {
                continue;
            }
            connection
                .execute(
                    "INSERT OR REPLACE INTO worker_items
                     (path, trigger, status, run_id, reason, authority, updated_at)
                     VALUES (?1, ?2, 'pending', NULL, NULL, NULL, ?3)",
                    params![path, trigger, now_epoch()],
                )
                .map_err(|error| error.to_string())?;
            accepted.push(PathBuf::from(path));
        }
        Ok(accepted)
    }

    pub fn is_active(&self, path: &Path) -> bool {
        let Ok(guard) = self.inner.lock() else {
            return false;
        };
        let Some(connection) = guard.as_ref() else {
            return false;
        };
        connection
            .query_row(
                "SELECT status FROM worker_items WHERE path = ?1",
                [storage_key(path).as_str()],
                |row| row.get::<_, String>(0),
            )
            .map(|status| matches!(status.as_str(), "pending" | "running"))
            .unwrap_or(false)
    }

    pub fn pending(&self) -> Vec<PathBuf> {
        let Ok(guard) = self.inner.lock() else {
            return Vec::new();
        };
        let Some(connection) = guard.as_ref() else {
            return Vec::new();
        };
        let Ok(mut statement) = connection.prepare(
            "SELECT path FROM worker_items WHERE status = 'pending' ORDER BY updated_at, path",
        ) else {
            return Vec::new();
        };
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(PathBuf::from)
            .collect()
    }

    pub fn recover_interrupted(&self) -> Result<usize, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection
            .execute(
                "UPDATE worker_items
                 SET status = 'pending', run_id = NULL, updated_at = ?1
                 WHERE status = 'running'",
                [now_epoch()],
            )
            .map_err(|error| error.to_string())
    }

    pub fn mark_running(&self, path: &Path, run_id: &str) -> Result<(), String> {
        self.update_item(path, "running", Some(run_id), None, None)
    }

    pub fn mark_finished(
        &self,
        path: &Path,
        status: &str,
        reason: Option<&str>,
        authority: Option<&str>,
    ) -> Result<(), String> {
        self.update_item(path, status, None, reason, authority)
    }

    fn update_item(
        &self,
        path: &Path,
        status: &str,
        run_id: Option<&str>,
        reason: Option<&str>,
        authority: Option<&str>,
    ) -> Result<(), String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection
            .execute(
                "UPDATE worker_items
                 SET status = ?2, run_id = COALESCE(?3, run_id), reason = ?4,
                     authority = ?5, updated_at = ?6
                 WHERE path = ?1",
                params![
                    storage_key(path),
                    status,
                    run_id,
                    reason,
                    authority,
                    now_epoch(),
                ],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn record_review_flag(
        &self,
        path: &Path,
        reason: &str,
        run_id: &str,
    ) -> Result<(), String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection
            .execute(
                "INSERT OR REPLACE INTO review_flags
                 (path, reason, run_id, created_at, reviewed_at)
                 VALUES (?1, ?2, ?3, ?4, NULL)",
                params![storage_key(path), reason, run_id, now_epoch()],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn unreviewed_review_paths(&self) -> Result<Vec<PathBuf>, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let mut statement = connection
            .prepare("SELECT path FROM review_flags WHERE reviewed_at IS NULL ORDER BY path")
            .map_err(|error| error.to_string())?;
        let paths = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(paths.into_iter().map(PathBuf::from).collect())
    }

    pub fn mark_reviewed(&self, path: &Path) -> Result<(), String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let requested = path.to_string_lossy().into_owned();
        let canonical = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned();
        connection
            .execute(
                "UPDATE review_flags SET reviewed_at = ?3
                 WHERE path IN (?1, ?2) AND reviewed_at IS NULL",
                params![requested, canonical, now_epoch()],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn storage_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn now_epoch() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "soundrobe-worker-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn pending_work_is_deduplicated_and_running_work_recovers_after_restart() {
        let root = root();
        let album = root.join("Artist/Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        assert_eq!(
            store
                .enqueue(std::slice::from_ref(&album), "watch")
                .unwrap(),
            vec![album.clone()]
        );
        assert!(store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap()
            .is_empty());
        store.mark_running(&album, "run-1").unwrap();
        drop(store);

        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        assert_eq!(reopened.recover_interrupted().unwrap(), 1);
        assert_eq!(reopened.pending(), vec![album]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn completed_work_and_review_flags_survive_restart_until_acknowledged() {
        let root = root();
        let album = root.join("Artist/Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        store.mark_running(&album, "run-1").unwrap();
        store
            .mark_finished(&album, "needs_review", Some("ai_fallback"), Some("llm"))
            .unwrap();
        store
            .record_review_flag(&album, "ai_fallback", "run-1")
            .unwrap();
        drop(store);

        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        assert_eq!(
            reopened.unreviewed_review_paths().unwrap(),
            vec![album.clone()]
        );
        reopened.mark_reviewed(&album).unwrap();
        assert!(reopened.unreviewed_review_paths().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
