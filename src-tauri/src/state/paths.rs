//! Soundrobe's application-data paths and legacy `auto-tagger` compatibility.

use std::path::{Path, PathBuf};
use std::{fs, io};

pub const APP_DIR_NAME: &str = ".soundrobe";
pub const LEGACY_APP_DIR_NAME: &str = ".auto-tagger";

pub fn app_dir(home: &Path) -> PathBuf {
    home.join(APP_DIR_NAME)
}

pub fn legacy_app_dir(home: &Path) -> PathBuf {
    home.join(LEGACY_APP_DIR_NAME)
}

/// Move the legacy application-data directory into the canonical location.
///
/// When `.soundrobe` does not exist, the directory rename preserves the whole
/// tree, including large datasets and SQLite sidecar files. When both
/// directories exist, entries missing from `.soundrobe` are moved there and
/// canonical entries win conflicts. The legacy directory is removed only
/// after it is empty, so a failed migration leaves a retryable source behind.
pub fn migrate_legacy_dir(home: &Path) -> io::Result<()> {
    let legacy = legacy_app_dir(home);
    let legacy_metadata = match fs::symlink_metadata(&legacy) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !legacy_metadata.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "legacy app-data path is not a directory: {}",
                legacy.display()
            ),
        ));
    }

    let canonical = app_dir(home);
    match fs::symlink_metadata(&canonical) {
        Ok(metadata) if !metadata.file_type().is_dir() => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "canonical app-data path is not a directory: {}",
                    canonical.display()
                ),
            ));
        }
        Ok(_) => merge_directory(&legacy, &canonical),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::rename(legacy, canonical),
        Err(error) => Err(error),
    }
}

fn merge_directory(source: &Path, destination: &Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let file_name = entry.file_name();
        let destination_path = destination.join(&file_name);
        let source_metadata = fs::symlink_metadata(&source_path)?;
        if is_cache_sidecar(&file_name) && destination.join("cache.db").exists() {
            remove_legacy_entry(&source_path)?;
            continue;
        }
        match fs::symlink_metadata(&destination_path) {
            Ok(destination_metadata)
                if source_metadata.file_type().is_dir()
                    && destination_metadata.file_type().is_dir() =>
            {
                merge_directory(&source_path, &destination_path)?;
            }
            Ok(_) => remove_legacy_entry(&source_path)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::rename(&source_path, &destination_path)?;
            }
            Err(error) => return Err(error),
        }
    }
    fs::remove_dir(source)
}

fn is_cache_sidecar(file_name: &std::ffi::OsStr) -> bool {
    matches!(
        file_name.to_str(),
        Some("cache.db-wal") | Some("cache.db-shm") | Some("cache.db.bak")
    )
}

fn remove_legacy_entry(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

pub fn canonical_path(home: &Path, file_name: &str) -> PathBuf {
    app_dir(home).join(file_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn canonical_path_uses_soundrobe_directory() {
        let home = Path::new("/tmp/soundrobe-path-home");
        assert_eq!(
            canonical_path(home, "config.yaml"),
            home.join(".soundrobe/config.yaml")
        );
    }

    #[test]
    fn migration_moves_legacy_tree_and_removes_legacy_directory() {
        let root =
            std::env::temp_dir().join(format!("soundrobe-paths-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(legacy_app_dir(&root).join("datasets")).unwrap();
        fs::write(
            legacy_app_dir(&root).join("config.yaml"),
            "remote_lookup_enabled: false\n",
        )
        .unwrap();
        fs::write(
            legacy_app_dir(&root).join("datasets/index.sqlite"),
            b"dataset",
        )
        .unwrap();

        migrate_legacy_dir(&root).unwrap();

        assert!(!legacy_app_dir(&root).exists());
        assert_eq!(
            fs::read_to_string(canonical_path(&root, "config.yaml")).unwrap(),
            "remote_lookup_enabled: false\n"
        );
        assert_eq!(
            fs::read(canonical_path(&root, "datasets/index.sqlite")).unwrap(),
            b"dataset"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_merges_missing_entries_and_keeps_soundrobe_conflicts() {
        let root =
            std::env::temp_dir().join(format!("soundrobe-paths-merge-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(app_dir(&root)).unwrap();
        fs::create_dir_all(legacy_app_dir(&root)).unwrap();
        fs::write(app_dir(&root).join("config.yaml"), b"new").unwrap();
        fs::write(legacy_app_dir(&root).join("config.yaml"), b"old").unwrap();
        fs::write(legacy_app_dir(&root).join("cache.db"), b"cache").unwrap();
        fs::create_dir_all(app_dir(&root).join("nested")).unwrap();
        fs::create_dir_all(legacy_app_dir(&root).join("nested")).unwrap();
        fs::write(app_dir(&root).join("nested/keep"), b"soundrobe").unwrap();
        fs::write(legacy_app_dir(&root).join("nested/move"), b"legacy").unwrap();

        migrate_legacy_dir(&root).unwrap();

        assert!(!legacy_app_dir(&root).exists());
        assert_eq!(
            fs::read(app_dir(&root).join("config.yaml")).unwrap(),
            b"new"
        );
        assert_eq!(fs::read(app_dir(&root).join("cache.db")).unwrap(), b"cache");
        assert_eq!(
            fs::read(app_dir(&root).join("nested/keep")).unwrap(),
            b"soundrobe"
        );
        assert_eq!(
            fs::read(app_dir(&root).join("nested/move")).unwrap(),
            b"legacy"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
