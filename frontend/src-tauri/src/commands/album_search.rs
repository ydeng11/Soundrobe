//! Manual album search commands: search releases, resolve release detail,
//! preview local-to-remote track matching, and apply a user-edited candidate.
//!
//! These commands are used by the Search button (manual workflow) and do not
//! change the existing auto-tag pipeline.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    commands::{
        auto_tag::{
            apply_selected_candidate_tags, convert_candidate_chinese, discogs_candidate,
            musicbrainz_candidate, AlbumCandidate, TrackCandidate,
        },
        library::collect_audio_files,
        track_matcher::match_remote_candidate_tracks,
        tracks::read_album,
    },
    state::{
        config::ConfigState,
        providers::{
            DiscogsClient, MusicBrainzClient, ProviderAlbum, ProviderState, ReleaseSearchSummary,
        },
        write_queue::WriteQueue,
    },
};

// ── Request / response types ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchReleasesRequest {
    pub provider: String,
    /// At least one of artist or album is required.
    pub artist: Option<String>,
    /// At least one of artist or album is required.
    pub album: Option<String>,
    pub year: Option<String>,
    pub country: Option<String>,
    pub format: Option<String>,
    pub catalog_number: Option<String>,
    pub barcode: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchReleasesResponse {
    pub results: Vec<ReleaseSearchSummary>,
    pub page: u32,
    pub page_size: u32,
    pub total: Option<u32>,
    pub has_next: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveReleaseRequest {
    pub provider: String,
    pub release_id: String,
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewMatchRequest {
    pub album_path: String,
    pub release: ProviderAlbum,
    pub provider: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackMappingRow {
    pub local_index: usize,
    pub local_title: Option<String>,
    pub local_artist: Option<String>,
    pub remote_index: Option<usize>,
    pub remote_title: Option<String>,
    pub remote_artist: Option<String>,
    pub remote_track_number: Option<u32>,
    pub remote_track_total: Option<u32>,
    pub evidence: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewMatchResult {
    pub release: ProviderAlbum,
    pub candidates: Vec<TrackMappingRow>,
    pub unused_remote_indices: Vec<usize>,
    pub album_candidate: AlbumCandidate,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyCandidateRequest {
    pub album_path: String,
    pub candidate: AlbumCandidate,
    pub selected_track_indices: Vec<usize>,
}

// ── Helpers ──────────────────────────────────────────────────────────

fn discogs_token(config: &ConfigState) -> Option<String> {
    config.raw().discogs_token.clone()
}

fn normalise_page_size(page_size: Option<u32>) -> u32 {
    page_size.unwrap_or(10).clamp(1, 100)
}

// ── Commands ─────────────────────────────────────────────────────────

/// Inner search with pre-normalised inputs. Trims all string values and
/// omits empty ones downstream. Returns `Err` when both artist and album
/// are empty after trimming.
async fn search_releases_inner(
    provider: &str,
    trimmed_artist: Option<String>,
    trimmed_album: Option<String>,
    year: Option<String>,
    country: Option<String>,
    format: Option<String>,
    catalog_number: Option<String>,
    barcode: Option<String>,
    page: u32,
    page_size: u32,
    providers: &ProviderState,
    discogs_token: Option<String>,
) -> Result<SearchReleasesResponse, String> {
    // Normalise every string input: trim and omit empty.
    let artist = trimmed_artist
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let album = trimmed_album
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let year = year
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let country = country
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let format = format
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let catno = catalog_number
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let bc = barcode
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if artist.is_none() && album.is_none() {
        return Err("Artist or album is required".into());
    }
    match provider {
        "musicbrainz" => {
            let client = MusicBrainzClient::at(providers.http(), providers.musicbrainz_base());
            let musicbrainz_artist_id = if let Some(ref artist) = artist {
                providers
                    .resolve_musicbrainz_artist_id_result(artist)
                    .await?
            } else {
                None
            };
            let mut query_parts: Vec<(&str, &str)> = Vec::new();
            if let Some(ref id) = musicbrainz_artist_id {
                query_parts.push(("arid", id.as_str()));
            } else if let Some(ref a) = artist {
                query_parts.push(("artist", a.as_str()));
            }
            if let Some(ref a) = album {
                query_parts.push(("release", a.as_str()));
            }
            if let Some(ref y) = year {
                query_parts.push(("date", y.as_str()));
            }
            if let Some(ref c) = country {
                query_parts.push(("country", c.as_str()));
            }
            if let Some(ref f) = format {
                query_parts.push(("format", f.as_str()));
            }
            if let Some(ref cn) = catno {
                query_parts.push(("catno", cn.as_str()));
            }
            if let Some(ref b) = bc {
                query_parts.push(("barcode", b.as_str()));
            }
            let offset = (page - 1) * page_size;
            let (summaries, total) = client
                .search_release_summaries(&query_parts, page_size, offset)
                .await?;
            let has_next = offset + page_size < total;
            Ok(SearchReleasesResponse {
                results: summaries,
                page,
                page_size,
                total: Some(total),
                has_next,
            })
        }
        "discogs" => {
            let client =
                DiscogsClient::at(providers.http(), discogs_token, providers.discogs_base());
            let mut params: Vec<(&str, &str)> = Vec::new();
            if let Some(ref a) = artist {
                params.push(("artist", a.as_str()));
            }
            if let Some(ref a) = album {
                params.push(("release_title", a.as_str()));
            }
            if let Some(ref y) = year {
                params.push(("year", y.as_str()));
            }
            if let Some(ref c) = country {
                params.push(("country", c.as_str()));
            }
            if let Some(ref f) = format {
                params.push(("format", f.as_str()));
            }
            if let Some(ref cn) = catno {
                params.push(("catno", cn.as_str()));
            }
            if let Some(ref b) = bc {
                params.push(("barcode", b.as_str()));
            }
            let (summaries, total) = client
                .search_release_summaries(&params, page, page_size)
                .await?;
            let has_next = (page * page_size) < total;
            Ok(SearchReleasesResponse {
                results: summaries,
                page,
                page_size,
                total: Some(total),
                has_next,
            })
        }
        other => Err(format!("Unknown provider: {other}")),
    }
}

/// Lightweight paged release search.
/// Returns summary records only — no per-result track detail fetch.
#[tauri::command]
pub async fn album_search_releases(
    request: SearchReleasesRequest,
    providers: State<'_, ProviderState>,
    config: State<'_, ConfigState>,
) -> Result<SearchReleasesResponse, String> {
    let page = request.page.unwrap_or(1).max(1);
    let page_size = normalise_page_size(request.page_size);

    let token = discogs_token(&config);
    search_releases_inner(
        &request.provider,
        request.artist.clone(),
        request.album.clone(),
        request.year.clone(),
        request.country.clone(),
        request.format.clone(),
        request.catalog_number.clone(),
        request.barcode.clone(),
        page,
        page_size,
        &providers,
        token,
    )
    .await
}

/// Resolve a single release by provider + ID, returning full `ProviderAlbum` with tracks.
#[tauri::command]
pub async fn album_resolve_release(
    request: ResolveReleaseRequest,
    providers: State<'_, ProviderState>,
    config: State<'_, ConfigState>,
) -> Result<ProviderAlbum, String> {
    resolve_release_inner(&request, &providers, &config).await
}

async fn resolve_release_inner(
    request: &ResolveReleaseRequest,
    providers: &ProviderState,
    config: &ConfigState,
) -> Result<ProviderAlbum, String> {
    match request.provider.as_str() {
        "musicbrainz" => {
            let client =
                MusicBrainzClient::at(providers.http(), providers.musicbrainz_base());
            client
                .release_by_id_result(&request.release_id)
                .await
        }
        "discogs" => {
            let token = discogs_token(config);
            let client = DiscogsClient::new(providers.http(), token);
            match request.kind.as_deref() {
                Some("master") => client
                    .master_metadata(&request.release_id)
                    .await
                    .ok_or_else(|| format!("Discogs master not found: {}", request.release_id)),
                _ => client
                    .release_metadata(&request.release_id)
                    .await
                    .ok_or_else(|| format!("Discogs release not found: {}", request.release_id)),
            }
        }
        other => Err(format!("Unknown provider: {other}")),
    }
}

/// Preview local-to-remote track matching for a selected release.
/// The release was already resolved on the renderer side, so this command
/// receives the full `ProviderAlbum` and runs matching against local tracks.
#[tauri::command]
pub async fn album_preview_release_match(
    request: PreviewMatchRequest,
) -> Result<PreviewMatchResult, String> {
    let album_path = PathBuf::from(&request.album_path);

    // Read local tracks
    let album_detail = read_album(&album_path).map_err(|e| format!("Failed to read album: {e}"))?;

    // Convert resolved release to AlbumCandidate
    let album_candidate = match request.provider.as_str() {
        "musicbrainz" => musicbrainz_candidate(request.release.clone()),
        "discogs" => discogs_candidate(request.release.clone()),
        other => return Err(format!("Unknown provider: {other}")),
    };

    // Build local TrackCandidates from album_detail.tracks
    let local_tracks: Vec<TrackCandidate> = album_detail
        .tracks
        .iter()
        .map(|t| TrackCandidate {
            title: t.title.clone(),
            match_titles: vec![t.title.clone().unwrap_or_default()],
            artist: t.artist.clone(),
            artists: t.artists.clone(),
            track_number: t.track_number,
            track_total: t.track_total,
            disc_number: t.disc_number,
            disc_total: t.disc_total,
            musicbrainz_track_id: t.musicbrainz_track_id.clone(),
            length: Some(t.duration),
            genre: t.genre.clone(),
            filename: Path::new(&t.path)
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string()),
        })
        .collect();

    let filenames: Vec<String> = album_detail
        .tracks
        .iter()
        .map(|t| {
            Path::new(&t.path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();

    // Run track matching
    let matched = match_remote_candidate_tracks(
        &local_tracks,
        &filenames,
        &album_candidate.tracks,
        &request.provider,
        &[],
        &[],
    );

    // Build mapping rows from matcher output
    let mut mapping: Vec<TrackMappingRow> = Vec::new();
    let mut used_remote = vec![false; album_candidate.tracks.len()];

    for (local_idx, local_t) in local_tracks.iter().enumerate() {
        let remote_idx = matched.remote_indices.get(local_idx).copied().flatten();
        let ev = matched.evidence.get(local_idx).copied().flatten();

        if let Some(ri) = remote_idx {
            if ri < used_remote.len() {
                used_remote[ri] = true;
            }
        }

        mapping.push(TrackMappingRow {
            local_index: local_idx,
            local_title: local_t.title.clone(),
            local_artist: local_t.artist.clone(),
            remote_index: remote_idx,
            remote_title: remote_idx
                .and_then(|ri| album_candidate.tracks.get(ri).and_then(|t| t.title.clone())),
            remote_artist: remote_idx.and_then(|ri| {
                album_candidate
                    .tracks
                    .get(ri)
                    .and_then(|t| t.artist.clone())
            }),
            remote_track_number: remote_idx
                .and_then(|ri| album_candidate.tracks.get(ri).and_then(|t| t.track_number)),
            remote_track_total: remote_idx
                .and_then(|ri| album_candidate.tracks.get(ri).and_then(|t| t.track_total)),
            evidence: ev.map(|e| format!("{e:?}")),
        });
    }

    let unused_remote_indices: Vec<usize> = (0..album_candidate.tracks.len())
        .filter(|i| !used_remote[*i])
        .collect();

    Ok(PreviewMatchResult {
        release: request.release,
        candidates: mapping,
        unused_remote_indices,
        album_candidate,
    })
}

/// Apply a user-edited album candidate to the given album directory.
/// Validates the positional track selection, applies the configured
/// Chinese-script conversion, then writes only explicitly selected rows.
#[tauri::command]
pub async fn album_search_apply_candidate(
    request: ApplyCandidateRequest,
    config: State<'_, ConfigState>,
    queue: State<'_, WriteQueue>,
) -> Result<usize, String> {
    apply_search_candidate(&request, &config, &queue).await
}

async fn apply_search_candidate(
    request: &ApplyCandidateRequest,
    config: &ConfigState,
    queue: &WriteQueue,
) -> Result<usize, String> {
    let album_path = PathBuf::from(&request.album_path);
    if !album_path.is_dir() {
        return Err(format!(
            "Album directory does not exist: {}",
            request.album_path
        ));
    }

    let local_files = collect_audio_files(&album_path);
    let candidate_track_count = request.candidate.tracks.len();

    if local_files.len() != candidate_track_count {
        return Err(format!(
            "Track count mismatch: album has {} audio files but candidate has {} tracks",
            local_files.len(),
            candidate_track_count,
        ));
    }

    if let Some(index) = request
        .selected_track_indices
        .iter()
        .find(|index| **index >= local_files.len())
    {
        return Err(format!(
            "Selected track index {index} is out of range for {} audio files",
            local_files.len(),
        ));
    }

    let candidate =
        convert_candidate_chinese(&request.candidate, config.raw().chinese_script.as_deref());

    apply_selected_candidate_tags(
        &album_path,
        &candidate,
        queue,
        &request.selected_track_indices,
    )
    .await
    .map_err(|e| format!("Failed to apply candidate tags: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::config::EnvMap;
    use crate::state::providers::ProviderState;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    const MB_RESULT: &str = r#"{"id":"1","title":"OK Computer","artist-credit":[{"name":"Radiohead","artist":{"id":"art-1"}}],"date":"1997-05-21","country":"GB","media":[{"format":"CD"}],"barcode":"724384467020","label-info":[{"catalog-number":"CDP 7243 8 44670 2 0"}]}"#;
    const DG_RESULT: &str = r#"{"id":123,"title":"Radiohead - OK Computer","type":"release","year":1997,"format":["CD"],"country":"Europe","barcode":["724384467020"],"catno":"CDP 7243 8 44670 2 0","artist":"Radiohead"}"#;

    fn mock_server() -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (send, recv) = std::sync::mpsc::channel();
        thread::spawn(move || {
            listener.set_nonblocking(false).unwrap();
            for _ in 0..20 {
                let (mut stream, _) = match listener.accept() {
                    Ok(conn) => conn,
                    Err(_) => break,
                };
                let mut buf = [0; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    continue;
                }
                let request = String::from_utf8_lossy(&buf[..n]);
                let _ = send.send(request.to_string());
                let (body_str, _is_mb) =
                    if request.contains("/ws/2/artist?") || request.contains("/ws/2/artist/?") {
                        (
                            r#"{"artists":[{"id":"art-1","name":"Radiohead"}]}"#.to_string(),
                            true,
                        )
                    } else if request.contains("/ws/2/release?") || request.contains("/release?") {
                        (
                            format!("{{\"releases\":[{}],\"count\":341}}", MB_RESULT),
                            true,
                        )
                    } else if request.contains("/database/search") {
                        (
                            format!(
                                "{{\"results\":[{}],\"pagination\":{{\"items\":1,\"pages\":1}}}}",
                                DG_RESULT
                            ),
                            false,
                        )
                    } else {
                        ("{}".to_string(), false)
                    };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body_str}",
                    body_str.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (base, recv)
    }

    fn providers_at(base: &str) -> ProviderState {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        ProviderState::at(http, &format!("{base}/ws/2"), base)
    }

    /// Call search_releases_inner with the given args, using a mock server.
    async fn inner_search(
        provider: &str,
        artist: Option<&str>,
        album: Option<&str>,
        year: Option<&str>,
        country: Option<&str>,
        format: Option<&str>,
        catalog_number: Option<&str>,
        barcode: Option<&str>,
        page: u32,
        page_size: u32,
    ) -> (SearchReleasesResponse, String) {
        let (base, rx) = mock_server();
        let providers = providers_at(&base);
        let result = search_releases_inner(
            provider,
            artist.map(|s| s.to_string()),
            album.map(|s| s.to_string()),
            year.map(|s| s.to_string()),
            country.map(|s| s.to_string()),
            format.map(|s| s.to_string()),
            catalog_number.map(|s| s.to_string()),
            barcode.map(|s| s.to_string()),
            page,
            page_size,
            &providers,
            None,
        )
        .await
        .unwrap();
        let req = loop {
            let req = rx.recv().unwrap();
            if req.contains("/release?") || req.contains("/database/search") {
                break req;
            }
        };
        (result, req)
    }

    // ── Validation (no HTTP) ───────────────────────────────────────

    async fn run_validation(
        provider: &str,
        artist: Option<&str>,
        album: Option<&str>,
    ) -> Result<SearchReleasesResponse, String> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(1))
            .build()
            .unwrap();
        let providers = ProviderState::at(http, "http://localhost:1", "http://localhost:2");
        search_releases_inner(
            provider,
            artist.map(|s| s.to_string()),
            album.map(|s| s.to_string()),
            None,
            None,
            None,
            None,
            None,
            1,
            10,
            &providers,
            None,
        )
        .await
    }

    #[tokio::test]
    async fn validation_rejects_both_empty() {
        let err = run_validation("musicbrainz", None, None).await.unwrap_err();
        assert!(err.contains("Artist or album is required"), "{err}");
        let err = run_validation("discogs", None, None).await.unwrap_err();
        assert!(err.contains("Artist or album is required"), "{err}");
    }

    #[tokio::test]
    async fn validation_rejects_whitespace_only() {
        let err = run_validation("musicbrainz", Some("   "), None)
            .await
            .unwrap_err();
        assert!(err.contains("Artist or album is required"), "{err}");
        let err = run_validation("discogs", None, Some("")).await.unwrap_err();
        assert!(err.contains("Artist or album is required"), "{err}");
    }

    #[tokio::test]
    async fn validation_accepts_artist_only() {
        // Dummy endpoints will fail, but validation should pass.
        let err = run_validation("musicbrainz", Some("Radiohead"), None)
            .await
            .unwrap_err();
        assert!(
            !err.contains("Artist or album is required"),
            "validation should pass: {err}"
        );
        let err = run_validation("discogs", Some("Nirvana"), None)
            .await
            .unwrap_err();
        assert!(
            !err.contains("Artist or album is required"),
            "validation should pass: {err}"
        );
    }

    #[tokio::test]
    async fn validation_accepts_album_only() {
        let err = run_validation("musicbrainz", None, Some("OK Computer"))
            .await
            .unwrap_err();
        assert!(
            !err.contains("Artist or album is required"),
            "validation should pass: {err}"
        );
        let err = run_validation("discogs", None, Some("Nevermind"))
            .await
            .unwrap_err();
        assert!(
            !err.contains("Artist or album is required"),
            "validation should pass: {err}"
        );
    }

    // ── MusicBrainz provider-level searches ────────────────────────

    #[tokio::test]
    async fn musicbrainz_search_artist_only() {
        let (res, req) = inner_search(
            "musicbrainz",
            Some("Radiohead"),
            None,
            None,
            None,
            None,
            None,
            None,
            1,
            10,
        )
        .await;
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "OK Computer");
        assert_eq!(res.total, Some(341));
        assert!(res.has_next);
        assert!(req.contains("query="));
        assert!(req.contains("arid%3Aart-1"), "{req}");
    }

    #[tokio::test]
    async fn musicbrainz_search_album_only() {
        let (res, req) = inner_search(
            "musicbrainz",
            None,
            Some("OK Computer"),
            None,
            None,
            None,
            None,
            None,
            1,
            10,
        )
        .await;
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "OK Computer");
        assert!(req.contains("query="));
    }

    #[tokio::test]
    async fn musicbrainz_search_artist_and_album() {
        let (res, _) = inner_search(
            "musicbrainz",
            Some("Radiohead"),
            Some("OK Computer"),
            None,
            None,
            None,
            None,
            None,
            1,
            10,
        )
        .await;
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "OK Computer");
    }

    #[tokio::test]
    async fn musicbrainz_search_with_optional_params() {
        let (res, req) = inner_search(
            "musicbrainz",
            Some("Radiohead"),
            Some("OK Computer"),
            Some("1997"),
            Some("GB"),
            Some("CD"),
            Some("CDP-1"),
            Some("12345"),
            2,
            5,
        )
        .await;
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.page, 2);
        assert_eq!(res.page_size, 5);
        assert!(req.contains("arid%3Aart-1"), "{req}");
        assert!(req.contains("date"));
        assert!(req.contains("country"));
        assert!(req.contains("format"));
        assert!(req.contains("catno"));
        assert!(req.contains("barcode"));
        assert!(req.contains("offset=5"), "{req}");
    }

    #[tokio::test]
    async fn musicbrainz_search_falls_back_to_artist_name_when_identity_is_unresolved() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (send, receive) = std::sync::mpsc::channel();
        thread::spawn(move || {
            for body in [
                r#"{"artists":[]}"#.to_string(),
                format!("{{\"releases\":[{}],\"count\":1}}", MB_RESULT),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 4096];
                let count = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..count]).into_owned();
                send.send(request).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let providers = providers_at(&base);

        search_releases_inner(
            "musicbrainz",
            Some("Unknown Artist".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            1,
            10,
            &providers,
            None,
        )
        .await
        .unwrap();

        assert!(receive.recv().unwrap().contains("/artist/?"));
        let release_request = receive.recv().unwrap();
        assert!(
            release_request.contains("artist%3A%22Unknown+Artist%22"),
            "{release_request}"
        );
        assert!(!release_request.contains("arid%3A"), "{release_request}");
    }

    #[tokio::test]
    async fn musicbrainz_search_surfaces_artist_identity_http_failures() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            let body = "{}";
            write!(
                stream,
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let providers = providers_at(&base);

        let error = search_releases_inner(
            "musicbrainz",
            Some("张学友".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            1,
            100,
            &providers,
            None,
        )
        .await
        .unwrap_err();

        assert!(error.contains("MusicBrainz artist HTTP error"), "{error}");
    }

    #[tokio::test]
    async fn resolve_musicbrainz_rate_limit_reports_status_not_not_found() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 4096];
            let _ = stream.read(&mut buf).unwrap();
            let body = "{}";
            let response = format!(
                "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 60\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        });

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let providers = ProviderState::at(http, &format!("{base}/ws/2"), &base);
        let config = ConfigState::init_with_env(
            std::env::temp_dir(),
            std::sync::Arc::new(crate::state::config::EnvMap::new()),
        );
        let request = ResolveReleaseRequest {
            provider: "musicbrainz".to_string(),
            release_id: "8ffc5477-b08a-4f4d-8e0b-304dee3cd59d".to_string(),
            kind: None,
        };

        let err = resolve_release_inner(&request, &providers, &config)
            .await
            .unwrap_err();
        assert!(err.contains("HTTP 503"), "{err}");
        assert!(!err.contains("not found"), "{err}");
    }

    // ── Discogs provider-level searches ────────────────────────────

    #[tokio::test]
    async fn discogs_search_artist_only() {
        let (res, req) = inner_search(
            "discogs",
            Some("Radiohead"),
            None,
            None,
            None,
            None,
            None,
            None,
            1,
            10,
        )
        .await;
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "OK Computer");
        assert!(req.contains("/database/search"));
    }

    #[tokio::test]
    async fn discogs_search_album_only() {
        let (res, _) = inner_search(
            "discogs",
            None,
            Some("OK Computer"),
            None,
            None,
            None,
            None,
            None,
            1,
            10,
        )
        .await;
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "OK Computer");
    }

    #[tokio::test]
    async fn discogs_search_artist_and_album() {
        let (res, _) = inner_search(
            "discogs",
            Some("Radiohead"),
            Some("OK Computer"),
            None,
            None,
            None,
            None,
            None,
            1,
            10,
        )
        .await;
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].title, "OK Computer");
    }

    #[tokio::test]
    async fn discogs_search_with_optional_params() {
        let (res, req) = inner_search(
            "discogs",
            Some("Radiohead"),
            Some("OK Computer"),
            Some("1997"),
            Some("EU"),
            Some("CD"),
            Some("CAT-1"),
            Some("12345"),
            1,
            20,
        )
        .await;
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.page_size, 20);
        assert!(req.contains("year"));
        assert!(req.contains("catno"));
    }

    // ── pagination ─────────────────────────────────────────────────

    #[tokio::test]
    async fn pagination_respects_page_and_page_size() {
        let (base, requests) = mock_server();
        let providers = providers_at(&base);
        let r = search_releases_inner(
            "musicbrainz",
            Some("Radiohead".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            3,
            25,
            &providers,
            None,
        )
        .await
        .unwrap();
        assert_eq!(r.page, 3);
        assert_eq!(r.page_size, 25);
        assert!(requests.recv().unwrap().contains("/artist/?"));
        let release_request = requests.recv().unwrap();
        assert!(release_request.contains("offset=50"), "{release_request}");
    }

    #[test]
    fn page_size_allows_musicbrainz_maximum() {
        assert_eq!(normalise_page_size(Some(100)), 100);
        assert_eq!(normalise_page_size(Some(101)), 100);
        assert_eq!(normalise_page_size(None), 10);
    }

    /// Validate preview match rejects a non-existent album path without panicking.
    #[tokio::test]
    async fn preview_match_rejects_nonexistent_path() {
        let request = PreviewMatchRequest {
            album_path: "/nonexistent/album-path".into(),
            provider: "musicbrainz".into(),
            release: ProviderAlbum {
                id: "1".into(),
                title: "Test Album".into(),
                artist: Some("Artist".into()),
                artists: vec!["Artist".into()],
                artist_id: None,
                year: None,
                genre: None,
                tracks: vec![crate::state::providers::ProviderTrack {
                    title: Some("Track 1".into()),
                    match_titles: vec![],
                    artist: Some("Artist".into()),
                    artists: vec!["Artist".into()],
                    track_number: Some(1),
                    track_total: None,
                    disc_number: None,
                    recording_id: None,
                    length: None,
                }],
            },
        };
        let result = album_preview_release_match(request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn preview_match_exposes_remote_track_totals_in_mapping_rows() {
        // The confirm dialog initializes its track-total input from the matched
        // remote track's total, so the preview row must carry it.
        let root = temp_root();
        let album = root.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        fs::copy(corpus_flac(), album.join("01.flac")).unwrap();
        fs::copy(corpus_flac(), album.join("02.flac")).unwrap();

        let request = PreviewMatchRequest {
            album_path: album.to_string_lossy().into_owned(),
            provider: "musicbrainz".into(),
            release: ProviderAlbum {
                id: "1".into(),
                title: "Test Album".into(),
                artist: Some("Artist".into()),
                artists: vec!["Artist".into()],
                artist_id: None,
                year: Some("2008".into()),
                genre: None,
                tracks: vec![
                    crate::state::providers::ProviderTrack {
                        title: Some("Track 1".into()),
                        match_titles: vec![],
                        artist: Some("Artist".into()),
                        artists: vec!["Artist".into()],
                        track_number: Some(1),
                        track_total: Some(2),
                        disc_number: Some(1),
                        recording_id: None,
                        length: None,
                    },
                    crate::state::providers::ProviderTrack {
                        title: Some("Track 2".into()),
                        match_titles: vec![],
                        artist: Some("Artist".into()),
                        artists: vec!["Artist".into()],
                        track_number: Some(2),
                        track_total: Some(2),
                        disc_number: Some(1),
                        recording_id: None,
                        length: None,
                    },
                ],
            },
        };

        let result = album_preview_release_match(request).await.unwrap();

        assert_eq!(result.candidates.len(), 2);
        assert_eq!(result.candidates[0].remote_track_number, Some(1));
        assert_eq!(result.candidates[0].remote_track_total, Some(2));
        assert_eq!(result.candidates[1].remote_track_total, Some(2));
        fs::remove_dir_all(root).unwrap();
    }

    // ── apply candidate ─────────────────────────────────────────────

    fn temp_root() -> PathBuf {
        static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "soundrobe-album-search-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn corpus_flac() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus/minimal.flac")
    }

    fn config_with_chinese_script(target: Option<&str>) -> ConfigState {
        let home = temp_root();
        std::fs::create_dir_all(home.join(".soundrobe")).unwrap();
        let text = target
            .map(|target| format!("chinese_script: {target}\n"))
            .unwrap_or_default();
        std::fs::write(crate::state::config::config_file_path(&home), text).unwrap();
        ConfigState::init_with_env(home, Arc::new(EnvMap::new()))
    }

    /// Renderer-shaped apply payload (snake_case album fields round-tripped
    /// from the preview, snake_case per-track fields from the manual match).
    fn renderer_apply_payload(
        album_path: &std::path::Path,
        candidate: serde_json::Value,
        selected_track_indices: &[usize],
    ) -> ApplyCandidateRequest {
        serde_json::from_value(serde_json::json!({
            "albumPath": album_path.to_string_lossy(),
            "candidate": candidate,
            "selectedTrackIndices": selected_track_indices,
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn apply_search_candidate_writes_manual_disc_and_track_numbers() {
        // Regression: the manual match emits snake_case track keys that match
        // the native TrackCandidate contract; disc/track numbers must land on
        // disk. camelCase keys were previously dropped silently by serde.
        let root = temp_root();
        let album = root.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        fs::copy(corpus_flac(), album.join("01.flac")).unwrap();

        let request = renderer_apply_payload(&album, serde_json::json!({
            "artist": "Artist",
            "artists": ["Artist"],
            "album": "Canonical Album",
            "album_artist": "Artist",
            "album_artists": ["Artist"],
            "year": "2008",
            "source": "musicbrainz",
            "tracks": [{
                "title": "背叛",
                "artist": "Artist",
                "artists": ["Artist"],
                "track_number": 22,
                "disc_number": 1
            }]
        }), &[0]);
        let config = config_with_chinese_script(None);

        let written = apply_search_candidate(&request, &config, &WriteQueue::default())
            .await
            .unwrap();

        assert_eq!(written, 1);
        let read = crate::commands::tracks::read_track_metadata(&album.join("01.flac")).unwrap();
        assert_eq!(read.title.as_deref(), Some("背叛"));
        assert_eq!(read.track_number, Some(22));
        assert_eq!(read.disc_number, Some(1));
        assert_eq!(read.album.as_deref(), Some("Canonical Album"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn apply_search_candidate_respects_chinese_script_conversion() {
        // The manual-search apply must honor chinese_script like auto-tag does.
        let root = temp_root();
        let album = root.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        fs::copy(corpus_flac(), album.join("01.flac")).unwrap();

        let request = renderer_apply_payload(&album, serde_json::json!({
            "artist": "楊宗緯",
            "artists": ["楊宗緯"],
            "album": "星空傳奇演唱會",
            "album_artist": "楊宗緯",
            "album_artists": ["楊宗緯"],
            "year": "2008",
            "source": "musicbrainz",
            "tracks": [{
                "title": "背叛",
                "artist": "楊宗緯",
                "artists": ["楊宗緯"],
                "track_number": 1,
                "disc_number": 1
            }]
        }), &[0]);
        // chinese_script=simplified is the config this user has set.
        let config = config_with_chinese_script(Some("simplified"));

        let written = apply_search_candidate(&request, &config, &WriteQueue::default())
            .await
            .unwrap();

        assert_eq!(written, 1);
        let read = crate::commands::tracks::read_track_metadata(&album.join("01.flac")).unwrap();
        assert_eq!(read.artist.as_deref(), Some("杨宗纬"));
        assert_eq!(read.album_artist.as_deref(), Some("杨宗纬"));
        assert_eq!(read.album.as_deref(), Some("星空传奇演唱会"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn apply_search_candidate_without_chinese_script_preserves_text() {
        // No chinese_script configured: the writer must not rewrite text.
        let root = temp_root();
        let album = root.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        fs::copy(corpus_flac(), album.join("01.flac")).unwrap();

        let request = renderer_apply_payload(&album, serde_json::json!({
            "artist": "楊宗緯",
            "artists": ["楊宗緯"],
            "album": "星空傳奇演唱會",
            "album_artist": "楊宗緯",
            "album_artists": ["楊宗緯"],
            "year": "2008",
            "source": "musicbrainz",
            "tracks": [{
                "title": "背叛",
                "artist": "楊宗緯",
                "artists": ["楊宗緯"],
                "track_number": 1,
                "disc_number": 1
            }]
        }), &[0]);
        let config = config_with_chinese_script(None);

        let written = apply_search_candidate(&request, &config, &WriteQueue::default())
            .await
            .unwrap();

        assert_eq!(written, 1);
        let read = crate::commands::tracks::read_track_metadata(&album.join("01.flac")).unwrap();
        assert_eq!(read.artist.as_deref(), Some("楊宗緯"));
        assert_eq!(read.album_artist.as_deref(), Some("楊宗緯"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn apply_search_candidate_skips_do_not_update_tracks_entirely() {
        let root = temp_root();
        let album = root.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        let matched_path = album.join("01.flac");
        let unmatched_path = album.join("02.flac");
        fs::copy(corpus_flac(), &matched_path).unwrap();
        fs::copy(corpus_flac(), &unmatched_path).unwrap();
        let unmatched_before = fs::read(&unmatched_path).unwrap();

        let request = renderer_apply_payload(
            &album,
            serde_json::json!({
                "artist": "Artist",
                "artists": ["Artist"],
                "album": "Canonical Album",
                "album_artist": "Artist",
                "album_artists": ["Artist"],
                "source": "musicbrainz",
                "tracks": [
                    {
                        "title": "Matched Title",
                        "artist": "Artist",
                        "artists": ["Artist"],
                        "track_number": 1
                    },
                    { "artists": [] }
                ]
            }),
            &[0],
        );
        let config = config_with_chinese_script(None);

        let written = apply_search_candidate(&request, &config, &WriteQueue::default())
            .await
            .unwrap();

        assert_eq!(written, 1);
        let matched = crate::commands::tracks::read_track_metadata(&matched_path).unwrap();
        assert_eq!(matched.title.as_deref(), Some("Matched Title"));
        assert_eq!(matched.album.as_deref(), Some("Canonical Album"));
        assert_eq!(fs::read(&unmatched_path).unwrap(), unmatched_before);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn apply_search_candidate_writes_selected_track_with_empty_track_fields() {
        let root = temp_root();
        let album = root.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        let selected_path = album.join("01.flac");
        let unmatched_path = album.join("02.flac");
        fs::copy(corpus_flac(), &selected_path).unwrap();
        fs::copy(corpus_flac(), &unmatched_path).unwrap();
        let unmatched_before = fs::read(&unmatched_path).unwrap();
        let request: ApplyCandidateRequest = serde_json::from_value(serde_json::json!({
            "albumPath": album.to_string_lossy(),
            "selectedTrackIndices": [0],
            "candidate": {
                "artist": "Artist",
                "artists": ["Artist"],
                "album": "Canonical Album",
                "album_artist": "Artist",
                "album_artists": ["Artist"],
                "source": "musicbrainz",
                "tracks": [{ "artists": [] }, { "artists": [] }]
            }
        }))
        .unwrap();
        let config = config_with_chinese_script(None);

        let written = apply_search_candidate(&request, &config, &WriteQueue::default())
            .await
            .unwrap();

        assert_eq!(written, 1);
        let selected = crate::commands::tracks::read_track_metadata(&selected_path).unwrap();
        assert_eq!(selected.album.as_deref(), Some("Canonical Album"));
        assert_eq!(fs::read(&unmatched_path).unwrap(), unmatched_before);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn apply_search_candidate_rejects_track_count_mismatch() {
        let root = temp_root();
        let album = root.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        fs::copy(corpus_flac(), album.join("01.flac")).unwrap();
        fs::copy(corpus_flac(), album.join("02.flac")).unwrap();

        let request = renderer_apply_payload(&album, serde_json::json!({
            "artist": "Artist",
            "artists": ["Artist"],
            "album": "Canonical Album",
            "album_artist": "Artist",
            "album_artists": ["Artist"],
            "source": "musicbrainz",
            "tracks": [{ "title": "Only One", "artists": [] }]
        }), &[0]);
        let config = config_with_chinese_script(None);

        let error = apply_search_candidate(&request, &config, &WriteQueue::default())
            .await
            .unwrap_err();
        assert!(error.contains("Track count mismatch"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }
}
