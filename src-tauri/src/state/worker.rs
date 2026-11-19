#[cfg(all(test, unix))]
use crate::infra::mock_run_authority::storage::files as fs;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
#[cfg(not(all(test, unix)))]
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use time::OffsetDateTime;

const WORKER_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS worker_items (
  path TEXT PRIMARY KEY,
  trigger TEXT NOT NULL,
  status TEXT NOT NULL,
  run_id TEXT,
  owner TEXT,
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
CREATE TABLE IF NOT EXISTS field_review_obligations (
  path TEXT NOT NULL,
  field TEXT NOT NULL,
  revision TEXT NOT NULL,
  before_value TEXT NOT NULL,
  after_value TEXT NOT NULL,
  provenance TEXT NOT NULL,
  evidence_ids TEXT NOT NULL,
  state TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  acknowledged_at INTEGER,
  PRIMARY KEY(path, field, revision)
);
CREATE TABLE IF NOT EXISTS write_intents (
  run_id TEXT NOT NULL,
  path TEXT NOT NULL,
  before_sha256 TEXT,
  before_payload_sha256 TEXT,
  after_sha256 TEXT,
  backup_path TEXT,
  patch TEXT NOT NULL,
  state TEXT NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(run_id, path)
);
CREATE TABLE IF NOT EXISTS authority_write_bindings (
  run_id TEXT NOT NULL,
  path TEXT NOT NULL,
  library_id TEXT NOT NULL,
  owner_profile_id TEXT NOT NULL,
  relative_path TEXT NOT NULL,
  PRIMARY KEY(run_id, path)
);
CREATE TABLE IF NOT EXISTS write_recovery_decisions (
  revision TEXT NOT NULL,
  path TEXT NOT NULL,
  action TEXT NOT NULL,
  observed_sha256 TEXT NOT NULL,
  decided_at INTEGER NOT NULL,
  PRIMARY KEY(revision, path)
);
CREATE TABLE IF NOT EXISTS reversal_manifests (
  revision TEXT PRIMARY KEY,
  album_path TEXT NOT NULL,
  manifest TEXT NOT NULL,
  status TEXT NOT NULL,
  owner_pid INTEGER,
  owner_token TEXT,
  updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS review_target_owners (
  sequence INTEGER PRIMARY KEY AUTOINCREMENT,
  path TEXT NOT NULL,
  revision TEXT NOT NULL,
  UNIQUE(path, revision)
);
CREATE TABLE IF NOT EXISTS review_target_writers (
  path TEXT PRIMARY KEY,
  revision TEXT NOT NULL,
  owner_pid INTEGER NOT NULL,
  owner_token TEXT NOT NULL
);
"#;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FieldReviewObligation {
    pub path: PathBuf,
    pub field: String,
    pub revision: String,
    pub before: Value,
    pub after: Value,
    pub provenance: Value,
    pub evidence_ids: Vec<String>,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WriteIntent {
    pub run_id: String,
    pub path: PathBuf,
    pub before_sha256: Option<String>,
    pub before_payload_sha256: Option<String>,
    pub after_sha256: Option<String>,
    pub backup_path: Option<PathBuf>,
    pub patch: Value,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteRecoveryAction {
    RetrySafe,
    ReceiptRecoverable,
    NeedsRecovery,
    Superseded,
    Reverted,
    KeptCurrent,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WriteIntentRecovery {
    pub intent: WriteIntent,
    pub action: WriteRecoveryAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

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
    /// Inventory must not initialize schemas, recover work, or start scheduling.
    /// A missing database means absent evidence, not a failed tagging attempt.
    pub fn read_only_history(data_dir: &Path) -> Result<Vec<serde_json::Value>, String> {
        let path = data_dir.join("worker.db");
        if !path.try_exists().map_err(|error| error.to_string())? {
            return Ok(Vec::new());
        }
        #[cfg(all(test, unix))]
        let connection = crate::infra::mock_run_authority::storage::open_sqlite(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|error| error.to_string())?;
        #[cfg(not(all(test, unix)))]
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT e.path, e.run_id, e.evidence, i.updated_at, i.run_id = e.run_id, i.status
             FROM worker_evidence e LEFT JOIN worker_items i ON e.path = i.path
             ORDER BY e.path, e.rowid",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<bool>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| {
            let (path, run_id, evidence, updated_at, current, status) =
                row.map_err(|error| error.to_string())?;
            let evidence: serde_json::Value =
                serde_json::from_str(&evidence).map_err(|error| error.to_string())?;
            Ok(
                serde_json::json!({"path":path,"runId":run_id,"evidence":evidence,
                "schedulingUpdatedAt":updated_at,"currentSchedulingRun":current.unwrap_or(false),"schedulingStatus":status}),
            )
        })
        .collect()
    }

    pub fn new_in(data_dir: PathBuf) -> Self {
        Self {
            path: data_dir.join("worker.db"),
            halt_path: data_dir.join("worker.halted.json"),
            inner: Arc::new(Mutex::new(None)),
            halted: Arc::new(Mutex::new(None)),
        }
    }

    pub fn data_dir(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new("."))
    }

    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn same_store(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
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
        #[cfg(all(test, unix))]
        let opened = crate::infra::mock_run_authority::storage::open_sqlite(
            &self.path,
            rusqlite::OpenFlags::default(),
        );
        #[cfg(not(all(test, unix)))]
        let opened = Connection::open(&self.path);
        let Ok(connection) = opened else {
            return false;
        };
        if connection.execute_batch(WORKER_SCHEMA).is_err() {
            return false;
        }
        let Ok(statement) = connection.prepare("SELECT * FROM worker_items LIMIT 0") else {
            return false;
        };
        let has_owner = statement.column_names().contains(&"owner");
        drop(statement);
        if !has_owner
            && connection
                .execute("ALTER TABLE worker_items ADD COLUMN owner TEXT", [])
                .is_err()
        {
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

    /// A desktop request may retry only its demonstrably abandoned process
    /// owner, never live desktop or unknown/server work. Retain old evidence.
    pub fn claim_desktop_run(&self, path: &Path, run_id: &str) -> Result<(), String> {
        self.ensure_not_halted()?;
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        require_clear_write_recovery(&transaction, path)?;
        let key = storage_key(path);
        let previous: Option<(String, Option<String>)> = transaction
            .query_row(
                "SELECT status, owner FROM worker_items WHERE path=?1",
                [&key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((status, owner)) = previous {
            if matches!(status.as_str(), "pending" | "running" | "deferred") {
                let pid = owner
                    .as_deref()
                    .and_then(|owner| owner.strip_prefix("desktop:"))
                    .and_then(|pid| pid.parse::<u32>().ok())
                    .filter(|pid| *pid > 0 && i32::try_from(*pid).is_ok());
                if pid.is_none_or(crate::commands::auto_tag_review::process_alive) {
                    return Err("album has live or unknown durable ownership; not reclaimed".into());
                }
            }
        }
        transaction
            .execute("DELETE FROM worker_lineage WHERE path=?1", [&key])
            .map_err(|e| e.to_string())?;
        transaction
            .execute("DELETE FROM worker_retries WHERE path=?1", [&key])
            .map_err(|e| e.to_string())?;
        transaction.execute("INSERT OR REPLACE INTO worker_items (path,trigger,status,run_id,owner,reason,authority,updated_at) VALUES (?1,'desktop','running',?2,?3,NULL,NULL,?4)",params![key,run_id,format!("desktop:{}",std::process::id()),now_epoch()]).map_err(|e|e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())
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
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        if let Some(halt) = &halt {
            for path in &halt.paths {
                transaction
                    .execute(
                        "UPDATE worker_items
                         SET status = 'cancelled', run_id = NULL, reason = ?2, updated_at = ?3
                         WHERE path = ?1 AND status IN ('pending', 'running', 'deferred')",
                        params![path, &halt.reason, now_epoch()],
                    )
                    .map_err(|error| halt_recovery_error(halt, error))?;
            }
        }
        let candidates = {
            let mut statement = transaction
                .prepare("SELECT path, owner FROM worker_items WHERE status='running'")
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        let mut recovered = 0;
        for (path, owner) in candidates {
            if let Some(owner) = owner {
                let pid = worker_owner_pid(&owner);
                if pid.is_none_or(crate::commands::auto_tag_review::process_alive) {
                    continue;
                }
            }
            recovered += transaction.execute("UPDATE worker_items SET status='pending',run_id=NULL,updated_at=?2 WHERE path=?1",params![path,now_epoch()]).map_err(|e|e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
        if let Some(halt) = &halt {
            fs::remove_file(&self.halt_path).map_err(|error| halt_recovery_error(halt, error))?;
            *self
                .halted
                .lock()
                .map_err(|_| "worker halt state unavailable")? = None;
        }
        Ok(recovered)
    }

    pub fn mark_running(&self, path: &Path, run_id: &str) -> Result<(), String> {
        self.ensure_not_halted()?;
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        require_clear_write_recovery(connection, path)?;
        let changed = connection.execute("UPDATE worker_items SET status='running',run_id=?2,owner=?3,reason=NULL,authority=NULL,updated_at=?4 WHERE path=?1 AND status IN ('pending','deferred')",params![storage_key(path),run_id,format!("server:{}",std::process::id()),now_epoch()]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("worker claim lost or album already running".into());
        }
        Ok(())
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
                "UPDATE worker_items SET run_id=?2, authority=?3 WHERE path=?1 AND (run_id=?2 OR (?4=0 AND run_id IS NULL))",
                params![storage_key(path), run_id, authority, evidence.get("proofVersion").is_some()],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("worker completion has no scheduling row".into());
        }
        let due = Self::schedule_retry(&transaction, path, origin, retry_after, now_epoch())?;
        let mut evidence = Self::merged_terminal_evidence(&transaction, path, run_id, evidence)?;
        Self::publish_core_receipts(path, run_id, false, &mut evidence)?;
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

    fn merged_terminal_evidence(
        transaction: &rusqlite::Transaction<'_>,
        path: &Path,
        run_id: &str,
        patch: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let previous: Option<String> = transaction
            .query_row(
                "SELECT evidence FROM worker_evidence WHERE path=?1 AND run_id=?2",
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
        let patch = patch
            .as_object()
            .ok_or("terminal evidence is not an object")?;
        let target = evidence
            .as_object_mut()
            .ok_or("diagnostic evidence is not an object")?;
        // Only the new terminal result may assert success. Progress from an
        // interrupted earlier implementation cannot supply a completion basis.
        for key in [
            "completionBasis",
            "outcome",
            "coreReceipts",
            "proofVersion",
            "proofPublication",
        ] {
            if !patch.contains_key(key) {
                target.remove(key);
            }
        }
        target.extend(patch.clone());
        Ok(evidence)
    }

    fn publish_core_receipts(
        path: &Path,
        run_id: &str,
        completed_core: bool,
        evidence: &mut serde_json::Value,
    ) -> Result<(), String> {
        use crate::commands::auto_tag::CoreReceipt;
        if evidence.get("proofVersion").is_none() {
            return Ok(());
        }
        if evidence["proofVersion"] != 1 {
            return Err("unsupported core proof version".into());
        }
        let folder = path.canonicalize().map_err(|e| e.to_string())?;
        let mut receipts: Vec<CoreReceipt> = serde_json::from_value(
            evidence
                .get("coreReceipts")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .map_err(|e| e.to_string())?;
        let mut paths = std::collections::BTreeSet::new();
        let mut attempt = None;
        for receipt in &mut receipts {
            let source = receipt.path().to_path_buf();
            if source.parent() != Some(folder.as_path())
                || !paths.insert(source.clone())
                || receipt.run_id.as_deref().is_some_and(|id| id != run_id)
                || attempt.as_ref().is_some_and(|id| id != &receipt.attempt_id)
            {
                return Err("core receipt ownership/coverage mismatch".into());
            }
            receipt.validate_current(&source)?;
            attempt = Some(receipt.attempt_id.clone());
            receipt.run_id = Some(run_id.into());
            receipt.published_at = Some(now_epoch());
        }
        if completed_core {
            let actual = crate::commands::library::collect_audio_files(&folder)
                .into_iter()
                .map(PathBuf::from)
                .collect::<std::collections::BTreeSet<_>>();
            if paths.is_empty() || paths != actual {
                return Err("core receipt physical coverage incomplete".into());
            }
        }
        evidence["coreReceipts"] = serde_json::to_value(receipts).map_err(|e| e.to_string())?;
        evidence["proofPublication"] =
            serde_json::json!({"version":1,"runId":run_id,"state":"terminal_validated"});
        Ok(())
    }

    pub fn finish_with_evidence(
        &self,
        path: &Path,
        run_id: &str,
        status: &str,
        reason: Option<&str>,
        authority: Option<&str>,
        evidence: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction()
            .map_err(|error| error.to_string())?;
        let mut evidence = Self::merged_terminal_evidence(&transaction, path, run_id, evidence)?;
        let verified_core = status == "completed"
            && evidence
                .get("completionBasis")
                .and_then(serde_json::Value::as_str)
                == Some("verified_track_core");
        Self::publish_core_receipts(path, run_id, verified_core, &mut evidence)?;
        // Failure reporting after a rejected finish must not reclaim a newer
        // run. Ordinary pending rows retain the existing unclaimed finish path.
        let changed = transaction.execute("UPDATE worker_items SET status=?2, run_id=?3, reason=?4, authority=?5, updated_at=?6 WHERE path=?1 AND (run_id = ?3 OR (?7 = 0 AND run_id IS NULL))", params![storage_key(path), status, run_id, reason, authority, now_epoch(), verified_core || evidence.get("proofVersion").is_some()]).map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("worker completion has no scheduling row".into());
        }
        transaction
            .execute(
                "INSERT OR REPLACE INTO worker_evidence VALUES (?1, ?2, ?3)",
                params![storage_key(path), run_id, evidence.to_string()],
            )
            .map_err(|error| error.to_string())?;
        if verified_core {
            // This is machine-verified track proof, not acknowledgement of an
            // AI or human obligation, or evidence from a different attempt.
            transaction.execute("DELETE FROM review_flags WHERE path=?1 AND run_id=?2 AND reason='recording_identity' AND reviewed_at IS NULL",
                params![storage_key(path), run_id]).map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(evidence)
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
        if [
            "coreReceipts",
            "proofVersion",
            "proofPublication",
            "completionBasis",
        ]
        .iter()
        .any(|key| patch.get(key).is_some())
        {
            return Err("terminal proof cannot be published by a checkpoint".into());
        }
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
                "INSERT INTO review_flags
                 (path, reason, run_id, created_at, reviewed_at)
                 VALUES (?1, ?2, ?3, ?4, NULL)
                 ON CONFLICT(path) DO UPDATE SET reason=excluded.reason,
                   run_id=excluded.run_id, created_at=excluded.created_at, reviewed_at=NULL
                 WHERE excluded.reason != 'recording_identity'
                   OR review_flags.reviewed_at IS NOT NULL
                   OR review_flags.reason = 'recording_identity'",
                params![storage_key(path), reason, run_id, now_epoch()],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Persist a field-specific review obligation without changing scheduling.
    /// A revision key is immutable so retries cannot silently rewrite an
    /// already acknowledged decision.
    pub fn record_field_review_obligation(
        &self,
        path: &Path,
        field: &str,
        revision: &str,
        before: &Value,
        after: &Value,
        provenance: &Value,
        evidence_ids: &[String],
    ) -> Result<(), String> {
        if field.trim().is_empty() || revision.trim().is_empty() {
            return Err("field review obligation requires a field and revision".into());
        }
        let key = recovery_key(path)?;
        let before = before.to_string();
        let after = after.to_string();
        let provenance = provenance.to_string();
        let evidence_ids = serde_json::to_string(evidence_ids).map_err(|e| e.to_string())?;
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection
            .execute(
                "INSERT OR IGNORE INTO field_review_obligations
                 (path, field, revision, before_value, after_value, provenance,
                  evidence_ids, state, created_at, acknowledged_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'open', ?8, NULL)",
                params![
                    key,
                    field,
                    revision,
                    before,
                    after,
                    provenance,
                    evidence_ids,
                    now_epoch()
                ],
            )
            .map_err(|e| e.to_string())?;
        let stored: (String, String, String, String) = connection
            .query_row(
                "SELECT before_value, after_value, provenance, evidence_ids
                 FROM field_review_obligations WHERE path=?1 AND field=?2 AND revision=?3",
                params![key, field, revision],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|e| e.to_string())?;
        if stored != (before, after, provenance, evidence_ids) {
            return Err("field review obligation conflicts with its stored revision".into());
        }
        Ok(())
    }

    pub fn field_review_obligations(
        &self,
        path: &Path,
    ) -> Result<Vec<FieldReviewObligation>, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let mut statement = connection
            .prepare(
                "SELECT path, field, revision, before_value, after_value, provenance,
                        evidence_ids, state
                 FROM field_review_obligations WHERE path=?1 ORDER BY revision, field",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([recovery_key(path)?], row_to_field_review)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Acknowledges exactly one revision-bound field. It never completes,
    /// cancels, or reschedules the corresponding worker item.
    pub fn acknowledge_field_review_obligation(
        &self,
        path: &Path,
        field: &str,
        revision: &str,
    ) -> Result<(), String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let key = recovery_key(path)?;
        let changed = connection
            .execute(
                "UPDATE field_review_obligations
                 SET state='acknowledged', acknowledged_at=?4
                 WHERE path=?1 AND field=?2 AND revision=?3 AND state='open'",
                params![key, field, revision, now_epoch()],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM field_review_obligations
                     WHERE path=?1 AND field=?2 AND revision=?3 AND state='acknowledged')",
                    params![key, field, revision],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !exists {
                return Err("field review obligation not found".into());
            }
        }
        Ok(())
    }

    pub(crate) fn verified_field_reviews(
        &self,
        revision: Option<&str>,
    ) -> Result<Vec<FieldReviewObligation>, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        Ok(load_verified_field_reviews(connection, revision)?
            .into_iter()
            .map(|(field, _)| field)
            .collect())
    }

    pub(crate) fn write_recovery_conflicts(&self) -> Result<Vec<(String, PathBuf)>, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let mut statement = connection
            .prepare(
                "SELECT run_id, path FROM write_intents
                      WHERE state IN ('needs_recovery', 'reverting') ORDER BY path",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| {
                let run_id: String = row.get(0)?;
                let path: String = row.get(1)?;
                Ok((
                    write_intent_revision(&run_id).to_owned(),
                    PathBuf::from(path),
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub(crate) fn recovery_file_digest(path: &Path) -> Option<String> {
        file_sha256(path).ok().flatten()
    }

    pub(crate) fn recovery_file_can_be_kept(
        &self,
        revision: &str,
        path: &Path,
    ) -> Result<bool, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let prefix = format!("{revision}::");
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM write_intents WHERE path=?1
               AND substr(run_id, 1, length(?2))=?2 AND state='needs_recovery')
             AND NOT EXISTS(SELECT 1 FROM write_intents WHERE path=?1 AND state='reverting')",
                params![storage_key(path), prefix],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
    }

    /// Record an explicit decision to preserve the file the user reviewed.
    /// The file and its backup remain untouched.
    pub(crate) fn keep_recovery_file(
        &self,
        revision: &str,
        path: &Path,
        expected_sha256: &str,
    ) -> Result<(), String> {
        if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid recovery file fingerprint".into());
        }
        let key = recovery_key(path)?;
        if Path::new(&key) != path {
            return Err("Recovery file path changed".into());
        }
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let owners = {
            let mut statement = transaction
                .prepare(
                    "SELECT DISTINCT i.owner FROM worker_items i JOIN write_intents w
                 ON substr(w.run_id, 1, length(i.run_id)+2)=i.run_id || '::'
                 WHERE w.path=?1 AND i.status='running'",
                )
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map([&key], |row| row.get::<_, Option<String>>(0))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        for owner in owners {
            if owner
                .as_deref()
                .and_then(worker_owner_pid)
                .is_none_or(crate::commands::auto_tag_review::process_alive)
            {
                return Err("Recovery is unavailable while a run may still be active".into());
            }
        }
        if file_sha256(path)?.as_deref() != Some(expected_sha256) {
            return Err("File changed since it was reviewed; refresh recovery details".into());
        }
        let prefix = format!("{revision}::");
        let reverting: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM write_intents WHERE path=?1 AND state='reverting')",
                [&key],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if reverting {
            return Err("An interrupted revert needs further recovery".into());
        }
        let changed = transaction
            .execute(
                "UPDATE write_intents SET state='kept_current', updated_at=?3
                 WHERE path=?1 AND substr(run_id, 1, length(?2))=?2
                   AND state='needs_recovery'",
                params![key, prefix, now_epoch()],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("Recovery item is no longer available".into());
        }
        transaction.execute(
            "INSERT INTO write_recovery_decisions (revision, path, action, observed_sha256, decided_at)
             VALUES (?1, ?2, 'kept_current', ?3, ?4)",
            params![revision, key, expected_sha256, now_epoch()],
        ).map_err(|e| e.to_string())?;
        transaction
            .execute(
                "UPDATE write_intents SET state='superseded', updated_at=?4
             WHERE path=?1 AND substr(run_id, 1, length(?2))=?2
               AND state='verified' AND after_sha256 != ?3",
                params![key, prefix, expected_sha256, now_epoch()],
            )
            .map_err(|e| e.to_string())?;
        transaction
            .execute(
                "UPDATE field_review_obligations SET state='dismissed', acknowledged_at=?3
                 WHERE path=?1 AND revision=?2 AND state='open'
                   AND NOT EXISTS (
                     SELECT 1 FROM write_intents w
                     WHERE w.path=?1 AND substr(w.run_id, 1, length(?2)+2)=?2 || '::'
                       AND w.state='verified' AND w.after_sha256=?4
                   )",
                params![key, revision, now_epoch(), expected_sha256],
            )
            .map_err(|e| e.to_string())?;
        if file_sha256(path)?.as_deref() != Some(expected_sha256) {
            return Err("File changed while resolving recovery".into());
        }
        transaction.commit().map_err(|e| e.to_string())
    }

    pub(crate) fn album_for_write_review(&self, revision: &str) -> Result<Option<PathBuf>, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection
            .query_row(
                "SELECT path FROM worker_items WHERE run_id=?1
                 UNION SELECT path FROM worker_evidence WHERE run_id=?1 LIMIT 1",
                [revision],
                |row| row.get::<_, String>(0).map(PathBuf::from),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub(crate) fn reversal_manifest(
        &self,
        revision: &str,
    ) -> Result<Option<(String, Value, String)>, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection
            .query_row(
                "SELECT album_path, manifest, status FROM reversal_manifests WHERE revision=?1",
                [revision],
                |row| Ok((row.get(0)?, parse_sql_json(row, 1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub(crate) fn copy_reversal_backup(
        &self,
        source: &Path,
        expected: &str,
    ) -> Result<PathBuf, String> {
        let directory = self.data_dir().join("reversal-recovery");
        private_recovery_directory(&directory, sync_directory)?;
        let backup = directory.join(format!("{}.bak", uuid::Uuid::new_v4()));
        fs::copy(source, &backup).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        fs::File::open(&backup)
            .and_then(|file| file.sync_all())
            .map_err(|e| e.to_string())?;
        if file_sha256(&backup)?.as_deref() != Some(expected) {
            return Err("Reversal backup failed verification".into());
        }
        sync_recovery_directory(&directory)?;
        Ok(backup)
    }

    pub(crate) fn sync_reversal_parent(path: &Path) -> Result<(), String> {
        sync_directory(path.parent().ok_or("Reversal target has no parent")?)
    }

    pub(crate) fn pending_reversals(&self) -> Result<Vec<String>, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let mut statement = connection.prepare(
            "SELECT revision FROM reversal_manifests WHERE status='pending' ORDER BY updated_at, revision"
        ).map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub(crate) fn create_reversal_manifest(
        &self,
        revision: &str,
        album: &str,
        manifest: &Value,
    ) -> Result<(), String> {
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let files = manifest["files"]
            .as_array()
            .ok_or("Invalid reversal manifest")?;
        for file in files {
            let path = file["target"].as_str().ok_or("Invalid reversal target")?;
            let writer: Option<u32> = transaction
                .query_row(
                    "SELECT owner_pid FROM review_target_writers WHERE path=?1",
                    [path],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if writer.is_some_and(crate::commands::auto_tag_review::process_alive) {
                return Err(format!("A live writer owns {path}"));
            }
            if writer.is_some() {
                transaction
                    .execute("DELETE FROM review_target_writers WHERE path=?1", [path])
                    .map_err(|e| e.to_string())?;
            }
            let owner: Option<String> = transaction.query_row(
                "SELECT revision FROM review_target_owners WHERE path=?1 ORDER BY sequence DESC LIMIT 1",
                [path], |row| row.get(0),
            ).optional().map_err(|e| e.to_string())?;
            if owner.as_deref() != Some(revision) {
                return Err(format!("A newer review owns {path}"));
            }
        }
        transaction.execute(
            "INSERT INTO reversal_manifests (revision, album_path, manifest, status, updated_at)
             VALUES (?1, ?2, ?3, 'pending', ?4)",
            params![revision, album, manifest.to_string(), now_epoch()],
        ).map_err(|e| e.to_string())?;
        transaction
            .execute(
                "UPDATE write_intents SET state='reverting', updated_at=?2
             WHERE substr(run_id, 1, length(?1)+2)=?1 || '::'
               AND state NOT IN ('reverted', 'kept_current')",
                params![revision, now_epoch()],
            )
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())
    }

    pub(crate) fn claim_reversal(&self, revision: &str, token: &str) -> Result<(), String> {
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (status, owner): (String, Option<u32>) = transaction
            .query_row(
                "SELECT status, owner_pid FROM reversal_manifests WHERE revision=?1",
                [revision],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if status != "pending" || owner.is_some_and(crate::commands::auto_tag_review::process_alive)
        {
            return Err("Reversal is complete or already owned by a live process".into());
        }
        transaction.execute(
            "UPDATE reversal_manifests SET owner_pid=?2, owner_token=?3, updated_at=?4 WHERE revision=?1",
            params![revision, std::process::id(), token, now_epoch()],
        ).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())
    }

    pub(crate) fn update_reversal(
        &self,
        revision: &str,
        token: &str,
        manifest: &Value,
        complete: bool,
    ) -> Result<(), String> {
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let changed = transaction
            .execute(
                "UPDATE reversal_manifests SET manifest=?3, status=?4, updated_at=?5
             WHERE revision=?1 AND owner_token=?2 AND owner_pid=?6 AND status='pending'",
                params![
                    revision,
                    token,
                    manifest.to_string(),
                    if complete { "completed" } else { "pending" },
                    now_epoch(),
                    std::process::id()
                ],
            )
            .map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("Reversal ownership was lost".into());
        }
        if complete {
            transaction
                .execute(
                    "UPDATE write_intents SET state='reverted', updated_at=?2
                 WHERE substr(run_id, 1, length(?1)+2)=?1 || '::' AND state='reverting'",
                    params![revision, now_epoch()],
                )
                .map_err(|e| e.to_string())?;
            transaction
                .execute(
                    "DELETE FROM review_target_owners WHERE revision=?1",
                    [revision],
                )
                .map_err(|e| e.to_string())?;
            transaction.execute(
                "UPDATE field_review_obligations SET state='reverted', acknowledged_at=?2 WHERE revision=?1 AND state='open'",
                params![revision, now_epoch()],
            ).map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())
    }

    pub(crate) fn release_reversal(&self, revision: &str, token: &str) -> Result<(), String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection
            .execute(
                "UPDATE reversal_manifests SET owner_pid=NULL, owner_token=NULL
             WHERE revision=?1 AND owner_token=?2 AND owner_pid=?3",
                params![revision, token, std::process::id()],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub(crate) fn register_review_target(
        &self,
        revision: &str,
        target: &Path,
        token: &str,
    ) -> Result<bool, String> {
        self.register_target(revision, target, token, false)
    }

    pub(crate) fn register_ordinary_target(
        &self,
        revision: &str,
        target: &Path,
        token: &str,
    ) -> Result<bool, String> {
        self.register_target(revision, target, token, true)
    }

    fn register_target(
        &self,
        revision: &str,
        target: &Path,
        token: &str,
        ordinary: bool,
    ) -> Result<bool, String> {
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let key = storage_key(target);
        if ordinary {
            let unresolved: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM write_intents WHERE path=?1 AND state IN ('prepared', 'applying', 'written', 'partial', 'needs_recovery', 'reverting'))",
                [&key], |row| row.get(0),
            ).map_err(|e| e.to_string())?;
            if unresolved {
                return Err(format!("An unresolved write needs recovery for {key}"));
            }
        }
        let writer: Option<u32> = transaction
            .query_row(
                "SELECT owner_pid FROM review_target_writers WHERE path=?1",
                [&key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if writer.is_some_and(crate::commands::auto_tag_review::process_alive) {
            return Err(format!("A live writer owns {key}"));
        }
        if writer.is_some() {
            transaction
                .execute("DELETE FROM review_target_writers WHERE path=?1", [&key])
                .map_err(|e| e.to_string())?;
        }
        let active = {
            let mut statement = transaction
                .prepare("SELECT manifest FROM reversal_manifests WHERE status='pending'")
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map([], |row| parse_sql_json::<Value>(row, 0))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        if active.iter().any(|manifest| {
            manifest["files"].as_array().is_some_and(|files| {
                files
                    .iter()
                    .any(|file| file["target"].as_str() == Some(&key))
            })
        }) {
            return Err(format!("A pending reversal owns {key}"));
        }
        let latest: Option<String> = transaction.query_row(
            "SELECT revision FROM review_target_owners WHERE path=?1 ORDER BY sequence DESC LIMIT 1",
            [&key], |row| row.get(0),
        ).optional().map_err(|e| e.to_string())?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM review_target_owners WHERE path=?1 AND revision=?2)",
                params![key, revision],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if exists && latest.as_deref() != Some(revision) {
            return Err(format!("A newer review owns {key}"));
        }
        let inserted = if ordinary && latest.is_none() {
            0
        } else {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO review_target_owners(path, revision) VALUES (?1, ?2)",
                    params![key, revision],
                )
                .map_err(|e| e.to_string())?
        };
        transaction.execute(
            "INSERT INTO review_target_writers(path, revision, owner_pid, owner_token) VALUES (?1, ?2, ?3, ?4)",
            params![key, revision, std::process::id(), token],
        ).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(inserted == 1)
    }

    pub(crate) fn release_review_target(
        &self,
        revision: &str,
        target: &Path,
        token: &str,
        discard_owner: bool,
    ) -> Result<(), String> {
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let key = storage_key(target);
        let removed = transaction.execute(
            "DELETE FROM review_target_writers WHERE path=?1 AND revision=?2 AND owner_pid=?3 AND owner_token=?4",
            params![key, revision, std::process::id(), token],
        ).map_err(|e| e.to_string())?;
        if removed != 1 {
            return Err("Target write ownership was lost".into());
        }
        if discard_owner {
            transaction
                .execute(
                    "DELETE FROM review_target_owners WHERE path=?1 AND revision=?2",
                    params![key, revision],
                )
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())
    }

    /// Persist a review decision only if the exact verified files still match.
    /// This transaction has no effect on worker scheduling or media.
    pub(crate) fn acknowledge_verified_field_reviews(&self, revision: &str) -> Result<(), String> {
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        for (review, expected) in load_verified_field_reviews(&transaction, Some(revision))? {
            if review.state != "open" {
                continue;
            }
            if file_sha256(&review.path)?.as_deref() != Some(&expected) {
                return Err(format!(
                    "File changed since auto-tag: {}",
                    review.path.display()
                ));
            }
            transaction
                .execute(
                    "UPDATE field_review_obligations SET state='acknowledged', acknowledged_at=?4
                     WHERE path=?1 AND field=?2 AND revision=?3 AND state='open'",
                    params![
                        storage_key(&review.path),
                        review.field,
                        revision,
                        now_epoch()
                    ],
                )
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())
    }

    /// Bind a prepared intent to the nominated library and relative target.
    #[cfg(unix)]
    pub fn bind_write_authority(
        &self,
        path: &Path,
        run_id: &str,
        lease: &crate::state::library_authority::AuthorityLease,
        relative: &Path,
    ) -> Result<(), String> {
        self.bind_write_authority_with(path, run_id, lease, relative, || {})
    }

    #[cfg(unix)]
    fn bind_write_authority_with(
        &self,
        path: &Path,
        run_id: &str,
        lease: &crate::state::library_authority::AuthorityLease,
        relative: &Path,
        after_check: impl FnOnce(),
    ) -> Result<(), String> {
        self.bind_write_authority_with_hooks(path, run_id, lease, relative, after_check, || {})
    }

    #[cfg(unix)]
    fn bind_write_authority_with_hooks(
        &self,
        path: &Path,
        run_id: &str,
        lease: &crate::state::library_authority::AuthorityLease,
        relative: &Path,
        after_check: impl FnOnce(),
        before_commit: impl FnOnce(),
    ) -> Result<(), String> {
        let (library_id, profile_id, canonical) = lease
            .binding_identity(path, relative)
            .map_err(|error| error.to_string())?;
        let relative_text = relative
            .to_str()
            .ok_or("authority target is not valid UTF-8")?;
        let key = canonical
            .to_str()
            .ok_or("authority path is not valid UTF-8")?
            .to_string();
        after_check();
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        if lease
            .binding_identity(path, relative)
            .map_err(|error| error.to_string())?
            != (library_id.clone(), profile_id.clone(), canonical.clone())
        {
            return Err("write authority changed while waiting for the database".into());
        }
        let intent =
            load_write_intent(&transaction, run_id, &key)?.ok_or("write intent not found")?;
        if !matches!(intent.state.as_str(), "prepared" | "retry_safe") {
            return Err("write authority can only bind before applying".into());
        }
        transaction.execute(
            "INSERT OR IGNORE INTO authority_write_bindings (run_id, path, library_id, owner_profile_id, relative_path) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, key, library_id, profile_id, relative_text],
        ).map_err(|error| error.to_string())?;
        let stored: (String, String, String) = transaction.query_row(
            "SELECT library_id, owner_profile_id, relative_path FROM authority_write_bindings WHERE run_id=?1 AND path=?2",
            params![run_id, key], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).map_err(|error| error.to_string())?;
        if stored
            != (
                library_id.clone(),
                profile_id.clone(),
                relative_text.to_string(),
            )
        {
            return Err("write intent is bound to another authority".into());
        }
        if lease
            .binding_identity(path, relative)
            .map_err(|error| error.to_string())?
            != (library_id.clone(), profile_id.clone(), canonical.clone())
        {
            return Err("write authority changed before binding".into());
        }
        before_commit();
        transaction.commit().map_err(|error| error.to_string())?;
        drop(guard);
        match lease.binding_identity(path, relative) {
            Ok(current) if current == (library_id, profile_id, canonical) => Ok(()),
            Ok(_) => Err("binding persisted but authority changed during commit".into()),
            Err(error) => Err(format!(
                "binding persisted but authority could not be verified after commit: {error}"
            )),
        }
    }

    /// Refuse legacy or foreign-authority intents when an authorized resume is requested.
    #[cfg(unix)]
    pub fn verify_write_authority(
        &self,
        path: &Path,
        run_id: &str,
        lease: &crate::state::library_authority::AuthorityLease,
        relative: &Path,
    ) -> Result<(), String> {
        self.verify_write_authority_with(path, run_id, lease, relative, || {})
    }

    /// Validate the staged audio and patch against a bound write intent before publication.
    #[cfg(unix)]
    pub fn validate_authority_staged_write(
        &self,
        path: &Path,
        run_id: &str,
        relative: &Path,
        stage: &crate::state::authority_io::PrivateStage,
    ) -> Result<String, String> {
        self.validate_authority_staged_write_with(path, run_id, relative, stage, || {})
    }

    #[cfg(unix)]
    fn validate_authority_staged_write_with(
        &self,
        path: &Path,
        run_id: &str,
        relative: &Path,
        stage: &crate::state::authority_io::PrivateStage,
        after_receipt: impl FnOnce(),
    ) -> Result<String, String> {
        self.verify_write_authority(path, run_id, stage.lease(), relative)?;
        let (staged_path, digest) = stage.candidate(path).map_err(|error| error.to_string())?;
        let (_, _, canonical) = stage
            .lease()
            .binding_identity(path, relative)
            .map_err(|error| error.to_string())?;
        let key = canonical
            .to_str()
            .ok_or("authority path is not valid UTF-8")?
            .to_string();
        let mut intent = {
            let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
            let connection = guard.as_ref().ok_or("worker state not initialized")?;
            load_write_intent(connection, run_id, &key)?.ok_or("write intent not found")?
        };
        if !matches!(
            intent.state.as_str(),
            "prepared" | "retry_safe" | "applying"
        ) {
            return Err("write intent is not eligible for staged validation".into());
        }
        if intent.before_sha256 != stage.source_digest() || intent.before_sha256.is_none() {
            return Err("staged source differs from write intent pre-state".into());
        }
        validate_recovery_backup(&intent, self.data_dir())?;
        let original_intent = intent.clone();
        intent.path = staged_path.clone();
        intent.after_sha256 = Some(digest.clone());
        if !valid_write_receipt(&intent) {
            return Err("staged write failed metadata or audio validation".into());
        }
        after_receipt();
        if stage.candidate(path).map_err(|error| error.to_string())?
            != (staged_path, digest.clone())
        {
            return Err("staged write changed during validation".into());
        }
        self.verify_write_authority(path, run_id, stage.lease(), relative)?;
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        if load_write_intent(connection, run_id, &key)? != Some(original_intent.clone()) {
            return Err("write intent changed during staged validation".into());
        }
        if stage
            .lease()
            .binding_identity(path, relative)
            .map_err(|error| error.to_string())?
            .2
            != canonical
        {
            return Err("write authority target changed during staged validation".into());
        }
        validate_recovery_backup(&original_intent, self.data_dir())?;
        Ok(digest)
    }

    #[cfg(unix)]
    fn verify_write_authority_with(
        &self,
        path: &Path,
        run_id: &str,
        lease: &crate::state::library_authority::AuthorityLease,
        relative: &Path,
        after_check: impl FnOnce(),
    ) -> Result<(), String> {
        let (library_id, profile_id, canonical) = lease
            .binding_identity(path, relative)
            .map_err(|error| error.to_string())?;
        let relative_text = relative
            .to_str()
            .ok_or("authority target is not valid UTF-8")?;
        let key = canonical
            .to_str()
            .ok_or("authority path is not valid UTF-8")?
            .to_string();
        after_check();
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        if lease
            .binding_identity(path, relative)
            .map_err(|error| error.to_string())?
            != (library_id.clone(), profile_id.clone(), canonical.clone())
        {
            return Err("write authority changed while waiting for the database".into());
        }
        let stored: Option<(String, String, String)> = connection.query_row(
            "SELECT library_id, owner_profile_id, relative_path FROM authority_write_bindings WHERE run_id=?1 AND path=?2",
            params![run_id, key], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional().map_err(|error| error.to_string())?;
        if stored
            != Some((
                library_id.clone(),
                profile_id.clone(),
                relative_text.to_string(),
            ))
        {
            return Err("write intent has no matching authority binding".into());
        }
        if lease
            .binding_identity(path, relative)
            .map_err(|error| error.to_string())?
            != (library_id, profile_id, canonical)
        {
            return Err("write authority changed during verification".into());
        }
        Ok(())
    }

    #[cfg(unix)]
    pub(crate) fn pending_marker_writes(&self) -> Result<Vec<(String, PathBuf, PathBuf)>, String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let mut statement = connection
            .prepare(
                "SELECT w.run_id, w.path, b.relative_path FROM write_intents w
             JOIN authority_write_bindings b ON b.run_id=w.run_id AND b.path=w.path
             WHERE w.state IN ('prepared', 'applying', 'written', 'partial', 'needs_recovery')
             ORDER BY w.path, w.run_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    PathBuf::from(row.get::<_, String>(1)?),
                    PathBuf::from(row.get::<_, String>(2)?),
                ))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    /// Reconcile one bound intent only while its nominated authority is held.
    #[cfg(unix)]
    pub fn reconcile_write_intent_authorized(
        &self,
        path: &Path,
        run_id: &str,
        lease: &crate::state::library_authority::AuthorityLease,
        relative: &Path,
    ) -> Result<String, String> {
        self.reconcile_write_intent_authorized_with(path, run_id, lease, relative, || {})
    }

    #[cfg(unix)]
    fn reconcile_write_intent_authorized_with(
        &self,
        path: &Path,
        run_id: &str,
        lease: &crate::state::library_authority::AuthorityLease,
        relative: &Path,
        before_commit: impl FnOnce(),
    ) -> Result<String, String> {
        let (library_id, profile_id, canonical) = lease
            .binding_identity(path, relative)
            .map_err(|error| error.to_string())?;
        let key = canonical
            .to_str()
            .ok_or("authority path is not valid UTF-8")?
            .to_string();
        let relative_text = relative
            .to_str()
            .ok_or("authority target is not valid UTF-8")?;
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        if lease
            .binding_identity(path, relative)
            .map_err(|error| error.to_string())?
            != (library_id.clone(), profile_id.clone(), canonical.clone())
        {
            return Err("write authority changed while waiting for reconciliation".into());
        }
        let binding: Option<(String, String, String)> = transaction.query_row(
            "SELECT library_id, owner_profile_id, relative_path FROM authority_write_bindings WHERE run_id=?1 AND path=?2",
            params![run_id, key], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional().map_err(|error| error.to_string())?;
        if binding
            != Some((
                library_id.clone(),
                profile_id.clone(),
                relative_text.to_string(),
            ))
        {
            return Err("write intent has no matching authority binding".into());
        }
        let mut intent =
            load_write_intent(&transaction, run_id, &key)?.ok_or("write intent not found")?;
        if !matches!(
            intent.state.as_str(),
            "prepared"
                | "applying"
                | "written"
                | "partial"
                | "needs_recovery"
                | "retry_safe"
                | "verified"
        ) {
            return Err("write intent is not eligible for reconciliation".into());
        }
        if matches!(
            intent.state.as_str(),
            "prepared" | "applying" | "written" | "partial" | "needs_recovery"
        ) {
            if let Some(state) = reconciled_write_state(&transaction, &intent)? {
                intent.state = state.into();
                save_write_intent(&transaction, &intent)?;
            }
        }
        if lease
            .binding_identity(path, relative)
            .map_err(|error| error.to_string())?
            != (library_id.clone(), profile_id.clone(), canonical.clone())
        {
            return Err("write authority changed during reconciliation".into());
        }
        let state = intent.state;
        before_commit();
        transaction.commit().map_err(|error| error.to_string())?;
        drop(guard);
        match lease.binding_identity(path, relative) {
            Ok(current) if current == (library_id, profile_id, canonical) => Ok(state),
            Ok(_) => Err("reconciliation persisted but authority changed during commit".into()),
            Err(error) => Err(format!("reconciliation persisted but authority could not be verified after commit: {error}")),
        }
    }

    /// Capture a private, synced before-state and its intent before mutation.
    /// The caller must mark the intent applying before writing. File hashes
    /// alone never establish a validated receipt.
    pub fn prepare_write_intent(
        &self,
        path: &Path,
        run_id: &str,
        patch: &Value,
    ) -> Result<WriteIntent, String> {
        self.prepare_write_intent_with_parent_sync(path, run_id, patch, sync_directory)
    }

    fn prepare_write_intent_with_parent_sync(
        &self,
        path: &Path,
        run_id: &str,
        patch: &Value,
        sync_parent_directory: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<WriteIntent, String> {
        if run_id.trim().is_empty() || !patch.is_object() {
            return Err("write intent requires a run ID and object patch".into());
        }
        let before_sha256 = file_sha256(path)?;
        let before_payload_sha256 = if crate::commands::library::is_audio_file(path) {
            Some(
                crate::commands::mutations::track_payload_digest(path)
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        let key = recovery_key(path)?;
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        if let Some(existing) = load_write_intent(connection, run_id, &key)? {
            if existing.patch != *patch {
                return Err("write intent patch conflicts with its stored revision".into());
            }
            return Ok(existing);
        }
        let backup_path = if before_sha256.is_some() {
            let directory = self.data_dir().join("write-recovery");
            private_recovery_directory(&directory, sync_parent_directory)?;
            let backup = directory.join(format!("{}.bak", uuid::Uuid::new_v4()));
            fs::copy(path, &backup).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o600))
                    .map_err(|e| e.to_string())?;
            }
            fs::File::open(&backup)
                .and_then(|file| file.sync_all())
                .map_err(|e| e.to_string())?;
            if file_sha256(&backup)? != before_sha256 || file_sha256(path)? != before_sha256 {
                let _ = fs::remove_file(&backup);
                return Err("media file changed while preparing its recovery backup".into());
            }
            sync_recovery_directory(&directory)?;
            Some(backup)
        } else {
            None
        };
        let intent = WriteIntent {
            run_id: run_id.into(),
            path: PathBuf::from(key.clone()),
            before_sha256,
            before_payload_sha256,
            after_sha256: None,
            backup_path,
            patch: patch.clone(),
            state: "prepared".into(),
        };
        let inserted = connection.execute(
            "INSERT INTO write_intents
             (run_id, path, before_sha256, before_payload_sha256, after_sha256,
              backup_path, patch, state, updated_at)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, 'prepared', ?7)",
            params![
                intent.run_id,
                key,
                intent.before_sha256,
                intent.before_payload_sha256,
                intent.backup_path.as_deref().map(Path::to_string_lossy),
                intent.patch.to_string(),
                now_epoch()
            ],
        );
        if let Err(error) = inserted {
            if let Some(backup) = &intent.backup_path {
                let _ = fs::remove_file(backup);
            }
            return Err(error.to_string());
        }
        Ok(intent)
    }

    pub fn begin_write_intent(&self, path: &Path, run_id: &str) -> Result<(), String> {
        let current = file_sha256(path)?;
        let key = recovery_key(path)?;
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let intent =
            load_write_intent(connection, run_id, &key)?.ok_or("write intent not found")?;
        if current != intent.before_sha256 {
            return Err("write intent pre-state changed; recovery is required".into());
        }
        match intent.state.as_str() {
            "prepared" | "retry_safe" => {
                connection
                    .execute(
                        "UPDATE write_intents SET state='applying', updated_at=?3
                         WHERE run_id=?1 AND path=?2 AND state IN ('prepared', 'retry_safe')",
                        params![run_id, key, now_epoch()],
                    )
                    .map_err(|e| e.to_string())?;
            }
            "applying" => {}
            _ => return Err("write intent is not ready to apply".into()),
        }
        Ok(())
    }

    pub fn record_write_intent_applied(
        &self,
        path: &Path,
        run_id: &str,
        write_succeeded: bool,
    ) -> Result<(), String> {
        let after_sha256 = file_sha256(path)?;
        let key = recovery_key(path)?;
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let state = if write_succeeded {
            "written"
        } else {
            "partial"
        };
        let mut intent =
            load_write_intent(connection, run_id, &key)?.ok_or("write intent not found")?;
        if intent.state == state && intent.after_sha256 == after_sha256 {
            return Ok(());
        }
        if intent.state != "applying" {
            return Err("write intent cannot record this post-state".into());
        }
        intent.after_sha256 = after_sha256;
        intent.state = state.into();
        save_write_intent(connection, &intent)
    }

    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn finalize_expected_write_intent(
        &self,
        path: &Path,
        run_id: &str,
        expected: &str,
        verify_target: impl FnOnce() -> std::io::Result<()>,
    ) -> Result<(), String> {
        let key = recovery_key(path)?;
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let mut intent =
            load_write_intent(connection, run_id, &key)?.ok_or("write intent not found")?;
        if intent.state != "applying" {
            return Err("write intent cannot record this post-state".into());
        }
        intent.after_sha256 = Some(expected.to_string());
        // Leave the durable intent applying until every readback check passes.
        // A restart before this checkpoint must inspect the media again.
        let verification = if valid_write_receipt(&intent) {
            verify_target().map_err(|error| error.to_string())
        } else {
            Err("published file differs from the validated receipt".into())
        };
        intent.state = if verification.is_ok() {
            "verified"
        } else {
            "needs_recovery"
        }
        .into();
        save_write_intent(connection, &intent)?;
        verification
    }

    pub fn verify_write_intent(&self, path: &Path, run_id: &str) -> Result<(), String> {
        let key = recovery_key(path)?;
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        let mut intent =
            load_write_intent(connection, run_id, &key)?.ok_or("write intent not found")?;
        if intent.state != "written" {
            return Err("write intent is not awaiting validation".into());
        }
        intent.state = if valid_write_receipt(&intent) {
            "verified"
        } else {
            "needs_recovery"
        }
        .into();
        save_write_intent(connection, &intent)?;
        if intent.state == "verified" {
            Ok(())
        } else {
            Err("write intent success/readback validation failed".into())
        }
    }

    /// Settle abandoned intent states before accepting new auto-tag work.
    /// Known post-state can be verified, while an unknown write or reversal
    /// requires review. The transaction only updates SQLite.
    pub fn reconcile_writes_at_startup(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let transaction = guard
            .as_mut()
            .ok_or("worker state not initialized")?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let intents = {
            let mut statement = transaction
                .prepare("SELECT run_id, path, before_sha256, before_payload_sha256, after_sha256,
                                 backup_path, patch, state FROM write_intents
                          WHERE state IN ('prepared', 'applying', 'written', 'partial', 'needs_recovery')
                            AND NOT EXISTS (
                                SELECT 1 FROM authority_write_bindings b
                                WHERE b.run_id=write_intents.run_id AND b.path=write_intents.path
                            )")
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map([], row_to_write_intent)
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        for mut intent in intents {
            if let Some(state) = reconciled_write_state(&transaction, &intent)? {
                intent.state = state.into();
                save_write_intent(&transaction, &intent)?;
            }
        }
        transaction.commit().map_err(|e| e.to_string())
    }

    pub fn supersede_prior_write_intents(
        &self,
        path: &Path,
        run_id_prefix: &str,
        current_run_id: &str,
    ) -> Result<(), String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection
            .execute(
                "UPDATE write_intents SET state='superseded', updated_at=?4
                 WHERE path=?1 AND state IN ('written', 'verified') AND run_id != ?3
                   AND substr(run_id, 1, length(?2)) = ?2",
                params![
                    recovery_key(path)?,
                    run_id_prefix,
                    current_run_id,
                    now_epoch()
                ],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn mark_review_write_intents_reverting(
        &self,
        path: &Path,
        run_id_prefix: &str,
    ) -> Result<(), String> {
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection
            .execute(
                "UPDATE write_intents SET state='reverting', updated_at=?3
                 WHERE path=?1 AND state NOT IN ('reverting', 'reverted')
                   AND substr(run_id, 1, length(?2)) = ?2",
                params![recovery_key(path)?, run_id_prefix, now_epoch()],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn mark_review_write_intents_reverted(
        &self,
        path: &Path,
        run_id_prefix: &str,
    ) -> Result<(), String> {
        let key = recovery_key(path)?;
        let target = PathBuf::from(&key);
        let parent = target.parent().ok_or("recovery target has no parent")?;
        sync_directory(parent)?;
        let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
        let connection = guard.as_ref().ok_or("worker state not initialized")?;
        connection
            .execute(
                "UPDATE write_intents SET state='reverted', updated_at=?3
                 WHERE path=?1 AND state='reverting'
                   AND substr(run_id, 1, length(?2)) = ?2",
                params![key, run_id_prefix, now_epoch()],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Classify interrupted writes without rewriting media. `backup_path` is
    /// retained for an operator recovery when neither known state matches.
    pub fn reconcile_write_intents(&self) -> Result<Vec<WriteIntentRecovery>, String> {
        let intents = {
            let guard = self.inner.lock().map_err(|_| "worker state unavailable")?;
            let connection = guard.as_ref().ok_or("worker state not initialized")?;
            let mut statement = connection
                .prepare("SELECT run_id, path, before_sha256, before_payload_sha256, after_sha256, backup_path, patch, state FROM write_intents ORDER BY updated_at, path")
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map([], row_to_write_intent)
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        Ok(intents
            .into_iter()
            .map(|intent| {
                let (action, error) = match intent.state.as_str() {
                    "superseded" => (WriteRecoveryAction::Superseded, None),
                    "reverted" => (WriteRecoveryAction::Reverted, None),
                    "kept_current" => (WriteRecoveryAction::KeptCurrent, None),
                    state => match file_sha256(&intent.path) {
                        Err(error) => (WriteRecoveryAction::NeedsRecovery, Some(error)),
                        Ok(current) => {
                            let action = match state {
                                "verified" if current == intent.after_sha256 => {
                                    WriteRecoveryAction::ReceiptRecoverable
                                }
                                "prepared" | "applying" | "retry_safe"
                                    if current == intent.before_sha256 =>
                                {
                                    WriteRecoveryAction::RetrySafe
                                }
                                "retry_safe" => WriteRecoveryAction::Superseded,
                                _ => WriteRecoveryAction::NeedsRecovery,
                            };
                            let error = if action == WriteRecoveryAction::NeedsRecovery {
                                Some(if current.is_none() {
                                    "recovery target is missing".into()
                                } else {
                                    format!("intent state {state} does not match the current file")
                                })
                            } else {
                                None
                            };
                            (action, error)
                        }
                    },
                };
                WriteIntentRecovery {
                    intent,
                    action,
                    error,
                }
            })
            .collect())
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

fn private_recovery_directory(
    path: &Path,
    sync_parent_directory: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("recovery directory must be a real directory".into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|e| e.to_string())?;
        }
        Err(error) => return Err(error.to_string()),
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("recovery directory changed to a symlink or non-directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let parent = path
        .parent()
        .ok_or("recovery directory has no data directory parent")?;
    sync_parent_directory(parent)
        .map_err(|error| format!("could not sync recovery directory parent: {error}"))?;
    Ok(())
}

#[cfg(not(windows))]
fn sync_directory(path: &Path) -> Result<(), String> {
    std::fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| format!("could not sync directory {}: {e}", path.display()))
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> Result<(), String> {
    use std::os::windows::fs::OpenOptionsExt;
    let directory = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|e| {
            format!(
                "could not open directory {} for durable sync: {e}",
                path.display()
            )
        })?;
    directory
        .sync_all()
        .map_err(|e| format!("could not sync directory {}: {e}", path.display()))
}

fn sync_recovery_directory(path: &Path) -> Result<(), String> {
    sync_directory(path).map_err(|e| format!("could not sync recovery directory: {e}"))
}

fn recovery_key(path: &Path) -> Result<String, String> {
    let canonical = match fs::metadata(path) {
        Ok(_) => fs::canonicalize(path),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or("per-file recovery path has no parent")?;
            let name = path
                .file_name()
                .ok_or("per-file recovery path has no filename")?;
            fs::canonicalize(parent).map(|parent| parent.join(name))
        }
        Err(error) => return Err(error.to_string()),
    }
    .map_err(|e| e.to_string())?;
    Ok(storage_key(&canonical))
}

fn worker_owner_pid(owner: &str) -> Option<u32> {
    let (runtime, pid) = owner.split_once(':')?;
    if !matches!(runtime, "desktop" | "server") {
        return None;
    }
    pid.parse()
        .ok()
        .filter(|pid| *pid > 0 && i32::try_from(*pid).is_ok())
}

fn write_intent_revision(run_id: &str) -> &str {
    run_id
        .split_once("::")
        .map_or(run_id, |(revision, _)| revision)
}

fn require_clear_write_recovery(connection: &Connection, album: &Path) -> Result<(), String> {
    let album = fs::canonicalize(album).unwrap_or_else(|_| album.to_path_buf());
    {
        let mut statement = connection
            .prepare("SELECT album_path, manifest FROM reversal_manifests WHERE status='pending'")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, parse_sql_json::<Value>(row, 1)?))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (recorded_album, manifest) = row.map_err(|e| e.to_string())?;
            let recorded_album =
                fs::canonicalize(&recorded_album).unwrap_or_else(|_| PathBuf::from(recorded_album));
            let shares_target = manifest["files"].as_array().is_some_and(|files| {
                files.iter().any(|file| {
                    file["target"].as_str().is_some_and(|target| {
                        let target = Path::new(target);
                        target.starts_with(&album)
                            || (target.parent() == album.parent()
                                && matches!(
                                    target.file_name().and_then(|name| name.to_str()),
                                    Some("artist.jpg" | "artist.png")
                                ))
                    })
                })
            });
            if recorded_album == album || shares_target {
                return Err(format!(
                    "Album reversal needs recovery: {}",
                    album.display()
                ));
            }
        }
    }
    let mut statement = connection
        .prepare("SELECT path FROM write_intents WHERE state IN ('needs_recovery', 'reverting')")
        .map_err(|e| e.to_string())?;
    let paths = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if paths.is_empty() {
        return Ok(());
    }
    for path in paths {
        let path = PathBuf::from(path);
        if path.starts_with(&album) {
            return Err(format!("Auto-tag write needs recovery: {}", path.display()));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn validate_recovery_backup(intent: &WriteIntent, data_dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let backup = intent
        .backup_path
        .as_deref()
        .ok_or("write intent has no recovery backup")?;
    let parent = backup.parent().ok_or("recovery backup has no directory")?;
    let parent_metadata = std::fs::symlink_metadata(parent).map_err(|error| error.to_string())?;
    if !parent_metadata.is_dir()
        || std::fs::canonicalize(parent).map_err(|error| error.to_string())?
            != std::fs::canonicalize(data_dir)
                .map_err(|error| error.to_string())?
                .join("write-recovery")
    {
        return Err("recovery backup directory changed".into());
    }
    let before = std::fs::symlink_metadata(backup).map_err(|error| error.to_string())?;
    if !before.is_file() || before.nlink() != 1 || file_sha256(backup)? != intent.before_sha256 {
        return Err("recovery backup is missing or changed".into());
    }
    let after = std::fs::symlink_metadata(backup).map_err(|error| error.to_string())?;
    if (before.dev(), before.ino(), before.nlink()) != (after.dev(), after.ino(), after.nlink()) {
        return Err("recovery backup changed during validation".into());
    }
    Ok(())
}

fn valid_write_receipt(intent: &WriteIntent) -> bool {
    let path = &intent.path;
    let matches_post_state = || {
        intent.after_sha256.is_some()
            && file_sha256(path).is_ok_and(|hash| hash == intent.after_sha256)
    };
    matches_post_state()
        && intent.before_payload_sha256.as_ref().is_some_and(|before| {
            crate::commands::mutations::track_payload_digest(path)
                .is_ok_and(|after| &after == before)
        })
        && crate::commands::auto_tag_review::track_view(path)
            .is_ok_and(|readback| readback_matches(&intent.patch, &readback))
        && matches_post_state()
}

fn readback_matches(patch: &Value, readback: &Value) -> bool {
    let Some(fields) = patch.as_object() else {
        return false;
    };
    !fields.is_empty()
        && fields
            .iter()
            .all(|(field, expected)| readback.get(field).is_some_and(|actual| actual == expected))
}

fn load_verified_field_reviews(
    connection: &Connection,
    revision: Option<&str>,
) -> Result<Vec<(FieldReviewObligation, String)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT f.path, f.field, f.revision, f.before_value, f.after_value, f.provenance,
                    f.evidence_ids, f.state, w.patch, w.after_sha256
             FROM field_review_obligations f JOIN write_intents w ON f.path=w.path
               AND substr(w.run_id, 1, length(f.revision)+2)=f.revision || '::'
             WHERE w.state='verified' AND w.after_sha256 IS NOT NULL
               AND f.state IN ('open', 'acknowledged')
               AND (f.revision=?1 OR (?1 IS NULL AND f.state='open'))
             ORDER BY f.revision, f.path, f.field",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([revision], |row| {
            let field = row_to_field_review(row)?;
            let patch: Value = parse_sql_json(row, 8)?;
            let hash = row.get(9)?;
            Ok((patch.get(&field.field) == Some(&field.after)).then_some((field, hash)))
        })
        .map_err(|e| e.to_string())?;
    let rows = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().flatten().collect())
}

fn row_to_field_review(row: &rusqlite::Row<'_>) -> rusqlite::Result<FieldReviewObligation> {
    Ok(FieldReviewObligation {
        path: PathBuf::from(row.get::<_, String>(0)?),
        field: row.get(1)?,
        revision: row.get(2)?,
        before: parse_sql_json(row, 3)?,
        after: parse_sql_json(row, 4)?,
        provenance: parse_sql_json(row, 5)?,
        evidence_ids: parse_sql_json(row, 6)?,
        state: row.get(7)?,
    })
}

fn parse_sql_json<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<T> {
    let text: String = row.get(index)?;
    serde_json::from_str(&text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn file_sha256(path: &Path) -> Result<Option<String>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return match std::fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
                    "recovery intent target is a symlink: {}",
                    path.display()
                )),
                Ok(_) => Err(format!(
                    "recovery intent target is not a regular file: {}",
                    path.display()
                )),
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error.to_string()),
            }
        }
        Err(error) => return Err(error.to_string()),
    };
    if std::fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
        || !metadata.is_file()
    {
        return Err(format!(
            "recovery intent target is not a regular file: {}",
            path.display()
        ));
    }
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(Some(format!("{:x}", digest.finalize())))
}

fn row_to_write_intent(row: &rusqlite::Row<'_>) -> rusqlite::Result<WriteIntent> {
    let path: String = row.get(1)?;
    let backup_path: Option<String> = row.get(5)?;
    let patch: String = row.get(6)?;
    Ok(WriteIntent {
        run_id: row.get(0)?,
        path: PathBuf::from(path),
        before_sha256: row.get(2)?,
        before_payload_sha256: row.get(3)?,
        after_sha256: row.get(4)?,
        backup_path: backup_path.map(PathBuf::from),
        patch: serde_json::from_str(&patch).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                patch.len(),
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        state: row.get(7)?,
    })
}

fn reconciled_write_state(
    connection: &Connection,
    intent: &WriteIntent,
) -> Result<Option<&'static str>, String> {
    let revision = write_intent_revision(&intent.run_id);
    let owner: Option<Option<String>> = connection
        .query_row(
            "SELECT owner FROM worker_items WHERE run_id=?1 AND status='running'",
            [revision],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(owner) = owner {
        let pid = owner.as_deref().and_then(worker_owner_pid);
        if pid.is_none_or(crate::commands::auto_tag_review::process_alive) {
            return Ok(None);
        }
    }
    Ok(match intent.state.as_str() {
        "written" if valid_write_receipt(intent) => Some("verified"),
        _ if intent.before_sha256.is_some()
            && file_sha256(&intent.path).is_ok_and(|hash| hash == intent.before_sha256) =>
        {
            Some("retry_safe")
        }
        "needs_recovery" => None,
        _ => Some("needs_recovery"),
    })
}

fn load_write_intent(
    connection: &Connection,
    run_id: &str,
    path: &str,
) -> Result<Option<WriteIntent>, String> {
    connection
        .query_row(
            "SELECT run_id, path, before_sha256, before_payload_sha256, after_sha256,
                    backup_path, patch, state
             FROM write_intents WHERE run_id=?1 AND path=?2",
            params![run_id, path],
            row_to_write_intent,
        )
        .optional()
        .map_err(|e| e.to_string())
}

fn save_write_intent(connection: &Connection, intent: &WriteIntent) -> Result<(), String> {
    let changed = connection
        .execute(
            "UPDATE write_intents
             SET before_sha256=?3, before_payload_sha256=?4, after_sha256=?5,
                 backup_path=?6, patch=?7, state=?8, updated_at=?9
             WHERE run_id=?1 AND path=?2",
            params![
                intent.run_id,
                storage_key(&intent.path),
                intent.before_sha256,
                intent.before_payload_sha256,
                intent.after_sha256,
                intent.backup_path.as_deref().map(Path::to_string_lossy),
                intent.patch.to_string(),
                intent.state,
                now_epoch()
            ],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("write intent update lost its durable row".into());
    }
    Ok(())
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
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn durable_write_authority_binding_requires_the_same_library_after_restart() {
        use crate::state::library_authority::{acquire, nominate};
        let root = root();
        let library = root.join("library");
        let other_library = root.join("other");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(&other_library).unwrap();
        let path = library.join("track.bin");
        let legacy = library.join("legacy.bin");
        fs::write(&path, b"before").unwrap();
        fs::write(&legacy, b"legacy").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&library, owner).unwrap();
        nominate(&other_library, owner).unwrap();
        let lease = acquire(&library, owner).unwrap();
        let other = acquire(&other_library, owner).unwrap();
        let store = WorkerStore::new_in(root.join("profile"));
        assert!(store.initialize());
        store
            .prepare_write_intent(&path, "run", &serde_json::json!({}))
            .unwrap();
        store
            .prepare_write_intent(&legacy, "legacy", &serde_json::json!({}))
            .unwrap();
        store
            .bind_write_authority(&path, "run", &lease, Path::new("track.bin"))
            .unwrap();
        store
            .bind_write_authority(&path, "run", &lease, Path::new("track.bin"))
            .unwrap();
        store.begin_write_intent(&path, "run").unwrap();
        assert!(store
            .bind_write_authority(&path, "run", &lease, Path::new("track.bin"))
            .is_err());
        assert!(store
            .verify_write_authority(&path, "run", &other, Path::new("track.bin"))
            .is_err());
        assert!(store
            .verify_write_authority(&legacy, "legacy", &lease, Path::new("legacy.bin"))
            .is_err());
        drop(store);
        drop(lease);
        let reopened = WorkerStore::new_in(root.join("profile"));
        assert!(reopened.initialize());
        let lease = acquire(&library, owner).unwrap();
        reopened
            .verify_write_authority(&path, "run", &lease, Path::new("track.bin"))
            .unwrap();
        assert!(reopened
            .bind_write_authority(&path, "run", &other, Path::new("track.bin"))
            .is_err());
        drop(lease);
        fs::rename(
            library.join(crate::state::library_authority::MARKER_NAME),
            library.join("old-marker"),
        )
        .unwrap();
        let replacement_owner = uuid::Uuid::new_v4();
        nominate(&library, replacement_owner).unwrap();
        let replacement = acquire(&library, replacement_owner).unwrap();
        assert!(reopened
            .verify_write_authority(&path, "run", &replacement, Path::new("track.bin"))
            .is_err());
        assert!(reopened
            .bind_write_authority(&path, "run", &replacement, Path::new("track.bin"))
            .is_err());
        assert_eq!(fs::read(path).unwrap(), b"before");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn legacy_worker_database_keeps_unbound_intents_during_authority_upgrade() {
        use crate::state::library_authority::{acquire, nominate};
        let root = root();
        let library = root.join("library");
        fs::create_dir_all(&library).unwrap();
        let path = library.join("track.bin");
        fs::write(&path, b"before").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&library, owner).unwrap();
        let lease = acquire(&library, owner).unwrap();
        let store = WorkerStore::new_in(root.join("profile"));
        assert!(store.initialize());
        store
            .prepare_write_intent(&path, "legacy", &serde_json::json!({}))
            .unwrap();
        {
            let guard = store.inner.lock().unwrap();
            guard
                .as_ref()
                .unwrap()
                .execute_batch("DROP TABLE authority_write_bindings")
                .unwrap();
        }
        drop(store);
        let reopened = WorkerStore::new_in(root.join("profile"));
        assert!(reopened.initialize());
        assert!(reopened
            .verify_write_authority(&path, "legacy", &lease, Path::new("track.bin"))
            .is_err());
        let guard = reopened.inner.lock().unwrap();
        assert!(load_write_intent(
            guard.as_ref().unwrap(),
            "legacy",
            &recovery_key(&path).unwrap()
        )
        .unwrap()
        .is_some());
        drop(guard);
        assert_eq!(fs::read(path).unwrap(), b"before");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn binding_and_verification_recheck_authority_after_waiting_for_worker_lock() {
        use crate::state::library_authority::{acquire, nominate, MARKER_NAME};
        for verifying in [false, true] {
            let root = root();
            let library = root.join("library");
            fs::create_dir_all(&library).unwrap();
            let path = library.join("track.bin");
            fs::write(&path, b"before").unwrap();
            let owner = uuid::Uuid::new_v4();
            nominate(&library, owner).unwrap();
            let lease = Arc::new(acquire(&library, owner).unwrap());
            let store = WorkerStore::new_in(root.join("profile"));
            assert!(store.initialize());
            store
                .prepare_write_intent(&path, "run", &serde_json::json!({}))
                .unwrap();
            if verifying {
                store
                    .bind_write_authority(&path, "run", &lease, Path::new("track.bin"))
                    .unwrap();
            }
            let held = store.inner.lock().unwrap();
            let (send, receive) = std::sync::mpsc::channel();
            let worker = store.clone();
            let worker_lease = lease.clone();
            let worker_path = path.clone();
            let thread = std::thread::spawn(move || {
                let after_check = || send.send(()).unwrap();
                if verifying {
                    worker.verify_write_authority_with(
                        &worker_path,
                        "run",
                        &worker_lease,
                        Path::new("track.bin"),
                        after_check,
                    )
                } else {
                    worker.bind_write_authority_with(
                        &worker_path,
                        "run",
                        &worker_lease,
                        Path::new("track.bin"),
                        after_check,
                    )
                }
            });
            receive
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap();
            fs::rename(library.join(MARKER_NAME), library.join("removed-marker")).unwrap();
            drop(held);
            assert!(thread.join().unwrap().is_err());
            let guard = store.inner.lock().unwrap();
            let count: i64 = guard
                .as_ref()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM authority_write_bindings", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, if verifying { 1 } else { 0 });
            drop(guard);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn binding_rejects_alias_retargeted_while_waiting_for_worker_lock() {
        use crate::state::library_authority::{acquire, nominate};
        use std::os::unix::fs::symlink;
        let root = root();
        let library = root.join("library");
        let other = root.join("other");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::write(library.join("track.bin"), b"before").unwrap();
        fs::write(other.join("track.bin"), b"other").unwrap();
        let alias = root.join("alias");
        symlink(&library, &alias).unwrap();
        let path = alias.join("track.bin");
        let owner = uuid::Uuid::new_v4();
        nominate(&library, owner).unwrap();
        let lease = Arc::new(acquire(&library, owner).unwrap());
        let store = WorkerStore::new_in(root.join("profile"));
        assert!(store.initialize());
        store
            .prepare_write_intent(&path, "run", &serde_json::json!({}))
            .unwrap();
        let held = store.inner.lock().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = store.clone();
        let worker_lease = lease.clone();
        let worker_path = path.clone();
        let thread = std::thread::spawn(move || {
            worker.bind_write_authority_with(
                &worker_path,
                "run",
                &worker_lease,
                Path::new("track.bin"),
                || send.send(()).unwrap(),
            )
        });
        receive
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        fs::remove_file(&alias).unwrap();
        symlink(&other, &alias).unwrap();
        drop(held);
        assert!(thread.join().unwrap().is_err());
        let guard = store.inner.lock().unwrap();
        let count: i64 = guard
            .as_ref()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM authority_write_bindings", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        drop(guard);
        assert_eq!(fs::read(other.join("track.bin")).unwrap(), b"other");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn binding_reports_authority_loss_while_commit_waits_for_reader() {
        use crate::state::library_authority::{acquire, nominate, MARKER_NAME};
        let root = root();
        let library = root.join("library");
        fs::create_dir_all(&library).unwrap();
        let path = library.join("track.bin");
        fs::write(&path, b"before").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&library, owner).unwrap();
        let lease = Arc::new(acquire(&library, owner).unwrap());
        let store = WorkerStore::new_in(root.join("profile"));
        assert!(store.initialize());
        store
            .prepare_write_intent(&path, "run", &serde_json::json!({}))
            .unwrap();
        let reader = Connection::open(&store.path).unwrap();
        reader
            .execute_batch("BEGIN; SELECT * FROM write_intents;")
            .unwrap();
        let (checked_send, checked_receive) = std::sync::mpsc::channel();
        let (continue_send, continue_receive) = std::sync::mpsc::channel();
        let worker = store.clone();
        let worker_lease = lease.clone();
        let worker_path = path.clone();
        let thread = std::thread::spawn(move || {
            worker.bind_write_authority_with_hooks(
                &worker_path,
                "run",
                &worker_lease,
                Path::new("track.bin"),
                || {},
                || {
                    checked_send.send(()).unwrap();
                    continue_receive.recv().unwrap();
                },
            )
        });
        checked_receive
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        continue_send.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::rename(library.join(MARKER_NAME), library.join("removed-marker")).unwrap();
        reader.execute_batch("COMMIT").unwrap();
        let error = thread.join().unwrap().unwrap_err();
        assert!(error.contains("binding persisted"), "{error}");
        let guard = store.inner.lock().unwrap();
        let count: i64 = guard
            .as_ref()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM authority_write_bindings", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
        drop(guard);
        assert_eq!(fs::read(path).unwrap(), b"before");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn authorized_reconciliation_after_restart_holds_legacy_intents() {
        use crate::state::library_authority::{acquire, nominate};
        let root = root();
        let library = root.join("library");
        fs::create_dir_all(&library).unwrap();
        let path = library.join("track.bin");
        let legacy = library.join("legacy.bin");
        fs::write(&path, b"before").unwrap();
        fs::write(&legacy, b"legacy").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&library, owner).unwrap();
        let lease = acquire(&library, owner).unwrap();
        let store = WorkerStore::new_in(root.join("profile"));
        assert!(store.initialize());
        store
            .prepare_write_intent(&path, "run", &serde_json::json!({}))
            .unwrap();
        store
            .bind_write_authority(&path, "run", &lease, Path::new("track.bin"))
            .unwrap();
        store.begin_write_intent(&path, "run").unwrap();
        store
            .prepare_write_intent(&legacy, "legacy", &serde_json::json!({}))
            .unwrap();
        drop(store);
        drop(lease);
        let reopened = WorkerStore::new_in(root.join("profile"));
        assert!(reopened.initialize());
        let lease = acquire(&library, owner).unwrap();
        assert!(reopened
            .reconcile_write_intent_authorized(&legacy, "legacy", &lease, Path::new("legacy.bin"))
            .is_err());
        assert_eq!(
            reopened
                .reconcile_write_intent_authorized(&path, "run", &lease, Path::new("track.bin"))
                .unwrap(),
            "retry_safe"
        );
        let guard = reopened.inner.lock().unwrap();
        let connection = guard.as_ref().unwrap();
        assert_eq!(
            load_write_intent(connection, "legacy", &recovery_key(&legacy).unwrap())
                .unwrap()
                .unwrap()
                .state,
            "prepared"
        );
        assert_eq!(
            load_write_intent(connection, "run", &recovery_key(&path).unwrap())
                .unwrap()
                .unwrap()
                .state,
            "retry_safe"
        );
        drop(guard);
        {
            let guard = reopened.inner.lock().unwrap();
            guard
                .as_ref()
                .unwrap()
                .execute(
                    "UPDATE write_intents SET state='superseded' WHERE run_id='run'",
                    [],
                )
                .unwrap();
        }
        assert!(reopened
            .reconcile_write_intent_authorized(&path, "run", &lease, Path::new("track.bin"))
            .is_err());
        assert_eq!(fs::read(path).unwrap(), b"before");
        assert_eq!(fs::read(legacy).unwrap(), b"legacy");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[ignore = "subprocess entry for authorized intent crash recovery"]
    fn authorized_intent_crash_child() {
        use crate::state::library_authority::acquire;
        let root = PathBuf::from(std::env::var_os("SOUNDROBE_INTENT_CRASH_ROOT").unwrap());
        let owner =
            uuid::Uuid::parse_str(&std::env::var("SOUNDROBE_INTENT_CRASH_OWNER").unwrap()).unwrap();
        let library = root.join("library");
        let after_write = std::env::var_os("SOUNDROBE_INTENT_CRASH_AFTER_WRITE").is_some();
        let filename = if after_write {
            "track.wav"
        } else {
            "track.bin"
        };
        let path = library.join(filename);
        let lease = acquire(&library, owner).unwrap();
        let store = WorkerStore::new_in(root.join("profile"));
        assert!(store.initialize());
        store.claim_desktop_run(&library, "run").unwrap();
        let patch = if after_write {
            serde_json::json!({"title":"Recovered title"})
        } else {
            serde_json::json!({})
        };
        store.prepare_write_intent(&path, "run", &patch).unwrap();
        store
            .bind_write_authority(&path, "run", &lease, Path::new(filename))
            .unwrap();
        store.begin_write_intent(&path, "run").unwrap();
        if after_write {
            let parsed: crate::commands::mutations::TrackPatch =
                serde_json::from_value(patch).unwrap();
            let queue = crate::state::write_queue::WriteQueue::default();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime
                .block_on(queue.run_checked(async {
                    crate::commands::mutations::write_track_dispatch(&path, &parsed)
                }))
                .unwrap()
                .unwrap();
            store
                .record_write_intent_applied(&path, "run", true)
                .unwrap();
        }
        std::process::exit(71);
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn authorized_intent_reconciles_after_owner_process_crash() {
        use crate::state::library_authority::{acquire, nominate};
        let root = root();
        let library = root.join("library");
        fs::create_dir_all(&library).unwrap();
        let path = library.join("track.bin");
        fs::write(&path, b"before").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&library, owner).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "state::worker::tests::authorized_intent_crash_child",
                "--ignored",
            ])
            .env("SOUNDROBE_INTENT_CRASH_ROOT", &root)
            .env("SOUNDROBE_INTENT_CRASH_OWNER", owner.to_string())
            .spawn()
            .unwrap();
        let child_pid = child.id();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("authorized intent child did not finish");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(status.code(), Some(71));
        let lease = acquire(&library, owner).unwrap();
        let reopened = WorkerStore::new_in(root.join("profile"));
        assert!(reopened.initialize());
        {
            let guard = reopened.inner.lock().unwrap();
            let owner_row: String = guard
                .as_ref()
                .unwrap()
                .query_row(
                    "SELECT owner FROM worker_items WHERE run_id='run' AND status='running'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(owner_row, format!("desktop:{child_pid}"));
            assert!(!crate::commands::auto_tag_review::process_alive(child_pid));
        }
        assert_eq!(
            reopened
                .reconcile_write_intent_authorized(&path, "run", &lease, Path::new("track.bin"))
                .unwrap(),
            "retry_safe"
        );
        assert_eq!(fs::read(path).unwrap(), b"before");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn authorized_written_intent_is_verified_after_owner_crash() {
        use crate::state::library_authority::{acquire, nominate, AuthorityError};
        let root = root();
        let library = root.join("library");
        fs::create_dir_all(&library).unwrap();
        let path = library.join("track.wav");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.wav"),
            &path,
        )
        .unwrap();
        let payload_before = crate::commands::mutations::track_payload_digest(&path).unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&library, owner).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "state::worker::tests::authorized_intent_crash_child",
                "--ignored",
            ])
            .env("SOUNDROBE_INTENT_CRASH_ROOT", &root)
            .env("SOUNDROBE_INTENT_CRASH_OWNER", owner.to_string())
            .env("SOUNDROBE_INTENT_CRASH_AFTER_WRITE", "1")
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("authorized writer child did not finish");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(status.code(), Some(71));
        assert!(matches!(
            acquire(&library, uuid::Uuid::new_v4()),
            Err(AuthorityError::OtherProfile)
        ));
        let lease = acquire(&library, owner).unwrap();
        let reopened = WorkerStore::new_in(root.join("profile"));
        assert!(reopened.initialize());
        assert_eq!(
            reopened
                .reconcile_write_intent_authorized(&path, "run", &lease, Path::new("track.wav"))
                .unwrap(),
            "verified"
        );
        assert_eq!(
            crate::commands::tracks::read_track_metadata(&path)
                .unwrap()
                .title
                .as_deref(),
            Some("Recovered title")
        );
        assert_eq!(
            crate::commands::mutations::track_payload_digest(&path).unwrap(),
            payload_before
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn reconciliation_reports_authority_loss_during_commit_wait() {
        use crate::state::library_authority::{acquire, nominate, MARKER_NAME};
        let root = root();
        let library = root.join("library");
        fs::create_dir_all(&library).unwrap();
        let path = library.join("track.bin");
        fs::write(&path, b"before").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&library, owner).unwrap();
        let lease = Arc::new(acquire(&library, owner).unwrap());
        let store = WorkerStore::new_in(root.join("profile"));
        assert!(store.initialize());
        store
            .prepare_write_intent(&path, "run", &serde_json::json!({}))
            .unwrap();
        store
            .bind_write_authority(&path, "run", &lease, Path::new("track.bin"))
            .unwrap();
        store.begin_write_intent(&path, "run").unwrap();
        let reader = Connection::open(&store.path).unwrap();
        reader
            .execute_batch("BEGIN; SELECT * FROM write_intents;")
            .unwrap();
        let (checked_send, checked_receive) = std::sync::mpsc::channel();
        let (continue_send, continue_receive) = std::sync::mpsc::channel();
        let worker = store.clone();
        let worker_lease = lease.clone();
        let worker_path = path.clone();
        let thread = std::thread::spawn(move || {
            worker.reconcile_write_intent_authorized_with(
                &worker_path,
                "run",
                &worker_lease,
                Path::new("track.bin"),
                || {
                    checked_send.send(()).unwrap();
                    continue_receive.recv().unwrap();
                },
            )
        });
        checked_receive
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        continue_send.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::rename(library.join(MARKER_NAME), library.join("removed-marker")).unwrap();
        reader.execute_batch("COMMIT").unwrap();
        let error = thread.join().unwrap().unwrap_err();
        assert!(error.contains("reconciliation persisted"), "{error}");
        let guard = store.inner.lock().unwrap();
        assert_eq!(
            load_write_intent(
                guard.as_ref().unwrap(),
                "run",
                &recovery_key(&path).unwrap()
            )
            .unwrap()
            .unwrap()
            .state,
            "retry_safe"
        );
        drop(guard);
        assert_eq!(fs::read(path).unwrap(), b"before");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn staged_write_must_match_bound_intent_and_audio_before_publication() {
        use crate::commands::mutations::{write_track_dispatch, TrackPatch};
        use crate::state::authority_io::AuthorizedRoot;
        use crate::state::library_authority::nominate;
        use crate::state::write_queue::WriteQueue;
        let root = root();
        let library = root.join("library");
        let work = root.join("private");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(&work).unwrap();
        let path = library.join("track.wav");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.wav"),
            &path,
        )
        .unwrap();
        let before = fs::read(&path).unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&library, owner).unwrap();
        let authority = AuthorizedRoot::open(&library, owner).unwrap();
        let store = WorkerStore::new_in(root.join("profile"));
        assert!(store.initialize());
        let patch = serde_json::json!({"title":"Verified staged title"});
        store.prepare_write_intent(&path, "run", &patch).unwrap();
        let mut stage = authority.stage_file(Path::new("track.wav"), &work).unwrap();
        store
            .bind_write_authority(&path, "run", stage.lease(), Path::new("track.wav"))
            .unwrap();
        let patch: TrackPatch = serde_json::from_value(patch).unwrap();
        let queue = WriteQueue::default();
        queue
            .run_checked(async {
                stage.run_writer(|staged| {
                    write_track_dispatch(staged, &patch)
                        .map(|_| ())
                        .map_err(|error| std::io::Error::other(error.to_string()))
                })
            })
            .await
            .unwrap()
            .unwrap();
        assert!(store
            .validate_authority_staged_write(&path, "run", Path::new("track.wav"), &stage)
            .is_ok());
        let wrong_patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title":"Wrong title"})).unwrap();
        queue
            .run_checked(async {
                stage.run_writer(|staged| {
                    write_track_dispatch(staged, &wrong_patch)
                        .map(|_| ())
                        .map_err(|error| std::io::Error::other(error.to_string()))
                })
            })
            .await
            .unwrap()
            .unwrap();
        assert!(store
            .validate_authority_staged_write(&path, "run", Path::new("track.wav"), &stage)
            .is_err());
        queue
            .run_checked(async {
                stage.run_writer(|staged| {
                    write_track_dispatch(staged, &patch)
                        .map(|_| ())
                        .map_err(|error| std::io::Error::other(error.to_string()))
                })
            })
            .await
            .unwrap()
            .unwrap();
        assert!(store
            .validate_authority_staged_write(&path, "run", Path::new("track.wav"), &stage)
            .is_ok());
        queue
            .run_checked(async {
                stage.run_writer(|staged| {
                    let mut bytes = fs::read(staged)?;
                    let mut position = 12usize;
                    let mut changed = false;
                    while position + 8 <= bytes.len() {
                        let size = u32::from_le_bytes(
                            bytes[position + 4..position + 8].try_into().unwrap(),
                        ) as usize;
                        let start = position + 8;
                        if &bytes[position..position + 4] == b"data"
                            && size > 0
                            && start + size <= bytes.len()
                        {
                            bytes[start] ^= 1;
                            changed = true;
                            break;
                        }
                        position = start
                            .checked_add(size + (size & 1))
                            .ok_or_else(|| std::io::Error::other("invalid WAV fixture"))?;
                    }
                    if !changed {
                        return Err(std::io::Error::other("WAV fixture has no audio payload"));
                    }
                    fs::write(staged, bytes)
                })
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            crate::commands::tracks::read_track_metadata(stage.path())
                .unwrap()
                .title
                .as_deref(),
            Some("Verified staged title")
        );
        assert!(store
            .validate_authority_staged_write(&path, "run", Path::new("track.wav"), &stage)
            .is_err());
        queue
            .run_checked(async { stage.run_writer(|staged| fs::write(staged, b"not audio")) })
            .await
            .unwrap()
            .unwrap();
        assert!(store
            .validate_authority_staged_write(&path, "run", Path::new("track.wav"), &stage)
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        stage.cleanup().unwrap();
        let other_patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title":"Newer source"})).unwrap();
        queue
            .run_checked(async { write_track_dispatch(&path, &other_patch) })
            .await
            .unwrap()
            .unwrap();
        let mut newer_stage = authority.stage_file(Path::new("track.wav"), &work).unwrap();
        queue
            .run_checked(async {
                newer_stage.run_writer(|staged| {
                    write_track_dispatch(staged, &patch)
                        .map(|_| ())
                        .map_err(|error| std::io::Error::other(error.to_string()))
                })
            })
            .await
            .unwrap()
            .unwrap();
        let error = store
            .validate_authority_staged_write(&path, "run", Path::new("track.wav"), &newer_stage)
            .unwrap_err();
        assert!(error.contains("pre-state"), "{error}");
        newer_stage.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn staged_validation_rejects_intent_changed_during_readback() {
        use crate::commands::mutations::{write_track_dispatch, TrackPatch};
        use crate::state::authority_io::AuthorizedRoot;
        use crate::state::library_authority::nominate;
        use crate::state::write_queue::WriteQueue;
        let root = root();
        let library = root.join("library");
        let work = root.join("private");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(&work).unwrap();
        let path = library.join("track.wav");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.wav"),
            &path,
        )
        .unwrap();
        let before = fs::read(&path).unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&library, owner).unwrap();
        let authority = AuthorizedRoot::open(&library, owner).unwrap();
        let store = WorkerStore::new_in(root.join("profile"));
        assert!(store.initialize());
        let patch = serde_json::json!({"title":"Staged title"});
        let backup = store
            .prepare_write_intent(&path, "run", &patch)
            .unwrap()
            .backup_path
            .unwrap();
        let mut stage = authority.stage_file(Path::new("track.wav"), &work).unwrap();
        store
            .bind_write_authority(&path, "run", stage.lease(), Path::new("track.wav"))
            .unwrap();
        let patch: TrackPatch = serde_json::from_value(patch).unwrap();
        WriteQueue::default()
            .run_checked(async {
                stage.run_writer(|staged| {
                    write_track_dispatch(staged, &patch)
                        .map(|_| ())
                        .map_err(|error| std::io::Error::other(error.to_string()))
                })
            })
            .await
            .unwrap()
            .unwrap();
        let backup_bytes = fs::read(&backup).unwrap();
        let backup_result = store.validate_authority_staged_write_with(
            &path,
            "run",
            Path::new("track.wav"),
            &stage,
            || fs::write(&backup, b"damaged").unwrap(),
        );
        assert!(
            backup_result.is_err(),
            "backup damage during staged validation must be rejected"
        );
        fs::write(&backup, backup_bytes).unwrap();
        store.begin_write_intent(&path, "run").unwrap();
        let result = store.validate_authority_staged_write_with(
            &path,
            "run",
            Path::new("track.wav"),
            &stage,
            || {
                store
                    .record_write_intent_applied(&path, "run", false)
                    .unwrap();
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        stage.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn field_acknowledgment_is_revisioned_and_does_not_complete_deferred_work() {
        let root = root();
        let album = root.join("Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        let due = store
            .defer_retry_at(&album, "run-1", None, 100)
            .unwrap()
            .unwrap();
        store
            .record_field_review_obligation(
                &album,
                "genre",
                "run-1",
                &serde_json::json!(null),
                &serde_json::json!("Jazz"),
                &serde_json::json!({"source":"assisted","confidence":0.91}),
                &["evidence-1".into()],
            )
            .unwrap();
        store
            .record_field_review_obligation(
                &album,
                "year",
                "run-1",
                &serde_json::json!(null),
                &serde_json::json!(1984),
                &serde_json::json!({"source":"provider"}),
                &["release-1".into()],
            )
            .unwrap();
        assert!(store
            .record_field_review_obligation(
                &album,
                "genre",
                "run-1",
                &serde_json::json!(null),
                &serde_json::json!("Blues"),
                &serde_json::json!({"source":"assisted","confidence":0.91}),
                &["evidence-1".into()],
            )
            .is_err());
        store
            .acknowledge_field_review_obligation(&album, "genre", "run-1")
            .unwrap();
        drop(store);

        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        let obligations = reopened.field_review_obligations(&album).unwrap();
        assert_eq!(obligations.len(), 2);
        assert_eq!(obligations[0].field, "genre");
        assert_eq!(obligations[0].state, "acknowledged");
        assert_eq!(obligations[0].after, "Jazz");
        assert_eq!(obligations[1].field, "year");
        assert_eq!(obligations[1].state, "open");
        assert_eq!(reopened.pending_at(due).unwrap(), vec![album.clone()]);
        assert_eq!(reopened.actionable_review_paths().unwrap(), vec![album]);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn field_acknowledgment_is_idempotent_through_a_parent_directory_alias() {
        use std::os::unix::fs::symlink;
        let root = root();
        let album = root.join("Album");
        fs::create_dir_all(&album).unwrap();
        let alias = root.join("alias");
        symlink(&album, &alias).unwrap();
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .record_field_review_obligation(
                &album,
                "genre",
                "run-1",
                &serde_json::json!(null),
                &serde_json::json!("Jazz"),
                &serde_json::json!({"source":"assisted"}),
                &[],
            )
            .unwrap();

        store
            .acknowledge_field_review_obligation(&alias, "genre", "run-1")
            .unwrap();
        assert!(
            store
                .acknowledge_field_review_obligation(&alias, "genre", "run-1")
                .is_ok(),
            "repeating an acknowledgement through the same physical path is idempotent"
        );
        assert_eq!(
            store.field_review_obligations(&album).unwrap()[0].state,
            "acknowledged"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn revert_intent_stays_reverting_when_media_parent_cannot_be_synced() {
        use std::os::unix::fs::PermissionsExt;
        let root = root();
        let media = root.join("media");
        fs::create_dir_all(&media).unwrap();
        let file = media.join("track.bin");
        fs::write(&file, b"before").unwrap();
        let store = WorkerStore::new_in(root.join("state"));
        assert!(store.initialize());
        store
            .prepare_write_intent(&file, "run", &serde_json::json!({"genre":"Jazz"}))
            .unwrap();
        store.begin_write_intent(&file, "run").unwrap();
        store
            .mark_review_write_intents_reverting(&file, "run")
            .unwrap();
        std::fs::set_permissions(&media, std::fs::Permissions::from_mode(0o111)).unwrap();

        let result = store.mark_review_write_intents_reverted(&file, "run");
        std::fs::set_permissions(&media, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            result.is_err(),
            "terminal revert requires a durable media directory entry"
        );
        let state: String = store
            .inner
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .query_row(
                "SELECT state FROM write_intents WHERE run_id='run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            state, "reverting",
            "failed durability must remain recoverable"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reconciliation_keeps_terminal_and_unrelated_intents_when_a_target_is_unreadable() {
        use std::os::unix::fs::PermissionsExt;
        let root = root();
        let missing = root.join("a-missing.bin");
        let retry = root.join("b-retry.bin");
        let inaccessible = root.join("c-inaccessible.bin");
        let terminal = root.join("z-reverted.bin");
        fs::write(&missing, b"before-missing").unwrap();
        fs::write(&retry, b"before-retry").unwrap();
        fs::write(&inaccessible, b"before-inaccessible").unwrap();
        fs::write(&terminal, b"before-terminal").unwrap();
        let store = WorkerStore::new_in(root.join("state"));
        assert!(store.initialize());
        for (path, run) in [
            (&missing, "missing"),
            (&retry, "retry"),
            (&inaccessible, "inaccessible"),
            (&terminal, "terminal"),
        ] {
            store
                .prepare_write_intent(path, run, &serde_json::json!({"genre":"Jazz"}))
                .unwrap();
        }
        store
            .mark_review_write_intents_reverting(&terminal, "terminal")
            .unwrap();
        store
            .mark_review_write_intents_reverted(&terminal, "terminal")
            .unwrap();
        fs::remove_file(&missing).unwrap();
        fs::remove_file(&terminal).unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&inaccessible, std::fs::Permissions::from_mode(0o000)).unwrap();

        let entries = store.reconcile_write_intents();
        #[cfg(unix)]
        std::fs::set_permissions(&inaccessible, std::fs::Permissions::from_mode(0o600)).unwrap();
        let entries = entries.unwrap();
        assert_eq!(
            entries.len(),
            4,
            "one unreadable target must not hide any intent"
        );
        let action = |run_id: &str| {
            &entries
                .iter()
                .find(|entry| entry.intent.run_id == run_id)
                .unwrap()
                .action
        };
        assert_eq!(*action("missing"), WriteRecoveryAction::NeedsRecovery);
        assert_eq!(*action("retry"), WriteRecoveryAction::RetrySafe);
        assert_eq!(*action("inaccessible"), WriteRecoveryAction::NeedsRecovery);
        assert_eq!(*action("terminal"), WriteRecoveryAction::Reverted);
        assert!(entries
            .iter()
            .find(|entry| entry.intent.run_id == "missing")
            .unwrap()
            .error
            .as_deref()
            .is_some_and(|error| error.contains("missing")));
        assert!(entries
            .iter()
            .find(|entry| entry.intent.run_id == "inaccessible")
            .unwrap()
            .error
            .as_deref()
            .is_some_and(|error| error.contains("Permission denied")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn write_intent_restart_reconciles_pre_post_and_conflicting_states_without_writes() {
        let root = root();
        let applied_file = root.join("applied.bin");
        let conflicted_file = root.join("conflicted.bin");
        let retry_file = root.join("retry.bin");
        fs::write(&applied_file, b"before-applied").unwrap();
        fs::write(&conflicted_file, b"before-conflicted").unwrap();
        fs::write(&retry_file, b"before-retry").unwrap();
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());

        store
            .prepare_write_intent(
                &applied_file,
                "run-applied",
                &serde_json::json!({"genre":"Jazz"}),
            )
            .unwrap();
        store
            .begin_write_intent(&applied_file, "run-applied")
            .unwrap();
        fs::write(&applied_file, b"after-applied").unwrap();
        store
            .record_write_intent_applied(&applied_file, "run-applied", true)
            .unwrap();

        store
            .prepare_write_intent(
                &retry_file,
                "run-retry",
                &serde_json::json!({"genre":"Jazz"}),
            )
            .unwrap();
        store.begin_write_intent(&retry_file, "run-retry").unwrap();

        store
            .prepare_write_intent(
                &conflicted_file,
                "run-conflict",
                &serde_json::json!({"genre":"Jazz"}),
            )
            .unwrap();
        store
            .begin_write_intent(&conflicted_file, "run-conflict")
            .unwrap();
        fs::write(&conflicted_file, b"external-change").unwrap();
        drop(store);

        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        let recovery = reopened.reconcile_write_intents().unwrap();
        assert_eq!(recovery.len(), 3);
        let action = |run_id| {
            &recovery
                .iter()
                .find(|entry| entry.intent.run_id == run_id)
                .unwrap()
                .action
        };
        assert_eq!(
            action("run-applied"),
            &WriteRecoveryAction::NeedsRecovery,
            "a file hash captured after a successful closure still needs readback validation"
        );
        assert_eq!(action("run-retry"), &WriteRecoveryAction::RetrySafe);
        assert_eq!(action("run-conflict"), &WriteRecoveryAction::NeedsRecovery);
        let conflict_backup = recovery
            .iter()
            .find(|entry| entry.intent.run_id == "run-conflict")
            .unwrap()
            .intent
            .backup_path
            .as_ref()
            .unwrap();
        assert_eq!(fs::read(conflict_backup).unwrap(), b"before-conflicted");
        assert_eq!(fs::read(&conflicted_file).unwrap(), b"external-change");
        assert_eq!(reopened.reconcile_write_intents().unwrap().len(), 3);
        assert_eq!(fs::read(&applied_file).unwrap(), b"after-applied");
        assert_eq!(fs::read(&retry_file).unwrap(), b"before-retry");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn validated_metadata_and_audio_payload_receipt_survives_restart() {
        let root = root();
        let file = root.join("01.flac");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/writer-corpus/padded.flac"),
            &file,
        )
        .unwrap();
        let store = WorkerStore::new_in(root.join("state"));
        assert!(store.initialize());
        let patch = serde_json::json!({"genre":"Jazz"});
        store.prepare_write_intent(&file, "run", &patch).unwrap();
        store.begin_write_intent(&file, "run").unwrap();
        let track_patch = serde_json::from_value(patch).unwrap();
        crate::commands::mutations::write_track_queued(
            &crate::state::write_queue::WriteQueue::default(),
            file.clone(),
            track_patch,
        )
        .await
        .unwrap();
        store
            .record_write_intent_applied(&file, "run", true)
            .unwrap();
        let written_bytes = fs::read(&file).unwrap();
        drop(store);

        let reopened = WorkerStore::new_in(root.join("state"));
        assert!(reopened.initialize());
        reopened.reconcile_writes_at_startup().unwrap();
        let recovery = reopened.reconcile_write_intents().unwrap();
        assert_eq!(recovery.len(), 1);
        assert_eq!(recovery[0].action, WriteRecoveryAction::ReceiptRecoverable);
        assert_eq!(fs::read(&file).unwrap(), written_bytes);
        assert_eq!(
            recovery[0]
                .intent
                .before_payload_sha256
                .as_ref()
                .unwrap()
                .len(),
            64
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_recovery_holds_unknown_changes_and_retires_untouched_abandoned_attempts() {
        let root = root();
        let untouched = root.join("untouched.bin");
        let failed_without_changes = root.join("failed.bin");
        let changed = root.join("changed.bin");
        let live = root.join("live.bin");
        let other_album = root.join("other-album");
        fs::create_dir(&other_album).unwrap();
        let store = WorkerStore::new_in(root.join("state"));
        assert!(store.initialize());
        store.claim_desktop_run(&root, "live").unwrap();
        for (path, id) in [
            (&untouched, "untouched"),
            (&failed_without_changes, "failed"),
            (&changed, "changed"),
            (&live, "live::operation"),
        ] {
            fs::write(path, b"before").unwrap();
            store
                .prepare_write_intent(path, id, &serde_json::json!({"genre":"Jazz"}))
                .unwrap();
            store.begin_write_intent(path, id).unwrap();
        }
        fs::write(&changed, b"external change").unwrap();
        fs::write(&live, b"in flight").unwrap();
        store
            .record_write_intent_applied(&failed_without_changes, "failed", false)
            .unwrap();

        store.reconcile_writes_at_startup().unwrap();
        assert!(store
            .reconcile_write_intents()
            .unwrap()
            .iter()
            .any(|entry| {
                entry.intent.run_id == "failed" && entry.action == WriteRecoveryAction::RetrySafe
            }));
        store.reconcile_writes_at_startup().unwrap();
        let recovery = store.reconcile_write_intents().unwrap();
        let intent = |id| {
            &recovery
                .iter()
                .find(|entry| entry.intent.run_id == id)
                .unwrap()
                .intent
        };
        assert_eq!(intent("untouched").state, "retry_safe");
        assert_eq!(intent("changed").state, "needs_recovery");
        assert_eq!(intent("live::operation").state, "applying");
        assert_eq!(fs::read(&changed).unwrap(), b"external change");
        assert_eq!(
            fs::read(intent("changed").backup_path.as_ref().unwrap()).unwrap(),
            b"before"
        );
        assert!(store
            .claim_desktop_run(&root, "fresh")
            .unwrap_err()
            .contains("recovery"));
        assert!(store.claim_desktop_run(&other_album, "unaffected").is_ok());
        fs::write(&untouched, b"new legitimate attempt").unwrap();
        fs::write(&changed, b"before").unwrap();
        store.reconcile_writes_at_startup().unwrap();
        let reconciled = store.reconcile_write_intents().unwrap();
        assert!(reconciled.iter().any(|entry| {
            entry.intent.run_id == "changed" && entry.action == WriteRecoveryAction::RetrySafe
        }));
        assert!(reconciled.iter().any(|entry| {
            entry.intent.run_id == "untouched" && entry.action == WriteRecoveryAction::Superseded
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn keeping_one_recovery_file_preserves_siblings_backups_and_live_owners() {
        let root = root();
        let album = root.join("album");
        fs::create_dir_all(&album).unwrap();
        let first = album.join("one.bin");
        let second = album.join("two.bin");
        let store = WorkerStore::new_in(root.join("state"));
        assert!(store.initialize());
        for (path, attempt) in [(&first, "run::one"), (&second, "run::two")] {
            fs::write(path, b"original").unwrap();
            store
                .prepare_write_intent(path, attempt, &serde_json::json!({"genre":"Jazz"}))
                .unwrap();
            store.begin_write_intent(path, attempt).unwrap();
            fs::write(path, b"changed").unwrap();
        }
        store.reconcile_writes_at_startup().unwrap();
        let first = fs::canonicalize(first).unwrap();
        let second = fs::canonicalize(second).unwrap();
        let digest = WorkerStore::recovery_file_digest(&first).unwrap();
        let backups: Vec<_> = store
            .reconcile_write_intents()
            .unwrap()
            .into_iter()
            .map(|entry| entry.intent.backup_path.unwrap())
            .collect();
        {
            let guard = store.inner.lock().unwrap();
            let connection = guard.as_ref().unwrap();
            connection
                .execute(
                    "INSERT INTO worker_items (path, trigger, status, run_id, owner, updated_at)
                 VALUES (?1, 'desktop', 'running', 'run', ?2, 1)",
                    params![
                        storage_key(&album),
                        format!("desktop:{}", std::process::id())
                    ],
                )
                .unwrap();
        }
        assert!(store.keep_recovery_file("run", &first, &digest).is_err());
        {
            let guard = store.inner.lock().unwrap();
            guard
                .as_ref()
                .unwrap()
                .execute("UPDATE worker_items SET status='failed'", [])
                .unwrap();
        }
        let second_connection = WorkerStore::new_in(root.join("state"));
        assert!(second_connection.initialize());
        store.keep_recovery_file("run", &first, &digest).unwrap();
        {
            let guard = store.inner.lock().unwrap();
            let stored: (String, String) = guard.as_ref().unwrap().query_row(
                "SELECT action, observed_sha256 FROM write_recovery_decisions WHERE revision='run' AND path=?1",
                [storage_key(&first)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert_eq!(stored, ("kept_current".to_owned(), digest.clone()));
        }
        assert!(second_connection
            .keep_recovery_file("run", &first, &digest)
            .is_err());
        assert_eq!(fs::read(&first).unwrap(), b"changed");
        assert_eq!(fs::read(&second).unwrap(), b"changed");
        assert!(backups
            .iter()
            .all(|path| fs::read(path).unwrap() == b"original"));
        assert!(second_connection
            .claim_desktop_run(&album, "next")
            .unwrap_err()
            .contains("recovery"));
        second_connection
            .keep_recovery_file("run", &second, &digest)
            .unwrap();
        assert!(second_connection.claim_desktop_run(&album, "next").is_ok());
        drop(store);
        drop(second_connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn recovery_rejects_retargeted_file_and_legacy_reverting_intents() {
        use std::os::unix::fs::symlink;
        let root = root();
        let file = root.join("track.bin");
        let replacement = root.join("replacement.bin");
        fs::write(&file, b"original").unwrap();
        fs::write(&replacement, b"changed").unwrap();
        let store = WorkerStore::new_in(root.join("state"));
        assert!(store.initialize());
        store
            .prepare_write_intent(&file, "run::one", &serde_json::json!({"genre":"Jazz"}))
            .unwrap();
        store.begin_write_intent(&file, "run::one").unwrap();
        fs::write(&file, b"changed").unwrap();
        store.reconcile_writes_at_startup().unwrap();
        let path = fs::canonicalize(&file).unwrap();
        let digest = WorkerStore::recovery_file_digest(&path).unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&replacement, &path).unwrap();
        assert!(store.keep_recovery_file("run", &path, &digest).is_err());
        assert_eq!(fs::read(&replacement).unwrap(), b"changed");
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"changed").unwrap();
        store
            .mark_review_write_intents_reverting(&path, "run::")
            .unwrap();
        assert!(!store.recovery_file_can_be_kept("run", &path).unwrap());
        assert!(store.keep_recovery_file("run", &path, &digest).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn startup_requires_known_post_state_metadata_and_unchanged_audio() {
        use crate::commands::mutations::{track_payload_digest, write_track_queued};
        use crate::state::write_queue::WriteQueue;

        let root = root();
        let store = WorkerStore::new_in(root.join("state"));
        assert!(store.initialize());
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/writer-corpus/padded.flac");
        let expected = serde_json::json!({"genre":"Jazz"});
        let queue = WriteQueue::default();
        for (id, genre) in [
            ("unknown", "Jazz"),
            ("different-tag", "Pop"),
            ("different-audio", "Jazz"),
        ] {
            let path = root.join(format!("{id}.flac"));
            fs::copy(&fixture, &path).unwrap();
            let intent = store.prepare_write_intent(&path, id, &expected).unwrap();
            store.begin_write_intent(&path, id).unwrap();
            write_track_queued(
                &queue,
                path.clone(),
                serde_json::from_value(serde_json::json!({"genre":genre})).unwrap(),
            )
            .await
            .unwrap();
            if id == "different-audio" {
                let mut bytes = fs::read(&path).unwrap();
                *bytes.last_mut().unwrap() ^= 1;
                fs::write(&path, bytes).unwrap();
                assert_ne!(
                    Some(track_payload_digest(&path).unwrap()),
                    intent.before_payload_sha256
                );
                assert_eq!(
                    crate::commands::auto_tag_review::track_view(&path).unwrap()["genre"],
                    "Jazz"
                );
            }
            if id != "unknown" {
                store.record_write_intent_applied(&path, id, true).unwrap();
            }
            let current = fs::read(&path).unwrap();
            store.reconcile_writes_at_startup().unwrap();
            assert_eq!(fs::read(&path).unwrap(), current);
            assert_eq!(
                fs::read(intent.backup_path.unwrap()).unwrap(),
                fs::read(&fixture).unwrap()
            );
        }
        let recovered = store.reconcile_write_intents().unwrap();
        assert_eq!(recovered.len(), 3);
        assert!(recovered
            .iter()
            .all(|entry| entry.intent.state == "needs_recovery"));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn write_intents_reject_a_symlinked_recovery_directory() {
        use std::os::unix::fs::symlink;
        let root = root();
        let outside = root.join("outside");
        let file = root.join("track.bin");
        fs::create_dir_all(&outside).unwrap();
        fs::write(&file, b"before").unwrap();
        symlink(&outside, root.join("write-recovery")).unwrap();
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        assert!(store
            .prepare_write_intent(&file, "run", &serde_json::json!({"genre":"Jazz"}))
            .is_err());
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_directory_parent_sync_failure_prevents_intent_preparation() {
        let root = root();
        let file = root.join("track.bin");
        fs::write(&file, b"before").unwrap();
        let store = WorkerStore::new_in(root.join("state"));
        assert!(store.initialize());

        let result = store.prepare_write_intent_with_parent_sync(
            &file,
            "run",
            &serde_json::json!({"genre":"Jazz"}),
            |parent| {
                assert_eq!(parent, store.data_dir());
                Err("injected parent directory sync failure".into())
            },
        );

        assert!(result
            .unwrap_err()
            .contains("injected parent directory sync failure"));
        assert_eq!(fs::read(&file).unwrap(), b"before");
        assert_eq!(store.reconcile_write_intents().unwrap().len(), 0);
        assert_eq!(
            fs::read_dir(store.data_dir().join("write-recovery"))
                .unwrap()
                .count(),
            0
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn recovery_directory_sync_opens_windows_directories_for_flush() {
        let root = root();
        assert!(sync_recovery_directory(&root).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn write_intent_keys_collapse_parent_directory_aliases() {
        use std::os::unix::fs::symlink;
        let root = root();
        let real = root.join("real");
        fs::create_dir_all(&real).unwrap();
        let file = real.join("track.bin");
        fs::write(&file, b"before").unwrap();
        let alias = root.join("alias");
        symlink(&real, &alias).unwrap();
        let aliased_file = alias.join("track.bin");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        let patch = serde_json::json!({"genre":"Jazz"});
        store
            .prepare_write_intent(&file, "same-run", &patch)
            .unwrap();
        store
            .prepare_write_intent(&aliased_file, "same-run", &patch)
            .unwrap();
        assert_eq!(store.reconcile_write_intents().unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_does_not_displace_a_live_desktop_run() {
        let root = root();
        let album = root.join("Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store.claim_desktop_run(&album, "desktop-live").unwrap();
        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        assert_eq!(reopened.recover_interrupted().unwrap(), 0);
        assert!(reopened.claim_desktop_run(&album, "conflict").is_err());
        assert!(reopened.mark_running(&album, "server-conflict").is_err());
        assert!(reopened.pending_result().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_worker_schema_gains_nullable_runtime_owner_without_rewriting_rows() {
        let root = root();
        let database = Connection::open(root.join("worker.db")).unwrap();
        database.execute_batch("CREATE TABLE worker_items (path TEXT PRIMARY KEY, trigger TEXT NOT NULL, status TEXT NOT NULL, run_id TEXT, reason TEXT, authority TEXT, updated_at INTEGER NOT NULL); INSERT INTO worker_items VALUES ('legacy','watch','running','old',NULL,NULL,123);").unwrap();
        drop(database);
        for _ in 0..2 {
            let store = WorkerStore::new_in(root.clone());
            assert!(store.initialize());
        }
        let database = Connection::open(root.join("worker.db")).unwrap();
        let row = database
            .query_row(
                "SELECT trigger,status,run_id,updated_at,owner FROM worker_items",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            row,
            ("watch".into(), "running".into(), "old".into(), 123, None)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn server_claim_replaces_abandoned_desktop_runtime_owner_not_trigger() {
        use std::process::{Command, Stdio};
        let root = root();
        let album = root.join("Album");
        let mut child = Command::new("sh")
            .args(["-c", "read line"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(
                std::slice::from_ref(&album),
                &format!("desktop:{}", child.id()),
            )
            .unwrap();
        store.inner.lock().unwrap().as_ref().unwrap().execute(
            "UPDATE worker_items SET status='running',run_id='old-desktop',owner=?2 WHERE path=?1",
            params![storage_key(&album), format!("desktop:{}", child.id())],
        ).unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(store.recover_interrupted().unwrap(), 1);
        store.mark_running(&album, "server-live").unwrap();
        assert!(store.claim_desktop_run(&album, "must-not-steal").is_err());
        assert_eq!(store.recover_interrupted().unwrap(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn abandoned_desktop_owner_can_retry_without_reclaiming_live_or_unknown_work() {
        use std::process::{Command, Stdio};
        for pending in [true, false] {
            let root = root();
            let album = root.join("Album");
            fs::create_dir(&album).unwrap();
            let mut child = Command::new("sh")
                .args(["-c", "read line"])
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            let trigger = format!("desktop:{}", child.id());
            let store = WorkerStore::new_in(root.clone());
            assert!(store.initialize());
            store
                .enqueue(std::slice::from_ref(&album), &trigger)
                .unwrap();
            if !pending {
                store.mark_running(&album, "abandoned").unwrap();
            }
            // The child represents the process recorded by this isolated row.
            store
                .inner
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .execute(
                    "UPDATE worker_items SET owner=?2 WHERE path=?1",
                    params![storage_key(&album), trigger],
                )
                .unwrap();
            store
                .checkpoint_evidence(
                    &album,
                    "abandoned",
                    &serde_json::json!({"pendingCoreReceipts":["diagnostic"]}),
                )
                .unwrap();
            assert!(store.claim_desktop_run(&album, "live-conflict").is_err());
            child.kill().unwrap();
            child.wait().unwrap();
            drop(store);
            let reopened = WorkerStore::new_in(root.clone());
            assert!(reopened.initialize());
            reopened
                .claim_desktop_run(&album, "retry")
                .expect("explicit retry must revalidate an abandoned desktop source");
            assert!(reopened
                .claim_desktop_run(&album, "second-live-conflict")
                .is_err());
            let foreign = root.join("Foreign");
            reopened
                .enqueue(std::slice::from_ref(&foreign), "manual")
                .unwrap();
            assert!(reopened
                .claim_desktop_run(&foreign, "unknown-owner")
                .is_err());
            assert!(reopened.evidence(&album, "abandoned").unwrap().is_some());
            drop(reopened);
            fs::remove_dir_all(root).unwrap();
        }
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
    fn pending_work_is_deduplicated_and_legacy_running_work_recovers_after_restart() {
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
        // Legacy rows have no recorded process owner; preserve their existing recovery contract.
        store
            .inner
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .execute(
                "UPDATE worker_items SET owner=NULL WHERE path=?1",
                [storage_key(&album)],
            )
            .unwrap();
        drop(store);

        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        assert_eq!(reopened.recover_interrupted().unwrap(), 1);
        assert_eq!(reopened.pending(), vec![album]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminal_core_commit_merges_progress_without_publishing_success_early() {
        let root = root();
        let album = root.join("Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        store.mark_running(&album, "run").unwrap();
        store
            .record_review_flag(&album, "recording_identity", "run")
            .unwrap();
        store
            .checkpoint_evidence(
                &album,
                "run",
                &serde_json::json!({"reports":["provider progress"]}),
            )
            .unwrap();
        assert!(store
            .evidence(&album, "run")
            .unwrap()
            .unwrap()
            .get("completionBasis")
            .is_none());
        store
            .finish_with_evidence(
                &album,
                "run",
                "completed",
                None,
                None,
                &serde_json::json!({"outcome":"applied","completionBasis":"verified_track_core"}),
            )
            .unwrap();
        let saved = store.evidence(&album, "run").unwrap().unwrap();
        assert_eq!(
            saved["reports"],
            serde_json::json!(["provider progress"]),
            "progress survives the one atomic terminal commit"
        );
        assert_eq!(saved["completionBasis"], "verified_track_core");
        assert!(store.unreviewed_review_paths().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verified_core_completion_closes_only_its_own_recording_review() {
        let root = root();
        let album = root.join("Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        store.mark_running(&album, "run").unwrap();
        store
            .record_review_flag(&album, "recording_identity", "run")
            .unwrap();
        let evidence = serde_json::json!({"completionBasis":"verified_track_core"});
        store
            .finish_with_evidence(
                &album,
                "run",
                "completed",
                None,
                Some("musicbrainz"),
                &evidence,
            )
            .unwrap();
        assert!(
            store.unreviewed_review_paths().unwrap().is_empty(),
            "verified core must not remain in the recording-review bucket"
        );
        assert_eq!(store.evidence(&album, "run").unwrap(), Some(evidence));
        drop(store);
        let reopened = WorkerStore::new_in(root.clone());
        assert!(reopened.initialize());
        assert!(reopened.unreviewed_review_paths().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn core_completion_preserves_other_review_scopes_and_rolls_back_on_checkpoint_failure() {
        for (status, basis, reason, flag_run) in [
            ("completed", false, "recording_identity", "run"),
            ("needs_review", true, "recording_identity", "run"),
            ("failed", true, "recording_identity", "run"),
            ("completed", true, "recording_identity", "other"),
            ("completed", true, "ai_fallback", "run"),
        ] {
            let root = root();
            let album = root.join("Album");
            let store = WorkerStore::new_in(root.clone());
            assert!(store.initialize());
            store
                .enqueue(std::slice::from_ref(&album), "manual")
                .unwrap();
            store.mark_running(&album, "run").unwrap();
            store.record_review_flag(&album, reason, flag_run).unwrap();
            let evidence = if basis {
                serde_json::json!({"completionBasis":"verified_track_core"})
            } else {
                serde_json::json!({})
            };
            store
                .finish_with_evidence(&album, "run", status, None, None, &evidence)
                .unwrap();
            assert_eq!(store.unreviewed_review_paths().unwrap(), vec![album]);
            fs::remove_dir_all(root).unwrap();
        }
        let root = root();
        let album = root.join("Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        store.mark_running(&album, "run").unwrap();
        store
            .record_review_flag(&album, "recording_identity", "run")
            .unwrap();
        {
            let guard = store.inner.lock().unwrap();
            guard.as_ref().unwrap().execute_batch("CREATE TRIGGER reject_core_evidence BEFORE INSERT ON worker_evidence BEGIN SELECT RAISE(ABORT, 'checkpoint blocked'); END;").unwrap();
        }
        assert!(store
            .finish_with_evidence(
                &album,
                "run",
                "completed",
                None,
                None,
                &serde_json::json!({"completionBasis":"verified_track_core"})
            )
            .is_err());
        assert_eq!(
            store.unreviewed_review_paths().unwrap(),
            vec![album.clone()]
        );
        assert!(store.evidence(&album, "run").unwrap().is_none());
        let status: String = store
            .inner
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .query_row("SELECT status FROM worker_items", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            status, "running",
            "failed evidence persistence must not publish completion"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recording_review_cannot_replace_an_open_ai_obligation() {
        let root = root();
        let album = root.join("Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .record_review_flag(&album, "ai_fallback", "prior")
            .unwrap();
        store
            .record_review_flag(&album, "recording_identity", "current")
            .unwrap();
        let guard = store.inner.lock().unwrap();
        let reason: String = guard
            .as_ref()
            .unwrap()
            .query_row("SELECT reason FROM review_flags", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            reason, "ai_fallback",
            "fresh track proof does not acknowledge a separate AI review"
        );
        drop(guard);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_core_completion_cannot_replace_a_newer_running_attempt() {
        let root = root();
        let album = root.join("Album");
        let store = WorkerStore::new_in(root.clone());
        assert!(store.initialize());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        store.mark_running(&album, "new").unwrap();
        store
            .record_review_flag(&album, "recording_identity", "old")
            .unwrap();
        assert!(store
            .finish_with_evidence(
                &album,
                "old",
                "completed",
                None,
                None,
                &serde_json::json!({"completionBasis":"verified_track_core"})
            )
            .is_err());
        assert_eq!(
            store.unreviewed_review_paths().unwrap(),
            vec![album.clone()]
        );
        assert!(store.evidence(&album, "old").unwrap().is_none());
        // The manual caller reports a rejected completion as a failed attempt.
        // Neither terminal path may reclaim another run's scheduling row.
        assert!(store
            .finish_with_evidence(
                &album,
                "old",
                "failed",
                Some("completion checkpoint failed"),
                None,
                &serde_json::json!({"error":"completion checkpoint failed"}),
            )
            .is_err());
        assert!(store.evidence(&album, "old").unwrap().is_none());
        store
            .finish_with_evidence(
                &album,
                "new",
                "completed",
                None,
                None,
                &serde_json::json!({"completionBasis":"verified_track_core"}),
            )
            .unwrap();
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
        store
            .inner
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .execute("UPDATE worker_items SET owner=NULL", [])
            .unwrap();
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
