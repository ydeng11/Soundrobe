//! Prepaid write-boundary controls for the mock evaluator.
//!
//! The native receiver installs the account before bootstrap output. Remaining
//! evaluator/helper integration is required before this establishes a bounded run.

use rusqlite::{ffi, Connection, OpenFlags};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Deref;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Used {
    growth: u64,
    writes: u64,
}

const MIB: u64 = 1024 * 1024;
const LAUNCHER_RECEIPT: u64 = 64 * 1024;
const TERMINAL: Used = Used {
    growth: 2 * MIB,
    writes: 8 * MIB,
};
// Each issuance writes one 256-byte record and one 256-byte anchor. Record
// capacity also leaves a sentinel and disjoint shared-revocation tail. The
// entire backing is consumed before creation, never recursively journaled.
const ACCOUNT_BACKING: Used = Used {
    growth: 2 * MIB,
    writes: 4 * MIB,
};
const RECORD: u64 = 256;
const ACCOUNT_NAME: &str = ".storage-ledger";
// At most 255 generation-lifetime owners, including the creator. Each can
// attempt at most one failed transaction (512 bytes) and one stop byte. Claims
// never recycle; 255 * 513 fits the 128-KiB failure allocation inside the caps.
const MAX_OWNERS: u16 = 255;
const FAILED_BOOKKEEPING: u64 = 128 * 1024;
// The last 64 KiB is disjoint from all records and the empty sentinel. Its
// first byte is a pinned shared revocation latch, not a persistence dependency.
// Initial touching and the once-only 0->1 transition cost two prepaid bytes.
const REVOCATION_OFFSET: u64 = ACCOUNT_BACKING.growth - 64 * 1024;
const MAX_RECORDS: u64 = REVOCATION_OFFSET / RECORD - 2;
const _: () =
    assert!(RECORD + MAX_RECORDS * 2 * RECORD + FAILED_BOOKKEEPING + 2 <= ACCOUNT_BACKING.writes);

#[derive(Clone, Copy, PartialEq, Eq)]
enum AccountPoint {
    Initialize,
    TicketCommitted,
}
thread_local! {
    static ACCOUNT_PAUSE: std::cell::RefCell<Option<(AccountPoint, Arc<std::sync::Barrier>)>> = const { std::cell::RefCell::new(None) };
}
fn account_pause(point: AccountPoint) {
    ACCOUNT_PAUSE.with(|slot| {
        if let Some((selected, barrier)) = slot.borrow().as_ref() {
            if *selected == point {
                barrier.wait();
                barrier.wait();
            }
        }
    });
}

struct Revocation(*mut c_void);
// MAP_SHARED aliases one byte across owners. AtomicU8 is lock-free on the
// supported Unix targets; the mapping stays pinned and alive through Arc.
unsafe impl Send for Revocation {}
unsafe impl Sync for Revocation {}
impl Revocation {
    fn map(file: &File) -> io::Result<Self> {
        let pointer = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                1,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                REVOCATION_OFFSET as libc::off_t,
            )
        };
        if pointer == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::mlock(pointer, 1) } != 0 {
            let error = io::Error::last_os_error();
            unsafe {
                libc::munmap(pointer, 1);
            }
            return Err(error);
        }
        Ok(Self(pointer))
    }

    fn byte(&self) -> &AtomicU8 {
        // Fresh file extension supplies zero. No record, sentinel or durable
        // stop-byte write overlaps this byte. Invalid contents close admission.
        unsafe { &*self.0.cast::<AtomicU8>() }
    }

    fn closed(&self) -> bool {
        self.byte().load(Ordering::SeqCst) != 0
    }

    fn close(&self) -> bool {
        // Only the winning owner changes the byte. Failed comparisons perform
        // no store; repeated stops cannot renew or consume content authority.
        self.closed()
            || self
                .byte()
                .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
    }
}
impl Drop for Revocation {
    fn drop(&mut self) {
        unsafe {
            libc::munlock(self.0, 1);
            libc::munmap(self.0, 1);
        }
    }
}

fn account_latch(root: &Path, generation: uuid::Uuid) -> Arc<AtomicBool> {
    type Key = (PathBuf, uuid::Uuid);
    static LATCHES: OnceLock<Mutex<std::collections::HashMap<Key, Arc<AtomicBool>>>> =
        OnceLock::new();
    LATCHES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry((root.into(), generation))
        .or_default()
        .clone()
}

#[derive(Clone)]
struct Account {
    root: PathBuf,
    generation: uuid::Uuid,
    failed: Arc<AtomicBool>,
    owner_slot: u8,
    owner_pid: u32,
    revocation: Option<Arc<Revocation>>,
}

#[derive(Clone, Copy)]
struct AccountState {
    limits: Used,
    ordinary: Used,
    sequence: u64,
    terminal_issued: bool,
    stopped: bool,
}

struct ProcessStorage {
    account: Account,
    ordinary: Allowance,
    terminal: Option<Allowance>,
    terminal_claimed: AtomicBool,
}
static PROCESS_STORAGE: OnceLock<ProcessStorage> = OnceLock::new();

pub(crate) fn is_scoped() -> bool {
    PROCESS_STORAGE.get().is_some()
}

pub(crate) fn stop_unsupported() -> ! {
    PROCESS_STORAGE
        .get()
        .expect("scoped unsupported effect")
        .ordinary
        .stop();
    unreachable!("scoped storage stop must close native ownership")
}

fn notify_scoped_failure() {
    if let Some(process) = PROCESS_STORAGE.get() {
        process.account.revoke();
        // Closing process-owned liveness descriptors notifies the existing
        // independent native owners. Never wait for the failed ledger first.
        // This does not promise durable stop recording or a terminal receipt.
        unsafe { libc::_exit(78) }
    }
}

pub(crate) fn install_receiver(root: &Path, generation: uuid::Uuid) -> io::Result<()> {
    let account = Account::create(
        root,
        generation,
        Used {
            growth: 512 * MIB,
            writes: 1024 * MIB,
        },
    )?;
    // Mint before ordinary issuance, including receiver bootstrap, can begin.
    let terminal = account.terminal()?;
    let ordinary = account.allowance();
    PROCESS_STORAGE
        .set(ProcessStorage {
            account,
            ordinary,
            terminal: Some(terminal),
            terminal_claimed: AtomicBool::new(false),
        })
        .map_err(|_| io::Error::other("storage process already installed"))
}

pub(crate) fn attach_from_env() -> io::Result<()> {
    if std::env::var("SOUNDROBE_STORAGE_REQUIRED").as_deref() != Ok("1") {
        return Ok(());
    }
    let root = PathBuf::from(
        std::env::var_os("SOUNDROBE_NATIVE_OUTPUT")
            .ok_or_else(|| io::Error::other("storage root missing"))?,
    );
    let generation = std::env::var("SOUNDROBE_NATIVE_GENERATION")
        .ok()
        .and_then(|v| uuid::Uuid::parse_str(&v).ok())
        .ok_or_else(|| io::Error::other("storage generation missing"))?;
    let account = Account::open(&root, generation)?;
    account.healthy()?;
    let ordinary = account.allowance();
    PROCESS_STORAGE
        .set(ProcessStorage {
            account,
            ordinary,
            terminal: None,
            terminal_claimed: AtomicBool::new(false),
        })
        .map_err(|_| io::Error::other("storage process already installed"))
}

#[cfg(feature = "server")]
pub(crate) fn descriptor() -> serde_json::Value {
    let Some(process) = PROCESS_STORAGE.get() else {
        return serde_json::Value::Null;
    };
    serde_json::json!({"generation":process.account.generation.to_string(),
        "logicalLimit":512 * MIB,"applicationWriteLimit":1024 * MIB,
        "terminalGrowthReserved":TERMINAL.growth,"terminalWritesReserved":TERMINAL.writes,
        "selfGrowthConsumed":ACCOUNT_BACKING.growth,"selfWritesConsumed":ACCOUNT_BACKING.writes,
        "maximumOwners":MAX_OWNERS,"failedBookkeepingReserved":FAILED_BOOKKEEPING,
        "launcherTerminalPartition":LAUNCHER_RECEIPT,
        "coverage":"evaluation_files_sqlite_helpers_prepaid_not_physical_io"})
}

pub(crate) fn write_new(path: &Path, bytes: &[u8], terminal: bool) -> io::Result<bool> {
    let Some(process) = PROCESS_STORAGE.get() else {
        return Ok(false);
    };
    let allowance = if terminal {
        if process.terminal_claimed.swap(true, Ordering::SeqCst) {
            return Err(io::Error::other("terminal receipt already attempted"));
        }
        process
            .terminal
            .as_ref()
            .ok_or_else(|| io::Error::other("no terminal publication rights"))?
    } else {
        &process.ordinary
    };
    let mut file = BoundedFile::create(path, allowance)?;
    file.write_all(bytes)?;
    file.file.sync_all().inspect_err(|_| allowance.stop())?;
    Ok(true)
}

#[cfg(feature = "server")]
pub(crate) fn healthy() -> io::Result<()> {
    if let Some(process) = PROCESS_STORAGE.get() {
        process.account.healthy()?;
    }
    Ok(())
}

#[cfg(feature = "server")]
pub(crate) fn write_new_or_unscoped(
    path: impl AsRef<Path>,
    bytes: impl AsRef<[u8]>,
) -> io::Result<()> {
    if !write_new(path.as_ref(), bytes.as_ref(), false)? {
        std::fs::write(path, bytes)?;
    }
    Ok(())
}

// A fresh open file description is essential: sharing/duplicating an existing
// flock description would not serialize threads within the same process.
struct AccountLock(File);
impl Drop for AccountLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

impl Account {
    fn claim_owner(root: &Path) -> io::Result<(u8, u32)> {
        for slot in 0..MAX_OWNERS {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(root.join(format!(".storage-owner-{slot:03}")))
            {
                Ok(file) => {
                    // Zero-length claims consume no content bytes or logical
                    // growth. Metadata/flush traffic is not physical-I/O-free.
                    // A failed sync leaves the claim consumed, never retried.
                    file.sync_all()?;
                    File::open(root)?.sync_all()?;
                    return Ok((slot as u8, std::process::id()));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("storage owner capacity exhausted"))
    }

    fn encode(&self, state: AccountState, ticket: Used) -> [u8; 256] {
        use sha2::{Digest, Sha256};
        let mut bytes = [0u8; 256];
        bytes[0] = u8::from(state.stopped);
        bytes[1..17].copy_from_slice(self.generation.as_bytes());
        bytes[17..49].copy_from_slice(&Sha256::digest(self.root.as_os_str().as_bytes()));
        for (offset, n) in [
            (49, state.limits.growth),
            (57, state.limits.writes),
            (65, state.ordinary.growth),
            (73, state.ordinary.writes),
            (81, state.sequence),
            (91, ticket.growth),
            (99, ticket.writes),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&n.to_le_bytes());
        }
        bytes[89] = u8::from(state.terminal_issued);
        bytes[107..111].copy_from_slice(&std::process::id().to_le_bytes());
        bytes[111] = self.owner_slot;
        bytes[112..128].copy_from_slice(b"mock-storage-v1\0");
        let digest = Sha256::digest(&bytes[1..224]);
        bytes[224..].copy_from_slice(&digest);
        bytes
    }

    fn decode(&self, bytes: &[u8; 256]) -> io::Result<AccountState> {
        use sha2::{Digest, Sha256};
        let number = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        if bytes[1..17] != *self.generation.as_bytes()
            || bytes[17..49] != Sha256::digest(self.root.as_os_str().as_bytes())[..]
            || bytes[224..] != Sha256::digest(&bytes[1..224])[..]
            || &bytes[112..128] != b"mock-storage-v1\0"
        {
            return Err(io::Error::other("storage identity or recording unknown"));
        }
        Ok(AccountState {
            limits: Used {
                growth: number(49),
                writes: number(57),
            },
            ordinary: Used {
                growth: number(65),
                writes: number(73),
            },
            sequence: number(81),
            terminal_issued: bytes[89] != 0,
            stopped: bytes[0] != 0,
        })
    }

    fn create(root: &Path, generation: uuid::Uuid, limits: Used) -> io::Result<Self> {
        let ordinary = ACCOUNT_BACKING;
        if limits.growth < ordinary.growth + TERMINAL.growth
            || limits.writes < ordinary.writes + TERMINAL.writes
        {
            return Err(io::Error::other("storage bootstrap backing unavailable"));
        }
        let root = root.canonicalize()?;
        let failed = account_latch(&root, generation);
        if failed.load(Ordering::SeqCst) {
            return Err(io::Error::other("storage initialization previously failed"));
        }
        let (owner_slot, owner_pid) = Self::claim_owner(&root).inspect_err(|_| {
            failed.store(true, Ordering::SeqCst);
        })?;
        let mut account = Self {
            failed,
            root,
            generation,
            owner_slot,
            owner_pid,
            revocation: None,
        };
        (|| {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(account.root.join(ACCOUNT_NAME))?;
            // Both operations are backed by the finite, nonrefundable allocation
            // above. No application output exists or is admitted yet.
            file.set_len(ACCOUNT_BACKING.growth)?;
            Self::write_once(
                &file,
                &account.encode(
                    AccountState {
                        limits,
                        ordinary,
                        sequence: 0,
                        terminal_issued: false,
                        stopped: false,
                    },
                    ACCOUNT_BACKING,
                ),
                0,
            )?;
            file.sync_all()?;
            File::open(&account.root)?.sync_all()?;
            let revocation = Arc::new(Revocation::map(&file)?);
            account_pause(AccountPoint::Initialize);
            // The fresh extension is zero. Touch it writable before activation;
            // pinning failure rejects initialization, never falls back to I/O.
            revocation
                .byte()
                .compare_exchange(0, 0, Ordering::SeqCst, Ordering::SeqCst)
                .map_err(|_| io::Error::other("storage revoked during initialization"))?;
            account.revocation = Some(revocation);
            Ok::<(), io::Error>(())
        })()
        .inspect_err(|_| account.failed.store(true, Ordering::SeqCst))?;
        Ok(account)
    }

    fn open(root: &Path, generation: uuid::Uuid) -> io::Result<Self> {
        let root = root.canonicalize()?;
        let mut account = Self {
            failed: account_latch(&root, generation),
            root,
            generation,
            owner_slot: u8::MAX,
            owner_pid: std::process::id(),
            revocation: None,
        };
        if account.failed.load(Ordering::SeqCst) {
            return Err(io::Error::other("storage recording previously failed"));
        }
        // Before claiming, read only: an invalid/stopped generation must not
        // create another owner file or attempt unbacked stop publication.
        {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(account.root.join(ACCOUNT_NAME))?;
            let _lock = FileLock::exclusive(&file)?;
            if file.metadata()?.len() != ACCOUNT_BACKING.growth {
                return Err(io::Error::other("storage ledger length unknown"));
            }
            let state = account.state(&file)?;
            if state.stopped || !state.terminal_issued {
                return Err(io::Error::other(
                    "storage generation stopped or unactivated",
                ));
            }
            account.revocation = Some(Arc::new(Revocation::map(&file)?));
            if account.state(&file)?.stopped {
                return Err(io::Error::other("storage generation stopped"));
            }
        }
        let (slot, _) = Self::claim_owner(&account.root).inspect_err(|_| {
            account.failed.store(true, Ordering::SeqCst);
            account.revoke();
        })?;
        account.owner_slot = slot;
        account.healthy()?;
        Ok(account)
    }

    fn revoked(&self) -> bool {
        self.revocation.as_ref().is_some_and(|latch| latch.closed())
    }

    fn revoke(&self) -> bool {
        self.revocation.as_ref().is_some_and(|latch| latch.close())
    }

    fn lock(&self) -> io::Result<AccountLock> {
        if self.failed.load(Ordering::SeqCst) {
            return Err(io::Error::other("storage recording previously failed"));
        }
        let lock = self.lock_unchecked().inspect_err(|_| {
            self.failed.store(true, Ordering::SeqCst);
            self.revoke();
        })?;
        if self.failed.load(Ordering::SeqCst) {
            return Err(io::Error::other("storage recording failed while waiting"));
        }
        Ok(lock)
    }

    fn lock_unchecked(&self) -> io::Result<AccountLock> {
        if self.owner_pid != std::process::id() || self.owner_slot == u8::MAX {
            return Err(io::Error::other(
                "forked storage owner must register separately",
            ));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.root.join(ACCOUNT_NAME))?;
        if file.metadata()?.len() != ACCOUNT_BACKING.growth {
            return Err(io::Error::other("storage ledger length unknown"));
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(AccountLock(file))
    }

    fn state(&self, file: &File) -> io::Result<AccountState> {
        let mut bytes = [0; 256];
        file.read_exact_at(&mut bytes, 0)?;
        let mut state = self.decode(&bytes)?;
        if state.sequence > MAX_RECORDS {
            return Err(io::Error::other("storage bookkeeping exhausted"));
        }
        if state.sequence != 0 {
            let mut current = [0; 256];
            file.read_exact_at(&mut current, state.sequence * RECORD)?;
            self.decode(&current)?;
            if bytes[1..] != current[1..] {
                return Err(io::Error::other(
                    "storage issuance record missing or mismatched",
                ));
            }
        }
        // A durable record without its matching anchor is an ambiguous grant.
        // Never replay/overwrite it or recover an executable run from it.
        let mut next = [0; 256];
        file.read_exact_at(&mut next, (state.sequence + 1) * RECORD)?;
        if next != [0; 256] {
            return Err(io::Error::other("storage grant recording unknown"));
        }
        state.stopped |= self.revoked();
        Ok(state)
    }

    fn snapshot(&self) -> io::Result<AccountState> {
        let lock = self.lock()?;
        self.checked_state(&lock.0)
    }

    fn fail_locked(&self, file: &File) {
        if !self.failed.swap(true, Ordering::SeqCst) {
            self.revoke();
            notify_scoped_failure();
            let _ = Self::stop_locked(file);
        }
    }

    fn checked_state(&self, file: &File) -> io::Result<AccountState> {
        if self.failed.load(Ordering::SeqCst) {
            return Err(io::Error::other("storage recording previously failed"));
        }
        self.state(file).inspect_err(|_| self.fail_locked(file))
    }

    fn write_once(file: &File, bytes: &[u8], offset: u64) -> io::Result<()> {
        // No hidden partial-write retry may spend unbacked ledger bytes.
        if file.write_at(bytes, offset)? != bytes.len() {
            return Err(io::Error::other("storage ledger short write"));
        }
        Ok(())
    }

    fn stop_locked(file: &File) -> io::Result<()> {
        if std::env::var("SOUNDROBE_STORAGE_STOP_DELAY").as_deref() == Ok("1") {
            std::thread::sleep(std::time::Duration::from_millis(750));
        }
        let mut stopped = [0];
        file.read_exact_at(&mut stopped, 0)?;
        if stopped[0] == 0 {
            // At most one stop-byte write per account, already backed above.
            Self::write_once(file, &[1], 0)?;
            file.sync_all()?;
        }
        Ok(())
    }

    fn stop(&self) -> io::Result<()> {
        if self.revoke() {
            return Ok(());
        }
        notify_scoped_failure();
        let lock = self.lock()?;
        self.publish_stop(&lock.0)
    }

    fn publish_stop(&self, file: &File) -> io::Result<()> {
        self.revoke();
        // In a managed owner, descriptor loss must reach containment before
        // stop persistence can stall. The native owner never resumes this run;
        // lack of a durable stop byte is not recoverable admission authority.
        notify_scoped_failure();
        Self::stop_locked(file).inspect_err(|_| {
            self.failed.store(true, Ordering::SeqCst);
            notify_scoped_failure();
        })
    }

    fn healthy(&self) -> io::Result<()> {
        if self.revoked() {
            return Err(io::Error::other("storage admission revoked"));
        }
        let lock = self.lock()?;
        let state = self.checked_state(&lock.0)?;
        if state.stopped {
            return Err(io::Error::other("storage stopped"));
        }
        Ok(())
    }

    fn record(&self, file: &File, mut state: AccountState, ticket: Used) -> io::Result<()> {
        let available = MAX_RECORDS - u64::from(!state.terminal_issued);
        if state.sequence + 1 > available {
            self.publish_stop(file)?;
            return Err(io::Error::other("storage bookkeeping exhausted"));
        }
        state.sequence += 1;
        let bytes = self.encode(state, ticket);
        (|| {
            Self::write_once(file, &bytes, state.sequence * RECORD)?;
            file.sync_all()?;
            Self::write_once(file, &bytes, 0)?;
            file.sync_all()
        })()
        .inspect_err(|_| self.fail_locked(file))
    }

    fn reserve(&self, ticket: Used) -> io::Result<()> {
        self.issue(ticket, false).map(|_| ())
    }

    fn issue(&self, mut ticket: Used, prepaid: bool) -> io::Result<Used> {
        if ticket == Used::default() {
            self.healthy()?;
            return Ok(ticket);
        }
        let lock = self.lock()?;
        let mut state = self.checked_state(&lock.0)?;
        if prepaid {
            // Bounded prepayment amortizes issuance without granting each child
            // a new cap. Unused tails remain consumed by this same account.
            let window = 256 * 1024;
            if ticket.growth != 0 {
                ticket.growth = ticket
                    .growth
                    .max(window)
                    .min(
                        state
                            .limits
                            .growth
                            .saturating_sub(TERMINAL.growth + state.ordinary.growth),
                    )
                    .max(ticket.growth);
            }
            if ticket.writes != 0 {
                ticket.writes = ticket
                    .writes
                    .max(window)
                    .min(
                        state
                            .limits
                            .writes
                            .saturating_sub(TERMINAL.writes + state.ordinary.writes),
                    )
                    .max(ticket.writes);
            }
        }
        let next = state
            .ordinary
            .growth
            .checked_add(ticket.growth)
            .zip(state.ordinary.writes.checked_add(ticket.writes));
        if !state.stopped {
            if let Some((growth, writes)) = next {
                if growth <= state.limits.growth - TERMINAL.growth
                    && writes <= state.limits.writes - TERMINAL.writes
                {
                    state.ordinary = Used { growth, writes };
                    self.record(&lock.0, state, ticket)?;
                    account_pause(AccountPoint::TicketCommitted);
                    // The final shared load is grant admission's linearization
                    // point. A revoked durable debit is retained, never refunded.
                    if self.revoked() {
                        return Err(io::Error::other("storage revoked during issuance"));
                    }
                    return Ok(ticket);
                }
            }
        }
        // Close peers before exposing the denial decision to an observer.
        self.revoke();
        if !state.stopped
            && std::env::var("SOUNDROBE_STORAGE_ACTION").as_deref() == Ok("quota_control")
        {
            // Control evidence goes once to the observer's pipe, never to a
            // run-owned file or the failed ledger. Missing delivery fails the
            // control; it must not delay native containment.
            let mut event = b"\nSOUNDROBE_STORAGE_DENIAL ".to_vec();
            event.extend(serde_json::to_vec(&serde_json::json!({
                "generation":self.generation.to_string(), "target":std::env::var("SOUNDROBE_STORAGE_TARGET").unwrap(),
                "requestedGrowth":ticket.growth, "requestedWrites":ticket.writes,
                "ordinaryWriteLimit":state.limits.writes-TERMINAL.writes,
                "ordinaryWritesReserved":state.ordinary.writes
            })).unwrap());
            event.push(b'\n');
            unsafe {
                let flags = libc::fcntl(libc::STDOUT_FILENO, libc::F_GETFL);
                if event.len() <= 4096
                    && flags >= 0
                    && libc::fcntl(libc::STDOUT_FILENO, libc::F_SETFL, flags | libc::O_NONBLOCK)
                        == 0
                {
                    libc::write(libc::STDOUT_FILENO, event.as_ptr().cast(), event.len());
                }
            }
        }
        self.publish_stop(&lock.0)?;
        Err(io::Error::other("storage cap stopped admission"))
    }

    fn terminal(&self) -> io::Result<Allowance> {
        let lock = self.lock()?;
        let mut state = self.checked_state(&lock.0)?;
        if state.terminal_issued {
            return Err(io::Error::other("terminal capacity already issued"));
        }
        state.terminal_issued = true;
        self.record(&lock.0, state, TERMINAL)?;
        Ok(Allowance::new(
            &self.root,
            TERMINAL.growth - LAUNCHER_RECEIPT,
            TERMINAL.writes - LAUNCHER_RECEIPT,
        ))
    }

    fn allowance(&self) -> Allowance {
        let mut allowance = Allowance::new(&self.root, u64::MAX, u64::MAX);
        allowance.account = Some(self.clone());
        allowance
    }
}

#[derive(Clone)]
struct Allowance {
    root: PathBuf,
    limits: Used,
    used: Arc<Mutex<Used>>,
    prepaid: Arc<Mutex<Used>>,
    stopped: Arc<AtomicBool>,
    account: Option<Account>,
}

impl Allowance {
    fn new(root: &Path, growth: u64, writes: u64) -> Self {
        Self {
            root: root.canonicalize().unwrap(),
            limits: Used { growth, writes },
            used: Arc::new(Mutex::new(Used::default())),
            prepaid: Arc::new(Mutex::new(Used::default())),
            stopped: Arc::new(AtomicBool::new(false)),
            account: None,
        }
    }

    fn used(&self) -> Used {
        *self.used.lock().unwrap()
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        notify_scoped_failure();
        if let Some(account) = &self.account {
            let _ = account.stop();
        }
    }

    fn check_path(&self, path: &Path) -> io::Result<()> {
        self.charge(0, 0)?;
        (|| {
            let parent = path
                .parent()
                .ok_or_else(|| io::Error::other("storage path"))?;
            let parent = parent.canonicalize()?;
            if !path.is_absolute() || !parent.starts_with(&self.root) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "storage root",
                ));
            }
            if parent == self.root
                && path.file_name().is_some_and(|name| {
                    let name = name.as_bytes();
                    name == ACCOUNT_NAME.as_bytes()
                        || name.starts_with(b".storage-owner-")
                        || name == b".storage-launcher-claim"
                })
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "storage control plane",
                ));
            }
            Ok(())
        })()
        .inspect_err(|_| self.stop())
    }

    fn charge(&self, growth: u64, writes: u64) -> io::Result<()> {
        let mut used = self
            .used
            .lock()
            .map_err(|_| io::Error::other("storage poisoned"))?;
        let next = used
            .growth
            .checked_add(growth)
            .zip(used.writes.checked_add(writes));
        if let Some((growth, writes)) = next {
            if !self.stopped.load(Ordering::SeqCst)
                && growth <= self.limits.growth
                && writes <= self.limits.writes
            {
                if let Some(account) = &self.account {
                    account.healthy().inspect_err(|_| self.stop())?;
                    let mut prepaid = self
                        .prepaid
                        .lock()
                        .map_err(|_| io::Error::other("storage prepaid poisoned"))?;
                    let missing = Used {
                        growth: growth.saturating_sub(prepaid.growth),
                        writes: writes.saturating_sub(prepaid.writes),
                    };
                    if missing != Used::default() {
                        let ticket = account.issue(missing, true).inspect_err(|_| self.stop())?;
                        prepaid.growth += ticket.growth;
                        prepaid.writes += ticket.writes;
                    }
                }
                *used = Used { growth, writes };
                return Ok(());
            }
        }
        self.stop();
        Err(io::Error::other("storage allowance stopped"))
    }
}

struct BoundedFile {
    file: File,
    allowance: Allowance,
    append: bool,
}

impl BoundedFile {
    fn create(path: &Path, allowance: &Allowance) -> io::Result<Self> {
        allowance.check_path(path)?;
        Ok(Self {
            file: OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)
                .inspect_err(|_| allowance.stop())?,
            allowance: allowance.clone(),
            append: false,
        })
    }

    fn set_len(&mut self, len: u64) -> io::Result<()> {
        (|| {
            let _lock = FileLock::exclusive(&self.file)?;
            self.allowance
                .charge(len.saturating_sub(self.file.metadata()?.len()), 0)?;
            self.file.set_len(len)
        })()
        .inspect_err(|_| self.allowance.stop())
    }
}

impl Write for BoundedFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        (|| {
            let _lock = FileLock::exclusive(&self.file)?;
            let length = self.file.metadata()?.len();
            let position = if self.append {
                length
            } else {
                self.file.stream_position()?
            };
            let end = position
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| io::Error::other("storage offset overflow"))?;
            self.allowance
                .charge(end.saturating_sub(length), bytes.len() as u64)?;
            self.file.write(bytes)
        })()
        .inspect_err(|_| self.allowance.stop())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush().inspect_err(|_| self.allowance.stop())
    }
}

impl Read for BoundedFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.file.read(bytes).inspect_err(|_| self.allowance.stop())
    }
}

impl Seek for BoundedFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file
            .seek(position)
            .inspect_err(|_| self.allowance.stop())
    }
}

// Exclusive operations also coordinate independently opened writable handles.
// The descriptor remains owned by the caller until this lexical guard drops.
struct FileLock(c_int);
impl FileLock {
    fn exclusive(file: &File) -> io::Result<Self> {
        let fd = file.as_raw_fd();
        if unsafe { libc::flock(fd, libc::LOCK_EX) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(fd))
    }
}
impl Drop for FileLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0, libc::LOCK_UN);
        }
    }
}

/// Existing filesystem owners select this facade only in test/unix builds.
/// Without an explicitly installed evaluation scope it delegates unchanged.
pub(crate) mod files {
    use super::{Allowance, BoundedFile, PROCESS_STORAGE};
    pub(crate) use std::fs::Metadata;
    use std::io::{self, Read, Seek, SeekFrom, Write};
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;

    fn checked<T>(result: io::Result<T>) -> io::Result<T> {
        result.inspect_err(|error| {
            if !matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::Interrupted
            ) {
                if let Some(process) = PROCESS_STORAGE.get() {
                    process.ordinary.stop();
                }
            }
        })
    }
    pub(crate) fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        checked(std::fs::read(path))
    }
    pub(crate) fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
        checked(std::fs::read_to_string(path))
    }
    pub(crate) fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
        checked(std::fs::metadata(path))
    }
    pub(crate) fn canonicalize(path: impl AsRef<Path>) -> io::Result<std::path::PathBuf> {
        checked(std::fs::canonicalize(path))
    }
    pub(crate) fn read_dir(
        path: impl AsRef<Path>,
    ) -> io::Result<impl Iterator<Item = io::Result<std::fs::DirEntry>>> {
        Ok(checked(std::fs::read_dir(path))?.map(checked))
    }

    enum Handle {
        Ordinary(std::fs::File),
        Scoped(BoundedFile),
    }
    pub(crate) struct File {
        inner: Handle,
    }

    impl File {
        fn raw(&self) -> &std::fs::File {
            match &self.inner {
                Handle::Ordinary(file) => file,
                Handle::Scoped(file) => &file.file,
            }
        }
        pub(crate) fn open(path: impl AsRef<Path>) -> io::Result<Self> {
            Ok(Self {
                inner: Handle::Ordinary(checked(std::fs::File::open(path))?),
            })
        }
        pub(crate) fn create(path: impl AsRef<Path>) -> io::Result<Self> {
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
        }
        pub(crate) fn metadata(&self) -> io::Result<Metadata> {
            checked(self.raw().metadata())
        }
        pub(crate) fn sync_all(&self) -> io::Result<()> {
            checked(self.raw().sync_all().inspect_err(|_| {
                if let Handle::Scoped(file) = &self.inner {
                    file.allowance.stop();
                }
            }))
        }
        pub(crate) fn set_len(&mut self, len: u64) -> io::Result<()> {
            match &mut self.inner {
                Handle::Ordinary(file) => checked(file.set_len(len)),
                Handle::Scoped(file) => file.set_len(len),
            }
        }
    }
    impl Read for File {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            match &mut self.inner {
                Handle::Ordinary(file) => checked(file.read(bytes)),
                Handle::Scoped(file) => file.read(bytes),
            }
        }
    }
    impl Seek for File {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            match &mut self.inner {
                Handle::Ordinary(file) => checked(file.seek(position)),
                Handle::Scoped(file) => file.seek(position),
            }
        }
    }
    impl Write for File {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            match &mut self.inner {
                Handle::Ordinary(file) => checked(file.write(bytes)),
                Handle::Scoped(file) => file.write(bytes),
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            match &mut self.inner {
                Handle::Ordinary(file) => checked(file.flush()),
                Handle::Scoped(file) => file.flush(),
            }
        }
    }
    impl lofty::io::Truncate for File {
        type Error = io::Error;
        fn truncate(&mut self, len: u64) -> io::Result<()> {
            self.set_len(len)
        }
    }
    impl lofty::io::Length for File {
        type Error = io::Error;
        fn len(&self) -> io::Result<u64> {
            Ok(self.metadata()?.len())
        }
    }

    #[derive(Clone)]
    pub(crate) struct OpenOptions {
        inner: std::fs::OpenOptions,
        writable: bool,
        append: bool,
        truncate: bool,
    }
    impl OpenOptions {
        pub(crate) fn new() -> Self {
            Self {
                inner: std::fs::OpenOptions::new(),
                writable: false,
                append: false,
                truncate: false,
            }
        }
        pub(crate) fn read(&mut self, value: bool) -> &mut Self {
            self.inner.read(value);
            self
        }
        pub(crate) fn write(&mut self, value: bool) -> &mut Self {
            self.inner.write(value);
            self.writable = value;
            self
        }
        pub(crate) fn append(&mut self, value: bool) -> &mut Self {
            self.inner.append(value);
            self.append = value;
            self
        }
        pub(crate) fn create(&mut self, value: bool) -> &mut Self {
            self.inner.create(value);
            self
        }
        pub(crate) fn create_new(&mut self, value: bool) -> &mut Self {
            self.inner.create_new(value);
            self
        }
        pub(crate) fn truncate(&mut self, value: bool) -> &mut Self {
            self.truncate = value;
            self
        }
        pub(crate) fn open(&self, path: impl AsRef<Path>) -> io::Result<File> {
            let path = path.as_ref();
            let Some(process) = PROCESS_STORAGE
                .get()
                .filter(|_| self.writable || self.append)
            else {
                return Ok(File {
                    inner: Handle::Ordinary(checked(
                        self.inner.clone().truncate(self.truncate).open(path),
                    )?),
                });
            };
            let allowance = &process.ordinary;
            allowance.check_path(path)?;
            let raw = self
                .inner
                .clone()
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)
                .inspect_err(|_| allowance.stop())?;
            let mut file = BoundedFile {
                file: raw,
                allowance: allowance.clone(),
                append: self.append,
            };
            if self.truncate {
                file.set_len(0)?;
            }
            Ok(File {
                inner: Handle::Scoped(file),
            })
        }
    }

    fn check(path: &Path) -> io::Result<Option<&'static Allowance>> {
        let Some(process) = PROCESS_STORAGE.get() else {
            return Ok(None);
        };
        process.ordinary.check_path(path)?;
        Ok(Some(&process.ordinary))
    }
    pub(crate) fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
        File::create(path)?.write_all(bytes.as_ref())
    }
    pub(crate) fn copy(source: impl AsRef<Path>, destination: impl AsRef<Path>) -> io::Result<u64> {
        if PROCESS_STORAGE.get().is_none() {
            return std::fs::copy(source, destination);
        }
        let mut source = File::open(source)?;
        let mut destination = File::create(destination)?;
        io::copy(&mut source, &mut destination)
    }
    pub(crate) fn rename(
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> io::Result<()> {
        let allowance = check(source.as_ref())?;
        check(destination.as_ref())?;
        std::fs::rename(source, destination).inspect_err(|_| {
            if let Some(a) = allowance {
                a.stop();
            }
        })
    }
    pub(crate) fn publish_new(source: &Path, destination: &Path) -> io::Result<()> {
        let allowance = check(source)?;
        check(destination)?;
        if let Some(allowance) = allowance {
            // Both names coexist until unlink; conservatively charge the
            // second logical length even though the inode is shared briefly.
            allowance.charge(metadata(source)?.len(), 0)?;
        }
        std::fs::hard_link(source, destination).inspect_err(|_| {
            if let Some(allowance) = allowance {
                allowance.stop();
            }
        })?;
        remove_file(source)
    }
    pub(crate) fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
        check(path.as_ref())?;
        checked(std::fs::remove_file(path))
    }
    pub(crate) fn remove_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
        if let Some(allowance) = check(path.as_ref())? {
            if canonicalize(path.as_ref())? == allowance.root {
                allowance.stop();
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "storage account root",
                ));
            }
        }
        checked(std::fs::remove_dir_all(path))
    }
    pub(crate) fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        (|| {
            if let Some(process) = PROCESS_STORAGE.get() {
                let ancestor = path
                    .ancestors()
                    .find(|p| metadata(p).is_ok())
                    .ok_or_else(|| io::Error::other("storage directory root missing"))?;
                let tail = path.strip_prefix(ancestor).map_err(io::Error::other)?;
                let ancestor = canonicalize(ancestor)?;
                if !path.is_absolute()
                    || !ancestor.starts_with(&process.ordinary.root)
                    || tail
                        .components()
                        .any(|part| !matches!(part, std::path::Component::Normal(_)))
                {
                    return Err(io::Error::other("storage directory outside owned root"));
                }
                // Inspect the first owned component even when later parents do
                // not exist. The ordinary path guard protects all control names.
                let destination = ancestor.join(tail);
                if let Some(first) = destination
                    .strip_prefix(&process.ordinary.root)
                    .map_err(io::Error::other)?
                    .components()
                    .next()
                {
                    process
                        .ordinary
                        .check_path(&process.ordinary.root.join(first.as_os_str()))?;
                } else {
                    process.ordinary.charge(0, 0)?;
                }
            }
            std::fs::create_dir_all(path)
        })()
        .inspect_err(|_| {
            if let Some(process) = PROCESS_STORAGE.get() {
                process.ordinary.stop();
            }
        })
    }
    pub(crate) fn create_dir(path: impl AsRef<Path>) -> io::Result<()> {
        check(path.as_ref())?;
        std::fs::create_dir(path).inspect_err(|_| {
            if let Some(process) = PROCESS_STORAGE.get() {
                process.ordinary.stop();
            }
        })
    }
}

// The raw VFS remains at a stable address while any scoped connection exists.
// It is explicitly selected by name, never installed as SQLite's default.
#[repr(C)]
struct Vfs {
    raw: ffi::sqlite3_vfs,
    parent: *mut ffi::sqlite3_vfs,
    allowance: Allowance,
}

struct ScopedSqlite {
    vfs: Box<Vfs>,
    name: CString,
}

struct ScopedConnection<'a> {
    connection: Connection,
    _scope: &'a ScopedSqlite,
}

impl Deref for ScopedConnection<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.connection
    }
}

impl ScopedSqlite {
    fn new(allowance: &Allowance) -> Self {
        // Select POSIX locking explicitly: macOS's auto-locking unix VFS can
        // choose proxy/AFP/dotfile writers not covered by these callbacks.
        unsafe {
            assert_eq!(ffi::sqlite3_initialize(), ffi::SQLITE_OK);
            #[cfg(target_os = "macos")]
            let parent_name = c"unix-posix";
            #[cfg(not(target_os = "macos"))]
            let parent_name = c"unix";
            let parent = ffi::sqlite3_vfs_find(parent_name.as_ptr());
            assert!(!parent.is_null());
            assert_eq!(CStr::from_ptr((*parent).zName), parent_name);
            let name = CString::new(format!("mock-storage-{}", uuid::Uuid::new_v4())).unwrap();
            let mut vfs = Box::new(Vfs {
                raw: *parent,
                parent,
                allowance: allowance.clone(),
            });
            vfs.raw.zName = name.as_ptr();
            vfs.raw.pNext = std::ptr::null_mut();
            vfs.raw.xOpen = Some(vfs_open);
            vfs.raw.xDelete = Some(vfs_delete);
            vfs.raw.xAccess = Some(vfs_access);
            vfs.raw.xFullPathname = Some(vfs_fullpath);
            assert_eq!(ffi::sqlite3_vfs_register(&mut vfs.raw, 0), ffi::SQLITE_OK);
            Self { vfs, name }
        }
    }

    fn open(&self, path: &Path) -> rusqlite::Result<ScopedConnection<'_>> {
        Ok(ScopedConnection {
            connection: self.open_owned(path, OpenFlags::default())?,
            _scope: self,
        })
    }

    fn open_owned(&self, path: &Path, flags: OpenFlags) -> rusqlite::Result<Connection> {
        if self.vfs.allowance.check_path(path).is_err() {
            return Err(rusqlite::Error::InvalidPath(path.into()));
        }
        let connection =
            Connection::open_with_flags_and_vfs(path, flags, self.name.to_str().unwrap())
                .inspect_err(|_| self.vfs.allowance.stop())?;
        let rc = unsafe {
            ffi::sqlite3_set_authorizer(
                connection.handle(),
                Some(sqlite_authorize),
                std::ptr::from_ref(&*self.vfs).cast_mut().cast(),
            )
        };
        if rc != ffi::SQLITE_OK {
            self.vfs.allowance.stop();
            return Err(rusqlite::Error::SqliteFailure(ffi::Error::new(rc), None));
        }
        Ok(connection)
    }
}

pub(crate) fn open_sqlite(path: &Path, flags: OpenFlags) -> rusqlite::Result<Connection> {
    let Some(process) = PROCESS_STORAGE.get() else {
        return Connection::open_with_flags(path, flags);
    };
    static NAME: OnceLock<String> = OnceLock::new();
    let name = NAME.get_or_init(|| {
        // One named VFS per explicitly scoped test process. Process-lifetime
        // retention lets ordinary store owners keep Connection without a raw
        // pointer outliving its scope; this never changes the default VFS.
        let scope = Box::leak(Box::new(ScopedSqlite::new(&process.ordinary)));
        scope.name.to_str().unwrap().to_owned()
    });
    if process.ordinary.check_path(path).is_err() {
        return Err(rusqlite::Error::InvalidPath(path.into()));
    }
    let connection = Connection::open_with_flags_and_vfs(path, flags, name.as_str())
        .inspect_err(|_| process.ordinary.stop())?;
    unsafe {
        let name = CString::new(name.as_str()).unwrap();
        let vfs = ffi::sqlite3_vfs_find(name.as_ptr());
        let rc =
            ffi::sqlite3_set_authorizer(connection.handle(), Some(sqlite_authorize), vfs.cast());
        if rc != ffi::SQLITE_OK {
            process.ordinary.stop();
            return Err(rusqlite::Error::SqliteFailure(ffi::Error::new(rc), None));
        }
    }
    Ok(connection)
}

unsafe extern "C" fn sqlite_authorize(
    context: *mut c_void,
    action: c_int,
    first: *const c_char,
    _second: *const c_char,
    _db: *const c_char,
    _trigger: *const c_char,
) -> c_int {
    // ATTACH also covers VACUUM INTO. SQLite can honor a URI's vfs= before any
    // xOpen callback, so these outputs must be denied at SQL authorization.
    let directory_pragma = action == ffi::SQLITE_PRAGMA && !first.is_null() && {
        let name = CStr::from_ptr(first).to_bytes();
        name.eq_ignore_ascii_case(b"temp_store_directory")
            || name.eq_ignore_ascii_case(b"data_store_directory")
    };
    if action == ffi::SQLITE_ATTACH || directory_pragma {
        (*context.cast::<Vfs>()).allowance.stop();
        ffi::SQLITE_DENY
    } else {
        ffi::SQLITE_OK
    }
}

impl Drop for ScopedSqlite {
    fn drop(&mut self) {
        // ScopedConnection's borrow prevents unregistering before xClose.
        unsafe {
            ffi::sqlite3_vfs_unregister(&mut self.vfs.raw);
        }
    }
}

// pMethods points at the first field of this box. The underlying unixFile stays
// in SQLite's original allocation, so unchanged callbacks receive its real ABI.
#[repr(C)]
struct SqliteFile {
    methods: ffi::sqlite3_io_methods,
    original: *const ffi::sqlite3_io_methods,
    allowance: Allowance,
}

unsafe fn sqlite_file<'a>(file: *mut ffi::sqlite3_file) -> &'a SqliteFile {
    &*((*file).pMethods.cast::<SqliteFile>())
}

unsafe fn path_from_sqlite<'a>(name: *const c_char) -> &'a Path {
    Path::new(std::ffi::OsStr::from_bytes(CStr::from_ptr(name).to_bytes()))
}

unsafe extern "C" fn vfs_access(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    flags: c_int,
    out: *mut c_int,
) -> c_int {
    let context = &*vfs.cast::<Vfs>();
    storage_result(
        &context.allowance,
        ((*context.parent).xAccess.unwrap())(context.parent, name, flags, out),
    )
}

unsafe extern "C" fn vfs_fullpath(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    size: c_int,
    out: *mut c_char,
) -> c_int {
    let context = &*vfs.cast::<Vfs>();
    storage_result(
        &context.allowance,
        ((*context.parent).xFullPathname.unwrap())(context.parent, name, size, out),
    )
}

unsafe extern "C" fn vfs_open(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out: *mut c_int,
) -> c_int {
    let context = &*vfs.cast::<Vfs>();
    (*file).pMethods = std::ptr::null();
    // An unnamed temporary file has no declared owned path. Never let SQLite
    // choose a system temporary directory behind the accounting boundary.
    if name.is_null()
        || context
            .allowance
            .check_path(path_from_sqlite(name))
            .is_err()
    {
        context.allowance.stop();
        return ffi::SQLITE_CANTOPEN;
    }
    let rc = ((*context.parent).xOpen.unwrap())(context.parent, name, file, flags, out);
    if rc != ffi::SQLITE_OK {
        context.allowance.stop();
        return rc;
    }
    let original = (*file).pMethods;
    let mut state = Box::new(SqliteFile {
        methods: *original,
        original,
        allowance: context.allowance.clone(),
    });
    state.methods.iVersion = state.methods.iVersion.min(2);
    state.methods.xClose = Some(sqlite_close);
    state.methods.xRead = Some(sqlite_read);
    state.methods.xFileSize = Some(sqlite_size);
    state.methods.xLock = Some(sqlite_lock);
    state.methods.xUnlock = Some(sqlite_unlock);
    state.methods.xCheckReservedLock = Some(sqlite_reserved);
    state.methods.xWrite = Some(sqlite_write);
    state.methods.xTruncate = Some(sqlite_truncate);
    state.methods.xSync = Some(sqlite_sync);
    state.methods.xFileControl = Some(sqlite_control);
    if state.methods.xShmMap.is_some() {
        state.methods.xShmMap = Some(sqlite_shm_map);
    }
    if state.methods.xShmLock.is_some() {
        state.methods.xShmLock = Some(sqlite_shm_lock);
    }
    if state.methods.xShmUnmap.is_some() {
        state.methods.xShmUnmap = Some(sqlite_shm_unmap);
    }
    state.methods.xFetch = None;
    state.methods.xUnfetch = None;
    (*file).pMethods = Box::into_raw(state).cast::<ffi::sqlite3_io_methods>();
    ffi::SQLITE_OK
}

unsafe extern "C" fn vfs_delete(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    sync: c_int,
) -> c_int {
    let context = &*vfs.cast::<Vfs>();
    if name.is_null()
        || context
            .allowance
            .check_path(path_from_sqlite(name))
            .is_err()
    {
        context.allowance.stop();
        return ffi::SQLITE_IOERR_DELETE;
    }
    // Deletion releases no prepaid allowance.
    storage_result(
        &context.allowance,
        ((*context.parent).xDelete.unwrap())(context.parent, name, sync),
    )
}

fn storage_result(allowance: &Allowance, rc: c_int) -> c_int {
    // Contention and unsupported controls are normal SQLite protocol results.
    if !matches!(
        rc & 0xff,
        ffi::SQLITE_OK | ffi::SQLITE_BUSY | ffi::SQLITE_LOCKED | ffi::SQLITE_NOTFOUND
    ) {
        allowance.stop();
    }
    rc
}

unsafe extern "C" fn sqlite_close(file: *mut ffi::sqlite3_file) -> c_int {
    let state = Box::from_raw((*file).pMethods as *mut SqliteFile);
    (*file).pMethods = state.original;
    storage_result(&state.allowance, ((*state.original).xClose.unwrap())(file))
}

unsafe extern "C" fn sqlite_read(
    file: *mut ffi::sqlite3_file,
    out: *mut c_void,
    len: c_int,
    offset: i64,
) -> c_int {
    let state = sqlite_file(file);
    let rc = ((*state.original).xRead.unwrap())(file, out, len, offset);
    // Empty and partially populated databases legitimately return short reads.
    if rc == ffi::SQLITE_IOERR_SHORT_READ {
        rc
    } else {
        storage_result(&state.allowance, rc)
    }
}

unsafe extern "C" fn sqlite_size(file: *mut ffi::sqlite3_file, out: *mut i64) -> c_int {
    let state = sqlite_file(file);
    storage_result(
        &state.allowance,
        ((*state.original).xFileSize.unwrap())(file, out),
    )
}

unsafe extern "C" fn sqlite_lock(file: *mut ffi::sqlite3_file, lock: c_int) -> c_int {
    let state = sqlite_file(file);
    storage_result(
        &state.allowance,
        ((*state.original).xLock.unwrap())(file, lock),
    )
}

unsafe extern "C" fn sqlite_unlock(file: *mut ffi::sqlite3_file, lock: c_int) -> c_int {
    let state = sqlite_file(file);
    storage_result(
        &state.allowance,
        ((*state.original).xUnlock.unwrap())(file, lock),
    )
}

unsafe extern "C" fn sqlite_reserved(file: *mut ffi::sqlite3_file, out: *mut c_int) -> c_int {
    let state = sqlite_file(file);
    storage_result(
        &state.allowance,
        ((*state.original).xCheckReservedLock.unwrap())(file, out),
    )
}

unsafe extern "C" fn sqlite_shm_lock(
    file: *mut ffi::sqlite3_file,
    offset: c_int,
    n: c_int,
    flags: c_int,
) -> c_int {
    let state = sqlite_file(file);
    storage_result(
        &state.allowance,
        ((*state.original).xShmLock.unwrap())(file, offset, n, flags),
    )
}

unsafe extern "C" fn sqlite_shm_unmap(file: *mut ffi::sqlite3_file, delete: c_int) -> c_int {
    let state = sqlite_file(file);
    storage_result(
        &state.allowance,
        ((*state.original).xShmUnmap.unwrap())(file, delete),
    )
}

unsafe fn sqlite_charge(file: *mut ffi::sqlite3_file, end: u64, bytes: u64) -> c_int {
    let state = sqlite_file(file);
    let mut size = 0;
    let rc = ((*state.original).xFileSize.unwrap())(file, &mut size);
    if rc != ffi::SQLITE_OK || size < 0 {
        return storage_result(&state.allowance, ffi::SQLITE_IOERR_FSTAT);
    }
    if state
        .allowance
        .charge(end.saturating_sub(size as u64), bytes)
        .is_err()
    {
        return ffi::SQLITE_FULL;
    }
    ffi::SQLITE_OK
}

unsafe extern "C" fn sqlite_write(
    file: *mut ffi::sqlite3_file,
    bytes: *const c_void,
    len: c_int,
    offset: i64,
) -> c_int {
    let state = sqlite_file(file);
    if len < 0 || offset < 0 {
        return storage_result(&state.allowance, ffi::SQLITE_IOERR_WRITE);
    }
    let Some(end) = (offset as u64).checked_add(len as u64) else {
        return storage_result(&state.allowance, ffi::SQLITE_FULL);
    };
    let rc = sqlite_charge(file, end, len as u64);
    if rc != ffi::SQLITE_OK {
        return rc;
    }
    // Debit before forwarding. A failed/partial write never refunds its request.
    storage_result(
        &state.allowance,
        ((*state.original).xWrite.unwrap())(file, bytes, len, offset),
    )
}

unsafe extern "C" fn sqlite_truncate(file: *mut ffi::sqlite3_file, len: i64) -> c_int {
    let state = sqlite_file(file);
    if len < 0 {
        return storage_result(&state.allowance, ffi::SQLITE_IOERR_TRUNCATE);
    }
    let rc = sqlite_charge(file, len as u64, 0);
    if rc != ffi::SQLITE_OK {
        return rc;
    }
    storage_result(
        &state.allowance,
        ((*state.original).xTruncate.unwrap())(file, len),
    )
}

unsafe extern "C" fn sqlite_sync(file: *mut ffi::sqlite3_file, flags: c_int) -> c_int {
    let state = sqlite_file(file);
    storage_result(
        &state.allowance,
        ((*state.original).xSync.unwrap())(file, flags),
    )
}

unsafe extern "C" fn sqlite_control(
    file: *mut ffi::sqlite3_file,
    op: c_int,
    arg: *mut c_void,
) -> c_int {
    match op {
        // SIZE_HINT can grow/write through unix fcntlSizeHint, bypassing xWrite.
        // Chunk sizing and lock proxies likewise need separate bounded support.
        ffi::SQLITE_FCNTL_SIZE_HINT
        | ffi::SQLITE_FCNTL_CHUNK_SIZE
        | ffi::SQLITE_FCNTL_SET_LOCKPROXYFILE
        | ffi::SQLITE_FCNTL_GET_LOCKPROXYFILE => ffi::SQLITE_NOTFOUND,
        ffi::SQLITE_FCNTL_MMAP_SIZE => {
            *arg.cast::<i64>() = 0;
            ffi::SQLITE_OK
        }
        ffi::SQLITE_FCNTL_LOCKSTATE
        | ffi::SQLITE_FCNTL_LAST_ERRNO
        | ffi::SQLITE_FCNTL_HAS_MOVED
        | ffi::SQLITE_FCNTL_PERSIST_WAL
        | ffi::SQLITE_FCNTL_POWERSAFE_OVERWRITE
        | ffi::SQLITE_FCNTL_VFSNAME => {
            let state = sqlite_file(file);
            storage_result(
                &state.allowance,
                ((*state.original).xFileControl.unwrap())(file, op, arg),
            )
        }
        _ => ffi::SQLITE_NOTFOUND,
    }
}

unsafe extern "C" fn sqlite_shm_map(
    file: *mut ffi::sqlite3_file,
    page: c_int,
    size: c_int,
    extend: c_int,
    out: *mut *mut c_void,
) -> c_int {
    *out = std::ptr::null_mut();
    let state = sqlite_file(file);
    let system_page = libc::sysconf(libc::_SC_PAGESIZE);
    if page < 0 || size <= 0 || system_page <= 0 {
        return storage_result(&state.allowance, ffi::SQLITE_IOERR_SHMMAP);
    }
    let Some(end) = (page as u64 + 1).checked_mul(size as u64) else {
        return storage_result(&state.allowance, ffi::SQLITE_FULL);
    };
    let Some(rounded) = end
        .checked_add(system_page as u64 - 1)
        .map(|n| n / system_page as u64 * system_page as u64)
    else {
        return storage_result(&state.allowance, ffi::SQLITE_FULL);
    };
    // The pinned bundled unix VFS may first truncate SHM to three bytes, then
    // write one byte per new 4096-byte page. Prepay the entire possible growth
    // on every map call, even non-extending calls that can initialize SHM. No
    // refund or assumption about an already mapped connection is needed.
    let Some(growth) = rounded.checked_add(3) else {
        return storage_result(&state.allowance, ffi::SQLITE_FULL);
    };
    if state
        .allowance
        .charge(growth, rounded.div_ceil(4096))
        .is_err()
    {
        return ffi::SQLITE_FULL;
    }
    storage_result(
        &state.allowance,
        ((*state.original).xShmMap.unwrap())(file, page, size, extend, out),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_root() -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.amp/in/artifacts/bounded-storage-20261112")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn queued_ledger_operations_recheck_failed_owner_before_io() {
        let root = make_root();
        let account = Account::create(
            &root,
            uuid::Uuid::new_v4(),
            Used {
                growth: 512 * MIB,
                writes: 1024 * MIB,
            },
        )
        .unwrap();
        let lock = account.lock().unwrap();
        // A waiter passed its initial check before another owner thread
        // latched a failed publication. No stale lock confers write authority.
        account.failed.store(true, Ordering::SeqCst);
        assert!(
            account.checked_state(&lock.0).is_err(),
            "stale waiter resumed after failure"
        );
    }

    #[test]
    fn failed_post_claim_initialization_cannot_reopen_the_generation() {
        let root = make_root();
        let generation = uuid::Uuid::new_v4();
        let limits = Used {
            growth: 512 * MIB,
            writes: 1024 * MIB,
        };
        let original = Account::create(&root, generation, limits).unwrap();
        // Exclusive ledger creation fails after claiming; that same error path
        // also covers an unsuccessful header write or either initial sync.
        assert!(Account::create(&root, generation, limits).is_err());
        assert!(
            Account::open(&root, generation).is_err(),
            "ambiguous initialization resumed"
        );
        assert!(
            original.healthy().is_err(),
            "old handle resumed after initialization failure"
        );
    }

    #[test]
    fn finite_owner_claims_are_consumed_even_after_handle_exit() {
        let root = make_root();
        let generation = uuid::Uuid::new_v4();
        let account = Account::create(
            &root,
            generation,
            Used {
                growth: 512 * MIB,
                writes: 1024 * MIB,
            },
        )
        .unwrap();
        account.terminal().unwrap();
        for _ in 1..255 {
            drop(Account::open(&root, generation).unwrap());
        }
        assert!(
            Account::open(&root, generation).is_err(),
            "exited owners replenished failure backing"
        );
        assert!(
            account
                .reserve(Used {
                    growth: 7,
                    writes: 11
                })
                .is_err(),
            "owner exhaustion must fail closed, not resume under an old handle"
        );
        assert_eq!(
            std::fs::read_dir(&root)
                .unwrap()
                .filter(|entry| entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".storage-owner-"))
                .count(),
            255
        );
    }

    #[test]
    fn scoped_writer_cannot_replace_account_control_files() {
        for (name, action) in [
            (ACCOUNT_NAME, "overwrite"),
            (".storage-owner-000", "overwrite"),
            (".storage-launcher-claim", "overwrite"),
            (".storage-owner-254", "mkdir_all"),
            (".storage-launcher-claim", "mkdir_all"),
            (".storage-launcher-claim/nested", "mkdir_all"),
        ] {
            let root = make_root();
            let generation = uuid::Uuid::new_v4();
            let account = Account::create(
                &root,
                generation,
                Used {
                    growth: 512 * MIB,
                    writes: 1024 * MIB,
                },
            )
            .unwrap();
            account.terminal().unwrap();
            let target = root.join(name);
            let before = std::fs::metadata(&target).ok().map(|m| m.len());
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "infra::mock_run_authority::storage::tests::storage_sink_process",
                    "--ignored",
                    "--nocapture",
                ])
                .env_clear()
                .env("SOUNDROBE_STORAGE_REQUIRED", "1")
                .env("SOUNDROBE_NATIVE_OUTPUT", &root)
                .env("SOUNDROBE_NATIVE_GENERATION", generation.to_string())
                .env("SOUNDROBE_STORAGE_TARGET", &target)
                .env("SOUNDROBE_STORAGE_ACTION", action)
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(78),
                "scoped writer replaced {name}"
            );
            assert_eq!(std::fs::metadata(&target).ok().map(|m| m.len()), before);
        }
    }

    #[test]
    fn scoped_recursive_deletion_cannot_remove_the_account_root_through_aliases() {
        for alias in [".", "child/.."] {
            let root = make_root();
            std::fs::create_dir(root.join("child")).unwrap();
            std::fs::write(root.join("child/retained"), b"keep").unwrap();
            let generation = uuid::Uuid::new_v4();
            Account::create(
                &root,
                generation,
                Used {
                    growth: 512 * MIB,
                    writes: 1024 * MIB,
                },
            )
            .unwrap()
            .terminal()
            .unwrap();
            let before = std::fs::read(root.join(ACCOUNT_NAME)).unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "infra::mock_run_authority::storage::tests::storage_sink_process",
                    "--ignored",
                    "--nocapture",
                ])
                .env_clear()
                .env("SOUNDROBE_STORAGE_REQUIRED", "1")
                .env("SOUNDROBE_NATIVE_OUTPUT", &root)
                .env("SOUNDROBE_NATIVE_GENERATION", generation.to_string())
                .env("SOUNDROBE_STORAGE_TARGET", root.join(alias))
                .env("SOUNDROBE_STORAGE_ACTION", "remove_all")
                .output()
                .unwrap();
            assert!(
                root.join(ACCOUNT_NAME).is_file(),
                "recursive deletion removed the ledger: {alias}"
            );
            assert_eq!(output.status.code(), Some(78));
            let after = std::fs::read(root.join(ACCOUNT_NAME)).unwrap();
            // Closing admission may change the revocation byte, not grant history.
            assert_eq!(
                &after[..REVOCATION_OFFSET as usize],
                &before[..REVOCATION_OFFSET as usize]
            );
            assert!(root.join(".storage-owner-000").is_file());
            assert!(!root.join(".storage-launcher-claim").exists());
            assert_eq!(std::fs::read(root.join("child/retained")).unwrap(), b"keep");
        }
    }

    #[test]
    fn helper_sink_uses_the_existing_generation_before_file_creation() {
        let root = make_root();
        let generation = uuid::Uuid::new_v4();
        let account = Account::create(
            &root,
            generation,
            Used {
                growth: 512 * MIB,
                writes: 1024 * MIB,
            },
        )
        .unwrap();
        account.terminal().unwrap();
        let target = root.join("helper-export.json");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infra::mock_run_authority::storage::tests::storage_sink_process",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("SOUNDROBE_STORAGE_REQUIRED", "1")
            .env("SOUNDROBE_NATIVE_OUTPUT", &root)
            .env("SOUNDROBE_NATIVE_GENERATION", generation.to_string())
            .env("SOUNDROBE_STORAGE_TARGET", &target)
            .env("SOUNDROBE_STORAGE_ACTION", "create")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"{\"value\":17}\n")
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(
            std::fs::read(&target).unwrap_or_default(),
            b"{\"value\":17}\n",
            "helper output did not reach the shared native writer"
        );
        let used = account.snapshot().unwrap().ordinary;
        assert!(
            used.growth >= ACCOUNT_BACKING.growth + 13
                && used.writes >= ACCOUNT_BACKING.writes + 13
        );
    }

    #[test]
    fn account_revocation_during_commit_never_delivers_or_refunds_the_ticket() {
        let root = make_root();
        let account = Account::create(
            &root,
            uuid::Uuid::new_v4(),
            Used {
                growth: 512 * MIB,
                writes: 1024 * MIB,
            },
        )
        .unwrap();
        account.terminal().unwrap();
        let issuer = account.clone();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let paused = barrier.clone();
        let issuing = std::thread::spawn(move || {
            ACCOUNT_PAUSE
                .with(|slot| *slot.borrow_mut() = Some((AccountPoint::TicketCommitted, paused)));
            issuer.reserve(Used {
                growth: 17,
                writes: 29,
            })
        });
        barrier.wait();
        account.revoke();
        barrier.wait();
        let result = issuing.join().unwrap();
        assert!(
            result.is_err(),
            "delivered a ticket after revocation during commit"
        );
        let state = account.snapshot().unwrap();
        assert_eq!(
            state.ordinary,
            Used {
                growth: ACCOUNT_BACKING.growth + 17,
                writes: ACCOUNT_BACKING.writes + 29
            }
        );
        assert!(state.stopped);
    }

    #[test]
    fn account_attachment_requires_completed_terminal_activation() {
        let root = make_root();
        let generation = uuid::Uuid::new_v4();
        let account = Account::create(
            &root,
            generation,
            Used {
                growth: 512 * MIB,
                writes: 1024 * MIB,
            },
        )
        .unwrap();
        assert!(
            Account::open(&root, generation).is_err(),
            "attached before native activation"
        );
        assert!(!root.join(".storage-owner-001").exists());
        account.terminal().unwrap();
        assert!(Account::open(&root, generation).is_ok());
    }

    #[test]
    fn account_initialization_never_resets_a_published_revocation() {
        let root = make_root();
        let generation = uuid::Uuid::new_v4();
        let creating_root = root.clone();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let paused = barrier.clone();
        let creating = std::thread::spawn(move || {
            ACCOUNT_PAUSE
                .with(|slot| *slot.borrow_mut() = Some((AccountPoint::Initialize, paused)));
            Account::create(
                &creating_root,
                generation,
                Used {
                    growth: 512 * MIB,
                    writes: 1024 * MIB,
                },
            )
        });
        barrier.wait();
        let ledger = OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.join(ACCOUNT_NAME))
            .unwrap();
        let revocation = Revocation::map(&ledger).unwrap();
        revocation.close();
        barrier.wait();
        assert!(
            creating.join().unwrap().is_err(),
            "initialized over an existing revocation"
        );
        assert!(revocation.closed(), "initialization reset the shared stop");
        assert!(Account::open(&root, generation).is_err());
    }

    #[test]
    fn quota_denial_closes_another_holders_prepaid_admission_before_containment() {
        let root = make_root();
        let generation = uuid::Uuid::new_v4();
        let account = Account::create(
            &root,
            generation,
            Used {
                growth: 512 * MIB,
                writes: 1024 * MIB,
            },
        )
        .unwrap();
        account.terminal().unwrap();
        let allowance = account.allowance();
        let path = root.join("prepaid-peer");
        let mut file = BoundedFile::create(&path, &allowance).unwrap();
        file.write_all(b"first").unwrap();
        let reserved = account.snapshot().unwrap().ordinary;
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infra::mock_run_authority::storage::tests::storage_sink_process",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("SOUNDROBE_STORAGE_REQUIRED", "1")
            .env("SOUNDROBE_NATIVE_OUTPUT", &root)
            .env("SOUNDROBE_NATIVE_GENERATION", generation.to_string())
            .env("SOUNDROBE_STORAGE_TARGET", root.join("must-not-create"))
            .env("SOUNDROBE_STORAGE_ACTION", "quota_control")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(78));
        assert!(String::from_utf8_lossy(&output.stdout).contains("SOUNDROBE_STORAGE_DENIAL "));
        // Keep the other holder alive to exercise the window before native
        // containment. Previously prepaid bytes must not authorize a new write.
        let result = file.write_all(b"second");
        assert!(
            result.is_err(),
            "peer admitted a new write after witnessed quota denial"
        );
        assert_eq!(std::fs::read(path).unwrap(), b"first");
        assert_eq!(account.snapshot().unwrap().ordinary, reserved);
    }

    #[test]
    fn concurrent_helper_processes_cannot_renew_the_generation_cap() {
        let root = make_root();
        let generation = uuid::Uuid::new_v4();
        let account = Account::create(
            &root,
            generation,
            Used {
                growth: ACCOUNT_BACKING.growth + TERMINAL.growth + 13,
                writes: ACCOUNT_BACKING.writes + TERMINAL.writes + 29,
            },
        )
        .unwrap();
        account.terminal().unwrap();
        let mut children = Vec::new();
        for name in ["left", "right"] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "infra::mock_run_authority::storage::tests::storage_sink_process",
                    "--ignored",
                    "--nocapture",
                ])
                .env_clear()
                .env("SOUNDROBE_STORAGE_REQUIRED", "1")
                .env("SOUNDROBE_NATIVE_OUTPUT", &root)
                .env("SOUNDROBE_NATIVE_GENERATION", generation.to_string())
                .env("SOUNDROBE_STORAGE_TARGET", root.join(name))
                .env("SOUNDROBE_STORAGE_ACTION", "create")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(b"12345678").unwrap();
            children.push(child);
        }
        let statuses: Vec<_> = children
            .iter_mut()
            .map(|child| child.wait().unwrap())
            .collect();
        assert!(statuses
            .iter()
            .all(|status| matches!(status.code(), Some(0 | 78))));
        assert!(statuses.iter().any(|status| status.code() == Some(78)));
        assert!(
            statuses.iter().filter(|status| status.success()).count() <= 1,
            "processes independently replenished allowance"
        );
        let state = account.snapshot().unwrap();
        // Scoped helpers notify by immediate exit, before stop persistence.
        // This standalone control proves shared nonrenewable capacity, not
        // durable revocation or native execution-group retirement.
        assert!(state.ordinary.growth <= ACCOUNT_BACKING.growth + 13);
        assert!(state.ordinary.writes <= ACCOUNT_BACKING.writes + 29);
        assert!(
            ["left", "right"]
                .iter()
                .map(|n| std::fs::metadata(root.join(n))
                    .map(|m| m.len())
                    .unwrap_or(0))
                .sum::<u64>()
                <= 8
        );
    }

    #[test]
    fn node_and_python_helpers_use_native_storage_tickets() {
        let root = make_root();
        let generation = uuid::Uuid::new_v4();
        let account = Account::create(
            &root,
            generation,
            Used {
                growth: 512 * MIB,
                writes: 1024 * MIB,
            },
        )
        .unwrap();
        account.terminal().unwrap();
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        for (index, (language, program)) in [
            ("node", "require('./scripts/auto-tag-eval-replay/bounded-storage.cjs').writeFileSync(process.env.SOUNDROBE_STORAGE_TARGET, 'metadata:曲\\n', {flag:'wx'})"),
            ("python3", "import os,pathlib,sys;sys.path.insert(0,'scripts');from bounded_storage import write_text;write_text(pathlib.Path(os.environ['SOUNDROBE_STORAGE_TARGET']), 'metadata:曲\\n', exclusive=True)"),
            ("node", "(async()=>{const s=new(require('node:stream').PassThrough)();const ended=require('node:events').once(s,'end');const log=require('./scripts/auto-tag-eval-replay/bounded-storage.cjs').logSink(process.env.SOUNDROBE_STORAGE_TARGET);log.attach(s);s.end('metadata:曲\\n');await ended;await log.finish()})()"),
        ].into_iter().enumerate() {
            let target = root.join(format!("{index}-{language}.json"));
            let before = account.snapshot().unwrap().ordinary;
            let output = std::process::Command::new(language)
                .args(if language == "node" { vec!["-e", program] } else { vec!["-B", "-c", program] })
                .current_dir(project).env_clear().env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("SOUNDROBE_STORAGE_REQUIRED", "1").env("SOUNDROBE_NATIVE_OUTPUT", &root)
                .env("SOUNDROBE_NATIVE_GENERATION", generation.to_string())
                .env("SOUNDROBE_STORAGE_EXECUTABLE", std::env::current_exe().unwrap())
                .env("SOUNDROBE_STORAGE_TARGET", &target).output().unwrap();
            assert!(output.status.success(), "{language} sink failed: {}", String::from_utf8_lossy(&output.stderr));
            assert_eq!(std::fs::read(&target).unwrap(), "metadata:曲\n".as_bytes());
            let after = account.snapshot().unwrap().ordinary;
            assert!(after.growth > before.growth && after.writes > before.writes, "{language} escaped native admission");
        }
        let before = account.snapshot().unwrap().ordinary;
        account.stop().unwrap();
        for attempt in 0..2 {
            let output = std::process::Command::new("node")
                .args(["-e", "require('./scripts/auto-tag-eval-replay/bounded-storage.cjs').writeLauncherOutcome(process.env.SOUNDROBE_STORAGE_EXECUTABLE,process.env.SOUNDROBE_NATIVE_OUTPUT,process.env.SOUNDROBE_NATIVE_GENERATION,{state:'incomplete',requestedFieldSlots:216})"])
                .current_dir(project).env_clear().env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("SOUNDROBE_NATIVE_OUTPUT", &root).env("SOUNDROBE_NATIVE_GENERATION", generation.to_string())
                .env("SOUNDROBE_STORAGE_EXECUTABLE", std::env::current_exe().unwrap()).output().unwrap();
            assert_eq!(
                output.status.success(),
                attempt == 0,
                "terminal receipt must be one-shot"
            );
        }
        assert_eq!(
            account.snapshot().unwrap().ordinary,
            before,
            "terminal receipt requested fresh ordinary capacity"
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                &std::fs::read(root.join("launcher-outcome.json")).unwrap()
            )
            .unwrap()["requestedFieldSlots"],
            216
        );
    }

    #[test]
    fn helper_directory_creation_rejects_missing_parent_traversal_before_io() {
        let root = make_root();
        let generation = uuid::Uuid::new_v4();
        Account::create(
            &root,
            generation,
            Used {
                growth: 512 * MIB,
                writes: 1024 * MIB,
            },
        )
        .unwrap()
        .terminal()
        .unwrap();
        let name = format!("escaped-{}", uuid::Uuid::new_v4());
        let target = root.join("new/../..").join(&name);
        let escaped = root.parent().unwrap().join(name);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infra::mock_run_authority::storage::tests::storage_sink_process",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("SOUNDROBE_STORAGE_REQUIRED", "1")
            .env("SOUNDROBE_NATIVE_OUTPUT", &root)
            .env("SOUNDROBE_NATIVE_GENERATION", generation.to_string())
            .env("SOUNDROBE_STORAGE_TARGET", target)
            .env("SOUNDROBE_STORAGE_ACTION", "mkdir_all")
            .output()
            .unwrap();
        assert!(!output.status.success(), "parent traversal was admitted");
        assert!(!escaped.exists(), "directory escaped the owned root");
        assert!(
            !root.join("new").exists(),
            "rejection happened after directory creation"
        );
    }

    #[test]
    #[ignore = "native sink for explicitly scoped mock helper output"]
    fn storage_sink_process() {
        assert_eq!(
            std::env::var("SOUNDROBE_STORAGE_REQUIRED").as_deref(),
            Ok("1")
        );
        let path = PathBuf::from(std::env::var_os("SOUNDROBE_STORAGE_TARGET").unwrap());
        let action = std::env::var("SOUNDROBE_STORAGE_ACTION").unwrap();
        // Terminal attachment is read-only and cannot issue ordinary tickets.
        // The launcher invokes it only after observing actual receiver exit.
        // A stopped generation may archive one incomplete receipt, never resume.
        if action != "terminal_launcher" {
            attach_from_env().unwrap();
        }
        match action.as_str() {
            "quota_control" => {
                PROCESS_STORAGE
                    .get()
                    .unwrap()
                    .account
                    .reserve(Used {
                        growth: 0,
                        writes: 1024 * MIB,
                    })
                    .unwrap();
                panic!("storage quota control was admitted");
            }
            "terminal_launcher" => {
                let root = PathBuf::from(std::env::var_os("SOUNDROBE_NATIVE_OUTPUT").unwrap())
                    .canonicalize()
                    .unwrap();
                let generation =
                    uuid::Uuid::parse_str(&std::env::var("SOUNDROBE_NATIVE_GENERATION").unwrap())
                        .unwrap();
                let account = Account {
                    failed: account_latch(&root, generation),
                    root,
                    generation,
                    owner_slot: u8::MAX,
                    owner_pid: std::process::id(),
                    revocation: None,
                };
                let ledger = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(account.root.join(ACCOUNT_NAME))
                    .unwrap();
                {
                    let _lock = FileLock::exclusive(&ledger).unwrap();
                    assert!(account.state(&ledger).unwrap().terminal_issued);
                }
                assert_eq!(path, account.root.join("launcher-outcome.json"));
                let mut bytes = Vec::new();
                io::stdin()
                    .take(LAUNCHER_RECEIPT + 1)
                    .read_to_end(&mut bytes)
                    .unwrap();
                assert!(bytes.len() as u64 <= LAUNCHER_RECEIPT);
                // Static disjoint partition of the original terminal grant,
                // not a post-retirement ordinary grant or a renewed cap.
                let claim = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(account.root.join(".storage-launcher-claim"))
                    .unwrap();
                claim.sync_all().unwrap();
                File::open(&account.root).unwrap().sync_all().unwrap();
                let allowance = Allowance::new(&account.root, LAUNCHER_RECEIPT, LAUNCHER_RECEIPT);
                let mut output = BoundedFile::create(&path, &allowance).unwrap();
                output.write_all(&bytes).unwrap();
                output.file.sync_all().unwrap();
                File::open(&account.root).unwrap().sync_all().unwrap();
            }
            "mkdir" => files::create_dir(&path).unwrap(),
            "mkdir_all" => files::create_dir_all(&path).unwrap(),
            "remove_all" => files::remove_dir_all(&path).unwrap(),
            "create" | "overwrite" => {
                let mut bytes = Vec::new();
                io::stdin()
                    .take(8 * MIB + 1)
                    .read_to_end(&mut bytes)
                    .unwrap();
                if bytes.len() as u64 > 8 * MIB {
                    notify_scoped_failure();
                }
                let temp = path.with_file_name(format!(".storage-{}", uuid::Uuid::new_v4()));
                let mut file = files::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp)
                    .unwrap();
                file.write_all(&bytes).unwrap();
                file.sync_all().unwrap();
                drop(file);
                if action == "create" {
                    files::publish_new(&temp, &path).unwrap();
                } else {
                    files::rename(&temp, &path).unwrap();
                }
                File::open(path.parent().unwrap())
                    .unwrap()
                    .sync_all()
                    .unwrap();
            }
            "stream" => {
                let mut file = files::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .unwrap();
                let mut input = io::stdin().lock();
                let mut buffer = [0; 32768];
                let mut total = 0;
                loop {
                    let read = input.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    total += read;
                    if total > 2 * MIB as usize {
                        notify_scoped_failure();
                    }
                    file.write_all(&buffer[..read]).unwrap();
                }
                file.sync_all().unwrap();
            }
            _ => notify_scoped_failure(),
        }
    }

    #[test]
    fn scoped_failure_notifies_native_owners_without_waiting_for_ledger() {
        use std::io::BufRead;
        for mode in ["direct", "bookkeeping", "quota"] {
            let root = make_root();
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "infra::mock_run_authority::storage::tests::blocked_stop_process",
                    "--ignored",
                    "--nocapture",
                ])
                .env_clear()
                .env("SOUNDROBE_STORAGE_CONTROL_ROOT", &root)
                .env("SOUNDROBE_STORAGE_STOP_MODE", mode)
                .env("SOUNDROBE_STORAGE_STOP_DELAY", "1")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
            let mut line = String::new();
            loop {
                assert!(
                    reader.read_line(&mut line).unwrap() > 0,
                    "control exited before ready"
                );
                if line.contains("storage-control-ready") {
                    break;
                }
                line.clear();
            }
            let ledger = OpenOptions::new()
                .read(true)
                .write(true)
                .open(root.join(ACCOUNT_NAME))
                .unwrap();
            let mut identity = [0; 16];
            ledger.read_exact_at(&mut identity, 1).unwrap();
            let peer = Account::open(&root, uuid::Uuid::from_bytes(identity))
                .unwrap()
                .allowance();
            let peer_path = root.join("surviving-prepaid-peer");
            let mut peer_file = BoundedFile::create(&peer_path, &peer).unwrap();
            peer_file.write_all(b"before").unwrap();
            let lock = (mode == "direct").then(|| FileLock::exclusive(&ledger).unwrap());
            child.stdin.take().unwrap().write_all(b"stop").unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
            let mut status = None;
            while std::time::Instant::now() < deadline {
                status = child.try_wait().unwrap();
                if status.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            // Admission must reject even while the direct-mode ledger remains
            // locked. A bounded channel makes a regression report, not deadlock.
            let (send, receive) = std::sync::mpsc::channel();
            let writer = std::thread::spawn(move || {
                let _ = send.send(peer_file.write_all(b"after"));
            });
            let peer_result = receive.recv_timeout(std::time::Duration::from_millis(100));
            drop(lock);
            writer.join().unwrap();
            let timely = status.is_some();
            let status = status.unwrap_or_else(|| child.wait().unwrap());
            assert!(
                timely,
                "{mode} failure blocked on ledger before notifying native owners"
            );
            assert_eq!(status.code(), Some(78));
            assert!(
                matches!(peer_result, Ok(Err(_))),
                "{mode} peer admission did not reject before ledger I/O"
            );
            assert_eq!(std::fs::read(peer_path).unwrap(), b"before");
        }
    }

    #[test]
    #[ignore = "isolated storage stop/descriptor-loss control"]
    fn blocked_stop_process() {
        let root = PathBuf::from(std::env::var_os("SOUNDROBE_STORAGE_CONTROL_ROOT").unwrap());
        install_receiver(&root, uuid::Uuid::new_v4()).unwrap();
        println!("storage-control-ready");
        io::stdout().flush().unwrap();
        let mut input = [0; 4];
        io::stdin().read_exact(&mut input).unwrap();
        let process = PROCESS_STORAGE.get().unwrap();
        match std::env::var("SOUNDROBE_STORAGE_STOP_MODE").as_deref() {
            Ok("quota") => {
                let _ = process.ordinary.charge(0, 1024 * MIB);
            }
            Ok("bookkeeping") => {
                let lock = process.account.lock().unwrap();
                let mut state = process.account.checked_state(&lock.0).unwrap();
                state.sequence = MAX_RECORDS;
                let _ = process.account.record(
                    &lock.0,
                    state,
                    Used {
                        growth: 1,
                        writes: 1,
                    },
                );
            }
            _ => process.ordinary.stop(),
        }
        panic!("scoped failure resumed");
    }

    #[test]
    fn diagnostic_cache_open_cannot_create_an_unmetered_database() {
        let root = make_root();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infra::mock_run_authority::storage::tests::sqlite_factory_process",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("SOUNDROBE_STORAGE_CONTROL_ROOT", &root)
            .env("SOUNDROBE_STORAGE_DIAGNOSTIC", "1")
            .status()
            .unwrap();
        assert!(
            !root.join("missing-cache.sqlite").exists(),
            "diagnostic created a database outside admission"
        );
        assert_eq!(status.code(), Some(78));
    }

    #[test]
    fn real_cache_worker_and_registry_connections_share_storage_admission() {
        let root = make_root();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infra::mock_run_authority::storage::tests::sqlite_factory_process",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("SOUNDROBE_STORAGE_CONTROL_ROOT", &root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "SQLite factory control failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn scoped_read_error_cannot_be_swallowed_before_a_later_write() {
        for mode in ["read", "stat", "options"] {
            let root = make_root();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "infra::mock_run_authority::storage::tests::sqlite_factory_process",
                    "--ignored",
                    "--nocapture",
                ])
                .env_clear()
                .env("SOUNDROBE_STORAGE_CONTROL_ROOT", &root)
                .env("SOUNDROBE_STORAGE_DIAGNOSTIC", mode)
                .status()
                .unwrap();
            assert!(root.join("absence-was-allowed").exists());
            assert!(!root.join("after-read-error").exists());
            assert_eq!(status.code(), Some(78));
        }
    }

    #[test]
    fn scoped_cleanup_errors_cannot_be_swallowed_before_a_later_write() {
        let mut resumed = Vec::new();
        for mode in [
            "remove-file",
            "remove-directory",
            "create-directory",
            "publish",
            "create-tree-empty",
            "readonly-truncate",
            "readonly-write",
        ] {
            let root = make_root();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "infra::mock_run_authority::storage::tests::sqlite_factory_process",
                    "--ignored",
                    "--nocapture",
                ])
                .env_clear()
                .env("SOUNDROBE_STORAGE_CONTROL_ROOT", &root)
                .env("SOUNDROBE_STORAGE_DIAGNOSTIC", mode)
                .status()
                .unwrap();
            assert!(root.join("absence-was-allowed").exists(), "{mode}");
            if root.join("after-cleanup-error").exists() || status.code() != Some(78) {
                resumed.push(mode);
            }
        }
        assert!(resumed.is_empty(), "cleanup failures resumed: {resumed:?}");
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn scoped_existence_probe_cannot_hide_a_metadata_error_before_a_later_write() {
        let root = make_root();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infra::mock_run_authority::storage::tests::sqlite_factory_process",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("SOUNDROBE_STORAGE_CONTROL_ROOT", &root)
            .env("SOUNDROBE_STORAGE_DIAGNOSTIC", "preflight")
            .status()
            .unwrap();
        assert!(root.join("absence-was-allowed").exists());
        assert!(!root.join("after-preflight-error").exists());
        assert_eq!(status.code(), Some(78));
    }

    #[test]
    fn journal_commands_preserve_large_bindings_without_rewriting_them() {
        let root = make_root();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infra::mock_run_authority::storage::tests::sqlite_factory_process",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("SOUNDROBE_STORAGE_CONTROL_ROOT", &root)
            .env("SOUNDROBE_STORAGE_DIAGNOSTIC", "journal")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for line in String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| line.starts_with("JOURNAL_BINDINGS_STORAGE "))
        {
            println!("{line}");
        }
    }

    #[test]
    fn queued_flac_writes_and_alias_overwrites_share_storage_admission() {
        let root = make_root();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infra::mock_run_authority::storage::tests::media_writer_process",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("SOUNDROBE_STORAGE_CONTROL_ROOT", &root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "media writer control failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "explicit isolated scoped-writer control; disposable corpus copies only"]
    fn media_writer_process() {
        use crate::commands::mutations::{write_track_queued, Patch, TrackPatch};
        let root = PathBuf::from(std::env::var_os("SOUNDROBE_STORAGE_CONTROL_ROOT").unwrap());
        // Separate unscoped control inputs, never claimed as run-owned output.
        let reference_root = make_root();
        let durations = [1.104512, 1.234567, 30.000011, 300.001];
        for (i, duration) in durations.iter().enumerate() {
            crate::commands::auto_tag::eval_tests::generate_synthetic_flac(
                &reference_root.join(format!("{i}.flac")),
                *duration,
            )
            .unwrap();
        }
        install_receiver(&root, uuid::Uuid::new_v4()).unwrap();
        let process = PROCESS_STORAGE.get().unwrap();
        for (i, duration) in durations.iter().enumerate() {
            let generated = root.join(format!("synthetic-{i}.flac"));
            let reference = reference_root.join(format!("{i}.flac"));
            let before = process.ordinary.used();
            crate::commands::auto_tag::eval_tests::generate_synthetic_flac(&generated, *duration)
                .unwrap();
            assert!(
                process.ordinary.used().writes - before.writes
                    >= std::fs::metadata(&generated).unwrap().len(),
                "synthetic FFmpeg output escaped native storage admission"
            );
            assert_eq!(
                crate::commands::mutations::track_payload_digest(&generated).unwrap(),
                crate::commands::mutations::track_payload_digest(&reference).unwrap(),
                "encoded frames changed"
            );
            assert_eq!(
                &std::fs::read(&generated).unwrap()[18..42],
                &std::fs::read(&reference).unwrap()[18..42],
                "sample count/PCM digest changed"
            );
            assert!(
                std::process::Command::new("flac")
                    .args(["--silent", "--test"])
                    .arg(&generated)
                    .output()
                    .unwrap()
                    .status
                    .success(),
                "decoder rejected generated PCM/header"
            );
        }
        // Exercise the maintained queue and both prefix/full-replacement paths.
        for fixture in ["padded.flac", "flac-insufficient-padding.flac"] {
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/writer-corpus")
                .join(fixture);
            let path = root.join(fixture);
            write_new(&path, &std::fs::read(source).unwrap(), false).unwrap();
            let staging = root.join(format!("verified-{fixture}"));
            let before = process.ordinary.used();
            crate::commands::auto_tag::RecordingSource::capture(&path)
                .unwrap()
                .copy_verified_to(&path, &staging)
                .unwrap();
            let length = std::fs::metadata(&path).unwrap().len();
            assert_eq!(
                process.ordinary.used().writes - before.writes,
                length,
                "verified-source staging escaped storage admission"
            );
            files::remove_file(&staging).unwrap();
            let before = process.ordinary.used();
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(write_track_queued(
                    &crate::state::write_queue::WriteQueue::default(),
                    path.clone(),
                    TrackPatch {
                        genre: Patch::Value("Metered storage control".into()),
                        ..Default::default()
                    },
                ))
                .unwrap();
            assert!(
                process.ordinary.used().writes > before.writes,
                "queued {fixture} mutation escaped storage admission"
            );
            let track = crate::commands::tracks::read_track_metadata(&path).unwrap();
            assert_eq!(track.genre.as_deref(), Some("Metered storage control"));
        }
        let path = root.join("aliases.json");
        for (hint, alias) in [("first", "First Artist"), ("second", "Second Artist")] {
            let before = process.ordinary.used();
            crate::infra::aliases::save_alias(&path, hint, alias).unwrap();
            let length = std::fs::metadata(&path).unwrap().len();
            assert!(
                process.ordinary.used().writes - before.writes >= length,
                "alias write/overwrite escaped storage admission"
            );
        }
        let worker = crate::state::worker::WorkerStore::new_in(root.join("halt-control"));
        let before = process.ordinary.used();
        assert!(worker
            .cancel_paths(&[root.join("padded.flac")], "control")
            .is_err());
        let length = std::fs::metadata(root.join("halt-control/worker.halted.json"))
            .unwrap()
            .len();
        assert_eq!(
            process.ordinary.used().writes - before.writes,
            length,
            "worker halt staging escaped storage admission"
        );
    }

    #[test]
    #[ignore = "explicit isolated scoped-connection control; generated stores only"]
    fn sqlite_factory_process() {
        let root = PathBuf::from(std::env::var_os("SOUNDROBE_STORAGE_CONTROL_ROOT").unwrap());
        install_receiver(&root, uuid::Uuid::new_v4()).unwrap();
        let process = PROCESS_STORAGE.get().unwrap();
        if let Ok(mode) = std::env::var("SOUNDROBE_STORAGE_DIAGNOSTIC") {
            if mode == "read" || mode == "stat" || mode == "options" {
                if mode == "read" {
                    let _ = files::read(root.join("expected-absent"));
                } else if mode == "options" {
                    let _ = files::OpenOptions::new()
                        .read(true)
                        .open(root.join("expected-absent"));
                } else {
                    let _ = files::metadata(root.join("expected-absent"));
                }
                files::write(root.join("absence-was-allowed"), b"optional input absent").unwrap();
                if mode == "read" {
                    let _ = files::read(&root);
                }
                // EISDIR, not absence.
                else {
                    files::write(root.join("not-a-directory"), b"file").unwrap();
                    if mode == "options" {
                        let _ = files::OpenOptions::new()
                            .read(true)
                            .open(root.join("not-a-directory/child"));
                    } else {
                        let _ = files::metadata(root.join("not-a-directory/child"));
                    }
                }
                files::write(root.join("after-read-error"), b"must not resume").unwrap();
                return;
            }
        }
        if let Ok(mode) = std::env::var("SOUNDROBE_STORAGE_DIAGNOSTIC") {
            if matches!(
                mode.as_str(),
                "remove-file"
                    | "remove-directory"
                    | "create-directory"
                    | "publish"
                    | "create-tree-empty"
                    | "readonly-truncate"
                    | "readonly-write"
            ) {
                let _ = files::remove_file(root.join("expected-absent"));
                let _ = files::remove_dir_all(root.join("expected-absent-directory"));
                files::write(root.join("absence-was-allowed"), b"optional cleanup absent").unwrap();
                let target = root.join("cleanup-target");
                match mode.as_str() {
                    "remove-file" => {
                        files::create_dir(&target).unwrap();
                        let _ = files::remove_file(&target);
                    }
                    "remove-directory" => {
                        files::write(&target, b"not a directory").unwrap();
                        let _ = files::remove_dir_all(&target);
                    }
                    "publish" => {
                        std::os::unix::fs::symlink("cleanup-target", &target).unwrap();
                        let _ = files::publish_new(&target, &root.join("published"));
                    }
                    "create-tree-empty" => {
                        let _ = files::create_dir_all(Path::new(""));
                    }
                    "readonly-truncate" | "readonly-write" => {
                        files::write(&target, b"read-only contents").unwrap();
                        let mut file = files::File::open(&target).unwrap();
                        if mode == "readonly-truncate" {
                            let _ = file.set_len(0);
                        } else {
                            let _ = file.write(b"must fail");
                        }
                    }
                    _ => {
                        files::write(&target, b"not a directory").unwrap();
                        let _ = files::create_dir(&target);
                    }
                }
                files::write(root.join("after-cleanup-error"), b"must not resume").unwrap();
                return;
            }
        }
        #[cfg(feature = "desktop")]
        if std::env::var("SOUNDROBE_STORAGE_DIAGNOSTIC").as_deref() == Ok("preflight") {
            assert!(!crate::commands::mutations::file_exists(
                root.join("expected-absent").to_string_lossy().into_owned()
            ));
            files::write(root.join("absence-was-allowed"), b"optional input absent").unwrap();
            let path = root.join("metadata-loop");
            std::os::unix::fs::symlink("metadata-loop", &path).unwrap();
            let _ = crate::commands::mutations::file_exists(path.to_string_lossy().into_owned());
            files::write(root.join("after-preflight-error"), b"must not resume").unwrap();
            return;
        }
        if std::env::var("SOUNDROBE_STORAGE_DIAGNOSTIC").as_deref() == Ok("journal") {
            let bindings = serde_json::json!({"large": "x".repeat(328_395)});
            let membership = serde_json::from_str(include_str!(
                "../../../test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json"
            ))
            .unwrap();
            let (journal, owner) = super::super::Journal::create(
                &root.join("large.sqlite"),
                membership,
                bindings.clone(),
                None,
            )
            .unwrap();
            journal
                .command(
                    &owner,
                    super::super::Command::Lease {
                        role: "owner".into(),
                    },
                )
                .unwrap();
            let before = process.ordinary.used().writes;
            for (index, job) in super::super::JOBS.into_iter().enumerate() {
                journal
                    .command(
                        &owner,
                        super::super::Command::Register {
                            job: job.into(),
                            stage: super::super::Stage::Preparation,
                            endpoint: "http://127.0.0.1:1234".into(),
                            manifest_sha256: "a".repeat(64),
                        },
                    )
                    .unwrap();
                let report = journal.report().unwrap();
                assert_eq!(report["bindings"], bindings);
                assert_eq!(report["sessions"].as_object().unwrap().len(), index + 1);
            }
            let after = process.ordinary.used().writes;
            assert!(
                after - before < MIB,
                "unchanged bindings consumed the mutation write budget"
            );
            println!(
                "JOURNAL_BINDINGS_STORAGE {}",
                serde_json::json!({
                    "bindingsJsonBytes":serde_json::to_vec(&bindings).unwrap().len(),
                    "writesBefore":before,"writesAfter":after,"sixRegistrationWriteBytes":after-before,
                    "accounting":"scoped_application_vfs_charges_not_six_job_fit"
                })
            );
            return;
        }
        if std::env::var_os("SOUNDROBE_STORAGE_DIAGNOSTIC").is_some() {
            crate::commands::auto_tag::eval_tests::cache_counts(&root.join("missing-cache.sqlite"));
            panic!("diagnostic resumed after an unavailable read-only database");
        }
        let cache_path = root.join("cache/cache.db");
        let cache = crate::state::sqlite::CacheState::new_in(root.join("cache"));
        assert!(cache.initialize(Some(cache_path.to_str().unwrap())));
        let cache_bytes = process.ordinary.used();
        assert!(
            cache_bytes.writes > 0 && cache_bytes.growth > 0,
            "cache escaped VFS admission"
        );
        let worker = crate::state::worker::WorkerStore::new_in(root.join("worker"));
        assert!(worker.initialize());
        let worker_bytes = process.ordinary.used();
        assert!(
            worker_bytes.writes > cache_bytes.writes && worker_bytes.growth > cache_bytes.growth,
            "worker escaped VFS admission"
        );
        let membership = serde_json::from_str(include_str!(
            "../../../test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json"
        ))
        .unwrap();
        let (journal, owner) = super::super::Journal::create(
            &root.join("registry.sqlite"),
            membership,
            serde_json::json!({"storageControl":"generated stores only"}),
            None,
        )
        .unwrap();
        let created = process.ordinary.used();
        assert!(
            created.writes > worker_bytes.writes && created.growth > worker_bytes.growth,
            "registry initialization escaped VFS admission"
        );
        journal
            .command(&owner, super::super::Command::Stop)
            .unwrap();
        assert!(
            process.ordinary.used().writes > created.writes,
            "registry mutation/reopen escaped VFS admission"
        );
        assert_eq!(journal.report().unwrap()["state"], "terminal");
    }

    #[test]
    fn durable_account_shares_nonrefundable_tickets_and_protects_terminal_capacity() {
        let root = make_root();
        let generation = uuid::Uuid::new_v4();
        let limits = Used {
            growth: ACCOUNT_BACKING.growth + TERMINAL.growth + 13,
            writes: ACCOUNT_BACKING.writes + TERMINAL.writes + 29,
        };
        let owner = Account::create(&root, generation, limits).unwrap();
        let terminal = owner.terminal().unwrap();
        let peer = Account::open(&root, generation).unwrap();
        owner
            .reserve(Used {
                growth: 8,
                writes: 17,
            })
            .unwrap();
        peer.reserve(Used {
            growth: 5,
            writes: 12,
        })
        .unwrap();
        assert_eq!(
            owner.snapshot().unwrap().ordinary,
            Used {
                growth: ACCOUNT_BACKING.growth + 13,
                writes: ACCOUNT_BACKING.writes + 29,
            }
        );
        assert!(peer
            .reserve(Used {
                growth: 0,
                writes: 1
            })
            .is_err());
        assert!(
            owner.reserve(Used::default()).is_err(),
            "stop was not cross-handle"
        );
        assert_eq!(
            terminal.limits,
            Used {
                growth: TERMINAL.growth - LAUNCHER_RECEIPT,
                writes: TERMINAL.writes - LAUNCHER_RECEIPT
            }
        );
        assert!(
            peer.terminal().is_err(),
            "terminal allowance was minted twice"
        );
        let mut receipt = BoundedFile::create(&root.join("one-receipt"), &terminal).unwrap();
        receipt.write_all(b"incomplete:216").unwrap();
        assert_eq!(
            std::fs::read(root.join("one-receipt")).unwrap(),
            b"incomplete:216"
        );
        assert_eq!(
            owner.snapshot().unwrap().ordinary,
            Used {
                growth: ACCOUNT_BACKING.growth + 13,
                writes: ACCOUNT_BACKING.writes + 29,
            }
        );
        assert!(Account::open(&root, uuid::Uuid::new_v4()).is_err());
    }

    #[test]
    fn durable_account_exhaustion_does_not_create_the_excess_file() {
        let root = make_root();
        let generation = uuid::Uuid::new_v4();
        let account = Account::create(
            &root,
            generation,
            Used {
                growth: ACCOUNT_BACKING.growth + TERMINAL.growth + 11,
                writes: ACCOUNT_BACKING.writes + TERMINAL.writes + 31,
            },
        )
        .unwrap();
        let allowance = account.allowance();
        let mut file = BoundedFile::create(&root.join("first"), &allowance).unwrap();
        file.write_all(b"asymmetric!").unwrap();
        assert_eq!(std::fs::metadata(root.join("first")).unwrap().len(), 11);
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"overwrite").unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"elevenbytes").unwrap();
        assert_eq!(allowance.used().writes, 31);
        assert!(file.write_all(b"x").is_err());
        assert!(!root.join("excess").exists());
        assert!(BoundedFile::create(&root.join("excess"), &account.allowance()).is_err());
    }

    #[test]
    fn prepaid_small_overwrites_do_not_recursively_exhaust_bookkeeping() {
        let root = make_root();
        let account = Account::create(
            &root,
            uuid::Uuid::new_v4(),
            Used {
                growth: ACCOUNT_BACKING.growth + TERMINAL.growth + 31,
                writes: ACCOUNT_BACKING.writes + TERMINAL.writes + 20000,
            },
        )
        .unwrap();
        let allowance = account.allowance();
        let mut file = BoundedFile::create(&root.join("overwritten"), &allowance).unwrap();
        for _ in 0..20000 {
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(b"x").unwrap();
        }
        assert_eq!(std::fs::read(root.join("overwritten")).unwrap(), b"x");
        assert_eq!(
            allowance.used(),
            Used {
                growth: 1,
                writes: 20000
            }
        );
        assert_eq!(
            account.snapshot().unwrap().ordinary.writes,
            ACCOUNT_BACKING.writes + 20000
        );
    }

    #[test]
    fn account_rejects_missing_current_record_and_never_resumes_cached_tickets() {
        let root = make_root();
        let generation = uuid::Uuid::new_v4();
        let account = Account::create(
            &root,
            generation,
            Used {
                growth: 512 * MIB,
                writes: 1024 * MIB,
            },
        )
        .unwrap();
        let allowance = account.allowance();
        allowance.charge(1, 3).unwrap();
        let sequence = account.snapshot().unwrap().sequence;
        let ledger = OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.join(ACCOUNT_NAME))
            .unwrap();
        let mut intact = [0; 256];
        ledger
            .read_exact_at(&mut intact, sequence * RECORD)
            .unwrap();
        ledger.write_all_at(&[0; 256], sequence * RECORD).unwrap();
        assert!(
            allowance.charge(0, 1).is_err(),
            "missing issuance was accepted"
        );
        ledger.write_all_at(&intact, sequence * RECORD).unwrap();
        assert!(
            allowance.charge(0, 1).is_err(),
            "cached ticket resumed after lost recording"
        );
        assert!(Account::open(&root, generation).is_err());
    }

    #[test]
    fn terminal_issuance_slot_survives_ordinary_bookkeeping_exhaustion() {
        let root = make_root();
        let account = Account::create(
            &root,
            uuid::Uuid::new_v4(),
            Used {
                growth: 512 * MIB,
                writes: 1024 * MIB,
            },
        )
        .unwrap();
        let mut state = account.snapshot().unwrap();
        // Inject a near-full synthetic ledger, not a workload/issued-history
        // claim. This avoids thousands of fsyncs in an exhaustion control.
        state.sequence = MAX_RECORDS - 1;
        let bytes = account.encode(state, Used::default());
        let ledger = OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.join(ACCOUNT_NAME))
            .unwrap();
        ledger
            .write_all_at(&bytes, state.sequence * RECORD)
            .unwrap();
        ledger.write_all_at(&bytes, 0).unwrap();
        assert!(
            account
                .reserve(Used {
                    growth: 1,
                    writes: 1
                })
                .is_err(),
            "ordinary work used terminal's slot"
        );
        let terminal = account.terminal().unwrap();
        let mut receipt = BoundedFile::create(&root.join("terminal"), &terminal).unwrap();
        receipt.write_all(b"incomplete:216").unwrap();
    }

    #[test]
    fn lost_ledger_cannot_be_restored_to_resume_same_process_admission() {
        let root = make_root();
        let generation = uuid::Uuid::new_v4();
        let account = Account::create(
            &root,
            generation,
            Used {
                growth: 512 * MIB,
                writes: 1024 * MIB,
            },
        )
        .unwrap();
        let allowance = account.allowance();
        allowance.charge(1, 3).unwrap();
        std::fs::rename(root.join(ACCOUNT_NAME), root.join("removed-ledger")).unwrap();
        assert!(allowance.charge(0, 1).is_err());
        std::fs::rename(root.join("removed-ledger"), root.join(ACCOUNT_NAME)).unwrap();
        assert!(
            allowance.charge(0, 1).is_err(),
            "local failure did not latch"
        );
        assert!(
            Account::open(&root, generation).is_err(),
            "same generation reopened after storage loss"
        );
    }

    #[test]
    fn exact_growth_rejects_the_next_byte_before_io() {
        let root = make_root();
        let allowance = Allowance::new(&root, 11, 37);
        let path = root.join("sparse");
        let mut file = BoundedFile::create(&path, &allowance).unwrap();
        file.seek(SeekFrom::Start(8)).unwrap();
        file.write_all(b"abc").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 11);
        assert!(file.write_all(b"x").is_err(), "growth+1 reached real I/O");
        assert_eq!(
            std::fs::read(path).unwrap(),
            [vec![0; 8], b"abc".to_vec()].concat()
        );
        assert_eq!(
            allowance.used(),
            Used {
                growth: 11,
                writes: 3
            }
        );
    }

    #[test]
    fn overwrites_consume_write_bytes_without_net_growth() {
        let root = make_root();
        let allowance = Allowance::new(&root, 8, 11);
        let path = root.join("overwrite");
        let mut file = BoundedFile::create(&path, &allowance).unwrap();
        file.write_all(b"abcde").unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"12345").unwrap();
        file.seek(SeekFrom::Start(2)).unwrap();
        file.write_all(b"Z").unwrap();
        assert!(file.write_all(b"!").is_err(), "write+1 reached real I/O");
        assert_eq!(std::fs::read(path).unwrap(), b"12Z45");
        assert_eq!(
            allowance.used(),
            Used {
                growth: 5,
                writes: 11
            }
        );
    }

    #[test]
    fn shrinking_deleting_and_atomic_replacement_do_not_refund_tickets() {
        for delete in [false, true] {
            let root = make_root();
            let allowance = Allowance::new(&root, 13, 29);
            let path = root.join("target");
            let temp = root.join("atomic-temp");
            let mut original = BoundedFile::create(&path, &allowance).unwrap();
            original.write_all(b"1234567").unwrap();
            let mut staging = BoundedFile::create(&temp, &allowance).unwrap();
            staging.write_all(b"abcdef").unwrap();
            assert_eq!(
                std::fs::metadata(&path).unwrap().len() + std::fs::metadata(&temp).unwrap().len(),
                13
            );
            std::fs::rename(&temp, &path).unwrap();
            if delete {
                std::fs::remove_file(&path).unwrap();
                let mut replacement = BoundedFile::create(&path, &allowance).unwrap();
                assert!(
                    replacement.write_all(b"x").is_err(),
                    "deletion refunded growth"
                );
                assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
            } else {
                staging.set_len(2).unwrap();
                assert!(staging.set_len(3).is_err(), "truncation refunded growth");
                assert_eq!(std::fs::metadata(&path).unwrap().len(), 2);
            }
            assert_eq!(
                allowance.used(),
                Used {
                    growth: 13,
                    writes: 13
                }
            );
        }
    }

    #[test]
    fn out_of_root_and_symlink_creation_are_denied_before_open() {
        let root = make_root();
        let other = make_root();
        let allowance = Allowance::new(&root, 11, 11);
        assert!(BoundedFile::create(&other.join("outside"), &allowance).is_err());
        assert!(!other.join("outside").exists());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&other, root.join("link")).unwrap();
            assert!(BoundedFile::create(&root.join("link/escaped"), &allowance).is_err());
            assert!(!other.join("escaped").exists());
        }
    }

    #[test]
    fn sqlite_zero_write_allowance_rejects_schema_before_database_growth() {
        let root = make_root();
        let allowance = Allowance::new(&root, 65536, 0);
        let scope = ScopedSqlite::new(&allowance);
        let path = root.join("zero.sqlite");
        let connection = scope.open(&path).unwrap();
        assert!(connection
            .execute_batch("CREATE TABLE forbidden (id INTEGER)")
            .is_err());
        assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
        assert_eq!(allowance.used(), Used::default());
    }

    #[test]
    fn sqlite_rollback_journal_and_overwrites_are_not_net_file_size() {
        let root = make_root();
        let allowance = Allowance::new(&root, 1024 * 1024, 4 * 1024 * 1024);
        let scope = ScopedSqlite::new(&allowance);
        let path = root.join("rollback.sqlite");
        let connection = scope.open(&path).unwrap();
        connection.execute_batch("PRAGMA page_size=1024; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; CREATE TABLE evidence (value TEXT); INSERT INTO evidence VALUES ('original');").unwrap();
        let initial = allowance.used();
        for value in ["changed", "another", "lastone"] {
            connection
                .execute("UPDATE evidence SET value=?1", [value])
                .unwrap();
        }
        assert_eq!(
            connection
                .query_row("SELECT value FROM evidence", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "lastone"
        );
        let net = std::fs::metadata(path).unwrap().len();
        assert!(
            allowance.used().writes > initial.writes + 3 * 1024,
            "SQLite overwrites or rollback-journal writes escaped the boundary"
        );
        assert!(allowance.used().writes > net);
        assert!(
            allowance.used().growth > net,
            "deleted rollback journals were refunded"
        );
    }

    #[test]
    fn sqlite_scope_does_not_change_default_connections() {
        let root = make_root();
        let scope = ScopedSqlite::new(&Allowance::new(&root, 0, 0));
        let guarded = scope.open(&root.join("guarded.sqlite")).unwrap();
        let ordinary = rusqlite::Connection::open(root.join("ordinary.sqlite")).unwrap();
        ordinary
            .execute_batch("CREATE TABLE permitted (id INTEGER)")
            .unwrap();
        assert!(guarded
            .execute_batch("CREATE TABLE rejected (id INTEGER)")
            .is_err());
        assert_eq!(
            ordinary
                .query_row("SELECT count(*) FROM permitted", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn sqlite_wal_checkpoint_and_shared_memory_are_within_one_allowance() {
        let root = make_root();
        let allowance = Allowance::new(&root, 1024 * 1024, 4 * 1024 * 1024);
        let scope = ScopedSqlite::new(&allowance);
        let path = root.join("wal.sqlite");
        let connection = scope.open(&path).unwrap();
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE evidence (value TEXT); INSERT INTO evidence VALUES ('before');").unwrap();
        let before = allowance.used();
        assert!(
            std::fs::metadata(root.join("wal.sqlite-wal"))
                .unwrap()
                .len()
                > 0
        );
        assert!(
            std::fs::metadata(root.join("wal.sqlite-shm"))
                .unwrap()
                .len()
                >= 32768
        );
        connection
            .execute_batch("UPDATE evidence SET value='after'; PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        assert_eq!(
            connection
                .query_row("SELECT value FROM evidence", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "after"
        );
        let after = allowance.used();
        assert!(after.writes > before.writes);
        assert!(after.growth >= before.growth);
        assert_eq!(
            std::fs::metadata(root.join("wal.sqlite-wal"))
                .unwrap()
                .len(),
            0
        );
        assert!(after.growth >= std::fs::metadata(&path).unwrap().len() + 32768);
    }

    #[test]
    fn sqlite_raw_sparse_growth_and_overwrite_reject_before_vfs_io() {
        for (growth, writes, rejected_offset) in [(11, 37, 11), (37, 5, 8)] {
            let root = make_root();
            let allowance = Allowance::new(&root, growth, writes);
            let scope = ScopedSqlite::new(&allowance);
            let path = root.join("boundary.sqlite");
            let connection = scope.open(&path).unwrap();
            unsafe {
                let mut file: *mut ffi::sqlite3_file = std::ptr::null_mut();
                assert_eq!(
                    ffi::sqlite3_file_control(
                        connection.handle(),
                        c"main".as_ptr(),
                        ffi::SQLITE_FCNTL_FILE_POINTER,
                        (&mut file as *mut *mut ffi::sqlite3_file).cast()
                    ),
                    ffi::SQLITE_OK
                );
                let methods = &*(*file).pMethods;
                assert_eq!(
                    methods.xWrite.unwrap()(file, b"abc".as_ptr().cast(), 3, 8),
                    ffi::SQLITE_OK
                );
                assert_eq!(
                    methods.xWrite.unwrap()(file, b"YZ".as_ptr().cast(), 2, 8),
                    ffi::SQLITE_OK
                );
                assert_eq!(
                    methods.xWrite.unwrap()(file, b"!".as_ptr().cast(), 1, rejected_offset),
                    ffi::SQLITE_FULL
                );
            }
            assert_eq!(
                std::fs::read(path).unwrap(),
                [vec![0; 8], b"YZc".to_vec()].concat()
            );
            assert_eq!(
                allowance.used(),
                Used {
                    growth: 11,
                    writes: 5
                }
            );
        }
    }

    #[test]
    fn sqlite_delegated_storage_errors_keep_debits_and_latch_stop() {
        unsafe extern "C" fn failed_write(
            _: *mut ffi::sqlite3_file,
            _: *const c_void,
            _: c_int,
            _: i64,
        ) -> c_int {
            ffi::SQLITE_IOERR_WRITE
        }
        unsafe extern "C" fn failed_truncate(_: *mut ffi::sqlite3_file, _: i64) -> c_int {
            ffi::SQLITE_IOERR_TRUNCATE
        }
        unsafe extern "C" fn failed_sync(_: *mut ffi::sqlite3_file, _: c_int) -> c_int {
            ffi::SQLITE_IOERR_FSYNC
        }
        unsafe extern "C" fn failed_shm(
            _: *mut ffi::sqlite3_file,
            _: c_int,
            _: c_int,
            _: c_int,
            _: *mut *mut c_void,
        ) -> c_int {
            ffi::SQLITE_IOERR_SHMMAP
        }
        for operation in ["write", "truncate", "sync", "shm"] {
            let root = make_root();
            let allowance = Allowance::new(&root, 65536, 101);
            let scope = ScopedSqlite::new(&allowance);
            let connection = scope.open(&root.join("fault.sqlite")).unwrap();
            let (rc, expected) = unsafe {
                let mut file: *mut ffi::sqlite3_file = std::ptr::null_mut();
                assert_eq!(
                    ffi::sqlite3_file_control(
                        connection.handle(),
                        c"main".as_ptr(),
                        ffi::SQLITE_FCNTL_FILE_POINTER,
                        (&mut file as *mut *mut ffi::sqlite3_file).cast()
                    ),
                    ffi::SQLITE_OK
                );
                let methods = *(*file).pMethods;
                let state = (*file).pMethods as *mut SqliteFile;
                let original = (*state).original;
                let mut fault = *original;
                fault.xWrite = Some(failed_write);
                fault.xTruncate = Some(failed_truncate);
                fault.xSync = Some(failed_sync);
                fault.xShmMap = Some(failed_shm);
                (*state).original = &fault;
                let result = match operation {
                    "write" => (
                        methods.xWrite.unwrap()(file, b"abc".as_ptr().cast(), 3, 0),
                        Used {
                            growth: 3,
                            writes: 3,
                        },
                    ),
                    "truncate" => (
                        methods.xTruncate.unwrap()(file, 7),
                        Used {
                            growth: 7,
                            writes: 0,
                        },
                    ),
                    "sync" => (
                        methods.xSync.unwrap()(file, ffi::SQLITE_SYNC_FULL),
                        Used::default(),
                    ),
                    "shm" => {
                        let mut out = std::ptr::null_mut();
                        (
                            methods.xShmMap.unwrap()(file, 0, 32768, 1, &mut out),
                            Used {
                                growth: 32771,
                                writes: 8,
                            },
                        )
                    }
                    _ => unreachable!(),
                };
                // Restore before any assertion can unwind past the fault table.
                (*state).original = original;
                result
            };
            assert_ne!(
                rc,
                ffi::SQLITE_OK,
                "{operation} did not propagate the fault"
            );
            assert_eq!(allowance.used(), expected, "{operation} refunded a debit");
            assert_eq!(
                std::fs::metadata(root.join("fault.sqlite")).unwrap().len(),
                0
            );
            assert!(
                scope.open(&root.join("later.sqlite")).is_err(),
                "{operation} did not stop later admission"
            );
            assert!(!root.join("later.sqlite").exists());
        }
    }

    #[test]
    fn sqlite_read_stat_lock_and_control_errors_stop_but_expected_results_do_not() {
        use std::sync::atomic::AtomicI32;
        static RESULT: AtomicI32 = AtomicI32::new(ffi::SQLITE_OK);
        unsafe extern "C" fn read(
            _: *mut ffi::sqlite3_file,
            _: *mut c_void,
            _: c_int,
            _: i64,
        ) -> c_int {
            RESULT.load(Ordering::SeqCst)
        }
        unsafe extern "C" fn size(_: *mut ffi::sqlite3_file, out: *mut i64) -> c_int {
            *out = 0;
            RESULT.load(Ordering::SeqCst)
        }
        unsafe extern "C" fn lock(_: *mut ffi::sqlite3_file, _: c_int) -> c_int {
            RESULT.load(Ordering::SeqCst)
        }
        unsafe extern "C" fn reserved(_: *mut ffi::sqlite3_file, out: *mut c_int) -> c_int {
            *out = 0;
            RESULT.load(Ordering::SeqCst)
        }
        unsafe extern "C" fn shm(_: *mut ffi::sqlite3_file, _: c_int, _: c_int, _: c_int) -> c_int {
            RESULT.load(Ordering::SeqCst)
        }
        unsafe extern "C" fn control(_: *mut ffi::sqlite3_file, _: c_int, _: *mut c_void) -> c_int {
            RESULT.load(Ordering::SeqCst)
        }
        let cases = [
            ("read", ffi::SQLITE_IOERR_READ, true),
            ("stat", ffi::SQLITE_IOERR_FSTAT, true),
            ("lock", ffi::SQLITE_IOERR_LOCK, true),
            ("unlock", ffi::SQLITE_IOERR_UNLOCK, true),
            ("reserved", ffi::SQLITE_IOERR_CHECKRESERVEDLOCK, true),
            ("shm", ffi::SQLITE_IOERR_SHMLOCK, true),
            ("unmap", ffi::SQLITE_IOERR_SHMMAP, true),
            ("control", ffi::SQLITE_IOERR, true),
            ("read", ffi::SQLITE_IOERR_SHORT_READ, false),
            ("lock", ffi::SQLITE_BUSY, false),
            ("shm", ffi::SQLITE_BUSY, false),
            ("control", ffi::SQLITE_NOTFOUND, false),
        ];
        let mut missed = Vec::new();
        for (operation, injected, must_stop) in cases {
            let root = make_root();
            let allowance = Allowance::new(&root, 65536, 101);
            let scope = ScopedSqlite::new(&allowance);
            let connection = scope.open(&root.join("fault.sqlite")).unwrap();
            let (rc, before) = unsafe {
                let mut file: *mut ffi::sqlite3_file = std::ptr::null_mut();
                assert_eq!(
                    ffi::sqlite3_file_control(
                        connection.handle(),
                        c"main".as_ptr(),
                        ffi::SQLITE_FCNTL_FILE_POINTER,
                        (&mut file as *mut *mut ffi::sqlite3_file).cast()
                    ),
                    ffi::SQLITE_OK
                );
                let methods = *(*file).pMethods;
                let mut mapping = std::ptr::null_mut();
                assert_eq!(
                    methods.xShmMap.unwrap()(file, 0, 32768, 1, &mut mapping),
                    ffi::SQLITE_OK
                );
                let before = allowance.used();
                let state = (*file).pMethods as *mut SqliteFile;
                let original = (*state).original;
                let mut fault = *original;
                fault.xRead = Some(read);
                fault.xFileSize = Some(size);
                fault.xLock = Some(lock);
                fault.xUnlock = Some(lock);
                fault.xCheckReservedLock = Some(reserved);
                fault.xShmLock = Some(shm);
                fault.xShmUnmap = Some(lock);
                fault.xFileControl = Some(control);
                (*state).original = &fault;
                RESULT.store(injected, Ordering::SeqCst);
                let mut out = 0;
                let result = match operation {
                    "read" => methods.xRead.unwrap()(file, (&mut out as *mut c_int).cast(), 4, 0),
                    "stat" => {
                        let mut length = 0;
                        methods.xFileSize.unwrap()(file, &mut length)
                    }
                    "lock" => methods.xLock.unwrap()(file, ffi::SQLITE_LOCK_SHARED),
                    "unlock" => methods.xUnlock.unwrap()(file, ffi::SQLITE_LOCK_NONE),
                    "reserved" => methods.xCheckReservedLock.unwrap()(file, &mut out),
                    "shm" => methods.xShmLock.unwrap()(
                        file,
                        0,
                        1,
                        ffi::SQLITE_SHM_LOCK | ffi::SQLITE_SHM_EXCLUSIVE,
                    ),
                    "unmap" => methods.xShmUnmap.unwrap()(file, 0),
                    "control" => methods.xFileControl.unwrap()(
                        file,
                        ffi::SQLITE_FCNTL_LOCKSTATE,
                        (&mut out as *mut c_int).cast(),
                    ),
                    _ => unreachable!(),
                };
                (*state).original = original;
                if operation == "shm" && result == ffi::SQLITE_OK {
                    assert_eq!(
                        ((*original).xShmLock.unwrap())(
                            file,
                            0,
                            1,
                            ffi::SQLITE_SHM_UNLOCK | ffi::SQLITE_SHM_EXCLUSIVE
                        ),
                        ffi::SQLITE_OK
                    );
                }
                if operation != "unmap" || result != ffi::SQLITE_OK {
                    assert_eq!(((*original).xShmUnmap.unwrap())(file, 1), ffi::SQLITE_OK);
                }
                (result, before)
            };
            let stopped = scope.open(&root.join("later.sqlite")).is_err();
            if rc != injected || stopped != must_stop {
                missed.push((operation, injected, rc, stopped));
            }
            assert_eq!(allowance.used(), before);
        }
        assert!(
            missed.is_empty(),
            "callback failures/statuses were not classified: {missed:?}"
        );
    }

    #[test]
    fn sqlite_vfs_access_and_path_failures_stop_later_admission() {
        unsafe extern "C" fn access(
            _: *mut ffi::sqlite3_vfs,
            _: *const c_char,
            _: c_int,
            out: *mut c_int,
        ) -> c_int {
            *out = 0;
            ffi::SQLITE_IOERR_ACCESS
        }
        unsafe extern "C" fn fullpath(
            _: *mut ffi::sqlite3_vfs,
            _: *const c_char,
            _: c_int,
            _: *mut c_char,
        ) -> c_int {
            ffi::SQLITE_CANTOPEN
        }
        let mut missed = Vec::new();
        for operation in ["access", "fullpath"] {
            let root = make_root();
            let allowance = Allowance::new(&root, 101, 103);
            let mut scope = ScopedSqlite::new(&allowance);
            let name = CString::new(root.join("fault.sqlite").as_os_str().as_bytes()).unwrap();
            let (rc, expected) = unsafe {
                let original = scope.vfs.parent;
                let mut fault = *original;
                fault.xAccess = Some(access);
                fault.xFullPathname = Some(fullpath);
                scope.vfs.parent = &mut fault;
                let result = if operation == "access" {
                    let mut out = 0;
                    (
                        scope.vfs.raw.xAccess.unwrap()(
                            &mut scope.vfs.raw,
                            name.as_ptr(),
                            ffi::SQLITE_ACCESS_EXISTS,
                            &mut out,
                        ),
                        ffi::SQLITE_IOERR_ACCESS,
                    )
                } else {
                    let mut out = [0; 4096];
                    (
                        scope.vfs.raw.xFullPathname.unwrap()(
                            &mut scope.vfs.raw,
                            name.as_ptr(),
                            out.len() as c_int,
                            out.as_mut_ptr(),
                        ),
                        ffi::SQLITE_CANTOPEN,
                    )
                };
                scope.vfs.parent = original;
                result
            };
            if rc != expected || scope.open(&root.join("later.sqlite")).is_ok() {
                missed.push((operation, rc));
            }
            assert_eq!(allowance.used(), Used::default());
        }
        assert!(missed.is_empty(), "unlatched VFS failures: {missed:?}");
    }

    #[test]
    fn sqlite_shm_exact_and_plus_one_tickets_precede_hidden_io() {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        assert!(page > 0);
        let mapped = 32768_u64.max(page as u64);
        let growth = mapped + 3;
        let writes = mapped / 4096;
        for (growth_cap, write_cap, allowed) in [
            (growth, writes, true),
            (growth - 1, writes + 5, false),
            (growth + 9, writes - 1, false),
        ] {
            let root = make_root();
            let allowance = Allowance::new(&root, growth_cap, write_cap);
            let scope = ScopedSqlite::new(&allowance);
            let connection = scope.open(&root.join("boundary.sqlite")).unwrap();
            unsafe {
                let mut file: *mut ffi::sqlite3_file = std::ptr::null_mut();
                assert_eq!(
                    ffi::sqlite3_file_control(
                        connection.handle(),
                        c"main".as_ptr(),
                        ffi::SQLITE_FCNTL_FILE_POINTER,
                        (&mut file as *mut *mut ffi::sqlite3_file).cast()
                    ),
                    ffi::SQLITE_OK
                );
                let methods = &*(*file).pMethods;
                let mut out = std::ptr::null_mut();
                let rc = methods.xShmMap.unwrap()(file, 0, 32768, 1, &mut out);
                let observed = std::fs::metadata(root.join("boundary.sqlite-shm")).map(|m| m.len());
                if rc == ffi::SQLITE_OK {
                    assert_eq!(methods.xShmUnmap.unwrap()(file, 1), ffi::SQLITE_OK);
                }
                if allowed {
                    assert_eq!(rc, ffi::SQLITE_OK);
                    assert!(!out.is_null());
                    assert_eq!(observed.unwrap(), mapped);
                    assert_eq!(allowance.used(), Used { growth, writes });
                } else {
                    assert_eq!(rc, ffi::SQLITE_FULL);
                    assert!(out.is_null());
                    assert_eq!(observed.unwrap_err().kind(), io::ErrorKind::NotFound);
                    assert_eq!(allowance.used(), Used::default());
                }
            }
        }
    }

    #[test]
    fn sqlite_unnamed_disk_temp_is_unsupported_not_out_of_root() {
        let root = make_root();
        let allowance = Allowance::new(&root, 16 * 1024 * 1024, 32 * 1024 * 1024);
        let scope = ScopedSqlite::new(&allowance);
        let connection = scope.open(&root.join("temp.sqlite")).unwrap();
        connection
            .execute_batch("PRAGMA temp_store=FILE; CREATE TEMP TABLE spill (value BLOB)")
            .unwrap();
        assert!(connection
            .execute_batch("INSERT INTO spill VALUES (zeroblob(4194304))")
            .is_err());
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM spill", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn storage_denial_latches_before_any_later_smaller_write() {
        let root = make_root();
        let allowance = Allowance::new(&root, 5, 101);
        let mut file = BoundedFile::create(&root.join("stopped"), &allowance).unwrap();
        file.write_all(b"12345").unwrap();
        assert!(file.write_all(b"!").is_err());
        file.seek(SeekFrom::Start(0)).unwrap();
        assert!(
            file.write_all(b"x").is_err(),
            "denial was swallowed and later I/O was granted"
        );
        assert_eq!(std::fs::read(root.join("stopped")).unwrap(), b"12345");
        assert!(BoundedFile::create(&root.join("after-stop"), &allowance).is_err());
        assert!(!root.join("after-stop").exists());
    }

    #[test]
    fn unsupported_paths_and_failed_creation_stop_the_whole_allowance() {
        for outside in [true, false] {
            let root = make_root();
            let other = make_root();
            let allowance = Allowance::new(&root, 101, 103);
            let path = if outside {
                other.join("outside")
            } else {
                let path = root.join("already-exists");
                std::fs::write(&path, b"preserve").unwrap();
                path
            };
            assert!(BoundedFile::create(&path, &allowance).is_err());
            assert!(
                BoundedFile::create(&root.join("later"), &allowance).is_err(),
                "a non-quota storage failure did not stop later admission"
            );
            assert!(!root.join("later").exists());
            assert_eq!(allowance.used(), Used::default());
        }
    }

    #[test]
    fn delegated_write_error_keeps_its_debit_and_stops_later_admission() {
        let root = make_root();
        let allowance = Allowance::new(&root, 101, 103);
        let path = root.join("descriptor-fault");
        let mut file = BoundedFile::create(&path, &allowance).unwrap();
        // Fault injection: a real read-only descriptor returns an OS write
        // error after admission. This is not an ENOSPC or power-loss proof.
        file.file = File::open(&path).unwrap();
        assert!(file.write_all(b"abc").is_err());
        assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
        assert_eq!(
            allowance.used(),
            Used {
                growth: 3,
                writes: 3
            }
        );
        assert!(BoundedFile::create(&root.join("later"), &allowance).is_err());
        assert!(!root.join("later").exists());
    }

    #[test]
    fn concurrent_files_share_one_nonrefundable_allowance() {
        let root = make_root();
        let allowance = Allowance::new(&root, 13, 29);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|index| {
                let allowance = allowance.clone();
                let barrier = barrier.clone();
                let path = root.join(format!("worker-{index}"));
                std::thread::spawn(move || -> io::Result<()> {
                    barrier.wait();
                    BoundedFile::create(&path, &allowance)?.write_all(b"1234567")
                })
            })
            .collect();
        let successes = workers
            .into_iter()
            .map(|worker| worker.join().unwrap().is_ok() as usize)
            .sum::<usize>();
        assert_eq!(successes, 1);
        assert_eq!(
            allowance.used(),
            Used {
                growth: 7,
                writes: 7
            }
        );
        let actual: u64 = std::fs::read_dir(root)
            .unwrap()
            .map(|file| file.unwrap().metadata().unwrap().len())
            .sum();
        assert_eq!(actual, 7, "concurrent admission exceeded the shared cap");
    }

    #[test]
    fn sqlite_terminal_symlink_is_rejected_before_following_it() {
        let root = make_root();
        let other = make_root();
        let target = other.join("outside.sqlite");
        std::fs::write(&target, b"").unwrap();
        let link = root.join("link.sqlite");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let scope = ScopedSqlite::new(&Allowance::new(&root, 1024 * 1024, 1024 * 1024));
        assert!(
            scope.open(&link).is_err(),
            "SQLite opened an undeclared symlink target"
        );
        assert_eq!(std::fs::metadata(target).unwrap().len(), 0);
    }

    #[test]
    fn sqlite_uri_cannot_replace_the_scoped_vfs_at_open_or_attach() {
        for attach in [true, false] {
            let root = make_root();
            let other = make_root();
            let scope = ScopedSqlite::new(&Allowance::new(&root, 1024 * 1024, 1024 * 1024));
            let outside = other.join("bypass.sqlite");
            let uri = format!("file:{}?vfs=unix", outside.display());
            if attach {
                let connection = scope.open(&root.join("main.sqlite")).unwrap();
                assert!(
                    connection
                        .execute("ATTACH DATABASE ?1 AS bypass", [&uri])
                        .is_err(),
                    "ATTACH selected an unmetered VFS"
                );
            } else {
                assert!(
                    scope.open(Path::new(&uri)).is_err(),
                    "URI selected an unmetered VFS"
                );
            }
            assert!(
                !outside.exists(),
                "unowned URI created a file before rejection"
            );
            assert!(
                scope.open(&root.join("later.sqlite")).is_err(),
                "URI/ATTACH denial did not latch scoped admission"
            );
            assert!(!root.join("later.sqlite").exists());
        }
    }

    #[test]
    #[ignore = "registry-only calibration; run exactly alone with SOUNDROBE_STORAGE_CALIBRATION_ONLY=1"]
    fn calibrate_current_journal_with_retained_binding_size() {
        use super::super::{Command, Journal, Stage};
        use serde_json::{json, Value};
        use sha2::{Digest, Sha256};
        assert_eq!(
            std::env::var("SOUNDROBE_STORAGE_CALIBRATION_ONLY").as_deref(),
            Ok("1")
        );
        let retained = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(
                "../.amp/in/artifacts/mock-supervision-20261112/run-114/authority/registry.sqlite",
            )
            .canonicalize()
            .unwrap();
        let original = std::fs::read(&retained).unwrap();
        let original_sha = format!("{:x}", Sha256::digest(&original));
        assert_eq!(
            original_sha,
            "572421bf5f7bbd16387ff91c010dfdd3a3941c06770d13b712c695bc8ba7cd65"
        );
        let state: Value = {
            let readonly = Connection::open_with_flags(
                format!("file:{}?immutable=1", retained.display()),
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
            )
            .unwrap();
            let bytes: String = readonly
                .query_row("SELECT state FROM authority WHERE id=1", [], |row| {
                    row.get(0)
                })
                .unwrap();
            serde_json::from_str(&bytes).unwrap()
        };
        let digest = |s: &str| format!("{:x}", Sha256::digest(s.as_bytes()));
        let mut bindings = state["bindings"].clone();
        // Existing lifecycle helpers bind this synthetic digest. All immutable
        // source bindings retain their actual sizes; no old seal is rewritten.
        bindings["executable"] = json!(digest("test executable"));
        let binding_bytes = serde_json::to_vec(&bindings).unwrap().len();
        let root = make_root();
        let allowance = Allowance::new(&root, 510 * 1024 * 1024, 1016 * 1024 * 1024);
        let mut scope = ScopedSqlite::new(&allowance);
        // Only this explicitly isolated ignored process changes its SQLite
        // default, to observe the CURRENT Journal without replacing its writer.
        // No application instance or ordinary test process uses this hook.
        unsafe {
            assert_eq!(
                ffi::sqlite3_vfs_register(&mut scope.vfs.raw, 1),
                ffi::SQLITE_OK
            );
        }
        let began = std::time::Instant::now();
        let (journal, owner) = Journal::create(
            &root.join("registry.sqlite"),
            state["membership"].clone(),
            bindings,
            None,
        )
        .unwrap();
        journal
            .command(
                &owner,
                Command::Lease {
                    role: "owner".into(),
                },
            )
            .unwrap();
        let checkpoint = super::super::tests::freeze(&journal, &owner);
        journal
            .command(
                &owner,
                Command::Consume {
                    fake_checkpoint_sha256: checkpoint,
                },
            )
            .unwrap();
        let session = journal
            .command(
                &owner,
                Command::Register {
                    job: "disabled-crowd".into(),
                    stage: Stage::Execution,
                    endpoint: "http://127.0.0.1:1234".into(),
                    manifest_sha256: digest("fixture"),
                },
            )
            .unwrap();
        let client = session["client"].as_str().unwrap();
        let service = session["service"].as_str().unwrap();
        journal
            .command(
                client,
                Command::Lease {
                    role: "native".into(),
                },
            )
            .unwrap();
        journal
            .command(
                service,
                Command::Lease {
                    role: "fixture".into(),
                },
            )
            .unwrap();
        journal
            .command(
                client,
                Command::Attach {
                    executable_sha256: digest("test executable"),
                    preparation_sha256: Some(digest("disabled-crowd")),
                },
            )
            .unwrap();
        let mut settled = 0;
        let mut transitions = 0;
        let result = (|| -> Result<(), String> {
            for _ in 0..497 {
                if began.elapsed() > std::time::Duration::from_secs(60) {
                    return Err("calibration_time_limit".into());
                }
                let op = journal.command(
                    client,
                    Command::Reserve {
                        method: "GET".into(),
                        path: "/musicbrainz/ws/2/release/storage-probe?inc=recordings".into(),
                        body_sha256: digest(""),
                    },
                )?;
                transitions += 1;
                let operation = op["operation"].as_str().unwrap().to_string();
                journal.command(
                    &owner,
                    Command::Dispatch {
                        operation: operation.clone(),
                    },
                )?;
                transitions += 1;
                journal.command(
                    service,
                    Command::Arrival {
                        operation: operation.clone(),
                        request_sha256: op["requestSha256"].as_str().unwrap().into(),
                    },
                )?;
                transitions += 1;
                journal.command(&owner, Command::Complete { operation })?;
                transitions += 1;
                settled += 1;
            }
            Ok(())
        })();
        let used = allowance.used();
        eprintln!(
            "{}",
            json!({"kind":"current-journal-only synthetic 497-provider calibration, NOT six-job evaluation or acceptance",
            "root":root,"bindingBytes":binding_bytes,"settledSyntheticOperations":settled,"acknowledgedOperationTransitions":transitions,
            "elapsedMs":began.elapsed().as_millis(),"growthTicketsConsumed":used.growth,"vfsWriteBytesIssued":used.writes,
            "ordinaryGrowthLimit":allowance.limits.growth,"ordinaryWriteLimit":allowance.limits.writes,
            "result":result,"requestedFieldSlots":216,"liveCalls":0,"storageRunAdmissionImplemented":false})
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(std::fs::read(retained).unwrap())),
            original_sha
        );
        assert_ne!(
            result.as_ref().err().map(String::as_str),
            Some("calibration_time_limit")
        );
        assert!(
            result.is_ok() || allowance.stopped.load(Ordering::SeqCst),
            "non-storage rejection confounded calibration"
        );
    }
}
