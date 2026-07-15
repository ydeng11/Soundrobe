//! Assistant session lifecycle and SQLite-compatible conversation history.

use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS conversation_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_uuid TEXT NOT NULL,
  session_number TEXT NOT NULL DEFAULT '',
  timestamp TEXT NOT NULL,
  entry_type TEXT NOT NULL,
  content TEXT NOT NULL DEFAULT '',
  model TEXT,
  prompt_tokens INTEGER NOT NULL DEFAULT 0,
  completion_tokens INTEGER NOT NULL DEFAULT 0,
  total_tokens INTEGER NOT NULL DEFAULT 0,
  cost REAL NOT NULL DEFAULT 0.0,
  metadata TEXT
);
CREATE INDEX IF NOT EXISTS idx_cl_session_uuid ON conversation_log(session_uuid);
CREATE INDEX IF NOT EXISTS idx_cl_session_number ON conversation_log(session_number);
CREATE INDEX IF NOT EXISTS idx_cl_session_both ON conversation_log(session_uuid, session_number);
CREATE INDEX IF NOT EXISTS idx_cl_timestamp ON conversation_log(timestamp);
"#;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationEntry {
    pub id: i64,
    pub session_uuid: String,
    pub session_number: String,
    pub timestamp: String,
    pub entry_type: String,
    pub content: String,
    pub model: Option<String>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub cost: f64,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_number: String,
    pub session_uuid: String,
    pub entry_count: i64,
    pub first_message: Option<String>,
    pub last_activity: String,
    pub api_call_count: i64,
    pub total_cost: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentSession {
    pub session_id: String,
    pub session_number: String,
}

struct ConversationInner {
    connection: Connection,
    current: CurrentSession,
}

pub struct ConversationState {
    home: PathBuf,
    inner: Mutex<Option<ConversationInner>>,
}

impl ConversationState {
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            inner: Mutex::new(None),
        }
    }

    pub fn initialize(&self, configured_cache_path: Option<&str>) -> bool {
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };
        if guard.is_some() {
            return true;
        }
        let path = configured_cache_path
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join(".auto-tagger/cache.db"));
        if ensure_parent(&path).is_err() {
            return false;
        }
        let Ok(connection) = open_database(&path) else {
            return false;
        };
        *guard = Some(ConversationInner {
            connection,
            current: CurrentSession {
                session_id: Uuid::new_v4().to_string(),
                session_number: session_number(),
            },
        });
        true
    }

    pub fn current(&self) -> Option<CurrentSession> {
        self.inner
            .lock()
            .ok()?
            .as_ref()
            .map(|inner| inner.current.clone())
    }

    pub fn conversation(&self, identifier: &str) -> Vec<ConversationEntry> {
        let Ok(guard) = self.inner.lock() else {
            return Vec::new();
        };
        let Some(inner) = guard.as_ref() else {
            return Vec::new();
        };
        query_conversation(&inner.connection, identifier).unwrap_or_default()
    }

    pub fn sessions(&self, limit: i64) -> Vec<SessionSummary> {
        let Ok(guard) = self.inner.lock() else {
            return Vec::new();
        };
        let Some(inner) = guard.as_ref() else {
            return Vec::new();
        };
        query_sessions(&inner.connection, limit).unwrap_or_default()
    }

    pub fn session(&self, identifier: &str) -> Option<SessionSummary> {
        self.sessions(1000).into_iter().find(|session| {
            session.session_uuid == identifier || session.session_number == identifier
        })
    }
}

fn ensure_parent(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn open_database(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.execute_batch(SCHEMA)?;
    Ok(connection)
}

fn session_number() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let suffix = u128::from(std::process::id()) * 997 % 1_000_000;
    format!("{millis}-{suffix}")
}

fn query_conversation(
    connection: &Connection,
    identifier: &str,
) -> rusqlite::Result<Vec<ConversationEntry>> {
    let mut statement = connection.prepare(
        "SELECT id, session_uuid, session_number, timestamp, entry_type, content,
                model, prompt_tokens, completion_tokens, total_tokens, cost, metadata
         FROM conversation_log
         WHERE session_uuid = ?1 OR session_number = ?1
         ORDER BY id ASC",
    )?;
    let rows = statement.query_map([identifier], |row| {
        Ok(ConversationEntry {
            id: row.get(0)?,
            session_uuid: row.get(1)?,
            session_number: row.get(2)?,
            timestamp: row.get(3)?,
            entry_type: row.get(4)?,
            content: row.get(5)?,
            model: row.get(6)?,
            prompt_tokens: row.get(7)?,
            completion_tokens: row.get(8)?,
            total_tokens: row.get(9)?,
            cost: row.get(10)?,
            metadata: row.get(11)?,
        })
    })?;
    rows.collect()
}

fn query_sessions(connection: &Connection, limit: i64) -> rusqlite::Result<Vec<SessionSummary>> {
    let mut statement = connection.prepare(
        "SELECT session_number, session_uuid, COUNT(*),
           (SELECT content FROM conversation_log cl2
            WHERE cl2.session_uuid = cl.session_uuid AND cl2.entry_type = 'user_message'
            ORDER BY cl2.id ASC LIMIT 1),
           MAX(timestamp),
           SUM(CASE WHEN entry_type IN ('api_request','api_response') THEN 1 ELSE 0 END),
           COALESCE(SUM(cost), 0)
         FROM conversation_log cl
         GROUP BY session_uuid, session_number
         ORDER BY MAX(id) DESC
         LIMIT ?1",
    )?;
    let rows = statement.query_map(params![limit], |row| {
        let first_message: Option<String> = row.get(3)?;
        Ok(SessionSummary {
            session_number: row.get(0)?,
            session_uuid: row.get(1)?,
            entry_count: row.get(2)?,
            first_message: first_message.map(|message| message.chars().take(200).collect()),
            last_activity: row.get(4)?,
            api_call_count: row.get(5)?,
            total_cost: row.get(6)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "auto-tagger-conversation-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn relative_cache_path_requires_no_parent_directory() {
        assert!(ensure_parent(Path::new("cache.db")).is_ok());
    }

    #[test]
    fn remains_unavailable_until_initialized_then_exposes_current_session() {
        let root = root();
        let state = ConversationState::new(root.clone());
        assert_eq!(state.current(), None);
        assert!(state.sessions(50).is_empty());
        assert!(state.initialize(None));
        let current = state.current().unwrap();
        assert!(Uuid::parse_str(&current.session_id).is_ok());
        assert!(current.session_number.contains('-'));
        assert!(root.join(".auto-tagger/cache.db").exists());
        let repeated = state.current().unwrap();
        assert!(state.initialize(None));
        assert_eq!(repeated, state.current().unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn queries_existing_electron_schema_by_uuid_or_number() {
        let root = root();
        fs::create_dir_all(&root).unwrap();
        let db = root.join("cache.db");
        let connection = open_database(&db).unwrap();
        for (kind, content, cost) in [
            ("user_message", "x".repeat(250), 0.0),
            ("api_request", "request".to_string(), 0.0),
            ("api_response", "response".to_string(), 0.25),
        ] {
            connection
                .execute(
                    "INSERT INTO conversation_log
                 (session_uuid, session_number, timestamp, entry_type, content, cost)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        "uuid-1",
                        "123-4",
                        "2026-01-01T00:00:00.000Z",
                        kind,
                        content,
                        cost
                    ],
                )
                .unwrap();
        }
        drop(connection);
        let state = ConversationState::new(root.clone());
        assert!(state.initialize(Some(db.to_str().unwrap())));
        assert_eq!(state.conversation("uuid-1").len(), 3);
        assert_eq!(state.conversation("123-4").len(), 3);
        let summaries = state.sessions(50);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].entry_count, 3);
        assert_eq!(
            summaries[0].first_message.as_ref().unwrap().chars().count(),
            200
        );
        assert_eq!(summaries[0].api_call_count, 2);
        assert_eq!(summaries[0].total_cost, 0.25);
        assert_eq!(state.session("uuid-1"), Some(summaries[0].clone()));
        assert_eq!(state.session("missing"), None);
        fs::remove_dir_all(root).unwrap();
    }
}
