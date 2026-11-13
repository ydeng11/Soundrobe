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
    observed_target: String,
    route: String,
    provider: String,
    input: GenreInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) offline_hosted: Option<boundary::TextPricing>,
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
        binding
    }
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
        let claims: Claims =
            serde_json::from_value(self.payload.clone()).map_err(|_| "mock_approval_invalid")?;
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
}
