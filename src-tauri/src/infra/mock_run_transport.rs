//! Broker-owned mock transport and disabled hosted boundary. No secret fallback.
use super::super::policy::boundary::{
    HttpObservation, HttpProjection, HttpTransport, HttpUsage, HttpValidation, ENDPOINT,
};
use super::{Command, Shared};
use crate::infra::mock_run_authority::{policy::GenreInput, require, sha};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Hosted admission stays closed. Offline bootstrap can select only a
/// native-owned loopback fixture, never a caller-provided destination.
pub(super) struct HostedHttp {
    endpoint: reqwest::Url,
    transport: HttpTransport,
}
impl HostedHttp {
    pub(super) fn pinned() -> Self {
        Self {
            endpoint: ENDPOINT.parse().unwrap(),
            transport: HttpTransport::HostedHttps,
        }
    }
    fn admit(&self, context: &super::super::policy::Rehearsal) -> Result<(), String> {
        match self.transport {
            // Real approval provisioning and run-specific activation are not
            // implemented by an offline public fixture or a settings flag.
            HttpTransport::HostedHttps => Err("live_dispatch_disabled".into()),
            HttpTransport::OfflineLocalHttp => require(context.disabled_hosted()),
        }
    }
    pub(super) fn local_control(endpoint: reqwest::Url) -> Result<Self, String> {
        require(
            endpoint.scheme() == "http"
                && endpoint.host_str() == Some("127.0.0.1")
                && endpoint.port().is_some()
                && endpoint.path() == "/api/v1/chat/completions"
                && endpoint.username().is_empty()
                && endpoint.password().is_none()
                && endpoint.query().is_none()
                && endpoint.fragment().is_none(),
        )?;
        Ok(Self {
            endpoint,
            transport: HttpTransport::OfflineLocalHttp,
        })
    }
    async fn send(
        &self,
        http: &reqwest::Client,
        mut request: reqwest::Request,
    ) -> Result<reqwest::Response, String> {
        require(
            request.url().as_str() == ENDPOINT
                && request.method() == reqwest::Method::POST
                && request
                    .body()
                    .and_then(reqwest::Body::as_bytes)
                    .is_some_and(|bytes| bytes.len() <= 32768)
                && request
                    .headers()
                    .get("content-type")
                    .is_some_and(|v| v == "application/json")
                && request
                    .headers()
                    .get("authorization")
                    .is_some_and(|v| v.is_sensitive())
                && !request.headers().contains_key("x-soundrobe-mock-operation"),
        )?;
        // The native control changes only the destination; no alternate URL is
        // accepted from evaluated input. TLS validation is never disabled.
        *request.url_mut() = self.endpoint.clone();
        http.execute(request)
            .await
            .map_err(|_| "mock_delivery_unknown".into())
    }
}

enum HostedExecutor {
    Injected(OfflineExecutor),
    Http(HostedHttp),
}

type Supplier = Box<dyn Fn() -> Result<String, &'static str> + Send + Sync>;
// In-process response injection only, constructed by explicit native offline
// bootstrap or native controls. IPC and environment cannot supply an executor.
type OfflineExecutor =
    Box<dyn Fn(reqwest::Request) -> Result<reqwest::Response, &'static str> + Send + Sync>;
pub(super) struct Transport {
    credentials: Supplier,
    lookups: AtomicUsize,
    hosted_executor: Option<HostedExecutor>,
    source_settings_sha256: Option<String>,
    // JoinSet aborts its native-owned fixture when the broker transport drops.
    fixture_tasks: tokio::task::JoinSet<()>,
}
impl Default for Transport {
    fn default() -> Self {
        Self::new(Box::new(|| Ok("MOCK_ONLY_SECRET_CANARY".into())))
    }
}
impl Transport {
    pub(super) fn new(credentials: Supplier) -> Self {
        Self {
            credentials,
            lookups: AtomicUsize::new(0),
            hosted_executor: None,
            source_settings_sha256: None,
            fixture_tasks: tokio::task::JoinSet::new(),
        }
    }
    pub(super) fn offline_boundary(
        source: super::super::policy::boundary::CredentialFile,
        executor: OfflineExecutor,
    ) -> Self {
        Self {
            hosted_executor: Some(HostedExecutor::Injected(executor)),
            source_settings_sha256: Some(source.settings_sha()),
            ..Self::new(Box::new(move || source.read()))
        }
    }
    // Native construction only; bootstrap never supplies the pinned HTTPS sender.
    pub(super) fn http_boundary(
        source: super::super::policy::boundary::CredentialFile,
        sender: HostedHttp,
    ) -> Self {
        Self {
            hosted_executor: Some(HostedExecutor::Http(sender)),
            source_settings_sha256: Some(source.settings_sha()),
            ..Self::new(Box::new(move || source.read()))
        }
    }
    /// Explicit native offline bootstrap only; never sends the hosted URL.
    pub(super) fn offline_rehearsal(
        context: &super::super::policy::Rehearsal,
        source: super::super::policy::boundary::CredentialFile,
    ) -> Result<Self, String> {
        require(context.disabled_hosted() && !context.offline_http)?;
        let bytes = fixture_response(context);
        Ok(Self::offline_boundary(
            source,
            Box::new(move |request| {
                if request.url().as_str() != super::super::policy::boundary::ENDPOINT
                    || request
                        .headers()
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        != Some("Bearer MOCK_ONLY_SECRET_CANARY")
                    || request.headers().contains_key("x-soundrobe-mock-operation")
                {
                    return Err("mock_offline_fixture_invalid");
                }
                Ok(reqwest::Response::from(
                    axum::http::Response::builder()
                        .status(200)
                        .body(bytes.clone())
                        .unwrap(),
                ))
            }),
        ))
    }
    pub(super) async fn offline_http_rehearsal(
        context: &super::super::policy::Rehearsal,
        source: super::super::policy::boundary::CredentialFile,
        observation: std::path::PathBuf,
        response_bytes: Option<Vec<u8>>,
    ) -> Result<Self, String> {
        require(
            context.disabled_hosted()
                && context.offline_http
                && context.binding()["offlineFixture"] == true,
        )?;
        let (response, status) = match (&context.offline_response, response_bytes) {
            (None, None) => (fixture_response(context), 200),
            (Some(metadata), Some(bytes)) => {
                require(
                    bytes.len() <= 65536
                        && sha(&bytes) == metadata.sha256
                        && (100..=599).contains(&metadata.status),
                )?;
                (bytes, metadata.status)
            }
            _ => return Err("mock_response_fixture_invalid".into()),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "mock_fixture_unavailable")?;
        let sender = HostedHttp::local_control(
            format!(
                "http://{}/api/v1/chat/completions",
                listener
                    .local_addr()
                    .map_err(|_| "mock_fixture_unavailable")?
            )
            .parse()
            .unwrap(),
        )?;
        let fixture = std::sync::Arc::new(OfflineHttpFixture {
            request: context
                .request("MOCK_ONLY_SECRET_CANARY", ENDPOINT.parse().unwrap())?
                .body()
                .unwrap()
                .as_bytes()
                .unwrap()
                .to_vec(),
            response,
            status,
            observation,
            served: std::sync::atomic::AtomicBool::new(false),
            shutdown: std::sync::Arc::new(tokio::sync::Notify::new()),
        });
        let shutdown = fixture.shutdown.clone();
        let mut transport = Self::http_boundary(source, sender);
        transport.fixture_tasks.spawn(async move {
            let _ = axum::serve(
                listener,
                axum::Router::new()
                    .fallback(offline_fixture_request)
                    .with_state(fixture),
            )
            .with_graceful_shutdown(async move { shutdown.notified().await })
            .await;
        });
        Ok(transport)
    }
    pub(super) fn lookups(&self) -> usize {
        self.lookups.load(Ordering::SeqCst)
    }
    pub(super) async fn genre(
        &self,
        state: &Shared,
        handle: &str,
        input: GenreInput,
    ) -> Result<Value, String> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let _permit = state
                .forwarding
                .acquire()
                .await
                .map_err(|_| "mock_transport_stopped")?;
            let rehearsal = state.journal.rehearsal.as_deref();
            let hosted = rehearsal.is_some_and(|r| r.disabled_hosted());
            // Hosted HTTPS stays closed. Explicit offline bootstrap uses
            // either injection or its own single-request loopback fixture.
            if hosted && self.hosted_executor.is_none() {
                return Err("live_dispatch_disabled".into());
            }
            require(hosted == self.hosted_executor.is_some())?;
            if let Some(context) = rehearsal.filter(|_| hosted) {
                if let Some(HostedExecutor::Http(sender)) = &self.hosted_executor {
                    sender.admit(context)?;
                }
                require(
                    self.source_settings_sha256.as_deref()
                        == context.binding()["boundary"]["settingsSha256"].as_str(),
                )?;
            }
            let body = match rehearsal {
                Some(r) => r.request_body(&input)?,
                None => serde_json::to_vec(&input.body()?).unwrap(),
            };
            let reservation = state.command(
                handle,
                Command::Reserve {
                    method: "POST".into(),
                    path: "/model/chat/completions".into(),
                    body_sha256: sha(&body),
                },
            )?;
            let operation = reservation["operation"]
                .as_str()
                .ok_or("mock_admission_invalid")?
                .to_string();
            let target =
                reqwest::Url::parse(reservation["url"].as_str().ok_or("mock_route_invalid")?)
                    .map_err(|_| "mock_route_invalid")?;
            require(
                target.scheme() == "http"
                    && target.host_str() == Some("127.0.0.1")
                    && target.port().is_some()
                    && target.path() == "/model/chat/completions"
                    && target.query().is_none()
                    && target.fragment().is_none()
                    && target.username().is_empty()
                    && target.password().is_none(),
            )?;
            state.command(
                &state.owner,
                Command::Dispatch {
                    operation: operation.clone(),
                },
            )?;
            // Both durable, nonrefundable admissions precede the injected fake
            // lookup and actual send. No credential leaves this broker scope.
            state.command(
                &state.owner,
                Command::CredentialLookup {
                    operation: operation.clone(),
                },
            )?;
            self.lookups.fetch_add(1, Ordering::SeqCst);
            let key = (self.credentials)().map_err(|_| "mock_credential_unavailable")?;
            let mut request = match rehearsal {
                Some(r) => r.request(
                    &key,
                    if hosted {
                        super::super::policy::boundary::ENDPOINT.parse().unwrap()
                    } else {
                        target
                    },
                )?,
                None => input.request(
                    &key,
                    &format!("http://127.0.0.1:{}/model", target.port().unwrap()),
                )?,
            };
            require(request.body().and_then(reqwest::Body::as_bytes) == Some(body.as_slice()))?;
            if !hosted {
                request.headers_mut().insert(
                    "x-soundrobe-mock-operation",
                    operation.parse().expect("journal operation is a UUID"),
                );
            }
            // Lookup and request preparation may take time. Admit only after
            // both return, with fresh expiry/revocation and native deadline checks.
            state.command(
                &state.owner,
                Command::ModelSendAdmission {
                    operation: operation.clone(),
                },
            )?;
            let mut response = match &self.hosted_executor {
                Some(HostedExecutor::Injected(execute)) => {
                    execute(request).map_err(|_| "mock_delivery_unknown")?
                }
                Some(HostedExecutor::Http(sender)) => sender.send(&state.http, request).await?,
                // Ordinary fixture forwarding remains registered numeric loopback only.
                None => state
                    .http
                    .execute(request)
                    .await
                    .map_err(|_| "mock_delivery_unknown")?,
            };
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "mock_delivery_unknown")?
            {
                require(bytes.len().saturating_add(chunk.len()) <= 65536)?;
                bytes.extend_from_slice(&chunk);
            }
            let (model, provider) = rehearsal
                .map_or(("scripted/genre-v1", "scripted-loopback-v1"), |r| {
                    (r.resolved_model.as_str(), r.provider.as_str())
                });
            let parsed = if response.status().as_u16() == 200 {
                parse(&bytes, model, provider)
            } else {
                Err("mock_authority_denied".into())
            };
            let http = match &self.hosted_executor {
                Some(HostedExecutor::Http(sender)) => Some(HttpObservation {
                    version: 1,
                    transport: sender.transport,
                    request_sha256: sha(&body),
                    validation: match &parsed {
                        Ok(p) if !p.genre.is_empty() => HttpValidation::Accepted,
                        Ok(_) => HttpValidation::Withheld,
                        Err(_) => HttpValidation::Rejected,
                    },
                    projection: parsed.as_ref().ok().cloned(),
                }),
                _ => None,
            };
            // Complete bounded responses settle even when rejected. Only typed
            // accepted projections persist; raw errors/headers never leave here.
            state.command(
                &state.owner,
                Command::ModelReceipt {
                    operation: operation.clone(),
                    sha256: sha(&bytes),
                    bytes: bytes.len() as u32,
                    status: response.status().as_u16(),
                    http,
                },
            )?;
            state.command(&state.owner, Command::Complete { operation })?;
            let parsed = parsed?;
            require(!state.stopped.load(Ordering::SeqCst))?;
            Ok(json!({"genre":parsed.genre,"confidence":parsed.confidence}))
        })
        .await
        .map_err(|_| "mock_delivery_unknown")?
    }
}

fn fixture_response(context: &super::super::policy::Rehearsal) -> Vec<u8> {
    serde_json::to_vec(&json!({"model":context.resolved_model,"provider":context.provider,
        "choices":[{"finish_reason":"stop","message":{"content":
            serde_json::to_string(&json!({"genre":"MOCK_ACCEPTED_GENRE_CANARY","confidence":0.8})).unwrap()}}],
        "usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10},
        "diagnostic":"MOCK_RAW_DIAGNOSTIC_CANARY"})).unwrap()
}

struct OfflineHttpFixture {
    request: Vec<u8>,
    response: Vec<u8>,
    status: u16,
    observation: std::path::PathBuf,
    served: std::sync::atomic::AtomicBool,
    shutdown: std::sync::Arc<tokio::sync::Notify>,
}

async fn offline_fixture_request(
    axum::extract::State(fixture): axum::extract::State<std::sync::Arc<OfflineHttpFixture>>,
    request: axum::extract::Request,
) -> axum::response::Response {
    let result = async {
        require(!fixture.served.swap(true, Ordering::SeqCst)
            && request.method() == reqwest::Method::POST
            && request.uri() == "/api/v1/chat/completions"
            && request.headers().get("authorization").is_some_and(|v| v == "Bearer MOCK_ONLY_SECRET_CANARY")
            && request.headers().get("content-type").is_some_and(|v| v == "application/json")
            && !request.headers().contains_key("x-soundrobe-mock-operation"))?;
        let bytes = axum::body::to_bytes(request.into_body(), 32768).await.map_err(|_| "mock_fixture_invalid")?;
        require(bytes.as_ref() == fixture.request)?;
        let observed = json!({"version":1,"transport":"offline_local_http","requests":1,
            "requestSha256":sha(&bytes),"requestBytes":bytes.len(),
            "responseSha256":sha(&fixture.response),"responseBytes":fixture.response.len(),"status":fixture.status,
            "exactRequest":true,"fakeCredentialPresented":true,"liveAuthority":false});
        super::super::storage::write_new_or_unscoped(&fixture.observation, serde_json::to_vec(&observed).unwrap())
            .map_err(|_| "mock_fixture_storage")?;
        super::super::storage::files::File::open(fixture.observation.parent().unwrap())
            .and_then(|directory| directory.sync_all()).map_err(|_| "mock_fixture_storage")?;
        Ok::<_, String>(())
    }.await;
    fixture.shutdown.notify_one();
    axum::response::Response::builder()
        .status(if result.is_ok() { fixture.status } else { 403 })
        .header("connection", "close")
        .body(axum::body::Body::from(if result.is_ok() {
            fixture.response.clone()
        } else {
            Vec::new()
        }))
        .unwrap()
}

#[tokio::test]
async fn offline_http_bootstrap_fixture_is_exact_single_request_and_not_external() {
    use super::super::policy::{boundary::CredentialFile, RehearsalPlan};
    use std::os::unix::fs::PermissionsExt;
    let plan: RehearsalPlan = serde_json::from_value(json!({
        "model":"~deepseek/deepseek-v4-flash-latest","observedTarget":"deepseek/deepseek-v4-flash-0731",
        "route":"wafer/us","provider":"Wafer","input":{"artist":"Offline Artist","album":"Asymmetric Album","tracks":["One","Different Two"]},
        "offlineHosted":{"promptPicoUsd":1,"completionPicoUsd":2,"catalogSha256":sha(b"catalog"),"termsSha256":sha(b"terms")},
        "offlineHttp":true})).unwrap();
    let root = std::env::temp_dir().join(format!("sr-offline-http-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let credential = root.join(".env.local");
    std::fs::write(
        &credential,
        format!(
            "LLM_API_KEY=MOCK_ONLY_SECRET_CANARY\nLLM_MODEL={}\n",
            plan.model
        ),
    )
    .unwrap();
    std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o600)).unwrap();
    for mode in [
        "success",
        "wrong-body",
        "wrong-key",
        "wrong-method",
        "custom-accepted",
        "custom-withheld",
        "custom-status",
    ] {
        let mut context = plan.context().unwrap();
        let custom = mode.starts_with("custom-");
        let expected_status = if mode == "custom-status" { 429 } else { 200 };
        let expected_genre = if mode == "custom-withheld" {
            ""
        } else if custom {
            "OTHER_PUBLIC_GENRE"
        } else {
            "MOCK_ACCEPTED_GENRE_CANARY"
        };
        let response_bytes = custom.then(|| {
            let bytes = serde_json::to_vec(&json!({"model":context.resolved_model,"provider":context.provider,
                "choices":[{"finish_reason":"stop","message":{"content":serde_json::to_string(&json!({
                    "genre":"OTHER_PUBLIC_GENRE","confidence":if mode == "custom-withheld" { 0.4 } else { 0.9 }})).unwrap()}}],
                "usage":{"prompt_tokens":17,"completion_tokens":6,"total_tokens":23}})).unwrap();
            context.offline_response = Some(super::super::policy::OfflineResponse { sha256:sha(&bytes), status:expected_status });
            bytes
        });
        let observation = root.join(format!("{mode}.json"));
        let source = CredentialFile::new(credential.clone(), plan.model.clone()).unwrap();
        let mut transport = Transport::offline_http_rehearsal(
            &context,
            source,
            observation.clone(),
            response_bytes.clone(),
        )
        .await
        .unwrap();
        let endpoint = match transport.hosted_executor.as_ref().unwrap() {
            HostedExecutor::Http(sender) => sender.endpoint.clone(),
            _ => panic!("offline HTTP selected injection"),
        };
        assert_eq!(endpoint.host_str(), Some("127.0.0.1"));
        assert_ne!(endpoint.as_str(), ENDPOINT);
        let mut request = context
            .request(
                if mode == "wrong-key" {
                    "WRONG_FAKE_KEY"
                } else {
                    "MOCK_ONLY_SECRET_CANARY"
                },
                ENDPOINT.parse().unwrap(),
            )
            .unwrap();
        let request_bytes = request.body().unwrap().as_bytes().unwrap().to_vec();
        if mode == "wrong-body" {
            *request.body_mut() = Some(b"{}".to_vec().into());
        }
        if mode == "wrong-method" {
            *request.method_mut() = reqwest::Method::GET;
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        *request.url_mut() = endpoint.clone();
        let response = client.execute(request).await.unwrap();
        let status = response.status();
        let bytes = response.bytes().await.unwrap();
        transport.fixture_tasks.join_next().await.unwrap().unwrap();
        assert!(
            client
                .post(endpoint)
                .body(request_bytes.clone())
                .send()
                .await
                .is_err(),
            "fixture accepted another connection"
        );
        if mode == "success" || custom {
            assert_eq!(status, expected_status);
            if let Some(expected) = &response_bytes {
                assert_eq!(bytes.as_ref(), expected);
            }
            let observed: Value =
                serde_json::from_slice(&std::fs::read(&observation).unwrap()).unwrap();
            assert_eq!(
                observed,
                json!({"version":1,"transport":"offline_local_http","requests":1,
                "requestSha256":sha(&request_bytes),"requestBytes":request_bytes.len(),
                "responseSha256":sha(&bytes),"responseBytes":bytes.len(),"status":expected_status,
                "exactRequest":true,"fakeCredentialPresented":true,"liveAuthority":false})
            );
            assert_eq!(
                parse(&bytes, &context.resolved_model, &context.provider)
                    .unwrap()
                    .genre,
                expected_genre
            );
        } else {
            assert_eq!(status, 403);
            assert!(
                !observation.exists(),
                "invalid fixture request published success"
            );
            assert!(!String::from_utf8_lossy(&bytes).contains("MOCK_"));
        }
        drop(transport);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn pinned_http_remains_closed_and_rejects_route_and_request_drift() {
    let sender = HostedHttp::pinned();
    let plan: super::super::policy::RehearsalPlan = serde_json::from_value(json!({
        "model":"~deepseek/deepseek-v4-flash-latest","observedTarget":"deepseek/deepseek-v4-flash-0731",
        "route":"wafer/us","provider":"Wafer","input":{"artist":"A","album":"B","tracks":["C"]},
        "offlineHosted":{"promptPicoUsd":1,"completionPicoUsd":2,"catalogSha256":sha(b"catalog"),"termsSha256":sha(b"terms")}
    })).unwrap();
    let context = plan.context().unwrap();
    assert_eq!(sender.endpoint.as_str(), ENDPOINT);
    assert!(
        sender.admit(&context).is_err(),
        "public offline approval must not enable the pinned external sender"
    );
    for url in [
        "https://127.0.0.1:123/api/v1/chat/completions",
        "http://localhost:123/api/v1/chat/completions",
        "http://127.0.0.2:123/api/v1/chat/completions",
        "http://user@127.0.0.1:123/api/v1/chat/completions",
        "http://127.0.0.1:123/other",
        "http://127.0.0.1:123/api/v1/chat/completions?x=1",
    ] {
        assert!(
            HostedHttp::local_control(url.parse().unwrap()).is_err(),
            "{url}"
        );
    }
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    for damage in [
        "url",
        "method",
        "oversize",
        "stream",
        "authorization",
        "header",
    ] {
        let mut request = context.request("FAKE", ENDPOINT.parse().unwrap()).unwrap();
        match damage {
            "url" => {
                *request.url_mut() = "https://other.invalid/api/v1/chat/completions"
                    .parse()
                    .unwrap()
            }
            "method" => *request.method_mut() = reqwest::Method::GET,
            "oversize" => *request.body_mut() = Some(vec![b'X'; 32769].into()),
            "stream" => *request.body_mut() = None,
            "authorization" => {
                request.headers_mut().remove("authorization");
            }
            "header" => {
                request
                    .headers_mut()
                    .insert("x-soundrobe-mock-operation", "forged".parse().unwrap());
            }
            _ => unreachable!(),
        }
        assert!(sender.send(&http, request).await.is_err(), "{damage}");
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    genre: String,
    confidence: f64,
}
fn parse(bytes: &[u8], model: &str, provider: &str) -> Result<HttpProjection, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "mock_response_invalid")?;
    // These are unverified upstream claims, not attested serving identity,
    // billing, tokenizer/version or price. They authorize no external route.
    require(value["model"] == model && value["provider"] == provider)?;
    let choices = value["choices"].as_array().ok_or("mock_response_invalid")?;
    require(choices.len() == 1 && choices[0]["finish_reason"] == "stop")?;
    let message = &choices[0]["message"];
    require(
        message["refusal"].is_null()
            && message["function_call"].is_null()
            && (message["tool_calls"].is_null()
                || message["tool_calls"].as_array().is_some_and(Vec::is_empty))
            && ["reasoning", "reasoning_content"]
                .iter()
                .all(|key| message[key].is_null() || message[key] == ""),
    )?;
    let text = message["content"].as_str().ok_or("mock_response_invalid")?;
    require(text.len() <= 4096)?;
    let answer: Answer = serde_json::from_str(text).map_err(|_| "mock_schema_invalid")?;
    require(
        answer.genre.len() <= 256
            && !answer.genre.chars().any(char::is_control)
            && answer.confidence.is_finite()
            && (0.0..=1.0).contains(&answer.confidence),
    )?;
    let usage = &value["usage"];
    if let Some(details) = usage.get("completion_tokens_details") {
        require(details.is_object())?;
        if let Some(reasoning) = details.get("reasoning_tokens") {
            require(reasoning.as_u64() == Some(0))?;
        }
    }
    let input = usage["prompt_tokens"]
        .as_u64()
        .ok_or("mock_usage_unknown")?;
    let output = usage["completion_tokens"]
        .as_u64()
        .ok_or("mock_usage_unknown")?;
    require(
        input <= 8192
            && output <= 1024
            && usage["total_tokens"].as_u64() == input.checked_add(output),
    )?;
    // A rejected genre never crosses IPC or enters general/persisted evidence.
    Ok(HttpProjection {
        model: model.into(),
        provider: provider.into(),
        genre: if answer.confidence >= 0.6 {
            answer.genre
        } else {
            String::new()
        },
        confidence: answer.confidence,
        usage: HttpUsage {
            prompt_tokens: input,
            completion_tokens: output,
            total_tokens: input + output,
            reasoning_tokens: 0,
        },
    })
}
