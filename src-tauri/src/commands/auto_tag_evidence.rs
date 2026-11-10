//! Offline evaluator observations only; never consumed by the resolver.
use super::*;

pub(super) fn guard_observations(request: &LookupRequest, inventory: &Value) -> Value {
    let mut sources = Vec::new();
    for receipt in inventory["records"].as_array().into_iter().flatten() {
        let query = receipt["query"]
            .as_str()
            .and_then(|query| serde_json::from_str::<Value>(query).ok());
        let received = inventory["requests"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|attempt| {
                attempt["provider"] == receipt["provider"]
                    && attempt["path"] == receipt["path"]
                    && Some(&attempt["query"]) == query.as_ref()
                    && attempt["status"] == 200
            });
        if !received || receipt["status"] != 200 {
            continue;
        }
        let body = &receipt["body"];
        let mut occurrences = Vec::new();
        if receipt["provider"] == "musicbrainz" {
            for (medium, value) in body["media"].as_array().into_iter().flatten().enumerate() {
                for (index, track) in value["tracks"].as_array().into_iter().flatten().enumerate() {
                    occurrences.push((format!("/media/{medium}/tracks/{index}"), track));
                }
            }
        } else if receipt["provider"] == "discogs" {
            for (index, track) in body["tracklist"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                occurrences.push((format!("/tracklist/{index}"), track));
            }
        }
        if occurrences.is_empty() {
            continue;
        }
        let mut pairs = Vec::new();
        for (local_index, local) in request.tracks.iter().enumerate() {
            for (selector, raw) in &occurrences {
                let recording_title = raw["recording"]["title"].as_str();
                let remote = TrackCandidate {
                    title: raw["title"]
                        .as_str()
                        .or(recording_title)
                        .map(str::to_string),
                    match_titles: recording_title
                        .filter(|title| Some(*title) != raw["title"].as_str())
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                    release_disambiguation: body["disambiguation"].as_str().map(str::to_string),
                    ..Default::default()
                };
                let filename = local.filename.as_deref().unwrap_or_default();
                let disambiguation = raw["recording"]["disambiguation"].as_str();
                let local_title = local
                    .title
                    .as_deref()
                    .or(local.filename.as_deref())
                    .unwrap_or_default();
                let calls = json!([
                    {"context":"named_local_to_raw_source_diagnostic", "function":"track_versions_compatible",
                     "args":[local,filename,remote],
                     "result":super::super::super::track_matcher::track_versions_compatible(local,filename,&remote)},
                    {"context":"named_local_to_raw_source_diagnostic", "function":"recording_versions_compatible",
                     "args":[local,recording_title,disambiguation],
                     "result":recording_title.map(|title| super::super::super::auto_tag_acoustid::recording_versions_compatible(local,title,disambiguation))},
                    {"context":"genre_route_diagnostic_not_execution", "function":"track_versions_compatible",
                     "args":[remote,"",local],
                     "result":super::super::super::track_matcher::track_versions_compatible(&remote,"",local)},
                    {"context":"genre_route_diagnostic_not_execution", "function":"recording_versions_compatible",
                     "args":[remote,local_title,Value::Null],
                     "result":super::super::super::auto_tag_acoustid::recording_versions_compatible(&remote,local_title,None)}
                ]);
                pairs.push(json!({"localIndex":local_index,"sourceSelector":selector,
                    "rawRecordingDisambiguation":raw["recording"]["disambiguation"],"calls":calls}));
            }
        }
        sources.push(
            json!({"provider":receipt["provider"],"path":receipt["path"],
            "query":query,"sha256":receipt["sha256"],"pairs":pairs}),
        );
    }
    json!(sources)
}

#[test]
fn evidence_export_retains_disambiguation_even_when_unchanged_guards_pass() {
    let local = TrackCandidate {
        title: Some("Old Time's Sake".into()),
        filename: Some("13 Old Time's Sake".into()),
        ..Default::default()
    };
    let request = LookupRequest {
        tracks: vec![local],
        ..Default::default()
    };
    let raw: Value = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/auto-tag-eval/provider-snapshots/discovery/details-2026-11-07/musicbrainz-22be98f32a7f4174.json"
    )).unwrap();
    let receipt = json!({"provider":"musicbrainz", "path":"/release/10e769d6-c110-4e1f-934e-2d12677d4fbb", "query":"[]", "sha256":"fixture", "status":200, "body":raw});
    let inventory = json!({"records":[receipt], "requests":[{"provider":"musicbrainz", "path":"/release/10e769d6-c110-4e1f-934e-2d12677d4fbb", "query":[], "status":200}]});
    let exported = guard_observations(&request, &inventory);
    let row = &exported[0]["pairs"][12];
    assert_eq!(row["rawRecordingDisambiguation"], "explicit");
    assert_eq!(row["localIndex"], 0);
    assert_eq!(row["sourceSelector"], "/media/0/tracks/12");
    assert_eq!(row["calls"][1]["function"], "recording_versions_compatible");
    assert_eq!(row["calls"][1]["args"][2], "explicit");
    assert_eq!(row["calls"][1]["result"], true);
    assert_eq!(row["calls"][3]["args"][2], Value::Null);
    assert_eq!(
        row["calls"][3]["context"],
        "genre_route_diagnostic_not_execution"
    );
    let unreceived = json!({"records":inventory["records"], "requests":[]});
    assert_eq!(guard_observations(&request, &unreceived), json!([]));
}
