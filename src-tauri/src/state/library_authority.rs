//! Single-writer identity for an explicitly nominated library root.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub const MARKER_NAME: &str = ".soundrobe-library-authority";
pub const PROFILE_ID_NAME: &str = "library-authority-profile-id";
const MAX_MARKER_BYTES: u64 = 4096;

#[derive(Debug, thiserror::Error)]
pub enum AuthorityError {
    #[error("library has not been nominated for writing")]
    NotEnrolled,
    #[error("library already has a nominated writer")]
    AlreadyEnrolled,
    #[error("library belongs to a different writer profile")]
    OtherProfile,
    #[error("library writer is already active")]
    Busy,
    #[error("library authority is invalid: {0}")]
    Invalid(String),
    #[error("library authority is unsupported on this platform")]
    Unsupported,
    #[error("library authority I/O failed: {0}")]
    Io(#[from] io::Error),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorityMarker {
    version: u8,
    library_id: String,
    owner_profile_id: String,
}

pub struct AuthorityLease {
    root: PathBuf,
    root_directory: File,
    marker: AuthorityMarker,
    file: File,
}

/// Identify a file's nominated library without inheriting a marker across a mount.
#[cfg(unix)]
pub(crate) fn root_for_write_target(
    path: &Path,
    allow_missing: bool,
) -> Result<PathBuf, AuthorityError> {
    use std::os::unix::fs::MetadataExt;
    let (parent, device, target) = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.nlink() == 1 => {
            let canonical = fs::canonicalize(path)?;
            let parent = canonical
                .parent()
                .ok_or_else(|| AuthorityError::Invalid("file has no parent".into()))?;
            (parent.to_owned(), metadata.dev(), Some(metadata))
        }
        Err(error) if allow_missing && error.kind() == io::ErrorKind::NotFound => {
            if path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                return Err(AuthorityError::Invalid(
                    "missing target contains parent traversal".into(),
                ));
            }
            let mut candidate = path;
            let parent = loop {
                candidate = candidate
                    .parent()
                    .filter(|path| !path.as_os_str().is_empty())
                    .ok_or_else(|| AuthorityError::Invalid("file has no existing parent".into()))?;
                match fs::symlink_metadata(candidate) {
                    Ok(_) => break root_path(candidate)?,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            };
            let metadata = fs::metadata(&parent)?;
            (parent, metadata.dev(), None)
        }
        Err(error) => return Err(error.into()),
        Ok(_) => {
            return Err(AuthorityError::Invalid(
                "write target must be a regular file with one link".into(),
            ))
        }
    };
    let root = marker_ancestor(&parent, device)?;
    let marker = fs::symlink_metadata(root.join(MARKER_NAME))?;
    if target
        .as_ref()
        .is_some_and(|file| (file.dev(), file.ino()) == (marker.dev(), marker.ino()))
        || path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(MARKER_NAME))
    {
        return Err(AuthorityError::Invalid(
            "write target is a library authority marker".into(),
        ));
    }
    Ok(root)
}

#[cfg(unix)]
fn marker_ancestor(directory: &Path, device: u64) -> Result<PathBuf, AuthorityError> {
    use std::os::unix::fs::MetadataExt;
    for ancestor in directory.ancestors() {
        if fs::metadata(ancestor)?.dev() != device {
            break;
        }
        match fs::symlink_metadata(ancestor.join(MARKER_NAME)) {
            Ok(_) => return Ok(ancestor.to_owned()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(AuthorityError::NotEnrolled)
}

#[cfg(unix)]
pub(crate) fn root_for_directory(path: &Path) -> Result<PathBuf, AuthorityError> {
    use std::os::unix::fs::MetadataExt;
    let directory = root_path(path)?;
    marker_ancestor(&directory, fs::metadata(&directory)?.dev())
}

#[cfg(not(unix))]
pub(crate) fn root_for_directory(_path: &Path) -> Result<PathBuf, AuthorityError> {
    Err(AuthorityError::Unsupported)
}

#[cfg(not(unix))]
pub(crate) fn root_for_write_target(
    _path: &Path,
    _allow_missing: bool,
) -> Result<PathBuf, AuthorityError> {
    Err(AuthorityError::Unsupported)
}

fn root_path(root: &Path) -> Result<PathBuf, AuthorityError> {
    let root = fs::canonicalize(root)?;
    if !root.is_dir() {
        return Err(AuthorityError::Invalid("root is not a directory".into()));
    }
    Ok(root)
}

#[cfg(unix)]
fn publish_without_replacement(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let source = std::ffi::CString::new(source.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid source path"))?;
        let target = std::ffi::CString::new(target.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid target path"))?;
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (source, target);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic authority publication is unsupported",
        ))
    }
}

#[cfg(unix)]
fn read_marker(file: &File) -> Result<AuthorityMarker, AuthorityError> {
    use std::os::unix::fs::FileExt;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_MARKER_BYTES {
        return Err(AuthorityError::Invalid(
            "marker is not a bounded regular file".into(),
        ));
    }
    let mut bytes = vec![0; MAX_MARKER_BYTES as usize + 1];
    let mut length = 0;
    while length < bytes.len() {
        let count = file.read_at(&mut bytes[length..], length as u64)?;
        if count == 0 {
            break;
        }
        length += count;
    }
    if length > MAX_MARKER_BYTES as usize {
        return Err(AuthorityError::Invalid(
            "marker is not a bounded regular file".into(),
        ));
    }
    bytes.truncate(length);
    let marker: AuthorityMarker = serde_json::from_slice(&bytes)
        .map_err(|error| AuthorityError::Invalid(error.to_string()))?;
    if marker.version != 1
        || uuid::Uuid::parse_str(&marker.library_id).is_err()
        || uuid::Uuid::parse_str(&marker.owner_profile_id).is_err()
    {
        return Err(AuthorityError::Invalid(
            "unsupported marker identity".into(),
        ));
    }
    Ok(marker)
}

#[cfg(unix)]
fn read_profile_id(path: &Path) -> Result<uuid::Uuid, AuthorityError> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > 128 {
        return Err(AuthorityError::Invalid(
            "profile identity is not a bounded regular file".into(),
        ));
    }
    let mut bytes = Vec::new();
    file.take(129).read_to_end(&mut bytes)?;
    if bytes.len() > 128 {
        return Err(AuthorityError::Invalid(
            "profile identity is not a bounded regular file".into(),
        ));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| AuthorityError::Invalid("profile identity is corrupt".into()))?;
    uuid::Uuid::parse_str(text.trim())
        .map_err(|_| AuthorityError::Invalid("profile identity is corrupt".into()))
}

#[cfg(unix)]
pub fn load_or_create_profile_id(profile: &Path) -> Result<uuid::Uuid, AuthorityError> {
    use std::os::unix::fs::OpenOptionsExt;
    let profile = root_path(profile)?;
    let target = profile.join(PROFILE_ID_NAME);
    match read_profile_id(&target) {
        Ok(id) => return Ok(id),
        Err(AuthorityError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let id = uuid::Uuid::new_v4();
    let temp = profile.join(format!(".soundrobe-profile-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)?;
        file.write_all(id.to_string().as_bytes())?;
        file.sync_all()?;
        match publish_without_replacement(&temp, &target) {
            Ok(()) => {
                File::open(&profile)?.sync_all()?;
                Ok(id)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => read_profile_id(&target),
            Err(error) => Err(error.into()),
        }
    })();
    let _ = fs::remove_file(&temp);
    result
}

#[cfg(not(unix))]
pub fn load_or_create_profile_id(_profile: &Path) -> Result<uuid::Uuid, AuthorityError> {
    Err(AuthorityError::Unsupported)
}

#[cfg(unix)]
fn open_marker(path: &Path) -> Result<File, AuthorityError> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                AuthorityError::NotEnrolled
            } else {
                AuthorityError::Io(error)
            }
        })
}

#[cfg(unix)]
fn same_marker(path: &Path, file: &File) -> Result<(), AuthorityError> {
    use std::os::unix::fs::MetadataExt;
    let on_disk = fs::symlink_metadata(path)?;
    let held = file.metadata()?;
    if !on_disk.file_type().is_file()
        || held.nlink() != 1
        || on_disk.dev() != held.dev()
        || on_disk.ino() != held.ino()
    {
        return Err(AuthorityError::Invalid("marker identity changed".into()));
    }
    Ok(())
}

#[cfg(unix)]
fn same_marker_at(root: &File, file: &File) -> Result<(), AuthorityError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    let name = std::ffi::CString::new(MARKER_NAME).expect("marker name contains no NUL");
    let mut entry: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::fstatat(
            root.as_raw_fd(),
            name.as_ptr(),
            &mut entry,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(io::Error::last_os_error().into());
    }
    let held = file.metadata()?;
    if held.dev() != entry.st_dev as u64 || held.ino() != entry.st_ino as u64 {
        return Err(AuthorityError::Invalid(
            "marker is outside pinned root".into(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn nomination_guard(root: &Path) -> Result<File, AuthorityError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    let device = fs::metadata(root)?.dev();
    let mut anchor = root;
    while let Some(parent) = anchor.parent() {
        if fs::metadata(parent)?.dev() != device {
            break;
        }
        anchor = parent;
    }
    let lock = File::open(anchor)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
            continue;
        }
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(if error.kind() == io::ErrorKind::WouldBlock {
                AuthorityError::Busy
            } else {
                error.into()
            });
        }
    }
    for ancestor in root.ancestors() {
        if fs::metadata(ancestor)?.dev() != device {
            break;
        }
        match fs::symlink_metadata(ancestor.join(MARKER_NAME)) {
            Ok(_) => return Err(AuthorityError::AlreadyEnrolled),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                let path = entry.path();
                if fs::metadata(&path)?.dev() != device {
                    return Err(AuthorityError::Invalid(
                        "library contains a mount crossing".into(),
                    ));
                }
                match fs::symlink_metadata(path.join(MARKER_NAME)) {
                    Ok(_) => return Err(AuthorityError::AlreadyEnrolled),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                pending.push(path);
            }
        }
    }
    Ok(lock)
}

#[cfg(unix)]
pub fn nominate(root: &Path, owner: uuid::Uuid) -> Result<AuthorityMarker, AuthorityError> {
    use std::os::unix::fs::OpenOptionsExt;
    let root = root_path(root)?;
    let _guard = nomination_guard(&root)?;
    let target = root.join(MARKER_NAME);
    let marker = AuthorityMarker {
        version: 1,
        library_id: uuid::Uuid::new_v4().to_string(),
        owner_profile_id: owner.to_string(),
    };
    let temp = root.join(format!(".soundrobe-authority-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)?;
        file.write_all(
            &serde_json::to_vec(&marker)
                .map_err(|error| AuthorityError::Invalid(error.to_string()))?,
        )?;
        file.sync_all()?;
        publish_without_replacement(&temp, &target).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                AuthorityError::AlreadyEnrolled
            } else {
                AuthorityError::Io(error)
            }
        })?;
        File::open(&root)?.sync_all()?;
        Ok(marker)
    })();
    let _ = fs::remove_file(&temp);
    result
}

#[cfg(unix)]
pub fn acquire(root: &Path, owner: uuid::Uuid) -> Result<AuthorityLease, AuthorityError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let root = root_path(root)?;
    let root_directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(&root)?;
    let marker_path = root.join(MARKER_NAME);
    let file = open_marker(&marker_path)?;
    same_marker(&marker_path, &file)?;
    same_marker_at(&root_directory, &file)?;
    let marker = read_marker(&file)?;
    if marker.owner_profile_id != owner.to_string() {
        return Err(AuthorityError::OtherProfile);
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = io::Error::last_os_error();
        return Err(if error.kind() == io::ErrorKind::WouldBlock {
            AuthorityError::Busy
        } else {
            AuthorityError::Io(error)
        });
    }
    let lease = AuthorityLease {
        root,
        root_directory,
        marker,
        file,
    };
    lease.validate()?;
    Ok(lease)
}

#[cfg(not(unix))]
pub fn nominate(_root: &Path, _owner: uuid::Uuid) -> Result<AuthorityMarker, AuthorityError> {
    Err(AuthorityError::Unsupported)
}
#[cfg(not(unix))]
pub fn acquire(_root: &Path, _owner: uuid::Uuid) -> Result<AuthorityLease, AuthorityError> {
    Err(AuthorityError::Unsupported)
}

#[cfg(unix)]
impl Drop for AuthorityLease {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        // Closing alone can leave the flock alive in a child between fork and
        // exec. The last in-process owner releases it before closing the file.
        loop {
            if unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) } == 0 {
                break;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                tracing::warn!(%error, "Could not explicitly release library authority");
                break;
            }
        }
    }
}

impl AuthorityLease {
    pub(crate) fn identity(&self) -> (&str, &str) {
        (&self.marker.library_id, &self.marker.owner_profile_id)
    }

    #[cfg(unix)]
    pub fn validate(&self) -> Result<(), AuthorityError> {
        let path = self.root.join(MARKER_NAME);
        same_marker(&path, &self.file)?;
        same_marker_at(&self.root_directory, &self.file)?;
        let marker = read_marker(&self.file)?;
        if marker.library_id != self.marker.library_id
            || marker.owner_profile_id != self.marker.owner_profile_id
        {
            return Err(AuthorityError::Invalid("marker contents changed".into()));
        }
        same_marker(&path, &self.file)
    }
    #[cfg(unix)]
    pub(crate) fn validate_root(&self, directory: &File) -> Result<(), AuthorityError> {
        use std::os::unix::fs::MetadataExt;
        let expected = self.root_directory.metadata()?;
        let actual = directory.metadata()?;
        if (expected.dev(), expected.ino()) != (actual.dev(), actual.ino()) {
            return Err(AuthorityError::Invalid("library root changed".into()));
        }
        self.validate()
    }
    #[cfg(unix)]
    pub(crate) fn relative_file(&self, path: &Path) -> Result<PathBuf, AuthorityError> {
        let canonical = fs::canonicalize(path)?;
        let relative = canonical
            .strip_prefix(&self.root)
            .map_err(|_| AuthorityError::Invalid("write target is outside its library".into()))?
            .to_owned();
        self.binding_identity(path, &relative)?;
        Ok(relative)
    }

    #[cfg(unix)]
    pub(crate) fn binding_identity(
        &self,
        path: &Path,
        relative: &Path,
    ) -> Result<(String, String, PathBuf), AuthorityError> {
        use std::os::unix::fs::MetadataExt;
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(AuthorityError::Invalid(
                "authority target is not relative".into(),
            ));
        }
        self.validate()?;
        let mut current = self.root.clone();
        let root_device = self.root_directory.metadata()?.dev();
        for part in relative.components() {
            current.push(part.as_os_str());
            let metadata = fs::symlink_metadata(&current)?;
            if metadata.file_type().is_symlink() || metadata.dev() != root_device {
                return Err(AuthorityError::Invalid(
                    "authority target crosses a link or mount".into(),
                ));
            }
        }
        let metadata = fs::metadata(&current)?;
        let marker = self.file.metadata()?;
        let canonical = fs::canonicalize(&current)?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || (metadata.dev(), metadata.ino()) == (marker.dev(), marker.ino())
            || fs::canonicalize(path)? != canonical
        {
            return Err(AuthorityError::Invalid(
                "authority target does not match library".into(),
            ));
        }
        self.validate()?;
        Ok((
            self.marker.library_id.clone(),
            self.marker.owner_profile_id.clone(),
            canonical,
        ))
    }
    #[cfg(not(unix))]
    pub fn validate(&self) -> Result<(), AuthorityError> {
        Err(AuthorityError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn dropping_writer_lease_releases_lock_while_descriptor_is_inherited() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let lease = acquire(&root, owner).unwrap();
        // Models a forked child that has not closed its descriptor on exec yet.
        let inherited = lease.file.try_clone().unwrap();
        assert!(matches!(acquire(&root, owner), Err(AuthorityError::Busy)));
        drop(lease);
        let replacement = acquire(&root, owner);
        drop(inherited);
        drop(replacement.unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    use super::*;
    use std::fs;

    #[test]
    #[cfg(unix)]
    #[ignore = "subprocess entry for the authority lease crash test"]
    fn authority_lease_child() {
        let root = PathBuf::from(std::env::var_os("SOUNDROBE_AUTHORITY_TEST_ROOT").unwrap());
        let owner =
            uuid::Uuid::parse_str(&std::env::var("SOUNDROBE_AUTHORITY_TEST_PROFILE").unwrap())
                .unwrap();
        let _lease = acquire(&root, owner).unwrap();
        fs::write(root.join("ready"), b"ready").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(10));
    }

    #[test]
    #[cfg(unix)]
    fn another_process_cannot_acquire_and_only_owner_recovers_after_crash() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let owner = uuid::Uuid::new_v4();
        let other = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "state::library_authority::tests::authority_lease_child",
                "--ignored",
            ])
            .env("SOUNDROBE_AUTHORITY_TEST_ROOT", &root)
            .env("SOUNDROBE_AUTHORITY_TEST_PROFILE", owner.to_string())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !root.join("ready").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            root.join("ready").exists(),
            "child did not acquire the lease"
        );
        assert!(matches!(acquire(&root, owner), Err(AuthorityError::Busy)));
        assert!(matches!(
            acquire(&root, other),
            Err(AuthorityError::OtherProfile)
        ));
        child.kill().unwrap();
        child.wait().unwrap();
        acquire(&root, owner).unwrap().validate().unwrap();
        assert!(matches!(
            acquire(&root, other),
            Err(AuthorityError::OtherProfile)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn nominated_root_requires_the_same_profile_and_exclusive_live_lease() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let owner = uuid::Uuid::new_v4();
        let other = uuid::Uuid::new_v4();
        assert!(matches!(
            acquire(&root, owner),
            Err(AuthorityError::NotEnrolled)
        ));
        let marker = nominate(&root, owner).unwrap();
        assert_eq!(marker.owner_profile_id, owner.to_string());
        assert!(matches!(
            acquire(&root, other),
            Err(AuthorityError::OtherProfile)
        ));
        let lease = acquire(&root, owner).unwrap();
        assert!(matches!(acquire(&root, owner), Err(AuthorityError::Busy)));
        lease.validate().unwrap();
        drop(lease);
        acquire(&root, owner).unwrap().validate().unwrap();
        assert!(matches!(
            nominate(&root, other),
            Err(AuthorityError::AlreadyEnrolled)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(not(unix))]
    fn unsupported_platform_fails_closed() {
        let owner = uuid::Uuid::new_v4();
        assert!(matches!(
            acquire(Path::new("."), owner),
            Err(AuthorityError::Unsupported)
        ));
        assert!(matches!(
            nominate(Path::new("."), owner),
            Err(AuthorityError::Unsupported)
        ));
        assert!(matches!(
            load_or_create_profile_id(Path::new(".")),
            Err(AuthorityError::Unsupported)
        ));
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn publication_moves_the_synced_file_without_an_extra_link() {
        use std::os::unix::fs::MetadataExt;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let temp = root.join("pending");
        let marker = root.join("marker");
        fs::write(&temp, b"first").unwrap();
        publish_without_replacement(&temp, &marker).unwrap();
        assert!(!temp.exists());
        assert_eq!(fs::metadata(&marker).unwrap().nlink(), 1);
        fs::write(&temp, b"second").unwrap();
        assert_eq!(
            publish_without_replacement(&temp, &marker)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&marker).unwrap(), b"first");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn fifo_profile_identity_is_rejected_without_waiting_for_a_writer() {
        use std::os::unix::ffi::OsStrExt;
        let profile = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&profile).unwrap();
        let path = profile.join(PROFILE_ID_NAME);
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let (send, receive) = std::sync::mpsc::channel();
        let child_profile = profile.clone();
        let child = std::thread::spawn(move || {
            send.send(load_or_create_profile_id(&child_profile).is_err())
                .unwrap();
        });
        let result = receive.recv_timeout(std::time::Duration::from_secs(1));
        if result.is_err() {
            // Unblock the buggy reader so the test never leaves a stuck thread behind.
            drop(OpenOptions::new().write(true).open(&path).unwrap());
        }
        child.join().unwrap();
        fs::remove_dir_all(profile).unwrap();
        assert!(result.unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn profile_identity_is_stable_and_corruption_is_not_replaced() {
        let profile = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&profile).unwrap();
        let first = load_or_create_profile_id(&profile).unwrap();
        assert_eq!(load_or_create_profile_id(&profile).unwrap(), first);
        let path = profile.join(PROFILE_ID_NAME);
        fs::write(&path, b"corrupt").unwrap();
        assert!(load_or_create_profile_id(&profile).is_err());
        assert_eq!(fs::read(path).unwrap(), b"corrupt");
        fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn concurrent_lease_validation_does_not_share_a_read_cursor() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let lease = std::sync::Arc::new(acquire(&root, owner).unwrap());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let lease = lease.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..200 {
                        lease.validate().unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        drop(lease);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn overlapping_roots_cannot_both_be_nominated() {
        let parent = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let child = parent.join("album");
        fs::create_dir_all(&child).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = [parent.clone(), child.clone()]
            .into_iter()
            .map(|root| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    nominate(&root, uuid::Uuid::new_v4())
                })
            })
            .collect();
        let successes = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(Result::is_ok)
            .count();
        assert_eq!(
            successes, 1,
            "overlapping roots must not obtain independent authorities"
        );
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn overlap_rejected_in_either_order_but_siblings_remain_independent() {
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let parent_first = base.join("parent-first");
        let child_first = base.join("child-first");
        fs::create_dir_all(parent_first.join("album")).unwrap();
        fs::create_dir_all(child_first.join("album")).unwrap();
        nominate(&parent_first, uuid::Uuid::new_v4()).unwrap();
        assert!(matches!(
            nominate(&parent_first.join("album"), uuid::Uuid::new_v4()),
            Err(AuthorityError::AlreadyEnrolled)
        ));
        nominate(&child_first.join("album"), uuid::Uuid::new_v4()).unwrap();
        assert!(matches!(
            nominate(&child_first, uuid::Uuid::new_v4()),
            Err(AuthorityError::AlreadyEnrolled)
        ));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[ignore = "subprocess entry for concurrent overlapping nomination"]
    fn nomination_race_child() {
        let root = PathBuf::from(std::env::var_os("SOUNDROBE_NOMINATION_ROOT").unwrap());
        let signal = PathBuf::from(std::env::var_os("SOUNDROBE_NOMINATION_SIGNAL").unwrap());
        let ready = PathBuf::from(std::env::var_os("SOUNDROBE_NOMINATION_READY").unwrap());
        fs::write(ready, b"ready").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !signal.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(signal.exists());
        std::process::exit(match nominate(&root, uuid::Uuid::new_v4()) {
            Ok(_) => 0,
            Err(AuthorityError::AlreadyEnrolled) => 2,
            Err(_) => 3,
        });
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn separate_processes_cannot_nominate_overlapping_roots() {
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let child_root = root.join("album");
        fs::create_dir_all(&child_root).unwrap();
        let signal = base.join("start");
        let mut children = Vec::new();
        for (index, candidate) in [root, child_root].iter().enumerate() {
            let ready = base.join(format!("ready-{index}"));
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "state::library_authority::tests::nomination_race_child",
                    "--ignored",
                ])
                .env("SOUNDROBE_NOMINATION_ROOT", candidate)
                .env("SOUNDROBE_NOMINATION_SIGNAL", &signal)
                .env("SOUNDROBE_NOMINATION_READY", ready)
                .spawn()
                .unwrap();
            children.push(child);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !(base.join("ready-0").exists() && base.join("ready-1").exists())
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(base.join("ready-0").exists() && base.join("ready-1").exists());
        fs::write(signal, b"start").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut statuses = Vec::new();
        for index in 0..children.len() {
            loop {
                if let Some(status) = children[index].try_wait().unwrap() {
                    statuses.push(status.code().unwrap());
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    for child in &mut children {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                    panic!("nomination child did not finish before the deadline");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        assert!(
            statuses == [0, 2] || statuses == [2, 0],
            "statuses: {statuses:?}"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn marker_replacement_and_symlink_are_rejected() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let lease = acquire(&root, owner).unwrap();
        let marker = root.join(MARKER_NAME);
        fs::remove_file(&marker).unwrap();
        fs::write(&marker, b"replacement").unwrap();
        assert!(lease.validate().is_err());
        drop(lease);
        fs::remove_file(&marker).unwrap();
        symlink(root.join("missing"), &marker).unwrap();
        assert!(acquire(&root, owner).is_err());
        assert!(nominate(&root, owner).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
