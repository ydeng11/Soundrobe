//! Process-wide serialization and activity tracking for media writes.
//!
//! The write queue supports two locking modes:
//!   - `run()`: global lock (serialises all operations) — used by single-file
//!     writes, renames, extra-tag edits, and delete operations.
//!   - `run_for_folder()`: per-folder lock — used by batch write so tracks in
//!     *different* album folders can be written concurrently.

use super::library_authority::{self, AuthorityError, AuthorityLease};
use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use tokio::sync::{Mutex, OwnedMutexGuard, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

const DEFAULT_FOLDER_CONCURRENCY: usize = 4;

/// All media mutations pass through one queue so two UI actions cannot race on
/// the same file and lifecycle code can block quit while work waits or runs.
///
/// Cloning is cheap — the inner state is shared via `Arc`. This allows passing
/// the queue to spawned tasks for concurrent per-folder batch writes.
#[derive(Clone)]
pub struct WriteQueue(Arc<WriteQueueInner>);

struct WriteQueueInner {
    max_folder_concurrency: usize,
    /// Shared operations may proceed under a read guard; assistant metadata
    /// batches use the write guard to keep preflight, writes, and readback atomic.
    coordination: Arc<RwLock<()>>,
    /// Global serialisation gate for single-file and extra-tag operations.
    gate: Arc<Mutex<()>>,
    /// Per-folder gates keyed by parent directory.
    /// Each folder gets its own `Mutex<()>` — writes to different
    /// album folders can proceed concurrently.
    folder_gates: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
    active: AtomicUsize,
    profile_lock: Option<PathBuf>,
    recovery: std::sync::OnceLock<Arc<crate::state::worker::WorkerStore>>,
    authority: OnceLock<ProfileWriteAuthority>,
}

struct ProfileWriteAuthority {
    owner: uuid::Uuid,
    active: std::sync::Mutex<HashMap<PathBuf, Weak<AuthorityLease>>>,
}

impl Default for WriteQueue {
    fn default() -> Self {
        Self::with_concurrency(None)
    }
}

impl WriteQueue {
    pub fn with_concurrency(configured: Option<usize>) -> Self {
        let max_folder_concurrency = configured
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_FOLDER_CONCURRENCY);
        Self(Arc::new(WriteQueueInner {
            max_folder_concurrency,
            coordination: Arc::new(RwLock::new(())),
            gate: Arc::new(Mutex::new(())),
            folder_gates: Mutex::new(HashMap::new()),
            active: AtomicUsize::new(0),
            profile_lock: None,
            recovery: std::sync::OnceLock::new(),
            authority: OnceLock::new(),
        }))
    }

    /// Share write coordination between runtimes using the same data directory.
    pub fn for_profile(data_dir: &Path, configured: Option<usize>) -> io::Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let lock_path = std::fs::canonicalize(data_dir)?.join("write-queue.lock");
        let file = open_profile_lock(&lock_path, true)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("Write queue lock is not a regular file"));
        }
        let max_folder_concurrency = configured
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_FOLDER_CONCURRENCY);
        Ok(Self(Arc::new(WriteQueueInner {
            max_folder_concurrency,
            coordination: Arc::new(RwLock::new(())),
            gate: Arc::new(Mutex::new(())),
            folder_gates: Mutex::new(HashMap::new()),
            active: AtomicUsize::new(0),
            profile_lock: Some(lock_path),
            recovery: std::sync::OnceLock::new(),
            authority: OnceLock::new(),
        })))
    }

    pub(crate) fn with_library_authority(self) -> Result<Self, AuthorityError> {
        let profile = self
            .0
            .profile_lock
            .as_deref()
            .and_then(Path::parent)
            .ok_or_else(|| AuthorityError::Invalid("write authority requires a profile".into()))?;
        let owner = library_authority::load_or_create_profile_id(profile)?;
        self.0
            .authority
            .set(ProfileWriteAuthority {
                owner,
                active: Default::default(),
            })
            .map_err(|_| AuthorityError::Invalid("write authority is already configured".into()))?;
        Ok(self)
    }

    pub(crate) fn admit_existing_file(
        &self,
        path: &Path,
    ) -> Result<Option<Arc<AuthorityLease>>, AuthorityError> {
        self.admit_write_target(path, false)
    }

    pub(crate) fn admit_write_target(
        &self,
        path: &Path,
        allow_missing: bool,
    ) -> Result<Option<Arc<AuthorityLease>>, AuthorityError> {
        self.admit_library(|| library_authority::root_for_write_target(path, allow_missing))
    }

    pub(crate) fn admit_album(
        &self,
        path: &Path,
    ) -> Result<Option<Arc<AuthorityLease>>, AuthorityError> {
        self.admit_library(|| library_authority::root_for_directory(path))
    }

    fn admit_library(
        &self,
        root: impl FnOnce() -> Result<PathBuf, AuthorityError>,
    ) -> Result<Option<Arc<AuthorityLease>>, AuthorityError> {
        let Some(authority) = self.0.authority.get() else {
            return Ok(None);
        };
        let root = root()?;
        let mut active = authority
            .active
            .lock()
            .map_err(|_| AuthorityError::Invalid("write authority state is unavailable".into()))?;
        if let Some(lease) = active.get(&root).and_then(Weak::upgrade) {
            lease.validate()?;
            return Ok(Some(lease));
        }
        active.retain(|_, lease| lease.strong_count() > 0);
        let lease = Arc::new(library_authority::acquire(&root, authority.owner)?);
        active.insert(root, Arc::downgrade(&lease));
        Ok(Some(lease))
    }

    pub(crate) fn reconcile_at_startup(
        &self,
        worker: &crate::state::worker::WorkerStore,
    ) -> Result<(), String> {
        worker.reconcile_writes_at_startup()?;
        #[cfg(unix)]
        self.reconcile_marker_writes(worker)?;
        Ok(())
    }

    #[cfg(unix)]
    fn reconcile_marker_writes(
        &self,
        worker: &crate::state::worker::WorkerStore,
    ) -> Result<(), String> {
        let pending = worker.pending_marker_writes()?;
        if pending.is_empty() {
            return Ok(());
        }
        let profile = std::fs::canonicalize(worker.data_dir()).map_err(|err| err.to_string())?;
        let lock_path = self
            .0
            .profile_lock
            .as_deref()
            .filter(|path| path.parent() == Some(profile.as_path()))
            .ok_or("marker recovery requires the matching profile queue")?;
        let Ok(_coordination) = self.0.coordination.try_write() else {
            tracing::warn!("Marker recovery deferred while the profile queue is active");
            return Ok(());
        };
        let lock = open_profile_lock(lock_path, false).map_err(|err| err.to_string())?;
        match lock_profile_file(&lock, true, true) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                tracing::warn!(
                    "Marker recovery deferred while another runtime holds the profile queue"
                );
                return Ok(());
            }
            Err(error) => return Err(error.to_string()),
        }
        let owner = library_authority::load_or_create_profile_id(&profile)
            .map_err(|error| error.to_string())?;
        for (run_id, path, relative) in pending {
            let result = library_authority::root_for_write_target(&path, false)
                .and_then(|root| library_authority::acquire(&root, owner))
                .map_err(|error| error.to_string())
                .and_then(|lease| {
                    worker.reconcile_write_intent_authorized(&path, &run_id, &lease, &relative)
                });
            if let Err(error) = result {
                tracing::warn!(?path, %run_id, %error, "Marker-bound recovery could not be confirmed");
            }
        }
        Ok(())
    }

    pub(crate) fn with_recovery_worker(
        self,
        worker: Arc<crate::state::worker::WorkerStore>,
    ) -> io::Result<Self> {
        self.0
            .recovery
            .set(worker)
            .map_err(|_| io::Error::other("write queue recovery is already configured"))?;
        Ok(self)
    }

    pub(crate) fn recovery_worker(&self) -> Option<Arc<crate::state::worker::WorkerStore>> {
        self.0.recovery.get().cloned()
    }

    #[cfg(test)]
    pub(crate) fn with_initialized_test_worker(self) -> Self {
        let profile = self
            .0
            .profile_lock
            .as_deref()
            .and_then(Path::parent)
            .unwrap();
        let worker = crate::state::worker::WorkerStore::new_in(profile.to_owned());
        assert!(worker.initialize());
        self.with_recovery_worker(Arc::new(worker)).unwrap()
    }

    async fn profile_guard(
        &self,
        exclusive: bool,
        nonblocking: bool,
    ) -> io::Result<Option<std::fs::File>> {
        if self.0.authority.get().is_some() && self.0.recovery.get().is_none() {
            return Err(io::Error::other(
                "Library write authority requires a recovery worker",
            ));
        }
        let Some(path) = self.0.profile_lock.clone() else {
            return Ok(None);
        };
        let file = open_profile_lock(&path, false).map_err(|error| {
            if nonblocking && error.kind() == io::ErrorKind::WouldBlock {
                io::Error::other(format!("Cannot open write queue lock: {error}"))
            } else {
                error
            }
        })?;
        loop {
            match lock_profile_file(&file, exclusive, true) {
                Ok(()) => return Ok(Some(file)),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock && !nonblocking => {
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub fn max_folder_concurrency(&self) -> usize {
        self.0.max_folder_concurrency
    }

    pub fn is_active(&self) -> bool {
        self.0.active.load(Ordering::Acquire) > 0
    }

    /// Acquires the global lock. Every caller waits for all other `run` and
    /// `run_for_folder` callers that happen to acquire the *global* gate.
    /// Per-folder operations that use `run_for_folder` do **not** block on
    /// this global gate — they only wait for their folder-specific lock.
    #[cfg(test)]
    pub async fn run<F, T>(&self, operation: F) -> T
    where
        F: Future<Output = T>,
    {
        self.run_checked(operation)
            .await
            .expect("Cannot acquire shared write queue lock")
    }

    pub async fn run_checked<F, T>(&self, operation: F) -> io::Result<T>
    where
        F: Future<Output = T>,
    {
        let activity = ActivityGuard::new(Arc::clone(&self.0));
        let coordination = self.0.coordination.clone().read_owned().await;
        let gate = self.0.gate.clone().lock_owned().await;
        let profile = self.profile_guard(false, false).await.map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Cannot acquire shared write queue lock: {error}"),
            )
        })?;
        let lease = Arc::new(QueueLease {
            _profile: profile,
            _activity: activity,
            _coordination: QueueCoordination::Shared {
                _guard: coordination,
            },
            _gate: Some(gate),
        });
        Ok(ACTIVE_QUEUE_LEASE.scope(lease, operation).await)
    }

    /// Acquires a lock scoped to a single album folder.
    ///
    /// Different folders can run their operations concurrently; two writes to
    /// the **same** folder are serialised (same as `run()` on the global gate).
    ///
    /// The `active` counter is still incremented so the quit guard sees
    /// in-flight work regardless of which locking mode is used.
    #[cfg(test)]
    pub async fn run_for_folder<F, T>(&self, folder: &Path, operation: F) -> T
    where
        F: Future<Output = T>,
    {
        self.run_for_folder_checked(folder, operation)
            .await
            .expect("Cannot acquire shared write queue lock")
    }

    pub async fn run_for_folder_checked<F, T>(&self, folder: &Path, operation: F) -> io::Result<T>
    where
        F: Future<Output = T>,
    {
        let activity = ActivityGuard::new(Arc::clone(&self.0));
        let coordination = self.0.coordination.clone().read_owned().await;
        let folder_lock = {
            let mut gates = self.0.folder_gates.lock().await;
            gates
                .entry(folder.to_path_buf())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let gate = folder_lock.lock_owned().await;
        let profile = self.profile_guard(false, false).await.map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Cannot acquire shared write queue lock: {error}"),
            )
        })?;
        let lease = Arc::new(QueueLease {
            _profile: profile,
            _activity: activity,
            _coordination: QueueCoordination::Shared {
                _guard: coordination,
            },
            _gate: Some(gate),
        });
        Ok(ACTIVE_QUEUE_LEASE.scope(lease, operation).await)
    }

    /// Runs a multi-step mutation while excluding every other queued write.
    /// Callers must use the underlying dispatch functions inside this guard,
    /// because calling `run` or `run_for_folder` recursively would deadlock.
    #[cfg(test)]
    pub async fn run_exclusive<F, T>(&self, operation: F) -> T
    where
        F: Future<Output = T>,
    {
        self.run_exclusive_checked(operation)
            .await
            .expect("Cannot acquire exclusive write queue lock")
    }

    /// Like `run_exclusive`, but reports profile lock failures to the caller.
    pub async fn run_exclusive_checked<F, T>(&self, operation: F) -> io::Result<T>
    where
        F: Future<Output = T>,
    {
        let activity = ActivityGuard::new(Arc::clone(&self.0));
        let coordination = self.0.coordination.clone().write_owned().await;
        let profile = self.profile_guard(true, false).await.map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Cannot acquire exclusive write queue lock: {error}"),
            )
        })?;
        let lease = Arc::new(QueueLease {
            _profile: profile,
            _activity: activity,
            _coordination: QueueCoordination::Exclusive {
                _guard: coordination,
            },
            _gate: None,
        });
        Ok(ACTIVE_QUEUE_LEASE.scope(lease, operation).await)
    }

    /// Claim exclusive coordination only when no write is already active.
    /// Used by application replacement, which must reject instead of waiting
    /// behind disk work the user may still expect to complete in this process.
    #[cfg(test)]
    pub async fn try_run_exclusive<F, T>(&self, operation: F) -> Option<T>
    where
        F: Future<Output = T>,
    {
        self.try_run_exclusive_checked(operation)
            .await
            .expect("Cannot acquire exclusive write queue lock")
    }

    pub async fn try_run_exclusive_checked<F, T>(&self, operation: F) -> io::Result<Option<T>>
    where
        F: Future<Output = T>,
    {
        if self.is_active() {
            return Ok(None);
        }
        let Ok(coordination) = self.0.coordination.clone().try_write_owned() else {
            return Ok(None);
        };
        if self.is_active() {
            return Ok(None);
        }
        let activity = ActivityGuard::new(Arc::clone(&self.0));
        let profile = match self.profile_guard(true, true).await {
            Ok(guard) => guard,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error),
        };
        let lease = Arc::new(QueueLease {
            _profile: profile,
            _activity: activity,
            _coordination: QueueCoordination::Exclusive {
                _guard: coordination,
            },
            _gate: None,
        });
        let output = ACTIVE_QUEUE_LEASE.scope(lease, operation).await;
        Ok(Some(output))
    }
}

fn open_profile_lock(path: &Path, create: bool) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("Write queue lock is not a regular file"));
    }
    Ok(file)
}

#[cfg(unix)]
fn lock_profile_file(file: &std::fs::File, exclusive: bool, nonblocking: bool) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let mut operation = if exclusive {
        libc::LOCK_EX
    } else {
        libc::LOCK_SH
    };
    if nonblocking {
        operation |= libc::LOCK_NB;
    }
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(windows)]
fn lock_profile_file(file: &std::fs::File, exclusive: bool, nonblocking: bool) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;
    let flags = if exclusive {
        LOCKFILE_EXCLUSIVE_LOCK
    } else {
        0
    } | if nonblocking {
        LOCKFILE_FAIL_IMMEDIATELY
    } else {
        0
    };
    let mut overlapped = OVERLAPPED::default();
    if unsafe { LockFileEx(file.as_raw_handle().cast(), flags, 0, 1, 0, &mut overlapped) } != 0 {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        if nonblocking && error.raw_os_error() == Some(33) {
            Err(io::Error::from(io::ErrorKind::WouldBlock))
        } else {
            Err(error)
        }
    }
}

struct ActivityGuard {
    inner: Arc<WriteQueueInner>,
}

impl ActivityGuard {
    fn new(inner: Arc<WriteQueueInner>) -> Self {
        inner.active.fetch_add(1, Ordering::AcqRel);
        Self { inner }
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        self.inner.active.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct QueueLease {
    _profile: Option<std::fs::File>,
    _activity: ActivityGuard,
    _coordination: QueueCoordination,
    _gate: Option<OwnedMutexGuard<()>>,
}

impl QueueLease {
    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn is_exclusive(&self) -> bool {
        matches!(&self._coordination, QueueCoordination::Exclusive { .. })
    }

    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn protects_worker(
        &self,
        worker: &crate::state::worker::WorkerStore,
    ) -> io::Result<bool> {
        let inner = &self._activity.inner;
        let Some(profile_lock) = &inner.profile_lock else {
            return Ok(false);
        };
        let expected = std::fs::canonicalize(worker.data_dir())?.join("write-queue.lock");
        Ok(*profile_lock == expected
            && inner
                .recovery
                .get()
                .is_some_and(|configured| configured.same_store(worker)))
    }
}

enum QueueCoordination {
    Shared { _guard: OwnedRwLockReadGuard<()> },
    Exclusive { _guard: OwnedRwLockWriteGuard<()> },
}

tokio::task_local! { static ACTIVE_QUEUE_LEASE: Arc<QueueLease>; }

pub(crate) fn current_queue_lease() -> Option<Arc<QueueLease>> {
    ACTIVE_QUEUE_LEASE.try_with(Arc::clone).ok()
}

pub(crate) fn current_write_queue() -> Option<WriteQueue> {
    ACTIVE_QUEUE_LEASE
        .try_with(|lease| WriteQueue(Arc::clone(&lease._activity.inner)))
        .ok()
}

pub(crate) fn current_recovery_worker() -> Option<Arc<crate::state::worker::WorkerStore>> {
    ACTIVE_QUEUE_LEASE
        .try_with(|lease| lease._activity.inner.recovery.get().cloned())
        .ok()
        .flatten()
}

pub(crate) async fn with_queue_lease<F: Future>(
    lease: Option<Arc<QueueLease>>,
    operation: F,
) -> F::Output {
    match lease {
        Some(lease) => ACTIVE_QUEUE_LEASE.scope(lease, operation).await,
        None => operation.await,
    }
}

pub(crate) fn spawn_write_blocking<F, T>(operation: F) -> tokio::task::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let lease = current_queue_lease();
    tokio::task::spawn_blocking(move || match lease {
        Some(lease) => ACTIVE_QUEUE_LEASE.sync_scope(lease, operation),
        None => operation(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tokio::sync::{Barrier, Notify};

    #[tokio::test]
    async fn unavailable_profile_lock_fails_all_admissions_without_polling() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let queue = WriteQueue::for_profile(&root, None).unwrap();
        std::fs::remove_file(root.join("write-queue.lock")).unwrap();
        let written = root.join("media");
        let error = queue
            .run_checked(async { std::fs::write(&written, b"x").unwrap() })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("write queue lock"));
        let error = queue
            .run_for_folder_checked(&root, async { std::fs::write(&written, b"x").unwrap() })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("write queue lock"));
        assert!(queue
            .try_run_exclusive_checked(async { std::fs::write(&written, b"x").unwrap() })
            .await
            .is_err());
        assert!(!written.exists());
        assert!(!queue.is_active());
        std::fs::remove_dir_all(root).unwrap();
    }

    async fn assert_eventually_exclusive(queue: &WriteQueue) {
        // A nonblocking attempt can see transient OS contention after the
        // holder task finishes, so require eventual release within a bound.
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if queue.try_run_exclusive(async {}).await.is_some() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("exclusive lock remained unavailable after the holder finished");
    }

    #[tokio::test]
    async fn unavailable_profile_lock_returns_error_without_running_write() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let queue = WriteQueue::for_profile(&root, None).unwrap();
        std::fs::remove_file(root.join("write-queue.lock")).unwrap();
        let written = root.join("media");
        let outcome = queue
            .run_exclusive_checked(async { std::fs::write(&written, b"changed") })
            .await;
        assert!(outcome.is_err());
        assert!(!written.exists());
        assert!(!queue.is_active());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    #[ignore = "subprocess entry for cross-process queue coordination"]
    async fn profile_lock_child() {
        let root = PathBuf::from(std::env::var_os("SOUNDROBE_TEST_QUEUE_ROOT").unwrap());
        let queue = WriteQueue::for_profile(&root, None).unwrap();
        queue
            .run_exclusive(async {
                std::fs::write(root.join("child-entered"), b"entered").unwrap();
            })
            .await;
    }

    #[tokio::test]
    async fn profile_queue_excludes_reversal_from_another_process_write() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let queue = WriteQueue::for_profile(&root, None).unwrap();
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let entered_task = Arc::clone(&entered);
        let release_task = Arc::clone(&release);
        let held = tokio::spawn(async move {
            queue
                .run(async {
                    entered_task.notify_one();
                    release_task.notified().await;
                })
                .await;
        });
        entered.notified().await;
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "state::write_queue::tests::profile_lock_child",
                "--ignored",
            ])
            .env("SOUNDROBE_TEST_QUEUE_ROOT", &root)
            .spawn()
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!root.join("child-entered").exists());
        assert!(child.try_wait().unwrap().is_none());
        release.notify_one();
        held.await.unwrap();
        assert!(child.wait().unwrap().success());
        assert!(root.join("child-entered").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn profile_queue_preserves_shared_writes_and_rejects_nonblocking_exclusive() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let first = WriteQueue::for_profile(&root, None).unwrap();
        let second = WriteQueue::for_profile(&root, None).unwrap();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());
        let first_entered = Arc::clone(&entered);
        let first_release = Arc::clone(&release);
        let holding = tokio::spawn(async move {
            first
                .run(async {
                    first_entered.wait().await;
                    first_release.notified().await;
                })
                .await
        });
        entered.wait().await;
        assert_eq!(second.run(async { 7 }).await, 7);
        assert!(second.try_run_exclusive(async { 8 }).await.is_none());
        release.notify_one();
        holding.await.unwrap();
        assert_eventually_exclusive(&second).await;
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn cancelled_request_keeps_profile_locked_until_blocking_write_finishes() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let first = WriteQueue::for_profile(&root, None).unwrap();
        let second = WriteQueue::for_profile(&root, None).unwrap();
        let observed = first.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            first
                .run(async move {
                    spawn_write_blocking(move || {
                        started_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        finished_tx.send(()).unwrap();
                    })
                    .await
                    .unwrap();
                })
                .await
        });
        started_rx.await.unwrap();
        task.abort();
        let _ = task.await;
        assert!(
            observed.is_active(),
            "A detached disk write still counts as active"
        );
        assert!(
            second.try_run_exclusive(async {}).await.is_none(),
            "Reversal must wait for the disk write"
        );
        release_tx.send(()).unwrap();
        finished_rx.await.unwrap();
        for _ in 0..50 {
            if !observed.is_active() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(!observed.is_active());
        assert_eventually_exclusive(&second).await;
        std::fs::remove_dir_all(root).unwrap();
    }

    async fn cancelled_write_keeps_gate_until_blocking_work_finishes(mode: u8) {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let queue = if mode >= 2 {
            WriteQueue::default()
        } else {
            WriteQueue::for_profile(&root, None).unwrap()
        };
        let first = queue.clone();
        let first_folder = root.clone();
        let (started_send, started_receive) = tokio::sync::oneshot::channel();
        let (release_send, release_receive) = std::sync::mpsc::channel();
        let (finished_send, finished_receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let operation = async move {
                spawn_write_blocking(move || {
                    started_send.send(()).unwrap();
                    release_receive.recv().unwrap();
                    finished_send.send(()).unwrap();
                })
                .await
                .unwrap();
            };
            match mode {
                0 => first.run_checked(operation).await,
                1 => first.run_for_folder_checked(&first_folder, operation).await,
                2 => first.run_exclusive_checked(operation).await,
                _ => first
                    .try_run_exclusive_checked(operation)
                    .await
                    .map(|result| result.expect("exclusive admission")),
            }
            .unwrap();
        });
        started_receive.await.unwrap();
        task.abort();
        let _ = task.await;
        let second = queue.clone();
        let second_folder = root.clone();
        let (entered_send, mut entered_receive) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            let operation = async {
                entered_send.send(()).unwrap();
            };
            if mode == 1 {
                second
                    .run_for_folder_checked(&second_folder, operation)
                    .await
                    .unwrap();
            } else {
                second.run_checked(operation).await.unwrap();
            }
        });
        let escaped =
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut entered_receive)
                .await
                .is_ok();
        release_send.send(()).unwrap();
        finished_receive.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), waiter)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !escaped,
            "a second write entered while the cancelled blocking write was still active"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn cancelled_write_keeps_same_queue_serialized_until_blocking_work_finishes() {
        cancelled_write_keeps_gate_until_blocking_work_finishes(0).await;
    }

    #[tokio::test]
    async fn cancelled_folder_write_keeps_folder_serialized_until_blocking_work_finishes() {
        cancelled_write_keeps_gate_until_blocking_work_finishes(1).await;
    }

    #[tokio::test]
    async fn cancelled_exclusive_write_keeps_default_queue_exclusive_until_blocking_work_finishes()
    {
        cancelled_write_keeps_gate_until_blocking_work_finishes(2).await;
    }

    #[tokio::test]
    async fn cancelled_try_exclusive_keeps_default_queue_exclusive_until_blocking_work_finishes() {
        cancelled_write_keeps_gate_until_blocking_work_finishes(3).await;
    }

    #[tokio::test]
    async fn cancelled_native_write_finishes_before_the_next_edit_of_the_same_file() {
        use crate::commands::mutations::{track_payload_digest, write_track_dispatch, TrackPatch};
        use crate::commands::tracks::read_track_metadata;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("track.wav");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.wav"),
            &path,
        )
        .unwrap();
        let payload = track_payload_digest(&path).unwrap();
        let queue = WriteQueue::for_profile(&root.join("profile"), None).unwrap();
        let first = queue.clone();
        let first_path = path.clone();
        let (started_send, started_receive) = tokio::sync::oneshot::channel();
        let (release_send, release_receive) = std::sync::mpsc::channel();
        let (finished_send, finished_receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            first
                .run_checked(async move {
                    spawn_write_blocking(move || {
                        started_send.send(()).unwrap();
                        release_receive.recv().unwrap();
                        let patch: TrackPatch =
                            serde_json::from_value(serde_json::json!({"title":"first"})).unwrap();
                        let result = write_track_dispatch(&first_path, &patch)
                            .map(|_| ())
                            .map_err(|error| error.to_string());
                        finished_send.send(result).unwrap();
                    })
                    .await
                    .unwrap();
                })
                .await
                .unwrap();
        });
        started_receive.await.unwrap();
        task.abort();
        let _ = task.await;
        let second = queue.clone();
        let second_path = path.clone();
        let (entered_send, mut entered_receive) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            second
                .run_checked(async move {
                    spawn_write_blocking(move || {
                        entered_send.send(()).unwrap();
                        let patch: TrackPatch =
                            serde_json::from_value(serde_json::json!({"title":"second"})).unwrap();
                        write_track_dispatch(&second_path, &patch)
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    })
                    .await
                    .unwrap()
                })
                .await
                .unwrap()
        });
        let escaped =
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut entered_receive)
                .await
                .is_ok();
        release_send.send(()).unwrap();
        finished_receive.await.unwrap().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            !escaped,
            "second write started while the cancelled disk write remained active"
        );
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("second")
        );
        assert_eq!(track_payload_digest(&path).unwrap(), payload);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn cancelled_profile_lock_wait_does_not_prevent_later_writes() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let first = WriteQueue::for_profile(&root, None).unwrap();
        let second = WriteQueue::for_profile(&root, None).unwrap();
        let observed = second.clone();
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let entered_task = Arc::clone(&entered);
        let release_task = Arc::clone(&release);
        let hold = tokio::spawn(async move {
            first
                .run_exclusive(async {
                    entered_task.notify_one();
                    release_task.notified().await;
                })
                .await
        });
        entered.notified().await;
        let waiter = tokio::spawn(async move { second.run(async {}).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(observed.is_active());
        waiter.abort();
        let _ = waiter.await;
        assert!(!observed.is_active());
        release.notify_one();
        hold.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), observed.run(async {}))
            .await
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn write_concurrency_is_explicit_and_shared_across_clones() {
        assert_eq!(WriteQueue::default().max_folder_concurrency(), 4);
        assert_eq!(
            WriteQueue::with_concurrency(Some(0)).max_folder_concurrency(),
            4
        );
        assert_eq!(
            WriteQueue::with_concurrency(Some(1)).max_folder_concurrency(),
            1
        );
        let configured = WriteQueue::with_concurrency(Some(8));
        assert_eq!(configured.max_folder_concurrency(), 8);
        assert_eq!(configured.clone().max_folder_concurrency(), 8);
    }

    /// Waiting operations count as active and execution stays serialized. This
    /// is what the quit guard needs: quitting with queued work is still unsafe.
    #[tokio::test]
    async fn serializes_operations_and_tracks_waiters() {
        let queue = WriteQueue::default();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());

        let first_queue = queue.clone();
        let first_entered = Arc::clone(&entered);
        let first_release = Arc::clone(&release);
        let first = tokio::spawn(async move {
            first_queue
                .run(async move {
                    first_entered.wait().await;
                    first_release.notified().await;
                })
                .await;
        });
        entered.wait().await;
        assert!(queue.is_active());

        let second_queue = queue.clone();
        let second_started = Arc::new(AtomicBool::new(false));
        let second_started_task = Arc::clone(&second_started);
        let second = tokio::spawn(async move {
            second_queue
                .run(async move {
                    second_started_task.store(true, Ordering::Release);
                })
                .await;
        });
        tokio::task::yield_now().await;
        assert!(queue.is_active());
        assert!(!second_started.load(Ordering::Acquire));

        release.notify_one();
        first.await.unwrap();
        second.await.unwrap();
        assert!(!queue.is_active());
    }

    /// Two writes to the same folder are serialised (second waits for first).
    #[tokio::test]
    async fn run_for_folder_serialises_same_folder() {
        let queue = WriteQueue::default();
        let folder = Path::new("/some/album");
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());

        let first_queue = queue.clone();
        let first_entered = Arc::clone(&entered);
        let first_release = Arc::clone(&release);
        let first_folder = folder.to_path_buf();
        let first = tokio::spawn(async move {
            first_queue
                .run_for_folder(&first_folder, async move {
                    first_entered.wait().await;
                    first_release.notified().await;
                })
                .await;
        });
        entered.wait().await;

        let second_queue = queue.clone();
        let second_started = Arc::new(AtomicBool::new(false));
        let second_started_task = Arc::clone(&second_started);
        let second_folder = folder.to_path_buf();
        let second = tokio::spawn(async move {
            second_queue
                .run_for_folder(&second_folder, async move {
                    second_started_task.store(true, Ordering::Release);
                })
                .await;
        });
        tokio::task::yield_now().await;
        // Second should NOT be started yet because same-folder lock is held.
        assert!(!second_started.load(Ordering::Acquire));

        release.notify_one();
        first.await.unwrap();
        second.await.unwrap();
        assert!(!queue.is_active());
    }

    /// Two writes to *different* folders proceed concurrently (no serialisation).
    #[tokio::test]
    async fn run_for_folder_allows_concurrent_different_folders() {
        let queue = WriteQueue::default();
        let folder_a = Path::new("/album/a");
        let folder_b = Path::new("/album/b");

        let a_entered = Arc::new(Barrier::new(2));
        let a_release = Arc::new(Notify::new());
        let b_entered = Arc::new(Barrier::new(2));
        let b_release = Arc::new(Notify::new());

        // Start operation A in folder_a
        let q_a = queue.clone();
        let f_a = folder_a.to_path_buf();
        let ae = Arc::clone(&a_entered);
        let ar = Arc::clone(&a_release);
        let a = tokio::spawn(async move {
            q_a.run_for_folder(&f_a, async move {
                ae.wait().await;
                ar.notified().await;
            })
            .await;
        });

        // Start operation B in folder_b
        let q_b = queue.clone();
        let f_b = folder_b.to_path_buf();
        let be = Arc::clone(&b_entered);
        let br = Arc::clone(&b_release);
        let b = tokio::spawn(async move {
            q_b.run_for_folder(&f_b, async move {
                be.wait().await;
                br.notified().await;
            })
            .await;
        });

        // Wait for both to enter (they should both be able to start)
        a_entered.wait().await;
        b_entered.wait().await;
        assert!(queue.is_active());

        // Release both
        a_release.notify_one();
        b_release.notify_one();
        a.await.unwrap();
        b.await.unwrap();
        assert!(!queue.is_active());
    }

    /// `run_for_folder` is tracked via `is_active` (quit guard needs this).
    #[tokio::test]
    async fn run_for_folder_tracks_active() {
        let queue = WriteQueue::default();
        let folder = Path::new("/some/album");
        // Two-phase barrier: phase 1 = child entered the operation,
        // phase 2 = main thread has checked is_active, child can exit.
        let barrier = Arc::new(Barrier::new(2));

        let q = queue.clone();
        let f = folder.to_path_buf();
        let b = Arc::clone(&barrier);
        let handle = tokio::spawn(async move {
            q.run_for_folder(&f, async move {
                // Signal: I'm inside the operation (active should be true)
                b.wait().await;
                // Wait for main thread to check is_active
                b.wait().await;
            })
            .await;
        });
        // Wait for child to enter the operation
        barrier.wait().await;
        assert!(queue.is_active());
        // Signal child it can exit
        barrier.wait().await;

        handle.await.unwrap();
        assert!(!queue.is_active());
    }

    /// Same folder key reuses the same mutex (serialisation verified above).
    /// This test checks that the HashMap entry persists across calls.
    #[tokio::test]
    async fn run_for_folder_reuses_mutex() {
        let queue = WriteQueue::default();
        let folder = Path::new("/reuse/album");

        // First call creates the entry
        queue.run_for_folder(folder, async {}).await;

        // Second call uses the same entry — we just need it not to panic.
        queue.run_for_folder(folder, async {}).await;

        // Verify: the HashMap should have exactly one entry
        let gates = queue.0.folder_gates.lock().await;
        assert_eq!(gates.len(), 1);
        assert!(gates.contains_key(folder));
    }

    /// `run` (global) and `run_for_folder` do NOT block each other
    /// (different lock domains). This is intentional so single-file writes
    /// don't block folder-scoped batches and vice versa.
    #[tokio::test]
    async fn global_and_folder_are_independent() {
        let queue = WriteQueue::default();
        let folder = Path::new("/independent/album");

        let global_entered = Arc::new(Barrier::new(2));
        let global_release = Arc::new(Notify::new());

        // Start a global-lock operation that holds the gate
        let q = queue.clone();
        let ge = Arc::clone(&global_entered);
        let gr = Arc::clone(&global_release);
        let global = tokio::spawn(async move {
            q.run(async move {
                ge.wait().await;
                gr.notified().await;
            })
            .await;
        });
        global_entered.wait().await;

        // A folder-scoped operation should be able to start concurrently
        let folder_started = Arc::new(AtomicBool::new(false));
        let q2 = queue.clone();
        let f = folder.to_path_buf();
        let started = Arc::clone(&folder_started);
        let folder_op = tokio::spawn(async move {
            q2.run_for_folder(&f, async move {
                started.store(true, Ordering::Release);
            })
            .await;
        });
        tokio::task::yield_now().await;
        // Folder op should have started (not blocked by global gate)
        assert!(folder_started.load(Ordering::Acquire));

        global_release.notify_one();
        global.await.unwrap();
        folder_op.await.unwrap();
        assert!(!queue.is_active());
    }

    #[tokio::test]
    async fn exclusive_operation_blocks_global_and_folder_writes() {
        let queue = WriteQueue::default();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());
        let exclusive_queue = queue.clone();
        let exclusive_entered = Arc::clone(&entered);
        let exclusive_release = Arc::clone(&release);
        let exclusive = tokio::spawn(async move {
            exclusive_queue
                .run_exclusive(async move {
                    exclusive_entered.wait().await;
                    exclusive_release.notified().await;
                })
                .await;
        });
        entered.wait().await;

        let global_started = Arc::new(AtomicBool::new(false));
        let global_flag = Arc::clone(&global_started);
        let global_queue = queue.clone();
        let global = tokio::spawn(async move {
            global_queue
                .run(async move {
                    global_flag.store(true, Ordering::Release);
                })
                .await;
        });
        let folder_started = Arc::new(AtomicBool::new(false));
        let folder_flag = Arc::clone(&folder_started);
        let folder_queue = queue.clone();
        let folder = tokio::spawn(async move {
            folder_queue
                .run_for_folder(Path::new("/album"), async move {
                    folder_flag.store(true, Ordering::Release);
                })
                .await;
        });
        tokio::task::yield_now().await;
        assert!(!global_started.load(Ordering::Acquire));
        assert!(!folder_started.load(Ordering::Acquire));

        release.notify_one();
        exclusive.await.unwrap();
        global.await.unwrap();
        folder.await.unwrap();
        assert!(global_started.load(Ordering::Acquire));
        assert!(folder_started.load(Ordering::Acquire));
    }

    /// Intent: updater installation must fail immediately when a media write
    /// is already active; waiting and then replacing the application could
    /// make the user believe the protected operation was safely coordinated.
    #[tokio::test]
    async fn try_exclusive_rejects_an_already_active_write() {
        let queue = WriteQueue::default();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());
        let write_queue = queue.clone();
        let write_entered = Arc::clone(&entered);
        let write_release = Arc::clone(&release);
        let write = tokio::spawn(async move {
            write_queue
                .run(async move {
                    write_entered.wait().await;
                    write_release.notified().await;
                })
                .await;
        });
        entered.wait().await;

        let result = queue.try_run_exclusive(async { "installed" }).await;
        assert_eq!(result, None);

        release.notify_one();
        write.await.unwrap();
    }

    /// Intent: once updater installation claims coordination, it is itself a
    /// protected disk operation and later writes remain blocked until it ends.
    #[tokio::test]
    async fn try_exclusive_tracks_activity_and_blocks_new_writes() {
        let queue = WriteQueue::default();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());
        let install_queue = queue.clone();
        let install_entered = Arc::clone(&entered);
        let install_release = Arc::clone(&release);
        let install = tokio::spawn(async move {
            install_queue
                .try_run_exclusive(async move {
                    install_entered.wait().await;
                    install_release.notified().await;
                })
                .await
        });
        entered.wait().await;
        assert!(queue.is_active());

        let write_started = Arc::new(AtomicBool::new(false));
        let write_flag = Arc::clone(&write_started);
        let write_queue = queue.clone();
        let write = tokio::spawn(async move {
            write_queue
                .run(async move { write_flag.store(true, Ordering::Release) })
                .await;
        });
        tokio::task::yield_now().await;
        assert!(!write_started.load(Ordering::Acquire));

        release.notify_one();
        assert_eq!(install.await.unwrap(), Some(()));
        write.await.unwrap();
        assert!(write_started.load(Ordering::Acquire));
        assert!(!queue.is_active());
    }
}
