//! Unscoped owner custody and private native-only launch coordination.

use crate::infra::mock_run_authority::{self as authority, owner, owner_checkpoint, policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::Duration;

pub(super) const OWNER_FD: &str = "SOUNDROBE_NATIVE_OWNER_CHANNEL";
static INHERITED: Mutex<Option<UnixStream>> = Mutex::new(None);

fn private_channel(stream: UnixStream) -> std::io::Result<UnixStream> {
    let fd = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(stream)
}

/// Call at native role entry, before any process creation. Only the two
/// intermediate control roles forward the descriptor, once, then close it.
pub(super) fn capture_inherited() -> Result<(), String> {
    let Some(raw) = std::env::var_os(OWNER_FD) else {
        return Ok(());
    };
    let fd: i32 = raw
        .to_str()
        .ok_or("owner_channel_invalid")?
        .parse()
        .map_err(|_| "owner_channel_invalid")?;
    authority::require(fd >= 3)?;
    // Validate before taking ownership: never turn a file/pipe into a socket.
    let mut kind: libc::c_int = 0;
    let mut length = std::mem::size_of_val(&kind) as libc::socklen_t;
    authority::require(
        unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_TYPE,
                (&mut kind as *mut libc::c_int).cast(),
                &mut length,
            )
        } == 0
            && kind == libc::SOCK_STREAM,
    )?;
    let stream = private_channel(unsafe { UnixStream::from_raw_fd(fd) })
        .map_err(|_| "owner_channel_invalid")?;
    let mut inherited = INHERITED.lock().map_err(|_| "owner_channel_invalid")?;
    authority::require(inherited.is_none())?;
    *inherited = Some(stream);
    Ok(())
}

pub(super) fn forward_command(command: &mut std::process::Command, role: &str) {
    if !matches!(role, "supervisor" | "broker") {
        return;
    }
    let Some(stream) = INHERITED
        .lock()
        .expect("native owner descriptor lock")
        .take()
    else {
        return;
    };
    command.env(OWNER_FD, stream.as_raw_fd().to_string());
    unsafe {
        command.pre_exec(move || {
            let fd = stream.as_raw_fd();
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ClaimedPublicBinding {
    pub(crate) version: u8,
    pub(crate) proposal_id: String,
    pub(crate) proposal_sha256: String,
    pub(crate) intent_sha256: String,
    pub(crate) execution_kind: owner::ExecutionKind,
    pub(crate) nonce: String,
    pub(crate) expires_unix_seconds: u64,
}

/// Public data, never the claimed capability or a signing request. The
/// disposable custody slot is accepted only by the explicit offline constructor.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OwnerBootstrap {
    pub(crate) proposal: owner::ExecutionProposal,
    pub(crate) intent: owner::OwnerExecutionIntent,
    pub(crate) claimed: ClaimedPublicBinding,
    pub(crate) execution_kind: owner::ExecutionKind,
    pub(crate) disposable_run_root: PathBuf,
    offline_custody: PathBuf,
}

impl OwnerBootstrap {
    pub(crate) fn prepared(
        &self,
        root: &Path,
        plan: &policy::RehearsalPlan,
    ) -> Result<policy::boundary::PreparedBootstrap, String> {
        authority::require(
            self.execution_kind == owner::ExecutionKind::OfflineRehearsal
                && self.proposal.execution_kind == self.execution_kind
                && self.intent.execution_kind == self.execution_kind
                && self.claimed.execution_kind == self.execution_kind
                && self.disposable_run_root == root
                && self.claimed.version == 1
                && self.claimed.proposal_id == self.proposal.proposal_id
                && self.intent.proposal_id == self.proposal.proposal_id
                && self.claimed.proposal_sha256 == self.proposal.sha256()?
                && self.intent.proposal_sha256 == self.claimed.proposal_sha256
                && self.claimed.intent_sha256 == self.intent.sha256()?
                && self.claimed.nonce == self.intent.nonce
                && self.claimed.expires_unix_seconds == self.intent.expires_unix_seconds
                && self.claimed.expires_unix_seconds > policy::now()?,
        )?;
        validate_disposable_custody(&self.offline_custody, &self.proposal, plan)?;
        let prepared = policy::boundary::PreparedBootstrap::read(&self.offline_custody, plan)?;
        authority::require(
            prepared.context.binding()["verifierSha256"] == self.proposal.verifier_sha256,
        )?;
        Ok(prepared)
    }

    /// Optional public bytes are bound by the signed proposal's source list.
    /// They can change only the disposable loopback reply, always HTTP 200.
    pub(crate) fn response_fixture(&self, plan_file: &Path) -> Result<Option<Vec<u8>>, String> {
        use std::os::unix::fs::MetadataExt;
        authority::require(
            self.execution_kind == owner::ExecutionKind::OfflineRehearsal
                && self.proposal.execution_kind == self.execution_kind
                && Some(plan_file)
                    == self
                        .disposable_run_root
                        .parent()
                        .map(|p| p.join("public-native-plan.json"))
                        .as_deref(),
        )?;
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("owner_workspace_invalid")?;
        let file = plan_file.with_file_name("response-fixture.json");
        let relative = file
            .strip_prefix(workspace)
            .map_err(|_| "owner_response_invalid")?
            .to_string_lossy();
        let sources = &self.proposal.sources_and_tools;
        let Some(lock) = sources
            .as_array()
            .ok_or("owner_sources_invalid")?
            .iter()
            .find(|source| source["path"] == relative.as_ref())
        else {
            return Ok(None);
        };
        authority::require(
            std::fs::symlink_metadata(&file)
                .is_ok_and(|m| m.is_file() && m.nlink() == 1 && m.len() <= 65536),
        )?;
        policy::OfflineResponse {
            sha256: lock["sha256"]
                .as_str()
                .ok_or("owner_response_invalid")?
                .into(),
            status: 200,
        }
        .read(plan_file, sources, workspace)
        .map(Some)
    }

    /// Signed, fixed-sibling fault input; never an endpoint or execution grant.
    pub(crate) fn response_wire(
        &self,
        plan_file: &Path,
        response_len: usize,
    ) -> Result<Option<usize>, String> {
        use std::os::unix::fs::MetadataExt;
        authority::require(
            self.execution_kind == owner::ExecutionKind::OfflineRehearsal
                && self.proposal.execution_kind == self.execution_kind
                && Some(plan_file)
                    == self
                        .disposable_run_root
                        .parent()
                        .map(|p| p.join("public-native-plan.json"))
                        .as_deref(),
        )?;
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("owner_workspace_invalid")?;
        let file = plan_file.with_file_name("response-wire.json");
        let relative = file
            .strip_prefix(workspace)
            .map_err(|_| "owner_wire_invalid")?
            .to_string_lossy();
        let locks: Vec<_> = self
            .proposal
            .sources_and_tools
            .as_array()
            .ok_or("owner_sources_invalid")?
            .iter()
            .filter(|source| source["path"] == relative.as_ref())
            .collect();
        if locks.is_empty() {
            return Ok(None);
        }
        authority::require(
            locks.len() == 1
                && file.canonicalize().ok().as_deref() == Some(file.as_path())
                && std::fs::symlink_metadata(&file)
                    .is_ok_and(|m| m.is_file() && m.nlink() == 1 && m.len() <= 1024)
                && self
                    .response_fixture(plan_file)?
                    .is_some_and(|bytes| bytes.len() == response_len),
        )?;
        let mut bytes = Vec::new();
        std::fs::File::open(&file)
            .map_err(|_| "owner_wire_invalid")?
            .take(1025)
            .read_to_end(&mut bytes)
            .map_err(|_| "owner_wire_invalid")?;
        authority::require(bytes.len() <= 1024 && locks[0]["sha256"] == authority::sha(&bytes))?;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            version: u8,
            truncate_after_bytes: usize,
        }
        let wire: Wire = serde_json::from_slice(&bytes).map_err(|_| "owner_wire_invalid")?;
        authority::require(
            wire.version == 1
                && wire.truncate_after_bytes > 0
                && wire.truncate_after_bytes < response_len,
        )?;
        Ok(Some(wire.truncate_after_bytes))
    }

    pub(crate) fn public_bootstrap(
        &self,
        root: &Path,
        plan: &policy::RehearsalPlan,
    ) -> Result<Value, String> {
        self.prepared(root, plan)?;
        let (config, _) =
            super::receiver::read_json(&self.offline_custody.join("prepared-bootstrap.json"))
                .map_err(|_| "owner_custody_invalid")?;
        let credential = Path::new(
            config["credentialPath"]
                .as_str()
                .ok_or("owner_custody_invalid")?,
        );
        Ok(
            serde_json::json!({"version":1,"proposal":self.proposal,"intent":self.intent,
            "ownerExecution":self.claimed_binding(),
            "offlineCredential":offline_credential_binding(credential, &plan.model)?}),
        )
    }

    /// Insert this exact value into bindings.ownerExecution BEFORE creating
    /// the Journal. Possession of it does not confer the owner's capability.
    pub(crate) fn claimed_binding(&self) -> Value {
        serde_json::to_value(&self.claimed).expect("public native binding serialization")
    }
}

fn write_bootstrap(stream: &mut UnixStream, bootstrap: &OwnerBootstrap) -> Result<(), String> {
    let bytes = serde_json::to_vec(bootstrap).map_err(|_| "owner_bootstrap_invalid")?;
    authority::require(bytes.len() <= 1024 * 1024)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| "owner_channel_lost")?;
    stream
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .and_then(|_| stream.write_all(&bytes))
        .map_err(|_| "owner_channel_lost".into())
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Status {
    Heartbeat,
    Frozen { run_id: String },
    Approved,
}

struct StatusChannel {
    stream: UnixStream,
    pending: Vec<u8>,
}
impl StatusChannel {
    fn new(stream: UnixStream) -> Result<Self, String> {
        stream
            .set_nonblocking(true)
            .map_err(|_| "owner_channel_lost")?;
        Ok(Self {
            stream,
            pending: Vec::new(),
        })
    }
    fn send(&mut self, status: &Status) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(status).map_err(|_| "owner_status_invalid")?;
        authority::require(bytes.len() < 512)?;
        bytes.push(b'\n');
        // Short or failed sends close authority. Never retry a status mutation.
        authority::require(
            self.stream
                .write(&bytes)
                .map_err(|_| "owner_channel_lost")?
                == bytes.len(),
        )
    }
    fn receive(&mut self) -> Result<Option<Status>, String> {
        if let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let frame: Vec<_> = self.pending.drain(..=end).collect();
            return serde_json::from_slice(&frame)
                .map(Some)
                .map_err(|_| "owner_status_invalid".into());
        }
        let mut bytes = [0; 512];
        match self.stream.read(&mut bytes) {
            Ok(0) => return Err("owner_channel_lost".into()),
            Ok(n) => self.pending.extend_from_slice(&bytes[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
            Err(_) => return Err("owner_channel_lost".into()),
        }
        authority::require(self.pending.len() <= 1024)?;
        Ok(None)
    }
}

/// Native-only capability for this signed disposable launch. It cannot select
/// an endpoint or authorize HTTPS, and publication alone does not consume approval.
pub(crate) struct OfflineDispatchPermit {
    context_sha256: String,
    expires: u64,
    published: Arc<AtomicBool>,
}
impl OfflineDispatchPermit {
    pub(crate) fn admit(&self, context: &policy::Rehearsal) -> Result<(), String> {
        authority::require(
            self.published.load(Ordering::SeqCst)
                && self.expires > policy::now()?
                && authority::sha(&serde_json::to_vec(&context.binding()).unwrap())
                    == self.context_sha256,
        )
    }
}

pub(crate) struct BrokerOwnerLink {
    frozen: mpsc::SyncSender<String>,
    announced: AtomicBool,
    approved: Arc<AtomicBool>,
}
impl BrokerOwnerLink {
    /// Call at the START of supervised_process, before waiting for the Node
    /// bootstrap or constructing context/Journal. Keep the link until exit.
    pub(crate) fn bootstrap(
        root: &Path,
        monitor: Arc<super::Monitor>,
    ) -> Result<Option<(Self, OwnerBootstrap)>, String> {
        let Some(mut stream) = INHERITED
            .lock()
            .map_err(|_| "owner_channel_invalid")?
            .take()
        else {
            return Ok(None);
        };
        monitor.check()?;
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .map_err(|_| "owner_channel_lost")?;
        let mut size = [0; 4];
        stream
            .read_exact(&mut size)
            .map_err(|_| "owner_channel_lost")?;
        let size = u32::from_le_bytes(size) as usize;
        authority::require(size > 0 && size <= 1024 * 1024)?;
        let mut bytes = vec![0; size];
        stream
            .read_exact(&mut bytes)
            .map_err(|_| "owner_channel_lost")?;
        let bootstrap: OwnerBootstrap =
            serde_json::from_slice(&bytes).map_err(|_| "owner_bootstrap_invalid")?;
        authority::require(
            bootstrap.execution_kind == owner::ExecutionKind::OfflineRehearsal
                && bootstrap.disposable_run_root == root,
        )?;
        monitor.check()?;
        let (tx, rx) = mpsc::sync_channel(1);
        let approved = Arc::new(AtomicBool::new(false));
        let published = approved.clone();
        let mut channel = StatusChannel::new(stream)?;
        std::thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                let mut seen = super::monotonic_ms();
                let mut heartbeat = 0;
                loop {
                    monitor.check()?;
                    let now = super::monotonic_ms();
                    if now.saturating_sub(seen) >= 1000 {
                        return Err("owner_heartbeat_lost".into());
                    }
                    match channel.receive()? {
                        Some(Status::Heartbeat) => seen = now,
                        Some(Status::Approved) if !published.swap(true, Ordering::SeqCst) => {
                            seen = now
                        }
                        Some(_) => return Err("owner_status_invalid".into()),
                        None => {}
                    }
                    match rx.try_recv() {
                        Ok(run_id) => channel.send(&Status::Frozen { run_id })?,
                        Err(mpsc::TryRecvError::Empty) => {}
                        Err(mpsc::TryRecvError::Disconnected) => {
                            return Err("owner_link_dropped".into())
                        }
                    }
                    if now.saturating_sub(heartbeat) >= 250 {
                        channel.send(&Status::Heartbeat)?;
                        heartbeat = now;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            })();
            if let Err(reason) = result {
                tracing::warn!(reason, "native_owner_channel_stopped");
                monitor.stopped.store(true, Ordering::SeqCst);
            }
        });
        Ok(Some((
            Self {
                frozen: tx,
                announced: AtomicBool::new(false),
                approved,
            },
            bootstrap,
        )))
    }

    pub(crate) fn frozen(&self, run_id: &str) -> Result<(), String> {
        authority::require(
            uuid::Uuid::parse_str(run_id)
                .is_ok_and(|id| id.get_version_num() == 4 && id.to_string() == run_id)
                && !self.announced.swap(true, Ordering::SeqCst),
        )?;
        self.frozen
            .try_send(run_id.into())
            .map_err(|_| "owner_channel_lost".into())
    }
    pub(crate) fn publication_flag(&self) -> Arc<AtomicBool> {
        self.approved.clone()
    }
    pub(crate) fn offline_dispatch(
        &self,
        bootstrap: &OwnerBootstrap,
        root: &Path,
        plan: &policy::RehearsalPlan,
    ) -> Result<OfflineDispatchPermit, String> {
        let prepared = bootstrap.prepared(root, plan)?;
        Ok(OfflineDispatchPermit {
            context_sha256: authority::sha(
                &serde_json::to_vec(&prepared.context.binding()).unwrap(),
            ),
            expires: bootstrap.claimed.expires_unix_seconds,
            published: self.approved.clone(),
        })
    }
}

/// Constructed only by native offline code. No environment-selected hosted
/// custody, no retry/resume, and no claimed capability crosses the channel.
pub(crate) struct OfflineOwnerLaunch {
    bootstrap: OwnerBootstrap,
    claimed: owner::ClaimedOwnerExecutionIntent,
    plan: policy::RehearsalPlan,
    plan_file: PathBuf,
}

struct OwnerWork {
    request: mpsc::SyncSender<String>,
    reply: mpsc::Receiver<Result<(), String>>,
    live: Arc<AtomicBool>,
}
impl Drop for OwnerWork {
    fn drop(&mut self) {
        self.live.store(false, Ordering::SeqCst);
    }
}
impl OwnerWork {
    fn start(
        work: impl FnOnce(String, &AtomicBool) -> Result<(), String> + Send + 'static,
    ) -> Self {
        let (request, requests) = mpsc::sync_channel(1);
        let (replies, reply) = mpsc::sync_channel(1);
        let live = Arc::new(AtomicBool::new(true));
        let worker_live = live.clone();
        std::thread::spawn(move || {
            if let Ok(run_id) = requests.recv() {
                let result = work(run_id, &worker_live);
                let _ = replies.send(result);
            }
        });
        Self {
            request,
            reply,
            live,
        }
    }
}

impl OfflineOwnerLaunch {
    pub(crate) fn prepare(
        directory: &Path,
        root: &Path,
        proposal: owner::ExecutionProposal,
        intent: owner::OwnerExecutionIntent,
        plan_file: &Path,
        bindings: &Value,
    ) -> Result<Self, String> {
        authority::require(
            !authority::storage::is_scoped()
                && proposal.execution_kind == owner::ExecutionKind::OfflineRehearsal,
        )?;
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("owner_workspace_invalid")?;
        let artifact_root = workspace
            .join(".amp/in/artifacts")
            .canonicalize()
            .map_err(|_| "owner_root_invalid")?;
        let parent = root
            .parent()
            .ok_or("owner_root_invalid")?
            .canonicalize()
            .map_err(|_| "owner_root_invalid")?;
        authority::require(
            root.is_absolute()
                && parent.starts_with(&artifact_root)
                && root.file_name().is_some_and(|n| n != "." && n != "..")
                && root == parent.join(root.file_name().ok_or("owner_root_invalid")?)
                && std::fs::symlink_metadata(root)
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                && bindings["disposableRunRoot"] == root.to_string_lossy().as_ref()
                && std::fs::symlink_metadata(directory.join("prepared-approval.json"))
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        )?;
        authority::require(
            plan_file == parent.join("public-native-plan.json")
                && plan_file.canonicalize().ok().as_deref() == Some(plan_file),
        )?;
        let (raw_plan, _) =
            super::receiver::read_json(plan_file).map_err(|_| "owner_plan_invalid")?;
        let plan: policy::RehearsalPlan =
            serde_json::from_value(raw_plan).map_err(|_| "owner_plan_invalid")?;
        let credential = validate_disposable_custody(directory, &proposal, &plan)?;
        let prepared = policy::boundary::PreparedBootstrap::read(directory, &plan)?;
        proposal.verify_inputs(
            owner::ExecutionKind::OfflineRehearsal,
            &serde_json::from_str::<Value>(super::receiver::MEMBERSHIP).unwrap(),
            bindings,
            &plan,
            &prepared.context,
            &proposal.verifier_sha256,
        )?;
        let verified = owner::verify_owner_intent(
            directory,
            &proposal,
            &intent,
            owner::ExecutionKind::OfflineRehearsal,
            &proposal.verifier_sha256,
        )?;
        verify_sources(&proposal, plan_file, &credential)?;
        let claimed = verified.claim_once(directory)?;
        let binding =
            serde_json::from_value(claimed.binding()?).map_err(|_| "owner_binding_invalid")?;
        Ok(Self {
            bootstrap: OwnerBootstrap {
                proposal,
                intent,
                claimed: binding,
                execution_kind: owner::ExecutionKind::OfflineRehearsal,
                disposable_run_root: root.into(),
                offline_custody: directory.into(),
            },
            claimed,
            plan,
            plan_file: plan_file.canonicalize().map_err(|_| "owner_plan_invalid")?,
        })
    }

    pub(crate) fn run(self) -> Result<(), String> {
        authority::require(!authority::storage::is_scoped())?;
        // The only start timestamp for this receiver/supervisor/broker launch.
        let start = super::monotonic_ms();
        let root = self.bootstrap.disposable_run_root.clone();
        let parent = unsafe { libc::getppid() };
        authority::require(parent > 1)?;
        let (mut owner_channel, broker_channel) =
            UnixStream::pair().map_err(|_| "owner_channel_unavailable")?;
        let mut receiver = super::child(
            "receiver",
            &self.bootstrap.disposable_run_root,
            start,
            300_000,
            &[(OWNER_FD, &broker_channel)],
        )
        .env("SOUNDROBE_NATIVE_EXECUTION_MS", "600000")
        .env("SOUNDROBE_NATIVE_CONTROL", "")
        .env("SOUNDROBE_MODEL_REHEARSAL_PLAN", &self.plan_file)
        .env_remove("SOUNDROBE_STORAGE_REQUIRED")
        .env_remove("SOUNDROBE_NATIVE_MANAGED")
        .process_group(0)
        .spawn()
        .map_err(|_| "owner_launch_failed")?;
        drop(broker_channel);
        // Every subsequent failure drops the private endpoint. Existing native
        // monitor/retirement controls, not an untrusted PGID, stop the run.
        write_bootstrap(&mut owner_channel, &self.bootstrap)?;
        let mut channel = StatusChannel::new(owner_channel)?;
        let worker = OwnerWork::start(move |run_id, live| {
            let prepared = self
                .bootstrap
                .prepared(&self.bootstrap.disposable_run_root, &self.plan)?;
            let checkpoint = owner_checkpoint::FrozenOwnerCheckpoint::read(
                &self
                    .bootstrap
                    .disposable_run_root
                    .join("authority/registry.sqlite"),
                prepared.context,
            )?;
            authority::require(
                checkpoint.report()["runId"] == run_id
                    && checkpoint.report()["bindings"]["disposableRunRoot"]
                        == self
                            .bootstrap
                            .disposable_run_root
                            .to_string_lossy()
                            .as_ref(),
            )?;
            let credential = validate_disposable_custody(
                &self.bootstrap.offline_custody,
                &self.bootstrap.proposal,
                &self.plan,
            )?;
            verify_sources(&self.bootstrap.proposal, &self.plan_file, &credential)?;
            authority::require(live.load(Ordering::SeqCst))?;
            let approval = checkpoint.issue(
                &self.bootstrap.offline_custody,
                &self.bootstrap.proposal,
                self.claimed,
                &self.plan,
            )?;
            authority::require(live.load(Ordering::SeqCst))?;
            policy::boundary::publish_owner_approval(&self.bootstrap.offline_custody, &approval)
        });
        let mut seen = start;
        let mut connected = false;
        let mut heartbeat = 0;
        let mut published = false;
        let mut requested = false;
        loop {
            authority::require(unsafe { libc::getppid() } == parent)?;
            if requested && !published {
                match worker.reply.try_recv() {
                    Ok(result) => {
                        result?;
                        published = true;
                        channel.send(&Status::Approved)?;
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => return Err("owner_worker_lost".into()),
                }
            }
            if let Some(status) = receiver.try_wait().map_err(|_| "owner_receiver_unknown")? {
                authority::require(status.success() && published)?;
                return archive_receiver_exit(&root, &mut receiver);
            }
            let now = super::monotonic_ms();
            // No independent execution clock or extension: the existing
            // native scope owns the 300/600-second transition and retirement.
            authority::require(now.saturating_sub(seen) < if connected { 1000 } else { 5000 })?;
            match channel.receive() {
                Ok(Some(Status::Heartbeat)) => {
                    seen = now;
                    connected = true;
                }
                Ok(Some(Status::Frozen { run_id })) => {
                    authority::require(!requested)?;
                    requested = true;
                    worker
                        .request
                        .try_send(run_id)
                        .map_err(|_| "owner_worker_lost")?;
                }
                Ok(Some(_)) => return Err("owner_status_invalid".into()),
                Ok(None) => {}
                Err(_) if published => {
                    // Broker closes during normal retirement. Wait only the
                    // existing retirement grace, with parent liveness checks.
                    let retirement = now;
                    loop {
                        authority::require(
                            unsafe { libc::getppid() } == parent
                                && super::monotonic_ms().saturating_sub(retirement) < 10_000,
                        )?;
                        if let Some(status) =
                            receiver.try_wait().map_err(|_| "owner_receiver_unknown")?
                        {
                            authority::require(status.success())?;
                            return archive_receiver_exit(&root, &mut receiver);
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
                Err(error) => return Err(error),
            }
            if now.saturating_sub(heartbeat) >= 250 {
                channel.send(&Status::Heartbeat)?;
                heartbeat = now;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

// Only an owned, actually reaped Child can supply exit observation. Public
// PID/status assertions never cross into this native archival path.
fn archive_receiver_exit(root: &Path, receiver: &mut std::process::Child) -> Result<(), String> {
    use serde_json::json;
    let status = receiver.try_wait().map_err(|_| "owner_receiver_unknown")?;
    authority::require(status.is_some_and(|status| status.success()))?;
    let (config, _) = super::receiver::read_json(&root.join("native-supervision.json"))
        .map_err(|_| "owner_outcome_unavailable")?;
    let (receipt, hash) = super::receiver::read_json(&root.join("receiver-outcome.json"))
        .map_err(|_| "owner_outcome_unavailable")?;
    let pid = u64::from(receiver.id());
    let positive = |key: &str| {
        receipt[key]
            .as_u64()
            .is_some_and(|n| n > 0 && n <= 9_007_199_254_740_991)
    };
    let digest = |key: &str| {
        receipt[key].as_str().is_some_and(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
    };
    authority::require(
        receipt["state"] == "complete"
            && receipt["authority"] == "native_witness_and_timely_supervisor_exit"
            && receipt["generation"] == config["generation"]
            && config["receiverPid"].as_u64() == Some(pid)
            && receipt["receiverPid"].as_u64() == Some(pid)
            && receipt["receiverGroup"] == config["receiverGroup"]
            && receipt["liveAuthority"] == false
            && receipt["controlOnly"] == false
            && receipt["requestedFieldSlots"].as_u64() == Some(216)
            && [
                "supervisorExitObservedAt",
                "acceptedAt",
                "deadline",
                "executionGroup",
                "supervisorGroup",
                "receiverGroup",
            ]
            .into_iter()
            .all(positive)
            && receipt["supervisorExitObservedAt"].as_u64() <= receipt["acceptedAt"].as_u64()
            && receipt["acceptedAt"].as_u64() < receipt["deadline"].as_u64()
            && receipt["executionGroup"] != receipt["receiverGroup"]
            && receipt["executionGroup"] != receipt["supervisorGroup"]
            && digest("resultSha256")
            && digest("evidenceSha256"),
    )?;
    let generation = config["generation"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .ok_or("owner_outcome_unavailable")?;
    let outcome = json!({"state":"complete","requestedFieldSlots":216,"liveAuthority":false,
        "receiverExit":0,"receiverSignal":null,"recovery":"no_staging_promotion_no_resume",
        "interruptedEffects":"unknown","generation":generation.to_string(),"receiverPid":pid,
        "receiverOutcomeSha256":super::receiver::digest_string(&hash),
        "authority":receipt["authority"],"actualReceiverExitObserved":true,
        "validatedFieldSlots":216,"unavailableFieldSlots":0});
    authority::storage::write_launcher_outcome(
        root,
        generation,
        &serde_json::to_vec(&outcome).map_err(|_| "owner_outcome_unavailable")?,
    )
    .map_err(|_| "owner_outcome_unavailable".into())
}

fn validate_disposable_custody(
    directory: &Path,
    proposal: &owner::ExecutionProposal,
    plan: &policy::RehearsalPlan,
) -> Result<Value, String> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("owner_workspace_invalid")?;
    let artifacts = workspace
        .join(".amp/in/artifacts")
        .canonicalize()
        .map_err(|_| "owner_custody_invalid")?;
    authority::require(
        proposal.execution_kind == owner::ExecutionKind::OfflineRehearsal
            && directory.is_absolute()
            && directory.canonicalize().ok().as_deref() == Some(directory)
            && directory.starts_with(&artifacts)
            && directory != artifacts,
    )?;
    // Read public configuration only. Never inspect arbitrary credential bytes.
    let (config, _) = super::receiver::read_json(&directory.join("prepared-bootstrap.json"))
        .map_err(|_| "owner_custody_invalid")?;
    let credential = Path::new(
        config["credentialPath"]
            .as_str()
            .ok_or("owner_custody_invalid")?,
    );
    let expected = offline_credential_binding(credential, &plan.model)?;
    let matches: Vec<_> = proposal
        .sources_and_tools
        .as_array()
        .ok_or("owner_sources_invalid")?
        .iter()
        .filter(|source| source["path"] == expected["path"])
        .collect();
    authority::require(matches.len() == 1 && matches[0] == &expected)?;
    Ok(expected)
}

/// Append this immutable public lock through the shared producer BEFORE both
/// proposal construction and the launched Node bootstrap. Does not read keys.
pub(crate) fn offline_credential_binding(credential: &Path, model: &str) -> Result<Value, String> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("owner_workspace_invalid")?;
    let artifacts = workspace
        .join(".amp/in/artifacts")
        .canonicalize()
        .map_err(|_| "owner_custody_invalid")?;
    authority::require(
        credential.file_name() == Some(std::ffi::OsStr::new(".env.local"))
            && credential.canonicalize().ok().as_deref() == Some(credential)
            && credential.starts_with(&artifacts),
    )?;
    Ok(
        serde_json::json!({"path":credential.strip_prefix(workspace).map_err(|_| "owner_custody_invalid")?.to_string_lossy(),
        "sha256":authority::sha(format!("LLM_API_KEY=MOCK_ONLY_SECRET_CANARY\nLLM_MODEL={model}\n").as_bytes())}),
    )
}

fn verify_sources(
    proposal: &owner::ExecutionProposal,
    plan_file: &Path,
    credential: &Value,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("owner_workspace_invalid")?;
    proposal.sha256()?;
    for source in proposal
        .sources_and_tools
        .as_array()
        .ok_or("owner_sources_invalid")?
    {
        if source["runtime"] == "nativeSupervision" {
            continue;
        }
        let relative = source["path"].as_str().ok_or("owner_sources_invalid")?;
        let path = workspace.join(relative);
        let response = path == plan_file.with_file_name("response-fixture.json");
        let wire = path == plan_file.with_file_name("response-wire.json");
        let limit = if wire {
            1024
        } else if response {
            65536
        } else {
            512 * 1024 * 1024
        };
        // Signing a path is not permission to inspect private workspace files.
        // These are the producer's public roots/slots, not a generic file reader.
        let public = !relative.split('/').any(|part| part.starts_with('.'))
            && ((relative.starts_with("src-tauri/src/") && relative.ends_with(".rs"))
                || (relative.starts_with("test/fixtures/tauri/") && relative.ends_with(".json"))
                || matches!(
                    relative,
                    "src-tauri/Cargo.toml"
                        | "src-tauri/Cargo.lock"
                        | "scripts/auto-tag-eval-replay/run-bounded.cjs"
                        | "scripts/auto-tag-eval-replay/run-supervised.cjs"
                        | "scripts/auto-tag-eval-replay/mock-authority.cjs"
                        | "scripts/auto-tag-eval-replay/bounded-storage.cjs"
                        | "scripts/auto-tag-eval-replay/score-observations.py"
                        | "scripts/auto-tag-eval-replay/audit-supervised.py"
                        | "scripts/mock-provider-service.cjs"
                        | "scripts/bounded_storage.py"
                        | "scripts/review_auto_tag_subset.py"
                ));
        let labels = [
            ".planning/plans/crowd-lu-user-acceptance-v1-20261111/",
            ".planning/plans/crowd-lu-annotation-sheet-20261111/",
            ".planning/plans/scorable-assistance-contract-20261111/",
        ]
        .iter()
        .any(|prefix| {
            relative.strip_prefix(prefix).is_some_and(|name| {
                !name.contains('/')
                    && !name.starts_with('.')
                    && (name == "SHA256SUMS"
                        || name.ends_with(".json")
                        || name.ends_with(".md")
                        || name.ends_with(".py"))
            })
        }) || matches!(
            relative,
            ".planning/research/bounded-assistance-machine-labels-20261111.json"
                | ".planning/research/bounded-assistance-label-review-20261111.json"
                | ".amp/in/eval-dummies/artist-batch-extension/corpus.json"
                | ".amp/in/eval-dummies/original/corpus.json"
        );
        authority::require(
            public
                || labels
                || path == plan_file
                || response
                || wire
                || source == credential
                || path == std::env::current_exe().map_err(|_| "owner_executable_unavailable")?,
        )?;
        authority::require(path.canonicalize().ok().as_deref() == Some(path.as_path()))?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|_| "owner_source_unavailable")?;
        authority::require(
            file.metadata()
                .is_ok_and(|m| m.is_file() && m.nlink() == 1 && m.len() <= limit),
        )?;
        let mut file = file.take(limit + 1);
        let mut digest = Sha256::new();
        let mut buffer = [0; 65536];
        let mut total = 0u64;
        loop {
            let n = file
                .read(&mut buffer)
                .map_err(|_| "owner_source_unavailable")?;
            if n == 0 {
                break;
            }
            total += n as u64;
            authority::require(total <= limit)?;
            digest.update(&buffer[..n]);
        }
        authority::require(source["sha256"] == format!("{:x}", digest.finalize()))?;
    }
    Ok(())
}

#[test]
#[ignore = "explicit offline native owner entry; no hosted execution"]
fn native_owner_process() {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Request {
        offline_custody: PathBuf,
        disposable_run_root: PathBuf,
        proposal: owner::ExecutionProposal,
        intent: owner::OwnerExecutionIntent,
        plan_file: PathBuf,
        bindings: Value,
    }
    let request = PathBuf::from(
        std::env::var_os("SOUNDROBE_OFFLINE_OWNER_REQUEST")
            .expect("explicit offline owner request"),
    );
    let (request, _) = super::receiver::read_json(&request).expect("bounded public owner request");
    let request: Request =
        serde_json::from_value(request).expect("typed public offline owner request");
    OfflineOwnerLaunch::prepare(
        &request.offline_custody,
        &request.disposable_run_root,
        request.proposal,
        request.intent,
        &request.plan_file,
        &request.bindings,
    )
    .and_then(OfflineOwnerLaunch::run)
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;

    #[test]
    fn archival_requires_observed_success_of_owned_child_before_evidence_reads() {
        let root = std::env::temp_dir().join(format!("owner-archive-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let mut child = std::process::Command::new("/bin/cat")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        assert!(child.try_wait().unwrap().is_none());
        assert_eq!(
            archive_receiver_exit(&root, &mut child).unwrap_err(),
            "mock_authority_denied"
        );
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        assert_eq!(
            archive_receiver_exit(&root, &mut child).unwrap_err(),
            "mock_authority_denied"
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn owner_channel_is_cloexec_and_peer_loss_is_observable() {
        let (owner, child) = UnixStream::pair().unwrap();
        unsafe { libc::fcntl(child.as_raw_fd(), libc::F_SETFD, 0) };
        let channel = private_channel(child).unwrap();
        assert_ne!(
            unsafe { libc::fcntl(channel.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
        drop(owner);
        assert!(channel.take_error().unwrap().is_none());
        let mut byte = [0];
        assert_eq!(std::io::Read::read(&mut &channel, &mut byte).unwrap(), 0);
    }

    #[test]
    fn slow_approval_work_does_not_block_native_heartbeats_and_cancels_on_loss() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut owner = StatusChannel::new(a).unwrap();
        let mut broker = StatusChannel::new(b).unwrap();
        let work = OwnerWork::start(|_, live| {
            std::thread::sleep(Duration::from_millis(1400));
            authority::require(live.load(Ordering::SeqCst))
        });
        work.request.send(uuid::Uuid::new_v4().to_string()).unwrap();
        let start = std::time::Instant::now();
        let mut seen = start;
        let mut beats = 0;
        while start.elapsed() < Duration::from_millis(1600) {
            owner.send(&Status::Heartbeat).unwrap();
            if matches!(broker.receive().unwrap(), Some(Status::Heartbeat)) {
                seen = std::time::Instant::now();
                beats += 1;
            }
            assert!(seen.elapsed() < Duration::from_millis(1000));
            if let Ok(result) = work.reply.try_recv() {
                result.unwrap();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(start.elapsed() >= Duration::from_millis(1400));
        assert!(beats > 20);
        let (done, result) = mpsc::sync_channel(1);
        let cancelled = OwnerWork::start(move |_, live| {
            std::thread::sleep(Duration::from_millis(80));
            done.send(live.load(Ordering::SeqCst)).unwrap();
            Ok(())
        });
        cancelled.request.send("native-run-id".into()).unwrap();
        drop(cancelled);
        assert!(!result.recv_timeout(Duration::from_secs(1)).unwrap());
    }

    fn probe(mode: &str) -> std::process::Command {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infra::mock_run_authority::supervisor::ownership::owner_launch::tests::native_owner_channel_probe",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("OWNER_PROBE_MODE", mode)
            .stdin(std::process::Stdio::null());
        command
    }

    #[test]
    fn inherited_channel_crosses_only_native_relays_and_intermediate_copies_close() {
        let (mut owner, child) = UnixStream::pair().unwrap();
        owner
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut command = probe("relay");
        command
            .stdout(std::process::Stdio::piped())
            .env(OWNER_FD, child.as_raw_fd().to_string());
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(child.as_raw_fd(), libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let relay = command.spawn().unwrap();
        drop(command);
        owner.write_all(b"p").unwrap();
        let mut reply = [0];
        owner.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"q");
        // Sink is deliberately still alive. EOF proves neither the relay nor
        // its exec payload retained the endpoint.
        let mut readiness = libc::pollfd {
            fd: owner.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut readiness, 1, 500) }, 1);
        assert_eq!(owner.read(&mut reply).unwrap(), 0);
        let result = relay.wait_with_output().unwrap();
        assert!(
            result.status.success() && String::from_utf8_lossy(&result.stdout).contains("1 passed")
        );
    }

    #[test]
    #[ignore = "private socket process probe, not an evaluator worker"]
    fn native_owner_channel_probe() {
        let mode = std::env::var("OWNER_PROBE_MODE").unwrap();
        if mode == "payload" {
            assert!(std::env::var_os(OWNER_FD).is_none());
            let fd: i32 = std::env::var("OWNER_PROBE_FD").unwrap().parse().unwrap();
            let mut kind: libc::c_int = 0;
            let mut length = std::mem::size_of_val(&kind) as libc::socklen_t;
            assert!(
                unsafe {
                    libc::getsockopt(
                        fd,
                        libc::SOL_SOCKET,
                        libc::SO_TYPE,
                        (&mut kind as *mut libc::c_int).cast(),
                        &mut length,
                    )
                } != 0
                    || kind != libc::SOCK_STREAM
            );
            return;
        }
        capture_inherited().unwrap();
        if mode == "relay" {
            for role in ["anchor", "node", "worker"] {
                let mut payload = probe("payload");
                payload.env("OWNER_PROBE_FD", std::env::var(OWNER_FD).unwrap());
                forward_command(&mut payload, role);
                let result = payload.output().unwrap();
                assert!(
                    result.status.success()
                        && String::from_utf8_lossy(&result.stdout).contains("1 passed")
                );
            }
            let mut command = probe("sink");
            command.stdout(std::process::Stdio::piped());
            forward_command(&mut command, "broker");
            let sink = command.spawn().unwrap();
            drop(command);
            assert!(INHERITED.lock().unwrap().is_none());
            let result = sink.wait_with_output().unwrap();
            assert!(
                result.status.success()
                    && String::from_utf8_lossy(&result.stdout).contains("1 passed")
            );
        } else {
            assert_eq!(mode, "sink");
            let mut stream = INHERITED.lock().unwrap().take().unwrap();
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            assert_eq!(&byte, b"p");
            stream.write_all(b"q").unwrap();
            drop(stream);
            std::thread::sleep(Duration::from_millis(800));
        }
    }

    #[test]
    fn offline_owner_claims_before_launch_and_refuses_reuse_hosted_and_wrong_root() {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        use serde_json::json;
        use std::os::unix::fs::PermissionsExt;
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let base = workspace
            .join(".amp/in/artifacts")
            .join(format!("owner-launch-fixture-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let custody = base.join("custody");
        std::fs::create_dir(&custody).unwrap();
        std::fs::set_permissions(&custody, std::fs::Permissions::from_mode(0o700)).unwrap();
        let private = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let signer = Ed25519KeyPair::from_pkcs8(private.as_ref()).unwrap();
        let plan: policy::RehearsalPlan = serde_json::from_value(json!({"nativePrepared":true,
            "model":"~deepseek/deepseek-v4-flash-latest","observedTarget":"deepseek/deepseek-v4-flash-0731",
            "route":"wafer/us","provider":"Wafer","input":{"artist":"Public Artist","album":"Public Album","tracks":["First","Other"]}})).unwrap();
        let plan_file = base.join("public-native-plan.json");
        std::fs::write(&plan_file, serde_json::to_vec(&plan).unwrap()).unwrap();
        let credential = base.join(".env.local");
        std::fs::write(
            &credential,
            format!(
                "LLM_API_KEY=MOCK_ONLY_SECRET_CANARY\nLLM_MODEL={}\n",
                plan.model
            ),
        )
        .unwrap();
        let config = serde_json::to_vec(&json!({"version":1,"planSha256":authority::sha(&serde_json::to_vec(&plan).unwrap()),
            "verifierSha256":authority::sha(signer.public_key().as_ref()),"credentialPath":credential,
            "pricing":{"promptPicoUsd":1,"completionPicoUsd":2,"catalogSha256":authority::sha(b"public catalog"),"termsSha256":authority::sha(b"public terms")}})).unwrap();
        for (name, bytes) in [
            ("approval-signing-key.pk8", private.as_ref()),
            ("approval-verifier.ed25519", signer.public_key().as_ref()),
            ("prepared-bootstrap.json", config.as_slice()),
        ] {
            let path = custody.join(name);
            std::fs::write(&path, bytes).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let root = base.join("run");
        let response_file = base.join("response-fixture.json");
        let response_bytes = b"public asymmetric response fixture";
        std::fs::write(&response_file, response_bytes).unwrap();
        let bindings = json!({"executable":owner::current_executable_sha256().unwrap(),"disposableRunRoot":root,
            "modelRehearsalPlan":plan,"sourcesAndTools":[offline_credential_binding(&credential,&plan.model).unwrap(),
            {"path":response_file.strip_prefix(workspace).unwrap().to_string_lossy(),"sha256":authority::sha(response_bytes)},
            {"path":root.join("native-supervision.json").strip_prefix(workspace).unwrap().to_string_lossy(),"runtime":"nativeSupervision"}]});
        let prepared = policy::boundary::PreparedBootstrap::read(&custody, &plan).unwrap();
        let proposal = owner::ExecutionProposal::construct(
            owner::ExecutionKind::OfflineRehearsal,
            &uuid::Uuid::new_v4().to_string(),
            &serde_json::from_str::<Value>(super::super::receiver::MEMBERSHIP).unwrap(),
            &bindings,
            &plan,
            &prepared.context,
            &authority::sha(signer.public_key().as_ref()),
        )
        .unwrap();
        let now = policy::now().unwrap();
        let intent = owner::sign_owner_intent(
            &custody,
            &proposal,
            &uuid::Uuid::new_v4().to_string(),
            now,
            now + 1200,
            false,
        )
        .unwrap();
        let mut hosted = proposal.clone();
        hosted.execution_kind = owner::ExecutionKind::HostedSingleRequest;
        assert!(OfflineOwnerLaunch::prepare(
            &custody,
            &root,
            hosted,
            intent.clone(),
            &plan_file,
            &bindings
        )
        .is_err());
        assert!(OfflineOwnerLaunch::prepare(
            &custody,
            &base.join("wrong"),
            proposal.clone(),
            intent.clone(),
            &plan_file,
            &bindings
        )
        .is_err());
        let other_plan = base.join("private-config.json");
        std::fs::write(&other_plan, serde_json::to_vec(&plan).unwrap()).unwrap();
        assert!(
            OfflineOwnerLaunch::prepare(
                &custody,
                &root,
                proposal.clone(),
                intent.clone(),
                &other_plan,
                &bindings
            )
            .is_err(),
            "only the declared public plan slot may be read before verification"
        );
        // A valid signature does not turn arbitrary workspace bytes into
        // public source inputs. Use benign private-shaped data, never real keys.
        let extra = base.join("private-fixture.pk8");
        std::fs::write(&extra, b"benign unapproved source input").unwrap();
        let mut extra_bindings = bindings.clone();
        extra_bindings["sourcesAndTools"]
            .as_array_mut()
            .unwrap()
            .push(json!({
            "path":extra.strip_prefix(workspace).unwrap().to_string_lossy(),
            "sha256":authority::sha(b"benign unapproved source input")}));
        let extra_proposal = owner::ExecutionProposal::construct(
            owner::ExecutionKind::OfflineRehearsal,
            &uuid::Uuid::new_v4().to_string(),
            &serde_json::from_str::<Value>(super::super::receiver::MEMBERSHIP).unwrap(),
            &extra_bindings,
            &plan,
            &prepared.context,
            &authority::sha(signer.public_key().as_ref()),
        )
        .unwrap();
        let extra_intent = owner::sign_owner_intent(
            &custody,
            &extra_proposal,
            &uuid::Uuid::new_v4().to_string(),
            now,
            now + 1200,
            false,
        )
        .unwrap();
        assert!(
            OfflineOwnerLaunch::prepare(
                &custody,
                &root,
                extra_proposal,
                extra_intent.clone(),
                &plan_file,
                &extra_bindings
            )
            .is_err(),
            "signed unapproved source must not be read or claimed"
        );
        assert!(!custody
            .join(format!("intent-{}.claimed.json", extra_intent.nonce))
            .exists());
        let launch = OfflineOwnerLaunch::prepare(
            &custody,
            &root,
            proposal.clone(),
            intent.clone(),
            &plan_file,
            &bindings,
        )
        .unwrap();
        assert!(custody
            .join(format!("intent-{}.claimed.json", intent.nonce))
            .is_file());
        assert!(!root.exists(), "claim must precede receiver launch");
        assert!(!custody.join("prepared-approval.json").exists());
        assert!(OfflineOwnerLaunch::prepare(
            &custody, &root, proposal, intent, &plan_file, &bindings
        )
        .is_err());
        assert_eq!(
            launch.bootstrap.claimed_binding()["executionKind"],
            "offlineRehearsal"
        );
        let public = launch.bootstrap.public_bootstrap(&root, &plan).unwrap();
        assert_eq!(public["ownerExecution"], launch.bootstrap.claimed_binding());
        assert_eq!(
            public["offlineCredential"],
            offline_credential_binding(&credential, &plan.model).unwrap()
        );
        assert_eq!(public.as_object().unwrap().len(), 5);
        assert!(public.get("offlineCustody").is_none());
        assert_eq!(
            launch.bootstrap.response_fixture(&plan_file).unwrap(),
            Some(response_bytes.to_vec())
        );
        assert!(launch.bootstrap.response_fixture(&other_plan).is_err());
        std::fs::write(&response_file, b"substituted response").unwrap();
        assert!(launch.bootstrap.response_fixture(&plan_file).is_err());
        std::fs::write(&response_file, response_bytes).unwrap();
        let alias = base.join("response-alias.json");
        std::fs::hard_link(&response_file, &alias).unwrap();
        assert!(launch.bootstrap.response_fixture(&plan_file).is_err());
        std::fs::remove_file(&alias).unwrap();
        let mut no_response = launch.bootstrap.clone();
        no_response
            .proposal
            .sources_and_tools
            .as_array_mut()
            .unwrap()
            .retain(|s| {
                s["path"]
                    != response_file
                        .strip_prefix(workspace)
                        .unwrap()
                        .to_string_lossy()
                        .as_ref()
            });
        assert_eq!(no_response.response_fixture(&plan_file).unwrap(), None);
        let wire_file = base.join("response-wire.json");
        let wire_bytes = br#"{"version":1,"truncateAfterBytes":1}"#;
        std::fs::write(&wire_file, wire_bytes).unwrap();
        assert_eq!(
            launch
                .bootstrap
                .response_wire(&plan_file, response_bytes.len())
                .unwrap(),
            None
        );
        let mut wire_bootstrap = launch.bootstrap.clone();
        wire_bootstrap
            .proposal
            .sources_and_tools
            .as_array_mut()
            .unwrap()
            .push(json!({
            "path":wire_file.strip_prefix(workspace).unwrap().to_string_lossy(),
            "sha256":authority::sha(wire_bytes)}));
        assert_eq!(
            wire_bootstrap
                .response_wire(&plan_file, response_bytes.len())
                .unwrap(),
            Some(1)
        );
        assert!(wire_bootstrap
            .response_wire(&other_plan, response_bytes.len())
            .is_err());
        for bad in [
            json!({"version":1,"truncateAfterBytes":0}),
            json!({"version":1,"truncateAfterBytes":response_bytes.len()}),
            json!({"version":1,"truncateAfterBytes":true}),
            json!({"version":1,"truncateAfterBytes":1,"endpoint":"https://invalid.example"}),
        ] {
            let bytes = serde_json::to_vec(&bad).unwrap();
            std::fs::write(&wire_file, &bytes).unwrap();
            let last = wire_bootstrap
                .proposal
                .sources_and_tools
                .as_array_mut()
                .unwrap()
                .last_mut()
                .unwrap();
            last["sha256"] = json!(authority::sha(&bytes));
            assert!(wire_bootstrap
                .response_wire(&plan_file, response_bytes.len())
                .is_err());
        }
        std::fs::write(&wire_file, wire_bytes).unwrap();
        assert!(
            wire_bootstrap
                .response_wire(&plan_file, response_bytes.len())
                .is_err(),
            "changed wire bytes accepted"
        );
        wire_bootstrap
            .proposal
            .sources_and_tools
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["sha256"] = json!(authority::sha(wire_bytes));
        let wire_alias = base.join("wire-alias.json");
        std::fs::hard_link(&wire_file, &wire_alias).unwrap();
        assert!(wire_bootstrap
            .response_wire(&plan_file, response_bytes.len())
            .is_err());
        std::fs::remove_file(&wire_alias).unwrap();
        std::fs::remove_file(&wire_file).unwrap();
        assert!(wire_bootstrap
            .response_wire(&plan_file, response_bytes.len())
            .is_err());
        let oversized = vec![b'x'; 65537];
        std::fs::write(&response_file, &oversized).unwrap();
        let mut oversized_bootstrap = launch.bootstrap.clone();
        let response_lock = oversized_bootstrap
            .proposal
            .sources_and_tools
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|s| {
                s["path"]
                    == response_file
                        .strip_prefix(workspace)
                        .unwrap()
                        .to_string_lossy()
                        .as_ref()
            })
            .unwrap();
        response_lock["sha256"] = json!(authority::sha(&oversized));
        assert!(
            oversized_bootstrap.response_fixture(&plan_file).is_err(),
            "even matching oversized bytes must fail"
        );
        std::fs::write(&response_file, response_bytes).unwrap();
        let mut wrong_kind = launch.bootstrap.clone();
        wrong_kind.execution_kind = owner::ExecutionKind::HostedSingleRequest;
        assert!(wrong_kind.prepared(&root, &plan).is_err());
        let (frozen, _receiver) = mpsc::sync_channel(1);
        let link = BrokerOwnerLink {
            frozen,
            announced: AtomicBool::new(false),
            approved: Arc::new(AtomicBool::new(false)),
        };
        let mut permit = link
            .offline_dispatch(&launch.bootstrap, &root, &plan)
            .unwrap();
        assert!(
            permit.admit(&prepared.context).is_err(),
            "unpublished approval"
        );
        link.approved.store(true, Ordering::SeqCst);
        permit
            .admit(&prepared.context)
            .expect("published owner offline dispatch");
        let mut drifted = launch.bootstrap.prepared(&root, &plan).unwrap().context;
        drifted.model = "~deepseek/different-alias".into();
        assert!(permit.admit(&drifted).is_err(), "changed prepared context");
        assert!(link.offline_dispatch(&wrong_kind, &root, &plan).is_err());
        assert!(link
            .offline_dispatch(&launch.bootstrap, &base.join("other"), &plan)
            .is_err());
        permit.expires = 0;
        assert!(
            permit.admit(&prepared.context).is_err(),
            "expired owner intent"
        );
        drop(launch);
        std::fs::remove_dir_all(base).unwrap();
    }
}
