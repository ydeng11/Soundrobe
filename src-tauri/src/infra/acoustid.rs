//! Bounded AcoustID fingerprint lookup support.
//!
//! AcoustID returns MusicBrainz recording and release identities. Callers must
//! validate full release coverage before using those identities as evidence.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Semaphore;

const API_URL: &str = "https://api.acoustid.org/v2/lookup";
const MAX_FP_OUTPUT: usize = 1024 * 1024;
const MAX_FP_STDERR: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_RESULTS: usize = 100;
const MAX_RECORDINGS: usize = 200;
const MAX_RELEASES_PER_RECORDING: usize = 100;
const FP_TIMEOUT: Duration = Duration::from_secs(45);
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const REQUEST_INTERVAL: Duration = Duration::from_millis(350);

static FP_SEMAPHORE: OnceLock<Arc<Semaphore>> = OnceLock::new();
static NEXT_ACOUSTID_REQUEST: OnceLock<tokio::sync::Mutex<Option<Instant>>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcoustidFailureKind {
    Unavailable,
    Incomplete,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcoustidError {
    MissingExecutable,
    ProcessFailed,
    ProcessTimedOut,
    OutputTooLarge,
    InvalidFingerprint,
    Cancelled,
    RateLimitCancelled,
    RequestFailed,
    HttpStatus,
    ResponseTooLarge,
    InvalidResponse,
    ApiRejected,
    IncompleteResponse,
}

impl AcoustidError {
    pub fn kind(self) -> AcoustidFailureKind {
        match self {
            Self::Cancelled | Self::RateLimitCancelled => AcoustidFailureKind::Cancelled,
            Self::ProcessTimedOut
            | Self::OutputTooLarge
            | Self::ResponseTooLarge
            | Self::InvalidResponse
            | Self::IncompleteResponse => AcoustidFailureKind::Incomplete,
            _ => AcoustidFailureKind::Unavailable,
        }
    }
}

impl std::fmt::Display for AcoustidError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::MissingExecutable => "fpcalc is not installed or could not be started",
            Self::ProcessFailed => "fpcalc failed to fingerprint an audio file",
            Self::ProcessTimedOut => "fpcalc timed out while fingerprinting an audio file",
            Self::OutputTooLarge => "fpcalc output exceeded the safety limit",
            Self::InvalidFingerprint => "fpcalc returned invalid fingerprint data",
            Self::Cancelled | Self::RateLimitCancelled => "AcoustID lookup was cancelled",
            Self::RequestFailed => "AcoustID request could not be completed",
            Self::HttpStatus => "AcoustID returned an unsuccessful HTTP response",
            Self::ResponseTooLarge => "AcoustID response exceeded the safety limit",
            Self::InvalidResponse => "AcoustID returned an invalid response",
            Self::ApiRejected => "AcoustID rejected the lookup request",
            Self::IncompleteResponse => "AcoustID returned incomplete lookup data",
        })
    }
}

impl std::error::Error for AcoustidError {}

#[derive(Clone, Debug, PartialEq)]
pub struct Fingerprint {
    pub duration_seconds: u32,
    pub value: String,
}

/// Parses only the bounded JSON output contract emitted by `fpcalc -json`.
pub fn parse_fpcalc_output(output: &[u8]) -> Result<Fingerprint, AcoustidError> {
    if output.len() > MAX_FP_OUTPUT {
        return Err(AcoustidError::OutputTooLarge);
    }
    let value: serde_json::Value =
        serde_json::from_slice(output).map_err(|_| AcoustidError::InvalidFingerprint)?;
    let duration = value
        .get("duration")
        .and_then(serde_json::Value::as_f64)
        .filter(|duration| duration.is_finite() && *duration >= 1.0 && *duration <= 86_400.0)
        .map(|duration| duration.round() as u32)
        .filter(|duration| *duration > 0)
        .ok_or(AcoustidError::InvalidFingerprint)?;
    let fingerprint = value
        .get("fingerprint")
        .and_then(serde_json::Value::as_str)
        .filter(|fingerprint| {
            !fingerprint.is_empty()
                && fingerprint.len() <= 900_000
                && fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        })
        .ok_or(AcoustidError::InvalidFingerprint)?;
    Ok(Fingerprint {
        duration_seconds: duration,
        value: fingerprint.to_string(),
    })
}

/// Use the optional executable override as one argument value; no shell is
/// involved. Otherwise process PATH resolution finds the packaged/runtime tool.
pub fn fpcalc_executable() -> PathBuf {
    if let Some(configured) = std::env::var_os("AUTO_TAG_FPCALC_PATH") {
        return PathBuf::from(configured);
    }
    [
        Path::new("/opt/homebrew/bin/fpcalc"),
        Path::new("/usr/local/bin/fpcalc"),
        Path::new("/opt/local/bin/fpcalc"),
        Path::new("/usr/bin/fpcalc"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
    .map(Path::to_path_buf)
    .unwrap_or_else(|| PathBuf::from("fpcalc"))
}

pub async fn fingerprint_file(
    path: &Path,
    cancelled: &AtomicBool,
) -> Result<Fingerprint, AcoustidError> {
    fingerprint_file_with(
        &fpcalc_executable(),
        path,
        cancelled,
        FP_TIMEOUT,
        MAX_FP_OUTPUT,
        MAX_FP_STDERR,
    )
    .await
}

pub(crate) async fn fingerprint_file_with(
    executable: &Path,
    path: &Path,
    cancelled: &AtomicBool,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<Fingerprint, AcoustidError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(AcoustidError::Cancelled);
    }
    let semaphore = FP_SEMAPHORE.get_or_init(|| Arc::new(Semaphore::new(2)));
    let permit = tokio::select! {
        permit = Arc::clone(semaphore).acquire_owned() => permit.map_err(|_| AcoustidError::ProcessFailed)?,
        _ = wait_for_cancel(cancelled) => return Err(AcoustidError::Cancelled),
    };
    let mut child = Command::new(executable)
        .arg("-json")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| AcoustidError::MissingExecutable)?;
    let stdout = child.stdout.take().ok_or(AcoustidError::ProcessFailed)?;
    let stderr = child.stderr.take().ok_or(AcoustidError::ProcessFailed)?;
    let stdout_reader = tokio::spawn(read_limited(stdout, stdout_limit));
    let stderr_reader = tokio::spawn(read_limited(stderr, stderr_limit));

    let mut timeout_sleep = Box::pin(tokio::time::sleep(timeout));
    let status = tokio::select! {
        status = child.wait() => status.map_err(|_| AcoustidError::ProcessFailed)?,
        _ = &mut timeout_sleep => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            drop(permit);
            return Err(AcoustidError::ProcessTimedOut);
        }
        _ = wait_for_cancel(cancelled) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            drop(permit);
            return Err(AcoustidError::Cancelled);
        }
    };
    drop(permit);
    let (stdout, stdout_oversized) = stdout_reader
        .await
        .map_err(|_| AcoustidError::ProcessFailed)?
        .map_err(|_| AcoustidError::ProcessFailed)?;
    let (_stderr, stderr_oversized) = stderr_reader
        .await
        .map_err(|_| AcoustidError::ProcessFailed)?
        .map_err(|_| AcoustidError::ProcessFailed)?;
    if stdout_oversized || stderr_oversized {
        return Err(AcoustidError::OutputTooLarge);
    }
    if !status.success() {
        // Never include process output in errors; it can contain paths or
        // provider-provided material that should not be copied to diagnostics.
        return Err(AcoustidError::ProcessFailed);
    }
    parse_fpcalc_output(&stdout)
}

async fn read_limited<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut oversized = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let remaining = limit.saturating_sub(bytes.len());
        bytes.extend_from_slice(&buffer[..count.min(remaining)]);
        oversized |= count > remaining;
    }
    Ok((bytes, oversized))
}

async fn wait_for_cancel(cancelled: &AtomicBool) {
    while !cancelled.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AcoustidArtist {
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AcoustidRelease {
    pub id: String,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AcoustidRecording {
    pub id: String,
    pub title: Option<String>,
    pub duration: Option<f64>,
    #[serde(default)]
    pub artists: Vec<AcoustidArtist>,
    #[serde(default)]
    pub releases: Vec<AcoustidRelease>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AcoustidResult {
    pub score: f64,
    #[serde(default)]
    pub recordings: Vec<AcoustidRecording>,
}

#[derive(Deserialize)]
struct LookupResponse {
    status: String,
    results: Option<Vec<AcoustidResult>>,
}

#[derive(Clone)]
pub struct AcoustidClient {
    http: reqwest::Client,
    endpoint: String,
}

impl AcoustidClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            endpoint: API_URL.to_string(),
        }
    }

    #[cfg(test)]
    pub(crate) fn at(http: reqwest::Client, endpoint: String) -> Self {
        Self { http, endpoint }
    }

    pub async fn lookup(
        &self,
        client_key: &str,
        fingerprint: &Fingerprint,
        cancelled: &AtomicBool,
    ) -> Result<Vec<AcoustidResult>, AcoustidError> {
        self.lookup_bounded(client_key, fingerprint, cancelled, MAX_RESPONSE_BYTES)
            .await
    }

    async fn lookup_bounded(
        &self,
        client_key: &str,
        fingerprint: &Fingerprint,
        cancelled: &AtomicBool,
        response_limit: usize,
    ) -> Result<Vec<AcoustidResult>, AcoustidError> {
        wait_for_rate_limit(cancelled).await?;
        let request = self
            .http
            .post(&self.endpoint)
            .timeout(HTTP_TIMEOUT)
            .form(&[
                ("client", client_key),
                ("duration", &fingerprint.duration_seconds.to_string()),
                ("fingerprint", fingerprint.value.as_str()),
                ("meta", "recordings releases"),
            ])
            .send();
        let mut response = tokio::select! {
            result = request => result.map_err(|_| AcoustidError::RequestFailed)?,
            _ = wait_for_cancel(cancelled) => return Err(AcoustidError::Cancelled),
        };
        if !response.status().is_success() {
            return Err(AcoustidError::HttpStatus);
        }
        if response
            .content_length()
            .is_some_and(|length| length > response_limit as u64)
        {
            return Err(AcoustidError::ResponseTooLarge);
        }
        let mut body = Vec::new();
        loop {
            let next = tokio::select! {
                chunk = response.chunk() => chunk.map_err(|_| AcoustidError::RequestFailed)?,
                _ = wait_for_cancel(cancelled) => return Err(AcoustidError::Cancelled),
            };
            let Some(chunk) = next else { break };
            if body.len().saturating_add(chunk.len()) > response_limit {
                return Err(AcoustidError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        let parsed: LookupResponse =
            serde_json::from_slice(&body).map_err(|_| AcoustidError::InvalidResponse)?;
        if parsed.status != "ok" {
            return Err(AcoustidError::ApiRejected);
        }
        let Some(results) = parsed.results else {
            return Err(AcoustidError::InvalidResponse);
        };
        if results.len() > MAX_RESULTS
            || results.iter().any(|result| {
                !result.score.is_finite()
                    || result.recordings.len() > MAX_RECORDINGS
                    || result
                        .recordings
                        .iter()
                        .any(|recording| recording.releases.len() > MAX_RELEASES_PER_RECORDING)
            })
        {
            return Err(AcoustidError::IncompleteResponse);
        }
        Ok(results)
    }
}

async fn wait_for_rate_limit(cancelled: &AtomicBool) -> Result<(), AcoustidError> {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(AcoustidError::RateLimitCancelled);
        }
        let next_request = NEXT_ACOUSTID_REQUEST.get_or_init(|| tokio::sync::Mutex::new(None));
        let wait = {
            let mut next = next_request.lock().await;
            let now = Instant::now();
            match *next {
                Some(ready_at) if ready_at > now => Some(ready_at.duration_since(now)),
                _ => {
                    *next = Some(now + REQUEST_INTERVAL);
                    return Ok(());
                }
            }
        };
        if let Some(wait) = wait {
            tokio::select! {
                _ = tokio::time::sleep(wait) => {},
                _ = wait_for_cancel(cancelled) => return Err(AcoustidError::RateLimitCancelled),
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::thread;

    fn fake_fpcalc(body: &str, exit_code: i32) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "soundrobe-fpcalc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s' '{}'\nexit {}\n",
                body.replace('\'', "'\\''"),
                exit_code
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[test]
    fn parses_full_file_duration_and_rejects_malformed_or_missing_output() {
        let parsed = parse_fpcalc_output(br#"{"duration":125.2,"fingerprint":"abc_123-"}"#)
            .expect("fpcalc's whole-file duration is the API duration");
        assert_eq!(parsed.duration_seconds, 125);
        assert_eq!(parsed.value, "abc_123-");
        assert_eq!(
            parse_fpcalc_output(b"not json"),
            Err(AcoustidError::InvalidFingerprint)
        );
        assert_eq!(
            parse_fpcalc_output(br#"{"duration":20}"#),
            Err(AcoustidError::InvalidFingerprint)
        );
        assert_eq!(
            parse_fpcalc_output(br#"{"duration":0,"fingerprint":"x"}"#),
            Err(AcoustidError::InvalidFingerprint)
        );
    }

    #[tokio::test]
    async fn fpcalc_failures_are_bounded_and_never_echo_process_output() {
        let cancelled = AtomicBool::new(false);
        let audio = Path::new("not-a-real-audio-file.flac");
        let malformed = fake_fpcalc("not json", 0);
        assert_eq!(
            fingerprint_file_with(
                &malformed,
                audio,
                &cancelled,
                Duration::from_secs(1),
                1024,
                1024
            )
            .await,
            Err(AcoustidError::InvalidFingerprint)
        );
        let missing_fields = fake_fpcalc("{\\\"duration\\\":2}", 0);
        assert_eq!(
            fingerprint_file_with(
                &missing_fields,
                audio,
                &cancelled,
                Duration::from_secs(1),
                1024,
                1024
            )
            .await,
            Err(AcoustidError::InvalidFingerprint)
        );
        let oversized = fake_fpcalc(&"x".repeat(40), 0);
        assert_eq!(
            fingerprint_file_with(
                &oversized,
                audio,
                &cancelled,
                Duration::from_secs(1),
                16,
                1024
            )
            .await,
            Err(AcoustidError::OutputTooLarge)
        );
        fs::write(&oversized, "#!/bin/sh\nexec head -c 131072 /dev/zero\n").unwrap();
        assert_eq!(
            fingerprint_file_with(
                &oversized,
                audio,
                &cancelled,
                Duration::from_secs(1),
                16,
                1024,
            )
            .await,
            Err(AcoustidError::OutputTooLarge),
            "oversized output must be drained to avoid blocking the child on a full pipe"
        );
        let nonzero = fake_fpcalc("private-path-secret", 3);
        let error = fingerprint_file_with(
            &nonzero,
            audio,
            &cancelled,
            Duration::from_secs(1),
            1024,
            1024,
        )
        .await
        .unwrap_err();
        assert_eq!(error, AcoustidError::ProcessFailed);
        assert!(!error.to_string().contains("private-path-secret"));
        let missing = Path::new("/this/path/does/not/exist/fpcalc");
        assert_eq!(
            fingerprint_file_with(
                missing,
                audio,
                &cancelled,
                Duration::from_secs(1),
                1024,
                1024
            )
            .await,
            Err(AcoustidError::MissingExecutable)
        );
        for path in [malformed, missing_fields, oversized, nonzero] {
            let _ = fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn fpcalc_timeout_and_cancellation_kill_the_child() {
        let cancelled = AtomicBool::new(false);
        let sleeper = fake_fpcalc("", 0);
        // Replace the script with an exec so killing the child also closes its pipes.
        fs::write(&sleeper, "#!/bin/sh\nexec sleep 2\n").unwrap();
        assert_eq!(
            fingerprint_file_with(
                &sleeper,
                Path::new("x"),
                &cancelled,
                Duration::from_millis(30),
                1024,
                1024
            )
            .await,
            Err(AcoustidError::ProcessTimedOut)
        );
        let task_cancelled = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&task_cancelled);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            signal.store(true, Ordering::Release);
        });
        assert_eq!(
            fingerprint_file_with(
                &sleeper,
                Path::new("x"),
                &task_cancelled,
                Duration::from_secs(2),
                1024,
                1024
            )
            .await,
            Err(AcoustidError::Cancelled)
        );
        let _ = fs::remove_file(sleeper);
    }

    #[test]
    fn api_failures_do_not_embed_credentials_or_fingerprint_material() {
        for error in [
            AcoustidError::ApiRejected,
            AcoustidError::HttpStatus,
            AcoustidError::RequestFailed,
            AcoustidError::InvalidResponse,
        ] {
            let detail = error.to_string();
            assert!(!detail.contains("registered-client-secret"));
            assert!(!detail.contains("fingerprint-private"));
            assert!(!detail.contains("api.acoustid.org"));
        }
    }

    fn response_server(status: &'static str, body: String) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/lookup", listener.local_addr().unwrap());
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                let text = String::from_utf8_lossy(&request);
                if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                    let content_length = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    });
                    if content_length.is_some_and(|length| body.len() >= length) {
                        break;
                    }
                }
            }
            let _ = send.send(String::from_utf8_lossy(&request).into_owned());
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (endpoint, receive)
    }

    fn test_client(endpoint: String) -> AcoustidClient {
        AcoustidClient::at(reqwest::Client::new(), endpoint)
    }

    fn test_fingerprint() -> Fingerprint {
        Fingerprint {
            duration_seconds: 125,
            value: "fingerprint-private".into(),
        }
    }

    #[tokio::test]
    async fn posts_bounded_form_and_parses_recording_release_evidence() {
        let body = r#"{"status":"ok","results":[{"score":0.97,"recordings":[{"id":"recording-1","title":"Song","duration":125,"artists":[{"id":"artist-1","name":"Singer"}],"releases":[{"id":"release-1","title":"Album"}]}]}]}"#.to_string();
        let (endpoint, request) = response_server("200 OK", body);
        let results = test_client(endpoint)
            .lookup(
                "client-key-private",
                &test_fingerprint(),
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
        let sent = request.recv_timeout(Duration::from_secs(2)).unwrap();
        let (headers, form) = sent.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with("POST /lookup HTTP/1.1"));
        assert!(form.contains("client=client-key-private"));
        assert!(form.contains("duration=125"));
        assert!(form.contains("fingerprint=fingerprint-private"));
        assert!(form.contains("meta=recordings+releases"));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].recordings[0].id, "recording-1");
        assert_eq!(results[0].recordings[0].releases[0].id, "release-1");
    }

    #[tokio::test]
    async fn empty_success_is_no_match_but_api_errors_and_oversize_are_failures() {
        let (endpoint, _) = response_server("200 OK", r#"{"status":"ok","results":[]}"#.into());
        assert!(test_client(endpoint)
            .lookup("client", &test_fingerprint(), &AtomicBool::new(false))
            .await
            .unwrap()
            .is_empty());

        let (endpoint, _) = response_server("200 OK", r#"{"status":"error","results":[]}"#.into());
        assert_eq!(
            test_client(endpoint)
                .lookup(
                    "client-secret",
                    &test_fingerprint(),
                    &AtomicBool::new(false)
                )
                .await
                .unwrap_err(),
            AcoustidError::ApiRejected
        );

        let (endpoint, _) = response_server("200 OK", r#"{"status":"ok"}"#.into());
        assert_eq!(
            test_client(endpoint)
                .lookup("client", &test_fingerprint(), &AtomicBool::new(false))
                .await
                .unwrap_err(),
            AcoustidError::InvalidResponse
        );

        let (endpoint, _) = response_server("503 Service Unavailable", "{}".into());
        assert_eq!(
            test_client(endpoint)
                .lookup("client", &test_fingerprint(), &AtomicBool::new(false))
                .await
                .unwrap_err(),
            AcoustidError::HttpStatus
        );

        let (endpoint, _) = response_server(
            "200 OK",
            format!("{{\"status\":\"ok\",\"padding\":\"{}\"}}", "x".repeat(100)),
        );
        assert_eq!(
            test_client(endpoint)
                .lookup_bounded("client", &test_fingerprint(), &AtomicBool::new(false), 32)
                .await
                .unwrap_err(),
            AcoustidError::ResponseTooLarge
        );
    }
}
