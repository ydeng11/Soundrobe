//! Process-level wiring for the explicitly selected, isolated mock runner.

#[path = "mock_run_receiver.rs"]
pub(crate) mod receiver;

use super::{group_absent, monotonic_ms, run_anchor, Scope};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::{net::UnixStream, process::CommandExt};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, OnceLock,
};
use std::time::Duration;

const ENTRY: &str = "infra::mock_run_authority::supervisor::ownership::native_supervision_process";

pub(crate) struct Monitor {
    pub(crate) stopped: Arc<AtomicBool>,
    control: String,
    managed: bool,
    staged: AtomicBool,
    payload_exited: AtomicBool,
    quiescent: AtomicBool,
    candidate: OnceLock<([u8; 32], [u8; 32])>,
    execution_requested: AtomicBool,
    executing: AtomicBool,
    deadline: AtomicU64,
}
static BROKER: OnceLock<Arc<Monitor>> = OnceLock::new();

pub(crate) fn broker_monitor() -> Option<&'static Arc<Monitor>> {
    BROKER.get()
}

impl Monitor {
    pub(crate) fn stage(&self) {
        self.staged.store(true, Ordering::SeqCst);
    }

    pub(crate) fn payload_exited(&self) -> bool {
        self.payload_exited.load(Ordering::SeqCst)
    }

    pub(crate) fn remaining(&self) -> Result<u64, String> {
        self.check()?;
        Ok(self
            .deadline
            .load(Ordering::SeqCst)
            .saturating_sub(monotonic_ms()))
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        if monotonic_ms() >= self.deadline.load(Ordering::SeqCst) {
            self.stopped.store(true, Ordering::SeqCst);
        }
        if self.stopped.load(Ordering::SeqCst) {
            Err("mock_supervision_stopped".into())
        } else {
            Ok(())
        }
    }

    pub(crate) fn enter_execution(&self) -> Result<(), String> {
        self.check()?;
        if self.execution_requested.swap(true, Ordering::SeqCst) {
            self.stopped.store(true, Ordering::SeqCst);
            return Err("mock_supervision_duplicate_transition".into());
        }
        while !self.executing.load(Ordering::SeqCst) {
            self.check()?;
            std::thread::sleep(Duration::from_millis(1));
        }
        self.check()
    }
}

struct Channel {
    stream: UnixStream,
    bytes: [u8; 9],
    used: usize,
}
impl Channel {
    fn new(stream: UnixStream) -> std::io::Result<Self> {
        stream.set_nonblocking(true)?;
        let fd = stream.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self {
            stream,
            bytes: [0; 9],
            used: 0,
        })
    }
    fn send(&mut self, op: u8, value: u64) -> std::io::Result<()> {
        self.stream
            .write_all(&[&[op], value.to_le_bytes().as_slice()].concat())
    }
    fn receive(&mut self) -> std::io::Result<Option<(u8, u64)>> {
        match self.stream.read(&mut self.bytes[self.used..]) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => self.used += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
            Err(e) => return Err(e),
        }
        if self.used != 9 {
            return Ok(None);
        }
        self.used = 0;
        Ok(Some((
            self.bytes[0],
            u64::from_le_bytes(self.bytes[1..].try_into().unwrap()),
        )))
    }
}

fn number(key: &str, default: u64) -> u64 {
    std::env::var(key).map_or(default, |v| {
        assert!(v.len() <= 20, "invalid native clock field");
        v.parse().expect("invalid native clock field")
    })
}

fn inherited(key: &str) -> UnixStream {
    let fd = number(key, u64::MAX);
    assert!(
        (3..=i32::MAX as u64).contains(&fd),
        "private native channel required"
    );
    unsafe { UnixStream::from_raw_fd(fd as i32) }
}

fn child(
    role: &str,
    root: &std::path::Path,
    start: u64,
    preparation: u64,
    fds: &[(&str, &UnixStream)],
) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", ENTRY, "--ignored", "--nocapture"])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("SOUNDROBE_NATIVE_ROLE", role)
        .env("SOUNDROBE_NATIVE_OUTPUT", root)
        .env("SOUNDROBE_NATIVE_START", start.to_string())
        .env("SOUNDROBE_NATIVE_PREPARATION_MS", preparation.to_string())
        .env(
            "SOUNDROBE_NATIVE_EXECUTION_MS",
            number("SOUNDROBE_NATIVE_EXECUTION_MS", 600_000).to_string(),
        )
        .env(
            "SOUNDROBE_NATIVE_CONTROL",
            std::env::var("SOUNDROBE_NATIVE_CONTROL").unwrap_or_default(),
        )
        .stdin(Stdio::null());
    for key in [
        "SOUNDROBE_NATIVE_MANAGED",
        "SOUNDROBE_NATIVE_GENERATION",
        "SOUNDROBE_STORAGE_REQUIRED",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let raw: Vec<_> = fds
        .iter()
        .map(|(key, stream)| {
            let fd = stream.as_raw_fd();
            command.env(key, fd.to_string());
            fd
        })
        .collect();
    unsafe {
        command.pre_exec(move || {
            for fd in &raw {
                let flags = libc::fcntl(*fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(*fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    command
}

struct Owned {
    anchor: Child,
    broker: Child,
    peer: Channel,
    group: Channel,
}

fn kill_control_owner() -> ! {
    if unsafe { libc::kill(std::process::id() as i32, libc::SIGKILL) } == 0 {
        std::thread::sleep(Duration::from_millis(100));
    }
    // Signal submission is not signal delivery; the parent checks SIGKILL.
    unsafe {
        libc::_exit(79);
    }
}

fn bootstrap(
    root: PathBuf,
    start: u64,
    preparation: u64,
    fault: Arc<AtomicBool>,
    monitor: Arc<Monitor>,
) -> std::io::Result<Owned> {
    // All potentially blocking bootstrap I/O and process creation belongs to
    // this worker. The calling native thread has already started its clock.
    crate::infra::mock_run_authority::storage::attach_from_env()?;
    crate::infra::mock_run_authority::storage::write_new_or_unscoped(
        root.join("bootstrap-entered"),
        b"native preparation scope",
    )?;
    if std::env::var("SOUNDROBE_NATIVE_CONTROL").as_deref() == Ok("blocked-bootstrap") {
        loop {
            std::thread::park();
        }
    }
    let (peer_s, peer_b) = UnixStream::pair()?;
    let (group_s, anchor_s) = UnixStream::pair()?;
    let (group_b, anchor_b) = UnixStream::pair()?;
    let anchor = child(
        "anchor",
        &root,
        start,
        preparation,
        &[
            ("SOUNDROBE_NATIVE_ANCHOR_S", &anchor_s),
            ("SOUNDROBE_NATIVE_ANCHOR_B", &anchor_b),
        ],
    )
    .process_group(0)
    .spawn()?;
    drop(anchor_s);
    drop(anchor_b);
    let broker = child(
        "broker",
        &root,
        start,
        preparation,
        &[
            ("SOUNDROBE_NATIVE_PEER", &peer_b),
            ("SOUNDROBE_NATIVE_GROUP_CHANNEL", &group_b),
        ],
    )
    .env("SOUNDROBE_NATIVE_GROUP", anchor.id().to_string())
    .spawn()?;
    drop(peer_b);
    drop(group_b);
    receiver::durable_json(
        &root,
        "group-identity.json",
        &serde_json::json!({
            "group":anchor.id(),"anchorPid":anchor.id(),"supervisorPid":std::process::id(),"brokerPid":broker.id()
        }),
    )?;
    let watched = root.clone();
    std::thread::spawn(move || {
        let mut requested = false;
        let mut phase_written = false;
        while !monitor.stopped.load(Ordering::SeqCst) {
            if !requested
                && crate::infra::mock_run_authority::storage::files::metadata(
                    watched.join("fault.request"),
                )
                .is_ok()
            {
                requested = true;
                fault.store(true, Ordering::SeqCst);
            }
            if !phase_written && monitor.executing.load(Ordering::SeqCst) {
                phase_written = true;
                let phase = serde_json::json!({"deadline":monitor.deadline.load(Ordering::SeqCst),
                    "executionMs":number("SOUNDROBE_NATIVE_EXECUTION_MS",600_000)});
                if crate::infra::mock_run_authority::storage::write_new_or_unscoped(
                    watched.join("execution-scope.tmp"),
                    serde_json::to_vec(&phase).unwrap(),
                )
                .is_err()
                    || std::fs::rename(
                        watched.join("execution-scope.tmp"),
                        watched.join("execution-scope.json"),
                    )
                    .is_err()
                {
                    monitor.stopped.store(true, Ordering::SeqCst);
                }
            }
            if monitor.managed
                && monitor.quiescent.load(Ordering::SeqCst)
                && monitor.candidate.get().is_none()
            {
                let candidate = receiver::read_json(&watched.join("authority/provisional.json"))
                    .and_then(|(report, evidence)| {
                        let (_, result) = receiver::read_json(&watched.join("results.json"))?;
                        if report["state"] != "provisional_complete"
                            || report["ownerLive"] != false
                            || report["resultSha256"] != receiver::digest_string(&result)
                        {
                            return Err(std::io::ErrorKind::InvalidData.into());
                        }
                        Ok((evidence, result))
                    });
                match candidate {
                    Ok(candidate) => {
                        let _ = monitor.candidate.set(candidate);
                    }
                    Err(_) => monitor.stopped.store(true, Ordering::SeqCst),
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    Ok(Owned {
        anchor,
        broker,
        peer: Channel::new(peer_s)?,
        group: Channel::new(group_s)?,
    })
}

fn admission_open(scope: &mut Scope, monitor: &Monitor) -> bool {
    let now = monotonic_ms();
    if scope.clip(now, 1).is_err() || monitor.check().is_err() {
        monitor.stopped.store(true, Ordering::SeqCst);
        scope.stop(now);
        return false;
    }
    true
}

fn owner_loop(
    mut peer: Channel,
    mut group: Channel,
    mut scope: Scope,
    group_id: i32,
    monitor: Arc<Monitor>,
    mut children: Option<OwnedChildren>,
    fault: Arc<AtomicBool>,
) -> ! {
    let mut peer_seen = monotonic_ms();
    let mut heartbeat = 0;
    let mut admitted = false;
    let mut peer_registered = false;
    let mut stop_sent = false;
    let mut group_closed = false;
    let mut anchor_reaped = false;
    let mut pending = None;
    let mut anchor_transition = false;
    let mut peer_transition = false;
    let mut transition_announced = false;
    let mut transition_reported = false;
    let mut payload_exited = false;
    let mut peer_staged = false;
    let mut stage_announced = false;
    let mut retiring = false;
    let mut retirement_intent = false;
    let mut peer_closed = false;
    let mut anchor_status = None;
    let mut broker_exited = false;
    loop {
        let now = monotonic_ms();
        if let Some(owned) = &mut children {
            if !broker_exited {
                match owned.broker.try_wait() {
                    Ok(Some(status)) if retiring && status.success() => broker_exited = true,
                    Ok(None) => {}
                    _ => {
                        monitor.stopped.store(true, Ordering::SeqCst);
                        scope.stop(now);
                    }
                }
            }
        }
        if children
            .as_mut()
            .and_then(|owned| owned.receiver.as_mut())
            .is_some_and(|link| !link.healthy())
        {
            monitor.stopped.store(true, Ordering::SeqCst);
            scope.stop(now);
        }
        if scope.clip(now, 1).is_err()
            || (!broker_exited && now.saturating_sub(peer_seen) >= 1_000)
            || monitor.stopped.load(Ordering::SeqCst)
        {
            monitor.stopped.store(true, Ordering::SeqCst);
            scope.stop(now);
        }
        'admissions: {
            if !admission_open(&mut scope, &monitor) {
                break 'admissions;
            }
            match if peer_closed {
                Ok(None)
            } else {
                peer.receive()
            } {
                Ok(Some((1, 0))) => peer_seen = now,
                Ok(Some((6, id))) if id == group_id as u64 && !peer_registered => {
                    peer_registered = true;
                    peer_seen = now;
                }
                Ok(Some((2, deadline)))
                    if children.is_some()
                        && pending.is_none()
                        && !scope.executing
                        && deadline > now
                        && deadline <= now.saturating_add(scope.execution_ms) =>
                {
                    pending = Some(deadline);
                    if group.send(2, deadline).is_err() || peer.send(11, deadline).is_err() {
                        monitor.stopped.store(true, Ordering::SeqCst);
                        scope.stop(now);
                    }
                }
                Ok(Some((11, deadline)))
                    if children.is_none() && pending == Some(deadline) && !transition_announced =>
                {
                    transition_announced = true;
                    if group.send(2, deadline).is_err() {
                        monitor.stopped.store(true, Ordering::SeqCst);
                        scope.stop(now);
                    }
                }
                Ok(Some((12, deadline)))
                    if pending == Some(deadline) && !peer_transition && !scope.executing =>
                {
                    peer_transition = true;
                }
                Ok(Some((13, 0))) if children.is_some() && !peer_staged && scope.executing => {
                    peer_staged = true;
                }
                Ok(Some((14, 0)))
                    if children.is_none()
                        && !retiring
                        && monitor.staged.load(Ordering::SeqCst)
                        && monitor.payload_exited()
                        && scope.executing =>
                {
                    retiring = true;
                    if group.send(3, 0).is_err() {
                        monitor.stopped.store(true, Ordering::SeqCst);
                        scope.stop(now);
                    }
                }
                Err(_) if retiring && children.is_some() => peer_closed = true,
                Ok(Some((32, 0)))
                    if children.is_none()
                        && matches!(
                            monitor.control.as_str(),
                            "execution-deadline" | "retirement" | "retirement-descendant"
                        ) =>
                {
                    monitor.execution_requested.store(true, Ordering::SeqCst);
                }
                Ok(Some((30, 0))) if children.is_none() && monitor.control == "broker-loss" => {
                    kill_control_owner();
                }
                Ok(Some((30, 0))) if children.is_none() && monitor.control == "broker-suspend" => unsafe {
                    libc::kill(std::process::id() as i32, libc::SIGSTOP);
                },
                Ok(None) => {}
                _ => {
                    monitor.stopped.store(true, Ordering::SeqCst);
                    scope.stop(now);
                }
            }
            if !admission_open(&mut scope, &monitor) {
                break 'admissions;
            }
            if !group_closed {
                match group.receive() {
                    Ok(Some((6, id))) if id == group_id as u64 && !admitted => {
                        admitted = true;
                        if peer.send(6, id).is_err() || group.send(5, scope.deadline).is_err() {
                            monitor.stopped.store(true, Ordering::SeqCst);
                            scope.stop(now);
                        }
                    }
                    Ok(Some((7, pid))) if admitted && pid > 1 => {}
                    Ok(Some((10, deadline))) if pending == Some(deadline) && !anchor_transition => {
                        anchor_transition = true;
                    }
                    Ok(Some((8, 0))) if !payload_exited => payload_exited = true,
                    Ok(Some((16, 0))) if payload_exited && !monitor.payload_exited() => {
                        monitor.payload_exited.store(true, Ordering::SeqCst);
                    }
                    Ok(Some((9, 1))) if retiring && !retirement_intent => {
                        retirement_intent = true;
                        group_closed = true;
                        if children.is_some() {
                            receiver::fault_barrier(
                                &PathBuf::from(
                                    std::env::var_os("SOUNDROBE_NATIVE_OUTPUT").unwrap(),
                                ),
                                "final-retire-intent",
                            );
                        }
                    }
                    Ok(None) => {}
                    _ => {
                        group_closed = true;
                        monitor.stopped.store(true, Ordering::SeqCst);
                        scope.stop(now);
                    }
                }
            }
            if !admission_open(&mut scope, &monitor) {
                break 'admissions;
            }
            if children.is_none()
                && pending.is_none()
                && admitted
                && peer_registered
                && monitor.execution_requested.load(Ordering::SeqCst)
            {
                let deadline = now.saturating_add(scope.execution_ms);
                pending = Some(deadline);
                if peer.send(2, deadline).is_err() {
                    monitor.stopped.store(true, Ordering::SeqCst);
                    scope.stop(now);
                }
            }
            if anchor_transition && !scope.executing {
                let deadline = pending.unwrap();
                if children.is_none() && !transition_reported {
                    transition_reported = true;
                    if peer.send(12, deadline).is_err() {
                        monitor.stopped.store(true, Ordering::SeqCst);
                        scope.stop(now);
                    }
                }
                if peer_transition {
                    if scope.execute(monotonic_ms()).is_err() {
                        monitor.stopped.store(true, Ordering::SeqCst);
                    } else {
                        scope.deadline = scope.deadline.min(deadline);
                        monitor.deadline.store(scope.deadline, Ordering::SeqCst);
                        monitor.executing.store(true, Ordering::SeqCst);
                        if let Some(link) =
                            children.as_mut().and_then(|owned| owned.receiver.as_mut())
                        {
                            if link.phase(scope.deadline).is_err() {
                                monitor.stopped.store(true, Ordering::SeqCst);
                                scope.stop(now);
                            }
                        }
                        if children.is_some()
                            && admission_open(&mut scope, &monitor)
                            && peer.send(12, deadline).is_err()
                        {
                            monitor.stopped.store(true, Ordering::SeqCst);
                            scope.stop(now);
                        }
                    }
                }
            }
            if !admission_open(&mut scope, &monitor) {
                break 'admissions;
            }
            if children.is_none() && monitor.staged.load(Ordering::SeqCst) && !stage_announced {
                stage_announced = true;
                if peer.send(13, 0).is_err() {
                    monitor.stopped.store(true, Ordering::SeqCst);
                    scope.stop(now);
                }
            }
            if children.is_some() && peer_staged && monitor.payload_exited() && !retiring {
                retiring = true;
                if group.send(3, 0).is_err() {
                    monitor.stopped.store(true, Ordering::SeqCst);
                    scope.stop(now);
                } else {
                    receiver::fault_barrier(
                        &PathBuf::from(std::env::var_os("SOUNDROBE_NATIVE_OUTPUT").unwrap()),
                        "final-retire-one",
                    );
                    if !admission_open(&mut scope, &monitor) {
                        break 'admissions;
                    }
                    if peer.send(14, 0).is_err() {
                        monitor.stopped.store(true, Ordering::SeqCst);
                        scope.stop(now);
                    }
                }
            }
            if !admission_open(&mut scope, &monitor) {
                break 'admissions;
            }
            if now.saturating_sub(heartbeat) >= 250 {
                heartbeat = now;
                if !peer_closed && !broker_exited && peer.send(1, 0).is_err() {
                    if retiring && children.is_some() {
                        peer_closed = true;
                    } else {
                        monitor.stopped.store(true, Ordering::SeqCst);
                        scope.stop(now);
                    }
                }
                if admitted && !retiring && group.send(1, 0).is_err() {
                    monitor.stopped.store(true, Ordering::SeqCst);
                    scope.stop(now);
                }
            }
            if children.is_some() && fault.swap(false, Ordering::SeqCst) {
                if monitor.control == "supervisor-loss" {
                    kill_control_owner();
                }
                if monitor.control == "supervisor-suspend" {
                    unsafe {
                        libc::kill(std::process::id() as i32, libc::SIGSTOP);
                    }
                }
                let op = if matches!(
                    monitor.control.as_str(),
                    "execution-deadline" | "retirement" | "retirement-descendant"
                ) {
                    32
                } else {
                    30
                };
                if peer.send(op, 0).is_err() {
                    monitor.stopped.store(true, Ordering::SeqCst);
                    scope.stop(now);
                }
            }
        }
        if scope.stopped_at.is_some() && !stop_sent {
            stop_sent = true;
            // No journal work, RPC acknowledgment, async runtime or Node event
            // loop is on this termination path.
            let _ = peer.send(4, 0);
            let _ = group.send(4, 0);
            let _ = group.stream.shutdown(std::net::Shutdown::Both);
        }
        if let Some(children) = &mut children {
            if !anchor_reaped {
                match children.anchor.try_wait() {
                    Ok(Some(status)) => {
                        anchor_reaped = true;
                        anchor_status = Some(status);
                        // A retirement request alone is not proof of retirement.
                        // A buffered intent must still be read and checked below.
                        if !retiring {
                            monitor.stopped.store(true, Ordering::SeqCst);
                            scope.stop(now);
                        }
                    }
                    Ok(None) => {}
                    Err(_) => {
                        monitor.stopped.store(true, Ordering::SeqCst);
                        scope.stop(now);
                    }
                }
            }
            if retirement_intent && anchor_reaped && scope.stopped_at.is_none() {
                use std::os::unix::process::ExitStatusExt;
                if anchor_status.as_ref().and_then(|status| status.signal()) != Some(9) {
                    monitor.stopped.store(true, Ordering::SeqCst);
                    scope.stop(now);
                } else if broker_exited && group_absent(group_id) {
                    monitor.quiescent.store(true, Ordering::SeqCst);
                    if !monitor.managed && monitor.control == "retirement" {
                        unsafe { libc::_exit(0) }
                    }
                    if let Some((evidence, result)) = monitor.candidate.get() {
                        if admission_open(&mut scope, &monitor) {
                            let complete = children.receiver.as_mut().is_some_and(|link| {
                                link.complete(*evidence, *result, scope.deadline).is_ok()
                            });
                            unsafe { libc::_exit(if complete { 0 } else { 78 }) }
                        }
                    }
                }
            }
            if scope.stopped_at.is_some() && anchor_reaped && group_absent(group_id) {
                // This is the supervisor's unreaped direct control child, not
                // an arbitrary payload PID or externally signalled PGID.
                if monitor.control != "broker-loss"
                    && matches!(children.broker.try_wait(), Ok(None))
                {
                    let _ = children.broker.kill();
                }
                if let Ok(Some(status)) = children.broker.try_wait() {
                    if monitor.control == "broker-loss" {
                        use std::os::unix::process::ExitStatusExt;
                        if status.signal() != Some(9) {
                            unsafe {
                                libc::_exit(79);
                            }
                        }
                    }
                    unsafe {
                        libc::_exit(78);
                    }
                }
            }
        } else if group_absent(group_id) {
            if retirement_intent
                && scope.stopped_at.is_none()
                && admission_open(&mut scope, &monitor)
            {
                unsafe { libc::_exit(0) }
            }
            if scope.stopped_at.is_some() {
                unsafe { libc::_exit(78) }
            }
        }
        if scope.stopped_at.is_some() && scope.grace_remaining(now) == 0 {
            unsafe {
                libc::_exit(78);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

struct OwnedChildren {
    anchor: Child,
    broker: Child,
    receiver: Option<receiver::SupervisorLink>,
}

#[test]
#[ignore = "explicit native mock ownership process entry; never application code"]
fn native_supervision_process() {
    let role = std::env::var("SOUNDROBE_NATIVE_ROLE").expect("explicit native role");
    let start = number("SOUNDROBE_NATIVE_START", monotonic_ms());
    let preparation = number("SOUNDROBE_NATIVE_PREPARATION_MS", 300_000);
    let execution = number("SOUNDROBE_NATIVE_EXECUTION_MS", 600_000);
    let mut scope = Scope::new(start, preparation, execution, 10_000).unwrap();
    scope.announce_transition = true;
    let root =
        PathBuf::from(std::env::var("SOUNDROBE_NATIVE_OUTPUT").expect("explicit disposable root"));
    let monitor = Arc::new(Monitor {
        stopped: Arc::new(AtomicBool::new(false)),
        control: std::env::var("SOUNDROBE_NATIVE_CONTROL").unwrap_or_default(),
        managed: std::env::var("SOUNDROBE_NATIVE_MANAGED").as_deref() == Ok("1"),
        staged: AtomicBool::new(false),
        payload_exited: AtomicBool::new(false),
        quiescent: AtomicBool::new(false),
        candidate: OnceLock::new(),
        execution_requested: AtomicBool::new(false),
        executing: AtomicBool::new(false),
        deadline: AtomicU64::new(scope.deadline),
    });
    match role.as_str() {
        "receiver" => receiver::run(root, scope),
        "receiver-sender-control" => receiver::sender_control(root, scope),
        "supervisor" => {
            let mut receiver = receiver::SupervisorLink::inherited();
            let (tx, rx) = mpsc::sync_channel(1);
            let fault = Arc::new(AtomicBool::new(false));
            let watched = fault.clone();
            let io_monitor = monitor.clone();
            std::thread::spawn(move || {
                let _ = tx.send(bootstrap(root, start, preparation, watched, io_monitor));
            });
            let owned = loop {
                if scope.clip(monotonic_ms(), 1).is_err()
                    || receiver.as_mut().is_some_and(|link| !link.healthy())
                {
                    unsafe {
                        libc::_exit(78);
                    }
                }
                match rx.try_recv() {
                    Ok(Ok(owned)) => break owned,
                    Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => unsafe {
                        libc::_exit(78);
                    },
                    Err(mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
                }
            };
            let id = owned.anchor.id() as i32;
            owner_loop(
                owned.peer,
                owned.group,
                scope,
                id,
                monitor,
                Some(OwnedChildren {
                    anchor: owned.anchor,
                    broker: owned.broker,
                    receiver,
                }),
                fault,
            );
        }
        "broker" => {
            let peer = Channel::new(inherited("SOUNDROBE_NATIVE_PEER")).unwrap();
            let group = Channel::new(inherited("SOUNDROBE_NATIVE_GROUP_CHANNEL")).unwrap();
            let id = number("SOUNDROBE_NATIVE_GROUP", 0) as i32;
            BROKER
                .set(monitor.clone())
                .ok()
                .expect("one native monitor per broker");
            let app_monitor = monitor.clone();
            let managed = monitor.managed;
            std::thread::spawn(move || {
                owner_loop(
                    peer,
                    group,
                    scope,
                    id,
                    monitor,
                    None,
                    Arc::new(AtomicBool::new(false)),
                )
            });
            if managed {
                crate::infra::mock_run_authority::storage::attach_from_env().unwrap();
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(4)
                    .enable_all()
                    .build()
                    .unwrap();
                runtime
                    .block_on(super::super::broker::supervised_process(root, app_monitor))
                    .unwrap();
                unsafe { libc::_exit(78) }
            }
            loop {
                if app_monitor.control == "retirement" && app_monitor.payload_exited() {
                    app_monitor.stage();
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        "anchor" => {
            crate::infra::mock_run_authority::storage::attach_from_env().unwrap();
            let owners = [
                inherited("SOUNDROBE_NATIVE_ANCHOR_S"),
                inherited("SOUNDROBE_NATIVE_ANCHOR_B"),
            ];
            // Restore CLOEXEC before the anchor's gated payload spawn.
            let owners = owners.map(|s| Channel::new(s).unwrap().stream);
            let mut payload = Command::new("node");
            if monitor.managed {
                let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap();
                if monitor.control.starts_with("final-") {
                    payload
                        .current_dir(project)
                        .arg(project.join("scripts/auto-tag-eval-replay/finalization-control.cjs"))
                        .arg(&root)
                        .arg(std::env::current_exe().unwrap());
                } else {
                    payload.current_dir(project).args([
                        project
                            .join("scripts/auto-tag-eval-replay/run-bounded.cjs")
                            .into_os_string(),
                        root.clone().into_os_string(),
                        std::env::current_exe().unwrap().into_os_string(),
                        project
                            .join(".planning/plans/crowd-lu-user-acceptance-v1-20261111")
                            .into_os_string(),
                        "--supervised-mock".into(),
                    ]);
                }
            } else {
                payload.args(["-e", WIRE_PAYLOAD]);
            }
            if monitor.control == "anchor-running-loss" {
                let watched = root.clone();
                std::thread::spawn(move || {
                    while crate::infra::mock_run_authority::storage::files::metadata(
                        watched.join("blocked-node.json"),
                    )
                    .is_err()
                    {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    // Observed payload execution, then anchor-only loss. The
                    // finite orphan exits itself; no stale group kill follows.
                    kill_control_owner();
                });
            }
            if monitor.control == "anchor-loss" {
                let (mut owner, held) = UnixStream::pair().unwrap();
                let owner_fd = owner.as_raw_fd();
                unsafe {
                    payload.pre_exec(move || {
                        libc::close(owner_fd);
                        let fd = held.as_raw_fd();
                        let mut byte = 1_u8;
                        if libc::write(fd, &byte as *const u8 as *const _, 1) != 1
                            || libc::read(fd, &mut byte as *mut u8 as *mut _, 1) != 1
                        {
                            return Err(std::io::ErrorKind::BrokenPipe.into());
                        }
                        Ok(())
                    });
                }
                let watched = root.clone();
                std::thread::spawn(move || {
                    let mut byte = [0];
                    owner.read_exact(&mut byte).unwrap();
                    crate::infra::mock_run_authority::storage::write_new_or_unscoped(
                        watched.join("anchor-preexec-blocked"),
                        b"native child gated before exec",
                    )
                    .unwrap();
                    std::thread::sleep(Duration::from_millis(100));
                    // Fault injection kills this anchor only, never its group.
                    kill_control_owner();
                });
            }
            payload
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("SOUNDROBE_NATIVE_OUTPUT", root)
                .env(
                    "SOUNDROBE_STORAGE_EXECUTABLE",
                    std::env::current_exe().unwrap(),
                )
                .env(
                    "SOUNDROBE_NATIVE_GENERATION",
                    std::env::var("SOUNDROBE_NATIVE_GENERATION").unwrap_or_default(),
                )
                .env(
                    "SOUNDROBE_STORAGE_REQUIRED",
                    std::env::var("SOUNDROBE_STORAGE_REQUIRED").unwrap_or_default(),
                )
                .env("SOUNDROBE_NATIVE_GROUP", std::process::id().to_string())
                .env("SOUNDROBE_NATIVE_CONTROL", &monitor.control)
                .env(
                    "SOUNDROBE_NATIVE_MANAGED",
                    if monitor.managed { "1" } else { "0" },
                )
                .stdin(Stdio::null());
            run_anchor(payload, owners, scope);
        }
        "dead-anchor-control" => {
            let (peer, mut remote_peer) = UnixStream::pair().unwrap();
            let (group, remote_group) = UnixStream::pair().unwrap();
            let mut anchor = Command::new("/usr/bin/true")
                .process_group(0)
                .spawn()
                .unwrap();
            let group_id = anchor.id() as i32;
            while anchor.try_wait().unwrap().is_none() {
                std::thread::sleep(Duration::from_millis(1));
            }
            let broker = Command::new("/usr/bin/true").spawn().unwrap();
            std::thread::spawn(move || {
                let _keep_group_open = remote_group;
                loop {
                    if remote_peer
                        .write_all(&[&[1], 0_u64.to_le_bytes().as_slice()].concat())
                        .is_err()
                    {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            });
            owner_loop(
                Channel::new(peer).unwrap(),
                Channel::new(group).unwrap(),
                scope,
                group_id,
                monitor,
                Some(OwnedChildren {
                    anchor,
                    broker,
                    receiver: None,
                }),
                Arc::new(AtomicBool::new(false)),
            );
        }
        "queued-stop-control" => {
            let (peer, mut remote_peer) = UnixStream::pair().unwrap();
            let (group, mut remote_group) = UnixStream::pair().unwrap();
            let group_id = std::process::id() as i32;
            remote_peer
                .write_all(&[&[4], 0_u64.to_le_bytes().as_slice()].concat())
                .unwrap();
            remote_group
                .write_all(&[&[6], (group_id as u64).to_le_bytes().as_slice()].concat())
                .unwrap();
            std::thread::spawn(move || {
                remote_group
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .unwrap();
                let mut frame = [0; 9];
                remote_group.read_exact(&mut frame).unwrap();
                std::fs::write(root.join("first-group-op"), [frame[0]]).unwrap();
                let _keep_peer_open = remote_peer;
                unsafe {
                    libc::_exit(0);
                }
            });
            owner_loop(
                Channel::new(peer).unwrap(),
                Channel::new(group).unwrap(),
                scope,
                group_id,
                monitor,
                None,
                Arc::new(AtomicBool::new(false)),
            );
        }
        _ => panic!("unsupported native mock role"),
    }
}

const WIRE_PAYLOAD: &str = r#"
const fs=require('node:fs'),net=require('node:net');
const root=process.env.SOUNDROBE_NATIVE_OUTPUT;
const server=net.createServer(socket=>{
  let bytes=Buffer.alloc(0);
  socket.on('data',chunk=>{
    bytes=Buffer.concat([bytes,chunk]);
    if(bytes.length>128) process.exit(79);
    if(!bytes.includes(Buffer.from('\r\n\r\n'))) return;
    if(bytes.toString()!=='GET / HTTP/1.0\r\n\r\n') process.exit(79);
    socket.end('HTTP/1.0 200 OK\r\nContent-Length: 5\r\n\r\nready',()=>{
      socket.destroy();
      setImmediate(()=>{
        fs.writeFileSync(root+'/wire-observed','ready',{flag:'wx'});
        if(process.env.SOUNDROBE_NATIVE_CONTROL.startsWith('retirement')) {
          const wait=setInterval(()=>{if(fs.existsSync(root+'/execution-scope.json')) {
            clearInterval(wait);
            process.exit(0);
          }},5);
          return;
        }
        const end=Date.now()+12000; while(Date.now()<end){}; process.exit(0);
      });
    });
  });
});
const listen=()=>server.listen(0,'127.0.0.1',()=>{
  fs.writeFileSync(root+'/wire-ready.tmp',JSON.stringify({
    pid:process.pid,group:Number(process.env.SOUNDROBE_NATIVE_GROUP),port:server.address().port
  }),{flag:'wx'});
  fs.renameSync(root+'/wire-ready.tmp',root+'/wire-ready.json');
});
if(process.env.SOUNDROBE_NATIVE_CONTROL==='retirement-descendant') {
  // Prepare a deliberately slow descendant before the execution clock starts.
  // Its survival after payload exit, not Node startup speed, is under test.
  require('node:child_process').spawn(process.execPath,['-e',
    "setTimeout(()=>{require('node:fs').writeFileSync(process.env.SOUNDROBE_NATIVE_OUTPUT+'/descendant-ready.json',JSON.stringify({pid:process.pid}));setInterval(()=>{},1000)},200)"],
    {env:{SOUNDROBE_NATIVE_OUTPUT:root},stdio:'ignore',detached:false});
  const prepared=setInterval(()=>{if(fs.existsSync(root+'/descendant-ready.json')) {
    clearInterval(prepared);listen();
  }},5);
} else listen();
"#;

#[cfg(test)]
mod tests {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    #[test]
    fn receiver_installs_storage_before_bootstrap_and_retains_generation() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.amp/in/artifacts/bounded-storage-20261112")
            .join(format!("native-bootstrap-{}", uuid::Uuid::new_v4()));
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", super::ENTRY, "--ignored", "--nocapture"])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("SOUNDROBE_NATIVE_ROLE", "receiver")
            .env("SOUNDROBE_NATIVE_OUTPUT", &root)
            .env("SOUNDROBE_NATIVE_PREPARATION_MS", "2000")
            .env("SOUNDROBE_NATIVE_EXECUTION_MS", "1000")
            .env("SOUNDROBE_NATIVE_CONTROL", "receiver-positive")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(
            status.success(),
            "compositional native receiver control failed"
        );
        assert_eq!(
            std::fs::metadata(root.join(".storage-ledger"))
                .unwrap()
                .len(),
            2 * 1024 * 1024
        );
        let configuration: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("native-supervision.json")).unwrap())
                .unwrap();
        assert_eq!(configuration["storage"]["logicalLimit"], 512 * 1024 * 1024);
        assert_eq!(
            configuration["storage"]["applicationWriteLimit"],
            1024 * 1024 * 1024
        );
        assert_eq!(
            configuration["storage"]["generation"],
            configuration["generation"]
        );
    }

    #[test]
    fn maintained_driver_bootstraps_the_real_broker_under_native_ownership() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.amp/in/artifacts/mock-supervision-20261112")
            .join(format!("maintained-bootstrap-{}", uuid::Uuid::new_v4()));
        let log = std::fs::File::create(root.with_extension("log")).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", super::ENTRY, "--ignored", "--nocapture"])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("SOUNDROBE_NATIVE_ROLE", "receiver")
            .env("SOUNDROBE_NATIVE_OUTPUT", &root)
            .env("SOUNDROBE_NATIVE_PREPARATION_MS", "5000")
            .env("SOUNDROBE_NATIVE_CONTROL", "broker-loss")
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        let until = Instant::now() + Duration::from_millis(2000);
        while !root.join("authority/ready.json").exists() && Instant::now() < until {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let real_broker_ready = root.join("authority/ready.json").exists();
        if root.is_dir() {
            std::fs::write(root.join("fault.request"), b"broker-loss").unwrap();
        }
        let until = Instant::now() + Duration::from_secs(3);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < until, "native cleanup did not finish");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(!status.success());
        assert!(
            real_broker_ready,
            "native ownership did not start the maintained broker; log {}",
            root.with_extension("log").display()
        );
        let descriptor: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("authority/ready.json")).unwrap())
                .unwrap();
        assert_eq!(descriptor["mode"], "durable_mock_only");
        assert!(
            std::fs::read_to_string(root.join("broker-trace.log"))
                .expect("managed broker must retain its charged TRACE witness")
                .contains("mock_broker_privacy_trace_active"),
            "discarded stderr cannot establish active managed tracing"
        );
        let identity: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("group-identity.json")).unwrap())
                .unwrap();
        assert!(super::super::group_absent(
            identity["group"].as_i64().unwrap() as i32
        ));
        assert!(!root.join("receiver-outcome.json").exists());
    }

    #[test]
    fn receiver_requires_timely_witness_and_observed_successful_exit() {
        for control in [
            "positive",
            "missing",
            "malformed",
            "rebound",
            "late-witness",
            "nonzero",
            "late-exit",
            "unknown-exit",
            "cancel-before",
            "cancel-after",
            "lost-recording",
            "lost-receiver",
            "trailing-byte",
            "duplicate",
            "trailing-cancel",
            "fragmented",
            "acceptance-gap",
            "post-cas-gap",
            "verify-blocked",
            "record-renamed",
            "suspended-after-witness",
        ] {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../.amp/in/artifacts/mock-supervision-20261112")
                .join(format!("receiver-{control}-{}", uuid::Uuid::new_v4()));
            let log = std::fs::File::create(root.with_extension("log")).unwrap();
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", super::ENTRY, "--ignored", "--nocapture"])
                .env_clear()
                .env("PATH", "/opt/homebrew/bin:/usr/bin:/bin")
                .env("SOUNDROBE_NATIVE_ROLE", "receiver")
                .env("SOUNDROBE_NATIVE_OUTPUT", &root)
                .env("SOUNDROBE_NATIVE_PREPARATION_MS", "2000")
                .env("SOUNDROBE_NATIVE_EXECUTION_MS", "400")
                .env("SOUNDROBE_NATIVE_CONTROL", format!("receiver-{control}"))
                .stdin(Stdio::null())
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap();
            let started = Instant::now();
            if control == "suspended-after-witness" {
                while !root.join("witness-sent-before-suspension").exists() {
                    assert!(started.elapsed() < Duration::from_secs(2));
                    assert!(child.try_wait().unwrap().is_none());
                    std::thread::sleep(Duration::from_millis(2));
                }
                let identity: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(root.join("supervisor-identity.json")).unwrap(),
                )
                .unwrap();
                loop {
                    let state = Command::new("/bin/ps")
                        .args(["-o", "state=", "-p", &identity["pid"].to_string()])
                        .output()
                        .unwrap();
                    assert!(
                        state.status.success(),
                        "sender disappeared before suspension observation"
                    );
                    if String::from_utf8_lossy(&state.stdout).contains('T') {
                        break;
                    }
                    assert!(
                        started.elapsed() < Duration::from_millis(300),
                        "actual sender suspension"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(4),
                    "receiver hung: {control}"
                );
                std::thread::sleep(Duration::from_millis(5));
            };
            let complete = matches!(control, "positive" | "cancel-after" | "fragmented");
            assert_eq!(
                status.success(),
                complete,
                "receiver acceptance: {control}; retained log {}",
                root.with_extension("log").display()
            );
            let membership: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root.join("requested-membership.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(membership["requestedFieldSlots"], 216);
            if complete {
                let receipt: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(root.join("receiver-outcome.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(receipt["state"], "complete");
                assert!(
                    receipt["supervisorExitObservedAt"].as_u64().unwrap()
                        <= receipt["acceptedAt"].as_u64().unwrap()
                );
                assert!(
                    receipt["acceptedAt"].as_u64().unwrap() < receipt["deadline"].as_u64().unwrap()
                );
                assert_ne!(receipt["receiverGroup"], receipt["supervisorGroup"]);
            } else {
                assert_eq!(
                    root.join("receiver-outcome.json").exists(),
                    control == "record-renamed",
                    "only the deliberately unacknowledged rename leaves a file: {control}"
                );
                if matches!(
                    control,
                    "post-cas-gap" | "verify-blocked" | "record-renamed"
                ) {
                    assert!(
                        root.join(format!("receiver-{control}.reached")).is_file(),
                        "actual fault boundary: {control}"
                    );
                }
                if control == "suspended-after-witness" {
                    let identity: serde_json::Value = serde_json::from_slice(
                        &std::fs::read(root.join("supervisor-identity.json")).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(
                        unsafe { libc::kill(identity["pid"].as_i64().unwrap() as i32, 0) },
                        -1
                    );
                    assert_eq!(
                        std::io::Error::last_os_error().raw_os_error(),
                        Some(libc::ESRCH)
                    );
                }
            }
        }
    }

    #[test]
    fn reaped_anchor_revokes_without_waiting_for_channel_eof_or_lease() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.amp/in/artifacts/mock-supervision-20261112")
            .join(format!("dead-anchor-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let start = Instant::now();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", super::ENTRY, "--ignored", "--nocapture"])
            .env_clear()
            .env("SOUNDROBE_NATIVE_ROLE", "dead-anchor-control")
            .env("SOUNDROBE_NATIVE_OUTPUT", &root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if start.elapsed() > Duration::from_millis(500) {
                child.kill().unwrap();
                break child.wait().unwrap();
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(
            status.code(),
            Some(78),
            "anchor death did not immediately revoke owners"
        );
        assert!(start.elapsed() < Duration::from_millis(300));
        assert!(!root.join("selection-status.json").exists());
    }

    #[test]
    fn queued_peer_stop_never_releases_ready_anchor() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.amp/in/artifacts/mock-supervision-20261112")
            .join(format!("queued-stop-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", super::ENTRY, "--ignored", "--nocapture"])
            .env_clear()
            .env("SOUNDROBE_NATIVE_ROLE", "queued-stop-control")
            .env("SOUNDROBE_NATIVE_OUTPUT", &root)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            std::fs::read(root.join("first-group-op")).unwrap(),
            [4],
            "revocation must precede and exclude startup grant"
        );
    }

    #[test]
    fn actual_broker_loss_terminates_the_observed_loopback_payload() {
        observed_owner_loss("broker-loss");
    }

    #[test]
    fn actual_supervisor_loss_terminates_the_observed_loopback_payload() {
        observed_owner_loss("supervisor-loss");
    }

    #[test]
    fn native_owners_share_the_once_only_execution_deadline() {
        observed_owner_loss("execution-deadline");
    }

    #[test]
    fn managed_finalization_requires_staging_retirement_and_timely_acceptance() {
        for control in [
            "positive",
            "file-sync",
            "provisional-rename",
            "retire-one",
            "retire-intent",
        ] {
            let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../.amp/in/artifacts/mock-supervision-20261112")
                .join(format!("final-{control}-{}", uuid::Uuid::new_v4()));
            let log = std::fs::File::create(root.with_extension("log")).unwrap();
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", super::ENTRY, "--ignored", "--nocapture"])
                .env_clear()
                .env("PATH", "/opt/homebrew/bin:/usr/bin:/bin")
                .env("SOUNDROBE_NATIVE_ROLE", "receiver")
                .env("SOUNDROBE_NATIVE_OUTPUT", &root)
                .env("SOUNDROBE_NATIVE_PREPARATION_MS", "3000")
                .env("SOUNDROBE_NATIVE_EXECUTION_MS", "1500")
                .env("SOUNDROBE_NATIVE_CONTROL", format!("final-{control}"))
                .stdin(Stdio::null())
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap();
            let started = Instant::now();
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(8),
                    "bounded managed finalization: {control}"
                );
                std::thread::sleep(Duration::from_millis(5));
            };
            assert!(
                root.join("final-control-produced.json").is_file(),
                "real broker lifecycle before boundary: {control}"
            );
            assert_eq!(
                status.success(),
                control == "positive",
                "managed finalization: {control}; {}",
                root.with_extension("log").display()
            );
            assert_eq!(
                root.join("receiver-outcome.json").is_file(),
                control == "positive"
            );
            if control != "positive" {
                assert!(
                    root.join(format!("final-{control}.reached")).is_file(),
                    "actual boundary: {control}"
                );
            }
            let identity: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.join("group-identity.json")).unwrap())
                    .unwrap();
            let group = identity["group"].as_i64().unwrap() as i32;
            // Failed receiver exit is not a quiescence certificate. Observe
            // surviving owners within the same original total fixture bound.
            // Successful acceptance must already have observed group absence.
            if control != "positive" {
                while !super::super::group_absent(group) {
                    assert!(
                        started.elapsed() < Duration::from_secs(8),
                        "bounded failed-finalization containment: {control}"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            assert!(super::super::group_absent(group));
            let membership: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root.join("requested-membership.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(membership["requestedFieldSlots"], 216);
            if control == "provisional-rename" {
                assert!(root.join("authority/provisional.json").is_file());
            }
            if control == "positive" {
                let receipt: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(root.join("receiver-outcome.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(receipt["controlOnly"], true);
                assert!(
                    receipt["acceptedAt"].as_u64().unwrap() < receipt["deadline"].as_u64().unwrap()
                );
            }
        }
    }

    #[test]
    fn joint_retirement_requires_actual_payload_and_group_exit() {
        observed_owner_loss("retirement");
    }

    #[test]
    fn surviving_descendant_blocks_success_until_execution_deadline() {
        observed_owner_loss("retirement-descendant");
    }

    fn observed_owner_loss(control: &str) {
        use std::io::{Read, Write};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.amp/in/artifacts/mock-supervision-20261112")
            .join(format!("{control}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let log = std::fs::File::create(root.join("native.log")).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infra::mock_run_authority::supervisor::ownership::native_supervision_process",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("PATH", "/opt/homebrew/bin:/usr/bin:/bin")
            .env("SOUNDROBE_NATIVE_ROLE", "supervisor")
            .env("SOUNDROBE_NATIVE_OUTPUT", &root)
            .env("SOUNDROBE_NATIVE_PREPARATION_MS", "8000")
            .env("SOUNDROBE_NATIVE_EXECUTION_MS", "300")
            .env("SOUNDROBE_NATIVE_CONTROL", control)
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        while !root.join("wire-ready.json").exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "native owner exited before wire readiness"
            );
            assert!(
                Instant::now() < until,
                "native owner failed to release payload"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let ready: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("wire-ready.json")).unwrap()).unwrap();
        let mut stream =
            std::net::TcpStream::connect(("127.0.0.1", ready["port"].as_u64().unwrap() as u16))
                .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        stream.write_all(b"GET / HTTP/1.0\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("200 OK") && response.ends_with("ready"));
        let group = ready["group"].as_i64().unwrap() as i32;
        assert_eq!(
            unsafe { libc::getpgid(ready["pid"].as_u64().unwrap() as i32) },
            group
        );
        std::thread::sleep(Duration::from_millis(300));
        assert!(child.try_wait().unwrap().is_none());
        assert!(!super::super::group_absent(group));
        assert!(root.join("wire-observed").exists());
        std::fs::write(root.join("fault.request"), control).unwrap();
        let stopped = Instant::now();
        if control == "retirement-descendant" {
            while !root.join("descendant-ready.json").exists()
                || unsafe { libc::getpgid(ready["pid"].as_u64().unwrap() as i32) } != -1
            {
                assert!(stopped.elapsed() < Duration::from_millis(150));
                std::thread::sleep(Duration::from_millis(2));
            }
            let descendant: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.join("descendant-ready.json")).unwrap())
                    .unwrap();
            assert_eq!(
                unsafe { libc::getpgid(ready["pid"].as_u64().unwrap() as i32) },
                -1
            );
            assert_eq!(
                unsafe { libc::getpgid(descendant["pid"].as_u64().unwrap() as i32) },
                group
            );
            assert!(child.try_wait().unwrap().is_none());
        }
        while !super::super::group_absent(group) {
            assert!(
                stopped.elapsed() < Duration::from_millis(1400),
                "{control} failed to contain the group"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(stopped.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(status.success(), control == "retirement");
        if matches!(control, "execution-deadline" | "retirement-descendant") {
            assert!(
                stopped.elapsed() >= Duration::from_millis(250),
                "not the execution deadline"
            );
            assert!(
                stopped.elapsed() < Duration::from_millis(900),
                "lease/preparation expiry is not the execution deadline"
            );
            let phase: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.join("execution-scope.json")).unwrap())
                    .unwrap();
            assert_eq!(phase["executionMs"], 300);
            assert!(phase["deadline"].as_u64().unwrap() <= super::super::monotonic_ms());
        }
        if control == "supervisor-loss" {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(status.signal(), Some(9));
        } else if control != "retirement" {
            assert_eq!(status.code(), Some(78), "broker SIGKILL not confirmed");
        }
        assert!(!root.join("selection-status.json").exists());
    }

    #[test]
    fn native_bootstrap_deadline_precedes_blocked_preparation() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.amp/in/artifacts/mock-supervision-20261112")
            .join(format!("bootstrap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let entry = concat!(
            "infra::mock_run_authority::supervisor::ownership::",
            "native_supervision_process"
        );
        let start = Instant::now();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", entry, "--ignored", "--nocapture"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("SOUNDROBE_NATIVE_ROLE", "supervisor")
            .env("SOUNDROBE_NATIVE_OUTPUT", &root)
            .env("SOUNDROBE_NATIVE_PREPARATION_MS", "150")
            .env("SOUNDROBE_NATIVE_CONTROL", "blocked-bootstrap")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "bootstrap deadline disabled"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(
            root.join("bootstrap-entered").exists(),
            "native ownership did not start before preparation"
        );
        assert!(!status.success(), "blocked preparation cannot complete");
        assert!(!root.join("payload-started").exists());
        assert!(!root.join("selection-status.json").exists());
        assert!(start.elapsed() < Duration::from_millis(650));
    }
}
