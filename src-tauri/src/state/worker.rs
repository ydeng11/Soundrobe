use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
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

#[derive(Clone, Deserialize, Serialize)]
struct WorkerHalt {
    paths: Vec<String>,
    reason: String,
    error: String,
}

impl WorkerHalt {
    fn message(&self) -> String {
        format!(
            "worker scheduling halted after cancellation persistence failed: {}",
            self.error
        )
    }
}

#[derive(Clone)]
pub struct WorkerStore {
    path: PathBuf,
    halt_path: PathBuf,
    inner: Arc<Mutex<Option<Connection>>>,
    halted: Arc<Mutex<Option<WorkerHalt>>>,
}

impl WorkerStore {
    pub fn new_in(data_dir: PathBuf) -> Self {
        Self {
            path: data_dir.join("worker.db"),
            halt_path: data_dir.join("worker.halted.json"),
            inner: Arc::new(Mutex::new(None)),
            halted: Arc::new(Mutex::new(None)),
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
        let halt = match fs::read_to_string(&self.halt_path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(halt) => Some(halt),
                Err(_) => return false,
            },
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(_) => return false,
        };
        *guard = Some(connection);
        drop(guard);
        let Ok(mut halted) = self.halted.lock() else {
            return false;
        };
        *halted = halt;
        true
    }

    pub fn enqueue(&self, paths: &[PathBuf], trigger: &str) -> Result<Vec<PathBuf>, String> {
        self.ensure_not_halted()?;
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

    /// Explicit operator retry may reclaim a stale `running` row left behind
    /// when the previous terminal checkpoint could not be written. Ordinary
    /// enqueue keeps deduplicating active work.
    pub fn requeue(&self, paths: &[PathBuf], trigger: &str) -> Result<Vec<PathBuf>, String> {
        self.ensure_not_halted()?;
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_mut().ok_or("worker state not initialized")?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        for path in paths {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO worker_items
                     (path, trigger, status, run_id, reason, authority, updated_at)
                     VALUES (?1, ?2, 'pending', NULL, NULL, NULL, ?3)",
                    params![storage_key(path), trigger, now_epoch()],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(paths.to_vec())
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

    #[cfg(test)]
    fn pending(&self) -> Vec<PathBuf> {
        self.pending_result().unwrap_or_default()
    }

    pub fn pending_result(&self) -> Result<Vec<PathBuf>, String> {
        self.ensure_not_halted()?;
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let mut statement = connection
            .prepare(
                "SELECT path FROM worker_items WHERE status = 'pending' ORDER BY updated_at, path",
            )
            .map_err(|error| error.to_string())?;
        let paths = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(paths.into_iter().map(PathBuf::from).collect())
    }

    pub fn recover_interrupted(&self) -> Result<usize, String> {
        let halt = self
            .halted
            .lock()
            .map_err(|_| "worker halt state unavailable")?
            .clone();
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_mut().ok_or("worker state not initialized")?;
        if let Some(halt) = halt {
            let transaction = connection
                .transaction()
                .map_err(|error| halt_recovery_error(&halt, error))?;
            for path in &halt.paths {
                transaction
                    .execute(
                        "UPDATE worker_items
                         SET status = 'cancelled', run_id = NULL, reason = ?2, updated_at = ?3
                         WHERE path = ?1 AND status IN ('pending', 'running')",
                        params![path, &halt.reason, now_epoch()],
                    )
                    .map_err(|error| halt_recovery_error(&halt, error))?;
            }
            let recovered = transaction
                .execute(
                    "UPDATE worker_items
                     SET status = 'pending', run_id = NULL, updated_at = ?1
                     WHERE status = 'running'",
                    [now_epoch()],
                )
                .map_err(|error| halt_recovery_error(&halt, error))?;
            transaction
                .commit()
                .map_err(|error| halt_recovery_error(&halt, error))?;
            fs::remove_file(&self.halt_path).map_err(|error| halt_recovery_error(&halt, error))?;
            *self
                .halted
                .lock()
                .map_err(|_| "worker halt state unavailable")? = None;
            return Ok(recovered);
        }
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

    pub fn cancel_paths(&self, paths: &[PathBuf], reason: &str) -> Result<usize, String> {
        let result: Result<usize, String> = (|| {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| "worker state unavailable".to_string())?;
            let connection = guard
                .as_mut()
                .ok_or_else(|| "worker state not initialized".to_string())?;
            let transaction = connection
                .transaction()
                .map_err(|error| error.to_string())?;
            let mut changed = 0;
            for path in paths {
                changed += transaction
                    .execute(
                        "UPDATE worker_items
                     SET status = 'cancelled', run_id = NULL, reason = ?2, updated_at = ?3
                     WHERE path = ?1 AND status IN ('pending', 'running')",
                        params![storage_key(path), reason, now_epoch()],
                    )
                    .map_err(|error| error.to_string())?;
            }
            transaction.commit().map_err(|error| error.to_string())?;
            Ok(changed)
        })();
        if let Err(error) = &result {
            self.persist_halt(paths, reason, error)?;
        }
        result
    }

    fn ensure_not_halted(&self) -> Result<(), String> {
        match self
            .halted
            .lock()
            .map_err(|_| "worker halt state unavailable")?
            .as_ref()
        {
            Some(halt) => Err(halt.message()),
            None => Ok(()),
        }
    }

    fn persist_halt(&self, paths: &[PathBuf], reason: &str, error: &str) -> Result<(), String> {
        let halt = WorkerHalt {
            paths: paths.iter().map(|path| storage_key(path)).collect(),
            reason: reason.to_string(),
            error: error.to_string(),
        };
        *self
            .halted
            .lock()
            .map_err(|_| "worker halt state unavailable")? = Some(halt.clone());
        if let Some(parent) = self.halt_path.parent() {
            fs::create_dir_all(parent).map_err(|persist_error| {
                format!("{error}; worker halt could not be persisted: {persist_error}")
            })?;
        }
        let temporary = self.halt_path.with_extension("json.tmp");
        let contents = serde_json::to_vec(&halt).map_err(|persist_error| {
            format!("{error}; worker halt could not be serialized: {persist_error}")
        })?;
        fs::write(&temporary, contents).map_err(|persist_error| {
            format!("{error}; worker halt could not be persisted: {persist_error}")
        })?;
        fs::rename(&temporary, &self.halt_path).map_err(|persist_error| {
            format!("{error}; worker halt could not be persisted: {persist_error}")
        })?;
        Ok(())
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

    pub fn actionable_review_paths(&self) -> Result<Vec<PathBuf>, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let mut statement = connection
            .prepare(
                "SELECT path FROM worker_items WHERE status = 'needs_review'
                 UNION
                 SELECT path FROM review_flags WHERE reviewed_at IS NULL
                 ORDER BY path",
            )
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
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "UPDATE worker_items
                 SET status = 'completed', reason = NULL, updated_at = ?3
                 WHERE path IN (?1, ?2) AND status = 'needs_review'",
                params![requested, canonical, now_epoch()],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn storage_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn halt_recovery_error(halt: &WorkerHalt, error: impl std::fmt::Display) -> String {
    format!("{}; cancellation recovery failed: {error}", halt.message())
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

    #[test]
    fn cancellation_is_terminal_until_an_explicit_retry() {
        let root = root();
        let first = root.join("Artist/First");
        let second = root.join("Artist/Second");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(&[first.clone(), second.clone()], "manual")
            .unwrap();
        store.mark_running(&first, "run-1").unwrap();

        assert_eq!(
            store
                .cancel_paths(&[first.clone(), second.clone()], "operator_cancelled")
                .unwrap(),
            2
        );
        assert!(store.pending_result().unwrap().is_empty());
        assert_eq!(
            store
                .enqueue(std::slice::from_ref(&first), "retry")
                .unwrap(),
            vec![first.clone()]
        );
        assert_eq!(store.pending_result().unwrap(), vec![first]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_requeue_recovers_a_stale_running_checkpoint() {
        let root = root();
        let album = root.join("Artist/Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        store.mark_running(&album, "run-1").unwrap();
        assert!(store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap()
            .is_empty());

        assert_eq!(
            store
                .requeue(std::slice::from_ref(&album), "retry")
                .unwrap(),
            vec![album.clone()]
        );
        assert_eq!(store.pending_result().unwrap(), vec![album]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_cancellation_persists_halt_across_restart_and_blocks_enqueue() {
        let root = root();
        let album = root.join("Artist/Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        store.mark_running(&album, "run-1").unwrap();
        *store.inner.lock().unwrap() = None;
        assert!(store
            .cancel_paths(std::slice::from_ref(&album), "operator_cancelled")
            .is_err());

        drop(store);
        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        let error = reopened
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap_err();
        assert!(error.contains("scheduling halted"));
        assert_eq!(reopened.recover_interrupted().unwrap(), 0);
        assert!(reopened.pending_result().unwrap().is_empty());
        assert_eq!(
            reopened
                .enqueue(std::slice::from_ref(&album), "retry")
                .unwrap(),
            vec![album]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_recovery_keeps_halt_until_all_running_rows_are_reconciled() {
        let root = root();
        let cancelled = root.join("Artist/Cancelled");
        let interrupted = root.join("Artist/Interrupted");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(&[cancelled.clone(), interrupted.clone()], "manual")
            .unwrap();
        store.mark_running(&cancelled, "run-1").unwrap();
        store.mark_running(&interrupted, "run-1").unwrap();
        *store.inner.lock().unwrap() = None;
        assert!(store
            .cancel_paths(std::slice::from_ref(&cancelled), "operator_cancelled")
            .is_err());

        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        reopened
            .inner
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_pending_recovery
                 BEFORE UPDATE ON worker_items
                 WHEN NEW.status = 'pending'
                 BEGIN SELECT RAISE(ABORT, 'pending recovery failed'); END;",
            )
            .unwrap();
        assert!(reopened.recover_interrupted().is_err());
        assert!(reopened
            .enqueue(std::slice::from_ref(&interrupted), "manual")
            .unwrap_err()
            .contains("scheduling halted"));

        reopened
            .inner
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_pending_recovery")
            .unwrap();
        assert_eq!(reopened.recover_interrupted().unwrap(), 1);
        assert_eq!(reopened.pending_result().unwrap(), vec![interrupted]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_halt_marker_write_still_blocks_current_process_enqueue() {
        let root = root();
        let album = root.join("Artist/Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        fs::create_dir_all(&store.halt_path).unwrap();
        *store.inner.lock().unwrap() = None;

        assert!(store
            .cancel_paths(std::slice::from_ref(&album), "operator_cancelled")
            .unwrap_err()
            .contains("halt could not be persisted"));
        assert!(store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap_err()
            .contains("scheduling halted"));
        assert!(store.pending().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn provider_deferrals_are_actionable_after_restart_and_acknowledgement() {
        let root = root();
        let album = root.join("Artist/Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        store
            .mark_finished(&album, "needs_review", Some("provider_incomplete"), None)
            .unwrap();
        drop(store);

        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        assert_eq!(
            reopened.actionable_review_paths().unwrap(),
            vec![album.clone()]
        );
        reopened.mark_reviewed(&album).unwrap();
        assert!(reopened.actionable_review_paths().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
