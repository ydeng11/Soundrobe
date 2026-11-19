//! Handle-bound media access for library authority.

#[cfg(unix)]
use std::fs::{File, OpenOptions};
#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::path::{Component, Path};

/// Copy filesystem metadata before writing replacement bytes on macOS.
#[cfg(target_os = "macos")]
pub fn copy_filesystem_metadata(source: &File, destination: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    if unsafe {
        libc::fcopyfile(
            source.as_raw_fd(),
            destination.as_raw_fd(),
            std::ptr::null_mut(),
            libc::COPYFILE_METADATA,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
pub struct AuthorizedRoot {
    root: PinnedRoot,
    lease: std::sync::Arc<super::library_authority::AuthorityLease>,
}

#[cfg(unix)]
pub struct AuthorizedFile {
    target: PinnedTarget,
    lease: std::sync::Arc<super::library_authority::AuthorityLease>,
}

#[cfg(unix)]
pub struct PrivateStage {
    workspace: File,
    directory: File,
    directory_name: std::ffi::CString,
    file_name: std::ffi::CString,
    owns_file: bool,
    file_identity: Option<(u64, u64)>,
    cleaned: bool,
    retained: bool,
    writer_verified: bool,
    path: std::path::PathBuf,
    source: AuthorizedFile,
    source_digest: Option<[u8; 32]>,
}

#[cfg(all(test, target_os = "macos"))]
struct PublishTemporary {
    parent: File,
    name: std::ffi::CString,
    identity: (u64, u64),
    published: bool,
}

#[cfg(all(test, target_os = "macos"))]
impl Drop for PublishTemporary {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        if self.published {
            return;
        }
        let mut entry: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                self.parent.as_raw_fd(),
                self.name.as_ptr(),
                &mut entry,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
            && self.identity == (entry.st_dev as u64, entry.st_ino as u64)
        {
            let _ = unsafe { libc::unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), 0) };
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
fn verify_published_target(
    parent: &File,
    name: &std::ffi::CStr,
    expected_identity: (u64, u64),
    expected_digest: &str,
) -> io::Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::MetadataExt;
    let raw = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(raw) };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || (metadata.dev(), metadata.ino()) != expected_identity
    {
        return Err(io::Error::other(
            "published file identity differs from validated temporary",
        ));
    }
    let digest: String = digest_file(&file)?
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if digest != expected_digest {
        return Err(io::Error::other(
            "published file differs from validated stage",
        ));
    }
    let mut entry: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            &mut entry,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let after = file.metadata()?;
    if after.nlink() != 1
        || (after.dev(), after.ino()) != expected_identity
        || expected_identity != (entry.st_dev as u64, entry.st_ino as u64)
    {
        return Err(io::Error::other(
            "published file changed during verification",
        ));
    }
    Ok(())
}

#[cfg(unix)]
impl PrivateStage {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn lease(&self) -> &super::library_authority::AuthorityLease {
        &self.source.lease
    }

    pub(crate) fn source_digest(&self) -> Option<String> {
        self.source_digest
            .map(|digest| digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn publish_with_intent(
        &mut self,
        worker: &crate::state::worker::WorkerStore,
        original: &Path,
        run_id: &str,
        relative: &Path,
        queue: &crate::state::write_queue::QueueLease,
    ) -> io::Result<()> {
        self.publish_with_intent_and_hooks(
            worker,
            original,
            run_id,
            relative,
            queue,
            || {},
            |_| {},
            || {},
            || {},
        )
    }

    #[cfg(all(test, target_os = "macos"))]
    #[allow(clippy::too_many_arguments)]
    fn publish_with_intent_and_hooks(
        &mut self,
        worker: &crate::state::worker::WorkerStore,
        original: &Path,
        run_id: &str,
        relative: &Path,
        queue: &crate::state::write_queue::QueueLease,
        after_begin: impl FnOnce(),
        before_rename: impl FnOnce(&std::ffi::CStr),
        after_rename: impl FnOnce(),
        before_final_readback: impl FnOnce(),
    ) -> io::Result<()> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::MetadataExt;
        if !queue.is_exclusive() || !queue.protects_worker(worker)? {
            return Err(io::Error::other(
                "publication requires the matching exclusive write queue",
            ));
        }
        let expected = worker
            .validate_authority_staged_write(original, run_id, relative, self)
            .map_err(io::Error::other)?;
        worker
            .begin_write_intent(original, run_id)
            .map_err(io::Error::other)?;
        after_begin();
        let prepared = (|| -> io::Result<PublishTemporary> {
            let target = &self.source.target;
            use sha2::{Digest, Sha256};
            let mut key = Sha256::new();
            key.update((run_id.len() as u64).to_be_bytes());
            key.update(run_id.as_bytes());
            key.update(target.name.to_bytes());
            let temp_name =
                std::ffi::CString::new(format!(".soundrobe-publish-{:x}", key.finalize()))
                    .expect("generated temporary name");
            let raw = unsafe {
                libc::openat(
                    target.parent.as_raw_fd(),
                    temp_name.as_ptr(),
                    libc::O_RDWR
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if raw < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut output = unsafe { File::from_raw_fd(raw) };
            let output_metadata = output.metadata()?;
            let temporary = PublishTemporary {
                parent: target.parent.try_clone()?,
                name: temp_name,
                identity: (output_metadata.dev(), output_metadata.ino()),
                published: false,
            };
            copy_filesystem_metadata(&target.file, &output)?;
            let staged_raw = unsafe {
                libc::openat(
                    self.directory.as_raw_fd(),
                    self.file_name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            };
            if staged_raw < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut staged = unsafe { File::from_raw_fd(staged_raw) };
            let staged_metadata = staged.metadata()?;
            if !staged_metadata.is_file()
                || staged_metadata.nlink() != 1
                || self.file_identity != Some((staged_metadata.dev(), staged_metadata.ino()))
            {
                return Err(io::Error::other("staged output changed before publication"));
            }
            io::copy(&mut staged, &mut output)?;
            output.sync_all()?;
            let actual: String = digest_file(&output)?
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            if actual != expected {
                return Err(io::Error::other(
                    "publication copy differs from validated stage",
                ));
            }
            if worker
                .validate_authority_staged_write(original, run_id, relative, self)
                .map_err(io::Error::other)?
                != expected
            {
                return Err(io::Error::other("staged output changed during publication"));
            }
            Ok(temporary)
        })();
        let mut temporary = match prepared {
            Ok(temporary) => temporary,
            Err(error) => {
                self.retained = true;
                return Err(io::Error::other(format!(
                    "publication did not replace the source; stage and backup retained: {error}"
                )));
            }
        };
        self.retained = true;
        let target = &self.source.target;
        let metadata = target.file.metadata()?;
        let source_identity = (metadata.dev(), metadata.ino());
        let source_digest = self
            .source_digest()
            .ok_or_else(|| io::Error::other("publication requires the original source digest"))?;
        target.parent.sync_all()?;
        before_rename(&temporary.name);
        // Preserve the actual destination even if a competing namespace change
        // lands after validation. There is no conditional inode check in swap.
        const RENAME_NOFOLLOW_ANY: libc::c_uint = 0x10; // macOS sys/stdio.h
        if unsafe {
            libc::renameatx_np(
                temporary.parent.as_raw_fd(),
                temporary.name.as_ptr(),
                target.parent.as_raw_fd(),
                target.name.as_ptr(),
                libc::RENAME_SWAP | RENAME_NOFOLLOW_ANY,
            )
        } != 0
        {
            return Err(io::Error::other(format!(
                "publication failed; stage retained: {}",
                io::Error::last_os_error()
            )));
        }
        // The temporary name now points at the displaced entry. Keep it for
        // recovery, including its metadata, even if verification fails.
        temporary.published = true;
        after_rename();
        let finish = || -> io::Result<()> {
            target.parent.sync_all()?;
            let verify_displaced = || {
                verify_published_target(
                    &temporary.parent,
                    &temporary.name,
                    source_identity,
                    &source_digest,
                )
            };
            verify_displaced()?;
            verify_published_target(&target.parent, &target.name, temporary.identity, &expected)?;
            worker
                .verify_write_authority(original, run_id, self.lease(), relative)
                .map_err(io::Error::other)?;
            worker
                .finalize_expected_write_intent(original, run_id, &expected, || {
                    before_final_readback();
                    verify_displaced()?;
                    verify_published_target(
                        &target.parent,
                        &target.name,
                        temporary.identity,
                        &expected,
                    )
                })
                .map_err(io::Error::other)?;
            Ok(())
        };
        if let Err(error) = finish() {
            return Err(io::Error::other(format!(
                "write may have been applied; stage, backup and displaced entry retained: {error}"
            )));
        }
        self.retained = false;
        Ok(())
    }

    pub(crate) fn candidate(&self, original: &Path) -> io::Result<(std::path::PathBuf, String)> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::MetadataExt;
        if !self.writer_verified || self.retained {
            return Err(io::Error::other(
                "staged writer has not completed successfully",
            ));
        }
        self.revalidate_source()?;
        let source = self.source.target.file.metadata()?;
        let supplied = std::fs::metadata(original)?;
        if (source.dev(), source.ino()) != (supplied.dev(), supplied.ino()) {
            return Err(io::Error::other(
                "staged source does not match intent target",
            ));
        }
        self.validate_path()?;
        let raw = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                self.file_name.as_ptr(),
                libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(raw) };
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || self.file_identity != Some((metadata.dev(), metadata.ino()))
        {
            return Err(io::Error::other("staged candidate identity changed"));
        }
        let digest = digest_file(&file)?;
        let mut after: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                self.directory.as_raw_fd(),
                self.file_name.as_ptr(),
                &mut after,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let held = file.metadata()?;
        if held.nlink() != 1
            || (held.dev(), held.ino()) != (after.st_dev as u64, after.st_ino as u64)
        {
            return Err(io::Error::other(
                "staged candidate changed during verification",
            ));
        }
        self.validate_path()?;
        self.revalidate_source()?;
        Ok((
            self.path.clone(),
            digest.iter().map(|byte| format!("{byte:02x}")).collect(),
        ))
    }

    pub fn revalidate_source(&self) -> io::Result<()> {
        self.revalidate_source_with(|| {})
    }

    fn revalidate_source_with(&self, after_digest: impl FnOnce()) -> io::Result<()> {
        self.source.lease.validate().map_err(io::Error::other)?;
        self.validate_source_identity()?;
        if self.source_digest != Some(digest_file(&self.source.target.file)?) {
            return Err(io::Error::other("staged source contents changed"));
        }
        after_digest();
        self.validate_source_identity()?;
        self.source.lease.validate().map_err(io::Error::other)
    }

    fn validate_source_identity(&self) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;
        let target = &self.source.target;
        let mut entry: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                target.parent.as_raw_fd(),
                target.name.as_ptr(),
                &mut entry,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let held = target.file.metadata()?;
        if !held.is_file()
            || held.nlink() != 1
            || (held.dev(), held.ino()) != (entry.st_dev as u64, entry.st_ino as u64)
        {
            return Err(io::Error::other("staged source identity changed"));
        }
        Ok(())
    }

    pub fn run_writer<T>(&mut self, writer: impl FnOnce(&Path) -> io::Result<T>) -> io::Result<T> {
        use std::os::fd::AsRawFd;
        self.source.lease.validate().map_err(io::Error::other)?;
        self.validate_path()?;
        let mut before: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                self.directory.as_raw_fd(),
                self.file_name.as_ptr(),
                &mut before,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if self.file_identity != Some((before.st_dev as u64, before.st_ino as u64)) {
            return Err(io::Error::other("private stage file changed before writer"));
        }
        self.retained = true;
        self.writer_verified = false;
        let result = writer(&self.path);
        let lease = self.source.lease.validate().map_err(io::Error::other);
        let value = match (result, lease) {
            (Err(error), _) | (Ok(_), Err(error)) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("{error}; changed staging files are retained for recovery"),
                ));
            }
            (Ok(value), Ok(())) => value,
        };
        if let Err(error) = self.reconcile_output() {
            return Err(io::Error::new(
                error.kind(),
                format!("{error}; changed staging files are retained for recovery"),
            ));
        }
        self.retained = false;
        self.writer_verified = true;
        Ok(value)
    }

    fn reconcile_output(&mut self) -> io::Result<()> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::MetadataExt;
        self.validate_path()?;
        let raw = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                self.file_name.as_ptr(),
                libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(raw) };
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(io::Error::other(
                "private stage output is not an owned regular file",
            ));
        }
        self.file_identity = Some((metadata.dev(), metadata.ino()));
        Ok(())
    }

    fn validate_path(&self) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt;
        let directory = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("private stage has no parent"))?;
        let workspace = directory
            .parent()
            .ok_or_else(|| io::Error::other("private stage has no workspace"))?;
        let path_workspace = std::fs::metadata(workspace)?;
        let held_workspace = self.workspace.metadata()?;
        let path_directory = std::fs::symlink_metadata(directory)?;
        let held_directory = self.directory.metadata()?;
        if (path_workspace.dev(), path_workspace.ino())
            != (held_workspace.dev(), held_workspace.ino())
            || !path_directory.is_dir()
            || (path_directory.dev(), path_directory.ino())
                != (held_directory.dev(), held_directory.ino())
        {
            return Err(io::Error::other("private stage path changed"));
        }
        Ok(())
    }

    pub fn cleanup(mut self) -> io::Result<()> {
        if self.retained {
            return Err(io::Error::other("private stage is retained for recovery"));
        }
        let result = self.cleanup_inner();
        self.cleaned = true;
        result
    }

    fn cleanup_inner(&mut self) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;
        if self.owns_file {
            let mut entry: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::fstatat(
                    self.directory.as_raw_fd(),
                    self.file_name.as_ptr(),
                    &mut entry,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            if self.file_identity != Some((entry.st_dev as u64, entry.st_ino as u64)) {
                return Err(io::Error::other("private stage file changed"));
            }
            if unsafe { libc::unlinkat(self.directory.as_raw_fd(), self.file_name.as_ptr(), 0) }
                != 0
            {
                return Err(io::Error::last_os_error());
            }
            self.owns_file = false;
        }
        let mut entry: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                self.workspace.as_raw_fd(),
                self.directory_name.as_ptr(),
                &mut entry,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let held = self.directory.metadata()?;
        if (held.dev(), held.ino()) != (entry.st_dev as u64, entry.st_ino as u64) {
            return Err(io::Error::other("private stage directory changed"));
        }
        if unsafe {
            libc::unlinkat(
                self.workspace.as_raw_fd(),
                self.directory_name.as_ptr(),
                libc::AT_REMOVEDIR,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for PrivateStage {
    fn drop(&mut self) {
        if !self.cleaned && !self.retained {
            let _ = self.cleanup_inner();
        }
    }
}

#[cfg(unix)]
impl AuthorizedRoot {
    pub fn open(path: &Path, owner: uuid::Uuid) -> io::Result<Self> {
        let lease = std::sync::Arc::new(
            super::library_authority::acquire(path, owner).map_err(io::Error::other)?,
        );
        let root = PinnedRoot::open(path)?;
        lease
            .validate_root(&root.directory)
            .map_err(io::Error::other)?;
        Ok(Self { root, lease })
    }

    pub fn open_file(&self, relative: &Path) -> io::Result<AuthorizedFile> {
        self.lease.validate().map_err(io::Error::other)?;
        let target = self.root.open_target(relative)?;
        self.lease.validate().map_err(io::Error::other)?;
        Ok(AuthorizedFile {
            target,
            lease: self.lease.clone(),
        })
    }

    pub fn stage_file(&self, relative: &Path, workspace: &Path) -> io::Result<PrivateStage> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let extension = relative
            .extension()
            .and_then(|s| s.to_str())
            .filter(|s| {
                !s.is_empty() && s.len() <= 16 && s.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "unsupported media extension")
            })?;
        let source = self.open_file(relative)?;
        let workspace = std::fs::canonicalize(workspace)?;
        let workspace_fd = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&workspace)?;
        let metadata = workspace_fd.metadata()?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "workspace is not private to its owner",
            ));
        }
        let directory_string = format!(".soundrobe-stage-{}", uuid::Uuid::new_v4());
        let directory_name =
            std::ffi::CString::new(directory_string.as_bytes()).expect("generated directory name");
        if unsafe { libc::mkdirat(workspace_fd.as_raw_fd(), directory_name.as_ptr(), 0o700) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let directory_raw = unsafe {
            libc::openat(
                workspace_fd.as_raw_fd(),
                directory_name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if directory_raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let directory = unsafe { File::from_raw_fd(directory_raw) };
        let file_string = format!("source.{extension}");
        let file_name =
            std::ffi::CString::new(file_string.as_bytes()).expect("validated file name");
        let path = workspace.join(&directory_string).join(file_string);
        let mut stage = PrivateStage {
            workspace: workspace_fd,
            directory,
            directory_name,
            file_name,
            owns_file: false,
            file_identity: None,
            cleaned: false,
            retained: false,
            writer_verified: false,
            path,
            source,
            source_digest: None,
        };
        let output_raw = unsafe {
            libc::openat(
                stage.directory.as_raw_fd(),
                stage.file_name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if output_raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut output = unsafe { File::from_raw_fd(output_raw) };
        let output_metadata = output.metadata()?;
        stage.file_identity = Some((output_metadata.dev(), output_metadata.ino()));
        stage.owns_file = true;
        io::copy(&mut stage.source, &mut output)?;
        output.sync_all()?;
        stage.source_digest = Some(digest_file(&output)?);
        stage.revalidate_source()?;
        Ok(stage)
    }
}

#[cfg(unix)]
impl io::Read for AuthorizedFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.lease.validate().map_err(io::Error::other)?;
        io::Read::read(&mut self.target.file, buf)
    }
}

#[cfg(unix)]
fn digest_file(file: &File) -> io::Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::FileExt;
    let mut hash = Sha256::new();
    let mut offset = 0u64;
    let mut bytes = [0u8; 65536];
    loop {
        let count = file.read_at(&mut bytes, offset)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
        offset += count as u64;
    }
    Ok(hash.finalize().into())
}

#[cfg(unix)]
struct PinnedTarget {
    file: File,
    parent: File,
    name: std::ffi::CString,
}

/// Pins a directory so resolving descendants never follows replaced ancestors.
#[cfg(unix)]
pub struct PinnedRoot {
    directory: File,
    device: u64,
}

#[cfg(unix)]
fn marker_identity(parent: &File) -> io::Result<Option<(u64, u64)>> {
    use std::os::fd::AsRawFd;
    let mut marker: libc::stat = unsafe { std::mem::zeroed() };
    let name = std::ffi::CString::new(super::library_authority::MARKER_NAME)
        .expect("authority marker name contains no NUL");
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            &mut marker,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } == 0
    {
        return Ok(Some((marker.st_dev as u64, marker.st_ino as u64)));
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::NotFound {
        Ok(None)
    } else {
        Err(error)
    }
}

#[cfg(unix)]
impl PinnedRoot {
    pub fn open(path: &Path) -> io::Result<Self> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)?;
        let device = directory.metadata()?.dev();
        Ok(Self { directory, device })
    }

    pub fn open_file(&self, relative: &Path) -> io::Result<File> {
        Ok(self.open_target(relative)?.file)
    }

    fn open_target(&self, relative: &Path) -> io::Result<PinnedTarget> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::MetadataExt;
        let parts = relative.components().collect::<Vec<_>>();
        if parts.is_empty()
            || parts
                .iter()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "target must be a relative path",
            ));
        }
        let mut parent = self.directory.try_clone()?;
        for (index, part) in parts.iter().enumerate() {
            let Component::Normal(name) = part else {
                unreachable!()
            };
            if name
                .as_bytes()
                .eq_ignore_ascii_case(super::library_authority::MARKER_NAME.as_bytes())
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "reserved authority marker",
                ));
            }
            let name = std::ffi::CString::new(name.as_bytes())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid target name"))?;
            let reserved_identity = marker_identity(&parent)?;
            let last = index + 1 == parts.len();
            let flags = libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC
                | if last { 0 } else { libc::O_DIRECTORY };
            let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let opened = unsafe { File::from_raw_fd(fd) };
            let metadata = opened.metadata()?;
            if reserved_identity == Some((metadata.dev(), metadata.ino())) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "reserved authority marker",
                ));
            }
            if metadata.dev() != self.device
                || (last && (!metadata.is_file() || metadata.nlink() != 1))
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "unsupported target",
                ));
            }
            if last {
                return Ok(PinnedTarget {
                    file: opened,
                    parent,
                    name,
                });
            }
            parent = opened;
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "target has no file",
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Read;
    use std::os::unix::fs::symlink;

    #[test]
    fn pinned_source_does_not_follow_a_replaced_parent() {
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("root");
        let outside = base.join("outside");
        fs::create_dir_all(root.join("album")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(root.join("album/track.flac"), b"inside").unwrap();
        fs::write(outside.join("track.flac"), b"outside").unwrap();
        let pinned = PinnedRoot::open(&root).unwrap();
        let mut source = pinned.open_file(Path::new("album/track.flac")).unwrap();
        fs::rename(root.join("album"), root.join("moved")).unwrap();
        symlink(&outside, root.join("album")).unwrap();
        let mut bytes = Vec::new();
        source.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"inside");
        assert!(pinned.open_file(Path::new("album/track.flac")).is_err());
        assert_eq!(fs::read(outside.join("track.flac")).unwrap(), b"outside");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn special_files_links_traversal_and_authority_markers_are_not_media() {
        use std::os::unix::ffi::OsStrExt;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let fifo = root.join("pipe.flac");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        fs::write(root.join("track.flac"), b"audio").unwrap();
        fs::hard_link(root.join("track.flac"), root.join("other.flac")).unwrap();
        symlink(root.join("track.flac"), root.join("alias.flac")).unwrap();
        fs::write(
            root.join(super::super::library_authority::MARKER_NAME),
            b"marker",
        )
        .unwrap();
        let pinned = PinnedRoot::open(&root).unwrap();
        for target in [
            "pipe.flac",
            "track.flac",
            "alias.flac",
            "../outside",
            super::super::library_authority::MARKER_NAME,
        ] {
            assert!(
                pinned.open_file(Path::new(target)).is_err(),
                "accepted {target}"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn returned_handle_is_close_on_exec_and_marker_case_aliases_are_reserved() {
        use std::os::fd::AsRawFd;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("track.flac"), b"audio").unwrap();
        fs::write(root.join(".SOUNDROBE-LIBRARY-AUTHORITY"), b"marker").unwrap();
        let pinned = PinnedRoot::open(&root).unwrap();
        let source = pinned.open_file(Path::new("track.flac")).unwrap();
        let flags = unsafe { libc::fcntl(source.as_raw_fd(), libc::F_GETFD) };
        assert!(flags >= 0);
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
        assert!(pinned
            .open_file(Path::new(".SOUNDROBE-LIBRARY-AUTHORITY"))
            .is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn filesystem_equivalent_unicode_marker_is_not_readable_as_media() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join(super::super::library_authority::MARKER_NAME),
            b"marker",
        )
        .unwrap();
        let alias = Path::new(".ſoundrobe-library-authority");
        let pinned = PinnedRoot::open(&root).unwrap();
        assert!(pinned.open_file(alias).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn captured_marker_identity_survives_a_concurrent_rename() {
        use std::os::unix::fs::MetadataExt;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let marker_path = root.join(super::super::library_authority::MARKER_NAME);
        fs::write(&marker_path, b"marker").unwrap();
        let directory = File::open(&root).unwrap();
        let identity = marker_identity(&directory).unwrap().unwrap();
        let candidate = File::open(&marker_path).unwrap();
        fs::rename(&marker_path, root.join("moved-marker")).unwrap();
        let metadata = candidate.metadata().unwrap();
        assert_eq!(identity, (metadata.dev(), metadata.ino()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn authorized_read_holds_the_writer_lease_and_refuses_a_changed_marker() {
        use super::super::library_authority::{nominate, MARKER_NAME};
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("track.flac"), b"audio").unwrap();
        let owner = uuid::Uuid::new_v4();
        assert!(AuthorizedRoot::open(&root, owner).is_err());
        nominate(&root, owner).unwrap();
        let other = root.join("other");
        fs::create_dir(&other).unwrap();
        let lease = super::super::library_authority::acquire(&root, owner).unwrap();
        assert!(lease.validate_root(&File::open(&other).unwrap()).is_err());
        drop(lease);
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let mut file = authorized.open_file(Path::new("track.flac")).unwrap();
        drop(authorized);
        assert!(AuthorizedRoot::open(&root, owner).is_err());
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"audio");
        drop(file);
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        fs::remove_file(root.join(MARKER_NAME)).unwrap();
        assert!(authorized.open_file(Path::new("track.flac")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_stage_can_be_edited_without_touching_source_and_is_cleaned() {
        use super::super::library_authority::nominate;
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(root.join("track.flac"), b"source").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let path = stage.path().to_path_buf();
        assert_eq!(path.extension().unwrap(), "flac");
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(fs::read(&path).unwrap(), b"source");
        fs::write(&path, b"changed").unwrap();
        assert_eq!(fs::read(root.join("track.flac")).unwrap(), b"source");
        drop(stage);
        assert!(!path.exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn private_stage_cleanup_reports_unexpected_remaining_files() {
        use super::super::library_authority::nominate;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(root.join("track.flac"), b"source").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let extra = stage.path().parent().unwrap().join("unexpected");
        fs::write(&extra, b"keep").unwrap();
        assert!(stage.cleanup().is_err());
        assert_eq!(fs::read(extra).unwrap(), b"keep");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn private_stage_cleanup_never_follows_a_replaced_directory() {
        use super::super::library_authority::nominate;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        let sentinel = base.join("sentinel");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::create_dir_all(&sentinel).unwrap();
        fs::write(root.join("track.flac"), b"source").unwrap();
        fs::write(sentinel.join("source.flac"), b"do not delete").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let directory = stage.path().parent().unwrap().to_path_buf();
        fs::rename(&directory, work.join("moved")).unwrap();
        symlink(&sentinel, &directory).unwrap();
        assert!(stage.cleanup().is_err());
        assert_eq!(
            fs::read(sentinel.join("source.flac")).unwrap(),
            b"do not delete"
        );
        let stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let directory = stage.path().parent().unwrap().to_path_buf();
        fs::rename(&directory, work.join("moved-again")).unwrap();
        symlink(&sentinel, &directory).unwrap();
        drop(stage);
        assert_eq!(
            fs::read(sentinel.join("source.flac")).unwrap(),
            b"do not delete"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn staging_rejects_writable_workspace_and_preserves_replaced_file() {
        use super::super::library_authority::nominate;
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(root.join("track.flac"), b"source").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        fs::set_permissions(&work, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(authorized
            .stage_file(Path::new("track.flac"), &work)
            .is_err());
        fs::set_permissions(&work, fs::Permissions::from_mode(0o700)).unwrap();
        let stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let path = stage.path().to_path_buf();
        fs::rename(&path, path.with_extension("moved")).unwrap();
        fs::write(&path, b"do not delete").unwrap();
        assert!(stage.cleanup().is_err());
        assert_eq!(fs::read(path).unwrap(), b"do not delete");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn successful_atomic_stage_writer_can_be_cleaned_without_touching_source() {
        use super::super::library_authority::nominate;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(root.join("track.flac"), b"source").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let mut stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        stage
            .run_writer(|path| {
                let replacement = path.with_extension("new");
                fs::write(&replacement, b"output")?;
                fs::rename(replacement, path)
            })
            .unwrap();
        assert_eq!(fs::read(stage.path()).unwrap(), b"output");
        stage.cleanup().unwrap();
        assert_eq!(fs::read(root.join("track.flac")).unwrap(), b"source");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn writer_is_not_called_when_the_stage_path_was_replaced() {
        use super::super::library_authority::nominate;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        let sentinel = base.join("sentinel");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::create_dir_all(&sentinel).unwrap();
        fs::write(root.join("track.flac"), b"source").unwrap();
        fs::write(sentinel.join("source.flac"), b"sentinel").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let mut stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let directory = stage.path().parent().unwrap().to_path_buf();
        fs::rename(&directory, work.join("moved")).unwrap();
        symlink(&sentinel, &directory).unwrap();
        let mut called = false;
        assert!(stage
            .run_writer(|_| {
                called = true;
                Ok(())
            })
            .is_err());
        assert!(!called);
        assert_eq!(fs::read(sentinel.join("source.flac")).unwrap(), b"sentinel");
        drop(stage);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn failed_writer_after_atomic_replacement_preserves_error_and_stage() {
        use super::super::library_authority::nominate;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(root.join("track.flac"), b"source").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let mut stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let path = stage.path().to_path_buf();
        let result: io::Result<()> = stage.run_writer(|path| {
            let replacement = path.with_extension("new");
            fs::write(&replacement, b"output")?;
            fs::rename(replacement, path)?;
            Err(io::Error::other("writer failed after replacement"))
        });
        let error = result.unwrap_err().to_string();
        assert!(error.contains("writer failed after replacement"));
        assert!(error.contains("retained for recovery"));
        assert!(stage.cleanup().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"output");
        assert_eq!(fs::read(root.join("track.flac")).unwrap(), b"source");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn lease_loss_after_atomic_stage_replacement_retains_stage() {
        use super::super::library_authority::{nominate, MARKER_NAME};
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(root.join("track.flac"), b"source").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let mut stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let result = stage.run_writer(|path| {
            let replacement = path.with_extension("new");
            fs::write(&replacement, b"output")?;
            fs::rename(replacement, path)?;
            fs::remove_file(root.join(MARKER_NAME))
        });
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("retained for recovery"));
        let path = stage.path().to_path_buf();
        assert!(stage.cleanup().is_err());
        assert_eq!(fs::read(path).unwrap(), b"output");
        assert_eq!(fs::read(root.join("track.flac")).unwrap(), b"source");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn unverified_successful_writer_output_reports_retained_stage() {
        use super::super::library_authority::nominate;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(root.join("track.flac"), b"source").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let mut stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let path = stage.path().to_path_buf();
        let result = stage.run_writer(|path| {
            let replacement = path.with_extension("new");
            fs::write(&replacement, b"output")?;
            fs::hard_link(&replacement, path.with_extension("link"))?;
            fs::rename(replacement, path)
        });
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("retained for recovery"));
        assert!(stage.cleanup().is_err());
        assert_eq!(fs::read(path).unwrap(), b"output");
        assert_eq!(fs::read(root.join("track.flac")).unwrap(), b"source");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn failed_in_place_writer_keeps_changed_stage_after_drop() {
        use super::super::library_authority::nominate;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(root.join("track.flac"), b"source").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let mut stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let path = stage.path().to_path_buf();
        let result: io::Result<()> = stage.run_writer(|path| {
            fs::write(path, b"changed")?;
            Err(io::Error::other("in-place writer failed"))
        });
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("retained for recovery"));
        drop(stage);
        assert_eq!(fs::read(&path).unwrap(), b"changed");
        assert_eq!(fs::read(root.join("track.flac")).unwrap(), b"source");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejected_in_place_output_remains_after_drop() {
        use super::super::library_authority::nominate;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(root.join("track.flac"), b"source").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let mut stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let path = stage.path().to_path_buf();
        let result = stage.run_writer(|path| {
            fs::write(path, b"changed")?;
            fs::hard_link(path, path.with_extension("link"))
        });
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("retained for recovery"));
        drop(stage);
        assert_eq!(fs::read(&path).unwrap(), b"changed");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn panicking_in_place_writer_keeps_the_changed_stage() {
        use super::super::library_authority::nominate;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(root.join("track.flac"), b"source").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let mut stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let path = stage.path().to_path_buf();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: io::Result<()> = stage.run_writer(|path| {
                fs::write(path, b"changed")?;
                panic!("writer panicked");
            });
        }));
        assert!(result.is_err());
        drop(stage);
        assert_eq!(fs::read(path).unwrap(), b"changed");
        assert_eq!(fs::read(root.join("track.flac")).unwrap(), b"source");
        fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn existing_native_writer_updates_only_the_queued_private_stage() {
        use super::super::library_authority::nominate;
        use crate::commands::mutations::{write_track_dispatch, TrackPatch};
        use crate::commands::tracks::read_track_metadata;
        use crate::state::write_queue::WriteQueue;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus/minimal.wav");
        let original = root.join("track.wav");
        fs::copy(fixture, &original).unwrap();
        let before = fs::read(&original).unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let mut stage = authorized
            .stage_file(Path::new("track.wav"), &work)
            .unwrap();
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Staged title"})).unwrap();
        WriteQueue::default()
            .run_checked(async {
                stage.run_writer(|path| {
                    write_track_dispatch(path, &patch)
                        .map(|_| ())
                        .map_err(|error| io::Error::other(error.to_string()))
                })
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            read_track_metadata(stage.path()).unwrap().title.as_deref(),
            Some("Staged title")
        );
        assert_eq!(fs::read(&original).unwrap(), before);
        stage.cleanup().unwrap();
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn copied_filesystem_metadata_preserves_mode_and_extended_attribute() {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let source_path = root.join("source");
        let target_path = root.join("target");
        fs::write(&source_path, b"source").unwrap();
        fs::write(&target_path, b"output").unwrap();
        fs::set_permissions(&source_path, fs::Permissions::from_mode(0o640)).unwrap();
        let source = File::open(source_path).unwrap();
        let target = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&target_path)
            .unwrap();
        let name = c"com.soundrobe.fixture";
        let value = b"keep";
        assert_eq!(
            unsafe {
                libc::fsetxattr(
                    source.as_raw_fd(),
                    name.as_ptr(),
                    value.as_ptr().cast(),
                    value.len(),
                    0,
                    0,
                )
            },
            0
        );
        copy_filesystem_metadata(&source, &target).unwrap();
        let mut readback = [0u8; 16];
        let size = unsafe {
            libc::fgetxattr(
                target.as_raw_fd(),
                name.as_ptr(),
                readback.as_mut_ptr().cast(),
                readback.len(),
                0,
                0,
            )
        };
        assert_eq!(&readback[..size as usize], value);
        let before = source.metadata().unwrap();
        let after = target.metadata().unwrap();
        assert_eq!(
            (after.mode() & 0o777, after.uid(), after.gid()),
            (before.mode() & 0o777, before.uid(), before.gid())
        );
        assert_eq!(fs::read(target_path).unwrap(), b"output");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_source_revalidation_rejects_in_place_edits_and_path_replacement() {
        use super::super::library_authority::nominate;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        let source = root.join("track.flac");
        fs::write(&source, b"source").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        stage.revalidate_source().unwrap();
        fs::write(&source, b"change").unwrap();
        assert!(stage.revalidate_source().is_err());
        drop(stage);
        let stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        fs::rename(&source, root.join("old.flac")).unwrap();
        fs::write(&source, b"change").unwrap();
        assert!(stage.revalidate_source().is_err());
        drop(stage);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn source_revalidation_rechecks_identity_after_digest() {
        use super::super::library_authority::nominate;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        let source = root.join("track.flac");
        fs::write(&source, b"source").unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authorized = AuthorizedRoot::open(&root, owner).unwrap();
        let stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let result = stage.revalidate_source_with(|| {
            fs::rename(&source, root.join("old.flac")).unwrap();
            fs::write(&source, b"source").unwrap();
        });
        assert!(result.is_err());
        drop(stage);
        let stage = authorized
            .stage_file(Path::new("track.flac"), &work)
            .unwrap();
        let result = stage
            .revalidate_source_with(|| fs::hard_link(&source, root.join("extra.flac")).unwrap());
        assert!(result.is_err());
        drop(stage);
        fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn validated_stage_publishes_under_queue_with_durable_backup() {
        use crate::commands::mutations::{track_payload_digest, write_track_dispatch, TrackPatch};
        use crate::commands::tracks::read_track_metadata;
        use crate::state::library_authority::nominate;
        use crate::state::worker::WorkerStore;
        use crate::state::write_queue::{current_queue_lease, WriteQueue};
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        let path = root.join("track.wav");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.wav"),
            &path,
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let original = fs::read(&path).unwrap();
        let payload = track_payload_digest(&path).unwrap();
        let source = File::open(&path).unwrap();
        let attr = c"com.soundrobe.fixture";
        assert_eq!(
            unsafe {
                libc::fsetxattr(
                    source.as_raw_fd(),
                    attr.as_ptr(),
                    b"keep".as_ptr().cast(),
                    4,
                    0,
                    0,
                )
            },
            0
        );
        drop(source);
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authority = AuthorizedRoot::open(&root, owner).unwrap();
        let store = WorkerStore::new_in(base.join("profile"));
        assert!(store.initialize());
        let patch_value = serde_json::json!({"title":"Published title"});
        let intent = store
            .prepare_write_intent(&path, "run", &patch_value)
            .unwrap();
        let backup = intent.backup_path.unwrap();
        let mut stage = authority.stage_file(Path::new("track.wav"), &work).unwrap();
        store
            .bind_write_authority(&path, "run", stage.lease(), Path::new("track.wav"))
            .unwrap();
        let patch: TrackPatch = serde_json::from_value(patch_value).unwrap();
        let queue = WriteQueue::for_profile(&base.join("profile"), None)
            .unwrap()
            .with_recovery_worker(std::sync::Arc::new(store.clone()))
            .unwrap();
        let rejected = queue
            .run_checked(async {
                stage.run_writer(|staged| {
                    write_track_dispatch(staged, &patch)
                        .map(|_| ())
                        .map_err(|error| io::Error::other(error.to_string()))
                })?;
                let queue_lease = current_queue_lease().unwrap();
                stage.publish_with_intent(
                    &store,
                    &path,
                    "run",
                    Path::new("track.wav"),
                    &queue_lease,
                )
            })
            .await
            .unwrap();
        assert!(rejected.is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        let unbound = WriteQueue::for_profile(&base.join("profile"), None).unwrap();
        let rejected = unbound
            .run_exclusive_checked(async {
                let queue_lease = current_queue_lease().unwrap();
                stage.publish_with_intent(
                    &store,
                    &path,
                    "run",
                    Path::new("track.wav"),
                    &queue_lease,
                )
            })
            .await
            .unwrap();
        assert!(rejected.is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        let saved_backup = backup.with_extension("held");
        fs::rename(&backup, &saved_backup).unwrap();
        let rejected = queue
            .run_exclusive_checked(async {
                let queue_lease = current_queue_lease().unwrap();
                stage.publish_with_intent(
                    &store,
                    &path,
                    "run",
                    Path::new("track.wav"),
                    &queue_lease,
                )
            })
            .await
            .unwrap();
        assert!(
            rejected.is_err(),
            "publication must not proceed without the durable backup"
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::rename(saved_backup, &backup).unwrap();
        queue
            .run_exclusive_checked(async {
                let queue_lease = current_queue_lease().unwrap();
                stage.publish_with_intent(
                    &store,
                    &path,
                    "run",
                    Path::new("track.wav"),
                    &queue_lease,
                )
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("Published title")
        );
        assert_eq!(track_payload_digest(&path).unwrap(), payload);
        assert_eq!(fs::read(backup).unwrap(), original);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let output = File::open(&path).unwrap();
        let mut value = [0u8; 8];
        let count = unsafe {
            libc::fgetxattr(
                output.as_raw_fd(),
                attr.as_ptr(),
                value.as_mut_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        assert_eq!(&value[..count as usize], b"keep");
        assert_eq!(
            store
                .reconcile_write_intent_authorized(
                    &path,
                    "run",
                    stage.lease(),
                    Path::new("track.wav")
                )
                .unwrap(),
            "verified"
        );
        stage.cleanup().unwrap();
        fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn publication_failure_after_begin_keeps_original_and_stage_for_recovery() {
        use crate::commands::mutations::{write_track_dispatch, TrackPatch};
        use crate::state::library_authority::nominate;
        use crate::state::worker::WorkerStore;
        use crate::state::write_queue::{current_queue_lease, WriteQueue};
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        let path = root.join("track.wav");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.wav"),
            &path,
        )
        .unwrap();
        let before = fs::read(&path).unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authority = AuthorizedRoot::open(&root, owner).unwrap();
        let worker = WorkerStore::new_in(base.join("profile"));
        assert!(worker.initialize());
        let patch = serde_json::json!({"title":"Staged title"});
        let backup = worker
            .prepare_write_intent(&path, "run", &patch)
            .unwrap()
            .backup_path
            .unwrap();
        let saved = backup.with_extension("held");
        let mut stage = authority.stage_file(Path::new("track.wav"), &work).unwrap();
        worker
            .bind_write_authority(&path, "run", stage.lease(), Path::new("track.wav"))
            .unwrap();
        let patch: TrackPatch = serde_json::from_value(patch).unwrap();
        let queue = WriteQueue::for_profile(&base.join("profile"), None)
            .unwrap()
            .with_recovery_worker(std::sync::Arc::new(worker.clone()))
            .unwrap();
        let result = queue
            .run_exclusive_checked(async {
                stage.run_writer(|staged| {
                    write_track_dispatch(staged, &patch)
                        .map(|_| ())
                        .map_err(|error| io::Error::other(error.to_string()))
                })?;
                let queue_lease = current_queue_lease().unwrap();
                stage.publish_with_intent_and_hooks(
                    &worker,
                    &path,
                    "run",
                    Path::new("track.wav"),
                    &queue_lease,
                    || fs::rename(&backup, &saved).unwrap(),
                    |_| {},
                    || {},
                    || {},
                )
            })
            .await
            .unwrap();
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("stage and backup retained"));
        assert_eq!(fs::read(&path).unwrap(), before);
        let staged_path = stage.path().to_path_buf();
        assert!(stage.cleanup().is_err());
        assert!(staged_path.exists());
        fs::rename(saved, backup).unwrap();
        assert_eq!(
            worker
                .reconcile_write_intent_authorized(
                    &path,
                    "run",
                    &authority.lease,
                    Path::new("track.wav")
                )
                .unwrap(),
            "retry_safe"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn publication_does_not_certify_different_valid_audio_after_replace() {
        assert_publication_rejects_changed_file(PublicationChange::AfterRename).await;
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn failed_final_publication_readback_holds_intent_for_recovery() {
        assert_publication_rejects_changed_file(PublicationChange::FinalReadback).await;
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn publication_preserves_new_destination_at_replacement_boundary() {
        assert_publication_rejects_changed_file(PublicationChange::Destination).await;
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn publication_preserves_displaced_source_if_temporary_changes() {
        assert_publication_rejects_changed_file(PublicationChange::Temporary).await;
    }

    #[derive(Clone, Copy, PartialEq)]
    #[cfg(target_os = "macos")]
    enum PublicationChange {
        AfterRename,
        FinalReadback,
        Destination,
        Temporary,
    }

    #[cfg(target_os = "macos")]
    async fn assert_publication_rejects_changed_file(change: PublicationChange) {
        use std::os::unix::ffi::OsStrExt;
        let mut displaced = None;
        use crate::commands::mutations::{write_track_dispatch, TrackPatch};
        use crate::state::library_authority::nominate;
        use crate::state::worker::WorkerStore;
        use crate::state::write_queue::{current_queue_lease, WriteQueue};
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        let path = root.join("track.wav");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.wav"),
            &path,
        )
        .unwrap();
        let before = fs::read(&path).unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let authority = AuthorizedRoot::open(&root, owner).unwrap();
        let worker = WorkerStore::new_in(base.join("profile"));
        assert!(worker.initialize());
        let patch = serde_json::json!({"title":"Expected title"});
        let backup = worker
            .prepare_write_intent(&path, "run", &patch)
            .unwrap()
            .backup_path
            .unwrap();
        let mut stage = authority.stage_file(Path::new("track.wav"), &work).unwrap();
        worker
            .bind_write_authority(&path, "run", stage.lease(), Path::new("track.wav"))
            .unwrap();
        let patch: TrackPatch = serde_json::from_value(patch).unwrap();
        let unrelated: TrackPatch =
            serde_json::from_value(serde_json::json!({"artist":"Injected artist"})).unwrap();
        let queue = WriteQueue::for_profile(&base.join("profile"), None)
            .unwrap()
            .with_recovery_worker(std::sync::Arc::new(worker.clone()))
            .unwrap();
        let result = queue
            .run_exclusive_checked(async {
                stage.run_writer(|staged| {
                    write_track_dispatch(staged, &patch)
                        .map(|_| ())
                        .map_err(|error| io::Error::other(error.to_string()))
                })?;
                let queue_lease = current_queue_lease().unwrap();
                stage.publish_with_intent_and_hooks(
                    &worker,
                    &path,
                    "run",
                    Path::new("track.wav"),
                    &queue_lease,
                    || {},
                    |name| {
                        let temporary = root.join(std::ffi::OsStr::from_bytes(name.to_bytes()));
                        if change == PublicationChange::Destination {
                            fs::rename(&path, root.join("moved.wav")).unwrap();
                            fs::write(&path, b"new destination data").unwrap();
                            displaced = Some((temporary, b"new destination data".to_vec()));
                        } else if change == PublicationChange::Temporary {
                            fs::rename(&temporary, root.join("moved.wav")).unwrap();
                            fs::write(&temporary, b"substituted temporary data").unwrap();
                            displaced = Some((temporary, before.clone()));
                        }
                    },
                    || {
                        if change == PublicationChange::AfterRename {
                            write_track_dispatch(&path, &unrelated).unwrap();
                        }
                    },
                    || {
                        if change == PublicationChange::FinalReadback {
                            write_track_dispatch(&path, &unrelated).unwrap();
                        }
                    },
                )
            })
            .await
            .unwrap();
        assert!(
            result.is_err(),
            "different valid metadata must not be certified as the staged result"
        );
        assert_eq!(fs::read(backup).unwrap(), before);
        if let Some((path, expected)) = displaced {
            assert_eq!(
                fs::read(path).expect("displaced entry must survive"),
                expected
            );
        }
        let restarted = WorkerStore::new_in(base.join("profile"));
        assert!(restarted.initialize());
        assert_eq!(
            restarted
                .reconcile_write_intent_authorized(
                    &path,
                    "run",
                    stage.lease(),
                    Path::new("track.wav")
                )
                .unwrap(),
            "needs_recovery"
        );
        assert!(stage.cleanup().is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "subprocess entry for staged publication crash recovery"]
    fn staged_publication_crash_child() {
        use crate::commands::mutations::{write_track_dispatch, TrackPatch};
        use crate::state::worker::WorkerStore;
        use crate::state::write_queue::{current_queue_lease, WriteQueue};
        let base =
            std::path::PathBuf::from(std::env::var_os("SOUNDROBE_PUBLISH_CRASH_ROOT").unwrap());
        let owner = uuid::Uuid::parse_str(&std::env::var("SOUNDROBE_PUBLISH_CRASH_OWNER").unwrap())
            .unwrap();
        let root = base.join("library");
        let work = base.join("private");
        let path = root.join("track.wav");
        let authority = AuthorizedRoot::open(&root, owner).unwrap();
        let worker = WorkerStore::new_in(base.join("profile"));
        assert!(worker.initialize());
        worker.claim_desktop_run(&root, "run").unwrap();
        let patch = serde_json::json!({"title":"Published before crash"});
        worker.prepare_write_intent(&path, "run", &patch).unwrap();
        let mut stage = authority.stage_file(Path::new("track.wav"), &work).unwrap();
        worker
            .bind_write_authority(&path, "run", stage.lease(), Path::new("track.wav"))
            .unwrap();
        let patch: TrackPatch = serde_json::from_value(patch).unwrap();
        let queue = WriteQueue::for_profile(&base.join("profile"), None)
            .unwrap()
            .with_recovery_worker(std::sync::Arc::new(worker.clone()))
            .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(queue.run_exclusive_checked(async {
            stage.run_writer(|staged| {
                write_track_dispatch(staged, &patch)
                    .map(|_| ())
                    .map_err(|error| io::Error::other(error.to_string()))
            })?;
            let queue_lease = current_queue_lease().unwrap();
            stage.publish_with_intent_and_hooks(
                &worker,
                &path,
                "run",
                Path::new("track.wav"),
                &queue_lease,
                || {},
                |_| {},
                || std::process::exit(71),
                || {},
            )
        }));
        panic!("publication did not crash at the requested point: {result:?}");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn crash_after_staged_replace_keeps_backup_and_requires_recovery() {
        use crate::commands::mutations::track_payload_digest;
        use crate::commands::tracks::read_track_metadata;
        use crate::state::library_authority::{acquire, nominate, AuthorityError};
        use crate::state::worker::WorkerStore;
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = base.join("library");
        let work = base.join("private");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&work).unwrap();
        let path = root.join("track.wav");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.wav"),
            &path,
        )
        .unwrap();
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let original_inode = fs::metadata(&path).unwrap().ino();
        let before = fs::read(&path).unwrap();
        let payload = track_payload_digest(&path).unwrap();
        let owner = uuid::Uuid::new_v4();
        nominate(&root, owner).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "state::authority_io::tests::staged_publication_crash_child",
                "--ignored",
            ])
            .env("SOUNDROBE_PUBLISH_CRASH_ROOT", &base)
            .env("SOUNDROBE_PUBLISH_CRASH_OWNER", owner.to_string())
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
                panic!("publication child did not finish");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(status.code(), Some(71));
        let displaced: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".soundrobe-publish-"))
            })
            .collect();
        assert_eq!(displaced.len(), 1);
        assert_eq!(fs::read(&displaced[0]).unwrap(), before);
        let metadata = fs::metadata(&displaced[0]).unwrap();
        assert_eq!(metadata.ino(), original_inode);
        assert_eq!(metadata.mode() & 0o777, 0o640);
        assert!(matches!(
            acquire(&root, uuid::Uuid::new_v4()),
            Err(AuthorityError::OtherProfile)
        ));
        let lease = acquire(&root, owner).unwrap();
        let worker = WorkerStore::new_in(base.join("profile"));
        assert!(worker.initialize());
        let records = worker.reconcile_write_intents().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].intent.state, "applying");
        assert_eq!(
            fs::read(records[0].intent.backup_path.as_ref().unwrap()).unwrap(),
            before
        );
        assert_eq!(
            worker
                .reconcile_write_intent_authorized(&path, "run", &lease, Path::new("track.wav"))
                .unwrap(),
            "needs_recovery"
        );
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("Published before crash")
        );
        assert_eq!(track_payload_digest(&path).unwrap(), payload);
        let stages: Vec<_> = fs::read_dir(&work)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(stages.len(), 1);
        assert!(stages[0].join("source.wav").exists());
        fs::remove_dir_all(base).unwrap();
    }
}
