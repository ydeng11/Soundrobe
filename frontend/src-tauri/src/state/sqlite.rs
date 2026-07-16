//! Shared SQLite cache schemas and album-state ledger.

use rusqlite::{params, Connection, OpenFlags};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use time::OffsetDateTime;

const CACHE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS lookup_cache (
  query_hash TEXT PRIMARY KEY,
  query_json TEXT NOT NULL,
  response_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  source TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS album_state (
  path_hash TEXT PRIMARY KEY,
  status TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  folder_name_hash TEXT,
  llm_extraction TEXT,
  disc_count INTEGER DEFAULT 0,
  error TEXT,
  processed_at TEXT
);
CREATE TABLE IF NOT EXISTS artist_release_cache (
  provider TEXT NOT NULL,
  artist_id TEXT NOT NULL,
  page INTEGER NOT NULL DEFAULT 1,
  releases_json TEXT NOT NULL,
  fetched_at TEXT NOT NULL,
  PRIMARY KEY (provider, artist_id, page)
);
CREATE TABLE IF NOT EXISTS release_detail_cache (
  provider TEXT NOT NULL,
  release_id TEXT NOT NULL,
  detail_json TEXT NOT NULL,
  fetched_at TEXT NOT NULL,
  PRIMARY KEY (provider, release_id)
);
"#;

#[derive(Debug, Clone, PartialEq)]
pub struct AlbumState {
    pub status: String,
    pub path_hash: String,
    pub content_hash: String,
    pub folder_name_hash: Option<String>,
    pub llm_extraction: Option<Value>,
    pub disc_count: i64,
    pub error: Option<String>,
    pub processed_at: Option<String>,
}

pub struct CacheState {
    home: PathBuf,
    inner: Mutex<Option<Connection>>,
}

impl CacheState {
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            inner: Mutex::new(None),
        }
    }

    pub fn initialize(&self, configured_path: Option<&str>) -> bool {
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };
        if guard.is_some() {
            return true;
        }
        let path = configured_path
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join(".auto-tagger/cache.db"));
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            if fs::create_dir_all(parent).is_err() {
                return false;
            }
        }
        let Ok(connection) = open_cache(&path) else {
            return false;
        };
        *guard = Some(connection);
        true
    }

    pub fn album_state(&self, album_path: &Path) -> Option<AlbumState> {
        let guard = self.inner.lock().ok()?;
        let connection = guard.as_ref()?;
        let hash = path_hash(album_path);
        connection
            .query_row(
                "SELECT status, content_hash, folder_name_hash, llm_extraction,
                        disc_count, error, processed_at
                 FROM album_state WHERE path_hash = ?1",
                [&hash],
                |row| {
                    let extraction: Option<String> = row.get(3)?;
                    Ok(AlbumState {
                        status: row.get(0)?,
                        path_hash: hash.clone(),
                        content_hash: row.get(1)?,
                        folder_name_hash: row.get(2)?,
                        llm_extraction: extraction.and_then(|raw| serde_json::from_str(&raw).ok()),
                        disc_count: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                        error: row.get(5)?,
                        processed_at: row.get(6)?,
                    })
                },
            )
            .ok()
    }

    pub fn set_album_state(
        &self,
        album_path: &Path,
        status: &str,
        disc_count: i64,
        error: Option<&str>,
    ) -> Result<(), String> {
        if !matches!(status, "pending" | "llm_parsed" | "tagged_ok" | "error") {
            return Err(format!(
                "Invalid album status: {status}. Valid: error, llm_parsed, pending, tagged_ok"
            ));
        }
        let guard = self.inner.lock().map_err(|_| "cache state unavailable")?;
        let connection = guard.as_ref().ok_or("cache state not initialized")?;
        let parent_hash = album_path.parent().map(folder_name_hash);
        connection
            .execute(
                "INSERT OR REPLACE INTO album_state
                 (path_hash, status, content_hash, folder_name_hash,
                  disc_count, error, processed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    path_hash(album_path),
                    status,
                    content_hash(album_path),
                    parent_hash,
                    disc_count,
                    error,
                    now(),
                ],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn clear_album_state(&self, album_path: &Path) -> bool {
        let Ok(guard) = self.inner.lock() else {
            return false;
        };
        let Some(connection) = guard.as_ref() else {
            return false;
        };
        connection
            .execute(
                "DELETE FROM album_state WHERE path_hash = ?1",
                [path_hash(album_path)],
            )
            .is_ok()
    }

    pub fn set_llm_extraction(&self, folder_name: &str, extraction: &Value) -> bool {
        let Ok(guard) = self.inner.lock() else {
            return false;
        };
        let Some(connection) = guard.as_ref() else {
            return false;
        };
        let hash = folder_name_hash(Path::new(folder_name));
        connection
            .execute(
                "INSERT OR REPLACE INTO album_state
                 (path_hash, status, content_hash, folder_name_hash,
                  llm_extraction, disc_count, error, processed_at)
                 VALUES (?1, 'llm_parsed', '', ?2, ?3, 0, NULL, ?4)",
                params![format!("_llm_{hash}"), hash, extraction.to_string(), now()],
            )
            .is_ok()
    }

    pub fn llm_extraction(&self, folder_name: &str) -> Option<Value> {
        let guard = self.inner.lock().ok()?;
        let connection = guard.as_ref()?;
        let raw: String = connection
            .query_row(
                "SELECT llm_extraction FROM album_state
                 WHERE folder_name_hash = ?1 AND llm_extraction IS NOT NULL LIMIT 1",
                [folder_name_hash(Path::new(folder_name))],
                |row| row.get(0),
            )
            .ok()?;
        serde_json::from_str(&raw).ok()
    }
}

fn open_cache(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.execute_batch(CACHE_SCHEMA)?;
    Ok(connection)
}

pub fn path_hash(path: &Path) -> String {
    sha256(&path.to_string_lossy())
}

pub fn folder_name_hash(path: &Path) -> String {
    sha256(path.to_string_lossy().trim())
}

pub fn content_hash(album_path: &Path) -> String {
    let Ok(metadata) = fs::metadata(album_path) else {
        return String::new();
    };
    if !metadata.is_dir() {
        return String::new();
    }
    let Ok(entries) = fs::read_dir(album_path) else {
        return String::new();
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = fs::metadata(entry.path()).ok()?;
            metadata
                .is_file()
                .then(|| format!("{}:{}", entry.file_name().to_string_lossy(), metadata.len()))
        })
        .collect::<Vec<_>>();
    files.sort();
    sha256(&files.join("|"))
}

fn sha256(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn now() -> String {
    let value = OffsetDateTime::now_utc();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "auto-tagger-cache-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn creates_all_electron_compatible_tables_and_round_trips_album_state() {
        let root = root();
        let db = root.join("cache.db");
        let album = root.join("Artist/Album");
        fs::create_dir_all(&album).unwrap();
        fs::write(album.join("02.mp3"), b"two").unwrap();
        fs::write(album.join("01.mp3"), b"one").unwrap();
        let state = CacheState::new(root.clone());
        assert!(state.initialize(Some(db.to_str().unwrap())));
        assert!(state.set_album_state(&album, "pending", 2, None).is_ok());
        let row = state.album_state(&album).unwrap();
        assert_eq!(row.status, "pending");
        assert_eq!(row.disc_count, 2);
        assert_eq!(row.content_hash, content_hash(&album));
        assert_eq!(
            row.content_hash,
            "8a11d0589b2d6b85bbc3bfde60387989ce47b3cc57878f7aa1bd9b8df82a83cb"
        );
        assert!(row
            .processed_at
            .as_deref()
            .is_some_and(|timestamp| timestamp.len() == 24 && timestamp.ends_with('Z')));
        assert_eq!(row.folder_name_hash, album.parent().map(folder_name_hash));
        assert!(state.set_album_state(&album, "unknown", 0, None).is_err());

        let connection = Connection::open(&db).unwrap();
        for table in [
            "lookup_cache",
            "album_state",
            "artist_release_cache",
            "release_detail_cache",
        ] {
            let exists: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "{table}");
        }
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn llm_extraction_and_clear_match_legacy_ledger() {
        let root = root();
        let state = CacheState::new(root.clone());
        assert!(state.initialize(None));
        let extraction = serde_json::json!({"artist": "Artist", "album": null});
        assert!(state.set_llm_extraction(" Folder ", &extraction));
        assert_eq!(state.llm_extraction("Folder"), Some(extraction));
        let album = root.join("Album");
        fs::create_dir_all(&album).unwrap();
        state
            .set_album_state(&album, "error", 0, Some("failed"))
            .unwrap();
        assert!(state.clear_album_state(&album));
        assert_eq!(state.album_state(&album), None);
        fs::remove_dir_all(root).unwrap();
    }
}
