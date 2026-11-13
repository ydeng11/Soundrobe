//! Offline test policy only. RFC 8032's PUBLIC TEST key is NOT user approval.
//! Never compiled into the desktop/server application or usable for live calls.
use super::{require, sha};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(super) const POLICY: &str =
    "offline-retained-evidence-v1;loopback-scripted-genre-v1;no-live;no-retry;no-repair";
const TEST_PUBLIC_KEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];
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
pub(super) fn scope(run: &super::Run) -> Value {
    json!({"runId":run.run_id,"checkpoint":run.checkpoint,"bindings":run.bindings,
        "membership":run.membership,"preparations":run.preparations,"policy":POLICY})
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GenreInput {
    artist: Option<String>,
    album: Option<String>,
    tracks: Vec<String>,
}
impl GenreInput {
    pub(crate) fn body(&self) -> Result<Value, String> {
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
        let body = json!({"model":"scripted/genre-v1","temperature":0.2,"max_tokens":1024,
            "messages":messages,
            "response_format":{"type":"json_schema","json_schema":{"name":"GenreFillResponse","schema":schema}}});
        require(serde_json::to_vec(&body).unwrap().len() <= 32768)?;
        Ok(body)
    }
}

pub(super) fn now() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| "mock_clock_unavailable".into())
}
impl Approval {
    pub(super) fn verify(&self, run: &super::Run) -> Result<u64, String> {
        let bytes = serde_json::to_vec(&self.payload).map_err(|_| "mock_approval_invalid")?;
        require(bytes.len() <= 4096 && self.signature.len() == 64)?;
        UnparsedPublicKey::new(&ED25519, TEST_PUBLIC_KEY)
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
