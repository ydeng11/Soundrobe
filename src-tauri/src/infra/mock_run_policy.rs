//! Test-build-only fixture policy and disabled hosted-request preparation.
//! Public fixture keys are NOT user approval; no live executor is enabled.
use super::{require, sha};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[path = "mock_run_live_boundary.rs"]
pub(super) mod boundary;

pub(super) const POLICY: &str =
    "offline-retained-evidence-v1;loopback-scripted-genre-v1;no-live;no-retry;no-repair";
const TEST_PUBLIC_KEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];
pub(super) const REHEARSAL_POLICY: &str =
    "offline-model-rehearsal-v1;separate-verifier;fake-credential;loopback-only;no-live;no-retry;no-repair";

/// Public preparation data only. No verifier, credential or dispatch authority.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RehearsalPlan {
    pub(crate) model: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) native_prepared: bool,
    observed_target: String,
    route: String,
    provider: String,
    input: GenreInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) offline_hosted: Option<boundary::TextPricing>,
    #[serde(
        default,
        skip_serializing_if = "std::ops::Not::not",
        deserialize_with = "offline_http_fixture"
    )]
    pub(crate) offline_http: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "offline_response_fixture"
    )]
    pub(crate) offline_response: Option<OfflineResponse>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OfflineResponse {
    pub(crate) sha256: String,
    pub(crate) status: u16,
}
impl OfflineResponse {
    pub(crate) fn read(
        &self,
        plan_file: &std::path::Path,
        sources: &Value,
        source_root: &std::path::Path,
    ) -> Result<Vec<u8>, String> {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;
        require(super::digest_valid(&self.sha256) && (100..=599).contains(&self.status))?;
        let file = plan_file
            .parent()
            .ok_or("mock_response_fixture_invalid")?
            .join("response-fixture.json");
        require(
            file.canonicalize()
                .map_err(|_| "mock_response_fixture_invalid")?
                == file,
        )?;
        let relative = file
            .strip_prefix(source_root)
            .map_err(|_| "mock_response_fixture_invalid")?
            .to_string_lossy();
        let locks: Vec<_> = sources
            .as_array()
            .ok_or("mock_response_fixture_invalid")?
            .iter()
            .filter(|s| s["path"].as_str() == Some(relative.as_ref()))
            .collect();
        require(locks.len() == 1 && locks[0]["sha256"] == self.sha256)?;
        let source = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(file)
            .map_err(|_| "mock_response_fixture_invalid")?;
        let metadata = source
            .metadata()
            .map_err(|_| "mock_response_fixture_invalid")?;
        require(metadata.is_file() && metadata.len() <= 65536)?;
        let mut bytes = Vec::new();
        source
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| "mock_response_fixture_invalid")?;
        require(bytes.len() <= 65536 && sha(&bytes) == self.sha256)?;
        Ok(bytes)
    }
}
fn offline_response_fixture<'de, D: serde::Deserializer<'de>>(
    input: D,
) -> Result<Option<OfflineResponse>, D::Error> {
    OfflineResponse::deserialize(input).map(Some)
}
#[test]
fn offline_response_plan_requires_exact_public_sibling_bytes_and_http_mode() {
    let raw = b"{\"public\":\"different response\"}\n";
    let mut value = json!({"model":"~deepseek/deepseek-v4-flash-latest","observedTarget":"deepseek/deepseek-v4-flash-0731",
        "route":"wafer/us","provider":"Wafer","input":{"artist":"A","album":"B","tracks":["One"]},
        "offlineHosted":{"promptPicoUsd":1,"completionPicoUsd":2,"catalogSha256":sha(b"catalog"),"termsSha256":sha(b"terms")},
        "offlineHttp":true,"offlineResponse":{"sha256":sha(raw),"status":429}});
    let plan: RehearsalPlan = serde_json::from_value(value.clone()).unwrap();
    let context = plan.context().unwrap();
    assert_eq!(
        context.binding()["offlineResponse"],
        value["offlineResponse"]
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.amp/in/response-fixtures")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let file = root.join("response-fixture.json");
    std::fs::write(&file, raw).unwrap();
    let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    let relative = file
        .strip_prefix(&source_root)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let locks = json!([{"path":relative,"sha256":sha(raw)}]);
    let fixture = plan.offline_response.as_ref().unwrap();
    assert_eq!(
        fixture
            .read(&root.join("public-plan.json"), &locks, &source_root)
            .unwrap(),
        raw
    );
    assert!(fixture
        .read(&root.join("public-plan.json"), &json!([]), &source_root)
        .is_err());
    let mut bad = locks.clone();
    bad[0]["sha256"] = json!(sha(b"other"));
    assert!(fixture
        .read(&root.join("public-plan.json"), &bad, &source_root)
        .is_err());
    std::fs::write(&file, b"changed").unwrap();
    assert!(fixture
        .read(&root.join("public-plan.json"), &locks, &source_root)
        .is_err());
    std::fs::remove_file(&file).unwrap();
    std::os::unix::fs::symlink(root.join("missing"), &file).unwrap();
    assert!(fixture
        .read(&root.join("public-plan.json"), &locks, &source_root)
        .is_err());
    std::fs::remove_file(&file).unwrap();
    for (key, wrong) in [
        ("status", json!(99)),
        ("status", json!(600)),
        ("status", json!(true)),
        ("sha256", json!("bad")),
        ("path", json!(".env.local")),
    ] {
        let mut changed = value.clone();
        changed["offlineResponse"][key] = wrong;
        assert!(serde_json::from_value::<RehearsalPlan>(changed)
            .and_then(|p| p.context().map_err(serde::de::Error::custom))
            .is_err());
    }
    value.as_object_mut().unwrap().remove("offlineHttp");
    assert!(serde_json::from_value::<RehearsalPlan>(value)
        .unwrap()
        .context()
        .is_err());
    std::fs::remove_dir_all(root).unwrap();
}

fn offline_http_fixture<'de, D: serde::Deserializer<'de>>(input: D) -> Result<bool, D::Error> {
    if bool::deserialize(input)? {
        Ok(true)
    } else {
        Err(serde::de::Error::custom(
            "offline HTTP fixture marker must be true",
        ))
    }
}
impl RehearsalPlan {
    pub(crate) fn body(&self, input: &GenreInput) -> Result<Value, String> {
        require(
            serde_json::to_value(input).unwrap() == serde_json::to_value(&self.input).unwrap(),
        )?;
        serde_json::from_slice(&Rehearsal::body(input, &self.model, &self.route)?)
            .map_err(|_| "mock_rehearsal_plan_invalid".into())
    }
    pub(crate) fn context(&self) -> Result<Rehearsal, String> {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        require(!self.native_prepared && (!self.offline_http || self.offline_hosted.is_some()))?;
        if let Some(response) = &self.offline_response {
            require(
                self.offline_http
                    && self.offline_hosted.is_some()
                    && super::digest_valid(&response.sha256)
                    && (100..=599).contains(&response.status),
            )?;
        }
        // Deliberately public fixture seed, independently selected natively.
        // Not read from the plan or approval and never real user trust.
        let seed = if self.offline_hosted.is_some() {
            43
        } else {
            42
        };
        let fixture = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap();
        if let Some(pricing) = &self.offline_hosted {
            let mut context = self.disabled_context(
                fixture.public_key().as_ref().try_into().unwrap(),
                pricing.clone(),
            )?;
            context.offline_fixture = true;
            context.offline_http = self.offline_http;
            context.offline_response = self.offline_response.clone();
            return Ok(context);
        }
        Rehearsal::new(
            &self.input,
            &self.model,
            &self.observed_target,
            &self.route,
            &self.provider,
            fixture.public_key().as_ref().try_into().unwrap(),
        )
    }
}

/// Pinned by native construction, never read from an approval or child IPC.
/// Hosted preparation is distinct from fixture trust and has no live executor.
pub(crate) struct Rehearsal {
    public_key: [u8; 32],
    body: Vec<u8>,
    pub(super) model: String,
    pub(super) resolved_model: String,
    pub(super) provider: String,
    route: String,
    boundary: Option<Value>,
    offline_fixture: bool,
    pub(super) offline_http: bool,
    pub(super) offline_response: Option<OfflineResponse>,
}
impl Rehearsal {
    pub(super) fn disabled_hosted(&self) -> bool {
        self.boundary.is_some()
    }
    pub(crate) fn new(
        input: &GenreInput,
        model: &str,
        resolved_model: &str,
        route: &str,
        provider: &str,
        public_key: [u8; 32],
    ) -> Result<Self, String> {
        require(
            public_key != TEST_PUBLIC_KEY
                && !provider.is_empty()
                && provider.len() <= 100
                && provider
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b" _-/".contains(&b)),
        )?;
        // Validate the separately observed response identifier, without replacing
        // the configured moving alias in the request body.
        Self::body(input, resolved_model, route)?;
        let body = Self::body(input, model, route)?;
        Ok(Self {
            public_key,
            body,
            model: model.into(),
            resolved_model: resolved_model.into(),
            provider: provider.into(),
            route: route.into(),
            boundary: None,
            offline_fixture: false,
            offline_http: false,
            offline_response: None,
        })
    }
    fn body(input: &GenreInput, model: &str, route: &str) -> Result<Vec<u8>, String> {
        input.body()?;
        let (messages, schema) =
            crate::commands::auto_tag::genre_request_parts(&serde_json::to_value(input).unwrap());
        let plan = crate::infra::openrouter::OpenRouterClient::at(
            "",
            model,
            "https://openrouter.ai/api/v1",
        )
        .with_generation(0.2, 1024)
        .evaluation_request_plan(&messages, "GenreFillResponse", &schema, route)
        .map_err(|_| "mock_rehearsal_plan_invalid")?;
        serde_json::to_vec(&plan["request"]["body"])
            .map_err(|_| "mock_rehearsal_plan_invalid".into())
    }
    pub(super) fn binding(&self) -> Value {
        let mut binding = json!({"policy":REHEARSAL_POLICY,"model":self.model,"observedTarget":self.resolved_model,"providerClaim":self.provider,"route":self.route,
            "requestSha256":sha(&self.body),"verifierSha256":sha(&self.public_key),"liveAuthority":false});
        if let Some(boundary) = &self.boundary {
            binding["policy"] = json!(boundary::DISABLED_POLICY);
            binding["boundary"] = boundary.clone();
        }
        if self.offline_fixture {
            binding["offlineFixture"] = json!(true);
        }
        if self.offline_http {
            binding["offlineHttp"] = json!(true);
        }
        if let Some(response) = &self.offline_response {
            binding["offlineResponse"] = serde_json::to_value(response).unwrap();
        }
        binding
    }
    #[cfg(feature = "server")]
    pub(super) fn request_body(&self, input: &GenreInput) -> Result<Vec<u8>, String> {
        let body = Self::body(input, &self.model, &self.route)?;
        require(body == self.body)?;
        Ok(body)
    }
    pub(super) fn request(
        &self,
        key: &str,
        target: reqwest::Url,
    ) -> Result<reqwest::Request, String> {
        if self.disabled_hosted() {
            require(target.as_str() == boundary::ENDPOINT && !key.is_empty() && key.len() <= 4096)?;
        } else {
            require(
                key.starts_with("MOCK_")
                    && key.len() <= 256
                    && target.scheme() == "http"
                    && target.host_str() == Some("127.0.0.1")
                    && target.port().is_some()
                    && target.path() == "/model/chat/completions"
                    && target.username().is_empty()
                    && target.password().is_none()
                    && target.query().is_none()
                    && target.fragment().is_none(),
            )?;
        }
        let mut request = reqwest::Request::new(reqwest::Method::POST, target);
        request.headers_mut().insert(
            "content-type",
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        let mut authorization = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| "mock_credential_unavailable")?;
        authorization.set_sensitive(true);
        request.headers_mut().insert("authorization", authorization);
        *request.body_mut() = Some(self.body.clone().into());
        Ok(request)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Approval {
    pub(crate) payload: Value,
    signature: Vec<u8>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Claims {
    version: u32,
    run_id: String,
    checkpoint: String,
    scope_sha256: String,
    nonce: String,
    expires_unix_seconds: u64,
}
pub(super) fn name(run: &super::Run) -> &'static str {
    if run.bindings["modelRehearsal"]["policy"] == boundary::DISABLED_POLICY {
        boundary::DISABLED_POLICY
    } else if run.bindings.get("modelRehearsal").is_some() {
        REHEARSAL_POLICY
    } else {
        POLICY
    }
}
pub(super) fn scope(run: &super::Run) -> Value {
    json!({"runId":run.run_id,"checkpoint":run.checkpoint,"bindings":run.bindings,
        "membership":run.membership,"preparations":run.preparations,"policy":name(run)})
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GenreInput {
    artist: Option<String>,
    album: Option<String>,
    tracks: Vec<String>,
}
impl GenreInput {
    pub(crate) fn body(&self) -> Result<Value, String> {
        // Preparation hashes the same bytes as dispatch, without credentials or I/O.
        let request = self.request("", "http://127.0.0.1:1/model")?;
        serde_json::from_slice(request.body().unwrap().as_bytes().unwrap())
            .map_err(|_| "mock_input_invalid".into())
    }
    pub(crate) fn request(&self, key: &str, base_url: &str) -> Result<reqwest::Request, String> {
        require(
            self.tracks.len() <= 100
                && self
                    .artist
                    .iter()
                    .chain(self.album.iter())
                    .chain(self.tracks.iter())
                    .all(|s| s.len() <= 2048 && !s.chars().any(char::is_control)),
        )?;
        let (messages, schema) =
            crate::commands::auto_tag::genre_request_parts(&serde_json::to_value(self).unwrap());
        crate::infra::openrouter::OpenRouterClient::at(key, "scripted/genre-v1", base_url)
            .with_generation(0.2, 1024)
            .bounded_json_request(&messages, "GenreFillResponse", &schema)
            .map_err(|_| "mock_input_invalid".into())
    }
}

#[test]
fn prepared_rehearsal_plan_keeps_worker_body_and_native_pin_separate() {
    let value = json!({"model":"~deepseek/deepseek-v4-flash-latest", "observedTarget":"deepseek/deepseek-v4-flash-0731",
        "route":"wafer/us", "provider":"Wafer", "input":{"artist":"Asymmetric Artist","album":"Different Album","tracks":["Track One","Other Two"]}});
    let plan: RehearsalPlan = serde_json::from_value(value.clone()).unwrap();
    let body = plan.body(&plan.input).unwrap();
    assert_eq!(body["model"], value["model"]);
    assert_eq!(body["provider"]["only"], json!(["wafer/us"]));
    assert_eq!(
        body["messages"][1]["content"],
        plan.input.body().unwrap()["messages"][1]["content"]
    );
    let context = plan.context().unwrap();
    assert_eq!(
        context.binding()["requestSha256"],
        sha(&serde_json::to_vec(&body).unwrap())
    );
    assert_ne!(context.binding()["verifierSha256"], sha(&TEST_PUBLIC_KEY));
    let changed = serde_json::from_value(json!({"artist":"Asymmetric Artist","album":"Wrong Album","tracks":["Track One","Other Two"]})).unwrap();
    assert!(plan.body(&changed).is_err());
    let mut hosted = value.clone();
    hosted["offlineHosted"] = json!({"promptPicoUsd":1000,"completionPicoUsd":2000,
        "catalogSha256":sha(b"catalog"),"termsSha256":sha(b"terms")});
    let hosted_plan: RehearsalPlan = serde_json::from_value(hosted.clone()).unwrap();
    let hosted_context = hosted_plan.context().unwrap();
    assert_eq!(
        hosted_context.binding()["policy"],
        boundary::DISABLED_POLICY
    );
    assert_eq!(hosted_context.binding()["offlineFixture"], true);
    assert_ne!(
        hosted_context.binding()["verifierSha256"],
        context.binding()["verifierSha256"]
    );
    assert_eq!(
        hosted_context.binding()["requestSha256"],
        context.binding()["requestSha256"]
    );
    assert_eq!(serde_json::to_value(hosted_plan).unwrap(), hosted);
    let mut http = hosted.clone();
    http["offlineHttp"] = json!(true);
    let http_plan: RehearsalPlan = serde_json::from_value(http.clone()).unwrap();
    let http_context = http_plan.context().unwrap();
    assert_eq!(http_context.binding()["offlineHttp"], true);
    assert_eq!(http_context.binding()["policy"], boundary::DISABLED_POLICY);
    assert_eq!(
        http_context.binding()["verifierSha256"],
        hosted_context.binding()["verifierSha256"]
    );
    assert_eq!(
        http_context.binding()["requestSha256"],
        hosted_context.binding()["requestSha256"]
    );
    assert_eq!(serde_json::to_value(http_plan).unwrap(), http);
    for invalid in [json!(false), json!(1), json!("true"), Value::Null] {
        let mut changed = http.clone();
        changed["offlineHttp"] = invalid;
        assert!(serde_json::from_value::<RehearsalPlan>(changed)
            .and_then(|p| p.context().map_err(serde::de::Error::custom))
            .is_err());
    }
    http.as_object_mut().unwrap().remove("offlineHosted");
    assert!(serde_json::from_value::<RehearsalPlan>(http)
        .unwrap()
        .context()
        .is_err());
    hosted["offlineHosted"]["readyForExecution"] = json!(true);
    assert!(serde_json::from_value::<RehearsalPlan>(hosted).is_err());
    let mut injected = value;
    injected["publicKey"] = json!(vec![42; 32]);
    assert!(serde_json::from_value::<RehearsalPlan>(injected).is_err());
}

pub(super) fn now() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| "mock_clock_unavailable".into())
}
impl Approval {
    pub(super) fn verify(
        &self,
        run: &super::Run,
        rehearsal: Option<&Rehearsal>,
    ) -> Result<u64, String> {
        let bytes = serde_json::to_vec(&self.payload).map_err(|_| "mock_approval_invalid")?;
        require(bytes.len() <= 4096 && self.signature.len() == 64)?;
        let key = rehearsal.map_or(&TEST_PUBLIC_KEY, |r| &r.public_key);
        UnparsedPublicKey::new(&ED25519, key)
            .verify(&bytes, &self.signature)
            .map_err(|_| "mock_approval_signature_invalid")?;
        validate_claims(&self.payload, run)
    }
}

pub(super) fn validate_claims(payload: &Value, run: &super::Run) -> Result<u64, String> {
    let claims: Claims =
        serde_json::from_value(payload.clone()).map_err(|_| "mock_approval_invalid")?;
    let now = now()?;
    require(
        claims.version == 1
            && claims.run_id == run.run_id
            && Some(&claims.checkpoint) == run.checkpoint.as_ref()
            && claims.scope_sha256 == sha(&serde_json::to_vec(&scope(run)).unwrap())
            && uuid::Uuid::parse_str(&claims.nonce).is_ok()
            && claims.expires_unix_seconds > now
            && claims.expires_unix_seconds <= now.saturating_add(1800),
    )?;
    Ok(claims.expires_unix_seconds)
}
