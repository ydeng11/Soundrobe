//! Bounded catalog discovery. Does not publish proof or call a writer.

use super::auto_tag::RecordingSource;
use crate::state::{
    providers::{MusicBrainzClient, ProviderRetryContext, ProviderRetryMetrics, ProviderState},
    sqlite::CacheState,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub const SCOPE: &str = "Catalog hypotheses only, not recognition, accepted core, user approval or permission to write. Retained acoustic alternatives remain unresolved. No proof publication or scheduler changes. Networking is opt-in; only the explicit isolated discovery cache is written.";
const CACHE_VERSION: &str = "recording-discovery-v1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    requests: Vec<Request>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    path: PathBuf,
    sha256: String,
    artist: String,
    title: String,
    version: Option<String>,
    changed_fact: String,
    #[serde(default)]
    recording_ids: Vec<String>,
    refresh_revision: Option<String>,
}

/// Read existing file evidence before bounded explicit catalog work. The cache
/// is separate from the application's cache so failed/null observations cannot
/// accidentally become accepted resolver facts.
pub async fn export(
    inventory: &Value,
    manifest_path: &Path,
    cache_dir: &Path,
    provider: &ProviderState,
    allow_network: bool,
    cancelled: &Arc<AtomicBool>,
) -> Result<Value, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut input = options.open(manifest_path).map_err(|e| e.to_string())?;
    let metadata = input.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 65536 {
        return Err("discovery manifest must be a regular file at most 64 KiB".into());
    }
    let mut bytes = Vec::new();
    (&mut input)
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("discovery manifest exceeds 64 KiB".into());
    }
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if manifest.requests.is_empty() || manifest.requests.len() > 8 {
        return Err("select 1 to 8 discovery requests".into());
    }
    for request in &manifest.requests {
        if [&request.artist, &request.title, &request.changed_fact]
            .iter()
            .any(|s| s.trim().is_empty() || s.len() > 1024)
            || request
                .version
                .as_ref()
                .is_some_and(|s| s.trim().is_empty() || s.len() > 1024)
            || request
                .refresh_revision
                .as_ref()
                .is_some_and(|s| s.trim().is_empty() || s.len() > 1024)
            || request.sha256.len() != 64
            || !request.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || request.recording_ids.len() > 16
            || request
                .recording_ids
                .iter()
                .any(|id| uuid::Uuid::parse_str(id).is_err())
        {
            return Err("discovery needs bounded artist/title, changedFact, SHA-256 and valid optional recording IDs/revision".into());
        }
    }
    let root = Path::new(inventory["root"].as_str().ok_or("missing inventory root")?)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let absolute = if cache_dir.is_absolute() {
        cache_dir.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(cache_dir)
    };
    if absolute
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err("discovery cache must not contain parent traversal".into());
    }
    let ancestor = absolute
        .ancestors()
        .find(|p| p.exists())
        .ok_or("cache has no existing ancestor")?;
    let resolved = ancestor
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(absolute.strip_prefix(ancestor).map_err(|e| e.to_string())?);
    let shared = dirs::home_dir().is_some_and(|home| {
        [".soundrobe", ".auto-tagger"]
            .iter()
            .any(|name| resolved.starts_with(home.join(name)))
    });
    if resolved.starts_with(&root)
        || shared
        || resolved.join("worker.db").exists()
        || resolved.join("config.yaml").exists()
        || ["cache.db", "cache.db-wal", "cache.db-shm"]
            .iter()
            .any(|name| fs::symlink_metadata(resolved.join(name)).is_ok_and(|m| !m.is_file()))
    {
        return Err(
            "discovery cache must be isolated from media and application/worker state".into(),
        );
    }
    let cache = CacheState::new_in(resolved);
    if !cache.initialize(None) {
        return Err("discovery cache could not be initialized".into());
    }
    let mut rows = Vec::new();
    let mut operations = 0usize;
    for request in manifest.requests {
        if cancelled.load(Ordering::Acquire) {
            break;
        }
        let file = inventory["files"]
            .as_array()
            .ok_or("missing physical files")?
            .iter()
            .find(|f| {
                f["relativePath"]
                    .as_str()
                    .is_some_and(|s| Path::new(s) == request.path)
            })
            .ok_or("discovery source must be an inventoried relative path")?;
        let path = Path::new(file["path"].as_str().ok_or("missing source path")?);
        let source = RecordingSource::capture(path).map_err(|e| e.to_string());
        let current = source
            .as_ref()
            .ok()
            .and_then(|s| serde_json::to_value(s).ok());
        let mut row = json!({"path":request.path,"source":current,"expectedSha256":request.sha256,
            "changedFact":request.changed_fact,"refreshRevision":request.refresh_revision,
            "lead":{"artist":request.artist,"title":request.title,"version":request.version},
            "retainedEvidence":file["selectedEvidence"],"allStoredEvidence":file["evidence"],
            "candidateSummary":file["candidateSummary"],"details":[],"search":null,"status":"hypotheses_only",
            "stopReason":"catalog_facts_do_not_bind_local_audio","reopenWhen":"Independent source-bound acoustic/reference evidence distinguishes every retained competitor and supports complete ordered billing."});
        if file["sourceStableDuringRead"] != true
            || current.as_ref().is_none_or(|s| {
                s["fingerprint"] != request.sha256.to_ascii_lowercase()
                    || s["fingerprint"] != file["sourceIdentity"]["fingerprint"]
                    || file["sourceIdentity"]
                        .get("physicalIdentity")
                        .is_some_and(|identity| s["physicalIdentity"] != *identity)
            })
            || !path.starts_with(&root)
            || path.canonicalize().ok().as_deref() != Some(path)
        {
            row["status"] = json!("source_changed");
            rows.push(row);
            continue;
        }
        let source = source?;
        let mut ids: BTreeSet<String> = request.recording_ids.iter().cloned().collect();
        let retained = &file["selectedEvidence"]["trackEvidence"];
        for value in std::iter::once(&retained["recordingId"]).chain(
            retained["candidates"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|c| [&c["recordingId"], &c["canonicalRecordingId"]]),
        ) {
            if let Some(id) = value
                .as_str()
                .filter(|id| uuid::Uuid::parse_str(id).is_ok())
            {
                ids.insert(id.to_string());
            }
        }
        let all_ids: Vec<_> = ids.into_iter().collect();
        row["retainedAndExplicitRecordingIds"] = json!(all_ids);
        let mut file_operations = 0;
        for id in all_ids.iter().map(Some).chain(std::iter::once(None)) {
            if cancelled.load(Ordering::Acquire) {
                break;
            }
            let query = json!({"cacheVersion":CACHE_VERSION,"providerBase":provider.musicbrainz_base(),
                "recordingId":id,"artist":if id.is_none() {Some(&request.artist)} else {None},
                "title":if id.is_none() {Some(&request.title)} else {None},
                "version":if id.is_none() {request.version.as_ref()} else {None},"refreshRevision":request.refresh_revision});
            let key = format!(
                "{CACHE_VERSION}:{:x}",
                Sha256::digest(query.to_string().as_bytes())
            );
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            let cached = cache.lookup(&key).filter(|v| {
                v["cacheVersion"] == CACHE_VERSION
                    && (v["terminal"] == true || v["retryAt"].as_i64().is_some_and(|t| t > now))
            });
            let cache_hit = cached.is_some();
            let observation = if let Some(cached) = cached {
                cached
            } else if !allow_network {
                json!({"status":"network_not_enabled"})
            } else if operations >= 32 || (id.is_some() && file_operations >= 4) {
                json!({"status":"budget_limited"})
            } else {
                operations += 1;
                file_operations += usize::from(id.is_some());
                let metrics = ProviderRetryMetrics::default();
                let client = MusicBrainzClient::at(provider.http(), provider.musicbrainz_base())
                    .with_retry_context(ProviderRetryContext::new(
                        Arc::clone(cancelled),
                        metrics.clone(),
                    ));
                let work = async {
                    if let Some(id) = id {
                        match client.recording_by_id_result(id).await {
                            Ok(recording) => {
                                json!({"status":"recording_facts","terminal":true,"recording":recording})
                            }
                            Err(failure) => {
                                json!({"status":"detail_incomplete","terminal":failure.status == Some(200) && failure.observation.is_some(),
                                "kind":failure.kind,"reason":failure.reason,"httpStatus":failure.status,
                                "retryable":failure.retryable,"partialObservation":failure.observation})
                            }
                        }
                    } else {
                        match client
                            .discover_recordings(
                                &request.artist,
                                &request.title,
                                request.version.as_deref(),
                            )
                            .await
                        {
                            Ok(mut value) => {
                                value["terminal"] = json!(true);
                                value
                            }
                            Err(error) => {
                                json!({"status":"search_incomplete","terminal":false,"reason":error})
                            }
                        }
                    }
                };
                let mut result = tokio::select! {
                    result = work => result,
                    _ = async { while !cancelled.load(Ordering::Acquire) { tokio::time::sleep(std::time::Duration::from_millis(50)).await; } } => return Err("recording discovery cancelled".into()),
                };
                result["cacheVersion"] = json!(CACHE_VERSION);
                let completed_at = time::OffsetDateTime::now_utc().unix_timestamp();
                result["observedAt"] = json!(completed_at);
                result["retryCount"] = json!(metrics.retry_count());
                if result["terminal"] != true {
                    let delay = metrics.max_retry_after_seconds().unwrap_or(60).max(60);
                    result["retryAt"] = json!(
                        completed_at.saturating_add(i64::try_from(delay).unwrap_or(i64::MAX))
                    );
                }
                if cancelled.load(Ordering::Acquire) {
                    return Err("recording discovery cancelled".into());
                }
                cache.set_lookup(&key, &query, &result, CACHE_VERSION)?;
                result
            };
            let value = json!({"query":query,"cacheHit":cache_hit,"observation":observation});
            if id.is_none() {
                row["search"] = value;
            } else {
                row["details"].as_array_mut().unwrap().push(value);
            }
        }
        if RecordingSource::capture(path).ok().as_ref() != Some(&source) {
            row["status"] = json!("source_changed");
            row["stopReason"] = json!("source_changed_during_discovery_rebind_before_use");
        }
        rows.push(row);
    }
    Ok(
        json!({"schemaVersion":1,"scope":SCOPE,"requests":rows,"networkOperations":operations,
        "limits":{"selectedFiles":8,"networkOperations":32,"detailOperationsPerFile":4,"searchResultsPerQuery":10,"httpAttemptsPerOperation":3},
        "cancelled":cancelled.load(Ordering::Acquire),"verificationPerformed":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[tokio::test]
    async fn discovery_preserves_empty_and_partial_results_across_restart_without_writes() {
        let temp = std::env::temp_dir().join(format!("discovery-{}", uuid::Uuid::new_v4()));
        let media = temp.join("media");
        fs::create_dir_all(&media).unwrap();
        let media = media.canonicalize().unwrap();
        let path = media.join("track.mp3");
        fs::write(&path, b"unchanged local audio").unwrap();
        let fingerprint = format!("{:x}", Sha256::digest(b"unchanged local audio"));
        let source = json!({"path":path,"fingerprint":fingerprint});
        let inventory = json!({"root":media,"summary":{"verifiedCoreFiles":0},"files":[{
            "path":path,"relativePath":"track.mp3","sourceIdentity":source,"sourceStableDuringRead":true,
            "selectedEvidence":{"trackEvidence":{"candidates":[{"recordingId":"11111111-1111-4111-8111-111111111111"}]}}
        }]});
        let manifest = temp.join("discovery.json");
        fs::write(&manifest, json!({"requests":[{"path":"track.mp3","sha256":fingerprint,
            "artist":"Primary","title":"Song","version":"remix","changedFact":"new primary artist from product evidence"}]}).to_string()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut bytes = [0; 4096];
                let length = stream.read(&mut bytes).unwrap();
                let request = String::from_utf8_lossy(&bytes[..length]);
                let body = if request.contains("/recording/11111111") {
                    json!({"id":"11111111-1111-4111-8111-111111111111","title":"Song (remix)","length":null,
                        "artist-credit":[{"name":"Primary","joinphrase":" feat. ","artist":{"name":"Primary"}},
                                         {"name":"Guest","artist":{"name":"Guest"}}]})
                } else {
                    assert!(request.contains("/recording?"));
                    json!({"count":0,"offset":0,"recordings":[]})
                }.to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        let provider =
            crate::state::providers::ProviderState::at(reqwest::Client::new(), &base, &base);
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cache_dir = temp.join("cache");
        let before = fs::read(&path).unwrap();
        let first = export(
            &inventory, &manifest, &cache_dir, &provider, true, &cancelled,
        )
        .await
        .unwrap();
        assert_eq!(first["networkOperations"], 2);
        assert_eq!(
            first["requests"][0]["search"]["observation"]["status"],
            "successful_empty"
        );
        assert_eq!(
            first["requests"][0]["details"][0]["observation"]["kind"],
            "recording_duration_missing"
        );
        assert_eq!(
            first["requests"][0]["details"][0]["observation"]["partialObservation"]
                ["rawOrderedBilling"][1]["name"],
            "Guest"
        );
        server.join().unwrap();
        let again = export(
            &inventory, &manifest, &cache_dir, &provider, false, &cancelled,
        )
        .await
        .unwrap();
        assert_eq!(again["networkOperations"], 0);
        assert_eq!(again["requests"][0]["search"]["cacheHit"], true);
        assert_eq!(again["requests"][0]["details"][0]["cacheHit"], true);
        assert_eq!(
            again["requests"][0]["retainedEvidence"],
            inventory["files"][0]["selectedEvidence"]
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(inventory["summary"]["verifiedCoreFiles"], 0);
        let offline = export(
            &inventory,
            &manifest,
            &temp.join("empty-cache"),
            &provider,
            false,
            &cancelled,
        )
        .await
        .unwrap();
        assert_eq!(offline["networkOperations"], 0);
        assert_eq!(
            offline["requests"][0]["search"]["observation"]["status"],
            "network_not_enabled"
        );
        #[cfg(unix)]
        {
            let linked = temp.join("linked-cache");
            fs::create_dir(&linked).unwrap();
            std::os::unix::fs::symlink(cache_dir.join("cache.db"), linked.join("cache.db"))
                .unwrap();
            let bytes = fs::read(cache_dir.join("cache.db")).unwrap();
            assert!(
                export(&inventory, &manifest, &linked, &provider, false, &cancelled)
                    .await
                    .is_err()
            );
            assert_eq!(fs::read(cache_dir.join("cache.db")).unwrap(), bytes);
        }
        #[cfg(unix)]
        {
            let mut bound = inventory.clone();
            bound["files"][0]["sourceIdentity"] =
                serde_json::to_value(RecordingSource::capture(&path).unwrap()).unwrap();
            let replacement = media.join("replacement");
            fs::write(&replacement, &before).unwrap();
            fs::rename(replacement, &path).unwrap();
            let replaced = export(&bound, &manifest, &cache_dir, &provider, false, &cancelled)
                .await
                .unwrap();
            assert_eq!(
                replaced["requests"][0]["status"], "source_changed",
                "identical bytes on a new inode cannot inherit the inventory binding"
            );
        }
        let mut stale = inventory.clone();
        fs::write(&path, b"different physical source").unwrap();
        let invalid = export(&stale, &manifest, &cache_dir, &provider, true, &cancelled)
            .await
            .unwrap();
        assert_eq!(invalid["networkOperations"], 0);
        assert_eq!(invalid["requests"][0]["status"], "source_changed");
        stale["files"][0]["sourceStableDuringRead"] = json!(false);
        assert!(
            export(&stale, &manifest, &media, &provider, false, &cancelled)
                .await
                .is_err()
        );
        fs::remove_dir_all(temp).unwrap();
    }

    #[tokio::test]
    async fn discovery_retry_after_starts_at_response_and_survives_restart() {
        let temp = std::env::temp_dir().join(format!("discovery-retry-{}", uuid::Uuid::new_v4()));
        let media = temp.join("media");
        fs::create_dir_all(&media).unwrap();
        let media = media.canonicalize().unwrap();
        let path = media.join("track.mp3");
        fs::write(&path, b"audio").unwrap();
        let fingerprint = format!("{:x}", Sha256::digest(b"audio"));
        let inventory = json!({"root":media,"files":[{"path":path,"relativePath":"track.mp3",
            "sourceIdentity":{"fingerprint":fingerprint},"sourceStableDuringRead":true}]});
        let manifest = temp.join("manifest.json");
        fs::write(
            &manifest,
            json!({"requests":[{"path":"track.mp3","sha256":fingerprint,
            "artist":"Lead","title":"Song","changedFact":"new named source"}]})
            .to_string(),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 4096];
            let _ = stream.read(&mut bytes);
            thread::sleep(std::time::Duration::from_millis(1200));
            stream.write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 120\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
            time::OffsetDateTime::now_utc().unix_timestamp()
        });
        let provider = ProviderState::at(reqwest::Client::new(), &base, &base);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cache = temp.join("cache");
        let first = export(&inventory, &manifest, &cache, &provider, true, &cancelled)
            .await
            .unwrap();
        let response_at = server.join().unwrap();
        let observation = &first["requests"][0]["search"]["observation"];
        assert_eq!(observation["terminal"], false);
        assert!(observation["retryAt"].as_i64().unwrap() >= response_at + 120);
        let second = export(&inventory, &manifest, &cache, &provider, true, &cancelled)
            .await
            .unwrap();
        assert_eq!(second["networkOperations"], 0);
        assert_eq!(second["requests"][0]["search"]["observation"], *observation);
        cancelled.store(true, Ordering::Release);
        assert_eq!(
            export(&inventory, &manifest, &cache, &provider, true, &cancelled)
                .await
                .unwrap()["networkOperations"],
            0
        );
        fs::remove_dir_all(temp).unwrap();
    }

    #[tokio::test]
    async fn discovery_resumes_uncached_details_without_spending_budget_on_old_nulls() {
        let temp = std::env::temp_dir().join(format!("discovery-budget-{}", uuid::Uuid::new_v4()));
        let media = temp.join("media");
        fs::create_dir_all(&media).unwrap();
        let media = media.canonicalize().unwrap();
        let path = media.join("track.mp3");
        fs::write(&path, b"audio").unwrap();
        let fingerprint = format!("{:x}", Sha256::digest(b"audio"));
        let ids: Vec<_> = (1..=6)
            .map(|n| format!("00000000-0000-4000-8000-{n:012}"))
            .collect();
        let inventory = json!({"root":media,"files":[{"path":path,"relativePath":"track.mp3",
            "sourceIdentity":{"fingerprint":fingerprint},"sourceStableDuringRead":true}]});
        let manifest = temp.join("manifest.json");
        fs::write(&manifest, json!({"requests":[{"path":"track.mp3","sha256":fingerprint,
            "artist":"Lead","title":"Song","changedFact":"new retained identity set","recordingIds":ids}]}).to_string()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let mut seen = BTreeSet::new();
            for _ in 0..7 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut bytes = [0; 4096];
                let length = stream.read(&mut bytes).unwrap();
                let request = String::from_utf8_lossy(&bytes[..length]);
                let path = request.split_whitespace().nth(1).unwrap();
                let body = if let Some(id) = path.strip_prefix("/recording/").and_then(|p| p.split('?').next()) {
                    assert!(seen.insert(id.to_string()), "successful null-duration request repeated");
                    json!({"id":id,"title":"Song","length":null,"artist-credit":[{"name":"Lead","artist":{"name":"Lead"}}]})
                } else { json!({"count":0,"offset":0,"recordings":[]}) }.to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
            seen.len()
        });
        let provider = ProviderState::at(reqwest::Client::new(), &base, &base);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cache = temp.join("cache");
        let first = export(&inventory, &manifest, &cache, &provider, true, &cancelled)
            .await
            .unwrap();
        assert_eq!(first["networkOperations"], 5);
        assert_eq!(
            first["requests"][0]["details"][4]["observation"]["status"],
            "budget_limited"
        );
        let next = export(&inventory, &manifest, &cache, &provider, true, &cancelled)
            .await
            .unwrap();
        assert_eq!(next["networkOperations"], 2);
        assert!(next["requests"][0]["details"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["observation"]["kind"] == "recording_duration_missing"));
        assert_eq!(server.join().unwrap(), 6);
        fs::remove_dir_all(temp).unwrap();
    }
}
