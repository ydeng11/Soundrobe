//! Read-only local dataset status.

use crate::state::config::ConfigState;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetStatus {
    pub available: bool,
    pub musicbrainz: bool,
    pub total_records: u64,
    pub last_updated: Option<String>,
}

impl DatasetStatus {
    fn unavailable() -> Self {
        Self {
            available: false,
            musicbrainz: false,
            total_records: 0,
            last_updated: None,
        }
    }
}

#[tauri::command]
pub fn dataset_status(config: State<'_, ConfigState>) -> DatasetStatus {
    let raw = config.raw();
    let path = raw
        .dataset_path
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".auto-tagger/dataset-index.sqlite")));
    path.map_or_else(DatasetStatus::unavailable, |path| dataset_status_at(&path))
}

pub fn dataset_status_at(path: &Path) -> DatasetStatus {
    if !path.exists() {
        return DatasetStatus::unavailable();
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let result = (|| -> rusqlite::Result<DatasetStatus> {
        let connection = Connection::open_with_flags(path, flags)?;
        let total: i64 =
            connection.query_row("SELECT COUNT(*) FROM dataset_lookup", [], |row| row.get(0))?;
        let musicbrainz: i64 = connection.query_row(
            "SELECT COUNT(*) FROM dataset_lookup WHERE service = 'musicbrainz'",
            [],
            |row| row.get(0),
        )?;
        Ok(DatasetStatus {
            available: true,
            musicbrainz: musicbrainz > 0,
            total_records: u64::try_from(total).unwrap_or(0),
            last_updated: None,
        })
    })();
    result.unwrap_or_else(|_| DatasetStatus::unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "soundrobe-dataset-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn missing_corrupt_and_wrong_schema_are_unavailable() {
        let root = root();
        assert_eq!(
            dataset_status_at(&root.join("missing.sqlite")),
            DatasetStatus::unavailable()
        );
        let corrupt = root.join("corrupt.sqlite");
        fs::write(&corrupt, b"not sqlite").unwrap();
        assert_eq!(dataset_status_at(&corrupt), DatasetStatus::unavailable());
        let wrong = root.join("wrong.sqlite");
        Connection::open(&wrong)
            .unwrap()
            .execute("CREATE TABLE other (id INTEGER)", [])
            .unwrap();
        assert_eq!(dataset_status_at(&wrong), DatasetStatus::unavailable());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_total_and_musicbrainz_presence() {
        let root = root();
        let path = root.join("dataset.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "CREATE TABLE dataset_lookup (service TEXT NOT NULL, album TEXT)",
                [],
            )
            .unwrap();
        for service in ["spotify", "musicbrainz", "musicbrainz"] {
            connection
                .execute(
                    "INSERT INTO dataset_lookup (service, album) VALUES (?1, 'Album')",
                    [service],
                )
                .unwrap();
        }
        drop(connection);
        assert_eq!(
            dataset_status_at(&path),
            DatasetStatus {
                available: true,
                musicbrainz: true,
                total_records: 3,
                last_updated: None,
            }
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn valid_dataset_without_musicbrainz_stays_available() {
        let root = root();
        let path = root.join("dataset.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute("CREATE TABLE dataset_lookup (service TEXT NOT NULL)", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO dataset_lookup (service) VALUES ('spotify')",
                [],
            )
            .unwrap();
        drop(connection);
        let status = dataset_status_at(&path);
        assert!(status.available);
        assert!(!status.musicbrainz);
        assert_eq!(status.total_records, 1);
        fs::remove_dir_all(root).unwrap();
    }
}
