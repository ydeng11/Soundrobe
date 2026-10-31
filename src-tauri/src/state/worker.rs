use rusqlite::{params, Connection, OptionalExtension};
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
CREATE TABLE IF NOT EXISTS worker_retries (
  path TEXT PRIMARY KEY,
  origin_run_id TEXT NOT NULL,
  attempt INTEGER NOT NULL,
  next_attempt_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS worker_lineage (path TEXT PRIMARY KEY, origin_run_id TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS worker_ai_circuits (run_id TEXT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS worker_evidence (
  path TEXT NOT NULL,
  run_id TEXT NOT NULL,
  evidence TEXT NOT NULL,
  PRIMARY KEY(path, run_id)
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
            if matches!(
                existing.as_deref(),
                Some("pending" | "running" | "deferred")
            ) {
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
                    "DELETE FROM worker_lineage WHERE path = ?1",
                    [storage_key(path)],
                )
                .map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "DELETE FROM worker_retries WHERE path = ?1",
                    [storage_key(path)],
                )
                .map_err(|error| error.to_string())?;
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
            .map(|status| matches!(status.as_str(), "pending" | "running" | "deferred"))
            .unwrap_or(false)
    }

    #[cfg(test)]
    fn pending(&self) -> Vec<PathBuf> {
        self.pending_result().unwrap_or_default()
    }

    pub fn pending_result(&self) -> Result<Vec<PathBuf>, String> {
        self.pending_at(now_epoch())
    }

    fn pending_at(&self, now: i64) -> Result<Vec<PathBuf>, String> {
        self.ensure_not_halted()?;
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let mut statement = connection
            .prepare(
                "SELECT path FROM worker_items WHERE status = 'pending'
                 OR (status = 'deferred' AND path IN
                   (SELECT path FROM worker_retries WHERE next_attempt_at <= ?1))
                 ORDER BY updated_at, path",
            )
            .map_err(|error| error.to_string())?;
        let paths = statement
            .query_map([now], |row| row.get::<_, String>(0))
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
                         WHERE path = ?1 AND status IN ('pending', 'running', 'deferred')",
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
                     WHERE path = ?1 AND status IN ('pending', 'running', 'deferred')",
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

    /// Bind every sibling before the first provider/AI request. Recovery keeps
    /// this lineage even when scheduling rows clear their transient run id.
    pub fn bind_run(&self, paths: &[PathBuf], run_id: &str) -> Result<(), String> {
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction()
            .map_err(|error| error.to_string())?;
        for path in paths {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO worker_lineage VALUES (?1, ?2)",
                    params![storage_key(path), run_id],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn retry_origin(&self, path: &Path) -> Result<Option<String>, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection
            .query_row(
                "SELECT origin_run_id FROM worker_lineage WHERE path = ?1 UNION ALL SELECT origin_run_id FROM worker_retries WHERE path = ?1 LIMIT 1",
                [storage_key(path)],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn defer_retry(
        &self,
        path: &Path,
        run_id: &str,
        retry_after: Option<u64>,
    ) -> Result<Option<i64>, String> {
        self.defer_retry_at(path, run_id, retry_after, now_epoch())
    }

    fn defer_retry_at(
        &self,
        path: &Path,
        run_id: &str,
        retry_after: Option<u64>,
        now: i64,
    ) -> Result<Option<i64>, String> {
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction()
            .map_err(|error| error.to_string())?;
        let due = Self::schedule_retry(&transaction, path, run_id, retry_after, now)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(due)
    }

    fn schedule_retry(
        transaction: &rusqlite::Transaction<'_>,
        path: &Path,
        run_id: &str,
        retry_after: Option<u64>,
        now: i64,
    ) -> Result<Option<i64>, String> {
        let key = storage_key(path);
        let previous: Option<(String, u32)> = transaction
            .query_row(
                "SELECT origin_run_id, attempt FROM worker_retries WHERE path = ?1",
                [&key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let (origin, attempt) = previous.unwrap_or_else(|| (run_id.to_owned(), 0));
        if attempt >= 3 {
            transaction.execute("UPDATE worker_items SET status = 'needs_review', reason = 'provider_retry_exhausted', updated_at = ?2 WHERE path = ?1", params![key, now]).map_err(|error| error.to_string())?;
            return Ok(None);
        }
        // Stable per-path jitter spreads restarts without relying on a model or clock RNG.
        let jitter = key
            .bytes()
            .fold(0u64, |sum, byte| sum.wrapping_add(u64::from(byte)))
            % 30;
        let delay = (60u64 * (1 << attempt)).min(3600) + jitter;
        let delay = delay.max(retry_after.unwrap_or(0)).min(i64::MAX as u64);
        let due = now.saturating_add(delay as i64);
        transaction
            .execute(
                "INSERT OR REPLACE INTO worker_retries VALUES (?1, ?2, ?3, ?4)",
                params![key, origin, attempt + 1, due],
            )
            .map_err(|error| error.to_string())?;
        transaction.execute("UPDATE worker_items SET status = 'deferred', reason = 'provider_retry_scheduled', updated_at = ?2 WHERE path = ?1",
            params![key, now]).map_err(|error| error.to_string())?;
        Ok(Some(due))
    }

    /// Scheduling, terminal state and retry diagnostics must survive as one unit.
    pub fn finish_deferred_with_evidence(
        &self,
        path: &Path,
        run_id: &str,
        origin: &str,
        retry_after: Option<u64>,
        authority: Option<&str>,
        evidence: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction()
            .map_err(|error| error.to_string())?;
        let changed = transaction
            .execute(
                "UPDATE worker_items SET run_id=?2, authority=?3 WHERE path=?1",
                params![storage_key(path), run_id, authority],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("worker completion has no scheduling row".into());
        }
        let due = Self::schedule_retry(&transaction, path, origin, retry_after, now_epoch())?;
        let mut evidence = evidence.clone();
        evidence
            .as_object_mut()
            .ok_or("diagnostic evidence is not an object")?
            .insert(
                "retry".into(),
                match due {
                    Some(due) => serde_json::json!({"nextAttemptAt":due}),
                    None => serde_json::json!({"exhausted":true}),
                },
            );
        transaction
            .execute(
                "INSERT OR REPLACE INTO worker_evidence VALUES (?1, ?2, ?3)",
                params![storage_key(path), run_id, evidence.to_string()],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(evidence)
    }

    pub fn ai_circuit_open(&self, run_id: &str) -> Result<bool, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM worker_ai_circuits WHERE run_id = ?1)",
                [run_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }

    pub fn open_ai_circuit(&self, run_id: &str) -> Result<(), String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection
            .execute(
                "INSERT OR IGNORE INTO worker_ai_circuits VALUES (?1)",
                [run_id],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn finish_with_evidence(
        &self,
        path: &Path,
        run_id: &str,
        status: &str,
        reason: Option<&str>,
        authority: Option<&str>,
        evidence: &serde_json::Value,
    ) -> Result<(), String> {
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction()
            .map_err(|error| error.to_string())?;
        let changed = transaction.execute("UPDATE worker_items SET status=?2, run_id=?3, reason=?4, authority=?5, updated_at=?6 WHERE path=?1", params![storage_key(path), status, run_id, reason, authority, now_epoch()]).map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("worker completion has no scheduling row".into());
        }
        transaction
            .execute(
                "INSERT OR REPLACE INTO worker_evidence VALUES (?1, ?2, ?3)",
                params![storage_key(path), run_id, evidence.to_string()],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn checkpoint_evidence(
        &self,
        path: &Path,
        run_id: &str,
        patch: &serde_json::Value,
    ) -> Result<(), String> {
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction()
            .map_err(|error| error.to_string())?;
        let previous: Option<String> = transaction
            .query_row(
                "SELECT evidence FROM worker_evidence WHERE path = ?1 AND run_id = ?2",
                params![storage_key(path), run_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let mut evidence = previous
            .map(|value| serde_json::from_str::<serde_json::Value>(&value))
            .transpose()
            .map_err(|error| error.to_string())?
            .unwrap_or_else(|| serde_json::json!({}));
        let object = evidence
            .as_object_mut()
            .ok_or("diagnostic evidence is not an object")?;
        object.extend(
            patch
                .as_object()
                .ok_or("diagnostic checkpoint is not an object")?
                .clone(),
        );
        transaction
            .execute(
                "INSERT OR REPLACE INTO worker_evidence VALUES (?1, ?2, ?3)",
                params![storage_key(path), run_id, evidence.to_string()],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }

    /// Keep each run's diagnostic snapshot independently of the scheduling row.
    pub fn record_evidence(
        &self,
        path: &Path,
        run_id: &str,
        evidence: &serde_json::Value,
    ) -> Result<(), String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection.execute(
            "INSERT OR REPLACE INTO worker_evidence(path, run_id, evidence) VALUES (?1, ?2, ?3)",
            params![storage_key(path), run_id, evidence.to_string()],
        ).map(|_| ()).map_err(|error| error.to_string())
    }

    pub fn run_evidence(&self, run_id: &str) -> Result<Vec<(String, serde_json::Value)>, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let mut statement = connection
            .prepare("SELECT path, evidence FROM worker_evidence WHERE run_id = ?1 ORDER BY path")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([run_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows.into_iter()
            .map(|(path, value)| {
                serde_json::from_str(&value)
                    .map(|value| (path, value))
                    .map_err(|error| error.to_string())
            })
            .collect()
    }

    pub fn evidence(&self, path: &Path, run_id: &str) -> Result<Option<serde_json::Value>, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let value: Option<String> = connection
            .query_row(
                "SELECT evidence FROM worker_evidence WHERE path = ?1 AND run_id = ?2",
                params![storage_key(path), run_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        value
            .map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
            .transpose()
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
                "SELECT path FROM worker_items WHERE status IN ('needs_review', 'deferred')
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
                 WHERE path IN (?1, ?2) AND status IN ('needs_review', 'deferred')",
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
    fn deferred_transient_retry_is_due_bounded_and_survives_restart() {
        let root = root();
        let album = root.join("Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        let due = store
            .defer_retry_at(&album, "origin", Some(500), 100)
            .unwrap()
            .unwrap();
        assert!(due >= 600, "never shorten Retry-After");
        assert!(store.pending_at(due - 1).unwrap().is_empty());
        drop(store);
        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        assert_eq!(reopened.pending_at(due).unwrap(), vec![album.clone()]);
        assert_eq!(
            reopened.retry_origin(&album).unwrap().as_deref(),
            Some("origin")
        );
        reopened
            .defer_retry_at(&album, "new-run", None, 1000)
            .unwrap()
            .unwrap();
        reopened
            .defer_retry_at(&album, "new-run", None, 2000)
            .unwrap()
            .unwrap();
        assert!(reopened
            .defer_retry_at(&album, "new-run", None, 3000)
            .unwrap()
            .is_none());
        reopened
            .requeue(std::slice::from_ref(&album), "operator")
            .unwrap();
        assert!(reopened.retry_origin(&album).unwrap().is_none());
        assert!(reopened
            .defer_retry_at(&album, "operator", None, 4000)
            .unwrap()
            .is_some());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn acknowledging_a_deferred_review_stops_its_automatic_retry() {
        let root = root();
        let album = root.join("Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        let due = store
            .defer_retry_at(&album, "run", None, 100)
            .unwrap()
            .unwrap();
        store.mark_reviewed(&album).unwrap();
        assert!(store.actionable_review_paths().unwrap().is_empty());
        assert!(
            store.pending_at(due).unwrap().is_empty(),
            "explicit acknowledgment must not rerun media later"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_initial_batch_retains_billing_lineage_for_running_and_pending_siblings() {
        let root = root();
        let first = root.join("First");
        let second = root.join("Second");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(&[first.clone(), second.clone()], "manual")
            .unwrap();
        store
            .bind_run(&[first.clone(), second.clone()], "initial-run")
            .unwrap();
        store.mark_running(&first, "initial-run").unwrap();
        store.open_ai_circuit("initial-run").unwrap();
        drop(store);
        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        reopened.recover_interrupted().unwrap();
        reopened
            .bind_run(&[first.clone(), second.clone()], "recovery-run")
            .unwrap();
        for path in [&first, &second] {
            let origin = reopened.retry_origin(path).unwrap().unwrap();
            assert_eq!(origin, "initial-run");
            assert!(reopened.ai_circuit_open(&origin).unwrap());
        }
        reopened
            .requeue(std::slice::from_ref(&first), "operator")
            .unwrap();
        assert!(reopened.retry_origin(&first).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn billing_circuit_is_durable_and_scoped_to_original_run() {
        let root = root();
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        assert!(!store.ai_circuit_open("run-1").unwrap());
        store.open_ai_circuit("run-1").unwrap();
        drop(store);
        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        assert!(reopened.ai_circuit_open("run-1").unwrap());
        assert!(!reopened.ai_circuit_open("explicit-new-run").unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transient_completion_commits_retry_status_and_evidence_atomically() {
        let root = root();
        let album = root.join("Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        store.mark_running(&album, "run").unwrap();
        {
            let guard = store.inner.lock().unwrap();
            guard.as_ref().unwrap().execute_batch("CREATE TRIGGER reject_evidence BEFORE INSERT ON worker_evidence BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
        }
        let evidence = serde_json::json!({"reasonCode":"provider_retryable"});
        assert!(store
            .finish_deferred_with_evidence(&album, "run", "origin", Some(300), None, &evidence)
            .is_err());
        {
            let guard = store.inner.lock().unwrap();
            let connection = guard.as_ref().unwrap();
            let status: String = connection
                .query_row("SELECT status FROM worker_items", [], |row| row.get(0))
                .unwrap();
            assert_eq!(
                status, "running",
                "failed completion must remain restart-recoverable"
            );
            let retries: u32 = connection
                .query_row("SELECT count(*) FROM worker_retries", [], |row| row.get(0))
                .unwrap();
            assert_eq!(retries, 0, "failed evidence must not consume retry budget");
            connection
                .execute_batch("DROP TRIGGER reject_evidence;")
                .unwrap();
        }
        let saved = store
            .finish_deferred_with_evidence(&album, "run", "origin", Some(300), None, &evidence)
            .unwrap();
        let due = saved["retry"]["nextAttemptAt"].as_i64().unwrap();
        assert!(due >= now_epoch() + 299);
        drop(store);
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        assert_eq!(store.evidence(&album, "run").unwrap(), Some(saved));
        assert!(store.pending_at(due - 1).unwrap().is_empty());
        assert_eq!(store.pending_at(due).unwrap(), vec![album.clone()]);
        for _ in 0..2 {
            store
                .finish_deferred_with_evidence(&album, "run", "origin", None, None, &evidence)
                .unwrap();
        }
        let exhausted = store
            .finish_deferred_with_evidence(&album, "run", "origin", None, None, &evidence)
            .unwrap();
        assert_eq!(exhausted["retry"]["exhausted"], true);
        assert_eq!(store.evidence(&album, "run").unwrap(), Some(exhausted));
        assert!(store.pending_at(i64::MAX).unwrap().is_empty());
        assert_eq!(store.actionable_review_paths().unwrap(), vec![album]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminal_outcome_and_evidence_commit_together_and_missing_rows_fail_loudly() {
        let root = root();
        let album = root.join("Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        let evidence = serde_json::json!({"reasonCode":"partial_collection"});
        assert!(store
            .finish_with_evidence(
                &album,
                "run",
                "needs_review",
                Some("partial_collection"),
                None,
                &evidence
            )
            .is_err());
        assert!(store.evidence(&album, "run").unwrap().is_none());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        store
            .finish_with_evidence(
                &album,
                "run",
                "needs_review",
                Some("partial_collection"),
                None,
                &evidence,
            )
            .unwrap();
        assert_eq!(store.evidence(&album, "run").unwrap(), Some(evidence));
        assert_eq!(store.actionable_review_paths().unwrap(), vec![album]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn album_evidence_survives_restart_and_keeps_run_history() {
        let root = root();
        let album = root.join("Artist/Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        let evidence = serde_json::json!({"providerAttempts":[{"provider":"musicbrainz","status":"unavailable"}], "coverage":{"localFiles":1,"releaseTracks":12}, "configuration":{"fingerprint":"safe"}});
        store.record_evidence(&album, "run-1", &evidence).unwrap();
        store
            .record_evidence(
                &album,
                "run-2",
                &serde_json::json!({"reason":"ambiguous_edition"}),
            )
            .unwrap();
        drop(store);
        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        assert_eq!(reopened.evidence(&album, "run-1").unwrap(), Some(evidence));
        assert_eq!(
            reopened.evidence(&album, "run-2").unwrap().unwrap()["reason"],
            "ambiguous_edition"
        );
        fs::remove_dir_all(root).unwrap();
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
