//! Structured session-state persistence for the AI assistant harness.
//!
//! Additive SQLite tables (CREATE TABLE IF NOT EXISTS):
//!   - `assistant_session` — current resolved intent, scope predicate, protocol, referents.
//!   - `assistant_tool_call` — native tool calls/results with stable call IDs.
//!   - `assistant_batch` — pending/preview batches with status.
//!
//! All tables coexist with the existing `conversation_log` schema in the same
//! `cache.db` database. Only additive changes are made to existing tables: a
//! new `assistant_session` column is added via `ALTER TABLE ... ADD COLUMN`
//! when a legacy database lacks it (see `migrate_clarification_column`).

use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;

const TASK_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS assistant_session (
  session_id TEXT PRIMARY KEY,
  intent TEXT,
  scope_predicate TEXT,
  scope_predicate_json TEXT,
  protocol TEXT NOT NULL DEFAULT 'native',
  referent_count INTEGER DEFAULT 0,
  referent_query TEXT,
  referent_field TEXT,
  referent_value TEXT,
  pending_batch_ids TEXT DEFAULT '',
  mutation_required INTEGER NOT NULL DEFAULT 0,
  consecutive_clarifications INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS assistant_tool_call (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id TEXT NOT NULL,
  call_id TEXT NOT NULL,
  step_number INTEGER NOT NULL,
  canonical_name TEXT NOT NULL,
  arguments_json TEXT,
  result_json TEXT,
  ok INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_atc_session ON assistant_tool_call(session_id);
CREATE INDEX IF NOT EXISTS idx_atc_call_id ON assistant_tool_call(call_id);

CREATE TABLE IF NOT EXISTS assistant_batch (
  batch_id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  batch_json TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  action_count INTEGER NOT NULL DEFAULT 0,
  readback_json TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_ab_session ON assistant_batch(session_id);
CREATE INDEX IF NOT EXISTS idx_ab_status ON assistant_batch(status);
"#;

/// Structured scope predicate for deterministic routing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScopePredicate {
    /// The user's current selection.
    Selected,
    /// The active album.
    ActiveAlbum,
    /// The entire loaded library.
    Library,
    /// Library tracks where a specific field is missing/empty.
    LibraryAndMissing { field: String },
    /// Album tracks where a specific field is missing/empty.
    AlbumAndMissing { field: String },
    /// Explicit paths (from a previous result).
    ExplicitPaths(Vec<String>),
    /// Persisted referent from a previous result (tracks matching a query).
    Referent {
        /// Number of tracks in the referent.
        count: usize,
        /// The query that produced this referent (e.g. "genre missing").
        query: String,
        /// The field involved.
        field: Option<String>,
        /// A hash to detect staleness (e.g. based on track paths).
        hash: String,
    },
}

impl ScopePredicate {
    /// Convert to a human-readable string for logging/persisting.
    pub fn as_str(&self) -> &str {
        match self {
            ScopePredicate::Selected => "selected",
            ScopePredicate::ActiveAlbum => "active_album",
            ScopePredicate::Library => "library",
            ScopePredicate::LibraryAndMissing { .. } => "library_missing",
            ScopePredicate::AlbumAndMissing { .. } => "album_missing",
            ScopePredicate::ExplicitPaths(_) => "explicit_paths",
            ScopePredicate::Referent { .. } => "referent",
        }
    }

    /// The field this predicate targets, if any.
    pub fn field(&self) -> Option<&str> {
        match self {
            ScopePredicate::LibraryAndMissing { field } => Some(field.as_str()),
            ScopePredicate::AlbumAndMissing { field } => Some(field.as_str()),
            _ => None,
        }
    }

    /// Serialize to JSON for persistence.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    /// Deserialize from JSON.
    pub fn from_json(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }
}

/// Resolved deterministic intent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResolvedIntent {
    /// Set a field to a literal value across the scope.
    SetField {
        field: String,
        value: String,
        only_if_missing: bool,
    },
    /// Derive a field from a native per-track source (e.g. the containing
    /// folder) instead of a literal value.
    SetFieldFrom { field: String, source: String },
    /// Remove/clear a field across the scope.
    RemoveField { field: String },
    /// Could not determine intent — LLM should decide.
    NotRouted,
}

/// Structured session state record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub session_id: String,
    pub intent: Option<String>,
    pub scope_predicate: Option<Value>,
    pub protocol: String,
    pub referent_count: i64,
    pub referent_query: Option<String>,
    pub referent_field: Option<String>,
    pub referent_value: Option<String>,
    pub pending_batch_ids: Vec<String>,
    pub mutation_required: bool,
    /// Consecutive message-only clarification responses in this session.
    /// Reset to 0 whenever a turn produces a tool preview or a non-question answer.
    #[serde(default)]
    pub consecutive_clarifications: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl SessionState {
    pub fn referent(&self) -> Option<&str> {
        self.referent_value.as_deref()
    }
}

/// Map errors to strings for Result<_, String> surfaces.
type Result<T> = std::result::Result<T, String>;

/// Additive SQLite persistence for assistant session state.
///
/// Shares the configured cache path with `ConversationState` and `CacheState`
/// via the same `~/.auto-tagger/cache.db` database file.
pub struct AssistantTaskState {
    /// Path to the SQLite database.
    db_path: PathBuf,
    inner: Mutex<Option<Connection>>,
}

impl AssistantTaskState {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            db_path,
            inner: Mutex::new(None),
        }
    }

    /// Initialize the database connection; creates tables if they don't exist.
    /// Idempotent — safe to call multiple times.
    pub fn initialize(&self) -> Result<()> {
        let mut guard = self.inner.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Ok(());
        }
        let connection = Connection::open_with_flags(
            &self.db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|e| format!("Failed to open assistant task DB: {e}"))?;

        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| format!("Failed to set WAL mode: {e}"))?;

        connection
            .execute_batch(TASK_SCHEMA)
            .map_err(|e| format!("Failed to create assistant task tables: {e}"))?;

        migrate_clarification_column(&connection)?;

        *guard = Some(connection);
        Ok(())
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Option<Connection>>> {
        self.inner.lock().map_err(|e| e.to_string())
    }

    // ── Session state ──────────────────────────────────────────────

    /// Upsert a session state record (INSERT OR REPLACE).
    pub fn upsert_session(&self, state: &SessionState) -> Result<()> {
        let guard = self.conn()?;
        let conn = guard.as_ref().ok_or("AssistantTaskState not initialized")?;

        let pending_ids = state.pending_batch_ids.join(",");

        conn.execute(
            "INSERT OR REPLACE INTO assistant_session
             (session_id, intent, scope_predicate, scope_predicate_json, protocol,
              referent_count, referent_query, referent_field, referent_value,
              pending_batch_ids, mutation_required, consecutive_clarifications,
              created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                state.session_id,
                state.intent,
                state
                    .scope_predicate
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .unwrap_or_default(),
                state
                    .scope_predicate
                    .as_ref()
                    .map(Value::to_string)
                    .unwrap_or_default(),
                state.protocol,
                state.referent_count,
                state.referent_query,
                state.referent_field,
                state.referent_value,
                pending_ids,
                state.mutation_required as i64,
                state.consecutive_clarifications,
                state.created_at,
                state.updated_at,
            ],
        )
        .map(|_| ())
        .map_err(|e| format!("Failed to upsert session: {e}"))
    }

    /// Load a session state record.
    pub fn load_session(&self, session_id: &str) -> Option<SessionState> {
        let guard = self.conn().ok()?;
        let conn = guard.as_ref()?;

        let (
            intent,
            scope_json,
            protocol,
            referent_count,
            referent_query,
            referent_field,
            referent_value,
            pending_ids,
            mutation_required,
            consecutive_clarifications,
            created_at,
            updated_at,
        ): (
            Option<String>,
            String,
            String,
            i64,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            i64,
            i64,
            String,
            String,
        ) = conn
            .query_row(
                "SELECT intent, scope_predicate_json, protocol,
                    referent_count, referent_query, referent_field, referent_value,
                    pending_batch_ids, mutation_required, consecutive_clarifications,
                    created_at, updated_at
             FROM assistant_session WHERE session_id = ?1",
                [session_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get::<_, String>(1).unwrap_or_default(),
                        row.get::<_, String>(2).unwrap_or_default(),
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get::<_, String>(7).unwrap_or_default(),
                        row.get(8)?,
                        row.get::<_, i64>(9).unwrap_or_default(),
                        row.get::<_, String>(10).unwrap_or_default(),
                        row.get::<_, String>(11).unwrap_or_default(),
                    ))
                },
            )
            .ok()?;

        let scope_predicate = if scope_json.is_empty() || scope_json == "null" {
            None
        } else {
            serde_json::from_str(&scope_json).ok()
        };

        let pending_batch_ids: Vec<String> = if pending_ids.is_empty() {
            Vec::new()
        } else {
            pending_ids.split(',').map(str::to_string).collect()
        };

        Some(SessionState {
            session_id: session_id.to_string(),
            intent,
            scope_predicate,
            protocol,
            referent_count,
            referent_query,
            referent_field,
            referent_value,
            pending_batch_ids,
            mutation_required: mutation_required != 0,
            consecutive_clarifications,
            created_at,
            updated_at,
        })
    }

    /// Persist the consecutive-clarification counter without clobbering the
    /// deterministic router's referent/intent fields (INSERT OR REPLACE would
    /// wipe them). Creates the row if the LLM path has never written one.
    pub fn save_clarification_count(&self, session_id: &str, count: i64) -> Result<()> {
        let guard = self.conn()?;
        let conn = guard.as_ref().ok_or("AssistantTaskState not initialized")?;
        let now = iso_now();
        conn.execute(
            "INSERT INTO assistant_session
             (session_id, protocol, consecutive_clarifications, created_at, updated_at)
             VALUES (?1, 'llm', ?2, ?3, ?3)
             ON CONFLICT(session_id) DO UPDATE SET
               consecutive_clarifications = excluded.consecutive_clarifications,
               updated_at = excluded.updated_at",
            params![session_id, count, now],
        )
        .map(|_| ())
        .map_err(|e| format!("Failed to save clarification count: {e}"))
    }

    /// Load the most recent session state (for `them` referents across turns).
    /// Releases the mutex guard before calling `load_session` to avoid deadlock.
    pub fn load_latest_session(&self) -> Option<SessionState> {
        let session_id: String = {
            let guard = self.conn().ok()?;
            let conn = guard.as_ref()?;
            conn.query_row(
                "SELECT session_id FROM assistant_session ORDER BY updated_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok()?
        };
        // guard is dropped; load_session can acquire its own lock.
        self.load_session(&session_id)
    }

    // ── Tool calls ─────────────────────────────────────────────────

    /// Record a tool call.
    pub fn record_tool_call(
        &self,
        session_id: &str,
        call_id: &str,
        step_number: usize,
        canonical_name: &str,
        arguments: &Value,
    ) -> Result<()> {
        let guard = self.conn()?;
        let conn = guard.as_ref().ok_or("AssistantTaskState not initialized")?;

        let now = iso_now();

        conn.execute(
            "INSERT INTO assistant_tool_call
             (session_id, call_id, step_number, canonical_name, arguments_json, ok, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
            params![
                session_id,
                call_id,
                step_number as i64,
                canonical_name,
                arguments.to_string(),
                now,
            ],
        )
        .map(|_| ())
        .map_err(|e| format!("Failed to record tool call: {e}"))
    }

    /// Update a tool call with its result.
    pub fn record_tool_result(
        &self,
        session_id: &str,
        call_id: &str,
        ok: bool,
        result_json: &Value,
    ) -> Result<()> {
        let guard = self.conn()?;
        let conn = guard.as_ref().ok_or("AssistantTaskState not initialized")?;

        conn.execute(
            "UPDATE assistant_tool_call
             SET result_json = ?1, ok = ?2
             WHERE session_id = ?3 AND call_id = ?4",
            params![result_json.to_string(), ok as i64, session_id, call_id],
        )
        .map(|_| ())
        .map_err(|e| format!("Failed to record tool result: {e}"))
    }

    // ── Batches ────────────────────────────────────────────────────

    /// Persist a batch.
    pub fn save_batch(
        &self,
        batch_id: &str,
        session_id: &str,
        batch_json: &Value,
        action_count: usize,
    ) -> Result<()> {
        let guard = self.conn()?;
        let conn = guard.as_ref().ok_or("AssistantTaskState not initialized")?;

        let now = iso_now();

        conn.execute(
            "INSERT OR REPLACE INTO assistant_batch
             (batch_id, session_id, batch_json, status, action_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6)",
            params![
                batch_id,
                session_id,
                batch_json.to_string(),
                action_count as i64,
                now.clone(),
                now,
            ],
        )
        .map(|_| ())
        .map_err(|e| format!("Failed to save batch: {e}"))
    }

    /// Update batch status.
    pub fn update_batch_status(&self, batch_id: &str, status: &str) -> Result<()> {
        let guard = self.conn()?;
        let conn = guard.as_ref().ok_or("AssistantTaskState not initialized")?;

        conn.execute(
            "UPDATE assistant_batch
             SET status = ?1, updated_at = ?2
             WHERE batch_id = ?3",
            params![status, iso_now(), batch_id],
        )
        .map(|_| ())
        .map_err(|e| format!("Failed to update batch status: {e}"))
    }

    /// Record readback evidence for a batch.
    pub fn record_batch_readback(&self, batch_id: &str, readback_json: &Value) -> Result<()> {
        let guard = self.conn()?;
        let conn = guard.as_ref().ok_or("AssistantTaskState not initialized")?;

        conn.execute(
            "UPDATE assistant_batch
             SET readback_json = ?1, updated_at = ?2
             WHERE batch_id = ?3",
            params![readback_json.to_string(), iso_now(), batch_id],
        )
        .map(|_| ())
        .map_err(|e| format!("Failed to record batch readback: {e}"))
    }

    /// Atomically persist a terminal batch status and its readback evidence.
    pub fn finalize_batch(
        &self,
        batch_id: &str,
        status: &str,
        readback_json: &Value,
    ) -> Result<()> {
        let guard = self.conn()?;
        let conn = guard.as_ref().ok_or("AssistantTaskState not initialized")?;
        let updated = conn
            .execute(
                "UPDATE assistant_batch
                 SET status = ?1, readback_json = ?2, updated_at = ?3
                 WHERE batch_id = ?4",
                params![status, readback_json.to_string(), iso_now(), batch_id],
            )
            .map_err(|e| format!("Failed to finalize assistant batch: {e}"))?;
        if updated == 0 {
            return Err(format!("Assistant batch not found: {batch_id}"));
        }
        Ok(())
    }

    /// Load all pending batches for a session.
    pub fn load_pending_batches(&self, session_id: &str) -> Vec<(String, Value)> {
        let guard = self.conn().ok();
        let conn = guard.as_ref().and_then(|c| c.as_ref());
        let Some(conn) = conn else { return Vec::new() };

        let mut stmt = match conn.prepare(
            "SELECT batch_id, batch_json, status
             FROM assistant_batch
             WHERE session_id = ?1 AND status = 'pending'
             ORDER BY created_at ASC",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };

        let results: Vec<(String, Value)> = stmt
            .query_map(params![session_id], |row| {
                let id: String = row.get(0)?;
                let json_str: String = row.get(1)?;
                let json: Value = serde_json::from_str(&json_str).unwrap_or_default();
                Ok((id, json))
            })
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect();

        results
    }

    /// Load all pending batches across all sessions (for runtime hydration on restart).
    pub fn load_all_pending_batches(&self) -> Vec<(String, String, Value)> {
        let guard = self.conn().ok();
        let conn = guard.as_ref().and_then(|c| c.as_ref());
        let Some(conn) = conn else { return Vec::new() };

        let mut stmt = match conn.prepare(
            "SELECT batch_id, session_id, batch_json
             FROM assistant_batch
             WHERE status = 'pending'
             ORDER BY created_at ASC",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };

        let results: Vec<(String, String, Value)> = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let session: String = row.get(1)?;
                let json_str: String = row.get(2)?;
                let json: Value = serde_json::from_str(&json_str).unwrap_or_default();
                Ok((id, session, json))
            })
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect();

        results
    }

    /// Get batch status.
    pub fn batch_status(&self, batch_id: &str) -> Option<String> {
        let guard = self.conn().ok()?;
        let conn = guard.as_ref()?;
        conn.query_row(
            "SELECT status FROM assistant_batch WHERE batch_id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .ok()
    }
}

pub fn iso_now() -> String {
    let value = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        value.year(),
        u8::from(value.month()),
        value.day(),
        value.hour(),
        value.minute(),
        value.second(),
        value.millisecond(),
    )
}

/// Add the `consecutive_clarifications` column to pre-existing databases.
/// `CREATE TABLE IF NOT EXISTS` cannot alter an existing table, so legacy
/// `cache.db` files need an explicit additive migration.
fn migrate_clarification_column(connection: &Connection) -> Result<()> {
    let columns: Vec<String> = connection
        .prepare("PRAGMA table_info(assistant_session)")
        .map_err(|e| format!("Failed to inspect assistant_session schema: {e}"))?
        .query_map([], |row| row.get(1))
        .map_err(|e| format!("Failed to read assistant_session schema: {e}"))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read assistant_session schema: {e}"))?;
    if !columns.iter().any(|column| column == "consecutive_clarifications") {
        connection
            .execute_batch(
                "ALTER TABLE assistant_session ADD COLUMN consecutive_clarifications INTEGER NOT NULL DEFAULT 0",
            )
            .map_err(|e| format!("Failed to add consecutive_clarifications column: {e}"))?;
    }
    Ok(())
}

// ── Predicate evaluation ───────────────────────────────────────────

/// Evaluate a scope predicate against the currently loaded tracks.
/// Returns the list of matching track paths and the matching count.
pub fn evaluate_predicate(
    predicate: &ScopePredicate,
    tracks: &[Value],
    active_album: Option<&str>,
    selected: &[String],
) -> (Vec<String>, usize) {
    match predicate {
        ScopePredicate::Selected => (selected.to_vec(), selected.len()),
        ScopePredicate::ActiveAlbum => {
            let paths: Vec<String> = tracks
                .iter()
                .filter_map(|t| t.get("path").and_then(Value::as_str))
                .filter(|p| active_album.is_some_and(|album| p.starts_with(album)))
                .map(str::to_string)
                .collect();
            let count = paths.len();
            (paths, count)
        }
        ScopePredicate::Library => {
            let paths: Vec<String> = tracks
                .iter()
                .filter_map(|t| t.get("path").and_then(Value::as_str))
                .map(str::to_string)
                .collect();
            let count = paths.len();
            (paths, count)
        }
        ScopePredicate::LibraryAndMissing { field } => {
            let paths: Vec<String> = tracks
                .iter()
                .filter(|t| {
                    let val = t.get(field.as_str()).and_then(Value::as_str);
                    val.is_none_or(|v| v.trim().is_empty())
                })
                .filter_map(|t| t.get("path").and_then(Value::as_str))
                .map(str::to_string)
                .collect();
            let count = paths.len();
            (paths, count)
        }
        ScopePredicate::AlbumAndMissing { field } => {
            let paths: Vec<String> = tracks
                .iter()
                .filter(|t| {
                    let in_album = active_album.is_some_and(|album| {
                        t.get("path")
                            .and_then(Value::as_str)
                            .is_some_and(|p| p.starts_with(album))
                    });
                    in_album && {
                        let val = t.get(field.as_str()).and_then(Value::as_str);
                        val.is_none_or(|v| v.trim().is_empty())
                    }
                })
                .filter_map(|t| t.get("path").and_then(Value::as_str))
                .map(str::to_string)
                .collect();
            let count = paths.len();
            (paths, count)
        }
        ScopePredicate::ExplicitPaths(paths) => {
            let count = paths.len();
            (paths.clone(), count)
        }
        ScopePredicate::Referent { .. } => {
            // Referents need to be resolved from stored paths; return empty here
            // and let the caller resolve from session state.
            (Vec::new(), 0)
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn db_path() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "soundrobe-assistant-task-{}-{}.db",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        // Clean up from previous runs if any
        let _ = fs::remove_file(&path);
        path
    }

    fn setup() -> (AssistantTaskState, String) {
        let path = db_path();
        let state = AssistantTaskState::new(path);
        state.initialize().unwrap();
        let session_id = format!("session-test-{}", SEQUENCE.fetch_add(1, Ordering::Relaxed));
        (state, session_id)
    }

    #[test]
    fn tables_created_and_idempotent_init() {
        let path = db_path();
        let state = AssistantTaskState::new(path.clone());
        state.initialize().unwrap();
        state.initialize().unwrap(); // second call must succeed

        // Verify tables exist
        let conn = Connection::open(&path).unwrap();
        for table in [
            "assistant_session",
            "assistant_tool_call",
            "assistant_batch",
        ] {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "table {table} should exist");
        }
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn session_round_trips() {
        let (state, session_id) = setup();

        let record = SessionState {
            session_id: session_id.clone(),
            intent: Some("set_field".to_string()),
            scope_predicate: Some(serde_json::json!({
                "type": "LibraryAndMissing",
                "field": "genre"
            })),
            protocol: "native".to_string(),
            referent_count: 102,
            referent_query: Some("missing genre".to_string()),
            referent_field: Some("genre".to_string()),
            referent_value: Some("Pop, Cantopop".to_string()),
            pending_batch_ids: vec!["batch-1".to_string()],
            mutation_required: true,
            consecutive_clarifications: 0,
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            updated_at: "2026-01-01T00:00:00.000Z".to_string(),
        };

        state.upsert_session(&record).unwrap();

        let loaded = state.load_session(&session_id).unwrap();
        assert_eq!(loaded.intent, Some("set_field".to_string()));
        assert_eq!(loaded.referent_count, 102);
        assert_eq!(loaded.referent_query, Some("missing genre".to_string()));
        assert_eq!(loaded.referent_value, Some("Pop, Cantopop".to_string()));
        assert_eq!(loaded.pending_batch_ids, vec!["batch-1"]);
        assert!(loaded.mutation_required);
    }

    #[test]
    fn clarification_count_persists_without_clobbering_referent_fields() {
        let (state, session_id) = setup();

        let record = SessionState {
            session_id: session_id.clone(),
            intent: Some("set_field".to_string()),
            scope_predicate: None,
            protocol: "native".to_string(),
            referent_count: 7,
            referent_query: Some("missing album".to_string()),
            referent_field: Some("album".to_string()),
            referent_value: Some("红昭愿".to_string()),
            pending_batch_ids: vec![],
            mutation_required: true,
            consecutive_clarifications: 0,
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            updated_at: "2026-01-01T00:00:00.000Z".to_string(),
        };
        state.upsert_session(&record).unwrap();

        state.save_clarification_count(&session_id, 2).unwrap();
        let loaded = state.load_session(&session_id).unwrap();
        assert_eq!(loaded.consecutive_clarifications, 2);
        // The deterministic router's referent fields must survive the LLM-path update.
        assert_eq!(loaded.referent_value.as_deref(), Some("红昭愿"));
        assert_eq!(loaded.referent_query.as_deref(), Some("missing album"));
        assert_eq!(loaded.intent.as_deref(), Some("set_field"));
        assert_eq!(loaded.protocol, "native");
    }

    #[test]
    fn clarification_count_creates_llm_path_row_when_absent() {
        let (state, session_id) = setup();
        assert!(state.load_session(&session_id).is_none());

        state.save_clarification_count(&session_id, 1).unwrap();
        let loaded = state.load_session(&session_id).unwrap();
        assert_eq!(loaded.consecutive_clarifications, 1);
        assert_eq!(loaded.protocol, "llm");
    }

    #[test]
    fn legacy_schema_gains_clarification_column_on_initialize() {
        let path = db_path();
        // Build a DB with the pre-column assistant_session schema.
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE assistant_session (
                   session_id TEXT PRIMARY KEY,
                   intent TEXT,
                   scope_predicate TEXT,
                   scope_predicate_json TEXT,
                   protocol TEXT NOT NULL DEFAULT 'native',
                   referent_count INTEGER DEFAULT 0,
                   referent_query TEXT,
                   referent_field TEXT,
                   referent_value TEXT,
                   pending_batch_ids TEXT DEFAULT '',
                   mutation_required INTEGER NOT NULL DEFAULT 0,
                   created_at TEXT NOT NULL,
                   updated_at TEXT NOT NULL
                 );",
            )
            .unwrap();
        drop(connection);

        let state = AssistantTaskState::new(path.clone());
        state.initialize().unwrap();

        let conn = Connection::open(&path).unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(assistant_session)").unwrap();
        let rows: std::result::Result<Vec<String>, _> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect();
        let has_column = rows
            .unwrap()
            .iter()
            .any(|name| name == "consecutive_clarifications");
        assert!(has_column, "migration must add the column");
        drop(stmt);
        drop(conn);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn referent_persists_across_turns() {
        let (state, session_id) = setup();

        // First turn: set referent
        let record = SessionState {
            session_id: session_id.clone(),
            intent: Some("set_field".to_string()),
            scope_predicate: Some(
                ScopePredicate::LibraryAndMissing {
                    field: "genre".into(),
                }
                .to_json(),
            ),
            protocol: "native".to_string(),
            referent_count: 102,
            referent_query: Some("missing genre".to_string()),
            referent_field: Some("genre".to_string()),
            referent_value: None,
            pending_batch_ids: vec![],
            mutation_required: false,
            consecutive_clarifications: 0,
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            updated_at: "2026-01-01T00:00:00.000Z".to_string(),
        };
        state.upsert_session(&record).unwrap();

        // Second turn: load latest session — referent should still be there
        let loaded = state.load_session(&session_id).unwrap();
        assert_eq!(loaded.referent_count, 102);
        assert_eq!(loaded.referent_field, Some("genre".to_string()));

        // Simulate setting the value in the referent
        let mut updated = record.clone();
        updated.referent_value = Some("Pop, Cantopop".to_string());
        updated.mutation_required = true;
        state.upsert_session(&updated).unwrap();

        let reloaded = state.load_session(&session_id).unwrap();
        assert_eq!(reloaded.referent_value, Some("Pop, Cantopop".to_string()));
    }

    #[test]
    fn tool_call_recording_and_result_updates() {
        let (state, session_id) = setup();

        state
            .record_tool_call(
                &session_id,
                "call_1",
                1,
                "metadata.patch",
                &serde_json::json!({"target_scope": "selected"}),
            )
            .unwrap();

        state
            .record_tool_result(
                &session_id,
                "call_1",
                true,
                &serde_json::json!({"ok": true, "summary": "Updated 2 tracks"}),
            )
            .unwrap();
    }

    #[test]
    fn batch_persists_and_loads_pending() {
        let (state, session_id) = setup();

        let batch_json = serde_json::json!({
            "id": "batch-1",
            "sessionId": session_id,
            "kind": "metadata-update",
            "title": "Patch metadata",
            "summary": "Update 2 fields across 2 tracks",
            "actions": [{"field": "genre", "newValue": "Pop"}]
        });

        state
            .save_batch("batch-1", &session_id, &batch_json, 2)
            .unwrap();

        // Update status
        state.update_batch_status("batch-1", "applied").unwrap();
        assert_eq!(state.batch_status("batch-1"), Some("applied".to_string()));

        // Record readback
        let readback = serde_json::json!([{"path": "/music/a.mp3", "genre": "Pop"}]);
        state.record_batch_readback("batch-1", &readback).unwrap();

        // Load pending — should be empty since it's applied
        let pending = state.load_pending_batches(&session_id);
        assert!(pending.is_empty());
    }

    #[test]
    fn pending_batch_survives_status_checks() {
        let (state, session_id) = setup();

        let batch_json = serde_json::json!({"id": "batch-pending", "kind": "metadata-update"});
        state
            .save_batch("batch-pending", &session_id, &batch_json, 3)
            .unwrap();

        let pending = state.load_pending_batches(&session_id);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, "batch-pending");

        // All-pending query
        let all = state.load_all_pending_batches();
        assert!(!all.is_empty());
    }

    #[test]
    fn finalized_batch_persists_verification_and_is_not_rehydrated() {
        let (state, session_id) = setup();
        let verification = serde_json::json!({
            "status": "verified",
            "scopeCount": 46,
            "expectedActionCount": 2,
            "verifiedActionCount": 2,
            "failures": []
        });
        for status in ["applied", "rejected", "failed"] {
            let batch_id = format!("batch-{status}");
            let batch_json = serde_json::json!({
                "id": batch_id,
                "sessionId": session_id,
                "kind": "metadata-update"
            });
            state
                .save_batch(&batch_id, &session_id, &batch_json, 2)
                .unwrap();
            state
                .finalize_batch(&batch_id, status, &verification)
                .unwrap();
        }

        assert!(state.load_pending_batches(&session_id).is_empty());
        assert!(state.load_all_pending_batches().is_empty());
        let guard = state.conn().unwrap();
        let conn = guard.as_ref().unwrap();
        let (status, readback): (String, String) = conn
            .query_row(
                "SELECT status, readback_json FROM assistant_batch WHERE batch_id = ?1",
                ["batch-applied"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "applied");
        assert_eq!(
            serde_json::from_str::<Value>(&readback).unwrap(),
            verification
        );
    }

    #[test]
    fn predicate_evaluation_missing_field() {
        let tracks = vec![
            serde_json::json!({"path": "/music/a.mp3", "genre": "Rock"}),
            serde_json::json!({"path": "/music/b.mp3", "genre": ""}),
            serde_json::json!({"path": "/music/c.mp3"}), // no genre key
        ];

        let predicate = ScopePredicate::LibraryAndMissing {
            field: "genre".into(),
        };
        let (paths, count) = evaluate_predicate(&predicate, &tracks, None, &[]);

        assert_eq!(count, 2);
        assert!(paths.contains(&"/music/b.mp3".to_string()));
        assert!(paths.contains(&"/music/c.mp3".to_string()));
        assert!(!paths.contains(&"/music/a.mp3".to_string()));
    }

    #[test]
    fn predicate_evaluation_selected() {
        let tracks = vec![
            serde_json::json!({"path": "/music/a.mp3"}),
            serde_json::json!({"path": "/music/b.mp3"}),
        ];

        let predicate = ScopePredicate::Selected;
        let (paths, count) =
            evaluate_predicate(&predicate, &tracks, None, &["/music/a.mp3".to_string()]);

        assert_eq!(count, 1);
        assert_eq!(paths, vec!["/music/a.mp3"]);
    }

    #[test]
    fn scope_predicate_json_serialization() {
        let pred = ScopePredicate::LibraryAndMissing {
            field: "genre".into(),
        };
        let json = pred.to_json();
        let back = ScopePredicate::from_json(&json).unwrap();
        assert_eq!(pred, back);

        let pred2 = ScopePredicate::Selected;
        let json2 = pred2.to_json();
        let back2 = ScopePredicate::from_json(&json2).unwrap();
        assert_eq!(pred2, back2);
    }

    #[test]
    fn latest_session_returns_most_recent() {
        let path = db_path();
        let state = AssistantTaskState::new(path);

        // Initialize without the temp_dir being created — create it manually
        std::fs::create_dir_all(state.db_path.parent().unwrap()).ok();
        state.initialize().unwrap();

        let session_1 = "session-old-1";
        let session_2 = "session-new-2";

        let old = SessionState {
            session_id: session_1.to_string(),
            intent: None,
            scope_predicate: None,
            protocol: "native".to_string(),
            referent_count: 0,
            referent_query: None,
            referent_field: None,
            referent_value: None,
            pending_batch_ids: vec![],
            mutation_required: false,
            consecutive_clarifications: 0,
            created_at: "2025-01-01T00:00:00.000Z".to_string(),
            updated_at: "2025-01-01T00:00:00.000Z".to_string(),
        };
        state.upsert_session(&old).unwrap();

        let new = SessionState {
            session_id: session_2.to_string(),
            intent: Some("set_field".to_string()),
            scope_predicate: None,
            protocol: "native".to_string(),
            referent_count: 102,
            referent_query: Some("missing genre".to_string()),
            referent_field: Some("genre".to_string()),
            referent_value: Some("Pop".to_string()),
            pending_batch_ids: vec![],
            mutation_required: true,
            consecutive_clarifications: 0,
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            updated_at: "2026-01-01T00:00:00.000Z".to_string(),
        };
        state.upsert_session(&new).unwrap();

        let latest = state.load_latest_session().unwrap();
        assert_eq!(latest.session_id, session_2);
        assert_eq!(latest.referent_count, 102);
    }

    #[test]
    fn load_latest_when_empty_returns_none() {
        let path = db_path();
        let state = AssistantTaskState::new(path);
        // Don't even initialize
        assert!(state.load_latest_session().is_none());
    }
}
