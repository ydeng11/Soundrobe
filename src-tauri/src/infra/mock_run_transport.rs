//! Broker-owned mock transport and disabled hosted boundary. No secret fallback.
use super::{Command, Shared};
use crate::infra::mock_run_authority::{policy::GenreInput, require, sha};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

type Supplier = Box<dyn Fn() -> Result<String, &'static str> + Send + Sync>;
// In-process response injection only, constructed by explicit native offline
// bootstrap or native controls. IPC and environment cannot supply an executor.
type OfflineExecutor =
    Box<dyn Fn(reqwest::Request) -> Result<reqwest::Response, &'static str> + Send + Sync>;
pub(super) struct Transport {
    credentials: Supplier,
    lookups: AtomicUsize,
    offline_executor: Option<OfflineExecutor>,
    source_settings_sha256: Option<String>,
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
            offline_executor: None,
            source_settings_sha256: None,
        }
    }
    pub(super) fn offline_boundary(
        source: super::super::policy::boundary::CredentialFile,
        executor: OfflineExecutor,
    ) -> Self {
        Self {
            offline_executor: Some(executor),
            source_settings_sha256: Some(source.settings_sha()),
            ..Self::new(Box::new(move || source.read()))
        }
    }
    /// Explicit native offline bootstrap only; never sends the hosted URL.
    pub(super) fn offline_rehearsal(
        context: &super::super::policy::Rehearsal,
        source: super::super::policy::boundary::CredentialFile,
    ) -> Result<Self, String> {
        require(context.disabled_hosted())?;
        let bytes = serde_json::to_vec(&json!({"model":context.resolved_model,"provider":context.provider,
            "choices":[{"finish_reason":"stop","message":{"content":
                serde_json::to_string(&json!({"genre":"MOCK_ACCEPTED_GENRE_CANARY","confidence":0.8})).unwrap()}}],
            "usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10},
            "diagnostic":"MOCK_RAW_DIAGNOSTIC_CANARY"})).unwrap();
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
            // No enabled external executor exists. The only hosted seam is a
            // native-constructed in-process offline control, never a URL/flag.
            if hosted && self.offline_executor.is_none() {
                return Err("live_dispatch_disabled".into());
            }
            require(hosted == self.offline_executor.is_some())?;
            if let Some(context) = rehearsal.filter(|_| hosted) {
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
            let mut response = if let Some(execute) = &self.offline_executor {
                execute(request).map_err(|_| "mock_delivery_unknown")?
            } else {
                // This arm is still restricted to the registered numeric
                // loopback target above; hosted requests cannot reach it.
                state
                    .http
                    .execute(request)
                    .await
                    .map_err(|_| "mock_delivery_unknown")?
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
            // Mark only a fully received wire operation settled, even when the
            // remote status/schema is rejected. No retry, repair, refund/resume.
            state.command(
                &state.owner,
                Command::ModelReceipt {
                    operation: operation.clone(),
                    sha256: sha(&bytes),
                    bytes: bytes.len() as u32,
                    status: response.status().as_u16(),
                },
            )?;
            state.command(&state.owner, Command::Complete { operation })?;
            require(response.status().as_u16() == 200)?;
            let (model, provider) = rehearsal
                .map_or(("scripted/genre-v1", "scripted-loopback-v1"), |r| {
                    (r.resolved_model.as_str(), r.provider.as_str())
                });
            let parsed = parse(&bytes, model, provider)?;
            require(!state.stopped.load(Ordering::SeqCst))?;
            Ok(parsed)
        })
        .await
        .map_err(|_| "mock_delivery_unknown")?
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    genre: String,
    confidence: f64,
}
fn parse(bytes: &[u8], model: &str, provider: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "mock_response_invalid")?;
    // These are unverified upstream claims, not attested serving identity,
    // billing, tokenizer/version or price. They authorize no external route.
    require(value["model"] == model && value["provider"] == provider)?;
    let choices = value["choices"].as_array().ok_or("mock_response_invalid")?;
    require(choices.len() == 1 && choices[0]["finish_reason"] == "stop")?;
    let text = choices[0]["message"]["content"]
        .as_str()
        .ok_or("mock_response_invalid")?;
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
    Ok(
        json!({"genre":if answer.confidence>=0.6 {answer.genre} else {String::new()},
        "confidence":answer.confidence}),
    )
}
