//! Broker-owned offline transport. No external endpoint or secret fallback.
use super::{Command, Shared};
use crate::infra::mock_run_authority::{policy::GenreInput, require, sha};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

type Supplier = Box<dyn Fn() -> Result<String, &'static str> + Send + Sync>;
pub(super) struct Transport {
    credentials: Supplier,
    lookups: AtomicUsize,
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
        }
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
            let body = serde_json::to_vec(&input.body()?).unwrap();
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
            // Lookup may take time: recheck expiry/revocation and the native
            // deadline after it returns, before admitting the actual send.
            state.command(
                &state.owner,
                Command::ModelSendAdmission {
                    operation: operation.clone(),
                },
            )?;
            let mut response = state
                .http
                .post(target)
                .bearer_auth(key)
                .header("x-soundrobe-mock-operation", &operation)
                .header("content-type", "application/json")
                .body(body)
                .send()
                .await
                .map_err(|_| "mock_delivery_unknown")?;
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
            let parsed = parse(&bytes)?;
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
fn parse(bytes: &[u8]) -> Result<Value, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "mock_response_invalid")?;
    // These are unverified upstream claims, not attested serving identity,
    // billing, tokenizer/version or price. They authorize no external route.
    require(value["model"] == "scripted/genre-v1" && value["provider"] == "scripted-loopback-v1")?;
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
