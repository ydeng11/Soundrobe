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

#[path = "mock_run_transport.rs"]
mod transport;
const IPC_BYTES: u64 = 1024 * 1024;
struct Shared {
    journal: Journal,
    owner: String,
    stopped: Arc<AtomicBool>,
    admission: std::sync::Mutex<()>,
    approval_expires: std::sync::atomic::AtomicU64,
    forwarding: Arc<Semaphore>,
    http: reqwest::Client,
    transport: transport::Transport,
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
        let dispatch = matches!(
            command,
            Command::Dispatch { .. } | Command::ModelSendAdmission { .. }
        );
        let consume = matches!(command, Command::Approve { .. });
        if matches!(command, Command::Freeze)
            && std::env::var("SOUNDROBE_NATIVE_CONTROL").as_deref() == Ok("journal-lock")
        {
            let root = std::path::PathBuf::from(
                std::env::var_os("SOUNDROBE_NATIVE_OUTPUT").ok_or("mock_control_missing")?,
            );
            super::storage::write_new_or_unscoped(
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
        if consume {
            if let Ok(value) = &result {
                // Immutable expiry comes only from the verified durable claim,
                // not an IPC field or mutable caller-supplied clock setting.
                self.approval_expires.store(
                    value["expiresUnixSeconds"].as_u64().unwrap(),
                    Ordering::SeqCst,
                );
            }
        }
        if result.is_ok() && !reconcile {
            if let Some(barrier) = &self.after_journal {
                barrier(dispatch);
            }
        }
        let result = result.and_then(|value| {
            let expiry = self.approval_expires.load(Ordering::SeqCst);
            if !reconcile && expiry != 0 {
                super::require(super::policy::now()? < expiry)?;
            }
            Ok(value)
        });
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
        Self::with_transport(journal, owner, transport::Transport::default(), None).await
    }
    async fn with_transport(
        journal: Journal,
        owner: String,
        transport: transport::Transport,
        after_journal: Option<Box<dyn Fn(bool) + Send + Sync>>,
    ) -> Result<Self, String> {
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
            .retry(reqwest::retry::never())
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
            approval_expires: std::sync::atomic::AtomicU64::new(0),
            forwarding: Arc::new(Semaphore::new(16)),
            http,
            transport,
            _registry_lock: lock,
            after_journal,
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
            let mut report = state.journal.report()?;
            report["actualFakeLookups"] = json!(state.transport.lookups());
            return Ok(report);
        }
        let command: Command =
            serde_json::from_value(envelope.command).map_err(|_| "mock_ipc_invalid")?;
        let command = match command {
            Command::Genre { input } => {
                return state.transport.genre(&state, &envelope.handle, input).await
            }
            other => other,
        };
        // Forwarding reservations and settlements are broker-internal, never
        // transferable send tickets returned to evaluated children.
        super::require(!matches!(
            command,
            Command::Reserve { .. }
                | Command::Consume { .. }
                | Command::CredentialLookup { .. }
                | Command::ModelSendAdmission { .. }
                | Command::ModelReceipt { .. }
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
        // Even an exact model body cannot bypass typed broker IPC.
        super::require(method == reqwest::Method::GET && !path.starts_with("/model"))?;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_authority_transport_requires_authenticated_approval_not_a_checkpoint_echo() {
    let (_root, journal, owner) = super::tests::setup();
    let broker = Broker::start(journal.clone(), owner.clone()).await.unwrap();
    let checkpoint = super::tests::freeze(&journal, &owner);
    let mut socket = tokio::net::UnixStream::connect(broker.descriptor["socket"].as_str().unwrap())
        .await
        .unwrap();
    socket
        .write_all(
            format!(
                "{}\n",
                json!({"handle":owner,"command":{
                    "op":"consume", "fake_checkpoint_sha256":checkpoint
                }})
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut reply = String::new();
    BufReader::new(socket).read_line(&mut reply).await.unwrap();
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(
        reply["ok"], false,
        "a caller's checkpoint echo is not authenticated approval"
    );
    let report = journal.report().unwrap();
    assert_eq!(report["consumptions"], 0);
    assert_eq!(report["counters"]["model"], 0);
    assert_eq!(report["mutationAdmissions"], 0);
}

// Independent, deliberately public rehearsal seed. Never user approval.
fn rehearsal_setup() -> (std::path::PathBuf, Journal, String) {
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let signer = Ed25519KeyPair::from_seed_unchecked(&[42; 32]).unwrap();
    let input = serde_json::from_value(json!({"artist":"Asymmetric Artist","album":"Different Album","tracks":["Track One","Other Two"]})).unwrap();
    let rehearsal = super::policy::Rehearsal::new(
        &input,
        "~deepseek/deepseek-v4-flash-latest",
        "deepseek/deepseek-v4-flash-0731",
        "wafer/us",
        "Wafer",
        signer.public_key().as_ref().try_into().unwrap(),
    )
    .unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.amp/in/artifacts/model-rehearsal-20261113")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).unwrap();
    let membership = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json"
    ))
    .unwrap();
    let (journal, owner) = Journal::create_rehearsal(
        &root.join("registry.sqlite"),
        membership,
        json!({"executable":super::sha(b"test executable")}),
        rehearsal,
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
    (root, journal, owner)
}
fn rehearsal_approval(report: &Value) -> Value {
    use ring::signature::Ed25519KeyPair;
    let scope = json!({"runId":report["runId"], "checkpoint":report["checkpoint"],
        "bindings":report["bindings"], "membership":report["membership"],
        "preparations":report["preparations"], "policy":super::policy::REHEARSAL_POLICY});
    let payload = json!({"version":1,"runId":report["runId"],"checkpoint":report["checkpoint"],
        "scopeSha256":super::sha(&serde_json::to_vec(&scope).unwrap()),
        "nonce":uuid::Uuid::new_v4().to_string(), "expiresUnixSeconds":super::policy::now().unwrap()+1200});
    let signature = Ed25519KeyPair::from_seed_unchecked(&[42; 32])
        .unwrap()
        .sign(&serde_json::to_vec(&payload).unwrap())
        .as_ref()
        .to_vec();
    json!({"payload":payload,"signature":signature})
}

// All signing material here is the public RFC 8032 test vector, not user trust.
fn fixture_approval(report: &Value) -> Value {
    let scope = json!({"runId":report["runId"], "checkpoint":report["checkpoint"],
        "bindings":report["bindings"], "membership":report["membership"],
        "preparations":report["preparations"],
        "policy":"offline-retained-evidence-v1;loopback-scripted-genre-v1;no-live;no-retry;no-repair"});
    let payload = json!({"version":1, "runId":report["runId"], "checkpoint":report["checkpoint"],
        "scopeSha256":super::sha(&serde_json::to_vec(&scope).unwrap()),
        "nonce":uuid::Uuid::new_v4().to_string(), "expiresUnixSeconds":
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()+1200});
    sign_fixture_payload(payload)
}
fn sign_fixture_payload(payload: Value) -> Value {
    use ring::signature::Ed25519KeyPair;
    let seed = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    let signature = Ed25519KeyPair::from_seed_unchecked(&seed)
        .unwrap()
        .sign(&serde_json::to_vec(&payload).unwrap())
        .as_ref()
        .to_vec();
    json!({"payload":payload,"signature":signature})
}
async fn control_request(broker: &Broker, handle: &str, command: Value) -> Value {
    let mut socket = tokio::net::UnixStream::connect(broker.descriptor["socket"].as_str().unwrap())
        .await
        .unwrap();
    socket
        .write_all(format!("{}\n", json!({"handle":handle,"command":command})).as_bytes())
        .await
        .unwrap();
    let mut reply = String::new();
    BufReader::new(socket).read_line(&mut reply).await.unwrap();
    serde_json::from_str(&reply).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn model_rehearsal_approval_cannot_reuse_fixture_trust_or_recover_authority() {
    for damage in [
        "none",
        "fixture-key",
        "wrong-key",
        "scope",
        "expired",
        "nonce",
        "extra-key",
        "replay",
        "race",
        "missing-pin",
    ] {
        let (_root, mut journal, owner) = rehearsal_setup();
        let broker = Broker::start(journal.clone(), owner.clone()).await.unwrap();
        super::tests::freeze_request(
            &journal,
            &owner,
            Some(
                journal.rehearsal.as_ref().unwrap().binding()["requestSha256"]
                    .as_str()
                    .unwrap()
                    .into(),
            ),
        );
        let mut approval = rehearsal_approval(&journal.report().unwrap());
        if damage == "fixture-key" {
            approval = sign_fixture_payload(approval["payload"].clone());
        } else if ["scope", "expired", "nonce", "wrong-key"].contains(&damage) {
            use ring::signature::Ed25519KeyPair;
            match damage {
                "scope" => {
                    approval["payload"]["scopeSha256"] = json!(super::sha(b"other-settings"))
                }
                "expired" => approval["payload"]["expiresUnixSeconds"] = json!(1),
                "nonce" => approval["payload"]["nonce"] = json!("invalid"),
                _ => {}
            }
            let seed = if damage == "wrong-key" {
                [43; 32]
            } else {
                [42; 32]
            };
            approval["signature"] = json!(Ed25519KeyPair::from_seed_unchecked(&seed)
                .unwrap()
                .sign(&serde_json::to_vec(&approval["payload"]).unwrap())
                .as_ref()
                .to_vec());
        } else if damage == "extra-key" {
            approval["publicKey"] = json!(vec![42; 32]);
        }
        let command = json!({"op":"approve","approval":approval});
        let valid = ["none", "replay", "race"].contains(&damage);
        if damage == "missing-pin" {
            let before = std::fs::read(&journal.path).unwrap();
            journal.rehearsal = None;
            assert_eq!(
                journal.report().unwrap()["membership"]["requestedFieldSlots"],
                216
            );
            assert_eq!(
                std::fs::read(&journal.path).unwrap(),
                before,
                "read-only recovery changed the registry"
            );
            let command = serde_json::from_value(command).unwrap();
            assert!(
                journal.command(&owner, command).is_err(),
                "lost pin fell back to fixture trust"
            );
        } else if damage == "race" {
            let (a, b) = tokio::join!(
                control_request(&broker, &owner, command.clone()),
                control_request(&broker, &owner, command)
            );
            assert_eq!(
                usize::from(a["ok"] == true) + usize::from(b["ok"] == true),
                1
            );
        } else {
            assert_eq!(
                control_request(&broker, &owner, command.clone()).await["ok"],
                valid,
                "{damage}"
            );
            if damage == "replay" {
                assert_eq!(control_request(&broker, &owner, command).await["ok"], false);
            }
        }
        let report = journal.report().unwrap();
        assert_eq!(report["consumptions"], usize::from(valid), "{damage}");
        assert_eq!(report["credentialLookups"], 0);
        assert_eq!(report["counters"]["model"], 0);
        assert_eq!(report["membership"]["requestedFieldSlots"], 216);
        assert_eq!(report["liveAuthority"], false);
        assert_eq!(report["testPolicy"], super::policy::REHEARSAL_POLICY);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mock_policy_approval_is_pinned_exact_single_use_and_durable() {
    for damage in [
        "none",
        "signature",
        "expired",
        "scope",
        "nonce",
        "key",
        "checkpoint",
        "run",
        "extra",
        "race",
        "replay",
        "revoked",
    ] {
        let (_root, journal, owner) = super::tests::setup();
        let broker = Broker::start(journal.clone(), owner.clone()).await.unwrap();
        super::tests::freeze(&journal, &owner);
        let mut approval = fixture_approval(&journal.report().unwrap());
        match damage {
            "signature" => {
                approval["signature"][0] = json!(approval["signature"][0].as_u64().unwrap() ^ 1);
            }
            "expired" => {
                approval["payload"]["expiresUnixSeconds"] = json!(1);
                approval = sign_fixture_payload(approval["payload"].clone());
            }
            "scope" => {
                approval["payload"]["scopeSha256"] = json!(super::sha(b"substitute"));
                approval = sign_fixture_payload(approval["payload"].clone());
            }
            "nonce" => {
                approval["payload"]["nonce"] = json!("");
                approval = sign_fixture_payload(approval["payload"].clone());
            }
            "key" => approval["publicKey"] = json!(vec![0; 32]),
            "checkpoint" => approval["payload"]["checkpoint"] = json!(super::sha(b"other")),
            "run" => approval["payload"]["runId"] = json!("other"),
            "extra" => approval["payload"]["allowLive"] = json!(true),
            "revoked" => {
                control_request(&broker, &owner, json!({"op":"stop"})).await;
            }
            _ => {}
        }
        let command = json!({"op":"approve", "approval":approval});
        let first;
        if damage == "race" {
            let (a, b) = tokio::join!(
                control_request(&broker, &owner, command.clone()),
                control_request(&broker, &owner, command)
            );
            assert_eq!(
                usize::from(a["ok"] == true) + usize::from(b["ok"] == true),
                1
            );
            first = json!({"ok":true});
        } else {
            first = control_request(&broker, &owner, command.clone()).await;
            if damage == "replay" {
                assert_eq!(control_request(&broker, &owner, command).await["ok"], false);
            }
        }
        let valid = ["none", "race", "replay"].contains(&damage);
        assert_eq!(first["ok"], valid, "approval disposition: {damage}");
        let report = journal.report().unwrap();
        assert_eq!(report["consumptions"], usize::from(valid));
        assert_eq!(report["credentialLookups"], 0);
        assert_eq!(report["counters"]["model"], 0);
        assert_eq!(report["mutationAdmissions"], 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mock_policy_genre_returns_only_schema_with_one_exact_wire_and_no_gateway_send() {
    for mode in [
        "success",
        "low-confidence",
        "status",
        "schema",
        "empty",
        "truncated",
        "usage",
        "identity",
        "oversize",
        "redirect",
        "gateway",
        "input-drift",
        "extra-url",
        "forged-handle",
        "replay",
        "supplier-failure",
        "supplier-expiry",
        "supplier-revocation",
        "send-commit-expiry",
        "revoked-after-approval",
        "rehearsal-success",
        "rehearsal-low-confidence",
        "rehearsal-status",
        "rehearsal-schema",
        "rehearsal-usage",
        "rehearsal-identity",
        "rehearsal-provider",
        "rehearsal-credential",
        "rehearsal-input-drift",
        "rehearsal-replay",
        "rehearsal-supplier-revocation",
        "rehearsal-supplier-failure",
        "rehearsal-reasoning",
        "rehearsal-usage-limit",
        "rehearsal-truncated-wire",
    ] {
        let rehearsal = mode.starts_with("rehearsal-");
        let mode = mode.strip_prefix("rehearsal-").unwrap_or(mode);
        let (_root, journal, owner) = if rehearsal {
            rehearsal_setup()
        } else {
            super::tests::setup()
        };
        let supplier_journal = journal.clone();
        let supplier_owner = owner.clone();
        let supplier = transport::Transport::new(Box::new(move || {
            if mode == "supplier-revocation" {
                supplier_journal
                    .command(&supplier_owner, Command::Stop)
                    .unwrap();
            }
            if mode == "supplier-failure" {
                return Err("SUPPLIER_SECRET_CANARY");
            }
            if mode == "supplier-expiry" {
                std::thread::sleep(std::time::Duration::from_millis(2100));
            }
            Ok(if mode == "credential" {
                "sk-REAL_LOOKING_SECRET_CANARY"
            } else {
                "MOCK_ONLY_SECRET_CANARY"
            }
            .into())
        }));
        let sends = std::sync::atomic::AtomicUsize::new(0);
        let barrier = Box::new(move |dispatch| {
            if dispatch && sends.fetch_add(1, Ordering::SeqCst) == 1 && mode == "send-commit-expiry"
            {
                std::thread::sleep(std::time::Duration::from_millis(2100));
            }
        });
        let broker =
            Broker::with_transport(journal.clone(), owner.clone(), supplier, Some(barrier))
                .await
                .unwrap();
        let input = json!({"artist":"Asymmetric Artist","album":"Different Album","tracks":["Track One","Other Two"]});
        let mut body = json!({"model":"scripted/genre-v1","temperature":0.2,"max_tokens":1024,
            "reasoning":{"enabled":false},"provider":{"allow_fallbacks":false,"require_parameters":true},"stream":false,
            "messages":[{"role":"system","content":"Infer a conservative music genre from the supplied artist, album, and track titles. Return a concise Discogs-style comma-separated genre and confidence. Use low confidence when the evidence is insufficient."},
                {"role":"user","content":input.to_string()}],
            "response_format":{"type":"json_schema","json_schema":{"name":"GenreFillResponse","schema":{
                "type":"object","properties":{"genre":{"type":"string"},"confidence":{"type":"number"}},"required":["genre","confidence"]}}}});
        if rehearsal {
            body["model"] = json!("~deepseek/deepseek-v4-flash-latest");
            body["provider"]["only"] = json!(["wafer/us"]);
            body["provider"]["order"] = json!(["wafer/us"]);
        }
        // Preparation's independent request digest is the authorization, not a
        // URL/body supplied by the evaluated child at dispatch time.
        super::tests::freeze_request(
            &journal,
            &owner,
            Some(super::sha(body.to_string().as_bytes())),
        );
        let mut approval = if rehearsal {
            rehearsal_approval(&journal.report().unwrap())
        } else {
            fixture_approval(&journal.report().unwrap())
        };
        if ["supplier-expiry", "send-commit-expiry"].contains(&mode) {
            let mut payload = approval["payload"].clone();
            payload["expiresUnixSeconds"] = json!(super::policy::now().unwrap() + 2);
            approval = sign_fixture_payload(payload);
        }
        assert_eq!(
            control_request(&broker, &owner, json!({"op":"approve","approval":approval})).await
                ["ok"],
            true
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let session = journal
            .command(
                &owner,
                Command::Register {
                    job: "assistance-crowd".into(),
                    stage: Stage::Execution,
                    endpoint,
                    manifest_sha256: super::sha(b"fixture"),
                },
            )
            .unwrap();
        let client = session["client"].as_str().unwrap();
        let service = session["service"].as_str().unwrap().to_string();
        for (handle, role) in [(client, "native"), (service.as_str(), "fixture")] {
            journal
                .command(handle, Command::Lease { role: role.into() })
                .unwrap();
        }
        journal
            .command(
                client,
                Command::Attach {
                    executable_sha256: super::sha(b"test executable"),
                    preparation_sha256: Some(super::sha(b"assistance-crowd")),
                },
            )
            .unwrap();
        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = captured.clone();
        let responses = Arc::new(std::sync::Mutex::new(Vec::new()));
        let response_capture = responses.clone();
        let j = journal.clone();
        let fixture = tokio::spawn(axum::serve(listener,Router::new().fallback(move |request: Request| {
            let observed=observed.clone(); let j=j.clone(); let service=service.clone(); let response_capture=response_capture.clone();
            async move {
                let operation=request.headers()["x-soundrobe-mock-operation"].to_str().unwrap().to_string();
                assert_eq!(request.uri().path(),"/model/chat/completions");
                assert_eq!(request.headers()["authorization"],"Bearer MOCK_ONLY_SECRET_CANARY");
                let bytes=to_bytes(request.into_body(),65536).await.unwrap();
                observed.lock().unwrap().push(bytes.to_vec());
                j.command(&service,Command::Arrival {operation,request_sha256:super::sha(&serde_json::to_vec(&json!(["POST","/model/chat/completions",super::sha(&bytes)])).unwrap())}).unwrap();
                let mut response=json!({"model":"scripted/genre-v1","provider":"scripted-loopback-v1","choices":[{"finish_reason":"stop","message":{"content":"{\"genre\":\"ACCEPTED_CANARY\",\"confidence\":0.8}"}}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}});
                if rehearsal {
                    response["model"] = json!("deepseek/deepseek-v4-flash-0731");
                    response["provider"] = json!("Wafer");
                }
                match mode {
                    "provider" => response["provider"] = json!("UNAPPROVED_HOST_CANARY"),
                    "low-confidence" => response["choices"][0]["message"]["content"]=json!("{\"genre\":\"REJECTED_CANARY\",\"confidence\":0.59}"),
                    "schema" => response["choices"][0]["message"]["content"]=json!("RAW_SCHEMA_SECRET_CANARY"),
                    "empty" => response["choices"][0]["message"]["content"]=json!(""),
                    "truncated" => response["choices"][0]["finish_reason"]=json!("length"),
                    "usage" => {response.as_object_mut().unwrap().remove("usage");}
                    "usage-limit" => response["usage"] = json!({"prompt_tokens":7,"completion_tokens":1025,"total_tokens":1032}),
                    "reasoning" => response["usage"]["completion_tokens_details"] = json!({"reasoning_tokens":2}),
                    "identity" => response["model"]=json!("REMOTE_IDENTITY_CANARY"),
                    _=>{}
                }
                let status = if mode=="status" {429} else if mode=="redirect" {307} else {200};
                let bytes = if mode=="oversize" {"X".repeat(65537)} else if mode=="status" {"RAW_BODY_SECRET_CANARY".into()} else {response.to_string()};
                response_capture.lock().unwrap().push((status, bytes.clone()));
                let body = if mode == "truncated-wire" {
                    Body::from_stream(futures_util::stream::iter(vec![Ok::<_,std::io::Error>("partial"),Err(std::io::Error::other("DISCONNECTED_CANARY"))]))
                } else { Body::from(bytes) };
                Response::builder().status(status)
                    .header("x-private-canary","HEADER_SECRET_CANARY").header("location","http://127.0.0.1:1/never")
                    .body(body).unwrap()
            }
        })).into_future());
        if mode == "revoked-after-approval" {
            assert_eq!(
                control_request(&broker, &owner, json!({"op":"stop"})).await["ok"],
                true
            );
        }
        let reply = if mode == "gateway" {
            let http = reqwest::Client::builder().no_proxy().build().unwrap();
            let response = http
                .post(format!(
                    "{}/model/chat/completions",
                    broker.descriptor["gateway"].as_str().unwrap()
                ))
                .header("x-soundrobe-mock-session", client)
                .body(body.to_string())
                .send()
                .await
                .unwrap();
            assert!(!response.status().is_success());
            json!({"ok":false})
        } else {
            let mut command = json!({"op":"genre","input":input});
            if mode == "input-drift" {
                command["input"]["album"] = json!("substituted");
            }
            if mode == "extra-url" {
                command["url"] = json!("https://invalid.example/never");
            }
            let handle = if mode == "forged-handle" {
                "forged"
            } else {
                client
            };
            let reply = control_request(&broker, handle, command.clone()).await;
            if mode == "replay" {
                assert_eq!(control_request(&broker, handle, command).await["ok"], false);
            }
            reply
        };
        let accepted = ["success", "replay"].contains(&mode);
        assert_eq!(
            reply["ok"],
            accepted || mode == "low-confidence",
            "{mode}: {reply}"
        );
        if accepted {
            assert_eq!(
                reply["value"],
                json!({"genre":"ACCEPTED_CANARY","confidence":0.8})
            );
        } else {
            assert!(!reply.to_string().contains("CANARY"), "{mode}: {reply}");
        }
        let no_lookup = [
            "gateway",
            "input-drift",
            "extra-url",
            "forged-handle",
            "revoked-after-approval",
        ]
        .contains(&mode);
        let no_wire = no_lookup
            || [
                "supplier-failure",
                "supplier-expiry",
                "supplier-revocation",
                "send-commit-expiry",
                "credential",
            ]
            .contains(&mode);
        let report = control_request(&broker, &owner, json!({"op":"report"})).await;
        assert_eq!(
            report["value"]["actualFakeLookups"],
            usize::from(!no_lookup)
        );
        assert_eq!(
            report["value"]["credentialLookups"],
            usize::from(!no_lookup)
        );
        assert!(!report.to_string().contains("CANARY"));
        assert_eq!(
            report["value"]["counters"]["model"],
            usize::from(!no_lookup)
        );
        assert_eq!(report["value"]["membership"]["requestedFieldSlots"], 216);
        if !no_wire {
            let operations = report["value"]["operations"].as_object().unwrap();
            let operation = operations.values().next().unwrap();
            let receipt = &operation["response"];
            if ["oversize", "truncated-wire"].contains(&mode) {
                assert!(receipt.is_null());
                assert_eq!(operation["completed"], false);
                assert_eq!(report["value"]["state"], "terminal");
            } else {
                let responses = responses.lock().unwrap();
                assert_eq!(responses.len(), 1);
                assert_eq!(receipt["sha256"], super::sha(responses[0].1.as_bytes()));
                assert_eq!(receipt["bytes"], responses[0].1.len());
                assert_eq!(receipt["status"], responses[0].0);
                assert_eq!(receipt["identityClaimsVerified"], false);
                assert_eq!(operation["completed"], true);
            }
        }
        assert!(
            !String::from_utf8_lossy(&std::fs::read(&journal.path).unwrap()).contains("CANARY")
        );
        let wires = captured.lock().unwrap();
        assert_eq!(wires.len(), usize::from(!no_wire), "{mode}");
        if !no_wire {
            assert_eq!(wires[0], body.to_string().as_bytes());
        }
        fixture.abort();
    }
}

#[test]
fn mock_policy_ambiguous_approval_crashes_never_reopen_or_refund() {
    for point in ["before_approval_commit", "after_approval_commit"] {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.amp/in/artifacts/mock-transport-20261113")
            .join(format!("approval-crash-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infra::mock_run_authority::broker::mock_policy_approval_crash_process",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("MOCK_APPROVAL_CRASH_ROOT", &root)
            .env("MOCK_APPROVAL_CRASH_POINT", point)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86), "{point}");
        let path =
            std::path::PathBuf::from(std::fs::read_to_string(root.join("registry-path")).unwrap());
        let j = Journal {
            path: path.clone(),
            fault: None,
            rehearsal: None,
        };
        let report = j.report().unwrap();
        assert_eq!(
            report["consumptions"],
            usize::from(point == "after_approval_commit")
        );
        assert_eq!(report["credentialLookups"], 0);
        assert_eq!(report["mutationAdmissions"], 0);
        let before = std::fs::read(&path).unwrap();
        assert!(Journal::create(&path, json!({}), json!({}), None).is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "recovery modified ambiguous evidence"
        );
    }
}
#[test]
#[ignore = "isolated offline approval crash control"]
fn mock_policy_approval_crash_process() {
    let root = std::path::PathBuf::from(std::env::var_os("MOCK_APPROVAL_CRASH_ROOT").unwrap());
    let (_r, mut j, owner) = super::tests::setup();
    super::tests::freeze(&j, &owner);
    std::fs::write(root.join("registry-path"), j.path.to_str().unwrap()).unwrap();
    j.fault = Some(
        serde_json::from_value(json!(std::env::var("MOCK_APPROVAL_CRASH_POINT").unwrap())).unwrap(),
    );
    let approval = serde_json::from_value(fixture_approval(&j.report().unwrap())).unwrap();
    j.command(&owner, Command::Approve { approval }).unwrap();
    panic!("ambiguous approval resumed");
}

#[test]
fn mock_authority_stop_revokes_admission_before_a_held_journal_gate_returns() {
    let (root, journal, owner) = super::tests::setup();
    let state = Arc::new(Shared {
        journal: journal.clone(),
        owner: owner.clone(),
        stopped: Arc::new(AtomicBool::new(false)),
        admission: std::sync::Mutex::new(()),
        approval_expires: std::sync::atomic::AtomicU64::new(0),
        forwarding: Arc::new(Semaphore::new(16)),
        http: reqwest::Client::builder().no_proxy().build().unwrap(),
        transport: transport::Transport::default(),
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
        approval_expires: std::sync::atomic::AtomicU64::new(0),
        forwarding: Arc::new(Semaphore::new(16)),
        http: reqwest::Client::builder().no_proxy().build().unwrap(),
        transport: transport::Transport::default(),
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
            approval_expires: std::sync::atomic::AtomicU64::new(0),
            forwarding: Arc::new(Semaphore::new(16)),
            http: reqwest::Client::builder().no_proxy().build().unwrap(),
            transport: transport::Transport::default(),
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
    // Launcher stderr may be discarded. Retain active broker/dependency TRACE
    // through the existing charged facade, without a user logging fallback.
    let trace = super::storage::files::File::create(root.join("broker-trace.log"))
        .map_err(|_| "mock_trace_storage_unavailable")?;
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(std::sync::Mutex::new(trace))
        .try_init()
        .map_err(|_| "mock_trace_unavailable")?;
    tracing::info!("mock_broker_privacy_trace_active");
    let directory = root.join("authority");
    let bootstrap = directory.join("bootstrap.json");
    while !super::storage::files::metadata(&bootstrap).is_ok_and(|metadata| metadata.is_file()) {
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
    let plan = input["bindings"].get("modelRehearsalPlan");
    super::require(plan.is_some() == std::env::var_os("SOUNDROBE_MODEL_REHEARSAL_PLAN").is_some())?;
    let (journal, owner) = if let Some(plan) = plan {
        let plan: super::policy::RehearsalPlan =
            serde_json::from_value(plan.clone()).map_err(|_| "mock_rehearsal_plan_invalid")?;
        Journal::create_rehearsal(
            &directory.join("registry.sqlite"),
            input["membership"].clone(),
            input["bindings"].clone(),
            plan.context()?,
        )?
    } else {
        Journal::create(
            &directory.join("registry.sqlite"),
            input["membership"].clone(),
            input["bindings"].clone(),
            None,
        )?
    };
    let broker = Broker::start(journal.clone(), owner).await?;
    receiver::durable_json(&directory, "ready.json", &broker.descriptor).map_err(io_error)?;
    if std::env::var("SOUNDROBE_NATIVE_CONTROL").as_deref() == Ok("stalled-storage") {
        use std::io::Read;
        while !super::storage::files::metadata(root.join("storage-release"))
            .is_ok_and(|metadata| metadata.is_file())
        {
            monitor.check()?;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let mut fifo = std::fs::File::open(root.join("storage-fifo")).map_err(io_error)?;
        super::storage::write_new_or_unscoped(
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
    // The native owner captures stderr through its bounded log sink. This
    // isolated process has no user logging/configuration fallback.
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(std::io::stderr)
        .try_init()
        .expect("isolated mock tracing");
    tracing::info!("mock_broker_privacy_trace_active");
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
    let fault = serde_json::from_value(input.get("fault").cloned().unwrap_or(Value::Null))
        .expect("invalid explicit mock crash selector");
    let (journal, owner) = Journal::create(
        &root.join("registry.sqlite"),
        input["membership"].clone(),
        input["bindings"].clone(),
        fault,
    )
    .unwrap();
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
