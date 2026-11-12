//! Native completion authority for the explicitly selected mock-only runner.
use super::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::atomic::AtomicU8;

pub(crate) const MEMBERSHIP: &str =
    include_str!("../../../test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json");
const FRAME: usize = 89;
const PENDING: u8 = 0;
const CANCELLED: u8 = 1;
const ACCEPTED: u8 = 2;

pub(crate) fn fault_barrier(root: &Path, control: &str) {
    if std::env::var("SOUNDROBE_NATIVE_CONTROL").as_deref() == Ok(control) {
        std::fs::write(
            root.join(format!("{control}.reached")),
            b"at selected native boundary",
        )
        .unwrap();
        loop {
            std::thread::park();
        }
    }
}

pub(crate) fn durable_json(root: &Path, name: &str, value: &Value) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 1024 * 1024 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let temp = root.join(format!("{name}.tmp"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    std::fs::rename(temp, root.join(name))?;
    if name == "provisional.json" {
        fault_barrier(root.parent().unwrap(), "final-provisional-rename");
    } else if name == "receiver-outcome.json" {
        fault_barrier(root, "receiver-record-renamed");
    }
    File::open(root)?.sync_all()
}

pub(crate) fn read_json(file: &Path) -> std::io::Result<(Value, [u8; 32])> {
    let mut bytes = Vec::new();
    File::open(file)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    Ok((
        serde_json::from_slice(&bytes)?,
        Sha256::digest(&bytes).into(),
    ))
}

pub(crate) fn digest_string(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone, Copy)]
struct Witness {
    op: u8,
    generation: [u8; 16],
    evidence: [u8; 32],
    result: [u8; 32],
    deadline: u64,
}
impl Witness {
    fn send(self, stream: &mut UnixStream) -> std::io::Result<()> {
        let mut frame = [0; FRAME];
        frame[0] = self.op;
        frame[1..17].copy_from_slice(&self.generation);
        frame[17..49].copy_from_slice(&self.evidence);
        frame[49..81].copy_from_slice(&self.result);
        frame[81..].copy_from_slice(&self.deadline.to_le_bytes());
        // One bounded nonblocking write. A short/failed write is not a witness.
        if stream.write(&frame)? != FRAME {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        Ok(())
    }
    fn decode(frame: &[u8; FRAME]) -> Self {
        Self {
            op: frame[0],
            generation: frame[1..17].try_into().unwrap(),
            evidence: frame[17..49].try_into().unwrap(),
            result: frame[49..81].try_into().unwrap(),
            deadline: u64::from_le_bytes(frame[81..].try_into().unwrap()),
        }
    }
}

pub(super) struct SupervisorLink {
    channel: Channel,
    generation: [u8; 16],
    seen: u64,
    heartbeat: u64,
}
impl SupervisorLink {
    pub(super) fn inherited() -> Option<Self> {
        std::env::var_os("SOUNDROBE_NATIVE_RECEIVER").map(|_| Self {
            channel: Channel::new(super::inherited("SOUNDROBE_NATIVE_RECEIVER")).unwrap(),
            generation: *uuid::Uuid::parse_str(
                &std::env::var("SOUNDROBE_NATIVE_GENERATION").unwrap(),
            )
            .unwrap()
            .as_bytes(),
            seen: monotonic_ms(),
            heartbeat: 0,
        })
    }
    pub(super) fn healthy(&mut self) -> bool {
        match self.channel.receive() {
            Ok(Some((1, 0))) => self.seen = monotonic_ms(),
            Ok(None) => {}
            _ => return false,
        }
        let now = monotonic_ms();
        if now.saturating_sub(self.seen) >= 1000 {
            return false;
        }
        if now.saturating_sub(self.heartbeat) >= 250 {
            self.heartbeat = now;
            if (Witness {
                op: 1,
                generation: self.generation,
                evidence: [0; 32],
                result: [0; 32],
                deadline: 0,
            })
            .send(&mut self.channel.stream)
            .is_err()
            {
                return false;
            }
        }
        true
    }
    pub(super) fn phase(&mut self, deadline: u64) -> std::io::Result<()> {
        Witness {
            op: 2,
            generation: self.generation,
            evidence: [0; 32],
            result: [0; 32],
            deadline,
        }
        .send(&mut self.channel.stream)
    }
    pub(super) fn complete(
        &mut self,
        evidence: [u8; 32],
        result: [u8; 32],
        deadline: u64,
    ) -> std::io::Result<()> {
        Witness {
            op: 3,
            generation: self.generation,
            evidence,
            result,
            deadline,
        }
        .send(&mut self.channel.stream)
    }
}

enum Work {
    Verify(Witness),
    Record(Value),
}
enum Reply {
    Verified(Option<i32>),
    Recorded,
}
struct OwnedReceiver {
    child: Child,
    channel: Channel,
    work: mpsc::SyncSender<Work>,
    replies: mpsc::Receiver<Reply>,
}

fn bootstrap_receiver(
    root: PathBuf,
    start: u64,
    preparation: u64,
    generation: uuid::Uuid,
    control: String,
    terminal: Arc<AtomicU8>,
    ready: mpsc::SyncSender<std::io::Result<OwnedReceiver>>,
) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let anchor = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".amp/in/artifacts")
        .canonicalize()?;
    if !root
        .parent()
        .ok_or(std::io::ErrorKind::InvalidInput)?
        .canonicalize()?
        .starts_with(&anchor)
    {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    std::fs::create_dir(&root)?;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
    durable_json(
        &root,
        "requested-membership.json",
        &serde_json::from_str(MEMBERSHIP)?,
    )?;
    durable_json(
        &root,
        "native-supervision.json",
        &json!({"generation":generation.to_string(),"start":start,
        "preparationMs":preparation,"executionMs":number("SOUNDROBE_NATIVE_EXECUTION_MS",600000),
        "receiverPid":std::process::id(),"receiverGroup":unsafe{libc::getpgrp()},"control":control}),
    )?;
    let (receiver, supervisor) = UnixStream::pair()?;
    let role = if control.starts_with("receiver-") {
        "receiver-sender-control"
    } else {
        "supervisor"
    };
    let spawned = child(
        role,
        &root,
        start,
        preparation,
        &[("SOUNDROBE_NATIVE_RECEIVER", &supervisor)],
    )
    .env("SOUNDROBE_NATIVE_GENERATION", generation.to_string())
    .env(
        "SOUNDROBE_NATIVE_MANAGED",
        if role == "supervisor" { "1" } else { "0" },
    )
    .process_group(0)
    .spawn()?;
    drop(supervisor);
    let supervisor_group = spawned.id() as i32;
    durable_json(
        &root,
        "supervisor-identity.json",
        &json!({"pid":spawned.id(),"group":spawned.id()}),
    )?;
    let (work, tasks) = mpsc::sync_channel(1);
    let (replies, messages) = mpsc::sync_channel(1);
    ready
        .send(Ok(OwnedReceiver {
            child: spawned,
            channel: Channel::new(receiver)?,
            work,
            replies: messages,
        }))
        .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
    while let Ok(task) = tasks.recv() {
        match task {
            Work::Verify(witness) => {
                fault_barrier(&root, "receiver-verify-blocked");
                let (evidence, hash) = read_json(&root.join("authority/provisional.json"))?;
                let (result, result_hash) = read_json(&root.join("results.json"))?;
                if hash != witness.evidence
                    || result_hash != witness.result
                    || evidence["resultSha256"] != digest_string(&result_hash)
                    || evidence["state"] != "provisional_complete"
                    || evidence["ownerLive"] != false
                    || evidence["bindings"]["supervision"]["generation"] != generation.to_string()
                    || evidence["membership"] != serde_json::from_str::<Value>(MEMBERSHIP)?
                    || result["requestedFieldSlots"] != 216
                {
                    return Err(std::io::ErrorKind::InvalidData.into());
                }
                let group = if control.starts_with("receiver-") {
                    None
                } else {
                    let group = evidence["bindings"]["supervision"]["executionGroup"]
                        .as_i64()
                        .and_then(|id| i32::try_from(id).ok())
                        .filter(|id| *id > 1)
                        .ok_or(std::io::ErrorKind::InvalidData)?;
                    if group == supervisor_group || group == unsafe { libc::getpgrp() } {
                        return Err(std::io::ErrorKind::InvalidData.into());
                    }
                    Some(group)
                };
                replies
                    .send(Reply::Verified(group))
                    .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
            }
            Work::Record(value) => {
                if control == "receiver-cancel-after" {
                    assert_eq!(
                        terminal.compare_exchange(
                            PENDING,
                            CANCELLED,
                            Ordering::SeqCst,
                            Ordering::SeqCst
                        ),
                        Err(ACCEPTED)
                    );
                }
                if control == "receiver-lost-recording" {
                    loop {
                        std::thread::park();
                    }
                }
                durable_json(&root, "receiver-outcome.json", &value)?;
                replies
                    .send(Reply::Recorded)
                    .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
            }
        }
    }
    Ok(())
}

pub(super) fn run(root: PathBuf, mut scope: Scope) -> ! {
    let generation = uuid::Uuid::new_v4();
    let control = std::env::var("SOUNDROBE_NATIVE_CONTROL").unwrap_or_default();
    let terminal = Arc::new(AtomicU8::new(PENDING));
    let (tx, rx) = mpsc::sync_channel(1);
    let worker_terminal = terminal.clone();
    let worker_control = control.clone();
    let worker_root = root.clone();
    let start = number(
        "SOUNDROBE_NATIVE_START",
        scope.deadline - number("SOUNDROBE_NATIVE_PREPARATION_MS", 300000),
    );
    std::thread::spawn(move || {
        let error = tx.clone();
        if let Err(e) = bootstrap_receiver(
            worker_root,
            start,
            number("SOUNDROBE_NATIVE_PREPARATION_MS", 300000),
            generation,
            worker_control,
            worker_terminal,
            tx,
        ) {
            let _ = error.send(Err(e));
        }
    });
    let mut owned = loop {
        if scope.clip(monotonic_ms(), 1).is_err() {
            unsafe { libc::_exit(78) }
        }
        match rx.try_recv() {
            Ok(Ok(owned)) => break owned,
            Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => unsafe { libc::_exit(78) },
            Err(mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    let mut frame = [0; FRAME];
    let mut used = 0;
    let mut witness = None;
    let mut verified = false;
    let mut execution_group = None;
    let mut exited = None;
    let mut closed = false;
    let mut heartbeat = 0;
    let mut supervisor_seen = monotonic_ms();
    let mut recording = false;
    let mut stop_sent = false;
    loop {
        let now = monotonic_ms();
        if !recording && scope.clip(now, 1).is_err() {
            let _ =
                terminal.compare_exchange(PENDING, CANCELLED, Ordering::SeqCst, Ordering::SeqCst);
        }
        if !recording && exited.is_none() && now.saturating_sub(supervisor_seen) >= 1000 {
            let _ =
                terminal.compare_exchange(PENDING, CANCELLED, Ordering::SeqCst, Ordering::SeqCst);
        }
        if !closed && !recording && terminal.load(Ordering::SeqCst) == PENDING {
            match owned.channel.stream.read(&mut frame[used..]) {
                Ok(0) => {
                    closed = true;
                    if used != 0 || witness.is_none() {
                        terminal.store(CANCELLED, Ordering::SeqCst);
                    }
                }
                Ok(n) => {
                    used += n;
                    if used == FRAME {
                        used = 0;
                        let received = Witness::decode(&frame);
                        let valid_generation = received.generation == *generation.as_bytes();
                        match received.op {
                            1 if valid_generation
                                && witness.is_none()
                                && received.deadline == 0
                                && received.evidence == [0; 32]
                                && received.result == [0; 32] =>
                            {
                                supervisor_seen = now;
                            }
                            2 if valid_generation
                                && !scope.executing
                                && received.evidence == [0; 32]
                                && received.result == [0; 32]
                                && received.deadline > now
                                && received.deadline <= now.saturating_add(scope.execution_ms) =>
                            {
                                supervisor_seen = now;
                                if scope.execute(monotonic_ms()).is_err() {
                                    terminal.store(CANCELLED, Ordering::SeqCst);
                                }
                                scope.deadline = scope.deadline.min(received.deadline);
                            }
                            3 if valid_generation
                                && scope.executing
                                && witness.is_none()
                                && received.deadline == scope.deadline =>
                            {
                                supervisor_seen = now;
                                witness = Some(received);
                                if control == "receiver-lost-receiver" {
                                    kill_control_owner();
                                }
                                if owned.work.try_send(Work::Verify(received)).is_err() {
                                    terminal.store(CANCELLED, Ordering::SeqCst);
                                }
                            }
                            _ => terminal.store(CANCELLED, Ordering::SeqCst),
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => terminal.store(CANCELLED, Ordering::SeqCst),
            }
        }
        if now.saturating_sub(heartbeat) >= 250 && !closed && !recording {
            heartbeat = now;
            // A successful sender may already have exited with a complete
            // witness buffered. Read-side framing and actual wait decide it.
            let _ = owned.channel.send(1, 0);
        }
        match owned.replies.try_recv() {
            Ok(Reply::Verified(group)) => {
                verified = true;
                execution_group = group;
            }
            Ok(Reply::Recorded) if recording => unsafe {
                // Recording is conservatively abandoned at the original bound;
                // it never receives another work scope after live acceptance.
                libc::_exit(if monotonic_ms() < scope.deadline {
                    0
                } else {
                    78
                })
            },
            Ok(_) | Err(mpsc::TryRecvError::Disconnected) => {
                let _ = terminal.compare_exchange(
                    PENDING,
                    CANCELLED,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if exited.is_none()
            && (control != "receiver-unknown-exit" || terminal.load(Ordering::SeqCst) == CANCELLED)
        {
            match owned.child.try_wait() {
                Ok(Some(status)) => {
                    exited = Some(monotonic_ms());
                    if !status.success() {
                        terminal.store(CANCELLED, Ordering::SeqCst);
                    }
                }
                Ok(None) => {}
                Err(_) => terminal.store(CANCELLED, Ordering::SeqCst),
            }
        }
        if let Some(witness) =
            witness.filter(|_| !recording && closed && used == 0 && verified && exited.is_some())
        {
            // Only this native receiver arbitrates completion. Its clock sample
            // follows actual wait/reap evidence, not the sender's final check.
            if execution_group.is_some_and(|group| !group_absent(group)) {
                terminal.store(CANCELLED, Ordering::SeqCst);
            }
            let accepted_at = monotonic_ms();
            if control == "receiver-acceptance-gap" {
                std::thread::sleep(Duration::from_millis(scope.execution_ms + 40));
            }
            if accepted_at < scope.deadline
                && terminal
                    .compare_exchange(PENDING, ACCEPTED, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                // This post-CAS sample establishes that the winning transition
                // itself occurred before the deadline, even across suspension.
                if control == "receiver-post-cas-gap" {
                    std::fs::write(
                        root.join("receiver-post-cas-gap.reached"),
                        b"accepted CAS before clock sample",
                    )
                    .unwrap();
                    std::thread::sleep(Duration::from_millis(scope.execution_ms + 40));
                }
                let accepted_at = monotonic_ms();
                if accepted_at >= scope.deadline {
                    unsafe { libc::_exit(78) }
                }
                let receipt = json!({"state":"complete","authority":"native_witness_and_timely_supervisor_exit",
                    "generation":generation.to_string(),"acceptedAt":accepted_at,"supervisorExitObservedAt":exited,
                    "deadline":scope.deadline,"evidenceSha256":digest_string(&witness.evidence),"resultSha256":digest_string(&witness.result),
                    "requestedFieldSlots":216,"receiverPid":std::process::id(),"receiverGroup":unsafe{libc::getpgrp()},"supervisorGroup":owned.child.id(),
                    "executionGroup":execution_group,"controlOnly":!control.is_empty(),
                    "liveAuthority":false,"recovery":"recorded_outcome_only_no_staging_promotion"});
                if owned.work.try_send(Work::Record(receipt)).is_err() {
                    unsafe { libc::_exit(78) }
                }
                recording = true;
            } else {
                let _ = terminal.compare_exchange(
                    PENDING,
                    CANCELLED,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
            }
        }
        if terminal.load(Ordering::SeqCst) == CANCELLED {
            scope.stop(monotonic_ms());
            if !stop_sent {
                stop_sent = true;
                let _ = owned.channel.send(4, 0);
                let _ = owned.channel.stream.shutdown(std::net::Shutdown::Both);
                // Direct, unreaped control-child ownership is lifetime-safe.
                // The surviving broker/anchor handle payload containment; this
                // is never signalling a stored execution-group number.
                if exited.is_none() {
                    let _ = owned.child.kill();
                }
            }
            if exited.is_some() {
                unsafe { libc::_exit(78) }
            }
            if scope.grace_remaining(monotonic_ms()) == 0 {
                unsafe { libc::_exit(78) }
            }
        }
        // Recording never grants a new scope. Losing the original remaining
        // time leaves no recoverable success, even after live acceptance.
        if recording && monotonic_ms() >= scope.deadline {
            unsafe { libc::_exit(78) }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub(super) fn sender_control(root: PathBuf, scope: Scope) -> ! {
    let mut link = SupervisorLink::inherited().unwrap();
    let control = std::env::var("SOUNDROBE_NATIVE_CONTROL").unwrap();
    let generation = uuid::Uuid::from_bytes(link.generation);
    std::fs::create_dir(root.join("authority")).unwrap();
    durable_json(&root, "results.json", &json!({"requestedFieldSlots":216})).unwrap();
    let (_, result) = read_json(&root.join("results.json")).unwrap();
    durable_json(
        &root.join("authority"),
        "provisional.json",
        &json!({"state":"provisional_complete","ownerLive":false,
        "resultSha256":digest_string(&result),"bindings":{"supervision":{"generation":generation.to_string()}},
        "membership":serde_json::from_str::<Value>(MEMBERSHIP).unwrap()}),
    )
    .unwrap();
    let (_, evidence) = read_json(&root.join("authority/provisional.json")).unwrap();
    let deadline = monotonic_ms() + scope.execution_ms;
    link.phase(deadline).unwrap();
    let mut witness = Witness {
        op: 3,
        generation: link.generation,
        evidence,
        result,
        deadline,
    };
    if control == "receiver-cancel-before" {
        Witness { op: 4, ..witness }
            .send(&mut link.channel.stream)
            .unwrap();
    }
    if control == "receiver-late-witness" {
        std::thread::sleep(Duration::from_millis(scope.execution_ms + 40));
    }
    if control == "receiver-rebound" {
        witness.generation = [0; 16];
    }
    if control == "receiver-malformed" {
        witness.op = 99;
    }
    if control == "receiver-fragmented" {
        let mut frame = vec![witness.op];
        frame.extend(witness.generation);
        frame.extend(witness.evidence);
        frame.extend(witness.result);
        frame.extend(witness.deadline.to_le_bytes());
        link.channel.stream.write_all(&frame[..17]).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        link.channel.stream.write_all(&frame[17..]).unwrap();
    } else if control != "receiver-missing" {
        let sent = witness.send(&mut link.channel.stream);
        if control == "receiver-suspended-after-witness" {
            sent.expect("complete witness before suspension");
        }
    }
    match control.as_str() {
        "receiver-trailing-byte" => {
            let _ = link.channel.stream.write(&[3]);
        }
        "receiver-duplicate" => {
            let _ = witness.send(&mut link.channel.stream);
        }
        "receiver-trailing-cancel" => {
            let _ = Witness { op: 4, ..witness }.send(&mut link.channel.stream);
        }
        _ => {}
    }
    if control == "receiver-late-exit" {
        std::thread::sleep(Duration::from_millis(scope.execution_ms + 60));
    }
    if control == "receiver-unknown-exit" {
        std::thread::sleep(Duration::from_millis(scope.execution_ms + 80));
    }
    if control == "receiver-suspended-after-witness" {
        std::fs::write(
            root.join("witness-sent-before-suspension"),
            b"full witness sent",
        )
        .unwrap();
        unsafe {
            libc::raise(libc::SIGSTOP);
        }
        unsafe { libc::_exit(79) }
    }
    unsafe { libc::_exit(if control == "receiver-nonzero" { 79 } else { 0 }) }
}
