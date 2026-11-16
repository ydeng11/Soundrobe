//! Test-only preparation boundary. External execution is not implemented.
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::time::{Duration, Instant};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    version: u32,
    mode: String,
    run_id: String,
    expires_unix_ms: u64,
    provider: String,
    model: String,
    endpoint: String,
    bindings: BTreeMap<String, Binding>,
    jobs: Vec<Job>,
    limits: Limits,
    hard_cost_microunits: Option<u64>,
    hard_total_billed_tokens: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Binding {
    path: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Job {
    id: String,
    input_sha256: String,
    requests: Vec<Request>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    role: String,
    body_sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Limits {
    attempts: u64,
    retries: u64,
    repairs: u64,
    reserved_output_tokens: u64,
    request_bytes: usize,
    response_bytes: usize,
    elapsed_ms: u64,
}

struct State {
    manifest: Manifest,
    manifest_sha256: String,
    root: PathBuf,
    deadline: Instant,
    attempts: Vec<Value>,
    counts: BTreeMap<String, u64>,
    retries: u64,
    reserved: u64,
    pending: Option<usize>,
    stopped: Option<&'static str>,
    denied: u64,
}

#[derive(Clone)]
pub(crate) struct Budget(Arc<Mutex<State>>);

struct Scope {
    budget: Budget,
    job: String,
    input: String,
}
tokio::task_local! { static ACTIVE: Scope; }
tokio::task_local! { static ATTEMPT: std::cell::Cell<Option<usize>>; }

// In-process, mock-only plan/replay. This is not durable admission or live approval.
#[derive(Clone)]
pub(crate) struct PairedMock(Arc<Mutex<PairedState>>);

struct PairedState {
    endpoint: String,
    model_allowed: bool,
    preparing: bool,
    events: Vec<Value>,
    observed: Vec<Value>,
    cursor: usize,
    requests: usize,
    assisted: Option<Value>,
    stopped: bool,
}

tokio::task_local! { static PAIRED: PairedMock; }

impl PairedMock {
    pub(crate) fn preparation(endpoint: &str, model_allowed: bool) -> Result<Self, String> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| "invalid mock endpoint")?;
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.path() != "/model"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("paired scope requires a strict loopback model endpoint".into());
        }
        Ok(Self(Arc::new(Mutex::new(PairedState {
            endpoint: endpoint.into(),
            model_allowed,
            preparing: true,
            events: Vec::new(),
            observed: Vec::new(),
            cursor: 0,
            requests: 0,
            assisted: None,
            stopped: false,
        }))))
    }

    pub(crate) async fn run<F: Future>(&self, future: F) -> F::Output {
        PAIRED.scope(self.clone(), future).await
    }

    pub(crate) fn replay(&self) -> Self {
        let s = self.0.lock().unwrap();
        Self(Arc::new(Mutex::new(PairedState {
            endpoint: s.endpoint.clone(),
            model_allowed: s.model_allowed,
            preparing: false,
            events: s.events.clone(),
            observed: Vec::new(),
            cursor: 0,
            requests: 0,
            assisted: None,
            stopped: s.stopped,
        })))
    }

    pub(crate) fn restore(
        plan: &Value,
        endpoint: &str,
        model_allowed: bool,
    ) -> Result<Self, String> {
        let events = plan["events"].as_array().ok_or("missing frozen events")?;
        let requests = events.iter().filter(|e| e["kind"] == "request").count();
        if plan["mode"] != "mock_only"
            || plan["preparing"] != true
            || plan["stopped"] != false
            || plan["requests"].as_u64() != Some(requests as u64)
            || requests > usize::from(model_allowed)
            || events.len() > 64
            || plan["planSha256"] != sha(&serde_json::to_vec(events).unwrap())
            || events
                .iter()
                .any(|e| serde_json::to_vec(e).unwrap().len() > 262144)
        {
            return Err("invalid frozen mock plan".into());
        }
        let prepared = Self::preparation(endpoint, model_allowed)?;
        prepared.0.lock().unwrap().events = events.clone();
        Ok(prepared.replay())
    }

    pub(crate) fn complete(&self) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        if s.stopped || (!s.preparing && s.cursor != s.events.len()) {
            s.stopped = true;
            return Err("paired decision/request/application drift".into());
        }
        Ok(())
    }

    pub(crate) fn observations(&self) -> Value {
        let s = self.0.lock().unwrap();
        let kind = if s.events.iter().any(|e| e["kind"] == "request") {
            "eligible_request"
        } else if s.events.iter().any(|e| e["kind"] == "recording_patch") {
            "recording_only"
        } else if s.events.iter().any(|e| e["kind"] == "patch") {
            "provider_only"
        } else {
            "hold"
        };
        json!({"mode":"mock_only", "preparing":s.preparing, "variant":kind,
            "events":s.events, "observedEvents":s.observed, "planSha256":sha(&serde_json::to_vec(&s.events).unwrap()),
            "matchedEvents":s.cursor, "requests":s.requests, "stopped":s.stopped,
            "durableOrLiveEnforcement":false})
    }
}

pub(crate) fn paired_active() -> bool {
    PAIRED.try_with(|_| ()).is_ok()
}

pub(crate) fn paired_preparing() -> bool {
    PAIRED
        .try_with(|p| p.0.lock().unwrap().preparing)
        .unwrap_or(false)
}

// True means plan-only: the caller records the exact effect but MUST NOT execute it.
pub(crate) fn paired_event(mut event: Value) -> Result<bool, String> {
    // These branch IDs describe a set, unlike ordered credits and track mappings.
    if event["kind"] == "branch"
        && matches!(
            event["name"].as_str(),
            Some("common_recording_fields" | "country_independent_recordings")
        )
    {
        if let Some(ids) = event["releaseIds"].as_array_mut() {
            ids.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        }
    }
    PAIRED
        .try_with(|p| {
            let mut s = p.0.lock().unwrap();
            if s.stopped
                || serde_json::to_vec(&event).unwrap().len() > 262144
                || (s.preparing && s.events.len() >= 64)
            {
                s.stopped = true;
                return Err("paired mock stopped or oversized plan".into());
            }
            if s.preparing {
                s.events.push(event);
                return Ok(true);
            }
            s.observed.push(event.clone());
            let mut expected = s.events.get(s.cursor).cloned().unwrap_or(Value::Null);
            // Derive one required final plan. Accepted assistance cannot disappear
            // from later effects; existing local/null patch values remain exact.
            if let Some(assisted) = &s.assisted {
                if expected["kind"] == "review" && expected["required"] == false {
                    expected["required"] = json!(true);
                }
                if expected["kind"] == "candidate" {
                    expected["candidate"]["assisted_genre"] = assisted.clone();
                    expected["candidate"]["genre"] = assisted["value"].clone();
                }
                if expected["kind"] == "patch" && expected["fields"].get("genre").is_none() {
                    expected["fields"]["genre"] = assisted["value"].clone();
                }
            }
            if event != expected {
                s.stopped = true;
                return Err("paired decision/request/application drift".into());
            }
            s.cursor += 1;
            Ok(false)
        })
        .unwrap_or(Ok(false))
        .inspect_err(|_| {
            deny("paired mock drift");
        })
}

pub(crate) fn paired_assisted(value: Value) {
    let _ = PAIRED.try_with(|p| p.0.lock().unwrap().assisted = Some(value));
}

pub(crate) fn paired_request(endpoint: &str, body: &Value) -> Result<(), String> {
    if !paired_active() {
        return Ok(());
    }
    let valid = PAIRED.with(|p| {
        let s = p.0.lock().unwrap();
        !s.stopped
            && s.model_allowed
            && s.endpoint == endpoint
            && s.requests == 0
            && body.pointer("/response_format/json_schema/name")
                == Some(&json!("GenreFillResponse"))
            && serde_json::to_vec(body).unwrap().len() <= 32768
    });
    if !valid {
        paired_stop();
        return Err("paired mock request denied".into());
    }
    let preparing = paired_event(json!({"kind":"request", "body":body}))?;
    PAIRED.with(|p| p.0.lock().unwrap().requests += 1);
    if preparing {
        Err("paired plan-only request; no dispatch".into())
    } else {
        Ok(())
    }
}

pub(crate) fn paired_stop() {
    let _ = PAIRED.try_with(|p| p.0.lock().unwrap().stopped = true);
}

pub(crate) async fn attempt_scope<
    F: Future<Output = Result<Value, crate::infra::openrouter::OpenRouterError>>,
>(
    future: F,
) -> F::Output {
    ATTEMPT
        .scope(std::cell::Cell::new(None), async {
            let result = future.await;
            if result.is_err() {
                abort_pending();
            }
            result
        })
        .await
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn check_bindings(root: &Path, bindings: &BTreeMap<String, Binding>) -> Result<(), String> {
    for key in ["source", "input", "labels", "expectations"] {
        if !bindings.contains_key(key) {
            return Err("missing frozen binding".into());
        }
    }
    for binding in bindings.values() {
        let path = Path::new(&binding.path);
        if !path.components().all(|c| matches!(c, Component::Normal(_)))
            || ![
                "test/",
                "scripts/",
                "src-tauri/src/",
                "src-tauri/target/debug/deps/",
                ".planning/",
                ".amp/in/",
            ]
            .iter()
            .any(|p| binding.path.starts_with(p))
        {
            return Err("unsupported binding path".into());
        }
        let full = root
            .join(path)
            .canonicalize()
            .map_err(|_| "missing binding")?;
        if !full.starts_with(root)
            || sha(&fs::read(full).map_err(|_| "unreadable binding")?) != binding.sha256
        {
            return Err("changed binding".into());
        }
    }
    Ok(())
}

impl Budget {
    pub(crate) fn prepare(
        bytes: &[u8],
        approval: &str,
        root: &Path,
        claims: &Path,
        clock: (u64, Instant),
    ) -> Result<Self, String> {
        let (now_ms, sampled_at) = clock;
        let manifest: Manifest =
            serde_json::from_slice(bytes).map_err(|_| "invalid bounded manifest")?;
        let digest = sha(bytes);
        if approval != digest {
            return Err("missing or changed mock approval".into());
        }
        // This is intentionally not a real approval verifier or paid transport.
        if manifest.mode != "mock"
            || manifest.hard_cost_microunits.is_some()
            || manifest.hard_total_billed_tokens.is_some()
        {
            return Err(
                "external activation and hard billed-token/cost contracts unsupported".into(),
            );
        }
        let endpoint = reqwest::Url::parse(&manifest.endpoint).map_err(|_| "invalid endpoint")?;
        if manifest.version != 1
            || manifest.provider != "openai"
            || manifest.model.is_empty()
            || endpoint.scheme() != "http"
            || endpoint.host_str() != Some("127.0.0.1")
            || endpoint.path() != "/model"
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || manifest.expires_unix_ms <= now_ms
            || manifest.run_id.is_empty()
            || !manifest
                .run_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            || manifest.jobs.is_empty()
            || manifest.limits.attempts == 0
            || manifest.limits.repairs != 0
            || manifest.limits.request_bytes == 0
            || manifest.limits.response_bytes == 0
            || manifest.limits.reserved_output_tokens == 0
            || manifest.limits.elapsed_ms == 0
        {
            return Err("unsupported bounded mock manifest".into());
        }
        let mut ids = BTreeSet::new();
        for job in &manifest.jobs {
            if job.id.is_empty()
                || !ids.insert(&job.id)
                || job.input_sha256.is_empty()
                || job.requests.iter().any(|r| {
                    !["GenreFillResponse", "TagCorrectionResponse"].contains(&r.role.as_str())
                        || r.body_sha256.len() != 64
                })
            {
                return Err("invalid job or role binding".into());
            }
        }
        let root = root.canonicalize().map_err(|_| "missing scope root")?;
        check_bindings(&root, &manifest.bindings)?;
        // Shared retained claim directory is mandatory. No resume or refund on crash.
        let mut claim = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(claims.join(format!("{}.mock-claim", manifest.run_id)))
            .map_err(|_| "mock approval already consumed or claim unavailable")?;
        use std::io::Write;
        claim
            .write_all(digest.as_bytes())
            .and_then(|_| claim.sync_all())
            .map_err(|_| "claim persistence failed")?;
        let deadline = sampled_at
            + Duration::from_millis(
                manifest
                    .limits
                    .elapsed_ms
                    .min(manifest.expires_unix_ms - now_ms),
            );
        if Instant::now() >= deadline {
            return Err("mock approval expired during preparation".into());
        }
        Ok(Self(Arc::new(Mutex::new(State {
            manifest,
            manifest_sha256: digest,
            root,
            deadline,
            attempts: Vec::new(),
            counts: BTreeMap::new(),
            retries: 0,
            reserved: 0,
            pending: None,
            stopped: None,
            denied: 0,
        }))))
    }

    pub(crate) fn validate_job(&self, job: &str, input: &str) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        if state.stopped.is_some()
            || state.pending.is_some()
            || Instant::now() >= state.deadline
            || !state
                .manifest
                .jobs
                .iter()
                .any(|j| j.id == job && j.input_sha256 == input)
        {
            state.stopped = Some("scope admission stopped");
            return Err("job or profile input mismatch".into());
        }
        let result = check_bindings(&state.root, &state.manifest.bindings);
        if result.is_err() || Instant::now() >= state.deadline {
            state.stopped = Some("changed binding or expired scope");
            return Err("changed binding or expired scope".into());
        }
        Ok(())
    }

    pub(crate) fn verify_consumed(&self, key: &str, path: &Path) -> Result<(), String> {
        self.read_consumed(key, path).map(|_| ())
    }

    pub(crate) fn verify_source_root(&self, selected: &Path) -> Result<(), String> {
        let (root, contract_path) = {
            let state = self.0.lock().unwrap();
            let binding = state
                .manifest
                .bindings
                .get("scoringContract")
                .ok_or("missing scoring contract")?;
            (state.root.clone(), state.root.join(&binding.path))
        };
        let contract: Value =
            serde_json::from_slice(&self.read_consumed("scoringContract", &contract_path)?)
                .map_err(|_| "invalid scoring contract")?;
        let claimed = contract["spec"]["sourceRoot"]
            .as_str()
            .ok_or("missing source root")?;
        let claimed = Path::new(claimed);
        if !claimed
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err("unsupported source root".into());
        }
        let expected = root
            .join(claimed)
            .canonicalize()
            .map_err(|_| "missing source root")?;
        let selected = selected
            .canonicalize()
            .map_err(|_| "missing selected source root")?;
        let disposable = root.join(".amp/in/eval-dummies");
        if selected != expected || !selected.starts_with(disposable) {
            return Err("source root is not the frozen generated media root".into());
        }
        Ok(())
    }

    pub(crate) fn verify_provider_base(&self, base: &str) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        if format!("{}/model", base.trim_end_matches('/')) != state.manifest.endpoint {
            state.stopped = Some("provider endpoint differs from approval");
            return Err("provider endpoint differs from approval".into());
        }
        Ok(())
    }

    pub(crate) fn read_consumed(&self, key: &str, path: &Path) -> Result<Vec<u8>, String> {
        let mut state = self.0.lock().unwrap();
        let actual = path.canonicalize().map_err(|_| "missing consumed input")?;
        let bytes = fs::read(&actual).map_err(|_| "unreadable consumed input")?;
        if state.manifest.bindings.get(key).is_none_or(|binding| {
            state.root.join(&binding.path).canonicalize().ok().as_ref() != Some(&actual)
                || sha(&bytes) != binding.sha256
        }) {
            state.stopped = Some("consumed input differs from approval");
            return Err("consumed input differs from approval".into());
        }
        Ok(bytes)
    }

    pub(crate) async fn read_inventory(&self) -> Result<Value, String> {
        let (url, deadline, limit) = {
            let state = self.0.lock().unwrap();
            if state.stopped.is_some() || Instant::now() >= state.deadline {
                return Err("inventory scope stopped".into());
            }
            (
                format!(
                    "{}/__fixtures",
                    state.manifest.endpoint.strip_suffix("/model").unwrap()
                ),
                state.deadline,
                state.manifest.limits.response_bytes,
            )
        };
        let result = tokio::time::timeout_at(deadline, async {
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "inventory client unavailable")?;
            let mut response = client
                .get(url)
                .send()
                .await
                .map_err(|_| "inventory request failed")?
                .error_for_status()
                .map_err(|_| "inventory status failed")?;
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "inventory body failed")?
            {
                if chunk.len() > limit.saturating_sub(bytes.len()) {
                    return Err("inventory byte ceiling");
                }
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&bytes).map_err(|_| "invalid inventory")
        })
        .await
        .map_err(|_| "inventory deadline")
        .and_then(|value| value);
        match result {
            Ok(value) => Ok(value),
            Err(reason) => {
                self.0.lock().unwrap().stopped = Some(reason);
                Err(reason.into())
            }
        }
    }

    pub(crate) fn verify_jobs(&self, jobs: &[String]) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        let actual = jobs.iter().collect::<BTreeSet<_>>();
        let approved = state
            .manifest
            .jobs
            .iter()
            .map(|j| &j.id)
            .collect::<BTreeSet<_>>();
        if actual != approved || actual.len() != jobs.len() {
            state.stopped = Some("selected jobs differ from approval");
            return Err("selected jobs differ from approval".into());
        }
        Ok(())
    }

    pub(crate) fn verify_fixture_records(&self, inventory: &Value) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        let expected = state
            .manifest
            .bindings
            .get("fixtures")
            .and_then(|binding| {
                let bytes = fs::read(state.root.join(&binding.path)).ok()?;
                (sha(&bytes) == binding.sha256).then_some(bytes)
            })
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        if expected
            .as_ref()
            .and_then(|v| v.get("records"))
            .filter(|v| v.is_array())
            != inventory.get("records").filter(|v| v.is_array())
            || inventory.get("records").is_none_or(|v| !v.is_array())
        {
            state.stopped = Some("provider inventory differs from approval");
            return Err("provider inventory differs from approval".into());
        }
        check_bindings(&state.root, &state.manifest.bindings)
    }

    pub(crate) async fn run<F: Future>(&self, job: &str, input: &str, future: F) -> F::Output {
        ACTIVE
            .scope(
                Scope {
                    budget: self.clone(),
                    job: job.into(),
                    input: input.into(),
                },
                future,
            )
            .await
    }

    pub(crate) fn observations(&self) -> Value {
        let s = self.0.lock().unwrap();
        json!({"mode":"mock_only", "manifestSha256":s.manifest_sha256, "attempts":s.attempts,
            "retries":s.retries,"deniedRequests":s.denied,"reservedOutputTokens":s.reserved,
            "pendingAttempt":s.pending,"stopReason":s.stopped,"externalActivationAvailable":false,
            "hardBilledTokenAndCostBounds":"unsupported", "groundedAccuracyCredit":0,
            "scope":"cumulative invocation observations, not per-phase request deltas"})
    }
}

pub(crate) fn active() -> bool {
    ACTIVE.try_with(|_| ()).is_ok()
}

// Carry the scope across the writer's blocking-task boundary. Ordinary writes
// have no scope and keep their existing behavior.
pub(crate) fn write_guard() -> impl Fn() -> Result<(), String> + Send + 'static {
    let scope = ACTIVE
        .try_with(|s| (s.budget.clone(), s.job.clone(), s.input.clone()))
        .ok();
    let paired = PAIRED.try_with(Clone::clone).ok();
    #[cfg(unix)]
    let authority = crate::infra::mock_run_authority::Client::current();
    move || {
        if paired.as_ref().is_some_and(|p| {
            let s = p.0.lock().unwrap();
            s.preparing || s.stopped
        }) {
            #[cfg(unix)]
            if let Some(client) = &authority {
                let _ = client.command(crate::infra::mock_run_authority::Command::Stop);
            }
            return Err("paired write not admitted".into());
        }
        #[cfg(unix)]
        if let Some(client) = &authority {
            client.command(crate::infra::mock_run_authority::Command::MutationAdmission)?;
        }
        match &scope {
            Some((budget, job, input)) => budget.validate_job(job, input),
            None => Ok(()),
        }
    }
}

pub(crate) fn health() -> Result<(), String> {
    write_guard()()
}

pub(crate) fn deadline(original: Instant) -> Instant {
    ACTIVE
        .try_with(|s| original.min(s.budget.0.lock().unwrap().deadline))
        .unwrap_or(original)
}

pub(crate) fn response_limit() -> Option<usize> {
    ACTIVE
        .try_with(|s| s.budget.0.lock().unwrap().manifest.limits.response_bytes)
        .ok()
        .or_else(|| paired_active().then_some(65536))
}

pub(crate) fn deny(reason: &'static str) -> String {
    #[cfg(unix)]
    if let Some(client) = crate::infra::mock_run_authority::Client::current() {
        let _ = client.command(crate::infra::mock_run_authority::Command::Stop);
    }
    let _ = ACTIVE.try_with(|s| {
        let mut state = s.budget.0.lock().unwrap();
        state.stopped = Some(reason);
        state.denied += 1;
    });
    reason.into()
}

pub(crate) fn authorize(endpoint: &str, body: &Value, router_metadata: bool) -> Result<(), String> {
    if !active() {
        return Ok(());
    }
    ACTIVE.with(|scope| {
        scope
            .budget
            .validate_job(&scope.job, &scope.input)
            .map_err(|_| deny("job or source binding denied; scope stopped"))?;
        let mut s = scope.budget.0.lock().unwrap();
        let role = body
            .pointer("/response_format/json_schema/name")
            .and_then(Value::as_str)
            .unwrap_or("");
        let bytes = serde_json::to_vec(body).map_err(|_| "invalid body")?;
        let digest = sha(&bytes);
        let tokens = body["max_tokens"].as_u64().ok_or("missing output limit")?;
        let retry_key = format!("{}:{role}:{digest}", scope.job);
        let retry = s.counts.get(&retry_key).copied().unwrap_or(0) > 0;
        let permitted = s
            .manifest
            .jobs
            .iter()
            .find(|j| j.id == scope.job)
            .is_some_and(|j| {
                j.requests
                    .iter()
                    .any(|r| r.role == role && r.body_sha256 == digest)
            });
        if s.stopped.is_some()
            || s.pending.is_some()
            || Instant::now() >= s.deadline
            || endpoint != s.manifest.endpoint
            || body["model"] != s.manifest.model
            || body.get("provider").is_some()
            || router_metadata
            || !permitted
            || bytes.len() > s.manifest.limits.request_bytes
            || s.attempts.len() as u64 >= s.manifest.limits.attempts
            || (retry && s.retries >= s.manifest.limits.retries)
            || tokens == 0
            || tokens
                > s.manifest
                    .limits
                    .reserved_output_tokens
                    .saturating_sub(s.reserved)
        {
            s.stopped = Some("bounded request denied");
            s.denied += 1;
            return Err("bounded request denied".into());
        }
        s.reserved += tokens;
        s.retries += u64::from(retry);
        *s.counts.entry(retry_key).or_default() += 1;
        let index = s.attempts.len();
        ATTEMPT.with(|owner| owner.set(Some(index)));
        s.pending = Some(index);
        s.attempts.push(
            json!({"job":scope.job,"role":role,"requestBodySha256":digest,
            "reservedOutputTokens":tokens,"retry":retry,"outcome":"pending"}),
        );
        Ok(())
    })
}

pub(crate) fn finish(status: u16, body: &[u8], payload: Option<&Value>) -> Result<(), String> {
    if !active() {
        return Ok(());
    }
    ACTIVE.with(|scope| {
        let mut s = scope.budget.0.lock().unwrap();
        let index = ATTEMPT
            .with(|owner| owner.take())
            .ok_or("unreserved response")?;
        if s.pending != Some(index) {
            return Err("response reservation mismatch".into());
        }
        s.pending = None;
        let model = payload.and_then(|p| p["model"].as_str());
        let usage = payload.and_then(|p| {
            Some((
                p.pointer("/usage/prompt_tokens")?.as_u64()?,
                p.pointer("/usage/completion_tokens")?.as_u64()?,
                p.pointer("/usage/total_tokens")?.as_u64()?,
            ))
        });
        let known = model == Some(s.manifest.model.as_str())
            && usage.is_some_and(|(a, b, total)| {
                a.checked_add(b).is_some_and(|sum| total >= sum)
                    && b <= s.attempts[index]["reservedOutputTokens"].as_u64().unwrap()
            });
        s.attempts[index]["responseSha256"] = json!(sha(body));
        s.attempts[index]["status"] = json!(status);
        s.attempts[index]["observedUsage"] = json!(usage);
        s.attempts[index]["outcome"] = json!(if known {
            "observed_mock_response"
        } else {
            "unknown_usage_or_model"
        });
        if !known || s.stopped.is_some() {
            s.stopped = Some("unknown usage or model; no further requests");
            return Err("unknown usage or model; no further requests".into());
        }
        Ok(())
    })
}

pub(crate) fn abort_pending() {
    let _ = ACTIVE.try_with(|scope| {
        let mut s = scope.budget.0.lock().unwrap();
        if let Some(index) = ATTEMPT
            .try_with(|owner| owner.take())
            .ok()
            .flatten()
            .filter(|index| s.pending == Some(*index))
        {
            s.pending = None;
            s.attempts[index]["outcome"] = json!("failed_or_cancelled_unknown_usage");
            s.stopped = Some("failed or cancelled attempt; no refund");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::openrouter::{ChatMessage, OpenRouterClient};
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[tokio::test]
    async fn paired_restore_keeps_conditional_requests_but_restarts_execution_accounting() {
        let endpoint = "http://127.0.0.1:1234/model";
        for allowed in [false, true] {
            let plan = PairedMock::preparation(endpoint, allowed).unwrap();
            let body = json!({"response_format":{"json_schema":{"name":"GenreFillResponse"}}});
            if allowed {
                plan.run(async {
                    assert!(paired_request(endpoint, &body).is_err());
                })
                .await;
            }
            let frozen = plan.observations();
            assert_eq!(frozen["requests"], u32::from(allowed));
            let replay = PairedMock::restore(&frozen, endpoint, allowed).unwrap();
            assert_eq!(replay.observations()["requests"], 0);
            replay
                .run(async {
                    if allowed {
                        paired_request(endpoint, &body).unwrap();
                    }
                })
                .await;
            replay.complete().unwrap();
            let mut forged = frozen.clone();
            forged["requests"] = json!(if allowed { 0 } else { 1 });
            assert!(PairedMock::restore(&forged, endpoint, allowed).is_err());
            if allowed {
                assert!(PairedMock::restore(&frozen, endpoint, false).is_err());
            }
        }
    }

    #[tokio::test]
    async fn paired_mock_plan_is_dry_and_replay_rejects_drift_before_effects() {
        let plan = PairedMock::preparation("http://127.0.0.1:1234/model", true).unwrap();
        let event = json!({"kind":"patch", "path":"one.flac", "fields":{"title":"Song (Live)"}, "source":{"inode":7}});
        plan.run(async {
            assert!(paired_event(event.clone()).unwrap());
            assert!(paired_request(
                "http://127.0.0.1:1234/model",
                &json!({"response_format":{"json_schema":{"name":"GenreFillResponse"}}})
            )
            .is_err());
        })
        .await;
        let replay = plan.replay();
        replay
            .run(async {
                let mut changed = event.clone();
                changed["source"]["inode"] = json!(8);
                assert!(paired_event(changed).is_err());
                assert!(health().is_err());
            })
            .await;
        assert!(replay.complete().is_err());
        let replay = plan.replay();
        replay
            .run(async {
                assert!(!paired_event(event).unwrap());
                assert!(paired_request("https://external.invalid", &json!({})).is_err());
            })
            .await;
        assert!(replay.complete().is_err());
        assert!(PairedMock::preparation("https://external.invalid", true).is_err());
    }

    #[tokio::test]
    async fn paired_genre_delta_cannot_hide_an_unrelated_candidate_value() {
        let plan = PairedMock::preparation("http://127.0.0.1:1234/model", true).unwrap();
        plan.run(async {
            paired_event(json!({"kind":"candidate", "candidate":{"genre":null}})).unwrap();
        })
        .await;
        let replay = plan.replay();
        replay
            .run(async {
                let provenance = json!({"value":"Diagnostic", "model":"scripted/genre-v1"});
                paired_assisted(provenance.clone());
                assert!(paired_event(json!({"kind":"candidate", "candidate":{
                "genre":"Unrelated", "assisted_genre":provenance}}))
                .is_err());
            })
            .await;
        assert!(replay.complete().is_err());
    }

    #[tokio::test]
    async fn paired_accepted_genre_is_required_through_candidate_review_and_patches() {
        let plan = PairedMock::preparation("http://127.0.0.1:1234/model", true).unwrap();
        let base = vec![
            json!({"kind":"candidate", "candidate":{"genre":null}}),
            json!({"kind":"review", "required":false}),
            json!({"kind":"patch", "path":"missing.flac", "fields":{"title":"Song"}}),
            json!({"kind":"patch", "path":"local.flac", "fields":{"genre":"User choice"}}),
            json!({"kind":"patch", "path":"cleared.flac", "fields":{"genre":null}}),
        ];
        plan.run(async {
            for event in &base {
                paired_event(event.clone()).unwrap();
            }
        })
        .await;
        let provenance = json!({"value":"Diagnostic", "model":"scripted/genre-v1"});
        let mut final_events = base.clone();
        final_events[0]["candidate"] = json!({"genre":"Diagnostic", "assisted_genre":provenance});
        final_events[1]["required"] = json!(true);
        final_events[2]["fields"]["genre"] = json!("Diagnostic");
        let valid = plan.replay();
        valid
            .run(async {
                paired_assisted(provenance.clone());
                for event in &final_events {
                    paired_event(event.clone()).unwrap();
                }
            })
            .await;
        valid.complete().unwrap();
        for index in 0..3 {
            let replay = plan.replay();
            replay
                .run(async {
                    paired_assisted(provenance.clone());
                    for event in &final_events[..index] {
                        paired_event(event.clone()).unwrap();
                    }
                    assert!(
                        paired_event(base[index].clone()).is_err(),
                        "accepted assistance cannot disappear at event {index}"
                    );
                })
                .await;
            assert!(replay.complete().is_err());
        }
    }

    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    struct Fixture {
        root: std::path::PathBuf,
        endpoint: String,
        count: Arc<AtomicUsize>,
        stopped: Arc<AtomicBool>,
        server: Option<thread::JoinHandle<()>>,
    }

    impl Fixture {
        fn new(responses: Vec<(u16, Value)>) -> Self {
            let root = std::env::temp_dir().join(format!("bounded-eval-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(root.join("test")).unwrap();
            fs::create_dir(root.join("claims")).unwrap();
            fs::write(
                root.join("test/binding.json"),
                b"synthetic source and labels",
            )
            .unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}/model", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let count = Arc::new(AtomicUsize::new(0));
            let stopped = Arc::new(AtomicBool::new(false));
            let (seen, stop) = (count.clone(), stopped.clone());
            let server = thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    let mut block = [0; 4096];
                    while let Ok(n) = stream.read(&mut block) {
                        if n == 0 {
                            break;
                        }
                        bytes.extend_from_slice(&block[..n]);
                        if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                            let headers =
                                String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                            let len = headers
                                .lines()
                                .find_map(|line| line.strip_prefix("content-length: "))
                                .unwrap_or("0")
                                .parse::<usize>()
                                .unwrap();
                            if bytes.len() >= end + 4 + len {
                                break;
                            }
                        }
                    }
                    if !bytes.windows(4).any(|v| v == b"\r\n\r\n") {
                        continue;
                    }
                    let i = seen.fetch_add(1, Ordering::AcqRel);
                    let (status, body) = &responses[i.min(responses.len() - 1)];
                    let body = body.to_string();
                    let _ = write!(stream, "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                }
            });
            Self {
                root,
                endpoint,
                count,
                stopped,
                server: Some(server),
            }
        }

        fn body(&self) -> Value {
            json!({"model":"mock/model-v1", "temperature":0.2, "max_tokens":1024,
                "messages":[{"role":"user","content":"synthetic input"}],
                "response_format":{"type":"json_schema","json_schema":{"name":"GenreFillResponse","schema":{"type":"object"}}}})
        }

        fn manifest(&self) -> Value {
            let binding =
                json!({"path":"test/binding.json", "sha256":sha(b"synthetic source and labels")});
            json!({"version":1,"mode":"mock","runId":"synthetic-run","expiresUnixMs":2000,
                "provider":"openai","model":"mock/model-v1","endpoint":self.endpoint,
                "bindings":{"source":binding,"input":binding,"labels":binding,"expectations":binding},
                "jobs":[{"id":"case/no-ids/cold","inputSha256":"synthetic-profile-input",
                    "requests":[{"role":"GenreFillResponse","bodySha256":sha(self.body().to_string().as_bytes())}]}],
                "limits":{"attempts":2,"retries":1,"repairs":0,"reservedOutputTokens":2048,
                    "requestBytes":4096,"responseBytes":4096,"elapsedMs":1000},
                "hardCostMicrounits":null,"hardTotalBilledTokens":null})
        }

        fn prepare(&self, manifest: &Value) -> Result<Budget, String> {
            let bytes = serde_json::to_vec(manifest).unwrap();
            Budget::prepare(
                &bytes,
                &sha(&bytes),
                &self.root,
                &self.root.join("claims"),
                (1000, Instant::now()),
            )
        }

        async fn request(&self) -> Result<Value, String> {
            OpenRouterClient::at(
                "injected-fake-not-a-real-credential",
                "mock/model-v1",
                &self.endpoint,
            )
            .with_generation(0.2, 1024)
            .complete_json(
                vec![ChatMessage::user("synthetic input")],
                "GenreFillResponse",
                json!({"type":"object"}),
                &AtomicBool::new(false),
            )
            .await
            .map(|response| response.data)
            .map_err(|e| e.diagnostic_code().to_string())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::Release);
            self.server.take().unwrap().join().unwrap();
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[tokio::test]
    async fn paired_recording_branch_compares_release_membership_not_hashset_order() {
        for name in ["common_recording_fields", "country_independent_recordings"] {
            let plan = PairedMock::preparation("http://127.0.0.1:1234/model", false).unwrap();
            plan.run(async {
                paired_event(json!({"kind":"branch", "name":name, "releaseIds":["a", "b"]}))
                    .unwrap();
            })
            .await;
            let replay = plan.replay();
            replay
                .run(async {
                    assert!(paired_event(
                        json!({"kind":"branch", "name":name, "releaseIds":["b", "a"]})
                    )
                    .is_ok());
                })
                .await;
            replay.complete().unwrap();
            let wrong = plan.replay();
            wrong
                .run(async {
                    assert!(paired_event(
                        json!({"kind":"branch", "name":name, "releaseIds":["a", "c"]})
                    )
                    .is_err());
                })
                .await;
        }
    }

    #[tokio::test]
    async fn paired_transport_bounds_the_outer_response_before_parsing() {
        let content = json!({"genre":"x".repeat(65536), "confidence":0.8}).to_string();
        let f = Fixture::new(vec![(200, response(&content))]);
        let plan = PairedMock::preparation(&f.endpoint, true).unwrap();
        assert!(plan.run(f.request()).await.is_err());
        let replay = plan.replay();
        let error = replay.run(f.request()).await.unwrap_err();
        assert_eq!(error, "network_error");
        assert_eq!(f.count.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn paired_transport_has_zero_preparation_wires_and_no_retry_or_repair() {
        for (status, content) in [
            (200, "{\"genre\":\"Diagnostic\",\"confidence\":0.8}"),
            (503, "{}"),
            (200, "not json"),
        ] {
            let f = Fixture::new(vec![(status, response(content))]);
            let plan = PairedMock::preparation(&f.endpoint, true).unwrap();
            assert!(plan.run(f.request()).await.is_err());
            assert_eq!(f.count.load(Ordering::Acquire), 0);
            let replay = plan.replay();
            let result = replay.run(f.request()).await;
            assert_eq!(result.is_ok(), status == 200 && content.starts_with('{'));
            assert_eq!(
                f.count.load(Ordering::Acquire),
                1,
                "neither HTTP failure nor malformed content may create a second wire"
            );
            if status == 200 && content == "not json" {
                assert!(replay.complete().is_err());
            }
        }
        let f = Fixture::new(vec![(200, response("{}"))]);
        let denied = PairedMock::preparation(&f.endpoint, false).unwrap();
        assert!(denied.run(f.request()).await.is_err());
        assert_eq!(f.count.load(Ordering::Acquire), 0);
    }

    fn response(content: &str) -> Value {
        json!({"model":"mock/model-v1","usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10},
            "choices":[{"message":{"content":content}}]})
    }

    #[test]
    fn bounded_preparation_denies_live_stale_changed_and_reused_approval() {
        let f = Fixture::new(vec![(200, response("{}"))]);
        let original = f.manifest();
        for (key, value) in [
            ("mode", json!("live")),
            ("expiresUnixMs", json!(1000)),
            ("endpoint", json!("https://example.invalid")),
            ("hardCostMicrounits", json!(1)),
        ] {
            let mut changed = original.clone();
            changed[key] = value;
            assert!(f.prepare(&changed).is_err());
        }
        let bytes = serde_json::to_vec(&original).unwrap();
        assert!(Budget::prepare(
            &bytes,
            "wrong",
            &f.root,
            &f.root.join("claims"),
            (1000, Instant::now())
        )
        .is_err());
        let _budget = f.prepare(&original).unwrap();
        assert!(
            f.prepare(&original).is_err(),
            "one-shot mock approval must not reset"
        );
        assert_eq!(f.count.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn bounded_stopped_scope_blocks_queued_writes_and_later_jobs() {
        use crate::commands::mutations::{write_track_queued, Patch, TrackPatch};
        use crate::state::write_queue::WriteQueue;
        let f = Fixture::new(vec![(200, response("{}"))]);
        let budget = f.prepare(&f.manifest()).unwrap();
        let path = f.root.join("disposable.wav");
        let source = include_bytes!("../../../test/fixtures/tauri/media-corpus/minimal.wav");
        fs::write(&path, source).unwrap();
        budget
            .run("case/no-ids/cold", "synthetic-profile-input", async {
                deny("deliberate mock stop");
                assert!(write_track_queued(
                    &WriteQueue::default(),
                    path.clone(),
                    TrackPatch {
                        title: Patch::Value("must not write".into()),
                        ..Default::default()
                    }
                )
                .await
                .is_err());
            })
            .await;
        assert_eq!(fs::read(&path).unwrap(), source);
        assert!(budget
            .validate_job("case/no-ids/cold", "synthetic-profile-input")
            .is_err());
    }

    #[test]
    fn bounded_consumed_inputs_must_be_the_approved_files() {
        let f = Fixture::new(vec![(200, response("{}"))]);
        let budget = f.prepare(&f.manifest()).unwrap();
        let alternative = f.root.join("test/other.json");
        fs::write(&alternative, b"different expectations").unwrap();
        assert!(budget
            .verify_consumed("expectations", &alternative)
            .is_err());
    }

    #[test]
    fn bounded_consumed_bytes_are_an_owned_verified_snapshot() {
        let f = Fixture::new(vec![(200, response("{}"))]);
        let budget = f.prepare(&f.manifest()).unwrap();
        let path = f.root.join("test/binding.json");
        let frozen = budget.read_consumed("expectations", &path).unwrap();
        fs::write(&path, b"substituted after snapshot").unwrap();
        assert_eq!(frozen, b"synthetic source and labels");
        assert!(budget.read_consumed("expectations", &path).is_err());
    }

    #[test]
    fn bounded_source_root_requires_the_frozen_generated_media_root() {
        for (claimed, selected, allowed) in [
            (
                ".amp/in/eval-dummies/a/media",
                ".amp/in/eval-dummies/a/media",
                true,
            ),
            (
                ".amp/in/eval-dummies/a/media",
                ".amp/in/eval-dummies/b/media",
                false,
            ),
            ("test/originals", "test/originals", false),
        ] {
            let f = Fixture::new(vec![(200, response("{}"))]);
            fs::create_dir_all(f.root.join(claimed)).unwrap();
            fs::create_dir_all(f.root.join(selected)).unwrap();
            let contract = json!({"spec":{"sourceRoot":claimed}}).to_string();
            fs::write(f.root.join("test/contract.json"), &contract).unwrap();
            let mut manifest = f.manifest();
            manifest["bindings"]["scoringContract"] =
                json!({"path":"test/contract.json", "sha256":sha(contract.as_bytes())});
            let budget = f.prepare(&manifest).unwrap();
            assert_eq!(
                budget.verify_source_root(&f.root.join(selected)).is_ok(),
                allowed
            );
            assert_eq!(f.count.load(Ordering::Acquire), 0);
        }
    }

    #[tokio::test]
    async fn bounded_inventory_read_enforces_the_byte_ceiling() {
        for enough in [false, true] {
            let payload = json!({"records":[]});
            let f = Fixture::new(vec![(200, payload.clone())]);
            let mut manifest = f.manifest();
            manifest["limits"]["responseBytes"] =
                json!(payload.to_string().len() - usize::from(!enough));
            let budget = f.prepare(&manifest).unwrap();
            assert_eq!(budget.read_inventory().await.is_ok(), enough);
            assert_eq!(f.count.load(Ordering::Acquire), 1);
            if !enough {
                assert!(budget
                    .validate_job("case/no-ids/cold", "synthetic-profile-input")
                    .is_err());
            }
        }
    }

    #[tokio::test]
    async fn bounded_stop_during_write_preparation_prevents_dispatch() {
        use crate::commands::mutations::{write_track_queued_guarded, Patch, TrackPatch};
        use crate::state::write_queue::WriteQueue;
        let f = Fixture::new(vec![(200, response("{}"))]);
        let budget = f.prepare(&f.manifest()).unwrap();
        let path = f.root.join("disposable.wav");
        let source = include_bytes!("../../../test/fixtures/tauri/media-corpus/minimal.wav");
        fs::write(&path, source).unwrap();
        let during_prepare = budget.clone();
        let result = budget
            .run(
                "case/no-ids/cold",
                "synthetic-profile-input",
                write_track_queued_guarded(
                    &WriteQueue::default(),
                    path.clone(),
                    TrackPatch {
                        title: Patch::Value("denied".into()),
                        ..Default::default()
                    },
                    false,
                    None,
                    move |_| {
                        during_prepare.0.lock().unwrap().stopped =
                            Some("stopped during caller validation");
                        Ok(())
                    },
                    |_, _, _| Ok(()),
                ),
            )
            .await;
        assert!(result.is_err());
        assert_eq!(fs::read(path).unwrap(), source);
    }

    #[tokio::test]
    async fn bounded_concurrent_denial_preserves_the_forwarded_response_receipt() {
        let f = Fixture::new(vec![(200, response("{}"))]);
        let budget = f.prepare(&f.manifest()).unwrap();
        budget
            .run("case/no-ids/cold", "synthetic-profile-input", async {
                let (a, b) = tokio::join!(f.request(), f.request());
                assert!(a.is_err() && b.is_err());
            })
            .await;
        assert_eq!(f.count.load(Ordering::Acquire), 1);
        let observations = budget.observations();
        assert_eq!(observations["attempts"][0]["status"], 200);
        assert!(observations["attempts"][0]["responseSha256"].is_string());
    }

    #[test]
    fn bounded_selected_membership_and_provider_inventory_are_exact() {
        let f = Fixture::new(vec![(200, response("{}"))]);
        let bytes =
            br#"{"records":[{"provider":"scripted-local-control","body":{"title":"synthetic"}}]}"#;
        fs::write(f.root.join("test/fixtures.json"), bytes).unwrap();
        let mut manifest = f.manifest();
        manifest["bindings"]["fixtures"] =
            json!({"path":"test/fixtures.json", "sha256":sha(bytes)});
        let budget = f.prepare(&manifest).unwrap();
        assert!(budget
            .verify_provider_base(f.endpoint.strip_suffix("/model").unwrap())
            .is_ok());
        assert!(budget.verify_jobs(&["case/no-ids/cold".into()]).is_ok());
        assert!(budget
            .verify_fixture_records(&serde_json::from_slice(bytes).unwrap())
            .is_ok());
        assert!(budget
            .verify_fixture_records(&json!({"records":[]}))
            .is_err());
        assert!(budget.verify_jobs(&[]).is_err());
        assert!(budget.verify_provider_base("http://127.0.0.1:1").is_err());
    }

    #[test]
    fn bounded_expiry_includes_preparation_time() {
        let f = Fixture::new(vec![(200, response("{}"))]);
        let bytes = serde_json::to_vec(&f.manifest()).unwrap();
        assert!(Budget::prepare(
            &bytes,
            &sha(&bytes),
            &f.root,
            &f.root.join("claims"),
            (1000, Instant::now() - Duration::from_secs(2))
        )
        .is_err());
        assert_eq!(f.count.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn bounded_wire_cap_is_shared_and_preserves_no_secret_observations() {
        let f = Fixture::new(vec![(
            200,
            response("{\"genre\":\"scripted\",\"confidence\":0.8}"),
        )]);
        let mut manifest = f.manifest();
        manifest["limits"]["attempts"] = json!(1);
        let budget = f.prepare(&manifest).unwrap();
        assert!(budget
            .run("case/no-ids/cold", "synthetic-profile-input", f.request())
            .await
            .is_ok());
        assert!(budget
            .run("case/no-ids/cold", "synthetic-profile-input", f.request())
            .await
            .is_err());
        assert_eq!(f.count.load(Ordering::Acquire), 1);
        let observations = budget.observations();
        assert_eq!(observations["attempts"].as_array().unwrap().len(), 1);
        assert!(!observations.to_string().contains("injected-fake"));
        assert_eq!(observations["groundedAccuracyCredit"], 0);
    }

    #[tokio::test]
    async fn bounded_unknown_usage_and_schema_repair_stop_before_another_wire() {
        for payload in [
            json!({"model":"mock/model-v1","choices":[{"message":{"content":"{}"}}]}),
            response("not JSON"),
        ] {
            let f = Fixture::new(vec![(200, payload)]);
            let budget = f.prepare(&f.manifest()).unwrap();
            assert!(budget
                .run("case/no-ids/cold", "synthetic-profile-input", f.request())
                .await
                .is_err());
            assert!(budget
                .run("case/no-ids/cold", "synthetic-profile-input", f.request())
                .await
                .is_err());
            assert_eq!(f.count.load(Ordering::Acquire), 1);
        }
    }

    #[tokio::test]
    async fn bounded_changed_labels_and_cancellation_permanently_stop_the_scope() {
        for cancel in [false, true] {
            let f = Fixture::new(vec![(200, response("{}"))]);
            let budget = f.prepare(&f.manifest()).unwrap();
            if cancel {
                let client = OpenRouterClient::at("fake", "mock/model-v1", &f.endpoint)
                    .with_generation(0.2, 1024);
                assert!(budget
                    .run("case/no-ids/cold", "synthetic-profile-input", async {
                        client
                            .complete_json(
                                vec![ChatMessage::user("synthetic input")],
                                "GenreFillResponse",
                                json!({"type":"object"}),
                                &AtomicBool::new(true),
                            )
                            .await
                    })
                    .await
                    .is_err());
            } else {
                fs::write(f.root.join("test/binding.json"), "changed labels").unwrap();
                assert!(budget
                    .run("case/no-ids/cold", "synthetic-profile-input", f.request())
                    .await
                    .is_err());
                fs::write(
                    f.root.join("test/binding.json"),
                    b"synthetic source and labels",
                )
                .unwrap();
            }
            assert!(budget
                .run("case/no-ids/cold", "synthetic-profile-input", f.request())
                .await
                .is_err());
            assert_eq!(f.count.load(Ordering::Acquire), 0);
        }
    }

    #[tokio::test]
    async fn bounded_byte_token_retry_and_deadline_boundaries_are_hard() {
        for (dimension, enough) in ["requestBytes", "responseBytes", "reservedOutputTokens"]
            .into_iter()
            .flat_map(|d| [(d, false), (d, true)])
        {
            let payload = response("{}");
            let f = Fixture::new(vec![(200, payload.clone())]);
            let limit = match dimension {
                "requestBytes" => f.body().to_string().len(),
                "responseBytes" => payload.to_string().len(),
                _ => 1024,
            };
            let mut manifest = f.manifest();
            manifest["limits"][dimension] = json!(limit - usize::from(!enough));
            let budget = f.prepare(&manifest).unwrap();
            assert_eq!(
                budget
                    .run("case/no-ids/cold", "synthetic-profile-input", f.request())
                    .await
                    .is_ok(),
                enough,
                "{dimension}"
            );
            assert_eq!(
                f.count.load(Ordering::Acquire),
                usize::from(enough || dimension == "responseBytes")
            );
        }
        for allowed_retries in [0, 1] {
            let f = Fixture::new(vec![(503, response("{}")), (200, response("{}"))]);
            let mut manifest = f.manifest();
            manifest["limits"]["retries"] = json!(allowed_retries);
            let budget = f.prepare(&manifest).unwrap();
            assert_eq!(
                budget
                    .run("case/no-ids/cold", "synthetic-profile-input", f.request())
                    .await
                    .is_ok(),
                allowed_retries == 1
            );
            assert_eq!(f.count.load(Ordering::Acquire), allowed_retries + 1);
        }
        let f = Fixture::new(vec![(200, response("{}"))]);
        let budget = f.prepare(&f.manifest()).unwrap();
        budget.0.lock().unwrap().deadline = Instant::now() - Duration::from_millis(1);
        assert!(budget
            .run("case/no-ids/cold", "synthetic-profile-input", f.request())
            .await
            .is_err());
        assert_eq!(f.count.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn bounded_jobs_inputs_models_endpoints_and_effective_policy_are_bound() {
        for mutation in ["job", "input", "model", "endpoint", "request", "policy"] {
            let f = Fixture::new(vec![(200, response("{}"))]);
            let mut manifest = f.manifest();
            match mutation {
                "model" => manifest["model"] = json!("other-model"),
                "endpoint" => manifest["endpoint"] = json!("http://127.0.0.1:1/model"),
                "request" => {
                    manifest["jobs"][0]["requests"][0]["bodySha256"] = json!("0".repeat(64))
                }
                "policy" => {
                    let mut body = f.body();
                    body["provider"] = json!({"sort":{"by":"price"},"preferred_min_throughput":{"p90":50},"allow_fallbacks":true});
                    manifest["jobs"][0]["requests"][0]["bodySha256"] =
                        json!(sha(body.to_string().as_bytes()));
                }
                _ => {}
            }
            let budget = f.prepare(&manifest).unwrap();
            if mutation == "policy" {
                let mut client = OpenRouterClient::at("fake", "mock/model-v1", &f.endpoint)
                    .with_generation(0.2, 1024);
                client.test_policy = Some(crate::infra::openrouter::TagCorrectionPolicy {
                    performance_routing: true,
                    ..Default::default()
                });
                assert!(budget
                    .run("case/no-ids/cold", "synthetic-profile-input", async {
                        client
                            .complete_json(
                                vec![ChatMessage::user("synthetic input")],
                                "GenreFillResponse",
                                json!({"type":"object"}),
                                &AtomicBool::new(false),
                            )
                            .await
                    })
                    .await
                    .is_err());
            } else {
                assert!(budget
                    .run(
                        if mutation == "job" {
                            "other-job"
                        } else {
                            "case/no-ids/cold"
                        },
                        if mutation == "input" {
                            "other-input"
                        } else {
                            "synthetic-profile-input"
                        },
                        f.request()
                    )
                    .await
                    .is_err());
            }
            assert_eq!(f.count.load(Ordering::Acquire), 0, "{mutation}");
        }
    }

    #[tokio::test]
    async fn bounded_cold_warm_share_reservations_and_do_not_count_as_retries() {
        let f = Fixture::new(vec![(200, response("{}"))]);
        let mut manifest = f.manifest();
        let mut warm = manifest["jobs"][0].clone();
        warm["id"] = json!("case/no-ids/warm");
        manifest["jobs"].as_array_mut().unwrap().push(warm);
        manifest["limits"]["retries"] = json!(0);
        let budget = f.prepare(&manifest).unwrap();
        for phase in ["cold", "warm"] {
            assert!(budget
                .run(
                    &format!("case/no-ids/{phase}"),
                    "synthetic-profile-input",
                    f.request()
                )
                .await
                .is_ok());
        }
        assert!(budget
            .run("case/no-ids/warm", "synthetic-profile-input", f.request())
            .await
            .is_err());
        assert_eq!(f.count.load(Ordering::Acquire), 2);
        assert_eq!(budget.observations()["reservedOutputTokens"], 2048);
        assert_eq!(budget.observations()["retries"], 0);
    }
}
