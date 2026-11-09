//! Reviewed catalog localizations, bound to exact provider occurrences.
//! No lookup, transliteration, edition inference or display-title replacement.

use super::providers::ProviderAlbum;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

const CATALOG: &str = include_str!("../../../data/provider-title-aliases.json");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleAliasEvidence {
    pub title: String,
    pub receipt_id: String,
    pub sources: Vec<AliasSource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AliasSource {
    pub request_url: String,
    pub sha256: String,
    pub captured_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Source {
    #[serde(flatten)]
    receipt: AliasSource,
    raw_body: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Binding {
    id: String,
    release_id: String,
    artist_id: String,
    recording_id: String,
    title: String,
    disambiguation: String,
    disc_number: u32,
    track_number: u32,
    track_count: usize,
    length_ms: u64,
    source_indices: [usize; 2],
    store_track_id: u64,
    store_collection_id: u64,
    store_artist_id: u64,
    store_disc_number: u64,
    store_track_number: u64,
    store_length_ms: u64,
}

struct ReviewedBinding {
    binding: Binding,
    aliases: Vec<TitleAliasEvidence>,
}

fn read_catalog(text: &str) -> Result<Vec<ReviewedBinding>, String> {
    #[derive(Deserialize)]
    struct Catalog {
        sources: Vec<Source>,
        bindings: Vec<Binding>,
    }
    let catalog: Catalog = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut reviewed = Vec::new();
    for binding in catalog.bindings {
        let reject = || format!("invalid title alias receipt {}", binding.id);
        let mut titles = Vec::new();
        let mut sources = Vec::new();
        let mut countries = Vec::new();
        for index in binding.source_indices {
            let source = catalog.sources.get(index).ok_or_else(reject)?;
            if format!("{:x}", Sha256::digest(source.raw_body.as_bytes())) != source.receipt.sha256
            {
                return Err(reject());
            }
            let url = reqwest::Url::parse(&source.receipt.request_url).map_err(|_| reject())?;
            let query = url
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            if url.scheme() != "https"
                || url.host_str() != Some("itunes.apple.com")
                || url.path() != "/lookup"
                || query.get("id").map(|s| s.as_ref())
                    != Some(binding.store_collection_id.to_string().as_str())
                || query.get("entity").map(|s| s.as_ref()) != Some("song")
            {
                return Err(reject());
            }
            countries.push(query.get("country").ok_or_else(reject)?.to_string());
            let body: serde_json::Value =
                serde_json::from_str(&source.raw_body).map_err(|_| reject())?;
            let results = body["results"].as_array().ok_or_else(reject)?;
            let tracks = results
                .iter()
                .filter(|r| r["trackId"].as_u64() == Some(binding.store_track_id))
                .collect::<Vec<_>>();
            let albums = results
                .iter()
                .filter(|r| r["wrapperType"] == "collection")
                .collect::<Vec<_>>();
            if tracks.len() != 1 || albums.len() != 1 {
                return Err(reject());
            }
            let (track, album) = (tracks[0], albums[0]);
            for row in [track, album] {
                if row["collectionId"].as_u64() != Some(binding.store_collection_id)
                    || row["artistId"].as_u64() != Some(binding.store_artist_id)
                {
                    return Err(reject());
                }
            }
            if track["discNumber"].as_u64() != Some(binding.store_disc_number)
                || track["trackNumber"].as_u64() != Some(binding.store_track_number)
                || track["trackTimeMillis"].as_u64() != Some(binding.store_length_ms)
                || album["trackCount"].as_u64() != Some(binding.track_count as u64)
            {
                return Err(reject());
            }
            let title = track["trackName"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(reject)?;
            titles.push(title.to_string());
            sources.push(source.receipt.clone());
        }
        if countries[0] == countries[1] || titles[0] == titles[1] {
            return Err(reject());
        }
        let aliases = titles
            .into_iter()
            .map(|title| TitleAliasEvidence {
                title,
                receipt_id: binding.id.clone(),
                sources: sources.clone(),
            })
            .collect();
        reviewed.push(ReviewedBinding { binding, aliases });
    }
    Ok(reviewed)
}

pub(super) fn enrich(album: &mut ProviderAlbum) {
    static REVIEWED: OnceLock<Vec<ReviewedBinding>> = OnceLock::new();
    let reviewed = REVIEWED
        .get_or_init(|| read_catalog(CATALOG).expect("bundled title alias receipts must validate"));
    for entry in reviewed {
        let binding = &entry.binding;
        if album.id != binding.release_id
            || album.artist_id.as_deref() != Some(&binding.artist_id)
            || album.disambiguation.as_deref() != Some(&binding.disambiguation)
            || album.tracks.len() != binding.track_count
            || album
                .tracks
                .iter()
                .filter(|t| t.recording_id.as_deref() == Some(&binding.recording_id))
                .count()
                != 1
        {
            continue;
        }
        for track in &mut album.tracks {
            if track.recording_id.as_deref() != Some(&binding.recording_id)
                || track.disc_number != Some(binding.disc_number)
                || track.track_number != Some(binding.track_number)
                || track.title.as_deref() != Some(&binding.title)
                || !track.match_titles.iter().all(|t| t == &binding.title)
                || track.length != Some(binding.length_ms as f64)
                || track.artist != album.artist
                || track.artists != album.artists
            {
                continue;
            }
            for alias in &entry.aliases {
                if !track.match_titles.contains(&alias.title) {
                    track.match_titles.push(alias.title.clone());
                }
            }
            track.title_alias_evidence = entry.aliases.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_alias_receipts_reject_broken_provenance_and_cross_product_evidence() {
        assert_eq!(read_catalog(CATALOG).unwrap()[0].aliases.len(), 2);
        let original: serde_json::Value = serde_json::from_str(CATALOG).unwrap();
        for (pointer, value) in [
            ("/sources/0/sha256", serde_json::json!("wrong-hash")),
            (
                "/sources/0/requestUrl",
                serde_json::json!("https://itunes.apple.com/lookup?id=1&entity=song&country=US"),
            ),
            ("/bindings/0/sourceIndices", serde_json::json!([0, 0])),
            ("/bindings/0/storeArtistId", serde_json::json!(1)),
            ("/bindings/0/storeCollectionId", serde_json::json!(1)),
            ("/bindings/0/storeTrackId", serde_json::json!(1)),
            ("/bindings/0/storeDiscNumber", serde_json::json!(1)),
            ("/bindings/0/storeTrackNumber", serde_json::json!(3)),
            ("/bindings/0/storeLengthMs", serde_json::json!(388000)),
        ] {
            let mut changed = original.clone();
            *changed.pointer_mut(pointer).unwrap() = value;
            assert!(read_catalog(&changed.to_string()).is_err(), "{pointer}");
        }
        for recompute_hash in [false, true] {
            let mut changed = original.clone();
            let mut body: serde_json::Value =
                serde_json::from_str(changed["sources"][0]["rawBody"].as_str().unwrap()).unwrap();
            let song = body["results"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|r| r["trackId"] == 1440748484)
                .unwrap();
            song["artistId"] = serde_json::json!(999);
            let bytes = body.to_string();
            changed["sources"][0]["rawBody"] = serde_json::json!(bytes);
            if recompute_hash {
                changed["sources"][0]["sha256"] =
                    serde_json::json!(format!("{:x}", Sha256::digest(bytes.as_bytes())));
            }
            assert!(
                read_catalog(&changed.to_string()).is_err(),
                "recomputed hash: {recompute_hash}"
            );
        }
    }
}
