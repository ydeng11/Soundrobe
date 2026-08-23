use serde::Serialize;
use std::{
    ffi::OsString,
    fs, io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LibraryRoot {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone)]
struct LibraryRootEntry {
    root: LibraryRoot,
    canonical_path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct LibraryRoots {
    entries: Arc<Vec<LibraryRootEntry>>,
    base_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfinedPath {
    pub root_id: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfinedMove {
    pub source: ConfinedPath,
    pub destination: ConfinedPath,
}

#[derive(Debug, thiserror::Error)]
pub enum PathSecurityError {
    #[error("browser path must be absolute")]
    RelativePath,
    #[error("path traversal is not allowed")]
    Traversal,
    #[error("path cannot be resolved: {0}")]
    Unresolvable(String),
    #[error("path is outside the mounted library roots")]
    OutsideRoots,
    #[error("source and destination must belong to the same library root")]
    CrossRoot,
}

pub fn discover_library_roots(library_root_dir: &Path) -> io::Result<LibraryRoots> {
    let canonical_base = library_root_dir.canonicalize()?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(library_root_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let file_type = entry.file_type()?;
        if !file_type.is_dir() {
            continue;
        }
        let path = entry.path();
        let canonical_path = path.canonicalize()?;
        if !canonical_path.starts_with(&canonical_base) {
            continue;
        }
        let name = name.to_string_lossy().into_owned();
        entries.push(LibraryRootEntry {
            root: LibraryRoot {
                id: name.clone(),
                name,
                path: canonical_path.to_string_lossy().into_owned(),
            },
            canonical_path,
        });
    }
    entries.sort_by(|left, right| left.root.name.cmp(&right.root.name));
    Ok(LibraryRoots {
        entries: Arc::new(entries),
        base_path: canonical_base,
    })
}

impl LibraryRoots {
    pub fn roots(&self) -> Vec<LibraryRoot> {
        self.entries
            .iter()
            .map(|entry| entry.root.clone())
            .collect()
    }

    pub fn resolve_path(&self, supplied: &Path) -> Result<ConfinedPath, PathSecurityError> {
        validate_browser_path(supplied)?;
        let relative = supplied
            .strip_prefix(&self.base_path)
            .map_err(|_| PathSecurityError::OutsideRoots)?;
        let Some(Component::Normal(root_name)) = relative.components().next() else {
            return Err(PathSecurityError::OutsideRoots);
        };
        let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.root.id == root_name.to_string_lossy())
        else {
            return Err(PathSecurityError::OutsideRoots);
        };
        let canonical = canonicalize_with_missing_tail(supplied)
            .map_err(|error| PathSecurityError::Unresolvable(error.to_string()))?;
        if !canonical.starts_with(&entry.canonical_path) {
            return Err(PathSecurityError::OutsideRoots);
        }
        Ok(ConfinedPath {
            root_id: entry.root.id.clone(),
            path: canonical,
        })
    }

    pub fn validate_move(
        &self,
        source: &Path,
        destination: &Path,
    ) -> Result<ConfinedMove, PathSecurityError> {
        if fs::symlink_metadata(source).is_err() {
            return Err(PathSecurityError::Unresolvable(
                "source path does not exist".to_string(),
            ));
        }
        let source = self.resolve_path(source)?;
        let destination = self.resolve_path(destination)?;
        if source.root_id != destination.root_id {
            return Err(PathSecurityError::CrossRoot);
        }
        Ok(ConfinedMove {
            source,
            destination,
        })
    }
}

fn validate_browser_path(path: &Path) -> Result<(), PathSecurityError> {
    if !path.is_absolute() {
        return Err(PathSecurityError::RelativePath);
    }
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(PathSecurityError::Traversal);
    }
    Ok(())
}

fn canonicalize_with_missing_tail(path: &Path) -> io::Result<PathBuf> {
    let mut cursor = path.to_path_buf();
    let mut missing = Vec::<OsString>::new();
    loop {
        match fs::symlink_metadata(&cursor) {
            Ok(_) => {
                let mut canonical = cursor.canonicalize()?;
                for component in missing.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(file_name) = cursor.file_name() else {
                    return Err(error);
                };
                missing.push(file_name.to_os_string());
                let Some(parent) = cursor.parent() else {
                    return Err(error);
                };
                cursor = parent.to_path_buf();
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "soundrobe-{label}-{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    #[test]
    fn discovers_only_immediate_real_directories() {
        let base = temp_dir("libraries");
        fs::create_dir_all(base.join("alpha/nested")).unwrap();
        fs::create_dir_all(base.join("beta")).unwrap();
        fs::write(base.join("not-a-library.txt"), b"ignored").unwrap();
        let roots = discover_library_roots(&base).unwrap();

        assert_eq!(roots.roots().len(), 2);
        assert_eq!(roots.roots()[0].name, "alpha");
        assert_eq!(roots.roots()[1].name, "beta");
        assert_eq!(
            roots.roots()[0].path,
            base.join("alpha").canonicalize().unwrap().to_string_lossy()
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn includes_hidden_immediate_directories_as_mounts() {
        let base = temp_dir("hidden-library");
        fs::create_dir_all(base.join(".hidden/nested")).unwrap();

        let roots = discover_library_roots(&base).unwrap();

        assert_eq!(roots.roots().len(), 1);
        assert_eq!(roots.roots()[0].id, ".hidden");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn missing_library_root_is_an_error() {
        let base = temp_dir("missing-library-root");

        assert_eq!(
            discover_library_roots(&base).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinked_library_directories() {
        use std::os::unix::fs::symlink;

        let base = temp_dir("symlink-libraries");
        let outside = temp_dir("outside");
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, base.join("escape")).unwrap();

        let roots = discover_library_roots(&base).unwrap();

        assert!(roots.roots().is_empty());
        fs::remove_dir_all(base).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn confines_existing_and_not_yet_existing_paths_to_one_root() {
        let base = temp_dir("confine");
        fs::create_dir_all(base.join("alpha/album")).unwrap();
        let roots = discover_library_roots(&base).unwrap();
        let canonical_base = base.canonicalize().unwrap();
        let inside = canonical_base.join("alpha/album/track.flac");
        fs::write(&inside, b"audio").unwrap();

        let resolved = roots.resolve_path(&inside).unwrap();
        assert_eq!(resolved.root_id, "alpha");
        assert_eq!(resolved.path, inside.canonicalize().unwrap());

        let destination = canonical_base.join("alpha/new/track.flac");
        assert_eq!(roots.resolve_path(&destination).unwrap().root_id, "alpha");
        assert!(roots.resolve_path(Path::new("relative/path.flac")).is_err());
        assert!(roots
            .resolve_path(&canonical_base.join("alpha/../outside.flac"))
            .is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_cross_root_moves() {
        let base = temp_dir("cross-root");
        fs::create_dir_all(base.join("alpha")).unwrap();
        fs::create_dir_all(base.join("beta")).unwrap();
        let roots = discover_library_roots(&base).unwrap();
        let canonical_base = base.canonicalize().unwrap();
        let source = canonical_base.join("alpha/source.flac");
        fs::write(&source, b"audio").unwrap();

        let error = roots
            .validate_move(&source, &canonical_base.join("beta/destination.flac"))
            .unwrap_err();

        assert!(error.to_string().contains("same library root"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_missing_move_sources() {
        let base = temp_dir("missing-source");
        fs::create_dir_all(base.join("alpha")).unwrap();
        let roots = discover_library_roots(&base).unwrap();

        assert!(roots
            .validate_move(
                &base.join("alpha/missing.flac"),
                &base.join("alpha/destination.flac")
            )
            .is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escapes_for_existing_and_new_paths() {
        use std::os::unix::fs::symlink;

        let base = temp_dir("path-escape");
        let outside = temp_dir("path-outside");
        fs::create_dir_all(base.join("alpha")).unwrap();
        fs::create_dir_all(outside.join("nested")).unwrap();
        symlink(&outside, base.join("alpha/link")).unwrap();
        let roots = discover_library_roots(&base).unwrap();

        assert!(roots
            .resolve_path(&base.join("alpha/link/existing.flac"))
            .is_err());
        assert!(roots
            .resolve_path(&base.join("alpha/link/new.flac"))
            .is_err());
        fs::remove_dir_all(base).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlink_from_one_root_into_another_root() {
        use std::os::unix::fs::symlink;

        let base = temp_dir("cross-root-symlink");
        fs::create_dir_all(base.join("alpha")).unwrap();
        fs::create_dir_all(base.join("beta")).unwrap();
        fs::write(base.join("beta/source.flac"), b"audio").unwrap();
        symlink(base.join("beta"), base.join("alpha/link")).unwrap();
        let roots = discover_library_roots(&base).unwrap();

        let error = roots
            .validate_move(
                &base.join("alpha/link/source.flac"),
                &base.join("beta/destination.flac"),
            )
            .unwrap_err();

        assert!(error.to_string().contains("outside"));
        fs::remove_dir_all(base).unwrap();
    }
}
