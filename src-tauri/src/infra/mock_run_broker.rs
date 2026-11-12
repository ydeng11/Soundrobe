//! Loopback-only forwarding owner for the explicitly selected mock evaluator.
use super::{Command, Journal, Stage};
use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::StatusCode,
    response::Response,
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::future::IntoFuture;
use std::os::fd::AsRawFd;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::Semaphore,
};

const IPC_BYTES: u64 = 1024 * 1024;
struct Shared {
    journal: Journal,
    owner: String,
    stopped: Arc<AtomicBool>,
    admission: std::sync::Mutex<()>,
    forwarding: Arc<Semaphore>,
    http: reqwest::Client,
    _registry_lock: std::fs::File,
    // Unit-control seam after a real SQLite commit; true selects dispatch.
    // Unset in broker construction and never bootstrap-selectable.
    after_journal: Option<Box<dyn Fn(bool) + Send + Sync>>,
}
impl Shared {
    fn command(&self, handle: &str, command: Command) -> Result<Value, String> {
        #[cfg(target_os = "macos")]
        if let Some(monitor) = super::supervisor::ownership::broker_monitor() {
            let _ = monitor.check();
        }
        let stop = matches!(command, Command::Stop | Command::IngressDenied { .. });
        if stop {
            // Revocation cannot wait for a held journal/admission lock. Durable
            // terminal bookkeeping may follow; it never reopens this latch.
            self.stopped.store(true, Ordering::SeqCst);
        }
        // Covers the fallback latch too: SQLite cannot serialize admissions
        // against an error whose terminal state could not reach storage.
        let _admission = self
            .admission
            .lock()
            .map_err(|_| "mock_authority_stopped")?;
        let reconcile = matches!(
            command,
            Command::Arrival { .. }
                | Command::Complete { .. }
                | Command::Stop
                | Command::EndLease { .. }
                | Command::IngressDenied { .. }
        );
        if self.stopped.load(Ordering::SeqCst) && !reconcile {
            return Err("mock_authority_stopped".into());
        }
        let dispatch = matches!(command, Command::Dispatch { .. });
        #[cfg(target_os = "macos")]
        let consume = matches!(command, Command::Consume { .. });
        if matches!(command, Command::Freeze)
            && std::env::var("SOUNDROBE_NATIVE_CONTROL").as_deref() == Ok("journal-lock")
        {
            let root = std::path::PathBuf::from(
                std::env::var_os("SOUNDROBE_NATIVE_OUTPUT").ok_or("mock_control_missing")?,
            );
            std::fs::write(
                root.join("journal-entered"),
                b"before actual SQLite transaction",
            )
            .map_err(|_| "mock_control_storage")?;
        }
        let result = self.journal.command(handle, command);
        #[cfg(target_os = "macos")]
        let result = result.and_then(|value| {
            if consume {
                if let Some(monitor) = super::supervisor::ownership::broker_monitor() {
                    monitor.enter_execution()?;
                }
            }
            Ok(value)
        });
        if result.is_ok() && !reconcile {
            if let Some(barrier) = &self.after_journal {
                barrier(dispatch);
            }
        }
        if stop || result.is_err() {
            self.stopped.store(true, Ordering::SeqCst);
            // Failure to persist stop is itself terminal in this broker. Startup
            // never reopens an existing registry for continued execution.
            if !stop {
                let _ = self.journal.command(&self.owner, Command::Stop);
            }
        }
        // A durable commit may return after independent revocation. It remains
        // consumed, but its result cannot release a new effect/session grant.
        #[cfg(target_os = "macos")]
        if let Some(monitor) = super::supervisor::ownership::broker_monitor() {
            let _ = monitor.check();
        }
        if result.is_ok() && self.stopped.load(Ordering::SeqCst) && !reconcile {
            Err("mock_authority_stopped".into())
        } else {
            result
        }
    }
}

pub(crate) struct Broker {
    pub(crate) descriptor: Value,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    socket_root: std::path::PathBuf,
}
impl Drop for Broker {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        let _ = std::fs::remove_file(self.socket_root.join("control.sock"));
        let _ = std::fs::remove_dir(&self.socket_root);
    }
}
impl Broker {
    pub(crate) async fn start(journal: Journal, owner: String) -> Result<Self, String> {
        use std::os::unix::fs::PermissionsExt;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(journal.path.with_extension("lock"))
            .map_err(|_| "mock_registry_unavailable")?;
        // Kernel-held advisory lock: process death releases ownership, but never
        // grants execution recovery. This is not a malicious-host boundary.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("mock_registry_owned".into());
        }
        journal.command(&owner, Command::ClaimBroker)?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|_| "mock_transport_unavailable")?;
        let stopped = Arc::new(AtomicBool::new(false));
        #[cfg(target_os = "macos")]
        let stopped = super::supervisor::ownership::broker_monitor()
            .map_or(stopped, |monitor| monitor.stopped.clone());
        let shared = Arc::new(Shared {
            journal,
            owner: owner.clone(),
            stopped,
            admission: std::sync::Mutex::new(()),
            forwarding: Arc::new(Semaphore::new(16)),
            http,
            _registry_lock: lock,
            after_journal: None,
        });
        // Darwin's Unix socket path limit is shorter than an artifact path.
        let socket_root = std::path::Path::new("/tmp").join(format!("sr-mock-{}", super::token()));
        std::fs::create_dir(&socket_root).map_err(|_| "mock_socket_unavailable")?;
        std::fs::set_permissions(&socket_root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "mock_socket_unavailable")?;
        let socket = socket_root.join("control.sock");
        let ipc = tokio::net::UnixListener::bind(&socket).map_err(|_| "mock_socket_unavailable")?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "mock_socket_unavailable")?;
        let gateway = format!(
            "http://{}",
            listener
                .local_addr()
                .map_err(|_| "mock_socket_unavailable")?
        );
        let http_shared = shared.clone();
        let http_task = tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                Router::new().fallback(forward).with_state(http_shared),
            )
            .await;
        });
        let ipc_task = tokio::spawn(async move {
            let slots = Arc::new(Semaphore::new(16));
            while let Ok((stream, _)) = ipc.accept().await {
                let Ok(permit) = slots.clone().try_acquire_owned() else {
                    let _ = shared.command(&shared.owner, Command::Stop);
                    continue;
                };
                let state = shared.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    control(stream, state).await;
                });
            }
            let _ = shared.command(&shared.owner, Command::Stop);
        });
        Ok(Self {
            descriptor: json!({"mode":"durable_mock_only","gateway":gateway,"socket":socket,"owner":owner}),
            tasks: vec![http_task, ipc_task],
            socket_root,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    handle: String,
    command: Value,
}

async fn control(stream: tokio::net::UnixStream, state: Arc<Shared>) {
    let mut reader = BufReader::new(stream.take(IPC_BYTES + 1));
    let mut bytes = Vec::new();
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.read_until(b'\n', &mut bytes),
    )
    .await;
    let mut watch = None;
    let result = async {
        if !matches!(read,Ok(Ok(n)) if n>0 && n<=IPC_BYTES as usize) || bytes.last() != Some(&b'\n')
        {
            return Err("mock_ipc_invalid".into());
        }
        let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|_| "mock_ipc_invalid")?;
        #[cfg(target_os = "macos")]
        if envelope.command == json!({"op":"scope"}) {
            super::require(envelope.handle == state.owner)?;
            let monitor =
                super::supervisor::ownership::broker_monitor().ok_or("mock_scope_unavailable")?;
            return Ok(json!({"remainingMs":monitor.remaining()?}));
        }
        if envelope.command == json!({"op":"report"}) {
            super::require(envelope.handle == state.owner)?;
            return state.journal.report();
        }
        let command: Command =
            serde_json::from_value(envelope.command).map_err(|_| "mock_ipc_invalid")?;
        // Forwarding reservations and settlements are broker-internal, never
        // transferable send tickets returned to evaluated children.
        super::require(!matches!(
            command,
            Command::Reserve { .. }
                | Command::Complete { .. }
                | Command::ClaimBroker
                | Command::Dispatch { .. }
                | Command::EndLease { .. }
        ))?;
        if let Command::Lease { role } = &command {
            watch = Some((envelope.handle.clone(), role.clone()));
        }
        let stop = matches!(command, Command::Stop);
        let value = state.command(&envelope.handle, command)?;
        let supervised = {
            #[cfg(target_os = "macos")]
            {
                super::supervisor::ownership::broker_monitor().is_some()
            }
            #[cfg(not(target_os = "macos"))]
            {
                false
            }
        };
        if stop && !supervised {
            let _quiescent = tokio::time::timeout(
                std::time::Duration::from_secs(6),
                state.forwarding.acquire_many(16),
            )
            .await
            .map_err(|_| "mock_stop_quiescence_unknown")?
            .map_err(|_| "mock_stop_quiescence_unknown")?;
        }
        Ok::<Value, String>(value)
    }
    .await;
    if result.is_err() {
        let _ = state.command(&state.owner, Command::Stop);
    }
    let admitted = result.is_ok();
    let mut bytes = serde_json::to_vec(&match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(code) => json!({"ok":false,"code":code}),
    })
    .unwrap();
    if bytes.len() > IPC_BYTES as usize {
        bytes = br#"{"ok":false,"code":"mock_ipc_oversize"}"#.to_vec();
        let _ = state.command(&state.owner, Command::Stop);
    }
    bytes.push(b'\n');
    if reader.get_mut().get_mut().write_all(&bytes).await.is_err() {
        let _ = state.command(&state.owner, Command::Stop);
        return;
    }
    if let Some((handle, role)) = watch.filter(|_| admitted) {
        let mut release = Vec::new();
        reader.get_mut().set_limit(8);
        let eof_is_release = {
            #[cfg(target_os = "macos")]
            {
                super::supervisor::ownership::broker_monitor().is_none()
            }
            #[cfg(not(target_os = "macos"))]
            {
                true
            }
        };
        let valid = reader.read_until(b'\n', &mut release).await.is_ok()
            && ((eof_is_release && release.is_empty()) || release == b"release\n");
        let result = if valid {
            state.command(&handle, Command::EndLease { role })
        } else {
            Err("mock_lease_invalid".into())
        };
        if result.is_err() {
            let _ = state.command(&state.owner, Command::Stop);
        }
        if !release.is_empty() {
            let bytes = if result.is_ok() {
                b"{\"ok\":true}\n".as_slice()
            } else {
                b"{\"ok\":false}\n".as_slice()
            };
            if reader.get_mut().get_mut().write_all(bytes).await.is_err() {
                let _ = state.command(&state.owner, Command::Stop);
            }
        }
    }
}

async fn forward(State(state): State<Arc<Shared>>, request: Request) -> Response {
    let result = async {
        let _in_flight = state
            .forwarding
            .acquire()
            .await
            .map_err(|_| "mock_transport_stopped")?;
        let handle = request
            .headers()
            .get("x-soundrobe-mock-session")
            .and_then(|v| v.to_str().ok())
            .ok_or("mock_session_missing")?
            .to_string();
        let method = request.method().clone();
        let uri = request.uri();
        // Absolute proxy requests must still target this loopback broker. No
        // CONNECT, redirects, arbitrary hostnames or environment proxy fallback.
        super::require(uri.host().is_none_or(|h| h == "127.0.0.1"))?;
        let path = uri
            .path_and_query()
            .ok_or("mock_route_invalid")?
            .as_str()
            .to_string();
        let body = to_bytes(request.into_body(), 65536)
            .await
            .map_err(|_| "mock_request_oversize")?;
        let reservation = state.command(
            &handle,
            Command::Reserve {
                method: method.as_str().into(),
                path: path.clone(),
                body_sha256: super::sha(&body),
            },
        )?;
        let operation = reservation["operation"].as_str().unwrap().to_string();
        if state.stopped.load(Ordering::SeqCst) {
            return Err("mock_transport_stopped".into());
        }
        // This transaction is the forwarding-admission point, ordered against
        // durable stop. A reservation alone grants no permission to forward.
        state.command(
            &state.owner,
            Command::Dispatch {
                operation: operation.clone(),
            },
        )?;
        let mut response = state
            .http
            .request(
                method,
                reqwest::Url::parse(reservation["url"].as_str().unwrap())
                    .map_err(|_| "mock_route_invalid")?,
            )
            .header("x-soundrobe-mock-operation", &operation)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| "mock_delivery_unknown")?;
        let status = response.status();
        let headers = response.headers().clone();
        let max = if path == "/__fixtures/compact" {
            8388608
        } else if path == "/model/chat/completions" {
            65536
        } else {
            8 * 1024 * 1024
        };
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "mock_delivery_unknown")?
        {
            super::require(bytes.len() + chunk.len() <= max)?;
            bytes.extend_from_slice(&chunk);
        }
        state.command(&state.owner, Command::Complete { operation })?;
        let mut builder = Response::builder().status(status);
        for name in [
            "content-type",
            "retry-after",
            "x-ratelimit-limit",
            "x-ratelimit-remaining",
            "x-ratelimit-used",
        ] {
            if let Some(value) = headers.get(name) {
                builder = builder.header(name, value);
            }
        }
        Ok::<Response, String>(builder.body(Body::from(bytes)).unwrap())
    }
    .await;
    match result {
        Ok(response) => response,
        Err(_) => {
            let _ = state.command(&state.owner, Command::Stop);
            Response::builder()
                .status(StatusCode::CONFLICT)
                .body(Body::from(r#"{"error":"mock_authority_denied"}"#))
                .unwrap()
        }
    }
}

#[test]
fn mock_authority_stop_revokes_admission_before_a_held_journal_gate_returns() {
    let (root, journal, owner) = super::tests::setup();
    let state = Arc::new(Shared {
        journal: journal.clone(),
        owner: owner.clone(),
        stopped: Arc::new(AtomicBool::new(false)),
        admission: std::sync::Mutex::new(()),
        forwarding: Arc::new(Semaphore::new(16)),
        http: reqwest::Client::builder().no_proxy().build().unwrap(),
        _registry_lock: std::fs::File::create(root.join("control-test.lock")).unwrap(),
        after_journal: None,
    });
    let held = state.admission.lock().unwrap();
    let request = state.clone();
    let (entered, ready) = std::sync::mpsc::sync_channel(0);
    let stop = std::thread::spawn(move || {
        entered.send(()).unwrap();
        request.command(&request.owner, Command::Stop)
    });
    ready
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while !state.stopped.load(Ordering::SeqCst) && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let revoked_while_held = state.stopped.load(Ordering::SeqCst);
    // Always release/join before assertions, including the expected red run.
    drop(held);
    stop.join().unwrap().unwrap();
    assert!(
        revoked_while_held,
        "a stalled journal/admission gate must not stall native revocation"
    );
    assert_eq!(
        journal.report().unwrap()["membership"]["requestedFieldSlots"],
        216
    );
    assert_eq!(journal.report().unwrap()["state"], "terminal");
}

#[test]
fn mock_authority_late_committed_admission_is_consumed_but_never_acknowledged_after_stop() {
    let (root, journal, owner) = super::tests::setup();
    let (arrived, committed) = std::sync::mpsc::sync_channel(1);
    let (release, waiting) = std::sync::mpsc::sync_channel(1);
    let waiting = std::sync::Mutex::new(waiting);
    let state = Arc::new(Shared {
        journal: journal.clone(),
        owner: owner.clone(),
        stopped: Arc::new(AtomicBool::new(false)),
        admission: std::sync::Mutex::new(()),
        forwarding: Arc::new(Semaphore::new(16)),
        http: reqwest::Client::builder().no_proxy().build().unwrap(),
        _registry_lock: std::fs::File::create(root.join("return-control.lock")).unwrap(),
        after_journal: Some(Box::new(move |_| {
            arrived.send(()).unwrap();
            waiting
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap();
        })),
    });
    let request = state.clone();
    let admission = std::thread::spawn(move || {
        request.command(
            &request.owner,
            Command::Register {
                job: "disabled-crowd".into(),
                stage: Stage::Preparation,
                endpoint: "http://127.0.0.1:1234".into(),
                manifest_sha256: super::sha(b"fixture"),
            },
        )
    });
    committed
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    let request = state.clone();
    let stop = std::thread::spawn(move || request.command(&request.owner, Command::Stop));
    let until = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while !state.stopped.load(Ordering::SeqCst) && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let revoked = state.stopped.load(Ordering::SeqCst);
    release.send(()).unwrap();
    let acknowledged = admission.join().unwrap();
    stop.join().unwrap().unwrap();
    assert!(revoked);
    assert_eq!(
        acknowledged,
        Err("mock_authority_stopped".into()),
        "a commit returning after revocation must not yield a usable session handle"
    );
    let report = journal.report().unwrap();
    assert_eq!(report["state"], "terminal");
    assert_eq!(
        report["sessions"].as_object().unwrap().len(),
        1,
        "stop is not rollback or refund"
    );
    assert_eq!(report["membership"]["requestedFieldSlots"], 216);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mock_authority_revoked_post_commit_dispatch_never_reaches_fixture_wire() {
    use std::sync::atomic::AtomicUsize;
    for revoke in [false, true] {
        let (root, journal, owner) = super::tests::setup();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let session = journal
            .command(
                &owner,
                Command::Register {
                    job: "disabled-crowd".into(),
                    stage: Stage::Preparation,
                    endpoint,
                    manifest_sha256: super::sha(b"fixture"),
                },
            )
            .unwrap();
        let service = session["service"].as_str().unwrap().to_string();
        journal
            .command(
                &service,
                Command::Lease {
                    role: "fixture".into(),
                },
            )
            .unwrap();
        let arrivals = Arc::new(AtomicUsize::new(0));
        let counted = arrivals.clone();
        let observed_journal = journal.clone();
        let fixture = tokio::spawn(
            axum::serve(
                listener,
                Router::new().fallback(move |request: Request| {
                    let counted = counted.clone();
                    let journal = observed_journal.clone();
                    let service = service.clone();
                    async move {
                        counted.fetch_add(1, Ordering::SeqCst);
                        let operation = request.headers()["x-soundrobe-mock-operation"]
                            .to_str()
                            .unwrap()
                            .to_string();
                        let digest = super::sha(
                            &serde_json::to_vec(&json!([
                                request.method().as_str(),
                                request.uri().path_and_query().unwrap().as_str(),
                                super::sha(b"")
                            ]))
                            .unwrap(),
                        );
                        journal
                            .command(
                                &service,
                                Command::Arrival {
                                    operation,
                                    request_sha256: digest,
                                },
                            )
                            .unwrap();
                        Response::new(Body::from("{}"))
                    }
                }),
            )
            .into_future(),
        );
        let (arrived, committed) = std::sync::mpsc::sync_channel(1);
        let (release, waiting) = std::sync::mpsc::sync_channel(1);
        let waiting = std::sync::Mutex::new(waiting);
        let state = Arc::new(Shared {
            journal: journal.clone(),
            owner: owner.clone(),
            stopped: Arc::new(AtomicBool::new(false)),
            admission: std::sync::Mutex::new(()),
            forwarding: Arc::new(Semaphore::new(16)),
            http: reqwest::Client::builder().no_proxy().build().unwrap(),
            _registry_lock: std::fs::File::create(root.join("dispatch-control.lock")).unwrap(),
            after_journal: Some(Box::new(move |dispatch| {
                if dispatch {
                    arrived.send(()).unwrap();
                    waiting
                        .lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(1))
                        .unwrap();
                }
            })),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gateway = format!(
            "http://{}/__fixtures/compact",
            listener.local_addr().unwrap()
        );
        let broker = tokio::spawn(
            axum::serve(
                listener,
                Router::new().fallback(forward).with_state(state.clone()),
            )
            .into_future(),
        );
        let client = session["client"].as_str().unwrap().to_string();
        let request = tokio::spawn(async move {
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(gateway)
                .header("x-soundrobe-mock-session", client)
                .send()
                .await
                .unwrap()
        });
        committed
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        let report = journal.report().unwrap();
        assert_eq!(report["counters"]["reserved"], 1);
        assert!(report["operations"]
            .as_object()
            .unwrap()
            .values()
            .all(|o| o["dispatched"] == true));
        assert_eq!(
            arrivals.load(Ordering::SeqCst),
            0,
            "the actual wire must still be empty at the barrier"
        );
        let stop = if revoke {
            let state = state.clone();
            Some(std::thread::spawn(move || {
                state.command(&state.owner, Command::Stop)
            }))
        } else {
            None
        };
        if revoke {
            let until = std::time::Instant::now() + std::time::Duration::from_millis(200);
            while !state.stopped.load(Ordering::SeqCst) && std::time::Instant::now() < until {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        let revoked_before_release = state.stopped.load(Ordering::SeqCst);
        release.send(()).unwrap();
        let response = request.await.unwrap();
        if let Some(stop) = stop {
            stop.join().unwrap().unwrap();
        }
        let status = response.status();
        let received = arrivals.load(Ordering::SeqCst);
        let report = journal.report().unwrap();
        fixture.abort();
        broker.abort();
        assert_eq!(revoked_before_release, revoke);
        assert_eq!(
            status,
            if revoke {
                StatusCode::CONFLICT
            } else {
                StatusCode::OK
            }
        );
        assert_eq!(
            received,
            usize::from(!revoke),
            "positive and revoked controls must differ at the actual loopback wire"
        );
        assert_eq!(
            report["counters"]["reserved"], 1,
            "a committed dispatch is never refunded"
        );
        assert_eq!(report["counters"]["arrived"], usize::from(!revoke));
        assert_eq!(report["counters"]["completed"], usize::from(!revoke));
        assert_eq!(report["membership"]["requestedFieldSlots"], 216);
        eprintln!("post-commit dispatch revoke={revoke} HTTP={status} actual-fixture-arrivals={received} reserved=1 slots=216");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mock_authority_broker_reserves_before_wire_and_stops_without_an_extra_arrival() {
    use axum::{
        body::{to_bytes, Body},
        extract::Request,
        response::Response,
        Router,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    let (_root, j, owner) = super::tests::setup();
    let service = Arc::new(Mutex::new(String::new()));
    let arrivals = Arc::new(AtomicUsize::new(0));
    let state = (j.clone(), service.clone(), arrivals.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let fixture = tokio::spawn(
        axum::serve(
            listener,
            Router::new().fallback(move |request: Request| {
                let (journal, service, arrivals) = state.clone();
                async move {
                    arrivals.fetch_add(1, Ordering::SeqCst);
                    let operation = request.headers()["x-soundrobe-mock-operation"]
                        .to_str()
                        .unwrap()
                        .to_string();
                    let method = request.method().as_str().to_string();
                    let path = request.uri().path_and_query().unwrap().as_str().to_string();
                    let body = to_bytes(request.into_body(), 65536).await.unwrap();
                    let digest = super::sha(
                        &serde_json::to_vec(&json!([method, path, super::sha(&body)])).unwrap(),
                    );
                    assert_eq!(journal.report().unwrap()["counters"]["reserved"], 1);
                    journal
                        .command(
                            &service.lock().unwrap(),
                            Command::Arrival {
                                operation,
                                request_sha256: digest,
                            },
                        )
                        .unwrap();
                    Response::new(Body::from("{}"))
                }
            }),
        )
        .into_future(),
    );
    let broker = Broker::start(j.clone(), owner.clone()).await.unwrap();
    let checkpoint = super::tests::freeze(&j, &owner);
    j.command(
        &owner,
        Command::Consume {
            fake_checkpoint_sha256: checkpoint,
        },
    )
    .unwrap();
    let session = j
        .command(
            &owner,
            Command::Register {
                job: "disabled-billie".into(),
                stage: Stage::Execution,
                endpoint,
                manifest_sha256: super::sha(b"fixture"),
            },
        )
        .unwrap();
    *service.lock().unwrap() = session["service"].as_str().unwrap().into();
    for (key, role) in [("client", "native"), ("service", "fixture")] {
        j.command(
            session[key].as_str().unwrap(),
            Command::Lease { role: role.into() },
        )
        .unwrap();
    }
    j.command(
        session["client"].as_str().unwrap(),
        Command::Attach {
            executable_sha256: super::sha(b"test executable"),
            preparation_sha256: Some(super::sha(b"disabled-billie")),
        },
    )
    .unwrap();
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let target = format!(
        "{}/musicbrainz/ws/2/release/test",
        broker.descriptor["gateway"].as_str().unwrap()
    );
    let response = http
        .get(&target)
        .header(
            "x-soundrobe-mock-session",
            session["client"].as_str().unwrap(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(j.report().unwrap()["counters"]["completed"], 1);
    let client = super::Client {
        socket: broker.descriptor["socket"].as_str().unwrap().into(),
        handle: session["client"].as_str().unwrap().into(),
    };
    let guard = client
        .scope(async { crate::infra::bounded_eval::write_guard() })
        .await;
    guard().unwrap();
    let mut ipc = tokio::net::UnixStream::connect(broker.descriptor["socket"].as_str().unwrap())
        .await
        .unwrap();
    ipc.write_all(format!("{}\n", json!({"handle":owner,"command":{"op":"stop"}})).as_bytes())
        .await
        .unwrap();
    let mut receipt = String::new();
    BufReader::new(ipc).read_line(&mut receipt).await.unwrap();
    assert_eq!(serde_json::from_str::<Value>(&receipt).unwrap()["ok"], true);
    assert!(
        tokio::task::spawn_blocking(guard).await.unwrap().is_err(),
        "blocking writer ignored global stop"
    );
    let denied = http
        .get(&target)
        .header(
            "x-soundrobe-mock-session",
            session["client"].as_str().unwrap(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 409);
    assert_eq!(arrivals.load(Ordering::SeqCst), 1);
    fixture.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mock_authority_broker_cannot_restart_a_previously_owned_registry() {
    let (_root, j, owner) = super::tests::setup();
    let first = Broker::start(j.clone(), owner.clone()).await.unwrap();
    assert!(Broker::start(j.clone(), owner.clone()).await.is_err());
    drop(first);
    let probe = std::fs::OpenOptions::new()
        .write(true)
        .open(j.path.with_extension("lock"))
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_UN) }, 0);
    drop(probe);
    assert!(
        Broker::start(j.clone(), owner).await.is_err(),
        "restarted an owned identity"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mock_authority_owner_disconnect_is_terminal_before_any_wire() {
    let (_root, j, owner) = super::tests::setup_unleased();
    let broker = Broker::start(j.clone(), owner.clone()).await.unwrap();
    let mut stream = tokio::net::UnixStream::connect(broker.descriptor["socket"].as_str().unwrap())
        .await
        .unwrap();
    stream
        .write_all(
            format!(
                "{}\n",
                json!({"handle":owner,"command":{"op":"lease","role":"owner"}})
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut receipt = String::new();
    reader.read_line(&mut receipt).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&receipt).unwrap()["ok"],
        true,
        "owner liveness session unavailable"
    );
    drop(reader);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while j.report().unwrap()["state"] != "terminal" {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        j.report().unwrap()["membership"]["requestedFieldSlots"],
        216
    );
    assert_eq!(j.report().unwrap()["counters"]["reserved"], 0);
}

#[cfg(target_os = "macos")]
pub(crate) async fn supervised_process(
    root: std::path::PathBuf,
    monitor: Arc<super::supervisor::ownership::Monitor>,
) -> Result<(), String> {
    use super::supervisor::ownership::receiver;
    let directory = root.join("authority");
    let bootstrap = directory.join("bootstrap.json");
    while !bootstrap.is_file() {
        monitor.check()?;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let io_error = |_| "mock_supervised_storage_unavailable".to_string();
    let (mut input, _) = receiver::read_json(&bootstrap).map_err(io_error)?;
    let (config, _) =
        receiver::read_json(&root.join("native-supervision.json")).map_err(io_error)?;
    super::require(
        input["membership"] == serde_json::from_str::<Value>(receiver::MEMBERSHIP).unwrap()
            && input["bindings"]["supervision"] == config,
    )?;
    input["bindings"]["supervision"]["executionGroup"] =
        json!(std::env::var("SOUNDROBE_NATIVE_GROUP")
            .map_err(|_| "mock_group_missing")?
            .parse::<i32>()
            .map_err(|_| "mock_group_invalid")?);
    monitor.check()?;
    let (journal, owner) = Journal::create(
        &directory.join("registry.sqlite"),
        input["membership"].clone(),
        input["bindings"].clone(),
    )?;
    let broker = Broker::start(journal.clone(), owner).await?;
    receiver::durable_json(&directory, "ready.json", &broker.descriptor).map_err(io_error)?;
    if std::env::var("SOUNDROBE_NATIVE_CONTROL").as_deref() == Ok("stalled-storage") {
        use std::io::Read;
        while !root.join("storage-release").is_file() {
            monitor.check()?;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let mut fifo = std::fs::File::open(root.join("storage-fifo")).map_err(io_error)?;
        std::fs::write(
            root.join("storage-read-started"),
            b"open FIFO, before blocking read",
        )
        .map_err(io_error)?;
        let mut byte = [0];
        // Actual blocking disposable FIFO I/O; no journal or watchdog lock is
        // held, and native owner polling must continue independently.
        fifo.read_exact(&mut byte).map_err(io_error)?;
        return Err("mock_storage_control_unexpectedly_resumed".into());
    }
    loop {
        monitor.check()?;
        let report = journal.report()?;
        if report["state"] == "terminal" {
            return Err("mock_supervised_terminal".into());
        }
        if report["state"] == "provisional_complete"
            && report["ownerLive"] == false
            && monitor.payload_exited()
        {
            let (_, result) = receiver::read_json(&root.join("results.json")).map_err(io_error)?;
            super::require(report["resultSha256"] == receiver::digest_string(&result))?;
            // All payload writers have actually exited. This normal staging is
            // inside execution, never a postdeadline audit/drain operation.
            let mut pending = vec![root.clone()];
            let mut directories = Vec::new();
            let mut files = 0;
            let mut bytes = 0_u64;
            while let Some(directory) = pending.pop() {
                directories.push(directory.clone());
                for entry in std::fs::read_dir(&directory).map_err(io_error)? {
                    monitor.check()?;
                    let entry = entry.map_err(io_error)?;
                    let kind = entry.file_type().map_err(io_error)?;
                    if kind.is_dir() {
                        pending.push(entry.path());
                    } else if kind.is_file() {
                        files += 1;
                        bytes = bytes.saturating_add(entry.metadata().map_err(io_error)?.len());
                        super::require(files <= 4096 && bytes <= 512 * 1024 * 1024)?;
                        receiver::fault_barrier(&root, "final-file-sync");
                        std::fs::File::open(entry.path())
                            .map_err(io_error)?
                            .sync_all()
                            .map_err(io_error)?;
                    } else {
                        return Err("mock_staging_non_regular_file".into());
                    }
                }
            }
            for directory in directories.into_iter().rev() {
                monitor.check()?;
                std::fs::File::open(directory)
                    .map_err(io_error)?
                    .sync_all()
                    .map_err(io_error)?;
            }
            monitor.check()?;
            receiver::durable_json(&directory, "provisional.json", &report).map_err(io_error)?;
            monitor.check()?;
            monitor.stage();
            // Native retirement owns exit; no six-second async drain or final
            // filesystem-success marker is permitted in this supervised path.
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit isolated mock broker process, never an application entry point"]
async fn mock_authority_process() {
    let bootstrap = std::path::PathBuf::from(
        std::env::var("SOUNDROBE_MOCK_AUTHORITY_BOOTSTRAP").expect("explicit bootstrap"),
    );
    let anchor = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".amp/in/artifacts")
        .canonicalize()
        .unwrap();
    assert!(bootstrap.canonicalize().unwrap().starts_with(&anchor));
    let bytes = std::fs::read(&bootstrap).unwrap();
    assert!(bytes.len() <= 1024 * 1024);
    let input: Value = serde_json::from_slice(&bytes).unwrap();
    let root = bootstrap.parent().unwrap();
    let (mut journal, owner) = Journal::create(
        &root.join("registry.sqlite"),
        input["membership"].clone(),
        input["bindings"].clone(),
    )
    .unwrap();
    journal.fault = serde_json::from_value(input.get("fault").cloned().unwrap_or(Value::Null))
        .expect("invalid explicit mock crash selector");
    let broker = Broker::start(journal.clone(), owner).await.unwrap();
    let ready = root.join("ready.json");
    std::fs::write(
        root.join("ready.tmp"),
        serde_json::to_vec(&broker.descriptor).unwrap(),
    )
    .unwrap();
    std::fs::rename(root.join("ready.tmp"), ready).unwrap();
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let report = journal.report().unwrap();
        if report["state"] == "complete" || report["state"] == "terminal" {
            // Allow already-admitted arrivals/settlements to reconcile. This is
            // a bounded process drain, not supervision or physical cancellation.
            tokio::time::sleep(std::time::Duration::from_secs(6)).await;
            std::fs::write(
                root.join("final.json"),
                serde_json::to_vec_pretty(&journal.report().unwrap()).unwrap(),
            )
            .unwrap();
            break;
        }
    }
    drop(broker);
}
