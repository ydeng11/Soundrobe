//! Read-only physical-file accounting and evidence triage. Never runs a resolver.

use super::{
    auto_tag::{CoreReceipt, RecordingSource},
    library::is_audio_file,
};
use crate::state::worker::WorkerStore;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const SCOPE: &str = "Read-only inventory; existing tags and historical decisions are not identity proof. Versioned terminal receipts are revalidated against current physical source hashes; legacy evidence remains source-unbound. Verified core, edition, enrichment and human approval remain separate; no recognition or approval is performed.";

#[path = "artist_inventory/recovery_preview.rs"]
mod recovery;
pub use recovery::{export as recovery_preview, markdown as recovery_preview_markdown};

#[cfg(test)]
#[path = "artist_inventory/recovery_preview_tests.rs"]
mod recovery_preview_tests;

/// Current independently rooted M2 labels can contradict a proposal even when
/// their retained facts are too sparse or differently worded to donate it.
pub(crate) fn reuse_root(file: &Value, current_history: &[Value]) -> Result<Value, String> {
    use sha2::{Digest, Sha256};
    if file["coreProof"]["state"] != "valid" {
        return Err("donor_proof_not_current_valid".into());
    }
    let stored = &file["coreProof"]["receipt"];
    let terminal = current_history.iter().any(|attempt| {
        attempt["currentSchedulingRun"] == true
            && attempt["runId"] == stored["runId"]
            && Path::new(text(&attempt["path"]))
                .canonicalize()
                .ok()
                .as_deref()
                == Path::new(text(&file["path"])).parent()
            && matches!(
                text(&attempt["schedulingStatus"]),
                "completed" | "needs_review" | "failed" | "deferred"
            )
            && attempt["evidence"]["proofPublication"]["state"] == "terminal_validated"
            && attempt["evidence"]["proofPublication"]["runId"] == stored["runId"]
            && attempt["evidence"]["coreReceipts"]
                .as_array()
                .is_some_and(|receipts| receipts.contains(stored))
    });
    if !terminal {
        return Err("donor_terminal_ownership_changed_or_pending".into());
    }
    let receipt: CoreReceipt = serde_json::from_value(stored.clone()).map_err(|e| e.to_string())?;
    receipt.validate_current(Path::new(text(&file["path"])))?;
    if receipt.parent_proof.is_some()
        || receipt.run_id.as_deref().is_none_or(str::is_empty)
        || receipt.published_at.is_none()
        || receipt.evidence["queryScope"] != "existing_acoustid_recording_resolution"
    {
        return Err("independent_root_unproven".into());
    }
    let row = &receipt.evidence["trackMatch"];
    let lookup = &row["lookupObservation"];
    if lookup["version"] != 1
        || lookup["provider"] != "acoustid"
        || lookup["status"] != "successful_nonempty"
        || lookup["source"] != json!(receipt.pre_write)
    {
        return Err("independent_lookup_binding_unproven".into());
    }
    Ok(
        json!({"source":receipt.post_write,"accepted":receipt.accepted,"decisionBasis":receipt.decision_basis,
        "root":{"kind":"independent_acoustid_recording_resolution","runId":receipt.run_id,
            "attemptId":receipt.attempt_id,"receiptVersion":receipt.version,"decisionRuleVersion":receipt.decision_rule_version,
            "evidenceVersion":receipt.evidence_version,"receiptDigest":format!("{:x}",Sha256::digest(stored.to_string().as_bytes())),
            "evidenceDigest":receipt.evidence_digest,"preWrite":receipt.pre_write,"postWrite":receipt.post_write,
            "observedAt":receipt.observed_at,"publishedAt":receipt.published_at,"parentProof":null},
        "provenance":receipt.evidence}),
    )
}

/// Donation remains narrower than current independent proof: retained original
/// recording facts must exactly support the complete proposed core.
pub(crate) fn validate_reuse_donation(root: &Value) -> Result<(), String> {
    let row = &root["provenance"]["trackMatch"];
    let lookup = &row["lookupObservation"];
    let core = &root["accepted"];
    let candidates = row["candidates"]
        .as_array()
        .ok_or("root_recording_facts_missing")?;
    let selected = root["decisionBasis"] == "selected_recording";
    let roots: Vec<_> = candidates
        .iter()
        .filter(|c| {
            if selected {
                c["disposition"] == "verified"
                    && c["canonicalRecordingId"] == core["musicbrainzTrackId"]
            } else {
                c["disposition"] == "supported" || c["disposition"] == "verified"
            }
        })
        .collect();
    let unique_ids: BTreeSet<_> = roots
        .iter()
        .filter_map(|c| c["canonicalRecordingId"].as_str())
        .collect();
    if (selected && roots.len() != 1)
        || (!selected
            && (unique_ids.len() < 2
                || unique_ids.len() != roots.len()
                || row["omittedCandidates"] != 0
                || row["candidateCount"] != json!(roots.len())
                || lookup["eligibleHypothesisCount"] != json!(roots.len())))
    {
        return Err("root_recording_facts_incomplete".into());
    }
    for candidate in roots {
        let observation = &candidate["providerObservation"];
        let id = text(&candidate["canonicalRecordingId"]);
        let billing = observation["rawOrderedBilling"]
            .as_array()
            .ok_or("root_billing_missing")?;
        let names: Vec<_> = billing.iter().map(|b| text(&b["name"])).collect();
        let display: String = billing
            .iter()
            .map(|b| format!("{}{}", text(&b["name"]), text(&b["joinphrase"])))
            .collect();
        if uuid::Uuid::parse_str(id).is_err()
            || observation["version"] != 1
            || observation["provider"] != "musicbrainz"
            || observation["sourceLevel"] != "recording"
            || observation["canonicalId"] != id
            || observation["responseId"] != id
            || candidate["authoritativeTitle"] != core["title"]
            || observation["title"] != core["title"]
            || candidate["artistCredits"] != core["artists"]
            || json!(names) != core["artists"]
            || json!(display) != core["artist"]
            || names.iter().any(|s| s.trim().is_empty())
            || billing
                .iter()
                .any(|b| b.get("joinphrase").is_some_and(|v| !v.is_string()))
            || candidate["authoritativeDuration"]
                .as_f64()
                .is_none_or(|d| !d.is_finite() || d <= 0.0)
        {
            return Err("root_recording_core_conflict_or_missing_facts".into());
        }
    }
    Ok(())
}

/// Full-file SHA-256 is a stale-input check, not an audio identity decision.
/// The scan is not a filesystem snapshot: each file records its observation.
pub fn inventory(
    root: &Path,
    worker_data: Option<&Path>,
    historical: Option<&Path>,
) -> Result<Value, String> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    if !root.is_dir() {
        return Err("artist root must be a directory".into());
    }
    let history = historical.map(load_history).transpose()?;
    let worker = worker_data
        .map(WorkerStore::read_only_history)
        .transpose()?
        .unwrap_or_default();
    let mut worker_folders = BTreeMap::<PathBuf, Vec<&Value>>::new();
    for attempt in &worker {
        if let Ok(path) = Path::new(text(&attempt["path"])).canonicalize() {
            worker_folders.entry(path).or_default().push(attempt);
        }
    }
    let mut paths = Vec::new();
    let mut excluded = Vec::new();
    collect(&root, &mut paths, &mut excluded).map_err(|error| error.to_string())?;
    paths.sort();
    excluded.sort();
    let mut files = Vec::new();
    let mut matched_history = BTreeSet::new();
    let mut changed_inputs = 0;
    for path in paths {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("non-UTF-8 media filename")?;
        let folder = path.parent().ok_or("media without parent")?;
        let before = fs::metadata(&path).map_err(|error| error.to_string())?;
        let source = RecordingSource::capture(&path)
            .map_err(|error| error.to_string())
            .and_then(|source| serde_json::to_value(source).map_err(|error| error.to_string()));
        let stable = fs::metadata(&path).is_ok_and(|after| {
            before.len() == after.len() && before.modified().ok() == after.modified().ok()
        });
        let source_error = source.as_ref().err().cloned();
        let source = source.unwrap_or(Value::Null);
        let mut evidence = Vec::new();
        let mut core_proof =
            json!({"version":1,"state":"absent","reason":"no_terminal_source_bound_receipt"});
        for attempt in worker_folders.get(folder).into_iter().flatten() {
            let snapshot = &attempt["evidence"];
            if attempt["currentSchedulingRun"] == true {
                let receipt = snapshot["coreReceipts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|r| r["postWrite"]["path"] == json!(path));
                if let Some(receipt) = receipt {
                    let validated = serde_json::from_value::<CoreReceipt>(receipt.clone())
                        .map_err(|e| e.to_string())
                        .and_then(|r| {
                            if snapshot["proofPublication"]["state"] != "terminal_validated"
                                || snapshot["proofPublication"]["runId"] != attempt["runId"]
                                || json!(r.run_id) != attempt["runId"]
                                || r.published_at.is_none()
                            {
                                return Err("receipt_not_terminal_bound".into());
                            }
                            r.validate_current(&path)
                        });
                    core_proof = json!({"version":1,"state":if validated.is_ok() && stable {"valid"} else {"invalid"},"reason":validated.err(),"receipt":receipt});
                } else if snapshot["pendingCoreReceipts"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|r| r["postWrite"]["path"] == json!(path)))
                {
                    core_proof = json!({"version":1,"state":"pending","reason":"write_observed_without_terminal_proof"});
                }
            }
            let row = track_rows(snapshot)
                .into_iter()
                .find(|row| row["fileName"] == name);
            evidence.push(json!({"source":"worker", "runId":attempt["runId"],
                "currentSchedulingRun":attempt["currentSchedulingRun"],
                "schedulingUpdatedAt":attempt["schedulingUpdatedAt"],"fetchedAt":null,
                "reasonCode":snapshot["reasonCode"],"providerAttempts":snapshot["providerAttempts"],
                "trackEvidence":row,"freshness":"source_unbound"}));
        }
        let historical_file = history.as_ref().and_then(|h| h.files.get(&path));
        let mut freshness = if evidence.is_empty() {
            "absent"
        } else {
            "source_unbound"
        };
        let mut prior_supported = false;
        if let Some(record) = historical_file {
            matched_history.insert(path.clone());
            let unchanged =
                stable && source["fingerprint"] == record["sourceHashBefore"] && !source.is_null();
            freshness = if unchanged {
                "unchanged_historical_input"
            } else {
                "changed_since_historical_input"
            };
            if !unchanged {
                changed_inputs += 1;
            }
            prior_supported = unchanged && record["providerVerifiedCore"] == true;
            evidence.push(json!({"source":"historical_composed", "fetchedAt":null,
                "freshness":freshness,"trackEvidence":record["trackEvidence"],
                "providerBackedCore":record["providerVerifiedCore"]}));
        }
        let selected = evidence
            .iter()
            .rev()
            .find(|e| e["source"] == "historical_composed")
            .or_else(|| evidence.iter().find(|e| e["currentSchedulingRun"] == true))
            .or_else(|| evidence.last());
        let selected = selected.cloned().unwrap_or(Value::Null);
        let row = &selected["trackEvidence"];
        let Triage {
            mut blockers,
            missing,
            conflicts,
            mut actions,
        } = triage(row, &selected["providerAttempts"]);
        if row.is_null() && !prior_supported {
            blockers.insert("evidence_absent".into());
            actions.insert("Collect recording evidence through the existing preview/tagging workflow on an approved copy.".into());
        }
        if freshness == "changed_since_historical_input" || !stable || source_error.is_some() {
            blockers.insert("stale_or_unreadable_source".into());
            actions.insert("Refresh evidence for this changed or unreadable physical file before using any prior decision.".into());
        }
        actions.insert("Revalidate source-bound track core and readback for every folder member before claiming completion.".into());
        let eligible_count = row["lookupObservation"]
            .get("eligibleHypothesisCount")
            .cloned()
            .unwrap_or_else(|| {
                if row["candidateSetComplete"] == true
                    || row["candidateCount"].as_u64().is_some_and(|n| n > 0)
                {
                    row["candidateCount"].clone()
                } else {
                    Value::Null
                }
            });
        let omitted_count = if eligible_count.is_null() {
            Value::Null
        } else {
            row["omittedCandidates"].clone()
        };
        files.push(json!({"path":path,"relativePath":path.strip_prefix(&root).unwrap(),
            "folder":folder.strip_prefix(&root).unwrap(),"sourceIdentity":source,
            "sourceStableDuringRead":stable,"sourceError":source_error,
            "observedAt":time::OffsetDateTime::now_utc().unix_timestamp(),
            "freshness":freshness,"coreProof":core_proof,"coreState":if core_proof["state"] == "valid" {"verified_core"} else if prior_supported {"requires_revalidation"} else {"unresolved"},
            "historicalProviderBackedCore":historical_file.map(|r| r["providerVerifiedCore"].clone()),
            "candidateSummary":{"eligibleHypothesisCount":eligible_count,
                "visibleAlternativeCount":row["candidates"].as_array().map(Vec::len),
                "omittedEligibleHypotheses":omitted_count,
                "candidateSetComplete":row["candidateSetComplete"],
                "fullAlternativeCount":null,"fullAlternativeCountKnown":false},
            "blockers":blockers,"missingFacts":missing,"conflictingFields":conflicts,
            "nextActions":actions,"selectedEvidence":selected,"evidence":evidence}));
    }
    let mut folders = BTreeMap::<String, (usize, usize)>::new();
    for file in &files {
        let counts = folders.entry(text(&file["folder"]).into()).or_default();
        counts.0 += 1;
        counts.1 += usize::from(file["coreState"] == "unresolved");
    }
    // Rank fact-sharing opportunities using explicit recording IDs, never title similarity.
    let mut gap_files = BTreeMap::<String, BTreeSet<String>>::new();
    for file in &files {
        for gap in file["missingFacts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|gap| !text(&gap["recordingId"]).is_empty())
        {
            gap_files
                .entry(gap.to_string())
                .or_default()
                .insert(text(&file["path"]).into());
        }
    }
    let mut worklist = files.clone();
    for file in &mut worklist {
        let unresolved = folders[text(&file["folder"])].1;
        let benefit = file["missingFacts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|gap| gap_files.get(&gap.to_string()).map(BTreeSet::len))
            .max()
            .unwrap_or(1);
        let identification = file["coreState"] == "unresolved";
        file["priorityGroup"] = json!(if identification && unresolved == 1 {
            0
        } else if identification {
            1
        } else {
            2
        });
        file["folderIdentificationUnresolvedFiles"] = json!(unresolved);
        file["sharedFactFileCount"] = json!(benefit);
        file["priorityReason"] = json!(if file["coreState"] == "verified_core" {
            "Current source-bound core proof; edition, enrichment and approval remain separate"
        } else if !identification {
            "Historical support still requires current source/readback validation"
        } else if unresolved == 1 {
            "One identification blocker; other folder members still need revalidation"
        } else {
            "Shared missing facts first, then low-cost provider evidence; all hard cases retained"
        });
    }
    worklist.sort_by_key(|file| {
        (
            file["priorityGroup"].as_u64().unwrap(),
            std::cmp::Reverse(file["sharedFactFileCount"].as_u64().unwrap()),
            !file["blockers"]
                .as_array()
                .unwrap()
                .contains(&json!("provider_failure")),
            text(&file["relativePath"]).to_owned(),
        )
    });
    for (index, file) in worklist.iter_mut().enumerate() {
        file["rank"] = json!(index + 1);
    }
    let unresolved = folders.values().map(|(_, count)| count).sum::<usize>();
    let verified = files
        .iter()
        .filter(|f| f["coreState"] == "verified_core")
        .count();
    let historical = history.map(|h| {
        let mut summary = h.summary;
        summary["matchedInputs"] = json!(matched_history.len());
        summary["changedInputs"] = json!(changed_inputs);
        summary["missingOrOutOfScopeInputs"] = json!(h
            .files
            .keys()
            .filter(|p| !matched_history.contains(*p))
            .collect::<Vec<_>>());
        summary
    });
    Ok(json!({"schemaVersion":2,"root":root,"scope":SCOPE,
        "summary":{"physicalFiles":files.len(),"folders":folders.len(),"verifiedCoreFiles":verified,
            "unresolvedFiles":files.len()-verified,"identificationUnresolvedFiles":unresolved,
            "revalidationFiles":files.len()-unresolved-verified,
            "oneIdentificationBlockerFolders":folders.values().filter(|(_,n)| *n == 1).count()},
        "historical":historical,"folders":folders.into_iter().map(|(path,(total,unresolved))|
            json!({"path":path,"physicalFiles":total,"identificationUnresolvedFiles":unresolved,
                "currentCompletion":if files.iter().filter(|f| text(&f["folder"]) == path).all(|f| f["coreState"] == "verified_core") {"verified_core"} else {"not_verified"}})).collect::<Vec<_>>(),
        "excludedSymlinks":excluded,"reviewQuestions":review_questions(&files),"files":files,"worklist":worklist}))
}

/// Byte equality shares a review question, never a recording identity or proof.
fn review_questions(files: &[Value]) -> Vec<Value> {
    let mut groups = BTreeMap::<(bool, String), Vec<&Value>>::new();
    for file in files.iter().filter(|f| f["coreProof"]["state"] != "valid") {
        let hash = text(&file["sourceIdentity"]["fingerprint"]);
        let hash_bound = file["sourceStableDuringRead"] == true
            && file["sourceError"].is_null()
            && hash.len() == 64
            && hash.bytes().all(|b| b.is_ascii_hexdigit());
        let key = if hash_bound {
            (true, hash.to_owned())
        } else {
            (false, text(&file["path"]).to_owned())
        };
        groups.entry(key).or_default().push(file);
    }
    groups.into_iter().map(|((hash_bound, key), members)| {
        let mut blockers = BTreeSet::new();
        let mut missing = Vec::new();
        let mut conflicts = BTreeSet::new();
        let mut actions = BTreeSet::new();
        let mut candidates = Vec::new();
        for file in &members {
            for (field, target) in [("blockers", &mut blockers),
                ("conflictingFields", &mut conflicts), ("nextActions", &mut actions)] {
                target.extend(file[field].as_array().into_iter().flatten().map(|v| text(v).to_owned()));
            }
            missing.extend(file["missingFacts"].as_array().into_iter().flatten().cloned());
            for evidence in file["evidence"].as_array().into_iter().flatten() {
                let row = &evidence["trackEvidence"];
                let observed = triage(row, &evidence["providerAttempts"]);
                blockers.extend(observed.blockers);
                missing.extend(observed.missing);
                conflicts.extend(observed.conflicts);
                actions.extend(observed.actions);
                candidates.extend(row["candidates"].as_array().into_iter().flatten().cloned());
            }
        }
        // Show cross-attempt disagreements too, without selecting or rejecting a candidate.
        conflicts.extend(triage(&json!({"candidates":candidates}), &Value::Null).conflicts);
        missing.sort_by_cached_key(Value::to_string);
        missing.dedup();
        json!({"groupingBasis":if hash_bound {"exact_whole_file_sha256"} else {"ungrouped_physical_source"},
            "wholeFileHash":if hash_bound {Some(key)} else {None},
            "question":"What independent evidence distinguishes the retained alternatives or supplies the missing core facts for these physical files?",
            "machineIdentityProof":false,
            "scope":"Read-only review. Byte equality is not recording identity. Source-bound user-approved values are never machine proof; no approval, apply or retry is performed.",
            "countScope":"Counts remain per physical member and stored observation, never summed across duplicates or attempts. Null means unknown, not zero; displayed alternatives include rejected candidates. No full alternative count is inferred.",
            "members":members,"blockers":blockers,"missingFacts":missing,"conflictingFields":conflicts,
            "nextDiscriminator":actions,
            "stopReason":"Current retained evidence does not establish source-bound machine-verified core for every member. Keep all alternatives unresolved; repeated unchanged evidence or user-approved values cannot establish machine proof.",
            "reopenWhen":"New independently identity-linked evidence resolves the listed contradictions or missing facts, or a changed/unreadable source is recaptured and revalidated. Recheck each physical member and its proof; a changed hash alone is not identity evidence."})
    }).collect()
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}

fn collect(
    root: &Path,
    files: &mut Vec<PathBuf>,
    excluded: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            excluded.push(entry.path());
        } else if kind.is_dir() {
            collect(&entry.path(), files, excluded)?;
        } else if kind.is_file() && is_audio_file(&entry.path()) {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn track_rows(snapshot: &Value) -> Vec<&Value> {
    // A terminal diagnostic supersedes the earlier checkpoint in `acoustid`.
    if let Some(diag) = snapshot["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .find(|diag| diag["stage"] == "acoustid")
    {
        return diag["trackMatches"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
    }
    snapshot["acoustid"]["trackMatches"]
        .as_array()
        .into_iter()
        .flatten()
        .collect()
}

struct Triage {
    blockers: BTreeSet<String>,
    missing: Vec<Value>,
    conflicts: BTreeSet<String>,
    actions: BTreeSet<String>,
}

/// Classification explains observations; it never changes acceptance rules.
fn triage(row: &Value, attempts: &Value) -> Triage {
    let mut blockers = BTreeSet::new();
    let mut missing = BTreeSet::new();
    let mut conflicts = BTreeSet::new();
    let mut actions = BTreeSet::new();
    let reason = text(&row["reason"]);
    for (pattern, category, action) in [
        ("audio_conflict", "audio_conflict", "Compare complete audio and an independently supported recording reference; do not collapse variants."),
        ("version", "version_credit_conflict", "Confirm the live/remix/edit/demo version using playback and an independent reference."),
        ("credit", "version_credit_conflict", "Verify the complete ordered artist credit, including guests."),
        ("tied", "candidate_ambiguity", "Obtain evidence that distinguishes every competing recording; context alone cannot exclude one."),
        ("duration", "missing_catalog_fact", "Find an independent recording-duration reference; local duration is not provider corroboration."),
        ("without recording identities", "human_identification", "Identify unnamed acoustic competitors independently before selecting a recording."),
        ("unidentified", "human_identification", "Identify unnamed acoustic competitors independently before selecting a recording."),
        ("budget", "provider_failure", "Continue saved bounded candidate hydration only through an explicitly approved retry."),
    ] {
        if reason.contains(pattern) { blockers.insert(category.into()); actions.insert(action.into()); }
    }
    for candidate in row["candidates"].as_array().into_iter().flatten() {
        let failure = &candidate["detailFailure"];
        let kind = text(&failure["kind"]);
        if candidate["disposition"] == "recording_id_unavailable" {
            blockers.insert("human_identification".into());
            actions.insert(
                "Identify unnamed acoustic competitors independently before selecting a recording."
                    .into(),
            );
        }
        if kind.contains("missing") {
            blockers.insert("missing_catalog_fact".into());
            let id = text(&candidate["recordingId"]);
            missing.insert((id.to_owned(), kind.to_owned()));
            actions.insert("Seek the specific missing catalog fact from an identity-linked source; do not repeat a successful-null response.".into());
        } else if !failure.is_null() {
            blockers.insert("provider_failure".into());
            actions.insert("Inspect provider failure and retry eligibility; retry only when policy or new evidence justifies it.".into());
        }
    }
    if attempts
        .as_array()
        .into_iter()
        .flatten()
        .any(|a| a["retryable"] == true || a["status"] == "unavailable")
    {
        blockers.insert("provider_failure".into());
        actions.insert("Inspect the saved provider attempt and retry-after policy before requesting more evidence.".into());
    }
    for key in [
        "authoritativeTitle",
        "artistCredits",
        "authoritativeDuration",
        "disambiguation",
    ] {
        let values = row["candidates"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|&c| !c[key].is_null())
            .map(|c| c[key].to_string())
            .collect::<BTreeSet<_>>();
        if values.len() > 1 {
            conflicts.insert(key.into());
        }
    }
    if blockers.is_empty() && !reason.is_empty() && reason != "recording_fields_consensus" {
        blockers.insert("human_identification".into());
        actions.insert("Inspect the per-file reason and obtain independent identity evidence; keep unresolved alternatives.".into());
    }
    Triage {
        blockers,
        missing: missing
            .into_iter()
            .map(|(id, kind)| json!({"recordingId":id,"fact":kind}))
            .collect(),
        conflicts,
        actions,
    }
}

struct Historical {
    files: BTreeMap<PathBuf, Value>,
    summary: Value,
}

fn read_json(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn load_history(dir: &Path) -> Result<Historical, String> {
    let preservation = read_json(&dir.join("preservation.json"))?;
    let audits = read_json(&dir.join("file-audit.json"))?;
    let tracks = read_json(&dir.join("track-evidence.json"))?;
    let albums = read_json(&dir.join("albums.json"))?;
    let audits = audits.as_array().ok_or("audit must be an array")?;
    let tracks = tracks.as_array().ok_or("track evidence must be an array")?;
    let albums = albums.as_array().ok_or("albums must be an array")?;
    let mut files = BTreeMap::new();
    for record in preservation["records"]
        .as_array()
        .ok_or("preservation records missing")?
    {
        let path = PathBuf::from(record["source"].as_str().ok_or("source path missing")?);
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("source filename missing")?;
        let folder = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            .ok_or("source folder missing")?;
        let matching = audits
            .iter()
            .filter(|a| a["folder"] == folder && a["fileName"] == name)
            .collect::<Vec<_>>();
        if matching.len() != 1
            || !path.is_absolute()
            || record["sourceHashBefore"]
                .as_str()
                .is_none_or(|s| s.len() != 64)
        {
            return Err(format!(
                "ambiguous or incomplete historical source mapping: {}",
                path.display()
            ));
        }
        let rows = tracks
            .iter()
            .filter(|a| a["folder"] == folder && a["fileName"] == name)
            .collect::<Vec<_>>();
        if rows.len() > 1 {
            return Err("duplicate historical track evidence".into());
        }
        if files.insert(path, json!({"sourceHashBefore":record["sourceHashBefore"],
            "providerVerifiedCore":matching[0]["providerVerifiedCore"],"trackEvidence":rows.first()})).is_some() {
            return Err("duplicate historical source path".into());
        }
    }
    if files.len() != audits.len() {
        return Err("historical audit/source counts disagree".into());
    }
    let supported = audits
        .iter()
        .filter(|a| a["providerVerifiedCore"] == true)
        .count();
    let mut folders = BTreeMap::<String, usize>::new();
    for row in audits {
        *folders.entry(text(&row["folder"]).into()).or_default() +=
            usize::from(row["providerVerifiedCore"] != true);
    }
    Ok(Historical {
        files,
        summary: json!({"scope":"Historical composed replay, not a fresh census or independent identity accuracy proof",
        "path":dir,"physicalFiles":audits.len(),"folders":folders.len(),
        "completeFolders":albums.iter().filter(|a| a["status"] == "completed").count(),
        "providerBackedCoreFiles":supported,"identificationUnresolvedFiles":audits.len()-supported,
        "oneIdentificationBlockerFolders":folders.values().filter(|n| **n == 1).count(),
        "providerFetchedAt":null}),
    })
}

pub fn markdown(report: &Value) -> String {
    let mut out = format!("# Artist-library inventory\n\n{}\n\nRoot: {}\n\nCurrent accounting: {} physical files in {} folders; {} freshly verified core files; {} identification blockers and {} historical-support files requiring revalidation.\n\n",
        SCOPE, text(&report["root"]),report["summary"]["physicalFiles"],report["summary"]["folders"],
        report["summary"]["verifiedCoreFiles"],report["summary"]["identificationUnresolvedFiles"],report["summary"]["revalidationFiles"]);
    if !report["historical"].is_null() {
        out.push_str(&format!(
            "Historical composed baseline (not current verification):\n\n```json\n{}\n```\n\n",
            serde_json::to_string_pretty(&report["historical"]).unwrap()
        ));
    }
    out.push_str("## Review questions\n\nRead-only; exact whole-file duplicates share a question, not an identity decision. Source-bound user-approved values are never machine proof. Detailed evidence remains below.\n\n");
    for (index, question) in report["reviewQuestions"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        out.push_str(&format!(
            "### Question {}\n\n{}\n\nGrouping: {}; whole-file SHA-256: {}.\n\nCount scope: {}\n\n",
            index + 1,
            text(&question["question"]),
            text(&question["groupingBasis"]),
            question["wholeFileHash"],
            text(&question["countScope"])
        ));
        for member in question["members"].as_array().into_iter().flatten() {
            out.push_str(&format!("- File: {}; hash: {}; proof: {}; freshness: {}; stable read: {}. Selected count scope: {}\n",
                text(&member["relativePath"]),member["sourceIdentity"]["fingerprint"],
                member["coreProof"]["state"],text(&member["freshness"]),member["sourceStableDuringRead"],member["candidateSummary"]));
            for evidence in member["evidence"].as_array().into_iter().flatten() {
                let row = &evidence["trackEvidence"];
                let alternatives: Vec<_> = row["candidates"].as_array().into_iter().flatten().map(|c|
                    json!({"recordingId":c["recordingId"],"canonicalRecordingId":c["canonicalRecordingId"],
                        "title":c["authoritativeTitle"],"disposition":c["disposition"],
                        "sourceLevel":c["providerObservation"]["sourceLevel"]})).collect();
                out.push_str(&format!("  - Stored source: {}; run: {}; fetched: {}; alternatives: {}. Raw observation counts (not a full count): candidateCount={}, omitted={}, complete={}.\n",
                    text(&evidence["source"]),evidence["runId"],evidence["fetchedAt"],json!(alternatives),
                    row["candidateCount"],row["omittedCandidates"],row["candidateSetComplete"]));
            }
        }
        out.push_str(&format!("\nBlockers: {}\n\nContradictions: {}\n\nMissing fields: {}\n\nNext discriminator: {}\n\nStop: {}\n\nReopen when: {}\n\n",
            question["blockers"],question["conflictingFields"],question["missingFacts"],question["nextDiscriminator"],
            text(&question["stopReason"]),text(&question["reopenWhen"])));
    }
    for file in report["worklist"].as_array().into_iter().flatten() {
        let row = &file["selectedEvidence"]["trackEvidence"];
        out.push_str(&format!("## {}. {}\n\n{}; freshness: {}.\n\nEligible hypotheses: {}; displayed alternatives (including rejected): {}; omitted eligible hypotheses: {}; candidate set complete: {}. Full alternative count: unknown (not recorded by legacy evidence). Null means unknown, not zero.\n\nBlockers: {}\n\nMissing facts: {}\n\nObserved candidate disagreements (not new rejection rules): {}\n\nNext actions: {}\n\n<details>\n<summary>All stored attempts and alternatives (fetch times may be unknown)</summary>\n\n```json\n{}\n```\n\n</details>\n\n",
            file["rank"],text(&file["relativePath"]),text(&file["priorityReason"]),text(&file["freshness"]),
            file["candidateSummary"]["eligibleHypothesisCount"],row["candidates"].as_array().map(Vec::len).map_or("unknown".into(),|n|n.to_string()),file["candidateSummary"]["omittedEligibleHypotheses"],row["candidateSetComplete"],
            file["blockers"],file["missingFacts"],file["conflictingFields"],file["nextActions"],
            serde_json::to_string_pretty(&file["evidence"]).unwrap()));
        out.push_str(&format!(
            "### Current per-file proof\n\n```json\n{}\n```\n\n",
            serde_json::to_string_pretty(&file["coreProof"]).unwrap()
        ));
    }
    if let Some(evidence) = report.get("audioEvidence") {
        out.push_str(&format!(
            "## Offline selected-pair audio diagnostics\n\n{}\n\n```json\n{}\n```\n",
            crate::infra::audio_evidence::SCOPE,
            serde_json::to_string_pretty(evidence).unwrap()
        ));
    }
    if let Some(proposals) = report.get("reuseProposals") {
        out.push_str(&format!(
            "## Verified-audio reuse proposals (no writes)\n\n{}\n\n```json\n{}\n```\n",
            crate::infra::audio_evidence::REUSE_SCOPE,
            serde_json::to_string_pretty(proposals).unwrap()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::worker::WorkerStore;
    use serde_json::json;
    use std::{fs, path::PathBuf};

    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!("artist-inventory-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn review_questions_group_exact_bytes_without_losing_sources_or_changing_accounting() {
        let root = fixture();
        let library = root.join("artist");
        fs::create_dir_all(&library).unwrap();
        for (name, bytes) in [("a.mp3", "same"), ("b.mp3", "same"), ("c.mp3", "different")] {
            fs::write(library.join(name), bytes).unwrap();
        }
        let data = root.join("data");
        let worker = WorkerStore::new_in(data.clone());
        assert!(worker.initialize());
        for run in ["older", "newer"] {
            worker
                .record_evidence(
                    &library,
                    run,
                    &json!({"acoustid":{"trackMatches":
                (["a.mp3", "b.mp3", "c.mp3"].map(|name| json!({
                    "fileName":name,"candidateCount":0,"candidateSetComplete":false,
                    "omittedCandidates":0,"reason":"recording_candidates_tied",
                    "candidates":[{"recordingId":run,"authoritativeTitle":"Same title",
                        "providerObservation":{"sourceLevel":"recording","title":"Same title"},
                        "detailFailure":{"kind":"recording_duration_missing"}},
                        {"recordingId":"rejected","disposition":"rejected",
                         "authoritativeTitle":"Same title (Live)"}]})))}}),
                )
                .unwrap();
        }
        let before = fs::read(data.join("worker.db")).unwrap();
        let report = inventory(&library, Some(&data), None).unwrap();
        let questions = report["reviewQuestions"]
            .as_array()
            .expect("review questions export");
        assert_eq!(
            questions.len(),
            2,
            "same titles with different bytes are not grouped"
        );
        let question = questions
            .iter()
            .find(|q| q["members"].as_array().unwrap().len() == 2)
            .unwrap();
        assert_eq!(question["groupingBasis"], "exact_whole_file_sha256");
        assert_eq!(question["machineIdentityProof"], false);
        assert_eq!(
            question["missingFacts"].as_array().unwrap().len(),
            2,
            "one concise gap per recording/fact; repeated source observations remain in members"
        );
        assert!(question["conflictingFields"]
            .as_array()
            .unwrap()
            .contains(&json!("authoritativeTitle")));
        assert!(question["stopReason"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        assert!(question["reopenWhen"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        for (index, name) in ["a.mp3", "b.mp3"].iter().enumerate() {
            let member = &question["members"][index];
            assert_eq!(member["relativePath"], *name);
            assert_eq!(
                member,
                report["files"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|f| f["relativePath"] == *name)
                    .unwrap()
            );
            assert_eq!(
                member["evidence"].as_array().unwrap().len(),
                2,
                "retain historical attempts, not just selected alternatives"
            );
            assert!(member["candidateSummary"]["eligibleHypothesisCount"].is_null());
            assert!(member["candidateSummary"]["omittedEligibleHypotheses"].is_null());
            assert_eq!(
                member["candidateSummary"]["fullAlternativeCountKnown"],
                false
            );
        }
        assert_eq!(
            report["summary"],
            json!({"physicalFiles":3,"folders":1,
            "verifiedCoreFiles":0,"unresolvedFiles":3,"identificationUnresolvedFiles":3,
            "revalidationFiles":0,"oneIdentificationBlockerFolders":0})
        );
        for (index, file) in report["worklist"].as_array().unwrap().iter().enumerate() {
            assert_eq!(file["rank"], index + 1);
            assert_eq!(file["priorityGroup"], 1);
            assert_eq!(file["sharedFactFileCount"], 3);
            assert_eq!(file["relativePath"], ["a.mp3", "b.mp3", "c.mp3"][index]);
        }
        let rendered = markdown(&report);
        for label in [
            "## Review questions",
            "Stop:",
            "Reopen when:",
            "Next discriminator:",
            "All stored attempts and alternatives",
            "Count scope:",
            "Same title (Live)",
        ] {
            assert!(rendered.contains(label), "missing {label}");
        }
        assert_eq!(fs::read(data.join("worker.db")).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn review_questions_exclude_valid_proof_but_not_approved_or_unstable_sources() {
        let root = fixture();
        fs::write(root.join("one.mp3"), b"media").unwrap();
        let report = inventory(&root, None, None).unwrap();
        let original = report["files"][0].clone();
        // This projection consumes the inventory's already validated proof state;
        // it must not turn another policy state into a new proof mechanism.
        let mut valid = original.clone();
        valid["coreProof"] = json!({"state":"valid","receipt":{"retained":"proof"}});
        valid["coreState"] = json!("verified_core");
        let mut approved = original.clone();
        approved["path"] = json!(root.join("approved.mp3"));
        approved["coreState"] = json!("user_approved");
        approved["userApproval"] =
            json!({"sourceIdentity":original["sourceIdentity"],"title":"Approved"});
        let mut unstable = original.clone();
        unstable["path"] = json!(root.join("unstable.mp3"));
        unstable["sourceStableDuringRead"] = json!(false);
        let files = vec![valid, approved.clone(), unstable.clone()];
        let before = files.clone();
        let questions = review_questions(&files);
        assert_eq!(
            files, before,
            "review projection must not upgrade or mutate facts"
        );
        assert_eq!(
            questions.len(),
            2,
            "unstable hashes cannot join a stable group"
        );
        let members: Vec<_> = questions
            .iter()
            .flat_map(|q| q["members"].as_array().unwrap())
            .collect();
        assert_eq!(
            members.len(),
            2,
            "valid proof must not become an unresolved question"
        );
        assert!(members.contains(&&approved));
        assert!(members.contains(&&unstable));
        assert!(questions.iter().all(|q| q["machineIdentityProof"] == false));
        assert!(review_questions(&before[..1]).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inventory_accounts_for_files_and_preserves_bounded_competitors_without_promoting_tags() {
        let root = fixture();
        let library = root.join("artist");
        for name in [
            "Single/one.mp3",
            "Multi/a.flac",
            "Multi/disc/b.wav",
            "loose.MP3",
        ] {
            let path = library.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"unreadable media still counts").unwrap();
        }
        let data = root.join("data");
        let worker = WorkerStore::new_in(data.clone());
        assert!(worker.initialize());
        worker.record_evidence(&library.join("Single"), "old-run", &json!({
            "acoustid":{"trackMatches":[{"fileName":"one.mp3", "candidateCount":17,
                "omittedCandidates":16,"candidateSetComplete":false,
                "reason":"recording_candidates_incomplete", "candidates":[{
                    "recordingId":"gap", "detailFailure":{"kind":"recording_duration_missing",
                    "retryable":false,"status":200},"disposition":"recording_duration_missing"}]}]}
        })).unwrap();
        for (folder, name, complete) in [("Multi", "a.flac", false), ("Multi/disc", "b.wav", true)]
        {
            worker.record_evidence(&library.join(folder), "seed-run", &json!({"acoustid":{"trackMatches":[{"fileName":name,"candidateCount":0,"omittedCandidates":0,"candidateSetComplete":complete,"candidates":[]}]}})).unwrap();
        }
        let before = fs::read(data.join("worker.db")).unwrap();
        let report = inventory(&library, Some(&data), None).unwrap();
        assert_eq!(report["summary"]["physicalFiles"], 4);
        assert_eq!(report["summary"]["verifiedCoreFiles"], 0);
        assert_eq!(report["summary"]["unresolvedFiles"], 4);
        let file = report["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["relativePath"] == "Single/one.mp3")
            .unwrap();
        assert_eq!(file["evidence"][0]["trackEvidence"]["candidateCount"], 17);
        assert_eq!(file["candidateSummary"]["eligibleHypothesisCount"], 17);
        assert_eq!(file["candidateSummary"]["visibleAlternativeCount"], 1);
        assert!(file["candidateSummary"]["fullAlternativeCount"].is_null());
        assert_eq!(file["candidateSummary"]["fullAlternativeCountKnown"], false);
        let summaries = |name: &str| {
            report["files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["relativePath"] == name)
                .unwrap()["candidateSummary"]
                .clone()
        };
        assert!(summaries("Multi/a.flac")["eligibleHypothesisCount"].is_null());
        assert!(summaries("Multi/a.flac")["omittedEligibleHypotheses"].is_null());
        let mut unknown_only = report.clone();
        unknown_only["worklist"] = json!(report["worklist"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["relativePath"] == "Multi/a.flac")
            .collect::<Vec<_>>());
        assert!(markdown(&unknown_only).contains("omitted eligible hypotheses: null"));
        assert_eq!(summaries("Multi/disc/b.wav")["eligibleHypothesisCount"], 0);
        assert_eq!(
            file["evidence"][0]["trackEvidence"]["omittedCandidates"],
            16
        );
        assert!(file["blockers"]
            .as_array()
            .unwrap()
            .contains(&json!("missing_catalog_fact")));
        assert_eq!(file["freshness"], "source_unbound");
        assert_eq!(fs::read(data.join("worker.db")).unwrap(), before);
        assert!(report["worklist"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["rank"].is_u64()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_database_is_not_created_and_symlinks_are_not_followed() {
        let root = fixture();
        fs::write(root.join("one.mp3"), b"media").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("one.mp3"), root.join("alias.mp3")).unwrap();
        let data = root.join("absent-data");
        let report = inventory(&root, Some(&data), None).unwrap();
        assert_eq!(report["summary"]["physicalFiles"], 1);
        assert!(!data.exists());
        #[cfg(unix)]
        assert_eq!(report["excludedSymlinks"].as_array().unwrap().len(), 1);
        assert!(markdown(&report).contains("not identity proof"));
        assert!(inventory(&root.join("missing"), None, None).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminal_diagnostics_supersede_checkpoints_and_retain_multiple_blockers() {
        let root = fixture();
        let album = root.join("Album");
        fs::create_dir_all(&album).unwrap();
        fs::write(album.join("song.mp3"), b"media").unwrap();
        let data = root.join("data");
        let worker = WorkerStore::new_in(data.clone());
        assert!(worker.initialize());
        worker
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        worker.mark_running(&album, "current").unwrap();
        worker.record_evidence(&album, "current", &json!({
            "acoustid":{"trackMatches":[{"fileName":"song.mp3","reason":"old checkpoint"}]},
            "diagnostics":[{"stage":"acoustid","trackMatches":[{
                "fileName":"song.mp3","candidateCount":3,"omittedCandidates":0,
                "reason":"recording_audio_conflict","candidates":[{
                    "recordingId":"missing", "detailFailure":{"kind":"recording_duration_missing","retryable":false}
                }, {"recordingId":"outage","detailFailure":{"kind":"http_error","retryable":true,"status":503}},
                {"disposition":"recording_id_unavailable"}]}]}]
        })).unwrap();
        let report = inventory(&root, Some(&data), None).unwrap();
        let file = &report["files"][0];
        assert_eq!(
            file["selectedEvidence"]["trackEvidence"]["candidateCount"],
            3
        );
        for category in [
            "audio_conflict",
            "missing_catalog_fact",
            "provider_failure",
            "human_identification",
        ] {
            assert!(
                file["blockers"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(category)),
                "{category}"
            );
        }
        assert_eq!(file["selectedEvidence"]["currentSchedulingRun"], true);
        assert_eq!(file["freshness"], "source_unbound");
        worker
            .record_evidence(
                &album,
                "current",
                &json!({"providerAttempts":[{
            "provider":"musicbrainz","status":"unavailable","retryable":false,
            "diagnostic":"HTTP 401"}]}),
            )
            .unwrap();
        let unavailable = inventory(&root, Some(&data), None).unwrap();
        assert!(
            unavailable["files"][0]["blockers"]
                .as_array()
                .unwrap()
                .contains(&json!("provider_failure")),
            "An authorization failure must not disappear merely because retryable=false"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn historical_audits_rank_one_blocker_first_but_never_certify_current_files() {
        let root = fixture();
        let library = root.join("artist");
        let history = root.join("history");
        fs::create_dir_all(&history).unwrap();
        let mut inputs = Vec::new();
        let mut audits = Vec::new();
        let mut tracks = Vec::new();
        for (folder, name, supported) in [
            ("Almost", "a.mp3", true),
            ("Almost", "b.mp3", false),
            ("Hard", "a.mp3", false),
            ("Hard", "b.mp3", false),
        ] {
            let path = library.join(folder).join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"media").unwrap();
            inputs.push(json!({"source":path.canonicalize().unwrap(),
                "sourceHashBefore":"721c9525ade2ea8903d343ef25cf68b9bf4ab0aad56bb7b01fbe48d09bc7fcf4"}));
            audits.push(json!({"folder":folder,"fileName":name,"providerVerifiedCore":supported}));
            tracks.push(json!({"folder":folder,"fileName":name,"candidateCount":2,
                "candidateSetComplete":true,"omittedCandidates":0,
                "reason":if supported {None} else {Some("recording_version_conflict")},
                "candidates":[{"authoritativeTitle":"Song", "artistCredits":["Lead","Guest"]},
                {"authoritativeTitle":"Song (Live)", "artistCredits":["Guest","Lead"]}]}));
        }
        for row in tracks.iter_mut().filter(|row| row["folder"] == "Hard") {
            row["candidates"][0]["recordingId"] = json!("same-recording");
            row["candidates"][0]["detailFailure"] = json!({"kind":"recording_duration_missing"});
        }
        for (name, value) in [
            ("preservation.json", json!({"records":inputs})),
            ("file-audit.json", json!(audits)),
            ("track-evidence.json", json!(tracks)),
            (
                "albums.json",
                json!([{"folder":"Almost","status":"needs_review"},
                {"folder":"Hard","status":"needs_review"}]),
            ),
        ] {
            fs::write(history.join(name), value.to_string()).unwrap();
        }
        let report = inventory(&library, None, Some(&history)).unwrap();
        assert_eq!(report["historical"]["providerBackedCoreFiles"], 1);
        assert_eq!(report["summary"]["identificationUnresolvedFiles"], 3);
        assert_eq!(report["summary"]["revalidationFiles"], 1);
        assert_eq!(report["worklist"][0]["relativePath"], "Almost/b.mp3");
        assert_eq!(report["worklist"][1]["relativePath"], "Hard/a.mp3");
        assert_eq!(report["worklist"][1]["sharedFactFileCount"], 2);
        assert_eq!(
            report["worklist"][0]["freshness"],
            "unchanged_historical_input"
        );
        assert!(report["worklist"][0]["conflictingFields"]
            .as_array()
            .unwrap()
            .contains(&json!("artistCredits")));
        fs::write(library.join("Almost/a.mp3"), b"replacement").unwrap();
        let changed = inventory(&library, None, Some(&history)).unwrap();
        assert_eq!(changed["summary"]["verifiedCoreFiles"], 0);
        assert_eq!(changed["summary"]["identificationUnresolvedFiles"], 4);
        assert_eq!(changed["historical"]["changedInputs"], 1);
        let mut manifest = read_json(&history.join("preservation.json")).unwrap();
        let duplicate = manifest["records"][0].clone();
        manifest["records"].as_array_mut().unwrap().push(duplicate);
        fs::write(history.join("preservation.json"), manifest.to_string()).unwrap();
        assert!(inventory(&library, None, Some(&history))
            .unwrap_err()
            .contains("duplicate"));
        fs::remove_dir_all(root).unwrap();
    }
}
