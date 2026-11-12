//! Mock-only native supervision; never an application module.

#[cfg(all(feature = "server", target_os = "macos"))]
#[path = "mock_run_ownership.rs"]
pub(crate) mod ownership;

// Absolute milliseconds in the owners' shared OS monotonic clock domain.
// Relative Rust Instants must not be serialized between processes.
struct Scope {
    deadline: u64,
    execution_ms: u64,
    executing: bool,
    grace_ms: u64,
    stopped_at: Option<u64>,
    announce_transition: bool,
}

impl Scope {
    fn new(
        start: u64,
        preparation_ms: u64,
        execution_ms: u64,
        grace_ms: u64,
    ) -> Result<Self, &'static str> {
        if !(1..=300_000).contains(&preparation_ms)
            || !(1..=600_000).contains(&execution_ms)
            || !(1..=10_000).contains(&grace_ms)
        {
            return Err("mock_supervision_invalid_limits");
        }
        Ok(Self {
            deadline: start
                .checked_add(preparation_ms)
                .ok_or("mock_supervision_clock_overflow")?,
            execution_ms,
            executing: false,
            grace_ms,
            stopped_at: None,
            announce_transition: false,
        })
    }

    fn clip(&mut self, now: u64, local_ms: u64) -> Result<u64, &'static str> {
        if now >= self.deadline {
            self.stop(self.deadline);
        }
        if self.stopped_at.is_some() {
            return Err("mock_supervision_stopped");
        }
        Ok(local_ms.min(self.deadline - now))
    }

    fn execute(&mut self, now: u64) -> Result<(), &'static str> {
        self.clip(now, 1)?;
        if self.executing {
            self.stop(now);
            return Err("mock_supervision_duplicate_transition");
        }
        let Some(deadline) = now.checked_add(self.execution_ms) else {
            self.stop(now);
            return Err("mock_supervision_clock_overflow");
        };
        self.executing = true;
        self.deadline = deadline;
        Ok(())
    }

    fn stop(&mut self, now: u64) {
        self.stopped_at.get_or_insert(now.min(self.deadline));
    }

    fn grace_remaining(&self, now: u64) -> u64 {
        self.stopped_at.map_or(0, |start| {
            self.grace_ms.saturating_sub(now.saturating_sub(start))
        })
    }
}

#[cfg(target_os = "macos")]
use mac::{group_absent, monotonic_ms, run_anchor};

#[cfg(target_os = "macos")]
mod mac {
    use super::Scope;
    use std::io::{Read, Write};
    use std::os::unix::process::CommandExt;
    use std::os::{fd::AsRawFd, unix::net::UnixStream};
    use std::process::{Child, Command};
    use std::sync::mpsc;

    #[link(name = "proc")]
    extern "C" {
        fn proc_listpids(kind: u32, group: u32, buffer: *mut i32, bytes: i32) -> i32;
    }
    extern "C" {
        fn getpgrp() -> i32;
        fn kill(pid: i32, signal: i32) -> i32;
        fn clock_gettime(clock: u32, output: *mut i64) -> i32;
        fn fcntl(fd: i32, command: i32, ...) -> i32;
        fn read(fd: i32, data: *mut u8, size: usize) -> isize;
        fn write(fd: i32, data: *const u8, size: usize) -> isize;
    }

    pub(super) fn monotonic_ms() -> u64 {
        let mut time = [0_i64; 2];
        // CLOCK_MONOTONIC and timespec from the installed Darwin SDK.
        if unsafe { clock_gettime(6, time.as_mut_ptr()) } != 0
            || time[0] < 0
            || !(0..1_000_000_000).contains(&time[1])
        {
            std::process::exit(78);
        }
        (time[0] as u64) * 1_000 + (time[1] as u64) / 1_000_000
    }

    pub(super) fn group_absent(group: i32) -> bool {
        group > 1
            && unsafe { kill(-group, 0) } == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(3) // ESRCH, not EPERM.
    }

    fn anchor_alone(group: i32) -> bool {
        let mut members = [0_i32; 4096];
        let capacity = std::mem::size_of_val(&members) as i32;
        // PROC_PGRP_ONLY: bounded kernel enumeration, never a direct-child list.
        let bytes = unsafe { proc_listpids(2, group as u32, members.as_mut_ptr(), capacity) };
        if bytes <= 0 || bytes >= capacity || bytes % 4 != 0 {
            return false;
        }
        let members = &members[..bytes as usize / 4];
        members == [group]
    }

    fn terminate_own_group(group: i32) -> ! {
        // Only the live group leader signals its own pinned group. Stored PGIDs
        // never confer signal authority to either external owner.
        if group > 1
            && group == std::process::id() as i32
            && unsafe { getpgrp() } == group
            && unsafe { kill(-group, 9) } == 0
        {
            // Do not race the pending group signal with an ordinary exit.
            // If it does not arrive, report unknown rather than success.
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        // A failed signal or identity mismatch is containment-unknown.
        std::process::exit(78);
    }

    fn event(owners: &mut [UnixStream; 2], op: u8, value: u64, group: i32) {
        let frame = [&[op], value.to_le_bytes().as_slice()].concat();
        for owner in owners {
            if owner.write_all(&frame).is_err() {
                terminate_own_group(group);
            }
        }
    }

    pub(super) fn run_anchor(
        mut payload: Command,
        mut owners: [UnixStream; 2],
        mut scope: Scope,
    ) -> ! {
        let group = std::process::id() as i32;
        if unsafe { getpgrp() } != group || group <= 1 {
            std::process::exit(78);
        }
        for owner in &owners {
            // F_GETFD / FD_CLOEXEC: payloads must not keep owner channels alive.
            let flags = unsafe { fcntl(owner.as_raw_fd(), 1) };
            if flags < 0 || flags & 1 == 0 || owner.set_nonblocking(true).is_err() {
                terminate_own_group(group);
            }
        }
        let (mut gate, child_gate) =
            UnixStream::pair().unwrap_or_else(|_| terminate_own_group(group));
        gate.set_nonblocking(true)
            .unwrap_or_else(|_| terminate_own_group(group));
        // The worker may block in spawn/exec, but never owns deadline progress.
        // The pre-exec child cannot run payload initialization before the native
        // polling owner releases this private gate. Only async-signal-safe calls
        // are permitted here, since the parent now has a spawn worker thread.
        unsafe {
            payload.pre_exec(move || {
                let fd = child_gate.as_raw_fd();
                let mut release = 0;
                if write(fd, &1_u8, 1) != 1 || read(fd, &mut release, 1) != 1 || release != 1 {
                    return Err(std::io::Error::from_raw_os_error(22));
                }
                Ok(())
            });
        }
        let mut payload = Some(payload);
        let mut spawning = None;
        let mut gate_released = false;
        event(&mut owners, 6, group as u64, group);
        let mut last_seen = [monotonic_ms(); 2];
        let mut frames = [[0; 9]; 2];
        let mut used = [0; 2];
        let mut started = [false; 2];
        let mut transitions = [None; 2];
        let mut retiring = [false; 2];
        let mut child: Option<Child> = None;
        let mut exited = false;
        let mut payload_absent_reported = false;
        loop {
            let now = monotonic_ms();
            if scope.clip(now, 1).is_err()
                || last_seen.iter().any(|t| now.saturating_sub(*t) >= 1_000)
            {
                terminate_own_group(group);
            }
            for i in 0..2 {
                // One fixed frame per owner per turn; traffic cannot starve time checks.
                match owners[i].read(&mut frames[i][used[i]..]) {
                    Ok(0) => terminate_own_group(group),
                    Ok(n) => used[i] += n,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => terminate_own_group(group),
                }
                if used[i] != 9 {
                    continue;
                }
                used[i] = 0;
                let op = frames[i][0];
                let value = u64::from_le_bytes(frames[i][1..].try_into().unwrap());
                match op {
                    1 if started[i] && value == 0 => {}
                    5 if !started[i] && value == scope.deadline => started[i] = true,
                    2 if child.is_some()
                        && !scope.executing
                        && transitions[i].is_none()
                        && value > now
                        && value <= now.saturating_add(scope.execution_ms) =>
                    {
                        transitions[i] = Some(value)
                    }
                    3 if child.is_some() && exited && !retiring[i] && value == 0 => {
                        retiring[i] = true
                    }
                    4 if value == 0 => terminate_own_group(group),
                    _ => terminate_own_group(group),
                }
                last_seen[i] = now;
            }
            if started == [true; 2] && child.is_none() && spawning.is_none() {
                if scope.clip(monotonic_ms(), 1).is_err() {
                    terminate_own_group(group);
                }
                let mut command = payload.take().unwrap_or_else(|| terminate_own_group(group));
                let (tx, rx) = mpsc::sync_channel(1);
                spawning = Some(rx);
                std::thread::spawn(move || {
                    let _ = tx.send(command.spawn());
                });
            }
            if spawning.is_some() && !gate_released {
                let mut ready = [0];
                match gate.read(&mut ready) {
                    Ok(1) if ready == [1] => {
                        if scope.clip(monotonic_ms(), 1).is_err() {
                            terminate_own_group(group);
                        }
                        gate.write_all(&[1])
                            .unwrap_or_else(|_| terminate_own_group(group));
                        gate_released = true;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    _ => terminate_own_group(group),
                }
            }
            if let Some(receiver) = &spawning {
                match receiver.try_recv() {
                    Ok(Ok(spawned)) if gate_released => {
                        event(&mut owners, 7, u64::from(spawned.id()), group);
                        child = Some(spawned);
                        spawning = None;
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                    _ => terminate_own_group(group),
                }
            }
            if !scope.executing && transitions.iter().all(Option::is_some) {
                if transitions[0] != transitions[1] || scope.execute(monotonic_ms()).is_err() {
                    terminate_own_group(group);
                }
                scope.deadline = scope.deadline.min(transitions[0].unwrap());
                if scope.announce_transition {
                    event(&mut owners, 10, scope.deadline, group);
                }
            }
            if let Some(child) = &mut child {
                if !exited {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            if !status.success() {
                                terminate_own_group(group);
                            }
                            exited = true;
                            event(&mut owners, 8, 0, group);
                        }
                        Ok(None) => {}
                        Err(_) => terminate_own_group(group),
                    }
                }
            }
            // Once only the anchor remains, no trusted payload writer can fork
            // again. Owners may durably stage evidence before joint retirement.
            if exited
                && scope.announce_transition
                && !payload_absent_reported
                && anchor_alone(group)
            {
                payload_absent_reported = true;
                event(&mut owners, 16, 0, group);
            }
            if retiring == [true; 2] && anchor_alone(group) {
                if scope.clip(monotonic_ms(), 1).is_err() {
                    terminate_own_group(group);
                }
                event(&mut owners, 9, 1, group);
                terminate_own_group(group);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_deadlines_clip_waits_and_reject_the_boundary() {
        let mut scope = Scope::new(17, 43, 71, 10).unwrap();
        assert_eq!(scope.clip(59, 5), Ok(1));
        scope.execute(59).unwrap();
        assert_eq!(scope.clip(129, 5), Ok(1));
        assert!(scope.clip(130, 5).is_err());
        assert_eq!(scope.stopped_at, Some(130));
    }

    #[test]
    fn late_or_duplicate_transition_never_extends_authority() {
        let mut late = Scope::new(17, 43, 71, 10).unwrap();
        assert!(late.execute(60).is_err());
        assert!(late.clip(61, 1).is_err());
        let mut duplicate = Scope::new(17, 43, 71, 10).unwrap();
        duplicate.execute(40).unwrap();
        assert!(duplicate.execute(41).is_err());
        assert!(duplicate.clip(42, 1).is_err());
    }

    #[test]
    fn failure_grace_starts_at_first_stop_and_is_not_a_work_scope() {
        let mut scope = Scope::new(17, 43, 71, 10).unwrap();
        scope.stop(29);
        scope.stop(35);
        assert_eq!(scope.stopped_at, Some(29));
        assert!(scope.clip(30, 1).is_err());
        assert_eq!(scope.grace_remaining(38), 1);
        assert_eq!(scope.grace_remaining(39), 0);
        let mut delayed = Scope::new(17, 43, 71, 10).unwrap();
        delayed.stop(65);
        assert_eq!(delayed.grace_remaining(69), 1);
        assert_eq!(delayed.grace_remaining(70), 0);
    }

    #[test]
    fn configured_test_intervals_cannot_raise_proposed_limits() {
        assert!(Scope::new(0, 300_000, 600_000, 10_000).is_ok());
        assert!(Scope::new(0, 300_001, 600_000, 10_000).is_err());
        assert!(Scope::new(0, 300_000, 600_001, 10_000).is_err());
        assert!(Scope::new(0, 300_000, 600_000, 10_001).is_err());
        assert!(Scope::new(0, 0, 600_000, 10_000).is_err());
        assert!(Scope::new(u64::MAX, 1, 1, 1).is_err());
    }

    #[cfg(target_os = "macos")]
    mod native {
        use super::*;
        use std::io::{Read, Write};
        use std::os::unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
            process::{CommandExt, ExitStatusExt},
        };
        use std::process::{Child, Command, Stdio};
        use std::time::{Duration, Instant};

        struct Control {
            child: Child,
            owners: [UnixStream; 2],
            barrier: Option<UnixStream>,
            root: std::path::PathBuf,
            deadline: u64,
        }
        impl Control {
            fn start(role: &str) -> Self {
                let start = monotonic_ms();
                let root = std::path::Path::new("/tmp")
                    .join(format!("sr-anchor-{}-{start}-{role}", std::process::id()));
                std::fs::create_dir(&root).unwrap();
                std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
                let barrier_listener = UnixListener::bind(root.join("barrier.sock")).unwrap();
                let preparation_ms = if (role.starts_with("spawn-blocked")
                    && !role.contains("owner"))
                    || role == "partial-transition"
                {
                    800
                } else {
                    8_000
                };
                let paths = [root.join("supervisor.sock"), root.join("broker.sock")];
                let listeners = paths.each_ref().map(|p| UnixListener::bind(p).unwrap());
                let child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        &format!(
                            "{}::native_anchor_process",
                            module_path!().split_once("::").unwrap().1
                        ),
                        "--ignored",
                        "--nocapture",
                    ])
                    .env_clear()
                    .env("PATH", "/usr/bin:/bin")
                    .env("ANCHOR_TEST_START", start.to_string())
                    .env("ANCHOR_TEST_PREPARATION", preparation_ms.to_string())
                    .env("ANCHOR_TEST_BARRIER", root.join("barrier.sock"))
                    .env("ANCHOR_TEST_ROLE", role)
                    .env("ANCHOR_TEST_SUPERVISOR", &paths[0])
                    .env("ANCHOR_TEST_BROKER", &paths[1])
                    .process_group(0)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                let owners = listeners.map(|l| {
                    l.set_nonblocking(true).unwrap();
                    let until = Instant::now() + Duration::from_secs(4);
                    let stream = loop {
                        match l.accept() {
                            Ok((s, _)) => break s,
                            Err(e)
                                if e.kind() == std::io::ErrorKind::WouldBlock
                                    && Instant::now() < until =>
                            {
                                std::thread::sleep(Duration::from_millis(5))
                            }
                            other => panic!("anchor did not register: {other:?}"),
                        }
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_millis(100)))
                        .unwrap();
                    stream
                });
                let mut control = Self {
                    child,
                    owners,
                    barrier: None,
                    root,
                    deadline: start + preparation_ms,
                };
                if role.starts_with("spawn-blocked") {
                    barrier_listener.set_nonblocking(true).unwrap();
                    let until = Instant::now() + Duration::from_millis(500);
                    let stream = loop {
                        match barrier_listener.accept() {
                            Ok((stream, _)) => break stream,
                            Err(e)
                                if e.kind() == std::io::ErrorKind::WouldBlock
                                    && Instant::now() < until =>
                            {
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            other => panic!("anchor did not connect fault barrier: {other:?}"),
                        }
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    control.barrier = Some(stream);
                }
                for owner in &mut control.owners {
                    let (event, pid) = Self::receive(owner);
                    assert_eq!((event, pid), (6, u64::from(control.child.id())));
                    Self::send(owner, 5, start + preparation_ms);
                }
                control
            }
            fn send(stream: &mut UnixStream, op: u8, value: u64) {
                stream
                    .write_all(&[&[op], value.to_le_bytes().as_slice()].concat())
                    .unwrap();
            }
            fn receive(stream: &mut UnixStream) -> (u8, u64) {
                let mut frame = [0; 9];
                stream.read_exact(&mut frame).unwrap();
                (frame[0], u64::from_le_bytes(frame[1..].try_into().unwrap()))
            }
            fn wait_killed(&mut self) {
                self.wait_killed_within(1_400);
            }
            fn wait_killed_within(&mut self, milliseconds: u64) {
                let until = Instant::now() + Duration::from_millis(milliseconds);
                let status = loop {
                    if let Some(status) = self.child.try_wait().unwrap() {
                        break status;
                    }
                    assert!(Instant::now() < until, "native anchor failed to terminate");
                    for owner in &mut self.owners {
                        let _ = owner.write_all(&[1, 0, 0, 0, 0, 0, 0, 0, 0]);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                };
                assert_eq!(status.signal(), Some(9), "anchor status: {status}");
                let until = Instant::now() + Duration::from_secs(1);
                while !group_absent(self.child.id() as i32) {
                    assert!(Instant::now() < until, "execution group quiescence unknown");
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }

        impl Drop for Control {
            fn drop(&mut self) {
                for owner in &self.owners {
                    let _ = owner.shutdown(std::net::Shutdown::Both);
                }
                if let Some(barrier) = self.barrier.take() {
                    let _ = barrier.shutdown(std::net::Shutdown::Both);
                }
                let until = Instant::now() + Duration::from_secs(2);
                while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(5));
                }
                for name in [
                    "supervisor.sock",
                    "broker.sock",
                    "barrier.sock",
                    "descendant.pid",
                    "descendant.blocked",
                    "descendant.fragment",
                ] {
                    let _ = std::fs::remove_file(self.root.join(name));
                }
                let _ = std::fs::remove_dir(&self.root);
            }
        }

        #[test]
        #[ignore = "isolated native anchor subprocess entry"]
        fn native_anchor_process() {
            let owners = ["ANCHOR_TEST_SUPERVISOR", "ANCHOR_TEST_BROKER"]
                .map(|key| UnixStream::connect(std::env::var(key).unwrap()).unwrap());
            let role = std::env::var("ANCHOR_TEST_ROLE").unwrap();
            let mut payload = Command::new("/bin/sh");
            payload
                .args([
                    "-c",
                    match role.as_str() {
                        "clean" => "exit 0",
                        role if role.starts_with("spawn-blocked") => {
                            "echo admitted > \"$PID_FILE\"; exit 0"
                        }
                        "descendant" => "sleep 60 & echo $! > \"$PID_FILE\"; exit 0",
                        _ => "sleep 60",
                    },
                ])
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env(
                    "PID_FILE",
                    std::path::Path::new(&std::env::var("ANCHOR_TEST_BARRIER").unwrap())
                        .with_file_name("descendant.pid"),
                );
            if role == "blocked-node" {
                // Fixed, disposable loopback payload; no Node options, HOME,
                // credentials, evaluator data or application configuration.
                payload = Command::new("/opt/homebrew/bin/node");
                payload.args(["-e", r#"
                    const fs = require('node:fs');
                    const net = require('node:net');
                    const server = net.createServer(socket => {
                        const expected = Buffer.from('GET /owned-mock-control HTTP/1.1\r\nHost: localhost\r\n\r\n');
                        let request = Buffer.alloc(0);
                        const receive = chunk => {
                            if (request.length + chunk.length > expected.length) process.exit(79);
                            request = Buffer.concat([request, chunk]);
                            if (request.length < expected.length) {
                                fs.writeFileSync(process.env.PID_FILE.replace(/\.pid$/, '.fragment'), 'partial');
                                return;
                            }
                            if (!request.equals(expected)) process.exit(79);
                            socket.off('data', receive);
                            socket.write('HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nready', () => {
                                fs.writeFileSync(process.env.PID_FILE.replace(/\.pid$/, '.blocked'), 'blocked');
                                while (true) {}
                            });
                        };
                        socket.on('data', receive);
                    });
                    server.listen(0, '127.0.0.1', () => fs.writeFileSync(process.env.PID_FILE, String(server.address().port)));
                "#]).env_clear().env("PATH", "/usr/bin:/bin").env("PID_FILE",
                    std::path::Path::new(&std::env::var("ANCHOR_TEST_BARRIER").unwrap()).with_file_name("descendant.pid"));
            }
            if role.starts_with("spawn-blocked") && role != "spawn-blocked-no-barrier" {
                use std::os::fd::AsRawFd;
                let barrier =
                    UnixStream::connect(std::env::var("ANCHOR_TEST_BARRIER").unwrap()).unwrap();
                // Async-signal-safe fault barrier: Command::spawn cannot return
                // until this actual child pre-exec hook is released.
                unsafe {
                    payload.pre_exec(move || {
                        extern "C" {
                            fn read(fd: i32, data: *mut u8, size: usize) -> isize;
                            fn write(fd: i32, data: *const u8, size: usize) -> isize;
                        }
                        let fd = barrier.as_raw_fd();
                        let mut byte = 0;
                        if write(fd, &1_u8, 1) != 1 || read(fd, &mut byte, 1) != 1 || byte != 1 {
                            return Err(std::io::Error::from_raw_os_error(22));
                        }
                        Ok(())
                    });
                }
            }
            let scope = Scope::new(
                std::env::var("ANCHOR_TEST_START").unwrap().parse().unwrap(),
                std::env::var("ANCHOR_TEST_PREPARATION")
                    .unwrap()
                    .parse()
                    .unwrap(),
                8_000,
                1_000,
            )
            .unwrap();
            run_anchor(payload, owners, scope);
        }

        #[test]
        fn native_deadline_survives_blocked_spawn_without_releasing_barrier() {
            let mut c = Control::start("spawn-blocked-deadline");
            let mut ready = [0];
            c.barrier.as_mut().unwrap().read_exact(&mut ready).unwrap();
            assert_eq!(ready, [1]);
            assert!(monotonic_ms() < c.deadline);
            c.wait_killed();
            assert!(
                !c.root.join("descendant.pid").exists(),
                "payload effects must stay behind the native startup gate"
            );
            assert!(
                c.barrier.as_mut().unwrap().write_all(&[1]).is_err(),
                "late release must have no surviving child reader"
            );
        }

        #[test]
        fn native_owner_loss_survives_blocked_spawn() {
            for owner in 0..2 {
                let mut c = Control::start(&format!("spawn-blocked-owner-{owner}"));
                let mut ready = [0];
                c.barrier.as_mut().unwrap().read_exact(&mut ready).unwrap();
                assert_eq!(ready, [1]);
                assert!(
                    c.deadline - monotonic_ms() > 6_000,
                    "preparation expiry must not masquerade as EOF enforcement"
                );
                let lost_at = monotonic_ms();
                c.owners[owner].shutdown(std::net::Shutdown::Both).unwrap();
                c.wait_killed_within(500);
                assert!(
                    monotonic_ms() - lost_at < 900,
                    "EOF must stop before silent-lease expiry"
                );
                assert!(!c.root.join("descendant.pid").exists());
            }
        }

        #[test]
        fn native_missing_fault_barrier_fails_bounded_startup() {
            let start = Instant::now();
            let result = std::panic::catch_unwind(|| Control::start("spawn-blocked-no-barrier"));
            let panic = match result {
                Err(panic) => panic,
                Ok(_) => panic!("missing barrier unexpectedly admitted startup"),
            };
            let reason = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("");
            assert!(
                reason.starts_with("anchor did not connect fault barrier:"),
                "unexpected startup failure: {reason}"
            );
            assert!(start.elapsed() < Duration::from_millis(1_500));
        }

        #[test]
        fn native_startup_gate_positive_control_releases_one_payload() {
            let mut c = Control::start("spawn-blocked-positive");
            let mut ready = [0];
            c.barrier.as_mut().unwrap().read_exact(&mut ready).unwrap();
            assert_eq!(ready, [1]);
            assert!(!c.root.join("descendant.pid").exists());
            c.barrier.as_mut().unwrap().write_all(&[1]).unwrap();
            for owner in &mut c.owners {
                assert_eq!(Control::receive(owner).0, 7);
                assert_eq!(Control::receive(owner), (8, 0));
                Control::send(owner, 3, 0);
            }
            assert_eq!(
                std::fs::read_to_string(c.root.join("descendant.pid")).unwrap(),
                "admitted\n"
            );
            for owner in &mut c.owners {
                assert_eq!(Control::receive(owner), (9, 1));
            }
            c.wait_killed();
        }

        #[test]
        fn native_anchor_stops_on_either_owner_loss_without_payload_progress() {
            for lost in 0..2 {
                let mut c = Control::start(if lost == 0 {
                    "lost-supervisor"
                } else {
                    "lost-broker"
                });
                for owner in &mut c.owners {
                    assert_eq!(Control::receive(owner).0, 7);
                }
                c.owners[lost].shutdown(std::net::Shutdown::Both).unwrap();
                c.wait_killed();
            }
        }

        #[test]
        fn native_anchor_only_retires_a_genuinely_empty_payload_group() {
            let mut c = Control::start("clean");
            for owner in &mut c.owners {
                assert_eq!(Control::receive(owner).0, 7);
                assert_eq!(Control::receive(owner), (8, 0));
                Control::send(owner, 3, 0);
            }
            for owner in &mut c.owners {
                assert_eq!(Control::receive(owner), (9, 1));
            }
            c.wait_killed();
        }

        #[test]
        fn native_anchor_keeps_descendants_until_deadline_and_never_claims_retirement() {
            let mut c = Control::start("descendant");
            for owner in &mut c.owners {
                assert_eq!(Control::receive(owner).0, 7);
                assert_eq!(Control::receive(owner), (8, 0));
            }
            let descendant: i32 = std::fs::read_to_string(c.root.join("descendant.pid"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            extern "C" {
                fn getpgid(pid: i32) -> i32;
            }
            assert_eq!(unsafe { getpgid(descendant) }, c.child.id() as i32);
            let deadline = monotonic_ms() + 400;
            for owner in &mut c.owners {
                Control::send(owner, 2, deadline);
                Control::send(owner, 3, 0);
            }
            let positive_until = monotonic_ms() + 150;
            while monotonic_ms() < positive_until {
                assert!(
                    c.child.try_wait().unwrap().is_none(),
                    "unintended early anchor termination"
                );
                assert_eq!(
                    unsafe { getpgid(descendant) },
                    c.child.id() as i32,
                    "descendant must still inhabit the registered group before deadline"
                );
                for owner in &mut c.owners {
                    Control::send(owner, 1, 0);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(monotonic_ms() < deadline);
            c.wait_killed();
            assert!(
                monotonic_ms() >= deadline,
                "termination must follow the matched transition deadline"
            );
            for owner in &mut c.owners {
                let mut remaining = Vec::new();
                let _ = owner.read_to_end(&mut remaining);
                assert!(
                    remaining.is_empty(),
                    "leaked descendant admitted normal retirement"
                );
            }
        }

        #[test]
        fn native_anchor_rejects_mismatched_duplicate_and_past_transitions() {
            for fault in ["mismatch", "duplicate", "past"] {
                let mut c = Control::start(fault);
                for owner in &mut c.owners {
                    assert_eq!(Control::receive(owner).0, 7);
                }
                let deadline = monotonic_ms() + 500;
                Control::send(
                    &mut c.owners[0],
                    2,
                    if fault == "past" {
                        monotonic_ms() - 1
                    } else {
                        deadline
                    },
                );
                Control::send(
                    &mut c.owners[1],
                    2,
                    if fault == "mismatch" {
                        deadline + 1
                    } else {
                        deadline
                    },
                );
                if fault == "duplicate" {
                    Control::send(&mut c.owners[0], 2, deadline);
                }
                c.wait_killed();
                assert!(
                    monotonic_ms() < deadline,
                    "invalid transition must stop promptly, not merely time out"
                );
            }
        }

        #[test]
        fn native_fragmented_frame_and_single_retirement_do_not_end_clean_payload() {
            let mut c = Control::start("clean");
            for owner in &mut c.owners {
                assert_eq!(Control::receive(owner).0, 7);
                assert_eq!(Control::receive(owner), (8, 0));
            }
            c.owners[0].write_all(&[1, 0, 0]).unwrap();
            std::thread::sleep(Duration::from_millis(20));
            assert!(c.child.try_wait().unwrap().is_none());
            c.owners[0].write_all(&[0; 6]).unwrap();
            Control::send(&mut c.owners[0], 3, 0);
            std::thread::sleep(Duration::from_millis(40));
            assert!(
                c.child.try_wait().unwrap().is_none(),
                "one retirement is not joint authorization"
            );
            Control::send(&mut c.owners[1], 3, 0);
            for owner in &mut c.owners {
                assert_eq!(Control::receive(owner), (9, 1));
            }
            c.wait_killed();
        }

        #[test]
        fn native_eof_mid_frame_never_releases_another_effect() {
            let mut c = Control::start("partial-eof");
            for owner in &mut c.owners {
                assert_eq!(Control::receive(owner).0, 7);
            }
            c.owners[0].write_all(&[2, 0, 0]).unwrap();
            c.owners[0].shutdown(std::net::Shutdown::Write).unwrap();
            c.wait_killed();
        }

        #[test]
        fn native_partial_transition_cannot_extend_preparation() {
            let mut c = Control::start("partial-transition");
            for owner in &mut c.owners {
                assert_eq!(Control::receive(owner).0, 7);
            }
            let execution_deadline = monotonic_ms() + 4_000;
            Control::send(&mut c.owners[0], 2, execution_deadline);
            c.wait_killed();
            assert!(monotonic_ms() >= c.deadline);
            assert!(monotonic_ms() < execution_deadline);
        }

        #[test]
        fn native_silent_owner_lease_expires_while_the_other_owner_is_active() {
            for silent in 0..2 {
                let mut c = Control::start("silent-owner");
                for owner in &mut c.owners {
                    assert_eq!(Control::receive(owner).0, 7);
                }
                let start = monotonic_ms();
                while c.child.try_wait().unwrap().is_none() && monotonic_ms() - start < 1_300 {
                    let _ = c.owners[1 - silent].write_all(&[1, 0, 0, 0, 0, 0, 0, 0, 0]);
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(
                    c.child.try_wait().unwrap().is_some(),
                    "an open channel is not liveness"
                );
                assert!(
                    monotonic_ms() - start >= 900,
                    "unintended early termination is not lease enforcement"
                );
                c.wait_killed();
            }
        }

        #[test]
        fn native_execution_deadline_stops_blocked_node_after_observed_loopback_wire() {
            let mut c = Control::start("blocked-node");
            let mut payload_id = 0;
            for owner in &mut c.owners {
                let (event, pid) = Control::receive(owner);
                assert_eq!(event, 7);
                if payload_id != 0 {
                    assert_eq!(payload_id, pid);
                }
                payload_id = pid;
            }
            extern "C" {
                fn getpgid(pid: i32) -> i32;
            }
            assert_eq!(unsafe { getpgid(payload_id as i32) }, c.child.id() as i32);
            let until = Instant::now() + Duration::from_millis(500);
            let port_file = c.root.join("descendant.pid");
            while !port_file.exists() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                port_file.exists(),
                "Node must positively open the loopback listener"
            );
            let port: u16 = std::fs::read_to_string(&port_file)
                .unwrap()
                .parse()
                .unwrap();
            let mut wire = std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                Duration::from_millis(200),
            )
            .unwrap();
            wire.set_read_timeout(Some(Duration::from_millis(300)))
                .unwrap();
            wire.write_all(b"GET /owned-mock-control HTTP/1.1\r\n")
                .unwrap();
            let fragment = port_file.with_extension("fragment");
            let until = Instant::now() + Duration::from_millis(200);
            while !fragment.exists() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                std::fs::read_to_string(fragment).unwrap(),
                "partial",
                "the fixture must observe, not assume away, a fragmented TCP request"
            );
            wire.write_all(b"Host: localhost\r\n\r\n").unwrap();
            let response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nready";
            let mut bytes = vec![0; response.len()];
            wire.read_exact(&mut bytes).unwrap();
            assert_eq!(bytes, response);
            let blocked = port_file.with_extension("blocked");
            let until = Instant::now() + Duration::from_millis(200);
            while !blocked.exists() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(std::fs::read_to_string(blocked).unwrap(), "blocked");
            let deadline = monotonic_ms() + 300;
            for owner in &mut c.owners {
                Control::send(owner, 2, deadline);
            }
            let positive_until = monotonic_ms() + 100;
            while monotonic_ms() < positive_until {
                assert!(c.child.try_wait().unwrap().is_none());
                for owner in &mut c.owners {
                    Control::send(owner, 1, 0);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            c.wait_killed();
            assert!(monotonic_ms() >= deadline);
            eprintln!("blocked Node payload={payload_id} anchor/group={} observed HTTP 200 ready before absolute deadline={deadline}; SIGKILL, reaped anchor and ESRCH confirmed", c.child.id());
        }
    }
}
