//! Isolated mock authority. Never compiled into an application binary.
#[cfg(all(feature = "server", unix))]
#[path = "mock_run_broker.rs"]
pub(crate) mod broker;
#[path = "mock_run_policy.rs"]
pub(crate) mod policy;
#[cfg(unix)]
#[path = "mock_run_storage.rs"]
pub(crate) mod storage;
#[cfg(all(feature = "server", target_os = "macos"))]
#[path = "mock_run_supervisor.rs"]
pub(crate) mod supervisor;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const JOBS: [&str; 6] = [
    "disabled-crowd",
    "disabled-billie",
    "disabled-enya",
    "assistance-billie",
    "assistance-enya",
    "assistance-crowd",
];
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest_valid(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
fn require(ok: bool) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err("mock_authority_denied".into())
    }
}
fn token() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage {
    Preparation,
    Execution,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Command {
    Register {
        job: String,
        stage: Stage,
        endpoint: String,
        manifest_sha256: String,
    },
    Prepared {
        preparation_sha256: String,
        request_sha256: Option<String>,
    },
    Finish {
        observation_sha256: String,
    },
    Close,
    Freeze,
    Publish,
    StageResult {
        result_sha256: String,
    },
    // Legacy in-process controls only; broker IPC always denies this operation.
    Consume {
        fake_checkpoint_sha256: String,
    },
    Approve {
        approval: policy::Approval,
    },
    CredentialLookup {
        operation: String,
    },
    ModelSendAdmission {
        operation: String,
    },
    Genre {
        input: policy::GenreInput,
    },
    Reserve {
        method: String,
        path: String,
        body_sha256: String,
    },
    Arrival {
        operation: String,
        request_sha256: String,
    },
    IngressDenied {
        reason: String,
    },
    Complete {
        operation: String,
    },
    ModelReceipt {
        operation: String,
        sha256: String,
        bytes: u32,
        status: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        http: Option<policy::boundary::HttpObservation>,
    },
    Attach {
        executable_sha256: String,
        preparation_sha256: Option<String>,
    },
    SealInputs,
    MutationAdmission,
    ClaimBroker,
    Lease {
        role: String,
    },
    EndLease {
        role: String,
    },
    Dispatch {
        operation: String,
    },
    Stop,
}

#[derive(Deserialize, Serialize)]
struct Run {
    run_id: String,
    owner: String,
    state: String,
    membership: Value,
    bindings: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bindings_sha256: Option<String>,
    sessions: BTreeMap<String, Session>,
    preparations: BTreeMap<String, Value>,
    operations: BTreeMap<String, Operation>,
    checkpoint: Option<String>,
    consumptions: u32,
    #[serde(default)]
    approval_expires: Option<u64>,
    #[serde(default)]
    approval_sha256: Option<String>,
    #[serde(default)]
    credential_lookups: u32,
    denied: u32,
    mutation_admissions: u32,
    broker_started: bool,
    #[serde(default)]
    owner_live: Option<bool>,
    #[serde(default)]
    ingress_denials: Vec<Value>,
    #[serde(default)]
    result_sha256: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
struct Session {
    job: String,
    stage: Stage,
    endpoint: String,
    manifest_sha256: String,
    client: String,
    service: String,
    closed: bool,
    finished: bool,
    attached: bool,
    sealed: bool,
    #[serde(default)]
    native_live: Option<bool>,
    #[serde(default)]
    fixture_live: Option<bool>,
    #[serde(default)]
    observation_sha256: Option<String>,
}
#[derive(Deserialize, Serialize)]
struct Operation {
    session: String,
    category: String,
    request_sha256: String,
    dispatched: bool,
    arrived: bool,
    completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response: Option<Value>,
}
impl Run {
    fn counters(&self) -> Value {
        let count = |category| {
            self.operations
                .values()
                .filter(|o| o.category == category)
                .count()
        };
        json!({"provider":count("provider"),"inventory":count("inventory"),"model":count("model"),
            "reserved":self.operations.len(),"arrived":self.operations.values().filter(|o|o.arrived).count(),
            "completed":self.operations.values().filter(|o|o.completed).count()})
    }
    fn apply(
        &mut self,
        handle: &str,
        command: Command,
        rehearsal: Option<&policy::Rehearsal>,
    ) -> Result<Value, String> {
        // A saved binding is not the separately pinned verifier. Never resume
        // a rehearsal under the default public-fixture trust after reopening.
        require(match rehearsal {
            Some(r) => self.bindings.get("modelRehearsal") == Some(&r.binding()),
            None => self.bindings.get("modelRehearsal").is_none(),
        })?;
        let native_response = rehearsal.is_some_and(|r| r.disabled_hosted());
        // Arrival/settlement can reconcile an already consumed operation after stop,
        // but never grant another operation or reopen an identity.
        require(
            self.state != "terminal"
                || matches!(
                    command,
                    Command::Arrival { .. }
                        | Command::Complete { .. }
                        | Command::Stop
                        | Command::EndLease { .. }
                        | Command::IngressDenied { .. }
                ),
        )?;
        if let Some(expiry) = self.approval_expires {
            if matches!(
                command,
                Command::Register { .. }
                    | Command::Reserve { .. }
                    | Command::Dispatch { .. }
                    | Command::CredentialLookup { .. }
                    | Command::ModelSendAdmission { .. }
                    | Command::MutationAdmission
                    | Command::Publish
                    | Command::StageResult { .. }
            ) {
                require(policy::now()? < expiry)?;
            }
        }
        let owner = handle == self.owner;
        let session = self.sessions.get(handle).cloned();
        let no_pending = self.operations.values().all(|o| o.completed);
        let session_settled = self
            .operations
            .values()
            .all(|o| o.session != handle || o.completed);
        let staged_result = match &command {
            Command::StageResult { result_sha256 } => {
                require(digest_valid(result_sha256))?;
                Some(result_sha256.clone())
            }
            _ => None,
        };
        match command {
            // Only the broker can perform this typed operation; it is not a
            // Journal grant or a transferable raw HTTP capability.
            Command::Genre { .. } => Err("mock_broker_required".into()),
            Command::Lease { role } => {
                if role == "owner" {
                    require(owner && self.owner_live.is_none() && self.state == "preparing")?;
                    self.owner_live = Some(true);
                } else if role == "native" {
                    let s = session.ok_or("mock_authority_denied")?;
                    require(!s.closed && !s.finished && s.native_live.is_none())?;
                    self.sessions.get_mut(handle).unwrap().native_live = Some(true);
                } else if role == "fixture" {
                    let s = self
                        .sessions
                        .values_mut()
                        .find(|s| s.service == handle)
                        .ok_or("mock_authority_denied")?;
                    require(!s.closed && s.fixture_live.is_none())?;
                    s.fixture_live = Some(true);
                } else {
                    return Err("mock_authority_denied".into());
                }
                Ok(json!({"leased":true}))
            }
            Command::EndLease { role } => {
                if role == "owner" {
                    require(owner && self.owner_live == Some(true))?;
                    self.owner_live = Some(false);
                    require(matches!(
                        self.state.as_str(),
                        "terminal" | "complete" | "provisional_complete"
                    ))?;
                } else if role == "native" {
                    let s = session.ok_or("mock_authority_denied")?;
                    require(s.native_live == Some(true))?;
                    self.sessions.get_mut(handle).unwrap().native_live = Some(false);
                    require(s.finished || self.state == "terminal")?;
                } else if role == "fixture" {
                    let s = self
                        .sessions
                        .values_mut()
                        .find(|s| s.service == handle)
                        .ok_or("mock_authority_denied")?;
                    require(s.fixture_live == Some(true))?;
                    s.fixture_live = Some(false);
                    require(s.closed || self.state == "terminal")?;
                } else {
                    return Err("mock_authority_denied".into());
                }
                Ok(json!({"released":true}))
            }
            Command::ClaimBroker => {
                require(owner && !self.broker_started && self.state == "preparing")?;
                self.broker_started = true;
                Ok(json!({"claimed":true}))
            }
            Command::Dispatch { operation } => {
                let o = self
                    .operations
                    .get_mut(&operation)
                    .ok_or("mock_authority_denied")?;
                let s = self.sessions.get(&o.session).unwrap();
                require(
                    owner
                        && !o.dispatched
                        && !o.arrived
                        && !s.closed
                        && self.state
                            == if s.stage == Stage::Preparation {
                                "preparing"
                            } else {
                                "executing"
                            },
                )?;
                o.dispatched = true;
                Ok(json!({"admitted":true}))
            }
            Command::Register {
                job,
                stage,
                endpoint,
                manifest_sha256,
            } => {
                require(
                    owner
                        && self.owner_live == Some(true)
                        && JOBS.contains(&job.as_str())
                        && digest_valid(&manifest_sha256),
                )?;
                let url = reqwest::Url::parse(&endpoint).map_err(|_| "mock_authority_denied")?;
                require(
                    url.scheme() == "http"
                        && url.host_str() == Some("127.0.0.1")
                        && url.port().is_some()
                        && url.path() == "/"
                        && url.query().is_none()
                        && url.fragment().is_none()
                        && url.username().is_empty()
                        && url.password().is_none(),
                )?;
                require(
                    self.state
                        == if stage == Stage::Preparation {
                            "preparing"
                        } else {
                            "executing"
                        },
                )?;
                require(
                    !self
                        .sessions
                        .values()
                        .any(|s| s.job == job && s.stage == stage),
                )?;
                if stage == Stage::Execution {
                    require(
                        no_pending
                            && self.preparations.contains_key(&job)
                            && self.sessions.values().any(|s| {
                                s.job == job
                                    && s.stage == Stage::Preparation
                                    && s.closed
                                    && s.manifest_sha256 == manifest_sha256
                            }),
                    )?;
                }
                let client = token();
                let service = token();
                self.sessions.insert(
                    client.clone(),
                    Session {
                        job,
                        stage,
                        endpoint,
                        manifest_sha256,
                        client: client.clone(),
                        service: service.clone(),
                        closed: false,
                        finished: false,
                        attached: false,
                        sealed: stage == Stage::Execution,
                        native_live: None,
                        fixture_live: None,
                        observation_sha256: None,
                    },
                );
                Ok(json!({"client":client,"service":service,"runId":self.run_id}))
            }
            Command::Prepared {
                preparation_sha256,
                request_sha256,
            } => {
                let s = session.ok_or("mock_authority_denied")?;
                require(
                    !s.closed
                        && !s.finished
                        && s.attached
                        && s.sealed
                        && s.native_live == Some(true)
                        && s.stage == Stage::Preparation
                        && self.state == "preparing"
                        && session_settled
                        && digest_valid(&preparation_sha256),
                )?;
                require(
                    request_sha256
                        .as_ref()
                        .is_none_or(|d| s.job == "assistance-crowd" && digest_valid(d)),
                )?;
                self.preparations.insert(
                    s.job,
                    json!({"preparationSha256":preparation_sha256,"requestSha256":request_sha256}),
                );
                self.sessions.get_mut(handle).unwrap().finished = true;
                Ok(json!({"prepared":true}))
            }
            Command::Finish { observation_sha256 } => {
                let s = session.ok_or("mock_authority_denied")?;
                require(
                    !s.closed
                        && !s.finished
                        && s.attached
                        && s.sealed
                        && s.native_live == Some(true)
                        && s.stage == Stage::Execution
                        && self.state == "executing"
                        && no_pending
                        && digest_valid(&observation_sha256),
                )?;
                let s = self.sessions.get_mut(handle).unwrap();
                s.finished = true;
                s.observation_sha256 = Some(observation_sha256.clone());
                Ok(json!({"finished":true,"observationSha256":observation_sha256}))
            }
            Command::Close => {
                let s = session.ok_or("mock_authority_denied")?;
                require(
                    !s.closed
                        && s.finished
                        && session_settled
                        && s.native_live == Some(false)
                        && s.fixture_live == Some(true),
                )?;
                self.sessions.get_mut(handle).unwrap().closed = true;
                Ok(json!({"closed":true}))
            }
            Command::Freeze => {
                require(
                    owner
                        && self.state == "preparing"
                        && no_pending
                        && self.preparations.len() == 6
                        && self.sessions.len() == 6
                        && self.sessions.values().all(|s| {
                            s.closed
                                && s.finished
                                && s.native_live == Some(false)
                                && s.fixture_live == Some(false)
                        }),
                )?;
                let checkpoint=sha(&serde_json::to_vec(&json!({"runId":self.run_id,"membership":self.membership,
                    "bindings":self.bindings,"preparations":self.preparations,"services":self.sessions,"counters":self.counters()})).unwrap());
                self.checkpoint = Some(checkpoint.clone());
                self.state = "frozen".into();
                Ok(json!({"checkpoint":checkpoint}))
            }
            Command::Publish | Command::StageResult { .. } => {
                require(
                    owner
                        && self.bindings.get("supervision").is_some() == staged_result.is_some()
                        && self.state == "executing"
                        && self.consumptions == 1
                        && no_pending
                        && self.sessions.len() == 12
                        && self.sessions.values().all(|s| {
                            s.finished
                                && s.closed
                                && s.attached
                                && s.sealed
                                && s.native_live == Some(false)
                                && s.fixture_live == Some(false)
                        }),
                )?;
                self.state = if staged_result.is_some() {
                    "provisional_complete"
                } else {
                    "complete"
                }
                .into();
                self.result_sha256 = staged_result;
                Ok(json!({"complete":self.state == "complete", "state":self.state}))
            }
            Command::Approve { approval } => {
                require(owner && self.state == "frozen" && self.consumptions == 0)?;
                let expiry = approval.verify(self, rehearsal)?;
                self.approval_sha256 = Some(sha(&serde_json::to_vec(&approval).unwrap()));
                self.approval_expires = Some(expiry);
                self.consumptions = 1;
                self.state = "executing".into();
                Ok(json!({"testFixtureOnly":true,"consumed":true,"expiresUnixSeconds":expiry}))
            }
            Command::CredentialLookup { operation } => {
                require(
                    owner
                        && self.state == "executing"
                        && self.approval_expires.is_some()
                        && self.consumptions == 1
                        && self.credential_lookups == 0
                        && self
                            .operations
                            .get(&operation)
                            .is_some_and(|o| o.category == "model" && o.dispatched && !o.completed),
                )?;
                self.credential_lookups += 1;
                Ok(json!({"lookups":self.credential_lookups}))
            }
            Command::ModelSendAdmission { operation } => {
                require(owner && self.state == "executing" && self.credential_lookups == 1)?;
                let operation = self
                    .operations
                    .get_mut(&operation)
                    .ok_or("mock_authority_denied")?;
                require(
                    operation.category == "model"
                        && operation.dispatched
                        && !operation.arrived
                        && !operation.completed,
                )?;
                // Only the trusted broker can request this post-supplier check.
                // Its one-shot reserve/lookup cannot be reused for retry/repair.
                Ok(json!({"admitted":true}))
            }
            Command::Consume {
                fake_checkpoint_sha256,
            } => {
                require(
                    owner
                        && self.state == "frozen"
                        && self.consumptions == 0
                        && self.checkpoint.as_deref() == Some(&fake_checkpoint_sha256),
                )?;
                self.consumptions = 1;
                self.state = "executing".into();
                Ok(json!({"fakeApprovalOnly":true,"consumed":true}))
            }
            Command::Reserve {
                method,
                path,
                body_sha256,
            } => {
                let s = session.ok_or("mock_authority_denied")?;
                require(
                    !s.closed
                        && s.fixture_live == Some(true)
                        && digest_valid(&body_sha256)
                        && path.len() <= 4096
                        && path.starts_with('/')
                        && !path.starts_with("//")
                        && !path.contains(['\r', '\n', '#']),
                )?;
                require(
                    self.state
                        == if s.stage == Stage::Preparation {
                            "preparing"
                        } else {
                            "executing"
                        },
                )?;
                let category = if method == "GET" && path == "/__fixtures/compact" {
                    "inventory"
                } else if method == "GET"
                    && (path.starts_with("/musicbrainz/ws/2/") || path.starts_with("/discogs/"))
                    && !s.finished
                {
                    "provider"
                } else if method == "POST"
                    && path == "/model/chat/completions"
                    && !s.finished
                    && s.stage == Stage::Execution
                    && s.job == "assistance-crowd"
                    && self
                        .preparations
                        .get(&s.job)
                        .and_then(|p| p["requestSha256"].as_str())
                        == Some(&body_sha256)
                {
                    "model"
                } else {
                    return Err("mock_authority_denied".into());
                };
                require(
                    category == "inventory"
                        || (s.attached && s.sealed && s.native_live == Some(true)),
                )?;
                let url =
                    reqwest::Url::parse(&format!("{}{}", s.endpoint.trim_end_matches('/'), path))
                        .map_err(|_| "mock_route_invalid")?;
                let target = format!(
                    "{}{}",
                    url.path(),
                    url.query().map(|q| format!("?{q}")).unwrap_or_default()
                );
                require(target == path && url.fragment().is_none())?;
                let counts = self.counters();
                let cap = match category {
                    "provider" => 512,
                    "inventory" => 32,
                    _ => 1,
                };
                require(counts[category].as_u64().unwrap() < cap && self.operations.len() < 576)?;
                let operation = token();
                let request_sha256 =
                    sha(&serde_json::to_vec(&json!([method, path, body_sha256])).unwrap());
                self.operations.insert(
                    operation.clone(),
                    Operation {
                        session: handle.into(),
                        category: category.into(),
                        request_sha256: request_sha256.clone(),
                        dispatched: false,
                        arrived: false,
                        completed: false,
                        response: None,
                    },
                );
                Ok(
                    json!({"operation":operation,"requestSha256":request_sha256,"url":url.as_str(),"service":s.service}),
                )
            }
            Command::Arrival {
                operation,
                request_sha256,
            } => {
                let o = self
                    .operations
                    .get_mut(&operation)
                    .ok_or("mock_authority_denied")?;
                let s = self.sessions.get(&o.session).unwrap();
                require(
                    handle == s.service
                        && !(native_response && o.category == "model")
                        && s.fixture_live == Some(true)
                        && !s.closed
                        && o.dispatched
                        && !o.arrived
                        && !o.completed
                        && request_sha256 == o.request_sha256,
                )?;
                o.arrived = true;
                Ok(json!({"arrived":true}))
            }
            Command::ModelReceipt {
                operation,
                sha256,
                bytes,
                status,
                http,
            } => {
                if let Some(observation) = &http {
                    require(native_response && owner)?;
                    observation.validate(rehearsal.unwrap(), status)?;
                }
                let o = self
                    .operations
                    .get_mut(&operation)
                    .ok_or("mock_authority_denied")?;
                require(
                    owner
                        && o.category == "model"
                        && o.dispatched
                        && if native_response {
                            !o.arrived && self.credential_lookups == 1
                        } else {
                            o.arrived
                        }
                        && !o.completed
                        && o.response.is_none()
                        && digest_valid(&sha256)
                        && bytes <= 65536
                        && (100..600).contains(&status),
                )?;
                o.response = Some(json!({"sha256":sha256,"bytes":bytes,"status":status,
                    "identityClaimsVerified":false,"pricingVerified":false,"usageClaimsVerified":false}));
                if native_response {
                    let response = o.response.as_mut().unwrap();
                    response["observation"] = json!(if http.is_some() {
                        "native_http_response"
                    } else {
                        "offline_injected_native_response"
                    });
                    response["remoteArrivalVerified"] = json!(false);
                    if let Some(observation) = http {
                        response["http"] = serde_json::to_value(observation).unwrap();
                    }
                }
                Ok(json!({"recorded":true}))
            }
            Command::Complete { operation } => {
                let o = self
                    .operations
                    .get_mut(&operation)
                    .ok_or("mock_authority_denied")?;
                require(
                    owner
                        && !o.completed
                        && (o.arrived
                            || (native_response && o.category == "model" && o.response.is_some())),
                )?;
                o.completed = true;
                Ok(json!({"completed":true}))
            }
            Command::IngressDenied { reason } => {
                let s = self
                    .sessions
                    .values()
                    .find(|s| s.service == handle)
                    .ok_or("mock_authority_denied")?;
                require(
                    s.fixture_live == Some(true)
                        && [
                            "missing_operation",
                            "oversized",
                            "aborted",
                            "arrival_denied",
                            "connect",
                        ]
                        .contains(&reason.as_str())
                        && self.ingress_denials.len() < 32,
                )?;
                self.ingress_denials
                    .push(json!({"session":s.client,"job":s.job,"stage":s.stage,"reason":reason}));
                self.state = "terminal".into();
                Ok(json!({"stopped":true}))
            }
            Command::Stop => {
                require(owner || session.is_some())?;
                self.state = "terminal".into();
                Ok(json!({"stopped":true}))
            }
            Command::Attach {
                executable_sha256,
                preparation_sha256,
            } => {
                let s = session.ok_or("mock_authority_denied")?;
                require(
                    !s.closed
                        && !s.attached
                        && s.native_live == Some(true)
                        && s.fixture_live == Some(true)
                        && self.bindings["executable"].as_str() == Some(&executable_sha256),
                )?;
                require(if s.stage == Stage::Preparation {
                    preparation_sha256.is_none()
                } else {
                    preparation_sha256.as_ref().is_some_and(|p| {
                        self.preparations
                            .get(&s.job)
                            .and_then(|v| v["preparationSha256"].as_str())
                            == Some(p)
                    })
                })?;
                self.sessions.get_mut(handle).unwrap().attached = true;
                Ok(json!({"attached":true,"stage":s.stage,"job":s.job}))
            }
            Command::SealInputs => {
                let s = session.ok_or("mock_authority_denied")?;
                require(
                    s.attached
                        && s.native_live == Some(true)
                        && !s.closed
                        && !s.sealed
                        && s.stage == Stage::Preparation
                        && self.state == "preparing",
                )?;
                self.sessions.get_mut(handle).unwrap().sealed = true;
                Ok(json!({"sealed":true}))
            }
            Command::MutationAdmission => {
                let s = session.ok_or("mock_authority_denied")?;
                require(
                    s.attached
                        && s.native_live == Some(true)
                        && s.fixture_live == Some(true)
                        && !s.closed
                        && !s.finished
                        && self.mutation_admissions < 4096
                        && ((s.stage == Stage::Preparation
                            && !s.sealed
                            && self.state == "preparing")
                            || (s.stage == Stage::Execution
                                && self.state == "executing"
                                && self.consumptions == 1)),
                )?;
                self.mutation_admissions += 1;
                Ok(json!({"admitted":true,"admission":self.mutation_admissions}))
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct Journal {
    path: PathBuf,
    fault: Option<CrashPoint>,
    rehearsal: Option<std::sync::Arc<policy::Rehearsal>>,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CrashPoint {
    BeforeInitializationCommit,
    AfterInitializationCommit,
    BeforeReservationCommit,
    AfterReservationCommit,
    AfterDispatchAdmission,
    BeforeApprovalCommit,
    AfterApprovalCommit,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Client {
    pub(crate) socket: PathBuf,
    pub(crate) handle: String,
}
tokio::task_local! {static CLIENT: Client;}
impl Client {
    pub(crate) fn current() -> Option<Self> {
        CLIENT.try_with(Clone::clone).ok()
    }
    pub(crate) async fn scope<F: std::future::Future>(&self, future: F) -> F::Output {
        CLIENT.scope(self.clone(), future).await
    }
    #[cfg(unix)]
    pub(crate) fn lease(&self, role: &str) -> Result<Lease, String> {
        use std::io::Write;
        let mut stream = std::os::unix::net::UnixStream::connect(&self.socket)
            .map_err(|_| "mock_authority_unavailable")?;
        stream
            .set_read_timeout(Some(Duration::from_secs(8)))
            .map_err(|_| "mock_authority_unavailable")?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| "mock_authority_unavailable")?;
        let bytes =
            serde_json::to_vec(&json!({"handle":self.handle,"command":{"op":"lease","role":role}}))
                .unwrap();
        stream
            .write_all(&bytes)
            .map_err(|_| "mock_admission_unknown")?;
        stream
            .write_all(b"\n")
            .map_err(|_| "mock_admission_unknown")?;
        Lease::reply(&mut stream)?;
        Ok(Lease(stream))
    }
    #[cfg(not(unix))]
    pub(crate) fn command(&self, _command: Command) -> Result<Value, String> {
        Err("mock_authority_unsupported_platform".into())
    }
    #[cfg(unix)]
    pub(crate) fn command(&self, command: Command) -> Result<Value, String> {
        use std::io::{BufRead, Read, Write};
        let mut socket = std::os::unix::net::UnixStream::connect(&self.socket)
            .map_err(|_| "mock_authority_unavailable")?;
        socket
            .set_read_timeout(Some(Duration::from_secs(8)))
            .map_err(|_| "mock_authority_unavailable")?;
        socket
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| "mock_authority_unavailable")?;
        let mut bytes =
            serde_json::to_vec(&json!({"handle":self.handle,"command":command})).unwrap();
        require(bytes.len() < 65536)?;
        bytes.push(b'\n');
        socket
            .write_all(&bytes)
            .map_err(|_| "mock_admission_unknown")?;
        let mut bytes = Vec::new();
        std::io::BufReader::new(socket.take(1024 * 1024 + 1))
            .read_until(b'\n', &mut bytes)
            .map_err(|_| "mock_admission_unknown")?;
        require(bytes.len() <= 1024 * 1024 && bytes.last() == Some(&b'\n'))?;
        let result: Value = serde_json::from_slice(&bytes).map_err(|_| "mock_admission_unknown")?;
        if result["ok"] == true {
            Ok(result["value"].clone())
        } else {
            Err("mock_authority_denied".into())
        }
    }
}

#[cfg(unix)]
pub(crate) struct Lease(std::os::unix::net::UnixStream);
#[cfg(unix)]
impl Lease {
    fn reply(stream: &mut std::os::unix::net::UnixStream) -> Result<(), String> {
        use std::io::{BufRead, Read};
        let mut bytes = Vec::new();
        std::io::BufReader::new(stream.take(65537))
            .read_until(b'\n', &mut bytes)
            .map_err(|_| "mock_admission_unknown")?;
        require(bytes.len() <= 65536 && bytes.last() == Some(&b'\n'))?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "mock_admission_unknown")?;
        require(value["ok"] == true)
    }
    pub(crate) fn release(mut self) -> Result<(), String> {
        use std::io::Write;
        self.0
            .write_all(b"release\n")
            .map_err(|_| "mock_admission_unknown")?;
        Self::reply(&mut self.0)
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Evaluation {
    pub(crate) client: Client,
    pub(crate) job: String,
    pub(crate) stage: Stage,
    pub(crate) gateway: String,
    pub(crate) retained_root: PathBuf,
    pub(crate) preparation: Option<PathBuf>,
    #[serde(default)]
    pub(crate) model_rehearsal: Option<policy::RehearsalPlan>,
    #[serde(skip)]
    preparation_snapshot: Option<Vec<u8>>,
}
tokio::task_local! {static EVALUATION: Evaluation;}
impl Evaluation {
    pub(crate) fn current() -> Option<Self> {
        EVALUATION.try_with(Clone::clone).ok()
    }
    pub(crate) fn read(path: &Path) -> Result<Self, String> {
        let anchor = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".amp/in/artifacts")
            .canonicalize()
            .map_err(|_| "mock_root_unavailable")?;
        require(
            path.canonicalize()
                .map_err(|_| "mock_descriptor_missing")?
                .starts_with(&anchor),
        )?;
        let bytes = std::fs::read(path).map_err(|_| "mock_descriptor_missing")?;
        require(bytes.len() <= 65536)?;
        let mut value: Self =
            serde_json::from_slice(&bytes).map_err(|_| "invalid mock descriptor")?;
        require(
            JOBS.contains(&value.job.as_str())
                && value
                    .retained_root
                    .parent()
                    .and_then(|p| p.canonicalize().ok())
                    .is_some_and(|p| p.starts_with(&anchor)),
        )?;
        require(if value.stage == Stage::Execution {
            value.preparation.is_some()
                && value
                    .retained_root
                    .canonicalize()
                    .is_ok_and(|p| p.starts_with(&anchor))
        } else {
            value.preparation.is_none()
        })?;
        if let Some(preparation) = &value.preparation {
            require(
                preparation
                    .canonicalize()
                    .map_err(|_| "mock_plan_missing")?
                    .starts_with(&anchor),
            )?;
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(preparation)
                .map_err(|_| "mock_plan_missing")?
                .take(2 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "mock_plan_missing")?;
            require(bytes.len() <= 2 * 1024 * 1024)?;
            value.preparation_snapshot = Some(bytes);
        }
        let url = reqwest::Url::parse(&value.gateway).map_err(|_| "invalid mock gateway")?;
        require(
            url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.path() == "/"
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
        )?;
        Ok(value)
    }
    pub(crate) fn preparation_bytes(&self) -> Result<&[u8], String> {
        self.preparation_snapshot
            .as_deref()
            .ok_or_else(|| "mock_plan_missing".into())
    }
    pub(crate) async fn scope<F: std::future::Future>(&self, future: F) -> F::Output {
        EVALUATION
            .scope(self.clone(), self.client.scope(future))
            .await
    }
}

impl Journal {
    fn read_run(connection: &Connection) -> Result<(Run, bool), String> {
        let bytes: String = connection
            .query_row("SELECT state FROM authority WHERE id=1", [], |r| r.get(0))
            .map_err(|_| "mock_registry_unavailable")?;
        let mut run: Run = serde_json::from_str(&bytes).map_err(|_| "mock_registry_invalid")?;
        let separated_bindings = run.bindings.is_null();
        if separated_bindings {
            let bindings: String = connection
                .query_row("SELECT value FROM authority_bindings WHERE id=1", [], |r| {
                    r.get(0)
                })
                .map_err(|_| "mock_registry_invalid")?;
            if run.bindings_sha256.as_deref() != Some(sha(bindings.as_bytes()).as_str()) {
                return Err("mock_registry_invalid".into());
            }
            run.bindings = serde_json::from_str(&bindings).map_err(|_| "mock_registry_invalid")?;
        } else if run.bindings_sha256.is_some() {
            return Err("mock_registry_invalid".into());
        }
        // Historical inline registries have no split reference and remain
        // readable without migration. A mixed representation is corruption.
        Ok((run, separated_bindings))
    }

    pub(crate) fn create_rehearsal(
        path: &Path,
        membership: Value,
        mut bindings: Value,
        rehearsal: policy::Rehearsal,
    ) -> Result<(Self, String), String> {
        let object = bindings
            .as_object_mut()
            .ok_or("mock_rehearsal_bindings_invalid")?;
        require(!object.contains_key("modelRehearsal"))?;
        object.insert("modelRehearsal".into(), rehearsal.binding());
        let (mut journal, owner) = Self::create(path, membership, bindings, None)?;
        journal.rehearsal = Some(std::sync::Arc::new(rehearsal));
        Ok((journal, owner))
    }

    pub(crate) fn create(
        path: &Path,
        membership: Value,
        bindings: Value,
        fault: Option<CrashPoint>,
    ) -> Result<(Self, String), String> {
        #[cfg(unix)]
        let exists = storage::files::metadata(path).is_ok();
        #[cfg(not(unix))]
        let exists = path.exists();
        require(!exists)?;
        let rows = membership["evaluations"]
            .as_array()
            .ok_or("invalid mock membership")?;
        let names = rows
            .iter()
            .filter_map(|e| e["name"].as_str())
            .collect::<BTreeSet<_>>();
        require(
            names == JOBS.into_iter().collect()
                && rows.len() == 6
                && membership["requestedFieldSlots"] == 216,
        )?;
        let slots = rows
            .iter()
            .map(|e| {
                e["physicalPaths"].as_array().map_or(0, Vec::len)
                    * e["fields"].as_array().map_or(0, Vec::len)
            })
            .sum::<usize>();
        require(slots == 216)?;
        for row in rows {
            let paths = row["physicalPaths"]
                .as_array()
                .ok_or("invalid mock membership")?;
            let unique = paths
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>();
            let name = row["name"].as_str().unwrap();
            require(
                unique.len() == paths.len()
                    && paths.len()
                        == if name.ends_with("crowd") {
                            11
                        } else if name.ends_with("billie") {
                            9
                        } else {
                            16
                        }
                    && row["fields"] == json!(["title", "artists", "genre"]),
            )?;
        }
        let owner = token();
        let mut run = Run {
            run_id: token(),
            owner: owner.clone(),
            state: "preparing".into(),
            membership,
            bindings,
            bindings_sha256: None,
            sessions: BTreeMap::new(),
            preparations: BTreeMap::new(),
            operations: BTreeMap::new(),
            checkpoint: None,
            consumptions: 0,
            approval_expires: None,
            approval_sha256: None,
            credential_lookups: 0,
            denied: 0,
            mutation_admissions: 0,
            broker_started: false,
            owner_live: None,
            ingress_denials: Vec::new(),
            result_sha256: None,
        };
        #[cfg(unix)]
        let c = storage::open_sqlite(path, OpenFlags::default())
            .map_err(|_| "mock_registry_unavailable")?;
        #[cfg(not(unix))]
        let c = Connection::open(path).map_err(|_| "mock_registry_unavailable")?;
        c.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON;
            CREATE TABLE authority (id INTEGER PRIMARY KEY CHECK(id=1), state TEXT NOT NULL);
            CREATE TABLE authority_bindings (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);")
            .map_err(|_| "mock_registry_unavailable")?;
        let tx = c
            .unchecked_transaction()
            .map_err(|_| "mock_registry_unavailable")?;
        // Source bindings never change. Keep them out of the mutable row so
        // growing operation history does not rewrite their overflow pages.
        let bindings = serde_json::to_string(&run.bindings).unwrap();
        run.bindings_sha256 = Some(sha(bindings.as_bytes()));
        tx.execute("INSERT INTO authority_bindings VALUES (1,?1)", [bindings])
            .map_err(|_| "mock_registry_unavailable")?;
        run.bindings = Value::Null;
        tx.execute(
            "INSERT INTO authority VALUES (1,?1)",
            [serde_json::to_string(&run).unwrap()],
        )
        .map_err(|_| "mock_registry_exists")?;
        if fault == Some(CrashPoint::BeforeInitializationCommit) {
            std::process::exit(86);
        }
        tx.commit().map_err(|_| "mock_registry_commit_unknown")?;
        if fault == Some(CrashPoint::AfterInitializationCommit) {
            std::process::exit(86);
        }
        Ok((
            Self {
                path: path.into(),
                fault,
                rehearsal: None,
            },
            owner,
        ))
    }
    pub(crate) fn command(&self, handle: &str, command: Command) -> Result<Value, String> {
        let reservation = matches!(&command, Command::Reserve { .. });
        let approval = matches!(&command, Command::Approve { .. });
        let dispatch = matches!(&command, Command::Dispatch { .. });
        #[cfg(unix)]
        let mut c = storage::open_sqlite(&self.path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|_| "mock_registry_unavailable")?;
        #[cfg(not(unix))]
        let mut c = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|_| "mock_registry_unavailable")?;
        c.busy_timeout(Duration::from_millis(100))
            .map_err(|_| "mock_registry_unavailable")?;
        // This setting is connection-local: every rewrite must scrub freed
        // cells/overflow pages, not leave extra untyped response projections.
        c.execute_batch("PRAGMA synchronous=FULL; PRAGMA secure_delete=ON")
            .map_err(|_| "mock_registry_unavailable")?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| "mock_registry_unavailable")?;
        let (mut run, separated_bindings) = Self::read_run(&tx)?;
        let result = run.apply(handle, command, self.rehearsal.as_deref());
        if result.is_err() {
            run.denied += 1;
            run.state = "terminal".into();
        }
        if separated_bindings {
            run.bindings = Value::Null;
        }
        tx.execute(
            "UPDATE authority SET state=?1 WHERE id=1",
            [serde_json::to_string(&run).unwrap()],
        )
        .map_err(|_| "mock_registry_unavailable")?;
        // Faults are explicit bootstrap-only controls in this test-only module.
        // exit bypasses Rust destructors so SQLite must recover the transaction.
        if result.is_ok()
            && ((reservation && self.fault == Some(CrashPoint::BeforeReservationCommit))
                || (approval && self.fault == Some(CrashPoint::BeforeApprovalCommit)))
        {
            std::process::exit(86);
        }
        tx.commit().map_err(|_| "mock_registry_commit_unknown")?;
        if result.is_ok()
            && ((reservation && self.fault == Some(CrashPoint::AfterReservationCommit))
                || (approval && self.fault == Some(CrashPoint::AfterApprovalCommit))
                || (dispatch && self.fault == Some(CrashPoint::AfterDispatchAdmission)))
        {
            std::process::exit(86);
        }
        result
    }
    /// Explicit owner-side issuance, not consumption or live-call authorization.
    /// No IPC operation or launcher invokes this signer.
    #[cfg(all(feature = "server", unix))]
    pub(crate) fn owner_approval(
        &self,
        owner: &str,
        directory: &Path,
        expected_checkpoint: &str,
        nonce: &str,
        expires: u64,
    ) -> Result<policy::Approval, String> {
        let (run, context) = self.frozen_owner_run(owner)?;
        require(run.checkpoint.as_deref() == Some(expected_checkpoint))?;
        let payload = json!({"version":1,"runId":run.run_id,"checkpoint":run.checkpoint,
            "scopeSha256":sha(&serde_json::to_vec(&policy::scope(&run)).unwrap()),
            "nonce":nonce,"expiresUnixSeconds":expires});
        policy::validate_claims(&payload, &run)?;
        let approval = context.sign_owner_payload(directory, payload)?;
        approval.verify(&run, Some(context))?;
        Ok(approval)
    }

    #[cfg(all(feature = "server", unix))]
    fn frozen_owner_run(&self, owner: &str) -> Result<(Run, &policy::Rehearsal), String> {
        let context = self
            .rehearsal
            .as_ref()
            .ok_or("approval_context_unavailable")?;
        let c = storage::open_sqlite(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "mock_registry_unavailable")?;
        c.busy_timeout(Duration::from_millis(100))
            .map_err(|_| "mock_registry_unavailable")?;
        let (run, _) = Self::read_run(&c)?;
        let binding = context.binding();
        require(
            owner == run.owner
                && run.owner_live == Some(true)
                && run.state == "frozen"
                && run.consumptions == 0
                && context.disabled_hosted()
                && binding.get("offlineFixture").is_none()
                && binding.get("offlineHttp").is_none()
                && binding.get("offlineResponse").is_none()
                && run.bindings["modelRehearsal"] == binding,
        )?;
        Ok((run, context))
    }

    /// Publishes an already issued approval; neither signs nor consumes it.
    #[cfg(all(feature = "server", unix))]
    pub(crate) fn publish_owner_approval(
        &self,
        owner: &str,
        directory: &Path,
        expected_checkpoint: &str,
        approval: &policy::Approval,
    ) -> Result<(), String> {
        let (run, context) = self.frozen_owner_run(owner)?;
        require(run.checkpoint.as_deref() == Some(expected_checkpoint))?;
        approval.verify(&run, Some(context))?;
        policy::boundary::publish_owner_approval(directory, approval)
    }

    #[cfg(all(feature = "server", unix))]
    pub(crate) fn owner_approval_status(&self, owner: &str, path: &Path) -> Result<Value, String> {
        let (run, context) = self.frozen_owner_run(owner)?;
        let approval = match policy::boundary::read_owner_approval(path) {
            Ok(approval) => approval,
            Err(code) if code == "approval_file_missing" => {
                return Ok(json!({
                "state":"pending","checkpoint":run.checkpoint,"liveAuthority":false}))
            }
            Err(code) => return Err(code),
        };
        approval.verify(&run, Some(context))?;
        Ok(
            json!({"state":"ready","checkpoint":run.checkpoint,"liveAuthority":false,
            "approvalSha256":sha(&serde_json::to_vec(&approval).map_err(|_| "approval_file_invalid")?)}),
        )
    }

    pub(crate) fn report(&self) -> Result<Value, String> {
        #[cfg(unix)]
        let c = storage::open_sqlite(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "mock_registry_unavailable")?;
        #[cfg(not(unix))]
        let c = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "mock_registry_unavailable")?;
        c.busy_timeout(Duration::from_millis(100))
            .map_err(|_| "mock_registry_unavailable")?;
        let (run, _) = Self::read_run(&c)?;
        Ok(
            json!({"runId":run.run_id,"state":run.state,"membership":run.membership,"bindings":run.bindings,
            "checkpoint":run.checkpoint,"preparations":run.preparations,"sessions":run.sessions,"counters":run.counters(),"operations":run.operations,
            "approvalSha256":run.approval_sha256,"approvalExpires":run.approval_expires,"credentialLookups":run.credential_lookups,"testPolicy":policy::name(&run),"consumptions":run.consumptions,"denied":run.denied,"ingressDenials":run.ingress_denials,"ownerLive":run.owner_live,"mutationAdmissions":run.mutation_admissions,"resultSha256":run.result_sha256,"liveAuthority":false,"recovery":"read_only_incomplete_no_resume"}),
        )
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    fn mock_authority_execution_consumes_the_attached_preparation_buffer() {
        let anchor = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".amp/in/artifacts");
        std::fs::create_dir_all(&anchor).unwrap();
        let root = anchor.join(format!("mock-buffer-{}", token()));
        std::fs::create_dir(&root).unwrap();
        let preparation = root.join("preparation.json");
        std::fs::write(&preparation, b"{\"original\":true}").unwrap();
        let descriptor = root.join("session.json");
        std::fs::write(
            &descriptor,
            serde_json::to_vec(&json!({
                "client":{"socket":"/tmp/not-opened","handle":"fake"},"job":"assistance-crowd",
                "stage":"execution","gateway":"http://127.0.0.1:1234", "retained_root":root,
                "preparation":preparation
            }))
            .unwrap(),
        )
        .unwrap();
        let context = Evaluation::read(&descriptor).unwrap();
        std::fs::write(&preparation, b"{\"replacement\":true}").unwrap();
        assert_eq!(context.preparation_bytes().unwrap(), b"{\"original\":true}");
        std::fs::remove_dir_all(root).unwrap();
    }

    fn digest(s: &str) -> String {
        format!("{:x}", Sha256::digest(s.as_bytes()))
    }

    #[test]
    fn mock_authority_missing_or_corrupt_immutable_bindings_never_admit_commands() {
        for damage in [
            "missing",
            "table",
            "malformed",
            "malformed_hashed",
            "changed",
            "reference",
            "inline",
        ] {
            let (_root, journal, owner) = setup_unleased();
            let connection = Connection::open(&journal.path).unwrap();
            match damage {
                "missing" => {
                    connection
                        .execute("DELETE FROM authority_bindings", [])
                        .unwrap();
                }
                "table" => {
                    connection
                        .execute_batch("DROP TABLE authority_bindings")
                        .unwrap();
                }
                "malformed" | "malformed_hashed" => {
                    connection
                        .execute("UPDATE authority_bindings SET value='{'", [])
                        .unwrap();
                    if damage == "malformed_hashed" {
                        let bytes: String = connection
                            .query_row("SELECT state FROM authority", [], |r| r.get(0))
                            .unwrap();
                        let mut state: Value = serde_json::from_str(&bytes).unwrap();
                        state["bindings_sha256"] = json!(digest("{"));
                        connection
                            .execute("UPDATE authority SET state=?1", [state.to_string()])
                            .unwrap();
                    }
                }
                "changed" => {
                    connection
                        .execute("UPDATE authority_bindings SET value='{}'", [])
                        .unwrap();
                }
                _ => {
                    let bytes: String = connection
                        .query_row("SELECT state FROM authority", [], |r| r.get(0))
                        .unwrap();
                    let mut state: Value = serde_json::from_str(&bytes).unwrap();
                    if damage == "inline" {
                        state["bindings"] = json!({});
                    } else {
                        state.as_object_mut().unwrap().remove("bindings_sha256");
                    }
                    connection
                        .execute("UPDATE authority SET state=?1", [state.to_string()])
                        .unwrap();
                }
            }
            drop(connection);
            let before = std::fs::read(&journal.path).unwrap();
            assert!(journal.report().is_err(), "accepted {damage} bindings");
            assert!(journal
                .command(
                    &owner,
                    Command::Lease {
                        role: "owner".into()
                    }
                )
                .is_err());
            assert_eq!(
                std::fs::read(&journal.path).unwrap(),
                before,
                "invalid evidence was modified"
            );
        }
    }

    #[test]
    fn mock_authority_historical_inline_bindings_remain_readable_without_migration() {
        let (_root, journal, _) = setup_unleased();
        let expected = journal.report().unwrap();
        let connection = Connection::open(&journal.path).unwrap();
        let bytes: String = connection
            .query_row("SELECT state FROM authority", [], |r| r.get(0))
            .unwrap();
        let mut inline: Value = serde_json::from_str(&bytes).unwrap();
        inline["bindings"] = expected["bindings"].clone();
        inline.as_object_mut().unwrap().remove("bindings_sha256");
        connection
            .execute("UPDATE authority SET state=?1", [inline.to_string()])
            .unwrap();
        connection
            .execute_batch("DROP TABLE authority_bindings")
            .unwrap();
        drop(connection);
        let before = std::fs::read(&journal.path).unwrap();
        assert_eq!(journal.report().unwrap(), expected);
        assert_eq!(std::fs::read(&journal.path).unwrap(), before);
    }

    #[test]
    fn mock_authority_initialization_crash_is_atomic_and_never_resumable() {
        for fault in [
            "before_initialization_commit",
            "after_initialization_commit",
        ] {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../.amp/in/artifacts/durable-mock-authority-20261112")
                .join(uuid::Uuid::new_v4().to_string());
            std::fs::create_dir_all(&root).unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "infra::mock_run_authority::tests::mock_authority_initialization_crash_process",
                    "--ignored",
                    "--nocapture",
                ])
                .env_clear()
                .env("SOUNDROBE_MOCK_INIT_ROOT", &root)
                .env("SOUNDROBE_MOCK_INIT_FAULT", fault)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(86));
            let journal = Journal {
                path: root.join("registry.sqlite"),
                fault: None,
                rehearsal: None,
            };
            let before = std::fs::read(&journal.path).unwrap();
            if fault == "before_initialization_commit" {
                assert!(
                    journal.report().is_err(),
                    "partial initialization became authority"
                );
            } else {
                assert_eq!(
                    journal.report().unwrap()["bindings"],
                    json!({"initialization":"split-binding-crash-control"})
                );
            }
            assert!(Journal::create(&journal.path, json!({}), json!({}), None).is_err());
            assert_eq!(
                std::fs::read(&journal.path).unwrap(),
                before,
                "recovery modified the registry"
            );
        }
    }

    #[test]
    #[ignore = "isolated initialization crash control; generated stores only"]
    fn mock_authority_initialization_crash_process() {
        let root = PathBuf::from(std::env::var_os("SOUNDROBE_MOCK_INIT_ROOT").unwrap());
        #[cfg(unix)]
        storage::install_receiver(&root, uuid::Uuid::new_v4()).unwrap();
        let fault =
            serde_json::from_value(json!(std::env::var("SOUNDROBE_MOCK_INIT_FAULT").unwrap()))
                .unwrap();
        let membership = serde_json::from_str(include_str!(
            "../../../test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json"
        ))
        .unwrap();
        let _ = Journal::create(
            &root.join("registry.sqlite"),
            membership,
            json!({"initialization":"split-binding-crash-control"}),
            Some(fault),
        )
        .unwrap();
        panic!("initialization fault resumed");
    }

    pub(super) fn setup() -> (PathBuf, Journal, String) {
        let (root, journal, owner) = setup_unleased();
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
    pub(super) fn setup_unleased() -> (PathBuf, Journal, String) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.amp/in/artifacts/durable-mock-authority-20261112")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let evaluations = ["disabled-crowd", "disabled-billie", "disabled-enya", "assistance-billie", "assistance-enya", "assistance-crowd"]
            .map(|name| json!({"name":name,"physicalPaths":(0..if name.ends_with("crowd") {11} else if name.ends_with("billie") {9} else {16}).map(|n|format!("{n}.flac")).collect::<Vec<_>>(),"fields":["title","artists","genre"]}));
        let (journal, owner) = Journal::create(
            &root.join("registry.sqlite"),
            json!({"evaluations":evaluations,"requestedFieldSlots":216}),
            json!({"executable":digest("test executable")}),
            None,
        )
        .unwrap();
        (root, journal, owner)
    }
    fn register(j: &Journal, owner: &str, job: &str, stage: Stage) -> Value {
        let session = j
            .command(
                owner,
                Command::Register {
                    job: job.into(),
                    stage,
                    endpoint: "http://127.0.0.1:1234".into(),
                    manifest_sha256: digest("fixture"),
                },
            )
            .unwrap();
        for (key, role) in [("client", "native"), ("service", "fixture")] {
            j.command(
                session[key].as_str().unwrap(),
                Command::Lease { role: role.into() },
            )
            .unwrap();
        }
        session
    }
    pub(super) fn freeze(j: &Journal, owner: &str) -> String {
        freeze_request(j, owner, None)
    }
    pub(super) fn freeze_request(j: &Journal, owner: &str, request: Option<String>) -> String {
        for job in [
            "disabled-crowd",
            "disabled-billie",
            "disabled-enya",
            "assistance-billie",
            "assistance-enya",
            "assistance-crowd",
        ] {
            let session = register(j, owner, job, Stage::Preparation);
            j.command(
                session["client"].as_str().unwrap(),
                Command::Attach {
                    executable_sha256: digest("test executable"),
                    preparation_sha256: None,
                },
            )
            .unwrap();
            j.command(session["client"].as_str().unwrap(), Command::SealInputs)
                .unwrap();
            j.command(
                session["client"].as_str().unwrap(),
                Command::Prepared {
                    preparation_sha256: digest(job),
                    request_sha256: if job == "assistance-crowd" {
                        request.clone()
                    } else {
                        None
                    },
                },
            )
            .unwrap();
            j.command(
                session["client"].as_str().unwrap(),
                Command::EndLease {
                    role: "native".into(),
                },
            )
            .unwrap();
            j.command(session["client"].as_str().unwrap(), Command::Close)
                .unwrap();
            j.command(
                session["service"].as_str().unwrap(),
                Command::EndLease {
                    role: "fixture".into(),
                },
            )
            .unwrap();
        }
        j.command(owner, Command::Freeze).unwrap()["checkpoint"]
            .as_str()
            .unwrap()
            .into()
    }
    #[test]
    fn mock_authority_preparation_settles_only_its_own_operations() {
        for (own_pending, closing_pending) in [(false, false), (true, false), (false, true)] {
            let (root, j, owner) = setup();
            let a = register(&j, &owner, "disabled-crowd", Stage::Preparation);
            let b = register(&j, &owner, "disabled-enya", Stage::Preparation);
            for session in [&a, &b] {
                let client = session["client"].as_str().unwrap();
                j.command(
                    client,
                    Command::Attach {
                        executable_sha256: digest("test executable"),
                        preparation_sha256: None,
                    },
                )
                .unwrap();
                j.command(client, Command::SealInputs).unwrap();
            }
            let client = a["client"].as_str().unwrap();
            let pending = if own_pending {
                client
            } else {
                b["client"].as_str().unwrap()
            };
            j.command(
                pending,
                Command::Reserve {
                    method: "GET".into(),
                    path: "/musicbrainz/ws/2/release/control".into(),
                    body_sha256: digest(""),
                },
            )
            .unwrap();
            let prepared = j.command(
                client,
                Command::Prepared {
                    preparation_sha256: digest("prepared"),
                    request_sha256: None,
                },
            );
            if own_pending {
                assert!(
                    prepared.is_err(),
                    "own unfinished wire operation must deny preparation"
                );
            } else {
                prepared.expect("another session's wire operation does not own this preparation");
                if closing_pending {
                    j.command(
                        client,
                        Command::Reserve {
                            method: "GET".into(),
                            path: "/__fixtures/compact".into(),
                            body_sha256: digest(""),
                        },
                    )
                    .unwrap();
                }
                j.command(
                    client,
                    Command::EndLease {
                        role: "native".into(),
                    },
                )
                .unwrap();
                let closed = j.command(client, Command::Close);
                if closing_pending {
                    assert!(closed.is_err(), "own pending inventory prevents Close");
                } else {
                    closed.expect("close waits for this session only");
                }
                assert_eq!(j.report().unwrap()["counters"]["completed"], 0);
                assert!(
                    j.command(&owner, Command::Freeze).is_err(),
                    "global freeze still requires all work"
                );
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn mock_authority_staged_results_are_not_final_authority() {
        let command = serde_json::from_value::<Command>(json!({
            "op": "stage_result", "result_sha256": digest("scored results")
        }));
        assert!(
            command.is_ok(),
            "the supervised result handoff must be typed"
        );
        let (root, original, _) = setup_unleased();
        let (j, owner) = Journal::create(
            &root.join("supervised.sqlite"),
            original.report().unwrap()["membership"].clone(),
            json!({"executable":digest("test executable"), "supervision":{"generation":"test"}}),
            None,
        )
        .unwrap();
        j.command(
            &owner,
            Command::Lease {
                role: "owner".into(),
            },
        )
        .unwrap();
        let checkpoint = freeze(&j, &owner);
        j.command(
            &owner,
            Command::Consume {
                fake_checkpoint_sha256: checkpoint,
            },
        )
        .unwrap();
        for job in JOBS {
            let session = register(&j, &owner, job, Stage::Execution);
            let client = session["client"].as_str().unwrap();
            j.command(
                client,
                Command::Attach {
                    executable_sha256: digest("test executable"),
                    preparation_sha256: Some(digest(job)),
                },
            )
            .unwrap();
            j.command(
                client,
                Command::Finish {
                    observation_sha256: digest("observed"),
                },
            )
            .unwrap();
            j.command(
                client,
                Command::EndLease {
                    role: "native".into(),
                },
            )
            .unwrap();
            j.command(client, Command::Close).unwrap();
            j.command(
                session["service"].as_str().unwrap(),
                Command::EndLease {
                    role: "fixture".into(),
                },
            )
            .unwrap();
        }
        j.command(&owner, command.unwrap()).unwrap();
        let staged = j.report().unwrap();
        assert_eq!(staged["state"], "provisional_complete");
        assert_eq!(staged["resultSha256"], digest("scored results"));
        assert_eq!(staged["ownerLive"], true);
        assert_eq!(staged["membership"]["requestedFieldSlots"], 216);
        j.command(
            &owner,
            Command::EndLease {
                role: "owner".into(),
            },
        )
        .unwrap();
        assert_eq!(j.report().unwrap()["state"], "provisional_complete");
        assert_eq!(j.report().unwrap()["ownerLive"], false);
    }
    #[test]
    fn mock_authority_freezes_all_membership_and_consumes_only_once() {
        let (_root, j, owner) = setup();
        let checkpoint = freeze(&j, &owner);
        j.command(
            &owner,
            Command::Consume {
                fake_checkpoint_sha256: checkpoint.clone(),
            },
        )
        .unwrap();
        assert!(j
            .command(
                &owner,
                Command::Consume {
                    fake_checkpoint_sha256: checkpoint
                }
            )
            .is_err());
        let report = j.report().unwrap();
        assert_eq!(report["membership"]["requestedFieldSlots"], 216);
        assert_eq!(report["state"], "terminal");
        assert_eq!(report["consumptions"], 1);
    }
    #[test]
    fn mock_authority_cannot_finish_an_unattached_execution() {
        let (_root, j, owner) = setup();
        let checkpoint = freeze(&j, &owner);
        j.command(
            &owner,
            Command::Consume {
                fake_checkpoint_sha256: checkpoint,
            },
        )
        .unwrap();
        let session = register(&j, &owner, "disabled-crowd", Stage::Execution);
        assert!(j
            .command(
                session["client"].as_str().unwrap(),
                Command::Finish {
                    observation_sha256: digest("unattached observation")
                }
            )
            .is_err());
        assert_eq!(j.report().unwrap()["state"], "terminal");
        assert_eq!(
            j.report().unwrap()["membership"]["requestedFieldSlots"],
            216
        );
    }
    #[test]
    fn mock_authority_pairs_durable_reservations_and_never_refunds() {
        let (_root, j, owner) = setup();
        let session = register(&j, &owner, "disabled-billie", Stage::Preparation);
        let client = session["client"].as_str().unwrap();
        let service = session["service"].as_str().unwrap();
        j.command(
            client,
            Command::Attach {
                executable_sha256: digest("test executable"),
                preparation_sha256: None,
            },
        )
        .unwrap();
        j.command(client, Command::SealInputs).unwrap();
        let op = j
            .command(
                client,
                Command::Reserve {
                    method: "GET".into(),
                    path: "/musicbrainz/ws/2/release/test?inc=recordings".into(),
                    body_sha256: digest(""),
                },
            )
            .unwrap();
        assert_eq!(j.report().unwrap()["counters"]["provider"], 1);
        j.command(
            &owner,
            Command::Dispatch {
                operation: op["operation"].as_str().unwrap().into(),
            },
        )
        .unwrap();
        j.command(
            service,
            Command::Arrival {
                operation: op["operation"].as_str().unwrap().into(),
                request_sha256: op["requestSha256"].as_str().unwrap().into(),
            },
        )
        .unwrap();
        j.command(
            &owner,
            Command::Complete {
                operation: op["operation"].as_str().unwrap().into(),
            },
        )
        .unwrap();
        j.command(client, Command::Stop).unwrap();
        assert!(j
            .command(
                client,
                Command::Reserve {
                    method: "GET".into(),
                    path: "/__fixtures/compact".into(),
                    body_sha256: digest("")
                }
            )
            .is_err());
        let report = j.report().unwrap();
        assert_eq!(
            report["counters"],
            json!({"provider":1,"inventory":0,"model":0,"reserved":1,"arrived":1,"completed":1})
        );
    }
    #[test]
    fn mock_authority_unknown_operation_prevents_freeze_and_recovery_is_read_only() {
        let (root, j, owner) = setup();
        let session = register(&j, &owner, "disabled-billie", Stage::Preparation);
        j.command(
            session["client"].as_str().unwrap(),
            Command::Reserve {
                method: "GET".into(),
                path: "/__fixtures/compact".into(),
                body_sha256: digest(""),
            },
        )
        .unwrap();
        assert!(j.command(&owner, Command::Freeze).is_err());
        assert!(
            Journal::create(&root.join("registry.sqlite"), json!({}), json!({}), None).is_err()
        );
        let recovered = Journal {
            path: root.join("registry.sqlite"),
            fault: None,
            rehearsal: None,
        };
        assert_eq!(recovered.report().unwrap()["counters"]["inventory"], 1);
        assert_eq!(
            recovered.report().unwrap()["membership"]["requestedFieldSlots"],
            216
        );
    }
    #[test]
    fn mock_authority_seals_preparation_and_stop_denies_mutation_admissions() {
        let (_root, j, owner) = setup();
        let session = register(&j, &owner, "disabled-crowd", Stage::Preparation);
        let client = session["client"].as_str().unwrap();
        j.command(
            client,
            Command::Attach {
                executable_sha256: digest("test executable"),
                preparation_sha256: None,
            },
        )
        .unwrap();
        j.command(client, Command::MutationAdmission).unwrap();
        j.command(client, Command::SealInputs).unwrap();
        assert!(j.command(client, Command::MutationAdmission).is_err());
        assert_eq!(j.report().unwrap()["mutationAdmissions"], 1);
        assert_eq!(j.report().unwrap()["state"], "terminal");
    }
    #[test]
    fn mock_authority_rejects_duplicate_physical_membership_even_with_216_slots() {
        let (_root, j, _owner) = setup();
        let mut membership = j.report().unwrap()["membership"].clone();
        membership["evaluations"][0]["physicalPaths"][1] = json!("0.flac");
        let path = j.path.parent().unwrap().join("bad.sqlite");
        assert!(Journal::create(&path, membership, json!({}), None).is_err());
    }
    #[test]
    fn mock_authority_denies_unattached_or_unsealed_preparation() {
        let (_root, j, owner) = setup();
        let session = register(&j, &owner, "disabled-crowd", Stage::Preparation);
        assert!(j
            .command(
                session["client"].as_str().unwrap(),
                Command::Prepared {
                    preparation_sha256: digest("plan"),
                    request_sha256: None
                }
            )
            .is_err());
        assert_eq!(j.report().unwrap()["state"], "terminal");
    }
    #[test]
    fn mock_authority_denies_normalized_routes_before_consuming_a_category() {
        for path in [
            "/discogs/../__fixtures/compact",
            "/discogs/%2e%2e/__fixtures/compact",
            "/discogs/..\\__fixtures/compact",
            "/musicbrainz/ws/2/../../../__fixtures",
        ] {
            let (_root, j, owner) = setup();
            let session = register(&j, &owner, "disabled-billie", Stage::Preparation);
            j.command(
                session["client"].as_str().unwrap(),
                Command::Attach {
                    executable_sha256: digest("test executable"),
                    preparation_sha256: None,
                },
            )
            .unwrap();
            j.command(session["client"].as_str().unwrap(), Command::SealInputs)
                .unwrap();
            assert!(
                j.command(
                    session["client"].as_str().unwrap(),
                    Command::Reserve {
                        method: "GET".into(),
                        path: path.into(),
                        body_sha256: digest("")
                    }
                )
                .is_err(),
                "accepted {path}"
            );
            assert_eq!(j.report().unwrap()["counters"]["reserved"], 0);
        }
    }
}
