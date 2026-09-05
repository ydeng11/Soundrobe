//! Explicit live latency gate. Copies source audio and never invokes a writer.
use super::*;
use crate::infra::openrouter::{CompletionDiagnostics, TagCorrectionPolicy};
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

struct CopiedAlbum(PathBuf);

#[test]
fn benchmark_records_non_object_responses_without_aborting_remaining_trials() {
    let request = LookupRequest::default();
    for data in [
        json!([]),
        json!([{}]),
        json!(null),
        json!(true),
        json!(1),
        json!("invalid"),
    ] {
        assert_eq!(
            structure_and_evidence(&request, &data),
            json!({"structure_valid": false, "code": "ai_validation_failed"}),
            "malformed responses must be recorded without interrupting the benchmark"
        );
    }
}

#[test]
fn benchmark_checks_coverage_even_when_real_confidence_gate_rejects() {
    let request = LookupRequest {
        artist_hint: Some("Twins".into()),
        album_hint: Some("LOL Live In HK".into()),
        tracks: vec![TrackCandidate {
            title: Some("One".into()),
            artist: Some("Twins".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut data = json!({"artist": "Twins", "artists": ["Twins"], "albumArtist": "Twins", "albumArtists": ["Twins"],
        "album": "LOL Live In HK", "year": "2016", "genre": null, "confidence": 0.8,
        "tracks": [{"index": 0, "title": "One", "artist": "Twins", "artists": ["Twins"], "trackNumber": 1, "trackTotal": 1, "discNumber": 1, "discTotal": 1}]});
    assert_eq!(
        validated_ai_candidate(&request, &data).unwrap_err().code,
        "ai_low_confidence"
    );
    assert_eq!(
        structure_and_evidence(&request, &data)["structure_valid"],
        true
    );
    data["tracks"] = json!([]);
    assert_eq!(
        structure_and_evidence(&request, &data)["structure_valid"],
        false
    );
    assert_eq!(
        data["confidence"], 0.8,
        "benchmark inspection must not change the response"
    );
    let (_, schema) = tag_correction_request(&request);
    let required = schema["properties"]["tracks"]["items"]["required"]
        .as_array()
        .unwrap();
    assert!(required.contains(&json!("trackTotal")) && required.contains(&json!("discTotal")));
}

impl Drop for CopiedAlbum {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn media_hashes(directory: &Path) -> Vec<String> {
    collect_audio_files(directory)
        .iter()
        .map(|path| {
            let mut file = fs::File::open(path).expect("read-only media hashing");
            let mut hash = Sha256::new();
            let mut buffer = [0; 65_536];
            loop {
                let count = file.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
            format!("{:x}", hash.finalize())
        })
        .collect()
}

async fn generation_stats(key: &str, id: &str) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .unwrap();
    let mut last_status = None;
    let mut last_error = "unavailable";
    for attempt in 0..3 {
        let response = client
            .get("https://openrouter.ai/api/v1/generation")
            .bearer_auth(key)
            .query(&[("id", id)])
            .send()
            .await;
        if let Ok(response) = response {
            last_status = Some(response.status().as_u16());
            last_error = "http_error";
            if response.status().is_success() {
                if let Ok(payload) = response.json::<Value>().await {
                    let data = &payload["data"];
                    let mut safe = serde_json::Map::new();
                    for field in [
                        "latency",
                        "generation_time",
                        "moderation_latency",
                        "native_tokens_prompt",
                        "native_tokens_completion",
                        "native_tokens_reasoning",
                        "total_cost",
                        "cancelled",
                        "streamed",
                    ] {
                        if let Some(value) =
                            data.get(field).filter(|v| v.is_number() || v.is_boolean())
                        {
                            safe.insert(field.into(), value.clone());
                        }
                    }
                    for field in ["provider_name", "model", "finish_reason"] {
                        if let Some(value) = data.get(field).and_then(Value::as_str).filter(|v| {
                            v.len() <= 160
                                && v.chars()
                                    .all(|c| c.is_ascii_alphanumeric() || "-_/.: ~".contains(c))
                        }) {
                            safe.insert(field.into(), json!(value));
                        }
                    }
                    return Value::Object(safe);
                }
            }
        }
        if attempt < 2 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    json!({"unavailable": true, "status": last_status, "error_code": last_error})
}

fn fresh_generation(attempt: &crate::infra::openrouter::AttemptDiagnostics, stats: &Value) -> bool {
    if attempt
        .cache_status
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("HIT"))
    {
        return false;
    }
    // OpenRouter response-cache hits zero all billable token counters and omit
    // router metadata. Positive completion usage plus a selected provider is
    // therefore generation evidence even when /generation is unavailable.
    attempt
        .cache_status
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("MISS"))
        || stats["generation_time"].as_u64().is_some_and(|ms| ms > 0)
        || (attempt.completion_tokens.is_some_and(|tokens| tokens > 0)
            && attempt.selected_provider.is_some())
}

#[test]
fn cache_evidence_does_not_require_the_optional_generation_endpoint() {
    use crate::infra::openrouter::AttemptDiagnostics;
    let mut attempt = AttemptDiagnostics {
        completion_tokens: Some(2445),
        selected_provider: Some("DeepInfra".into()),
        ..Default::default()
    };
    assert!(fresh_generation(&attempt, &json!({"unavailable": true})));
    attempt.cache_status = Some("hit".into());
    assert!(!fresh_generation(
        &attempt,
        &json!({"generation_time": 100})
    ));
    assert!(!fresh_generation(
        &AttemptDiagnostics::default(),
        &json!({})
    ));
}

fn structure_and_evidence(request: &LookupRequest, data: &Value) -> Value {
    // This cloned value is used only for structural inspection after the real
    // confidence gate. It can never authorize a write or alter the live result.
    let mut structural = data.clone();
    let Some(object) = structural.as_object_mut() else {
        return json!({"structure_valid": false, "code": "ai_validation_failed"});
    };
    object.insert("confidence".into(), json!(1.0));
    let candidate = match validated_ai_candidate(request, &structural) {
        Ok(candidate) => candidate,
        Err(error) => return json!({"structure_valid": false, "code": error.code}),
    };
    let normalize =
        |value: &str| normalized_release_identity(&convert_chinese_text(value, "simplified"));
    let mut title_mismatches = Vec::new();
    let mut credit_mismatches = Vec::new();
    let mut position_mismatches = Vec::new();
    for (index, (local, output)) in request.tracks.iter().zip(&candidate.tracks).enumerate() {
        let output_title = normalize(output.title.as_deref().unwrap_or_default());
        let filename_title = local.filename.as_deref().and_then(filename_track_title);
        if ![local.title.as_deref(), filename_title.as_deref()]
            .into_iter()
            .flatten()
            .any(|title| normalize(title) == output_title)
        {
            title_mismatches.push(index);
        }
        if local.artist.as_deref().is_some_and(|artist| {
            normalize(artist) != normalize(output.artist.as_deref().unwrap_or_default())
        }) {
            credit_mismatches.push(index);
        }
        if local
            .track_number
            .is_some_and(|n| Some(n) != output.track_number)
            || local
                .disc_number
                .is_some_and(|n| Some(n) != output.disc_number)
        {
            position_mismatches.push(index);
        }
    }
    json!({"structure_valid": true, "track_count": candidate.tracks.len(),
        "title_mismatch_indices": title_mismatches, "credit_mismatch_indices": credit_mismatches,
        "position_mismatch_indices": position_mismatches,
        "album_matches_hint": request.album_hint.as_deref().is_some_and(|hint| album_names_match(hint, candidate.album.as_deref().unwrap_or_default())),
        "album_comparison": {"local_normalized": normalize(request.album_hint.as_deref().unwrap_or_default()),
            "proposed_normalized": normalize(candidate.album.as_deref().unwrap_or_default())},
        "artist_matches_hint": request.artist_hint.as_deref().is_some_and(|hint| exact_artist_identity(hint, candidate.artist.as_deref().unwrap_or_default())),
        "confidence": data.get("confidence")})
}

#[tokio::test]
#[ignore = "requires explicit real-album path, credentials, and six live OpenRouter requests"]
async fn live_auto_tag_deepseek_latency() {
    let source = PathBuf::from(
        std::env::var("SOUNDROBE_LATENCY_SOURCE").expect("explicit source album required"),
    );
    let config = AutoTagConfig {
        llm_api_key: Some(std::env::var("LLM_API_KEY").expect("LLM_API_KEY required")),
        llm_model: Some(std::env::var("LLM_MODEL").expect("LLM_MODEL required")),
        llm_provider: Some("openrouter".into()),
        llm_base_url: Some("https://openrouter.ai/api/v1".into()),
        ..Default::default()
    };
    assert_eq!(
        config.llm_model.as_deref(),
        Some("deepseek/deepseek-v4-flash-0731")
    );
    let original_hashes = media_hashes(&source);
    assert_eq!(
        original_hashes.len(),
        34,
        "this gate is calibrated for the 34-track album"
    );
    let root = std::env::temp_dir().join(format!(
        "soundrobe-latency-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let copy = CopiedAlbum(root);
    let album = copy
        .0
        .join(source.parent().unwrap().file_name().unwrap())
        .join(source.file_name().unwrap());
    fs::create_dir_all(&album).unwrap();
    for path in collect_audio_files(&source) {
        let file = Path::new(&path);
        fs::copy(file, album.join(file.file_name().unwrap())).unwrap();
    }
    assert_eq!(media_hashes(&album), original_hashes);
    let request = build_lookup_request(&album).unwrap();
    assert_eq!(request.tracks.len(), 34);
    let (messages, schema) = tag_correction_request(&request);
    let mut final_results = Vec::new();
    for phase in [
        "baseline",
        "reasoning_off",
        "routing",
        "final_1",
        "final_2",
        "final_3",
    ] {
        let mut client = tag_correction_client(&config, request.tracks.len()).unwrap();
        if phase == "baseline" || phase == "reasoning_off" {
            client.test_policy = Some(TagCorrectionPolicy {
                disable_reasoning: phase == "reasoning_off",
                performance_routing: false,
                router_metadata: true,
            });
        }
        println!(
            "BENCHMARK_START {}",
            json!({"phase": phase, "model": config.llm_model, "timeout_ms": 120000, "max_tokens": 5376})
        );
        let mut diagnostics = CompletionDiagnostics::default();
        let started = Instant::now();
        let result = client
            .complete_json_observed(
                messages.clone(),
                "TagCorrectionResponse",
                schema.clone(),
                &AtomicBool::new(false),
                &mut diagnostics,
            )
            .await;
        let validation = result
            .as_ref()
            .map(|response| validated_ai_candidate(&request, &response.data));
        let elapsed_ms = started.elapsed().as_millis();
        let outcome = match &validation {
            Ok(Ok(_)) => "validated",
            Ok(Err(error)) => error.code,
            Err(error) => error.diagnostic_code(),
        };
        let evidence = result
            .as_ref()
            .map_or(json!({"structure_valid": false}), |r| {
                structure_and_evidence(&request, &r.data)
            });
        let mut stats = Vec::new();
        for attempt in &diagnostics.attempts {
            stats.push(match &attempt.generation_id {
                Some(id) => generation_stats(config.llm_api_key.as_deref().unwrap(), id).await,
                None => json!({"unavailable": true}),
            });
        }
        let reasoning_zero = !diagnostics.attempts.is_empty()
            && diagnostics
                .attempts
                .iter()
                .zip(&stats)
                .all(|(attempt, stats)| {
                    attempt
                        .reasoning_tokens
                        .or_else(|| stats["native_tokens_reasoning"].as_u64())
                        == Some(0)
                });
        // Positive generation evidence is required when the cache header is absent.
        let fresh = diagnostics
            .attempts
            .iter()
            .zip(&stats)
            .all(|(attempt, stats)| fresh_generation(attempt, stats));
        let passed = elapsed_ms <= 45000
            && evidence["structure_valid"] == true
            && reasoning_zero
            && fresh
            && matches!(outcome, "validated" | "ai_low_confidence");
        println!(
            "BENCHMARK_RESULT {}",
            json!({"phase": phase, "elapsed_ms": elapsed_ms, "outcome": outcome,
            "diagnostics": diagnostics, "generation_stats": stats, "evidence": evidence,
            "reasoning_zero": reasoning_zero, "fresh_generation": fresh, "performance_passed": passed})
        );
        if phase.starts_with("final_") {
            final_results.push(passed);
        }
    }
    let original_unchanged = media_hashes(&source) == original_hashes;
    let copy_unchanged = media_hashes(&album) == original_hashes;
    println!(
        "BENCHMARK_INTEGRITY {}",
        json!({"original_unchanged": original_unchanged, "copy_unchanged": copy_unchanged, "tracks": 34})
    );
    assert!(original_unchanged && copy_unchanged);
    assert!(final_results.iter().all(|passed| *passed), "45-second live gate unmet; inspect every BENCHMARK_RESULT, including metadata quality evidence");
}
