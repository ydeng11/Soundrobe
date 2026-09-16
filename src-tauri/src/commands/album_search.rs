//! Manual album search commands: search releases, resolve release detail,
//! preview local-to-remote track matching, and apply a user-edited candidate.
//!
//! These commands are used by the Search button (manual workflow) and do not
//! change the existing auto-tag pipeline.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
#[cfg(feature = "desktop")]
use tauri::State;

use crate::commands::lyrics::LyricsDocument;
use crate::commands::{
    library::collect_audio_files,
    mutations::{write_track_queued, TrackPatch},
    tracks::read_album,
};
#[cfg(any(feature = "desktop", feature = "server"))]
use crate::commands::track_matcher::match_remote_candidate_tracks;
use crate::error::ApiError;
use crate::state::{
    config::ConfigState,
    providers::{
        DiscogsClient, MusicBrainzClient, ProviderAlbum, ProviderState, ReleaseSearchSummary,
    },
};
use crate::state::write_queue::WriteQueue;

// ── Request / response types ─────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LookupSource {
    #[default]
    Beets,
    Dataset,
    Discogs,
    Folder,
    Llm,
    Musicbrainz,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TrackCandidate {
    pub title: Option<String>,
    #[serde(default)]
    pub match_titles: Vec<String>,
    pub artist: Option<String>,
    #[serde(default)]
    pub artists: Vec<String>,
    pub track_number: Option<u32>,
    pub track_total: Option<u32>,
    pub disc_number: Option<u32>,
    pub disc_total: Option<u32>,
    /// Provider media label keeps CD audio separate from DVD extras.
    #[serde(default)]
    pub media_type: Option<String>,
    #[serde(rename = "musicbrainz_trackid")]
    pub musicbrainz_track_id: Option<String>,
    pub length: Option<f64>,
    pub genre: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AlbumCandidate {
    pub artist: Option<String>,
    #[serde(default)]
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    #[serde(default)]
    pub album_artists: Vec<String>,
    pub year: Option<String>,
    pub genre: Option<String>,
    #[serde(rename = "musicbrainz_albumid")]
    pub musicbrainz_album_id: Option<String>,
    #[serde(rename = "musicbrainz_artistid")]
    pub musicbrainz_artist_id: Option<String>,
    pub discogs_artist_id: Option<String>,
    pub discogs_release_id: Option<String>,
    #[serde(default)]
    pub tracks: Vec<TrackCandidate>,
    pub distance: Option<f64>,
    pub source: LookupSource,
    pub verification: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveReleaseRequest {
    pub provider: String,
    pub release_id: String,
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyCandidateRequest {
    pub album_path: String,
    pub candidate: AlbumCandidate,
    pub selected_track_indices: Vec<usize>,
}

// ── Helpers ──────────────────────────────────────────────────────────

pub(crate) fn discogs_token(config: &ConfigState) -> Option<String> {
    config.raw().discogs_token.clone()
}

pub(crate) fn normalise_page_size(page_size: Option<u32>) -> u32 {
    page_size.unwrap_or(10).clamp(1, 100)
}

pub(crate) fn normalise_page(page: Option<u32>) -> u32 {
    page.unwrap_or(1).clamp(1, 10_000)
}

pub(crate) fn split_collaborative_artists(
    artist: &Option<String>,
    artists: &[String],
) -> Vec<String> {
    if artists.len() > 1 {
        return artists.to_vec();
    }
    let source = artist
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| artists.first().map(String::as_str))
        .unwrap_or_default();
    let split = crate::state::providers::split_artist_names(&[source.to_string()]);
    if split.is_empty() {
        artists.to_vec()
    } else {
        split
    }
}

pub fn musicbrainz_candidate(album: ProviderAlbum) -> AlbumCandidate {
    let artist = album.artist.clone();
    AlbumCandidate {
        artist: artist.clone(),
        artists: album.artists.clone(),
        album: Some(album.title),
        album_artist: artist,
        album_artists: album.artists,
        year: album.year,
        genre: album.genre,
        musicbrainz_album_id: Some(album.id),
        musicbrainz_artist_id: album.artist_id,
        tracks: album
            .tracks
            .into_iter()
            .map(|track| TrackCandidate {
                title: track.title,
                match_titles: track.match_titles,
                artist: track.artist,
                artists: track.artists,
                track_number: track.track_number,
                track_total: track.track_total,
                disc_number: track.disc_number,
                musicbrainz_track_id: track.recording_id,
                length: track.length,
                ..TrackCandidate::default()
            })
            .collect(),
        source: LookupSource::Musicbrainz,
        ..AlbumCandidate::default()
    }
}

pub fn discogs_candidate(album: ProviderAlbum) -> AlbumCandidate {
    let artist = album.artist.clone();
    AlbumCandidate {
        artist: artist.clone(),
        artists: album.artists.clone(),
        album: Some(album.title),
        album_artist: artist,
        album_artists: album.artists,
        year: album.year,
        genre: album.genre,
        discogs_artist_id: album.artist_id,
        discogs_release_id: Some(album.id),
        tracks: album
            .tracks
            .into_iter()
            .map(|track| TrackCandidate {
                title: track.title,
                match_titles: track.match_titles,
                artist: track.artist,
                artists: track.artists,
                track_number: track.track_number,
                track_total: track.track_total,
                disc_number: track.disc_number,
                length: track.length,
                ..TrackCandidate::default()
            })
            .collect(),
        source: LookupSource::Discogs,
        ..AlbumCandidate::default()
    }
}

pub fn convert_candidate_chinese(
    candidate: &AlbumCandidate,
    target: Option<&str>,
) -> AlbumCandidate {
    let Some(target) = target.filter(|target| matches!(*target, "traditional" | "simplified"))
    else {
        return candidate.clone();
    };
    let convert = |value: &Option<String>| {
        value
            .as_deref()
            .map(|value| crate::state::providers::convert_chinese_text(value, target))
    };
    let convert_many = |values: &[String]| {
        values
            .iter()
            .map(|value| crate::state::providers::convert_chinese_text(value, target))
            .collect()
    };
    let mut converted = candidate.clone();
    converted.artist = convert(&candidate.artist);
    converted.artists = convert_many(&candidate.artists);
    converted.album = convert(&candidate.album);
    converted.album_artist = convert(&candidate.album_artist);
    converted.album_artists = convert_many(&candidate.album_artists);
    converted.year = convert(&candidate.year);
    converted.genre = convert(&candidate.genre);
    converted.tracks = candidate
        .tracks
        .iter()
        .map(|track| {
            let mut track = track.clone();
            track.title = convert(&track.title);
            track.artist = convert(&track.artist);
            track.artists = convert_many(&track.artists);
            track.genre = convert(&track.genre);
            track
        })
        .collect();
    converted
}

#[derive(Clone, Copy)]
pub(crate) enum CandidateApplyScope<'a> {
    SelectedTracks(&'a HashSet<usize>),
}

impl CandidateApplyScope<'_> {
    fn includes(self, index: usize) -> bool {
        matches!(self, Self::SelectedTracks(indices) if indices.contains(&index))
    }
}

fn has_writable_track_fields(track: &TrackCandidate) -> bool {
    track.title.is_some()
        || track.artist.is_some()
        || !track.artists.is_empty()
        || track.track_number.is_some()
        || track.track_total.is_some()
        || track.disc_number.is_some()
        || track.disc_total.is_some()
        || track.musicbrainz_track_id.is_some()
}

pub(crate) async fn apply_candidate_tags_reported(
    album_path: &Path,
    candidate: &AlbumCandidate,
    queue: &WriteQueue,
    scope: CandidateApplyScope<'_>,
    lyrics_map: HashMap<PathBuf, LyricsDocument>,
    mut report_write: impl FnMut(&str),
) -> Result<usize, ApiError> {
    let fallback_artist = album_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let album_artists = if candidate.album_artists.is_empty() {
        vec![fallback_artist.to_string()]
    } else {
        candidate.album_artists.clone()
    };
    let album_artist = album_artists.join(" & ");
    let mut album_fields = serde_json::Map::new();
    insert_option(&mut album_fields, "album", &candidate.album);
    album_fields.insert("albumArtist".into(), album_artist.into());
    album_fields.insert("albumArtists".into(), serde_json::json!(album_artists));
    if let Some(year) = &candidate.year {
        album_fields.insert("year".into(), year.clone().into());
    }
    if let Some(genre) = &candidate.genre {
        album_fields.insert("genre".into(), genre.clone().into());
    }
    insert_option(
        &mut album_fields,
        "musicbrainzAlbumId",
        &candidate.musicbrainz_album_id,
    );
    insert_option(
        &mut album_fields,
        "musicbrainzArtistId",
        &candidate.musicbrainz_artist_id,
    );
    insert_option(
        &mut album_fields,
        "discogsReleaseId",
        &candidate.discogs_release_id,
    );
    insert_option(
        &mut album_fields,
        "discogsArtistId",
        &candidate.discogs_artist_id,
    );

    let mut written = 0;
    let mut failures = Vec::new();
    for (index, file_path) in collect_audio_files(album_path).into_iter().enumerate() {
        let track = candidate.tracks.get(index);
        if !scope.includes(index) {
            continue;
        }
        let mut fields = album_fields.clone();
        if let Some(track) = track {
            if has_writable_track_fields(track) {
                insert_option(&mut fields, "title", &track.title);
                insert_option(&mut fields, "artist", &track.artist);
                if !track.artists.is_empty() {
                    fields.insert("artists".into(), serde_json::json!(track.artists));
                }
                insert_number(&mut fields, "trackNumber", track.track_number);
                insert_number(&mut fields, "trackTotal", track.track_total);
                insert_number(&mut fields, "discNumber", track.disc_number);
                insert_number(&mut fields, "discTotal", track.disc_total);
                if let Some(track_id) = &track.musicbrainz_track_id {
                    fields.insert("musicbrainzTrackId".into(), track_id.clone().into());
                }
            }
        }
        if let Some(lyrics) = lyrics_map.get(Path::new(&file_path)) {
            fields.insert("lyrics".into(), serde_json::json!(lyrics));
        }
        let patch: TrackPatch = serde_json::from_value(fields.into())
            .map_err(|error| ApiError::WriteTask(error.to_string()))?;
        match write_track_queued(queue, file_path.clone().into(), patch).await {
            Ok(()) => {
                written += 1;
                report_write(&file_path);
            }
            Err(error) => {
                tracing::warn!(path = %file_path, %error, "manual search write failed");
                failures.push(format!("{file_path}: {error}"));
            }
        }
    }
    if failures.is_empty() {
        Ok(written)
    } else {
        Err(ApiError::WriteTask(format!(
            "manual search wrote {written} file(s), but {} file(s) failed: {}",
            failures.len(),
            failures.join("; ")
        )))
    }
}

pub(crate) async fn apply_selected_candidate_tags(
    album_path: &Path,
    candidate: &AlbumCandidate,
    queue: &WriteQueue,
    selected_track_indices: &[usize],
) -> Result<usize, ApiError> {
    let selected_track_indices = selected_track_indices
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    apply_candidate_tags_reported(
        album_path,
        candidate,
        queue,
        CandidateApplyScope::SelectedTracks(&selected_track_indices),
        HashMap::new(),
        |_| {},
    )
    .await
}

fn insert_option(
    fields: &mut serde_json::Map<String, serde_json::Value>,
    name: &str,
    value: &Option<String>,
) {
    fields.insert(name.to_string(), serde_json::json!(value));
}

fn insert_number(
    fields: &mut serde_json::Map<String, serde_json::Value>,
    name: &str,
    value: Option<u32>,
) {
    if let Some(value) = value {
        fields.insert(name.to_string(), value.into());
    }
}

// ── Commands ─────────────────────────────────────────────────────────

/// Inner search with pre-normalised inputs. Trims all string values and
/// omits empty ones downstream. Returns `Err` when both artist and album
/// are empty after trimming.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn search_releases_inner(
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
            if album.is_none()
                && year.is_none()
                && country.is_none()
                && format.is_none()
                && catno.is_none()
                && bc.is_none()
            {
                if let Some(artist_id) = musicbrainz_artist_id.as_deref() {
                    let (summaries, total) = client
                        .browse_release_summaries(artist_id, page, page_size)
                        .await?;
                    let offset = (page - 1) * page_size;
                    return Ok(SearchReleasesResponse {
                        results: summaries,
                        page,
                        page_size,
                        total: Some(total),
                        has_next: offset + page_size < total,
                    });
                }
            }
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
#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn album_search_releases(
    request: SearchReleasesRequest,
    providers: State<'_, ProviderState>,
    config: State<'_, ConfigState>,
) -> Result<SearchReleasesResponse, String> {
    let page = normalise_page(request.page);
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

/// Read only the provider track count; never run genre or candidate enrichment.
#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn album_release_track_count(
    request: ResolveReleaseRequest,
    providers: State<'_, ProviderState>,
    config: State<'_, ConfigState>,
) -> Result<Option<u32>, String> {
    use crate::state::providers::{ProviderRetryContext, ProviderRetryMetrics};
    let retry = ProviderRetryContext::new(
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        ProviderRetryMetrics::default(),
    );
    match request.provider.as_str() {
        "musicbrainz" => MusicBrainzClient::at(providers.http(), providers.musicbrainz_base())
            .with_retry_context(retry)
            .search_track_count(&request.release_id)
            .await,
        "discogs" => DiscogsClient::at(
            providers.http(),
            discogs_token(&config),
            providers.discogs_base(),
        )
        .with_retry_context(retry)
        .search_track_count(&request.release_id, request.kind.as_deref())
        .await,
        other => Err(format!("Unknown provider: {other}")),
    }
}

/// Resolve a single release by provider + ID, returning full `ProviderAlbum` with tracks.
#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn album_resolve_release(
    request: ResolveReleaseRequest,
    providers: State<'_, ProviderState>,
    config: State<'_, ConfigState>,
) -> Result<ProviderAlbum, String> {
    resolve_release_inner(&request, &providers, &config).await
}

pub(crate) async fn resolve_release_inner(
    request: &ResolveReleaseRequest,
    providers: &ProviderState,
    config: &ConfigState,
) -> Result<ProviderAlbum, String> {
    let album = match request.provider.as_str() {
        "musicbrainz" => {
            let client = MusicBrainzClient::at(providers.http(), providers.musicbrainz_base());
            client.release_by_id_result(&request.release_id).await
        }
        "discogs" => {
            let token = discogs_token(config);
            let client = DiscogsClient::new(providers.http(), token);
            match request.kind.as_deref() {
                Some("master") => client
                    .master_metadata(&request.release_id)
                    .await
                    .ok_or_else(|| "Discogs master metadata unavailable".to_string()),
                _ => client
                    .release_metadata_result(&request.release_id)
                    .await,
            }
        }
        other => Err(format!("Unknown provider: {other}")),
    }?;

    Ok(album)
}

/// Preview local-to-remote track matching for a selected release.
/// The release was already resolved on the renderer side, so this command
/// receives the full `ProviderAlbum` and runs matching against local tracks.
#[cfg_attr(feature = "desktop", tauri::command)]
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
            media_type: None,
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

    // The shared matcher is owned by auto-tag. Manual search has an equivalent
    // transport type, so convert at the boundary instead of maintaining a
    // second matching algorithm.
    let matcher_track = |track: &TrackCandidate| crate::commands::auto_tag::TrackCandidate {
        title: track.title.clone(),
        match_titles: track.match_titles.clone(),
        artist: track.artist.clone(),
        artists: track.artists.clone(),
        track_number: track.track_number,
        track_total: track.track_total,
        disc_number: track.disc_number,
        disc_total: track.disc_total,
        media_type: track.media_type.clone(),
        musicbrainz_track_id: track.musicbrainz_track_id.clone(),
        length: track.length,
        genre: track.genre.clone(),
        filename: track.filename.clone(),
    };
    let matcher_local_tracks = local_tracks.iter().map(matcher_track).collect::<Vec<_>>();
    let matcher_remote_tracks = album_candidate.tracks.iter().map(matcher_track).collect::<Vec<_>>();

    // Run track matching
    let matched = match_remote_candidate_tracks(
        &matcher_local_tracks,
        &filenames,
        &matcher_remote_tracks,
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
#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn album_search_apply_candidate(
    request: ApplyCandidateRequest,
    config: State<'_, ConfigState>,
    queue: State<'_, WriteQueue>,
) -> Result<usize, String> {
    apply_search_candidate(&request, &config, &queue).await
}

pub(crate) async fn apply_search_candidate(
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

    let mut candidate = request.candidate.clone();
    for index in &request.selected_track_indices {
        if let Some(track) = candidate.tracks.get_mut(*index) {
            track.artists = split_collaborative_artists(&track.artist, &track.artists);
        }
    }
    let candidate = convert_candidate_chinese(&candidate, config.raw().chinese_script.as_deref());

    apply_selected_candidate_tags(
        &album_path,
        &candidate,
        queue,
        &request.selected_track_indices,
    )
    .await
    .map_err(|e| format!("Failed to apply candidate tags: {e}"))
}

#[cfg(all(test, feature = "desktop"))]
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

    const MB_RESULT: &str = r#"{"id":"1","title":"OK Computer","artist-credit":[{"name":"Radiohead","artist":{"id":"art-1"}}],"date":"1997-05-21","country":"GB","track-count":10,"media":[{"format":"CD","track-count":10}],"barcode":"724384467020","label-info":[{"catalog-number":"CDP 7243 8 44670 2 0"}]}"#;
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
                    } else if request.contains("/release?artist=art-1") {
                        (
                            format!(
                                "{{\"releases\":[{}],\"release-count\":341,\"release-offset\":0}}",
                                MB_RESULT
                            ),
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
    #[allow(clippy::too_many_arguments)]
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
        assert_eq!(res.results[0].track_count, Some(10));
        assert_eq!(res.total, Some(341));
        assert!(res.has_next);
        assert!(req.contains("artist=art-1"), "{req}");
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
            for _ in 0..2 {
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
            }
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
                    media_type: None,
                    recording_id: None,
                    length: None,
                }],
                ..ProviderAlbum::default()
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
                        media_type: None,
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
                        media_type: None,
                        recording_id: None,
                        length: None,
                    },
                ],
                ..ProviderAlbum::default()
            },
        };

        let result = album_preview_release_match(request).await.unwrap();

        assert_eq!(result.candidates.len(), 2);
        assert_eq!(result.candidates[0].remote_track_number, Some(1));
        assert_eq!(result.candidates[0].remote_track_total, Some(2));
        assert_eq!(result.candidates[1].remote_track_total, Some(2));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn manual_release_reuses_auto_tag_genre_fill_when_provider_genre_is_missing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (send, receive) = std::sync::mpsc::channel();
        thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 8192];
                let read = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..read]).to_string();
                let body = if request.starts_with("GET /ws/2/release/release-id?") {
                    serde_json::json!({
                        "id": "release-id",
                        "title": "Album",
                        "artist-credit": [{
                            "name": "Artist",
                            "artist": {"id": "artist-id", "name": "Artist"}
                        }],
                        "date": "2004",
                        "media": [{
                            "position": 1,
                            "tracks": [{
                                "number": "1",
                                "position": 1,
                                "title": "Track",
                                "recording": {"id": "recording-id", "title": "Track"}
                            }]
                        }]
                    })
                    .to_string()
                } else {
                    serde_json::json!({
                        "choices": [{
                            "finish_reason": "stop",
                            "message": {"content": "{\"genre\":\"Rock, Indie Rock\",\"confidence\":0.9}"}
                        }]
                    })
                    .to_string()
                };
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
        let config = config_with_auto_tag(&base);
        let request = ResolveReleaseRequest {
            provider: "musicbrainz".into(),
            release_id: "release-id".into(),
            kind: None,
        };

        let release = resolve_release_inner(&request, &providers, &config)
            .await
            .unwrap();

        assert_eq!(release.genre.as_deref(), Some("Rock, Indie Rock"));
        let requests = [receive.recv().unwrap(), receive.recv().unwrap()];
        assert!(
            requests
                .iter()
                .any(|request| request.starts_with("GET /ws/2/release/release-id?")),
            "{requests:?}"
        );
        assert!(
            requests.iter().any(|request| {
                request.starts_with("POST /chat/completions ")
                    && request.contains("GenreFillResponse")
            }),
            "{requests:?}"
        );
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

    fn config_with_auto_tag(base: &str) -> ConfigState {
        let home = temp_root();
        std::fs::create_dir_all(home.join(".soundrobe")).unwrap();
        let text = format!("llm_api_key: test-key\nllm_model: test-model\nllm_base_url: {base}\n");
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
    async fn apply_search_candidate_normalizes_edited_artists_and_writes_genre_only_to_selected_flac(
    ) {
        let root = temp_root();
        let album = root.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        let selected_path = album.join("01.flac");
        let unselected_path = album.join("02.flac");
        fs::copy(corpus_flac(), &selected_path).unwrap();
        fs::copy(corpus_flac(), &unselected_path).unwrap();
        let unselected_before = fs::read(&unselected_path).unwrap();

        let request = renderer_apply_payload(
            &album,
            serde_json::json!({
                "artist": "Album Artist",
                "artists": ["Album Artist"],
                "album": "Canonical Album",
                "album_artist": "Album Artist",
                "album_artists": ["Album Artist"],
                "genre": "Rock, Indie Rock",
                "source": "musicbrainz",
                "tracks": [
                    {
                        "title": "Collaborative Track",
                        "artist": "Artist A feat. Artist B",
                        "artists": ["Artist A feat. Artist B"],
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
        let selected = crate::commands::tracks::read_track_metadata(&selected_path).unwrap();
        assert_eq!(selected.artist.as_deref(), Some("Artist A feat. Artist B"));
        assert_eq!(selected.artists, vec!["Artist A", "Artist B"]);
        assert_eq!(selected.genre.as_deref(), Some("Rock, Indie Rock"));
        assert_eq!(fs::read(&unselected_path).unwrap(), unselected_before);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn apply_search_candidate_without_genre_preserves_existing_genre() {
        let root = temp_root();
        let album = root.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        let track_path = album.join("01.flac");
        fs::copy(corpus_flac(), &track_path).unwrap();
        let config = config_with_chinese_script(None);
        let queue = WriteQueue::default();

        let initial = renderer_apply_payload(
            &album,
            serde_json::json!({
                "artist": "Artist",
                "artists": ["Artist"],
                "album": "Album",
                "album_artist": "Artist",
                "album_artists": ["Artist"],
                "genre": "Existing Genre",
                "source": "discogs",
                "tracks": [{ "title": "Before", "artists": [] }]
            }),
            &[0],
        );
        apply_search_candidate(&initial, &config, &queue)
            .await
            .unwrap();

        let missing_genre = renderer_apply_payload(
            &album,
            serde_json::json!({
                "artist": "Artist",
                "artists": ["Artist"],
                "album": "Album",
                "album_artist": "Artist",
                "album_artists": ["Artist"],
                "source": "musicbrainz",
                "tracks": [{ "title": "After", "artists": [] }]
            }),
            &[0],
        );
        apply_search_candidate(&missing_genre, &config, &queue)
            .await
            .unwrap();

        let read = crate::commands::tracks::read_track_metadata(&track_path).unwrap();
        assert_eq!(read.title.as_deref(), Some("After"));
        assert_eq!(read.genre.as_deref(), Some("Existing Genre"));
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
