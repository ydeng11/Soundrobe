//! Auto-tag candidate contracts and, once complete, task orchestration.

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    commands::{
        covers::{download_album_artwork_at, download_artist_artwork_at},
        library::collect_audio_files,
        lyrics::{fetch_album_lyrics, LyricsDocument, DEFAULT_BASE_URL},
        mutations::{write_track_queued, TrackPatch},
        tracks::read_album,
    },
    error::ApiError,
    infra::{
        aliases::save_alias,
        openrouter::{ChatMessage, OpenRouterClient, OpenRouterError},
    },
    state::{
        config::AutoTagConfig,
        providers::{
            album_names_match, convert_chinese_text, ArtistIdentity, DiscogsClient,
            MusicBrainzClient, ProviderAlbum, ProviderReleaseSummary, ProviderRetryContext,
            ProviderRetryMetrics, ProviderState, RemoteArtworkClient,
        },
        sqlite::CacheState,
        tasks::{TaskRegistry, TaskStatus},
        write_queue::WriteQueue,
    },
};

use super::track_matcher::{match_remote_candidate_tracks, MatchEvidence};

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
    #[serde(rename = "musicbrainz_trackid")]
    pub musicbrainz_track_id: Option<String>,
    pub length: Option<f64>,
    pub genre: Option<String>,
    /// File stem (without extension) for LLM title inference.
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
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub formats: Vec<String>,
    #[serde(default)]
    pub catalog_number: Option<String>,
    #[serde(default)]
    pub barcode: Option<String>,
    #[serde(default)]
    pub linked_discogs_release_id: Option<String>,
    #[serde(rename = "musicbrainz_albumid")]
    pub musicbrainz_album_id: Option<String>,
    #[serde(rename = "musicbrainz_artistid")]
    pub musicbrainz_artist_id: Option<String>,
    pub discogs_artist_id: Option<String>,
    pub discogs_release_id: Option<String>,
    #[serde(default)]
    pub tracks: Vec<TrackCandidate>,
    pub distance: Option<f64>,
    #[serde(default)]
    pub confidence: Option<f64>,
    pub source: LookupSource,
    pub verification: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LookupRequest {
    pub path: String,
    pub artist_hint: Option<String>,
    #[serde(default)]
    pub artist_aliases: Vec<String>,
    #[serde(default)]
    pub tagged_artist_hint: Option<String>,
    #[serde(default)]
    pub folder_artist_hint: Option<String>,
    pub album_hint: Option<String>,
    #[serde(default)]
    pub tagged_album_hint: Option<String>,
    #[serde(default)]
    pub folder_album_hint: Option<String>,
    pub year_hint: Option<String>,
    #[serde(default)]
    pub country_hint: Option<String>,
    pub musicbrainz_album_id: Option<String>,
    pub musicbrainz_artist_id: Option<String>,
    pub discogs_release_id: Option<String>,
    pub discogs_artist_id: Option<String>,
    pub selected_disc_number: Option<u32>,
    pub tracks: Vec<TrackCandidate>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderAlbumEvidence {
    pub search_album: Option<String>,
    pub tagged_album: Option<String>,
    pub folder_album: Option<String>,
    pub year: Option<String>,
    pub country: Option<String>,
}

pub fn build_lookup_request(album_path: &Path) -> Result<LookupRequest, ApiError> {
    let detail = read_album(album_path)?;
    let supplied_folder = album_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let cd_subfolder = Regex::new(
        r"(?i)^(?:cd|disc|disk|ディスク)\s*\d+\s*$|^.+(?:\s|\(|\[)(?:cd|disc|disk)\s*\d+\s*$",
    )
    .expect("valid CD subfolder regex")
    .is_match(supplied_folder);
    // Extract disc number from folder suffix like "CD1", "Disc 2", "(CD1)", "挑信 CD1".
    let selected_disc_number = Regex::new(r"(?i)(?:cd|disc|disk)\s*(\d+)")
        .expect("valid disc number regex")
        .captures(supplied_folder)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u32>().ok());
    let identity_album_path = if cd_subfolder {
        album_path.parent().unwrap_or(album_path)
    } else {
        album_path
    };
    let folder_name = identity_album_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let parent_name = identity_album_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let folder_artist = clean_folder_name(parent_name);
    let folder_album = clean_folder_name(folder_name);
    let tagged_artist = detail
        .tracks
        .iter()
        .find_map(|track| track.artist.clone().or_else(|| track.album_artist.clone()));
    let tagged_album = detail.tracks.iter().find_map(|track| track.album.clone());
    let album_evidence = parse_folder_album_evidence(folder_name, tagged_album.as_deref());
    let tagged_year = detail.tracks.iter().find_map(|track| track.year.clone());
    let musicbrainz_album_id = detail
        .tracks
        .iter()
        .find_map(|track| track.musicbrainz_album_id.clone());
    let musicbrainz_artist_id = detail
        .tracks
        .iter()
        .find_map(|track| track.musicbrainz_artist_id.clone());
    let discogs_release_id = detail
        .tracks
        .iter()
        .find_map(|track| track.discogs_release_id.clone());
    let discogs_artist_id = detail
        .tracks
        .iter()
        .find_map(|track| track.discogs_artist_id.clone());
    let folder_artist_hint = non_empty(folder_artist);
    let artist_hint = if folder_artist_hint
        .as_deref()
        .is_some_and(is_compilation_folder)
    {
        Some("Various Artists".to_string())
    } else {
        tagged_artist.clone().or_else(|| folder_artist_hint.clone())
    };
    let total = u32::try_from(detail.tracks.len()).ok();
    let tracks = detail
        .tracks
        .into_iter()
        .enumerate()
        .map(|(index, track)| {
            let filename_stem = Path::new(&track.path)
                .file_stem()
                .and_then(|stem| stem.to_str());
            let filename_number = filename_stem.and_then(filename_track_number);
            TrackCandidate {
                title: track
                    .title
                    .or_else(|| filename_stem.and_then(filename_track_title)),
                filename: filename_stem.map(|s| s.to_owned()),
                artist: track.artist.clone(),
                artists: if track.artists.is_empty() {
                    track.artist.into_iter().collect()
                } else {
                    track.artists
                },
                track_number: filename_number
                    .or(track.track_number)
                    .or_else(|| u32::try_from(index + 1).ok()),
                track_total: total,
                disc_number: track.disc_number,
                disc_total: track.disc_total,
                musicbrainz_track_id: track.musicbrainz_track_id,
                length: Some(track.duration),
                genre: track.genre,
                ..TrackCandidate::default()
            }
        })
        .collect();

    Ok(LookupRequest {
        path: album_path.to_string_lossy().into_owned(),
        artist_hint,
        artist_aliases: Vec::new(),
        tagged_artist_hint: tagged_artist,
        folder_artist_hint,
        album_hint: album_evidence.search_album,
        tagged_album_hint: album_evidence.tagged_album,
        folder_album_hint: album_evidence.folder_album.or_else(|| non_empty(folder_album)),
        year_hint: album_evidence.year.or(tagged_year),
        country_hint: album_evidence.country,
        musicbrainz_album_id,
        musicbrainz_artist_id,
        discogs_release_id,
        discogs_artist_id,
        selected_disc_number,
        tracks,
    })
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn extract_folder_year(name: &str) -> Option<String> {
    let year_prefix =
        Regex::new(r"^\s*((?:19|20)\d{2})(?:\s*[-.]|[^\d]|$)").expect("valid folder year regex");
    let extract_prefix = |value: &str| {
        year_prefix
            .captures(value)
            .and_then(|captures| captures.get(1))
            .map(|year| year.as_str().to_string())
    };
    if let Some(year) = extract_prefix(name) {
        return Some(year);
    }

    for (open, close) in [('《', '》'), ('〈', '〉'), ('「', '」'), ('『', '』')] {
        for remainder in name.split(open).skip(1) {
            let Some((quoted_title, _)) = remainder.split_once(close) else {
                continue;
            };
            if let Some(year) = extract_prefix(quoted_title) {
                return Some(year);
            }
        }
    }
    None
}

fn clean_folder_name(name: &str) -> String {
    let mut cleaned = name.to_string();
    cleaned = Regex::new(r"^\d{4}\s*[.-]\s*")
        .expect("valid folder year prefix regex")
        .replace(&cleaned, "")
        .to_string();
    cleaned = cleaned.replace(['《', '》', '「', '」', '【', '】', '[', ']'], "");
    cleaned = Regex::new(
        r"(?i)\s*(?:香港首版|台湾首版|引进版|日本版|欧版|美版|内地版|中国大陆版|大陆版|德国版|澳洲版|新加坡版|马来西亚版|韩版)\s*",
    )
    .expect("valid edition regex")
    .replace_all(&cleaned, " ")
    .to_string();
    cleaned = Regex::new(r"(?i)\s*(?:flac|mp3|wav|aac|ogg|m4a|wma|ape)(?:\s*分轨)?\s*$")
        .expect("valid format suffix regex")
        .replace(&cleaned, "")
        .trim()
        .to_string();
    cleaned
}

pub fn parse_folder_album_evidence(
    folder_name: &str,
    tagged_album: Option<&str>,
) -> FolderAlbumEvidence {
    let year = extract_folder_year(folder_name);
    let country = Regex::new(
        r"(?i)(?:japanese|japan|日本)\s*(?:edition|version|pressing|版)?",
    )
    .expect("valid Japanese edition regex")
    .is_match(folder_name)
    .then(|| "JP".to_string())
    .or_else(|| {
        Regex::new(r"(?i)(?:european|europe|欧版|歐版)\s*(?:edition|version|pressing)?")
            .expect("valid European edition regex")
            .is_match(folder_name)
            .then(|| "XE".to_string())
    })
    .or_else(|| {
        Regex::new(
            r"(?i)(?:^|[^\p{Alphabetic}])(?:american|usa?|美版)(?:\s*(?:edition|version|pressing|版))?(?:$|[^\p{Alphabetic}])",
        )
            .expect("valid US edition regex")
            .is_match(folder_name)
            .then(|| "US".to_string())
    });

    let mut folder_album = Regex::new(r"^\s*\d{4}\s*[.-]\s*")
        .expect("valid folder year regex")
        .replace(folder_name, "")
        .to_string();
    folder_album = Regex::new(r"(?i)-?tracks\s*$")
        .expect("valid tracks suffix regex")
        .replace(&folder_album, "")
        .to_string();
    folder_album = Regex::new(r"[\[【(（][^\]】)）]*[\]】)）]")
        .expect("valid folder annotation regex")
        .replace_all(&folder_album, " ")
        .to_string();
    folder_album = Regex::new(
        r"(?i)\s+(?:japanese|japan|european|europe|american|usa?)\s+(?:edition|version|pressing)\s*$",
    )
    .expect("valid trailing edition regex")
    .replace(&folder_album, "")
    .trim()
    .to_string();
    let folder_album = non_empty(folder_album);
    let tagged_album = tagged_album
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    FolderAlbumEvidence {
        search_album: tagged_album.clone().or_else(|| folder_album.clone()),
        tagged_album,
        folder_album,
        year,
        country,
    }
}

fn is_compilation_folder(name: &str) -> bool {
    let normalized = Regex::new(r"[ _]+")
        .expect("valid compilation whitespace regex")
        .replace_all(name.trim(), " ")
        .to_lowercase();
    matches!(
        normalized.as_str(),
        "compilations"
            | "compilation"
            | "various artists"
            | "various"
            | "va"
            | "soundtracks"
            | "soundtrack"
            | "ost"
            | "samplers"
            | "sampler"
            | "christmas"
    )
}

fn filename_track_number(stem: &str) -> Option<u32> {
    Regex::new(r"^(\d{1,3})\s*[.\-_\s]+")
        .expect("valid filename track number regex")
        .captures(stem)
        .and_then(|captures| captures.get(1))
        .and_then(|number| number.as_str().parse().ok())
}

/// Extract the track title from a filename stem by removing the track number prefix.
/// E.g. "01 从今以后" → Some("从今以后"), "track" → None.
fn filename_track_title(stem: &str) -> Option<String> {
    let title = Regex::new(r"^\d{1,3}\s*[.\-_\s]+")
        .expect("valid filename title regex")
        .replace(stem, "")
        .trim()
        .to_string();
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

pub fn folder_candidate(request: &LookupRequest) -> AlbumCandidate {
    let artist = request.artist_hint.clone();
    let album_artist = if artist.as_deref().is_some_and(is_compilation_folder) {
        Some("Various Artists".to_string())
    } else {
        artist
    };
    let album_artists = album_artist.iter().cloned().collect::<Vec<_>>();
    let album_artists = split_collaborative_artists(&album_artist, &album_artists);
    let tracks = request
        .tracks
        .iter()
        .map(|track| {
            let artists = split_collaborative_artists(&track.artist, &track.artists);
            TrackCandidate {
                artists,
                ..track.clone()
            }
        })
        .collect();
    AlbumCandidate {
        artist: album_artist.clone(),
        artists: album_artists.clone(),
        album: request.album_hint.clone(),
        album_artist,
        album_artists,
        year: request.year_hint.clone(),
        musicbrainz_album_id: request.musicbrainz_album_id.clone(),
        musicbrainz_artist_id: request.musicbrainz_artist_id.clone(),
        discogs_release_id: request.discogs_release_id.clone(),
        discogs_artist_id: request.discogs_artist_id.clone(),
        tracks,
        source: LookupSource::Folder,
        ..AlbumCandidate::default()
    }
}

/// Normalize a fallback (Folder/LLM) artist list so a collaborative credit
/// stored as a single concatenated string (e.g. "陶晶莹&张雨生") is split into
/// individual ARTISTS entries. The display `artist` credit is the split
/// source (preferred over a possibly stale one-item `artists` list left by an
/// LLM correction); an already-explicit multi-artist list is left untouched,
/// and a solo artist that does not match any collaborative separator stays
/// as-is. When `artists` is empty and `artist` is set, the list is derived
/// from `artist` so a solo credit still produces a single ARTISTS entry.
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

fn candidate_priority(candidate: &AlbumCandidate) -> i8 {
    match candidate.source {
        LookupSource::Musicbrainz if candidate.musicbrainz_album_id.is_some() => return -1,
        LookupSource::Discogs if candidate.discogs_release_id.is_some() => return -1,
        _ => {}
    }
    match candidate.source {
        LookupSource::Musicbrainz => 0,
        LookupSource::Discogs => 1,
        LookupSource::Llm => 2,
        LookupSource::Folder => 3,
        _ => 10,
    }
}

fn preferred_candidate_index(candidates: &[AlbumCandidate]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .min_by_key(|(_, candidate)| candidate_priority(candidate))
        .map(|(index, _)| index)
}

pub fn merge_candidate_fields(candidates: Vec<AlbumCandidate>) -> Vec<AlbumCandidate> {
    let Some(preferred_index) = preferred_candidate_index(&candidates) else {
        return Vec::new();
    };
    let preferred = &candidates[preferred_index];
    let provider_year = candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.source,
                LookupSource::Musicbrainz | LookupSource::Discogs
            )
        })
        .filter_map(|candidate| {
            candidate
                .year
                .as_ref()
                .map(|year| (candidate_priority(candidate), year))
        })
        .min_by_key(|(priority, _)| *priority)
        .map(|(_, year)| year.clone());
    let mut merged = AlbumCandidate {
        source: preferred.source,
        verification: preferred.verification.clone(),
        year: provider_year,
        ..AlbumCandidate::default()
    };

    for candidate in std::iter::once(preferred).chain(
        candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| (index != preferred_index).then_some(candidate)),
    ) {
        fill_option(&mut merged.artist, &candidate.artist);
        fill_vec(&mut merged.artists, &candidate.artists);
        fill_option(&mut merged.album, &candidate.album);
        fill_option(&mut merged.album_artist, &candidate.album_artist);
        fill_vec(&mut merged.album_artists, &candidate.album_artists);
        fill_option(&mut merged.year, &candidate.year);
        fill_option(&mut merged.genre, &candidate.genre);
        fill_option(
            &mut merged.musicbrainz_album_id,
            &candidate.musicbrainz_album_id,
        );
        fill_option(
            &mut merged.musicbrainz_artist_id,
            &candidate.musicbrainz_artist_id,
        );
        fill_option(
            &mut merged.discogs_release_id,
            &candidate.discogs_release_id,
        );
        fill_option(&mut merged.discogs_artist_id, &candidate.discogs_artist_id);
        if merged.tracks.is_empty() {
            merged.tracks.clone_from(&candidate.tracks);
        } else {
            fill_track_gaps(&mut merged.tracks, &candidate.tracks);
        }
    }

    if merged.album_artists.is_empty() {
        if let Some(album_artist) = &merged.album_artist {
            merged.album_artists.push(album_artist.clone());
        }
    } else if merged.album_artist.is_none() {
        merged.album_artist = merged.album_artists.first().cloned();
    }

    let mut result = vec![merged];
    result.extend(
        candidates
            .into_iter()
            .enumerate()
            .filter_map(|(index, candidate)| (index != preferred_index).then_some(candidate)),
    );
    result
}

fn fill_option<T: Clone>(target: &mut Option<T>, source: &Option<T>) {
    if target.is_none() {
        target.clone_from(source);
    }
}

fn fill_vec<T: Clone>(target: &mut Vec<T>, source: &[T]) {
    if target.is_empty() {
        target.extend_from_slice(source);
    }
}

fn fill_track_gaps(target: &mut [TrackCandidate], source: &[TrackCandidate]) {
    for (target, source) in target.iter_mut().zip(source) {
        if target
            .artist
            .as_deref()
            .is_none_or(|artist| artist.trim().is_empty())
        {
            target.artist.clone_from(&source.artist);
            if target.artists.is_empty() {
                target.artists.clone_from(&source.artists);
            }
        }
        fill_option(
            &mut target.musicbrainz_track_id,
            &source.musicbrainz_track_id,
        );
        fill_option(&mut target.length, &source.length);
        fill_option(&mut target.genre, &source.genre);
    }
}

pub fn apply_canonical_artist_name(
    mut candidate: AlbumCandidate,
    canonical_name: Option<&str>,
) -> AlbumCandidate {
    let Some(canonical_name) = clean_provider_artist_name(canonical_name) else {
        return candidate;
    };
    let artist_identities = split_collaborative_artists(&candidate.artist, &candidate.artists);
    let album_artist_identities =
        split_collaborative_artists(&candidate.album_artist, &candidate.album_artists);
    let old_album_artist = artist_identities
        .first()
        .or_else(|| album_artist_identities.first())
        .cloned()
        .or_else(|| candidate.artist.clone());
    let Some(old_album_artist) = old_album_artist else {
        return candidate;
    };
    candidate.artists =
        canonicalize_artist_identities(&artist_identities, &old_album_artist, &canonical_name);
    candidate.album_artists = canonicalize_artist_identities(
        &album_artist_identities,
        &old_album_artist,
        &canonical_name,
    );
    candidate.artist = Some(candidate.artists.join(" & "));
    candidate.album_artist = Some(candidate.album_artists.join(" & "));
    for track in &mut candidate.tracks {
        let same_solo_identity = |artist: &str| {
            artist.trim().eq_ignore_ascii_case(&canonical_name)
                || artist.trim().eq_ignore_ascii_case(old_album_artist.trim())
        };
        let artist_is_solo = track.artist.as_deref().is_none_or(&same_solo_identity);
        let artists_are_solo = track
            .artists
            .iter()
            .all(|artist| same_solo_identity(artist) || is_placeholder_artist_identity(artist));
        let is_solo = artist_is_solo && artists_are_solo;
        if is_solo {
            track.artist = Some(canonical_name.clone());
            track.artists = vec![canonical_name.clone()];
        }
    }
    candidate
}

fn canonicalize_artist_identities(
    artists: &[String],
    old_artist: &str,
    canonical_name: &str,
) -> Vec<String> {
    let mut canonicalized = artists
        .iter()
        .filter(|artist| !is_placeholder_artist_identity(artist))
        .map(|artist| {
            if artist.trim().eq_ignore_ascii_case(old_artist.trim()) {
                canonical_name.to_string()
            } else {
                artist.clone()
            }
        })
        .collect::<Vec<_>>();
    if canonicalized.is_empty() {
        canonicalized.push(canonical_name.to_string());
    }
    canonicalized
}

#[cfg(test)]
fn apply_verified_canonical_artist_name_with_provenance(
    request: &LookupRequest,
    candidate: AlbumCandidate,
    selected_provider_identity: Option<(LookupSource, String)>,
) -> AlbumCandidate {
    let canonical_name = request.artist_hint.as_deref();
    let has_cjk_name = canonical_name.is_some_and(|name| {
        Regex::new(r"\p{Han}")
            .expect("valid CJK artist regex")
            .is_match(name)
    });
    let same_provider_identity = match selected_provider_identity {
        Some((LookupSource::Musicbrainz, selected))
            if candidate.source == LookupSource::Musicbrainz =>
        {
            request
                .musicbrainz_artist_id
                .as_deref()
                .is_some_and(|resolved| resolved == selected)
        }
        Some((LookupSource::Discogs, selected)) if candidate.source == LookupSource::Discogs => {
            request
                .discogs_artist_id
                .as_deref()
                .is_some_and(|resolved| resolved == selected)
        }
        _ => false,
    };
    if has_cjk_name && same_provider_identity {
        apply_canonical_artist_name(candidate, canonical_name)
    } else {
        candidate
    }
}

fn is_placeholder_artist_identity(artist: &str) -> bool {
    artist.trim() == "???"
}

#[cfg(test)]
fn provider_artist_identity(candidate: &AlbumCandidate) -> Option<(LookupSource, String)> {
    match candidate.source {
        LookupSource::Musicbrainz => candidate
            .musicbrainz_artist_id
            .clone()
            .map(|id| (LookupSource::Musicbrainz, id)),
        LookupSource::Discogs => candidate
            .discogs_artist_id
            .clone()
            .map(|id| (LookupSource::Discogs, id)),
        _ => None,
    }
}

fn fill_request_artist_identity(request: &mut LookupRequest, identity: &ArtistIdentity) {
    fill_option(
        &mut request.musicbrainz_artist_id,
        &identity.musicbrainz_artist_id,
    );
    fill_option(&mut request.discogs_artist_id, &identity.discogs_artist_id);
    for alias in &identity.english_aliases {
        if !request
            .artist_aliases
            .iter()
            .any(|known| known.eq_ignore_ascii_case(alias))
        {
            request.artist_aliases.push(alias.clone());
        }
    }
}

fn fill_candidate_artist_identity(candidate: &mut AlbumCandidate, identity: &ArtistIdentity) {
    if candidate.source == LookupSource::Llm {
        return;
    }
    fill_option(
        &mut candidate.musicbrainz_artist_id,
        &identity.musicbrainz_artist_id,
    );
    fill_option(
        &mut candidate.discogs_artist_id,
        &identity.discogs_artist_id,
    );
}

fn clean_provider_artist_name(name: Option<&str>) -> Option<String> {
    let name = name?.trim();
    let numbered = Regex::new(r"\s+\(\d+\)$").expect("valid provider suffix regex");
    let disambiguation =
        Regex::new(r"\s+\([^)]*[;，,][^)]*\)\s*$").expect("valid provider suffix regex");
    let cleaned = disambiguation
        .replace(&numbered.replace(name, ""), "")
        .trim()
        .to_string();
    (!cleaned.is_empty()).then_some(cleaned)
}

pub fn hints_are_ambiguous(
    album_hint: Option<&str>,
    artist_hint: Option<&str>,
    path: &str,
    year_hint: Option<&str>,
) -> bool {
    let (Some(album_hint), Some(artist_hint)) = (album_hint, artist_hint) else {
        return true;
    };
    if album_hint.is_empty() || artist_hint.is_empty() {
        return true;
    }
    let folder_name = path
        .split(['/', '\\'])
        .rfind(|segment| !segment.is_empty())
        .unwrap_or_default();
    let format_suffix =
        Regex::new(r"(?i)\[?(flac|mp3|wav|aac|ogg|m4a|wma|ape|flac\s*分轨|wav\s*分轨)\]?\s*$")
            .expect("valid format suffix regex");
    let clean_name = format_suffix.replace(folder_name, "");
    let year_prefix = Regex::new(r"^\d{4}[-.]").expect("valid year prefix regex");
    let cjk_dot = Regex::new(r"[\p{Han}]\.[\p{Han}]").expect("valid CJK dot regex");

    clean_name
        .chars()
        .any(|character| "[]《》「」【】".contains(character))
        || cjk_dot.is_match(folder_name)
        || folder_name.contains('。')
        || year_prefix.is_match(album_hint)
        || (album_hint.contains('.') && year_hint.is_none())
}

#[derive(Serialize)]
struct HashTrack<'a> {
    title: &'a Option<String>,
    track_number: Option<u32>,
    disc_number: Option<u32>,
    musicbrainz_track_id: &'a Option<String>,
}

#[derive(Serialize)]
struct HashQuery<'a> {
    cache_version: u8,
    artist_hint: &'a Option<String>,
    artist_aliases: &'a Vec<String>,
    tagged_artist_hint: &'a Option<String>,
    folder_artist_hint: &'a Option<String>,
    album_hint: &'a Option<String>,
    tagged_album_hint: &'a Option<String>,
    folder_album_hint: &'a Option<String>,
    year_hint: &'a Option<String>,
    country_hint: &'a Option<String>,
    musicbrainz_album_id: &'a Option<String>,
    musicbrainz_artist_id: &'a Option<String>,
    discogs_release_id: &'a Option<String>,
    discogs_artist_id: &'a Option<String>,
    selected_disc_number: Option<u32>,
    tracks: Vec<HashTrack<'a>>,
    track_count: usize,
}

pub fn query_hash(request: &LookupRequest) -> String {
    let query = HashQuery {
        cache_version: 5,
        artist_hint: &request.artist_hint,
        artist_aliases: &request.artist_aliases,
        tagged_artist_hint: &request.tagged_artist_hint,
        folder_artist_hint: &request.folder_artist_hint,
        album_hint: &request.album_hint,
        tagged_album_hint: &request.tagged_album_hint,
        folder_album_hint: &request.folder_album_hint,
        year_hint: &request.year_hint,
        country_hint: &request.country_hint,
        musicbrainz_album_id: &request.musicbrainz_album_id,
        musicbrainz_artist_id: &request.musicbrainz_artist_id,
        discogs_release_id: &request.discogs_release_id,
        discogs_artist_id: &request.discogs_artist_id,
        selected_disc_number: request.selected_disc_number,
        tracks: request
            .tracks
            .iter()
            .map(|track| HashTrack {
                title: &track.title,
                track_number: track.track_number,
                disc_number: track.disc_number,
                musicbrainz_track_id: &track.musicbrainz_track_id,
            })
            .collect(),
        track_count: request.tracks.len(),
    };
    let payload = serde_json::to_vec(&query).expect("hash query serializes");
    format!("{:x}", Sha256::digest(payload))
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
        country: album.country,
        formats: album.formats,
        catalog_number: album.catalog_number,
        barcode: album.barcode,
        linked_discogs_release_id: album.linked_discogs_release_id,
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
        country: album.country,
        formats: album.formats,
        catalog_number: album.catalog_number,
        barcode: album.barcode,
        linked_discogs_release_id: album.linked_discogs_release_id,
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
            .map(|value| convert_chinese_text(value, target))
    };
    let convert_many = |values: &[String]| {
        values
            .iter()
            .map(|value| convert_chinese_text(value, target))
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

pub fn protect_candidate_tracks(
    request: &LookupRequest,
    candidate: &AlbumCandidate,
) -> AlbumCandidate {
    if !matches!(
        candidate.source,
        LookupSource::Musicbrainz | LookupSource::Discogs
    ) || candidate.tracks.is_empty()
    {
        return candidate.clone();
    }
    let filenames = collect_audio_files(Path::new(&request.path))
        .into_iter()
        .filter_map(|path| Path::new(&path).file_name()?.to_str().map(str::to_string))
        .collect::<Vec<_>>();
    let artist_hints = request
        .artist_hint
        .iter()
        .chain(candidate.artist.iter())
        .chain(candidate.album_artist.iter())
        .chain(candidate.artists.iter())
        .chain(candidate.album_artists.iter())
        .filter(|artist| !artist.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    let source = match candidate.source {
        LookupSource::Musicbrainz => "musicbrainz",
        LookupSource::Discogs => "discogs",
        _ => unreachable!("remote sources checked above"),
    };
    // If the user selected a specific disc (e.g. CD1 of a 2-CD set), scope
    // the provider candidate tracks to that disc number. A bonus track on
    // the selected disc is still an allowed provider extra, so do not use
    // the full multi-disc release merely because the counts differ.
    let candidate_tracks = if let Some(disc_number) = request.selected_disc_number {
        let scoped: Vec<_> = candidate
            .tracks
            .iter()
            .filter(|track| track.disc_number == Some(disc_number))
            .cloned()
            .collect();
        if !scoped.is_empty()
            || candidate
                .tracks
                .iter()
                .any(|track| track.disc_number.is_some())
        {
            scoped
        } else {
            candidate.tracks.clone()
        }
    } else {
        candidate.tracks.clone()
    };
    let matched = match_remote_candidate_tracks(
        &request.tracks,
        &filenames,
        &candidate_tracks,
        source,
        &artist_hints,
        &[],
    );
    let mut protected = candidate.clone();
    protected.tracks = matched.tracks;
    protected
}

#[cfg(test)]
fn select_protect_and_canonicalize_candidate(
    request: &LookupRequest,
    fresh: Vec<AlbumCandidate>,
    cached: Vec<AlbumCandidate>,
    folder: AlbumCandidate,
) -> Option<(AlbumCandidate, usize)> {
    let candidates = candidate_source_rows(fresh, cached, folder);
    let candidate_count = candidates.len();
    let preferred_index = preferred_candidate_index(&candidates)?;
    let selected_provider_identity = provider_artist_identity(&candidates[preferred_index]);
    let selected = merge_candidate_fields(candidates).into_iter().next()?;
    let protected = protect_candidate_tracks(request, &selected);
    let canonicalized = apply_verified_canonical_artist_name_with_provenance(
        request,
        protected,
        selected_provider_identity,
    );
    Some((canonicalized, candidate_count))
}

fn candidate_source_rows(
    mut fresh: Vec<AlbumCandidate>,
    cached: Vec<AlbumCandidate>,
    folder: AlbumCandidate,
) -> Vec<AlbumCandidate> {
    fresh.retain(|candidate| candidate.source != LookupSource::Dataset);
    fresh.push(folder);
    fresh.extend(
        cached
            .into_iter()
            .filter(|candidate| candidate.source != LookupSource::Dataset),
    );
    fresh
}

pub fn combine_candidate_sources(
    fresh: Vec<AlbumCandidate>,
    cached: Vec<AlbumCandidate>,
    folder: AlbumCandidate,
) -> Vec<AlbumCandidate> {
    merge_candidate_fields(candidate_source_rows(fresh, cached, folder))
}

pub fn filter_candidates_for_album(
    album_hint: Option<&str>,
    candidates: Vec<AlbumCandidate>,
    request_mb_album_id: Option<&str>,
) -> Vec<AlbumCandidate> {
    candidates
        .into_iter()
        .filter(|candidate| {
            // Trust candidates obtained via direct MB ID lookup — the user
            // explicitly set this ID in the file tags so the album name on
            // MusicBrainz may differ from the folder name (e.g. "SACD Best
            // Collection" vs "精选 Collection").
            if let Some(request_id) = request_mb_album_id {
                if candidate.musicbrainz_album_id.as_deref() == Some(request_id) {
                    return true;
                }
            }
            let (Some(hint), Some(album)) = (album_hint, candidate.album.as_deref()) else {
                return true;
            };
            album_names_match(hint, album)
        })
        .collect()
}

pub fn rank_artist_releases(
    releases: Vec<ProviderReleaseSummary>,
    album_hint: Option<&str>,
    year_hint: Option<&str>,
) -> Vec<ProviderReleaseSummary> {
    rank_artist_releases_with_country(releases, album_hint, year_hint, None)
}

fn rank_artist_releases_with_country(
    releases: Vec<ProviderReleaseSummary>,
    album_hint: Option<&str>,
    year_hint: Option<&str>,
    country_hint: Option<&str>,
) -> Vec<ProviderReleaseSummary> {
    let mut ranked = releases
        .into_iter()
        .filter(|release| album_hint.is_none_or(|hint| album_names_match(hint, &release.title)))
        .map(|release| {
            let country_score = match (country_hint, release.country.as_deref()) {
                (Some(hint), Some(country)) if countries_match(hint, country) => 2,
                (Some(_), None) => 1,
                _ => 0,
            };
            let year_score = match (year_hint, release.year) {
                (Some(hint), Some(year)) if hint == year.to_string() => 1,
                _ => 0,
            };
            (country_score, year_score, release)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(
        |(left_country, left_year, left), (right_country, right_year, right)| {
            right_country
                .cmp(left_country)
                .then_with(|| right_year.cmp(left_year))
                .then_with(|| right.year.cmp(&left.year))
                .then_with(|| left.title.cmp(&right.title))
        },
    );
    ranked
        .into_iter()
        .map(|(_, _, release)| release)
        .collect()
}

fn rank_candidate_details(
    candidates: Vec<AlbumCandidate>,
    request: &LookupRequest,
    source: &str,
) -> Vec<AlbumCandidate> {
    let filenames = collect_audio_files(Path::new(&request.path))
        .into_iter()
        .filter_map(|path| Path::new(&path).file_name()?.to_str().map(str::to_string))
        .collect::<Vec<_>>();
    let artist_hints = request.artist_hint.iter().cloned().collect::<Vec<_>>();
    let mut scored = candidates
        .into_iter()
        .enumerate()
        .map(|(order, candidate)| {
            let matched = match_remote_candidate_tracks(
                &request.tracks,
                &filenames,
                &candidate.tracks,
                source,
                &artist_hints,
                &[],
            );
            let title_matches = matched
                .evidence
                .iter()
                .filter(|evidence| {
                    evidence.is_some() && **evidence != Some(MatchEvidence::Position)
                })
                .count();
            let track_delta = candidate.tracks.len().abs_diff(request.tracks.len());
            (title_matches, track_delta, order, candidate)
        })
        .collect::<Vec<_>>();
    scored.sort_by(
        |(left_matches, left_delta, left_order, _),
         (right_matches, right_delta, right_order, _)| {
            right_matches
                .cmp(left_matches)
                .then_with(|| left_delta.cmp(right_delta))
                .then_with(|| left_order.cmp(right_order))
        },
    );
    scored
        .into_iter()
        .map(|(_, _, _, candidate)| candidate)
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredibilityScore {
    pub explicit_id: bool,
    pub edition_matches: usize,
    pub exact_track_count: bool,
    pub provider_extras: usize,
    pub exact_year: bool,
}

fn select_credible_provider_candidate(
    request: &LookupRequest,
    candidates: Vec<AlbumCandidate>,
) -> Option<AlbumCandidate> {
    let mut credible = candidates
        .into_iter()
        .filter_map(|candidate| {
            provider_candidate_credibility(request, &candidate)
                .ok()
                .map(|score| (score, candidate))
        })
        .collect::<Vec<_>>();
    credible.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .explicit_id
            .cmp(&left_score.explicit_id)
            .then_with(|| right_score.edition_matches.cmp(&left_score.edition_matches))
            .then_with(|| right_score.exact_track_count.cmp(&left_score.exact_track_count))
            .then_with(|| left_score.provider_extras.cmp(&right_score.provider_extras))
            .then_with(|| right_score.exact_year.cmp(&left_score.exact_year))
            .then_with(|| {
                let priority = |source| match source {
                    LookupSource::Musicbrainz => 0,
                    LookupSource::Discogs => 1,
                    _ => 2,
                };
                priority(left.source).cmp(&priority(right.source))
            })
            .then_with(|| provider_stable_id(left).cmp(provider_stable_id(right)))
    });
    let mut selected = credible.into_iter().next()?.1;
    if selected.source == LookupSource::Musicbrainz && selected.discogs_release_id.is_none() {
        selected.discogs_release_id = selected.linked_discogs_release_id.clone();
    }
    Some(selected)
}

fn provider_stable_id(candidate: &AlbumCandidate) -> &str {
    candidate
        .musicbrainz_album_id
        .as_deref()
        .or(candidate.discogs_release_id.as_deref())
        .unwrap_or_default()
}

fn safe_provider_error(error: &str) -> String {
    Regex::new(r"https?://\S+")
        .expect("valid URL redaction regex")
        .replace_all(error, "[redacted-url]")
        .chars()
        .take(240)
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
enum ProviderAuthorityDecision {
    Selected(Box<AlbumCandidate>),
    Providerless,
    Unavailable,
}

fn provider_authority_decision(
    request: &LookupRequest,
    candidates: Vec<AlbumCandidate>,
    attempts: &[ProviderAttempt],
) -> ProviderAuthorityDecision {
    if let Some(candidate) = select_credible_provider_candidate(request, candidates) {
        return ProviderAuthorityDecision::Selected(Box::new(candidate));
    }
    if attempts
        .iter()
        .any(|attempt| attempt.status == ProviderAttemptStatus::Unavailable)
    {
        ProviderAuthorityDecision::Unavailable
    } else {
        ProviderAuthorityDecision::Providerless
    }
}

fn normalized_release_identity(value: &str) -> String {
    let without_annotations = Regex::new(
        r"(?i)[\[【(（][^\]】)）]*(?:edition|version|pressing|remaster|deluxe|日本|japan|精選|精选)[^\]】)）]*[\]】)）]",
    )
    .expect("valid release annotation regex")
    .replace_all(value, " ");
    without_annotations
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_cjk_text(value: &str) -> bool {
    value.chars().any(|character| {
        matches!(character as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF)
    })
}

fn exact_album_identity(left: &str, right: &str) -> bool {
    let left = normalized_release_identity(left);
    let right = normalized_release_identity(right);
    left == right
        || ((has_cjk_text(&left) || has_cjk_text(&right)) && album_names_match(&left, &right))
}

fn exact_artist_identity(left: &str, right: &str) -> bool {
    normalized_release_identity(left.trim_end_matches('*'))
        == normalized_release_identity(right.trim_end_matches('*'))
}

fn normalized_country(value: &str) -> String {
    match normalized_release_identity(value).as_str() {
        "jp" | "japan" | "japanese" => "JP".to_string(),
        "us" | "usa" | "united states" => "US".to_string(),
        "xe" | "europe" | "european" => "XE".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

fn discogs_country_name(value: &str) -> Option<&'static str> {
    match normalized_country(value).as_str() {
        "JP" => Some("Japan"),
        "US" => Some("United States"),
        "XE" => Some("Europe"),
        _ => None,
    }
}

fn countries_match(left: &str, right: &str) -> bool {
    normalized_country(left) == normalized_country(right)
}

fn candidate_tracks_for_request(
    request: &LookupRequest,
    candidate: &AlbumCandidate,
) -> Vec<TrackCandidate> {
    if let Some(disc_number) = request.selected_disc_number {
        let scoped = candidate
            .tracks
            .iter()
            .filter(|track| track.disc_number == Some(disc_number))
            .cloned()
            .collect::<Vec<_>>();
        if !scoped.is_empty()
            || candidate
                .tracks
                .iter()
                .any(|track| track.disc_number.is_some())
        {
            return scoped;
        }
    }
    candidate.tracks.clone()
}

pub fn provider_candidate_credibility(
    request: &LookupRequest,
    candidate: &AlbumCandidate,
) -> Result<CredibilityScore, String> {
    if !matches!(candidate.source, LookupSource::Musicbrainz | LookupSource::Discogs) {
        return Err("candidate is not provider-backed".to_string());
    }
    let hint_album = request
        .tagged_album_hint
        .as_deref()
        .or(request.album_hint.as_deref())
        .ok_or_else(|| "album evidence is missing".to_string())?;
    let candidate_album = candidate
        .album
        .as_deref()
        .ok_or_else(|| "provider album is missing".to_string())?;
    if !exact_album_identity(hint_album, candidate_album) {
        return Err("provider album title conflicts with local evidence".to_string());
    }
    let provider_artist = candidate
        .album_artist
        .as_deref()
        .or(candidate.artist.as_deref())
        .filter(|artist| !artist.trim().is_empty())
        .ok_or_else(|| "provider artist is missing".to_string())?;
    if let Some(hint) = request.artist_hint.as_deref() {
        if !exact_artist_identity(hint, provider_artist)
            && !request
                .artist_aliases
                .iter()
                .any(|alias| exact_artist_identity(alias, provider_artist))
        {
            return Err("provider artist conflicts with local evidence".to_string());
        }
    }
    if let (Some(hint), Some(provider)) = (request.year_hint.as_deref(), candidate.year.as_deref()) {
        if hint != provider {
            return Err("provider year conflicts with local evidence".to_string());
        }
    }
    if let (Some(hint), Some(provider)) = (
        request.country_hint.as_deref(),
        candidate.country.as_deref(),
    ) {
        if !countries_match(hint, provider) {
            return Err("provider country conflicts with edition evidence".to_string());
        }
    }

    let provider_tracks = candidate_tracks_for_request(request, candidate);
    let filenames = request
        .tracks
        .iter()
        .map(|track| track.filename.clone().unwrap_or_default())
        .collect::<Vec<_>>();
    let artists = request.artist_hint.iter().cloned().collect::<Vec<_>>();
    let source = if candidate.source == LookupSource::Musicbrainz {
        "musicbrainz"
    } else {
        "discogs"
    };
    let matched = match_remote_candidate_tracks(
        &request.tracks,
        &filenames,
        &provider_tracks,
        source,
        &artists,
        &[],
    );
    if matched.stats.matched != request.tracks.len()
        || matched
            .evidence
            .iter()
            .any(|evidence| evidence.is_none() || *evidence == Some(MatchEvidence::Position))
    {
        return Err("provider tracks do not strongly cover every local file".to_string());
    }

    Ok(CredibilityScore {
        explicit_id: match candidate.source {
            LookupSource::Musicbrainz => request.musicbrainz_album_id.as_ref().is_some_and(|_| {
                request.musicbrainz_album_id == candidate.musicbrainz_album_id
            }),
            LookupSource::Discogs => request
                .discogs_release_id
                .as_ref()
                .is_some_and(|_| request.discogs_release_id == candidate.discogs_release_id),
            _ => false,
        },
        edition_matches: usize::from(
            request.country_hint.is_some()
                && request
                    .country_hint
                    .as_deref()
                    .zip(candidate.country.as_deref())
                    .is_some_and(|(hint, provider)| countries_match(hint, provider)),
        ),
        exact_track_count: provider_tracks.len() == request.tracks.len(),
        provider_extras: provider_tracks.len().saturating_sub(request.tracks.len()),
        exact_year: request.year_hint.is_some()
            && request.year_hint.as_deref() == candidate.year.as_deref(),
    })
}

fn provider_rejection_code(reason: &str) -> &'static str {
    if reason.contains("track") {
        "track_mapping_incomplete"
    } else if reason.contains("album title") {
        "title_conflict"
    } else if reason.contains("artist") {
        "artist_conflict"
    } else if reason.contains("year") {
        "year_conflict"
    } else if reason.contains("country") {
        "country_conflict"
    } else {
        "provider_field_missing"
    }
}

fn provider_selection_diagnostics(
    request: &LookupRequest,
    candidates: &[AlbumCandidate],
    attempts: &[ProviderAttempt],
) -> Vec<serde_json::Value> {
    let mut candidate_counts = HashMap::<&str, usize>::new();
    let mut credible_counts = HashMap::<&str, usize>::new();
    let mut rejection_counts = HashMap::<&str, usize>::new();
    for attempt in attempts {
        candidate_counts.entry(attempt.provider).or_default();
        credible_counts.entry(attempt.provider).or_default();
    }
    for candidate in candidates {
        let provider = lookup_source_name(candidate.source);
        if !matches!(candidate.source, LookupSource::Musicbrainz | LookupSource::Discogs) {
            continue;
        }
        *candidate_counts.entry(provider).or_default() += 1;
        match provider_candidate_credibility(request, candidate) {
            Ok(_) => *credible_counts.entry(provider).or_default() += 1,
            Err(reason) => {
                *rejection_counts.entry(provider_rejection_code(&reason)).or_default() += 1
            }
        }
    }
    vec![serde_json::json!({
        "stage": "provider_selection",
        "candidateCounts": candidate_counts,
        "credibleCounts": credible_counts,
        "rejectionCodes": rejection_counts,
    })]
}

async fn musicbrainz_artist_candidates(
    client: &MusicBrainzClient,
    cache: &CacheState,
    request: &LookupRequest,
) -> Result<Vec<AlbumCandidate>, String> {
    let Some(artist_id) = request.musicbrainz_artist_id.as_deref() else {
        return Ok(Vec::new());
    };
    let releases = cached_artist_releases(cache, "musicbrainz-v2", artist_id).unwrap_or_default();
    let releases = if releases.is_empty() {
        let fetched = client.artist_release_page_result(artist_id, 1, 100).await?;
        if let Ok(value) = serde_json::to_value(&fetched) {
            let _ = cache.set_artist_releases("musicbrainz-v2", artist_id, 1, &value);
        }
        fetched
    } else {
        releases
    };
    let mut candidates = Vec::new();
    for release in rank_artist_releases_with_country(
        releases,
        request.album_hint.as_deref(),
        request.year_hint.as_deref(),
        request.country_hint.as_deref(),
    )
    .into_iter()
    .take(3)
    {
        let album = cached_release_detail(cache, "musicbrainz-v4", &release.id);
        let album = match album {
            Some(album) => Some(album),
            None => {
                let fetched = client.release_by_id_result(&release.id).await?;
                if let Ok(value) = serde_json::to_value(&fetched) {
                    let _ = cache.set_release_detail("musicbrainz-v4", &release.id, &value);
                }
                Some(fetched)
            }
        };
        if let Some(album) = album {
            candidates.push(musicbrainz_candidate(album));
        }
    }
    Ok(rank_candidate_details(candidates, request, "musicbrainz"))
}

async fn discogs_artist_candidates(
    client: &DiscogsClient,
    cache: &CacheState,
    request: &LookupRequest,
) -> Result<Vec<AlbumCandidate>, String> {
    let Some(artist_id) = request.discogs_artist_id.as_deref() else {
        return Ok(Vec::new());
    };
    let releases = cached_artist_releases(cache, "discogs-v2", artist_id).unwrap_or_default();
    let releases = if releases.is_empty() {
        let fetched = client.artist_release_page_result(artist_id, 1, 100).await?;
        if let Ok(value) = serde_json::to_value(&fetched) {
            let _ = cache.set_artist_releases("discogs-v2", artist_id, 1, &value);
        }
        fetched
    } else {
        releases
    };
    let mut candidates = Vec::new();
    for release in rank_artist_releases_with_country(
        releases,
        request.album_hint.as_deref(),
        request.year_hint.as_deref(),
        request.country_hint.as_deref(),
    )
    .into_iter()
    .take(3)
    {
        let album = cached_release_detail(cache, "discogs-v3", &release.id);
        let album = match album {
            Some(album) => Some(album),
            None => {
                let fetched = client.release_metadata_result(&release.id).await?;
                if let Ok(value) = serde_json::to_value(&fetched) {
                    let _ = cache.set_release_detail("discogs-v3", &release.id, &value);
                }
                Some(fetched)
            }
        };
        if let Some(album) = album {
            candidates.push(discogs_candidate(album));
        }
    }
    Ok(rank_candidate_details(candidates, request, "discogs"))
}

fn cached_artist_releases(
    cache: &CacheState,
    provider: &str,
    artist_id: &str,
) -> Option<Vec<ProviderReleaseSummary>> {
    serde_json::from_value(cache.artist_releases(provider, artist_id, 1)?).ok()
}

fn cached_release_detail(
    cache: &CacheState,
    provider: &str,
    release_id: &str,
) -> Option<ProviderAlbum> {
    serde_json::from_value(cache.release_detail(provider, release_id)?).ok()
}

pub fn should_replace_lookup_cache(fresh: &[AlbumCandidate], had_cached: bool) -> bool {
    !fresh.is_empty()
        && (!had_cached
            || fresh.iter().any(|candidate| {
                matches!(
                    candidate.source,
                    LookupSource::Musicbrainz | LookupSource::Discogs
                )
            }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoTagOutcome {
    Applied,
    NeedsReview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAttemptStatus {
    Matched,
    NoMatch,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAttempt {
    pub provider: &'static str,
    pub status: ProviderAttemptStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub retry_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoTagRunResult {
    pub outcome: AutoTagOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<LookupSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<AlbumCandidate>,
    pub written: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_attempts: Vec<ProviderAttempt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_threshold: Option<f64>,
}

pub struct AutoTagServices<'a> {
    pub providers: &'a ProviderState,
    pub cache: &'a CacheState,
    pub queue: &'a WriteQueue,
    pub alias_file: &'a Path,
}

#[derive(Clone)]
pub(crate) struct AutoTagRetryContexts {
    pub(crate) musicbrainz: ProviderRetryContext,
    pub(crate) discogs: ProviderRetryContext,
}

impl AutoTagRetryContexts {
    pub(crate) fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            musicbrainz: ProviderRetryContext::new(
                Arc::clone(&cancelled),
                ProviderRetryMetrics::default(),
            ),
            discogs: ProviderRetryContext::new(cancelled, ProviderRetryMetrics::default()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoTagEvent {
    pub task_id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub message: String,
    pub progress: u64,
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

fn auto_tag_event(
    task_id: &str,
    kind: &'static str,
    message: impl Into<String>,
    progress: u64,
    data: Option<serde_json::Value>,
) -> AutoTagEvent {
    AutoTagEvent {
        task_id: task_id.to_string(),
        kind,
        message: message.into(),
        progress,
        total: 9,
        data,
    }
}

fn auto_tag_completion_message(candidate: &AlbumCandidate) -> &'static str {
    if candidate.genre.is_some() {
        "Complete"
    } else {
        "Complete — genre remains missing"
    }
}

const AI_TAG_CONFIDENCE_THRESHOLD: f64 = 0.85;
const AUTO_TAG_LLM_TIMEOUT: Duration = Duration::from_secs(120);

fn auto_tag_llm_max_tokens(track_count: usize) -> u32 {
    let scaled = 1_024usize.saturating_add(track_count.saturating_mul(128));
    u32::try_from(scaled.clamp(2_048, 8_192)).expect("auto-tag LLM budget fits in u32")
}

#[derive(Clone, Debug, PartialEq)]
pub struct AiValidationFailure {
    pub code: &'static str,
    pub detail: String,
    pub confidence: Option<f64>,
}

fn ai_failure(code: &'static str, detail: impl Into<String>) -> AiValidationFailure {
    AiValidationFailure {
        code,
        detail: detail.into(),
        confidence: None,
    }
}

fn ai_validation_failure_from_error(error: &OpenRouterError) -> AiValidationFailure {
    match error {
        OpenRouterError::Cancelled => ai_failure("ai_cancelled", "AI request cancelled"),
        OpenRouterError::Timeout(milliseconds) => ai_failure(
            "ai_timeout",
            format!("AI request timed out after {milliseconds}ms"),
        ),
        OpenRouterError::Http { status, .. } => ai_failure(
            "ai_failed",
            format!("AI provider returned HTTP {status}"),
        ),
        OpenRouterError::Network(_) => ai_failure("ai_failed", "AI provider request failed"),
        OpenRouterError::MissingChoices(_) => {
            ai_failure("ai_failed", "AI provider returned no choices")
        }
        OpenRouterError::EmptyContent(_) => {
            ai_failure("ai_malformed", "AI provider returned empty content")
        }
        OpenRouterError::NonJson(_) | OpenRouterError::MalformedJson { .. } => {
            ai_failure("ai_malformed", "AI response was malformed JSON")
        }
    }
}

fn required_ai_string(
    value: &serde_json::Value,
    field: &str,
) -> Result<String, AiValidationFailure> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ai_failure("ai_validation_failed", format!("{field} is required")))
}

fn required_ai_strings(
    value: &serde_json::Value,
    field: &str,
) -> Result<Vec<String>, AiValidationFailure> {
    let values = value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ai_failure("ai_validation_failed", format!("{field} is required")))?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if values.is_empty() {
        Err(ai_failure(
            "ai_validation_failed",
            format!("{field} must not be empty"),
        ))
    } else {
        Ok(values)
    }
}

fn required_ai_u32(
    value: &serde_json::Value,
    field: &str,
) -> Result<u32, AiValidationFailure> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
        .filter(|number| *number > 0)
        .ok_or_else(|| {
            ai_failure(
                "ai_validation_failed",
                format!("{field} must be a positive integer"),
            )
        })
}

pub fn validated_ai_candidate(
    request: &LookupRequest,
    value: &serde_json::Value,
) -> Result<AlbumCandidate, AiValidationFailure> {
    let confidence = value
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .filter(|confidence| confidence.is_finite() && (0.0..=1.0).contains(confidence))
        .ok_or_else(|| ai_failure("ai_validation_failed", "confidence must be numeric"))?;
    if confidence < AI_TAG_CONFIDENCE_THRESHOLD {
        let mut failure = ai_failure(
            "ai_low_confidence",
            format!(
                "confidence {confidence:.3} is below {AI_TAG_CONFIDENCE_THRESHOLD:.2}"
            ),
        );
        failure.confidence = Some(confidence);
        return Err(failure);
    }

    let artist = required_ai_string(value, "artist")?;
    let artists = required_ai_strings(value, "artists")?;
    let album_artist = required_ai_string(value, "albumArtist")?;
    let album_artists = required_ai_strings(value, "albumArtists")?;
    let album = required_ai_string(value, "album")?;
    let year = match value.get("year") {
        None | Some(serde_json::Value::Null) => None,
        Some(year) => {
            let year = year
                .as_str()
                .map(str::trim)
                .filter(|year| {
                    year.len() == 4 && year.chars().all(|character| character.is_ascii_digit())
                })
                .ok_or_else(|| ai_failure("ai_validation_failed", "year must be four digits"))?;
            Some(year.to_string())
        }
    };
    let genre = match value.get("genre") {
        None | Some(serde_json::Value::Null) => None,
        Some(genre) => Some(
            genre
                .as_str()
                .map(str::trim)
                .filter(|genre| !genre.is_empty())
                .ok_or_else(|| ai_failure("ai_validation_failed", "genre must not be empty"))?
                .to_string(),
        ),
    };

    let raw_tracks = value
        .get("tracks")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ai_failure("ai_validation_failed", "tracks is required"))?;
    if raw_tracks.len() != request.tracks.len() {
        return Err(ai_failure(
            "ai_validation_failed",
            format!(
                "expected {} tracks, received {}",
                request.tracks.len(),
                raw_tracks.len()
            ),
        ));
    }

    let mut indexed_tracks = Vec::with_capacity(raw_tracks.len());
    let mut indices = HashSet::new();
    let mut positions = HashSet::new();
    let mut per_disc = HashMap::<u32, u32>::new();
    let mut per_disc_numbers = HashMap::<u32, HashSet<u32>>::new();
    let mut common_disc_total = None;
    for track in raw_tracks {
        let index = track
            .get("index")
            .and_then(serde_json::Value::as_u64)
            .and_then(|index| usize::try_from(index).ok())
            .ok_or_else(|| ai_failure("ai_validation_failed", "track index is required"))?;
        if index >= request.tracks.len() || !indices.insert(index) {
            return Err(ai_failure(
                "ai_validation_failed",
                "track indices must cover each local file exactly once",
            ));
        }
        let title = required_ai_string(track, "title")?;
        let track_artist = required_ai_string(track, "artist")?;
        let track_artists = required_ai_strings(track, "artists")?;
        let track_number = required_ai_u32(track, "trackNumber")?;
        let track_total = required_ai_u32(track, "trackTotal")?;
        let disc_number = required_ai_u32(track, "discNumber")?;
        let disc_total = required_ai_u32(track, "discTotal")?;
        if !positions.insert((disc_number, track_number)) {
            return Err(ai_failure(
                "ai_validation_failed",
                "track positions must be unique",
            ));
        }
        *per_disc.entry(disc_number).or_default() += 1;
        per_disc_numbers
            .entry(disc_number)
            .or_default()
            .insert(track_number);
        if common_disc_total.replace(disc_total).is_some_and(|total| total != disc_total) {
            return Err(ai_failure(
                "ai_validation_failed",
                "discTotal must be consistent",
            ));
        }
        indexed_tracks.push((
            index,
            track_total,
            TrackCandidate {
                title: Some(title),
                artist: Some(track_artist),
                artists: track_artists,
                track_number: Some(track_number),
                track_total: Some(track_total),
                disc_number: Some(disc_number),
                disc_total: Some(disc_total),
                ..TrackCandidate::default()
            },
        ));
    }
    let disc_total = common_disc_total.unwrap_or(0);
    if usize::try_from(disc_total).ok() != Some(per_disc.len())
        || (1..=disc_total).any(|disc| !per_disc.contains_key(&disc))
    {
        return Err(ai_failure(
            "ai_validation_failed",
            "disc numbering must be contiguous and match discTotal",
        ));
    }
    for (_, track_total, track) in &indexed_tracks {
        let count = per_disc.get(&track.disc_number.unwrap_or_default()).copied();
        if count != Some(*track_total) {
            return Err(ai_failure(
                "ai_validation_failed",
                "trackTotal must equal the number of tracks on its disc",
            ));
        }
    }
    for (disc_number, numbers) in &per_disc_numbers {
        let Some(track_total) = per_disc.get(disc_number).copied() else {
            return Err(ai_failure(
                "ai_validation_failed",
                "track numbering is missing a disc total",
            ));
        };
        if numbers.len() != usize::try_from(track_total).unwrap_or_default()
            || (1..=track_total).any(|number| !numbers.contains(&number))
        {
            return Err(ai_failure(
                "ai_validation_failed",
                "track numbering must be contiguous on each disc",
            ));
        }
    }
    indexed_tracks.sort_by_key(|(index, _, _)| *index);

    Ok(AlbumCandidate {
        artist: Some(artist),
        artists,
        album: Some(album),
        album_artist: Some(album_artist),
        album_artists,
        year,
        genre,
        confidence: Some(confidence),
        tracks: indexed_tracks
            .into_iter()
            .map(|(_, _, track)| track)
            .collect(),
        source: LookupSource::Llm,
        ..AlbumCandidate::default()
    })
}

#[derive(Debug, PartialEq)]
enum GenreFillOutcome {
    Applied(String),
    Rejected {
        genre: Option<String>,
        confidence: Option<f64>,
    },
    Failed(String),
}

fn llm_confidence(value: Option<&serde_json::Value>) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
}

pub fn genre_from_value(value: &serde_json::Value) -> Option<String> {
    let confidence = llm_confidence(value.get("confidence"))?;
    if confidence < 0.6 {
        return None;
    }
    llm_string(value.get("genre"))
}

fn genre_fill_outcome(result: Result<serde_json::Value, String>) -> GenreFillOutcome {
    match result {
        Ok(value) => match genre_from_value(&value) {
            Some(genre) => GenreFillOutcome::Applied(genre),
            None => GenreFillOutcome::Rejected {
                genre: llm_string(value.get("genre")),
                confidence: llm_confidence(value.get("confidence")),
            },
        },
        Err(error) => GenreFillOutcome::Failed(error),
    }
}

fn llm_string(value: Option<&serde_json::Value>) -> Option<String> {
    let value = value?.as_str()?.trim();
    if value.is_empty()
        || Regex::new(r"(?i)^(?:null|none|undefined|n/a|unknown)$")
            .expect("valid LLM null sentinel regex")
            .is_match(value)
    {
        None
    } else {
        Some(value.to_string())
    }
}

async fn resolve_tags_via_llm(
    request: &LookupRequest,
    config: &AutoTagConfig,
    cancelled: &AtomicBool,
) -> Result<AlbumCandidate, AiValidationFailure> {
    let api_key = config.llm_api_key.as_deref().filter(|key| !key.is_empty());
    if api_key.is_none() {
        tracing::debug!(
            hints_artist = ?request.artist_hint,
            hints_album = ?request.album_hint,
            "LLM resolution skipped: no API key configured",
        );
        return Err(ai_failure(
            "ai_not_configured",
            "no AI API key is configured",
        ));
    }
    let model = config
        .llm_model
        .as_deref()
        .filter(|model| !model.is_empty())
        .unwrap_or("deepseek/deepseek-chat");
    let album_path = Path::new(&request.path);
    let payload = serde_json::json!({
        "folder_name": album_path.file_name().and_then(|name| name.to_str()),
        "parent_name": album_path.parent().and_then(Path::file_name).and_then(|name| name.to_str()),
        "full_path": request.path,
        "parsed_hints": {
            "artist": request.artist_hint,
            "album": request.album_hint,
            "year": request.year_hint,
        },
        "current_tracks": request.tracks.iter().enumerate().map(|(index, track)| serde_json::json!({
            "index": index,
            "title": track.title,
            "filename": track.filename,
            "artist": track.artist,
            "track_number": track.track_number,
            "genre": track.genre,
        })).collect::<Vec<_>>(),
    });
    let messages = vec![
        ChatMessage::system(concat!(
            "Providers found no credible release. Resolve authoritative music metadata from folder structure, parser hints, existing tags, and filenames. ",
            "Return only the requested JSON. Use zero-based track indices and exactly one track per local file. ",
            "Use the per-track filename field to infer titles when the existing title is missing or garbled. ",
            "Strip year and format annotations from album names. Preserve uncertain fields as null. ",
            "Use Various Artists only for true compilations. Number tracks and discs consistently. ",
            "Do not invent provider IDs. Genre should use conservative Discogs-style comma-separated tags."
        )),
        ChatMessage::user(payload.to_string()),
    ];
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "artist": {"type": ["string", "null"]},
            "artists": {"type": "array", "items": {"type": "string"}},
            "albumArtist": {"type": ["string", "null"]},
            "albumArtists": {"type": "array", "items": {"type": "string"}},
            "album": {"type": ["string", "null"]},
            "year": {"type": ["string", "null"]},
            "genre": {"type": ["string", "null"]},
            "tracks": {"type": "array", "items": {"type": "object", "properties": {
                "index": {"type": "number"},
                "title": {"type": ["string", "null"]},
                "artist": {"type": ["string", "null"]},
                "artists": {"type": "array", "items": {"type": "string"}},
                "trackNumber": {"type": "number"},
                "trackTotal": {"type": "number"},
                "discNumber": {"type": "number"},
                "discTotal": {"type": "number"}
            }, "required": ["index", "title", "artist", "artists", "trackNumber", "trackTotal", "discNumber", "discTotal"]}},
            "confidence": {"type": "number"}
        },
        "required": ["artist", "artists", "albumArtist", "albumArtists", "album", "year", "genre", "tracks", "confidence"]
    });
    tracing::debug!(model, "calling auto-tag LLM");
    let api_key = api_key.expect("API key checked above");
    let llm_endpoint = crate::infra::openrouter::LlmEndpoint::from_config(
        config.llm_provider.as_deref(),
        config.llm_base_url.as_deref(),
    );
    let result = OpenRouterClient::at(api_key, model, &llm_endpoint.base_url)
        .with_provider(llm_endpoint.provider)
        .with_generation(0.0, auto_tag_llm_max_tokens(request.tracks.len()))
        .with_timeout(AUTO_TAG_LLM_TIMEOUT)
        .complete_json(messages, "TagCorrectionResponse", schema, cancelled)
        .await;
    let response = result.map_err(|error| {
        tracing::warn!(error_code = error.diagnostic_code(), "auto-tag LLM failed");
        ai_validation_failure_from_error(&error)
    })?;
    tracing::debug!("auto-tag LLM succeeded");
    validated_ai_candidate(request, &response.data)
}

async fn fill_genre_if_missing(
    candidate: &AlbumCandidate,
    request: &LookupRequest,
    config: &AutoTagConfig,
    cancelled: &AtomicBool,
) -> (AlbumCandidate, Option<GenreFillOutcome>) {
    if candidate.genre.is_some() {
        return (candidate.clone(), None);
    }
    let Some(api_key) = config.llm_api_key.as_deref().filter(|key| !key.is_empty()) else {
        return (candidate.clone(), None);
    };
    let model = config
        .llm_model
        .as_deref()
        .filter(|model| !model.is_empty())
        .unwrap_or("deepseek/deepseek-chat");
    let payload = serde_json::json!({
        "artist": candidate.artist.as_ref().or(request.artist_hint.as_ref()),
        "album": candidate.album.as_ref().or(request.album_hint.as_ref()),
        "tracks": candidate.tracks.iter().filter_map(|track| track.title.as_ref()).collect::<Vec<_>>(),
    });
    let messages = vec![
        ChatMessage::system(concat!(
            "Infer a conservative music genre from the supplied artist, album, and track titles. ",
            "Return a concise Discogs-style comma-separated genre and confidence. ",
            "Use low confidence when the evidence is insufficient."
        )),
        ChatMessage::user(payload.to_string()),
    ];
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "genre": {"type": "string"},
            "confidence": {"type": "number"}
        },
        "required": ["genre", "confidence"]
    });
    let endpoint = crate::infra::openrouter::LlmEndpoint::from_config(
        config.llm_provider.as_deref(),
        config.llm_base_url.as_deref(),
    );
    let result = OpenRouterClient::at(api_key, model, &endpoint.base_url)
        .with_provider(endpoint.provider)
        .with_generation(0.2, 1024)
        .complete_json(messages, "GenreFillResponse", schema, cancelled)
        .await
        .map(|response| response.data)
        .map_err(|error| {
            tracing::warn!(error = %error, "auto-tag genre fill failed");
            ai_validation_failure_from_error(&error).detail
        });
    let outcome = genre_fill_outcome(result);
    let mut filled = candidate.clone();
    if let GenreFillOutcome::Applied(genre) = &outcome {
        filled.genre = Some(genre.clone());
    }
    (filled, Some(outcome))
}

/// Reuse auto-tag's conservative genre fill for a manually resolved release.
/// The manual workflow has no task cancellation token, so this bounded helper
/// uses a request-local token and returns the unchanged candidate when genre
/// inference is unavailable or rejected.
pub(crate) async fn fill_manual_candidate_genre_if_missing(
    candidate: &AlbumCandidate,
    config: &AutoTagConfig,
) -> AlbumCandidate {
    let request = LookupRequest {
        artist_hint: candidate.artist.clone(),
        album_hint: candidate.album.clone(),
        year_hint: candidate.year.clone(),
        musicbrainz_album_id: candidate.musicbrainz_album_id.clone(),
        musicbrainz_artist_id: candidate.musicbrainz_artist_id.clone(),
        discogs_release_id: candidate.discogs_release_id.clone(),
        discogs_artist_id: candidate.discogs_artist_id.clone(),
        tracks: candidate.tracks.clone(),
        ..LookupRequest::default()
    };
    fill_genre_if_missing(candidate, &request, config, &AtomicBool::new(false))
        .await
        .0
}

pub async fn resolve_and_apply_album(
    album_path: &Path,
    config: &AutoTagConfig,
    services: AutoTagServices<'_>,
    cancelled: &AtomicBool,
    progress: impl FnMut(u64, &str),
    report: impl FnMut(&'static str, String, Option<serde_json::Value>),
) -> Result<AutoTagRunResult, ApiError> {
    resolve_and_apply_album_with_retry_context(
        album_path, config, services, cancelled, None, progress, report,
    )
    .await
}

pub(crate) async fn resolve_and_apply_album_with_retry_context(
    album_path: &Path,
    config: &AutoTagConfig,
    services: AutoTagServices<'_>,
    cancelled: &AtomicBool,
    retry_contexts: Option<AutoTagRetryContexts>,
    mut progress: impl FnMut(u64, &str),
    mut report: impl FnMut(&'static str, String, Option<serde_json::Value>),
) -> Result<AutoTagRunResult, ApiError> {
    progress(1, "Parsing folder hints...");
    let mut request = build_lookup_request(album_path)?;
    check_cancelled(cancelled)?;

    progress(3, "Checking cache...");
    let hash = query_hash(&request);
    let cached = services
        .cache
        .lookup(&hash)
        .and_then(|value| serde_json::from_value::<Vec<AlbumCandidate>>(value).ok())
        .unwrap_or_default();
    if !cached.is_empty() {
        report(
            "source",
            format!("Cache: {} candidate(s)", cached.len()),
            Some(serde_json::json!({"source": "cache", "count": cached.len()})),
        );
    }
    let mut fresh = Vec::new();
    let mut provider_attempts = Vec::new();

    progress(4, "Direct provider ID lookup...");
    let musicbrainz_metrics = retry_contexts
        .as_ref()
        .map(|contexts| contexts.musicbrainz.metrics())
        .unwrap_or_default();
    let discogs_metrics = retry_contexts
        .as_ref()
        .map(|contexts| contexts.discogs.metrics())
        .unwrap_or_default();
    let musicbrainz = retry_contexts
        .as_ref()
        .map(|contexts| {
            MusicBrainzClient::new(services.providers.http())
                .with_retry_context(contexts.musicbrainz.clone())
        })
        .unwrap_or_else(|| MusicBrainzClient::new(services.providers.http()));
    let mut musicbrainz_direct_error = None;
    let mut musicbrainz_error = None;
    if config.remote_lookup_enabled != Some(false) {
        if let Some(release_id) = request.musicbrainz_album_id.as_deref() {
            match musicbrainz.release_by_id_result(release_id).await {
                Ok(album) => fresh.push(musicbrainz_candidate(album)),
                Err(error) => musicbrainz_direct_error = Some(safe_provider_error(&error)),
            }
        }
    }
    let discogs = retry_contexts
        .as_ref()
        .map(|contexts| {
            DiscogsClient::new(services.providers.http(), config.discogs_token.clone())
                .with_retry_context(contexts.discogs.clone())
        })
        .unwrap_or_else(|| {
            DiscogsClient::new(services.providers.http(), config.discogs_token.clone())
        });
    let mut discogs_direct_error = None;
    let mut discogs_error = None;
    if config.discogs_enabled != Some(false) {
        if let Some(release_id) = request.discogs_release_id.as_deref() {
            match discogs.release_metadata_result(release_id).await {
                Ok(album) => fresh.push(discogs_candidate(album)),
                Err(error) => discogs_direct_error = Some(safe_provider_error(&error)),
            }
        }
    }
    if !fresh.is_empty() {
        report(
            "source",
            format!("Direct ID lookup: {} candidate(s)", fresh.len()),
            Some(serde_json::json!({"source": "direct-id", "count": fresh.len()})),
        );
    }
    check_cancelled(cancelled)?;

    let needs_musicbrainz_identity =
        request.musicbrainz_artist_id.is_none() && config.remote_lookup_enabled != Some(false);
    let needs_discogs_identity =
        request.discogs_artist_id.is_none() && config.discogs_enabled != Some(false);
    let resolved_identity = if (needs_musicbrainz_identity || needs_discogs_identity)
        && request
            .artist_hint
            .as_deref()
            .is_some_and(|artist| !is_compilation_folder(artist))
    {
        let artist = request.artist_hint.as_deref().unwrap_or_default();
        let resolution = services
            .providers
            .resolve_artist_identity_result_with_context(
                artist,
                config.discogs_token.clone(),
                needs_musicbrainz_identity,
                needs_discogs_identity,
                retry_contexts
                    .as_ref()
                    .map(|contexts| contexts.musicbrainz.clone()),
                retry_contexts
                    .as_ref()
                    .map(|contexts| contexts.discogs.clone()),
            )
            .await;
        musicbrainz_error = resolution
            .musicbrainz_error
            .as_deref()
            .map(safe_provider_error);
        discogs_error = resolution
            .discogs_error
            .as_deref()
            .map(safe_provider_error);
        for alias in &resolution.identity.english_aliases {
            if let Err(error) = save_alias(services.alias_file, artist, alias) {
                tracing::warn!(%error, "failed to persist resolved artist alias");
            }
        }
        fill_request_artist_identity(&mut request, &resolution.identity);
        Some(resolution.identity)
    } else {
        None
    };
    check_cancelled(cancelled)?;

    if config.remote_lookup_enabled != Some(false) {
        progress(5, "Searching MusicBrainz...");
        let mut provider_candidates = cached
            .iter()
            .filter(|candidate| candidate.source == LookupSource::Musicbrainz)
            .cloned()
            .chain(
                fresh
                    .iter()
                    .filter(|candidate| candidate.source == LookupSource::Musicbrainz)
                    .cloned(),
            )
            .collect::<Vec<_>>();
        match musicbrainz_artist_candidates(&musicbrainz, services.cache, &request).await {
            Ok(scoped) => provider_candidates.extend(scoped),
            Err(error) => musicbrainz_error = Some(safe_provider_error(&error)),
        }
        let mut credible = select_credible_provider_candidate(&request, provider_candidates.clone());
        let mut generic_search_attempted = false;
        if credible.is_none() {
            if let (Some(artist), Some(album)) = (
                request.artist_hint.as_deref(),
                request.album_hint.as_deref(),
            ) {
                generic_search_attempted = true;
                match musicbrainz.search_album_result(artist, album, 10).await {
                    Ok(albums) => {
                        provider_candidates.extend(albums.into_iter().map(musicbrainz_candidate));
                    }
                    Err(error) => musicbrainz_error = Some(safe_provider_error(&error)),
                }
            }
            credible = select_credible_provider_candidate(&request, provider_candidates.clone());
        }
        let count = provider_candidates.len();
        fresh.extend(provider_candidates);
        let diagnostic = if generic_search_attempted {
            musicbrainz_error.clone()
        } else {
            musicbrainz_error
                .clone()
                .or_else(|| musicbrainz_direct_error.clone())
        };
        let status = if credible.is_some() {
            ProviderAttemptStatus::Matched
        } else if diagnostic.is_some() {
            ProviderAttemptStatus::Unavailable
        } else {
            ProviderAttemptStatus::NoMatch
        };
        provider_attempts.push(ProviderAttempt {
            provider: "musicbrainz",
            status,
            diagnostic,
            retry_count: musicbrainz_metrics.retry_count(),
            retry_after_seconds: musicbrainz_metrics.max_retry_after_seconds(),
        });
        report(
            "source",
            format!("MusicBrainz: {count} candidate(s)"),
            Some(serde_json::json!({"source": "musicbrainz", "count": count, "status": status})),
        );
    }
    check_cancelled(cancelled)?;

    if config.discogs_enabled != Some(false) {
        progress(6, "Searching Discogs releases...");
        let mut provider_candidates = cached
            .iter()
            .filter(|candidate| candidate.source == LookupSource::Discogs)
            .cloned()
            .chain(
                fresh
                    .iter()
                    .filter(|candidate| candidate.source == LookupSource::Discogs)
                    .cloned(),
            )
            .collect::<Vec<_>>();
        match discogs_artist_candidates(&discogs, services.cache, &request).await {
            Ok(scoped) => provider_candidates.extend(scoped),
            Err(error) => discogs_error = Some(safe_provider_error(&error)),
        }
        let mut credible = select_credible_provider_candidate(&request, provider_candidates.clone());
        let mut generic_search_attempted = false;
        if credible.is_none() {
            if let (Some(artist), Some(album)) = (
                request.artist_hint.as_deref(),
                request.album_hint.as_deref(),
            ) {
                generic_search_attempted = true;
                match discogs
                    .search_album_result_with_context(
                        artist,
                        album,
                        request.year_hint.as_deref(),
                        request.country_hint.as_deref().and_then(discogs_country_name),
                        None,
                        10,
                    )
                    .await
                {
                    Ok(albums) => {
                        provider_candidates.extend(albums.into_iter().map(discogs_candidate));
                    }
                    Err(error) => discogs_error = Some(safe_provider_error(&error)),
                }
            }
            credible = select_credible_provider_candidate(&request, provider_candidates.clone());
        }
        let count = provider_candidates.len();
        fresh.extend(provider_candidates);
        let diagnostic = if generic_search_attempted {
            discogs_error.clone()
        } else {
            discogs_error.clone().or_else(|| discogs_direct_error.clone())
        };
        let status = if credible.is_some() {
            ProviderAttemptStatus::Matched
        } else if diagnostic.is_some() {
            ProviderAttemptStatus::Unavailable
        } else {
            ProviderAttemptStatus::NoMatch
        };
        provider_attempts.push(ProviderAttempt {
            provider: "discogs",
            status,
            diagnostic,
            retry_count: discogs_metrics.retry_count(),
            retry_after_seconds: discogs_metrics.max_retry_after_seconds(),
        });
        report(
            "source",
            format!("Discogs releases: {count} candidate(s)"),
            Some(serde_json::json!({"source": "discogs", "count": count, "status": status})),
        );
    }
    check_cancelled(cancelled)?;

    progress(7, "Selecting authoritative metadata...");
    let provider_decision =
        provider_authority_decision(&request, fresh.clone(), &provider_attempts);
    let provider_diagnostics =
        provider_selection_diagnostics(&request, &fresh, &provider_attempts);
    let providers_confirmed_no_match =
        provider_decision == ProviderAuthorityDecision::Providerless;
    let mut candidate = match provider_decision {
        ProviderAuthorityDecision::Selected(candidate) => Some(*candidate),
        ProviderAuthorityDecision::Providerless | ProviderAuthorityDecision::Unavailable => None,
    };
    let mut ai_failure = None;
    if candidate.is_none() && providers_confirmed_no_match {
        match resolve_tags_via_llm(&request, config, cancelled).await {
            Ok(ai_candidate) => candidate = Some(ai_candidate),
            Err(error) => ai_failure = Some(error),
        }
        check_cancelled(cancelled)?;
    }

    let Some(mut candidate) = candidate else {
        let (reason_code, reason, ai_status, ai_confidence) = if provider_attempts
            .iter()
            .any(|attempt| attempt.status == ProviderAttemptStatus::Unavailable)
        {
            (
                "provider_unavailable",
                "no credible provider release and a provider was unavailable".to_string(),
                None,
                None,
            )
        } else if let Some(error) = ai_failure {
            (
                error.code,
                error.detail,
                Some(error.code.to_string()),
                error.confidence,
            )
        } else {
            (
                "ai_validation_failed",
                "no authoritative metadata candidate was available".to_string(),
                Some("ai_validation_failed".to_string()),
                None,
            )
        };
        progress(9, "Needs review — no authoritative metadata match");
        report(
            "needs_review",
            format!("Needs review: {reason}"),
            Some(serde_json::json!({"reasonCode": reason_code})),
        );
        return Ok(AutoTagRunResult {
            outcome: AutoTagOutcome::NeedsReview,
            authority: None,
            candidate: None,
            written: 0,
            reason_code: Some(reason_code.to_string()),
            diagnostics: provider_diagnostics
                .into_iter()
                .chain(std::iter::once(serde_json::json!({"reason": reason})))
                .collect(),
            provider_attempts,
            ai_status,
            ai_confidence,
            ai_threshold: Some(AI_TAG_CONFIDENCE_THRESHOLD),
        });
    };

    let cache_payload = fresh
        .iter()
        .filter(|item| matches!(item.source, LookupSource::Musicbrainz | LookupSource::Discogs))
        .cloned()
        .collect::<Vec<_>>();
    if should_replace_lookup_cache(&cache_payload, !cached.is_empty()) {
        if let (Ok(query), Ok(response)) = (
            serde_json::to_value(&request),
            serde_json::to_value(&cache_payload),
        ) {
            let source = cache_payload
                .first()
                .map(|candidate| candidate.source)
                .unwrap_or(LookupSource::Folder);
            let _ = services.cache.set_lookup(
                &hash,
                &query,
                &response,
                lookup_source_name(source),
            );
        }
    }
    candidate = protect_candidate_tracks(&request, &candidate);
    report(
        "source",
        format!("Selected authoritative {} candidate", lookup_source_name(candidate.source)),
        Some(serde_json::json!({"source": lookup_source_name(candidate.source)})),
    );
    check_cancelled(cancelled)?;
    progress(8, "Resolving genre...");
    if let Some(identity) = &resolved_identity {
        fill_candidate_artist_identity(&mut candidate, identity);
    }
    let (candidate, genre_outcome) = if candidate.source == LookupSource::Llm {
        (candidate, None)
    } else {
        fill_genre_if_missing(&candidate, &request, config, cancelled).await
    };
    match genre_outcome {
        Some(GenreFillOutcome::Applied(genre)) => report(
            "source",
            format!("Genre inferred by LLM: {genre}"),
            Some(serde_json::json!({"source": "llm", "genre": genre})),
        ),
        Some(GenreFillOutcome::Rejected { genre, confidence }) => {
            tracing::warn!(
                genre = ?genre,
                confidence = ?confidence,
                threshold = 0.6,
                "auto-tag genre fill rejected"
            );
            report(
                "warning",
                "Genre remains missing: LLM response was empty or below the confidence threshold"
                    .to_string(),
                Some(serde_json::json!({
                    "source": "llm",
                    "genre": genre,
                    "confidence": confidence,
                    "threshold": 0.6
                })),
            );
        }
        Some(GenreFillOutcome::Failed(error)) => {
            tracing::warn!(%error, "auto-tag genre fill failed");
            report(
                "warning",
                format!("Genre remains missing: {error}"),
                Some(serde_json::json!({"source": "llm", "error": error})),
            );
        }
        None => {}
    }
    report(
        "source",
        format!(
            "Selected {} candidate",
            lookup_source_name(candidate.source)
        ),
        Some(serde_json::json!({"source": lookup_source_name(candidate.source)})),
    );
    check_cancelled(cancelled)?;
    progress(9, "Applying tags...");
    let candidate = convert_candidate_chinese(&candidate, config.chinese_script.as_deref());

    // Fetch lyrics before writing tags so both can be written in one pass,
    // eliminating a separate file rewrite on the lyrics pass.
    let lyrics_url = if config.lyrics_download_enabled == Some(true) {
        Some(config.lyrics_api_url.as_deref().unwrap_or(DEFAULT_BASE_URL))
    } else {
        None
    };
    let fetched_lyrics = fetch_album_lyrics(album_path, lyrics_url).await;
    if !fetched_lyrics.is_empty() {
        report(
            "source",
            format!("Fetched lyrics for {} track(s)", fetched_lyrics.len()),
            Some(serde_json::json!({"source": "lyrics", "count": fetched_lyrics.len()})),
        );
    }

    let written = apply_candidate_tags_reported(
        album_path,
        &candidate,
        services.queue,
        CandidateApplyScope::AllTracks,
        fetched_lyrics.into_iter().collect::<HashMap<_, _>>(),
        |path| {
            let path = Path::new(path);
            report(
                "write",
                format!(
                    "Wrote tags: {}",
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default()
                ),
                Some(serde_json::json!({"path": path.to_string_lossy()})),
            );
        },
    )
    .await?;

    if config.remote_lookup_enabled != Some(false)
        || config.discogs_enabled != Some(false)
        || config.theaudiodb_api_key.is_some()
    {
        let remote = RemoteArtworkClient::new(
            services.providers.http(),
            config.discogs_token.clone(),
            config.theaudiodb_api_key.clone(),
        );
        if let Some((_bytes, source, path)) =
            download_album_artwork_at(album_path, &remote, services.queue).await
        {
            report(
                "source",
                format!("Cover art: {}", path.display()),
                Some(serde_json::json!({
                    "source": "cover",
                    "provider": source,
                    "path": path.to_string_lossy()
                })),
            );
        }
        if let Some((_bytes, source, path)) =
            download_artist_artwork_at(album_path, &remote, services.queue).await
        {
            report(
                "source",
                format!("Artist art: {}", path.display()),
                Some(serde_json::json!({
                    "source": "artist",
                    "provider": source,
                    "path": path.to_string_lossy()
                })),
            );
        }
    }
    check_cancelled(cancelled)?;
    let ai_status = (candidate.source == LookupSource::Llm).then(|| "accepted".to_string());
    let ai_confidence = candidate.confidence;
    let ai_threshold = ai_status
        .as_ref()
        .map(|_| AI_TAG_CONFIDENCE_THRESHOLD);
    Ok(AutoTagRunResult {
        outcome: AutoTagOutcome::Applied,
        authority: Some(candidate.source),
        candidate: Some(candidate),
        written,
        reason_code: None,
        diagnostics: provider_diagnostics,
        provider_attempts,
        ai_status,
        ai_confidence,
        ai_threshold,
    })
}

#[tauri::command]
pub fn album_auto_tag(
    album_path: String,
    app: AppHandle,
    tasks: State<'_, TaskRegistry>,
) -> Result<String, ApiError> {
    if !Path::new(&album_path).is_dir() {
        return Err(ApiError::Message(format!(
            "Album directory does not exist: {album_path}"
        )));
    }
    let task_id = tasks.create("auto-tag", 9, "Starting...");
    let spawned_task_id = task_id.clone();
    tauri::async_runtime::spawn(async move {
        let path = PathBuf::from(album_path);
        let tasks = app.state::<TaskRegistry>();
        let Some(cancelled) = tasks.cancellation(&spawned_task_id) else {
            return;
        };
        let config_state = app.state::<crate::state::config::ConfigState>();
        let alias_file = config_state.alias_file_path();
        let config = config_state.raw();
        let providers = app.state::<ProviderState>();
        let cache = app.state::<CacheState>();
        let queue = app.state::<WriteQueue>();
        let progress_app = app.clone();
        let progress_task_id = spawned_task_id.clone();
        let report_app = app.clone();
        let report_task_id = spawned_task_id.clone();
        let operation = resolve_and_apply_album_with_retry_context(
            &path,
            &config,
            AutoTagServices {
                providers: &providers,
                cache: &cache,
                queue: &queue,
                alias_file: &alias_file,
            },
            &cancelled,
            Some(AutoTagRetryContexts::new(Arc::clone(&cancelled))),
            move |step, message| {
                let tasks = progress_app.state::<TaskRegistry>();
                if tasks.update(&progress_task_id, step, message) {
                    let _ = progress_app.emit(
                        "auto-tag:event",
                        auto_tag_event(&progress_task_id, "progress", message, step, None),
                    );
                }
            },
            move |kind, message, data| {
                let progress = report_app
                    .state::<TaskRegistry>()
                    .get(&report_task_id)
                    .map(|task| task.progress)
                    .unwrap_or(0);
                let _ = report_app.emit(
                    "auto-tag:event",
                    auto_tag_event(&report_task_id, kind, message, progress, data),
                );
            },
        )
        .await;

        match operation {
            Ok(result) => {
                let data = serde_json::to_value(&result).unwrap_or_default();
                match result.outcome {
                    AutoTagOutcome::Applied => {
                        let candidate = result
                            .candidate
                            .as_ref()
                            .expect("applied auto-tag result has a candidate");
                        let message = auto_tag_completion_message(candidate);
                        tasks.finish(
                            &spawned_task_id,
                            TaskStatus::Completed,
                            message,
                            data.clone(),
                        );
                        let _ = app.emit(
                            "auto-tag:event",
                            auto_tag_event(&spawned_task_id, "completed", message, 9, Some(data)),
                        );
                    }
                    AutoTagOutcome::NeedsReview => {
                        let message = format!(
                            "Needs review — {}",
                            result.reason_code.as_deref().unwrap_or("no authoritative match")
                        );
                        tasks.finish(
                            &spawned_task_id,
                            TaskStatus::NeedsReview,
                            &message,
                            data.clone(),
                        );
                        let _ = app.emit(
                            "auto-tag:event",
                            auto_tag_event(
                                &spawned_task_id,
                                "needs_review",
                                message,
                                9,
                                Some(data),
                            ),
                        );
                    }
                }
            }
            Err(error) if cancelled.load(Ordering::Acquire) => {
                let progress = tasks
                    .get(&spawned_task_id)
                    .map(|task| task.progress)
                    .unwrap_or(0);
                tasks.finish(
                    &spawned_task_id,
                    TaskStatus::Cancelled,
                    "Cancelled",
                    serde_json::Value::Null,
                );
                let _ = app.emit(
                    "auto-tag:event",
                    auto_tag_event(&spawned_task_id, "cancelled", "Cancelled", progress, None),
                );
                tracing::debug!(%error, "auto-tag task cancelled");
            }
            Err(error) => {
                let message = error.to_string();
                let data = serde_json::json!({"error": message});
                tasks.finish(&spawned_task_id, TaskStatus::Failed, &message, data.clone());
                let _ = app.emit(
                    "auto-tag:event",
                    auto_tag_event(&spawned_task_id, "failed", message, 0, Some(data)),
                );
            }
        }
    });
    Ok(task_id)
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), ApiError> {
    if cancelled.load(Ordering::Acquire) {
        Err(ApiError::Message("Cancelled".to_string()))
    } else {
        Ok(())
    }
}

fn lookup_source_name(source: LookupSource) -> &'static str {
    match source {
        LookupSource::Beets => "beets",
        LookupSource::Dataset => "dataset",
        LookupSource::Discogs => "discogs",
        LookupSource::Folder => "folder",
        LookupSource::Llm => "llm",
        LookupSource::Musicbrainz => "musicbrainz",
    }
}

pub async fn apply_candidate_tags(
    album_path: &Path,
    candidate: &AlbumCandidate,
    queue: &WriteQueue,
) -> Result<usize, ApiError> {
    apply_candidate_tags_reported(
        album_path,
        candidate,
        queue,
        CandidateApplyScope::AllTracks,
        HashMap::new(),
        |_| {},
    )
    .await
}

/// Apply only explicitly selected positional candidate rows. Selection is
/// independent of field content so an intentionally selected empty row can
/// still receive album metadata while unselected files remain untouched.
pub async fn apply_selected_candidate_tags(
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

#[derive(Clone, Copy)]
enum CandidateApplyScope<'a> {
    AllTracks,
    SelectedTracks(&'a HashSet<usize>),
}

impl CandidateApplyScope<'_> {
    fn includes(self, index: usize) -> bool {
        match self {
            Self::AllTracks => true,
            Self::SelectedTracks(indices) => indices.contains(&index),
        }
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

async fn apply_candidate_tags_reported(
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
    // Only write year when the candidate has one; a missing year must not
    // remove an existing YEAR tag (insert_option would emit `null` → Patch::Null).
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
            // In the all-track auto-tag scope, an empty candidate row still
            // receives album fields but leaves its per-track tags untouched.
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
                } else if candidate.source == LookupSource::Llm {
                    fields.insert("musicbrainzTrackId".into(), serde_json::Value::Null);
                }
            }
        }
        // Include lyrics in the same write pass, avoiding a separate file rewrite.
        if let Some(lyrics) = lyrics_map.get(Path::new(&file_path)) {
            fields.insert("lyrics".into(), serde_json::json!(lyrics));
        } else {
            // Only omit if no lyrics at all; preserve any existing lyrics field
            // by not inserting a lyrics key (serde default = Patch::Omitted).
        }
        let patch: TrackPatch = serde_json::from_value(fields.into())
            .map_err(|error| ApiError::WriteTask(error.to_string()))?;
        match write_track_queued(queue, file_path.clone().into(), patch).await {
            Ok(()) => {
                written += 1;
                report_write(&file_path);
            }
            Err(error) => {
                tracing::warn!(path = %file_path, %error, "auto-tag write failed");
                failures.push(format!("{file_path}: {error}"));
            }
        }
    }
    if failures.is_empty() {
        Ok(written)
    } else {
        Err(ApiError::WriteTask(format!(
            "auto-tag wrote {written} file(s), but {} file(s) failed: {}",
            failures.len(),
            failures.join("; ")
        )))
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::openrouter::OpenRouterError;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "soundrobe-auto-tag-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn corpus_mp3() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus/minimal.mp3")
    }

    fn corpus_aiff() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus/minimal.aiff")
    }

    fn corpus_flac() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus/minimal.flac")
    }

    fn corpus_wav() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus/minimal.wav")
    }

    fn track(title: &str, artist: &str) -> TrackCandidate {
        TrackCandidate {
            title: Some(title.into()),
            artist: Some(artist.into()),
            artists: vec![artist.into()],
            ..TrackCandidate::default()
        }
    }

    fn apply_verified_for_test(
        request: &LookupRequest,
        candidate: AlbumCandidate,
    ) -> AlbumCandidate {
        let identity = provider_artist_identity(&candidate);
        apply_verified_canonical_artist_name_with_provenance(request, candidate, identity)
    }

    fn scoped_detail_failure_server(provider: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 4096];
                let count = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..count]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or_default();
                let (status, body) = if provider == "musicbrainz"
                    && path.starts_with("/release?artist=mb-artist")
                {
                    (
                        "200 OK",
                        r#"{"releases":[{"id":"scoped-release","title":"Album","date":"2000-01-01","artist-credit":[{"name":"Artist"}]}]}"#,
                    )
                } else if provider == "discogs" && path.starts_with("/artists/7/releases?") {
                    (
                        "200 OK",
                        r#"{"releases":[{"id":42,"title":"Album","year":2000,"type":"release"}]}"#,
                    )
                } else {
                    ("503 Service Unavailable", "{}")
                };
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        base
    }

    #[test]
    fn folder_year_reads_year_at_start_of_bracketed_album_title() {
        assert_eq!(
            extract_folder_year("张卫健-《1993-真挚的朋友精选》[WAV 分轨]").as_deref(),
            Some("1993")
        );
    }

    #[test]
    fn folder_year_reads_year_from_supported_paired_title_quotes() {
        for name in [
            "Artist-《1993-Album》",
            "Artist-〈1993-Album〉",
            "Artist-「1993-Album」",
            "Artist-『1993-Album』",
        ] {
            assert_eq!(extract_folder_year(name).as_deref(), Some("1993"));
        }
    }

    #[test]
    fn folder_year_requires_a_closing_title_quote() {
        assert_eq!(extract_folder_year("Album《1993 remaster"), None);
    }

    #[test]
    fn folder_year_requires_the_matching_closing_title_quote() {
        assert_eq!(extract_folder_year("Album《1993 remaster」"), None);
    }

    #[test]
    fn folder_year_does_not_read_parenthesized_remaster_annotation() {
        assert_eq!(extract_folder_year("Album (2004 remaster)"), None);
    }

    #[test]
    fn folder_year_does_not_read_bracketed_mix_annotation() {
        assert_eq!(extract_folder_year("Album [1993 Mix]"), None);
    }

    #[test]
    fn folder_year_keeps_leading_year_support() {
        assert_eq!(
            extract_folder_year("2004 - Folder Album [FLAC]").as_deref(),
            Some("2004")
        );
    }

    #[test]
    fn folder_year_does_not_treat_unbracketed_name_number_as_year() {
        assert_eq!(extract_folder_year("The 1975 [WAV]"), None);
    }

    #[test]
    fn provider_candidate_wins_while_lower_priority_sources_only_fill_gaps() {
        let llm = AlbumCandidate {
            artist: Some("Guessed Artist".into()),
            album: Some("Guessed Album".into()),
            year: Some("2001".into()),
            genre: Some("Pop".into()),
            tracks: vec![track("Guessed Title", "Guessed Artist")],
            source: LookupSource::Llm,
            ..AlbumCandidate::default()
        };
        let provider = AlbumCandidate {
            artist: Some("Canonical Artist".into()),
            album: Some("Canonical Album".into()),
            musicbrainz_album_id: Some("release-id".into()),
            tracks: vec![track("Canonical Title", "Canonical Artist")],
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };

        let merged = merge_candidate_fields(vec![llm.clone(), provider]);

        assert_eq!(merged[0].artist.as_deref(), Some("Canonical Artist"));
        assert_eq!(merged[0].album.as_deref(), Some("Canonical Album"));
        assert_eq!(merged[0].year.as_deref(), Some("2001"));
        assert_eq!(merged[0].genre.as_deref(), Some("Pop"));
        assert_eq!(
            merged[0].tracks[0].title.as_deref(),
            Some("Canonical Title")
        );
        assert_eq!(merged[1], llm);
    }

    #[test]
    fn accepted_provider_year_fills_gap_before_folder_year() {
        let preferred = AlbumCandidate {
            album: Some("Album".into()),
            year: None,
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };
        let cached = AlbumCandidate {
            album: Some("Album".into()),
            year: Some("1994".into()),
            source: LookupSource::Discogs,
            ..AlbumCandidate::default()
        };
        let folder = AlbumCandidate {
            album: Some("Album".into()),
            year: Some("1993".into()),
            source: LookupSource::Folder,
            ..AlbumCandidate::default()
        };

        let merged = combine_candidate_sources(vec![preferred], vec![cached], folder);

        assert_eq!(merged[0].source, LookupSource::Musicbrainz);
        assert_eq!(merged[0].year.as_deref(), Some("1994"));
    }

    #[test]
    fn canonical_artist_preserves_featured_track_credit() {
        let candidate = AlbumCandidate {
            artist: Some("林俊傑".into()),
            album_artist: Some("林俊傑".into()),
            tracks: vec![
                track("Solo", "林俊傑"),
                track("Featured", "林俊傑 feat. MC HotDog"),
            ],
            ..AlbumCandidate::default()
        };

        let updated = apply_canonical_artist_name(candidate, Some("JJ Lin (123)"));

        assert_eq!(updated.artist.as_deref(), Some("JJ Lin"));
        assert_eq!(updated.tracks[0].artist.as_deref(), Some("JJ Lin"));
        assert_eq!(
            updated.tracks[1].artist.as_deref(),
            Some("林俊傑 feat. MC HotDog")
        );
    }

    #[test]
    fn verified_canonical_artist_requires_the_same_provider_identity() {
        let request = LookupRequest {
            artist_hint: Some("郑少秋".into()),
            discogs_artist_id: Some("3156508".into()),
            ..LookupRequest::default()
        };
        let candidate = AlbumCandidate {
            artist: Some("Different Artist".into()),
            artists: vec!["Different Artist".into()],
            discogs_artist_id: Some("999".into()),
            source: LookupSource::Discogs,
            tracks: vec![track("Song", "Different Artist")],
            ..AlbumCandidate::default()
        };

        let updated = apply_verified_for_test(&request, candidate.clone());

        assert_eq!(updated, candidate);
    }

    #[test]
    fn verified_canonical_artist_preserves_genuine_collaborators() {
        let request = LookupRequest {
            artist_hint: Some("郑少秋".into()),
            discogs_artist_id: Some("3156508".into()),
            ..LookupRequest::default()
        };
        let candidate = AlbumCandidate {
            artist: Some("Adam Cheng".into()),
            artists: vec!["Adam Cheng".into()],
            discogs_artist_id: Some("3156508".into()),
            source: LookupSource::Discogs,
            tracks: vec![TrackCandidate {
                title: Some("Duet".into()),
                artist: Some("Adam Cheng & Liza Wang".into()),
                artists: vec!["Adam Cheng".into(), "Liza Wang".into()],
                ..TrackCandidate::default()
            }],
            ..AlbumCandidate::default()
        };

        let updated = apply_verified_for_test(&request, candidate);

        assert_eq!(
            updated.tracks[0].artist.as_deref(),
            Some("Adam Cheng & Liza Wang")
        );
        assert_eq!(updated.tracks[0].artists, vec!["Adam Cheng", "Liza Wang"]);
    }

    #[test]
    fn canonical_artist_rewrites_album_alias_without_collapsing_collaborators() {
        let candidate = AlbumCandidate {
            artist: Some("Adam Cheng & Liza Wang".into()),
            artists: vec!["Adam Cheng".into(), "Liza Wang".into()],
            album_artist: Some("Adam Cheng & Liza Wang".into()),
            album_artists: vec!["Adam Cheng".into(), "Liza Wang".into()],
            ..AlbumCandidate::default()
        };

        let updated = apply_canonical_artist_name(candidate, Some("郑少秋"));

        assert_eq!(updated.artist.as_deref(), Some("郑少秋 & Liza Wang"));
        assert_eq!(updated.artists, vec!["郑少秋", "Liza Wang"]);
        assert_eq!(updated.album_artist.as_deref(), Some("郑少秋 & Liza Wang"));
        assert_eq!(updated.album_artists, vec!["郑少秋", "Liza Wang"]);
    }

    #[test]
    fn canonical_artist_replaces_only_the_exact_structured_identity() {
        let candidate = AlbumCandidate {
            artist: Some("王菲 & 王菲菲".into()),
            artists: vec!["王菲".into(), "王菲菲".into()],
            album_artist: Some("王菲 & 王菲菲".into()),
            album_artists: vec!["王菲".into(), "王菲菲".into()],
            ..AlbumCandidate::default()
        };

        let updated = apply_canonical_artist_name(candidate, Some("Faye Wong"));

        assert_eq!(updated.artist.as_deref(), Some("Faye Wong & 王菲菲"));
        assert_eq!(updated.artists, vec!["Faye Wong", "王菲菲"]);
        assert_eq!(updated.album_artist.as_deref(), Some("Faye Wong & 王菲菲"));
        assert_eq!(updated.album_artists, vec!["Faye Wong", "王菲菲"]);
    }

    #[test]
    fn canonical_artist_preserves_genuine_punctuation_artist_identity() {
        let candidate = AlbumCandidate {
            artist: Some("Adam Cheng".into()),
            artists: vec!["Adam Cheng".into()],
            tracks: vec![TrackCandidate {
                artist: Some("Adam Cheng".into()),
                artists: vec!["Adam Cheng".into(), "!!!".into()],
                ..TrackCandidate::default()
            }],
            ..AlbumCandidate::default()
        };

        let updated = apply_canonical_artist_name(candidate, Some("郑少秋"));

        assert_eq!(updated.tracks[0].artist.as_deref(), Some("Adam Cheng"));
        assert_eq!(updated.tracks[0].artists, vec!["Adam Cheng", "!!!"]);
    }

    #[test]
    fn selected_candidate_finalization_is_equivalent_for_fresh_and_cached_remote_rows() {
        let request = LookupRequest {
            artist_hint: Some("郑少秋".into()),
            discogs_artist_id: Some("3156508".into()),
            ..LookupRequest::default()
        };
        let remote = AlbumCandidate {
            artist: Some("Adam Cheng".into()),
            artists: vec!["Adam Cheng".into()],
            discogs_artist_id: Some("3156508".into()),
            source: LookupSource::Discogs,
            ..AlbumCandidate::default()
        };

        let fresh = select_protect_and_canonicalize_candidate(
            &request,
            vec![remote.clone()],
            vec![],
            folder_candidate(&request),
        )
        .unwrap();
        let cached = select_protect_and_canonicalize_candidate(
            &request,
            vec![],
            vec![remote],
            folder_candidate(&request),
        )
        .unwrap();

        assert_eq!(fresh.0.artist.as_deref(), Some("郑少秋"));
        assert_eq!(cached.0.artist.as_deref(), Some("郑少秋"));
    }

    #[test]
    fn selected_candidate_finalization_accepts_musicbrainz_same_id() {
        let request = LookupRequest {
            artist_hint: Some("張宇".into()),
            musicbrainz_artist_id: Some("mb-artist".into()),
            ..LookupRequest::default()
        };
        let remote = AlbumCandidate {
            artist: Some("Phil Chang".into()),
            artists: vec!["Phil Chang".into()],
            musicbrainz_artist_id: Some("mb-artist".into()),
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };

        let finalized = select_protect_and_canonicalize_candidate(
            &request,
            vec![remote],
            vec![],
            folder_candidate(&request),
        )
        .unwrap();

        assert_eq!(finalized.0.artist.as_deref(), Some("張宇"));
    }

    #[test]
    fn inherited_folder_provider_id_does_not_authorize_canonicalization() {
        let request = LookupRequest {
            artist_hint: Some("郑少秋".into()),
            discogs_artist_id: Some("3156508".into()),
            ..LookupRequest::default()
        };
        let remote_without_own_id = AlbumCandidate {
            artist: Some("Adam Cheng".into()),
            artists: vec!["Adam Cheng".into()],
            source: LookupSource::Discogs,
            ..AlbumCandidate::default()
        };

        let finalized = select_protect_and_canonicalize_candidate(
            &request,
            vec![remote_without_own_id],
            vec![],
            folder_candidate(&request),
        )
        .unwrap();

        assert_eq!(finalized.0.discogs_artist_id.as_deref(), Some("3156508"));
        assert_eq!(finalized.0.artist.as_deref(), Some("Adam Cheng"));
    }

    #[test]
    fn copied_folder_release_id_does_not_outrank_cached_remote_candidate() {
        let request = LookupRequest {
            artist_hint: Some("郑少秋".into()),
            discogs_release_id: Some("32867382".into()),
            discogs_artist_id: Some("3156508".into()),
            ..LookupRequest::default()
        };
        let cached = AlbumCandidate {
            artist: Some("Adam Cheng".into()),
            artists: vec!["Adam Cheng".into()],
            discogs_release_id: Some("32867382".into()),
            discogs_artist_id: Some("3156508".into()),
            source: LookupSource::Discogs,
            ..AlbumCandidate::default()
        };

        let finalized = select_protect_and_canonicalize_candidate(
            &request,
            vec![],
            vec![cached],
            folder_candidate(&request),
        )
        .unwrap();

        assert_eq!(finalized.0.source, LookupSource::Discogs);
        assert_eq!(finalized.0.artist.as_deref(), Some("郑少秋"));
    }

    #[test]
    fn verified_canonical_artist_does_not_rewrite_a_same_id_latin_request_alias() {
        let request = LookupRequest {
            artist_hint: Some("Adam Cheng".into()),
            discogs_artist_id: Some("3156508".into()),
            ..LookupRequest::default()
        };
        let candidate = AlbumCandidate {
            artist: Some("郑少秋".into()),
            discogs_artist_id: Some("3156508".into()),
            source: LookupSource::Discogs,
            tracks: vec![track("Song", "郑少秋")],
            ..AlbumCandidate::default()
        };

        let updated = apply_verified_for_test(&request, candidate.clone());

        assert_eq!(updated, candidate);
    }

    #[test]
    fn artist_identity_only_fills_missing_provider_ids() {
        let mut request = LookupRequest {
            musicbrainz_artist_id: Some("existing-mb".into()),
            ..LookupRequest::default()
        };
        let identity = crate::state::providers::ArtistIdentity {
            musicbrainz_artist_id: Some("new-mb".into()),
            discogs_artist_id: Some("discogs-7".into()),
            english_aliases: vec!["Alias".into()],
        };

        fill_request_artist_identity(&mut request, &identity);

        assert_eq!(
            request.musicbrainz_artist_id.as_deref(),
            Some("existing-mb")
        );
        assert_eq!(request.discogs_artist_id.as_deref(), Some("discogs-7"));
    }

    #[test]
    fn artist_identity_does_not_enrich_an_authoritative_ai_candidate() {
        let identity = crate::state::providers::ArtistIdentity {
            musicbrainz_artist_id: Some("mb-artist".into()),
            discogs_artist_id: Some("discogs-artist".into()),
            english_aliases: Vec::new(),
        };
        let mut candidate = AlbumCandidate {
            source: LookupSource::Llm,
            ..AlbumCandidate::default()
        };

        fill_candidate_artist_identity(&mut candidate, &identity);

        assert_eq!(candidate.musicbrainz_artist_id, None);
        assert_eq!(candidate.discogs_artist_id, None);
    }

    #[test]
    fn plain_music_folder_does_not_become_a_us_edition() {
        assert_eq!(
            parse_folder_album_evidence("1991 - Music", None).country,
            None
        );
    }

    #[test]
    fn ambiguity_ignores_format_suffix_but_detects_naming_annotations() {
        assert!(!hints_are_ambiguous(
            Some("Album"),
            Some("Artist"),
            "/music/Artist/Album [FLAC]",
            None,
        ));
        assert!(hints_are_ambiguous(
            Some("Album"),
            Some("Artist"),
            "/music/Artist/Album【香港首版】 [FLAC]",
            None,
        ));
        assert!(hints_are_ambiguous(
            Some("2001-Album"),
            Some("Artist"),
            "/music/Artist/2001-Album",
            Some("2001"),
        ));
    }

    #[test]
    fn query_hash_ignores_path_but_separates_year_and_edition_evidence() {
        let request = LookupRequest {
            path: "/one/location".into(),
            artist_hint: Some("Artist".into()),
            album_hint: Some("Album".into()),
            year_hint: Some("2001".into()),
            tracks: vec![TrackCandidate {
                title: Some("Title".into()),
                track_number: Some(1),
                ..TrackCandidate::default()
            }],
            ..LookupRequest::default()
        };
        let mut relocated = request.clone();
        relocated.path = "/another/location".into();

        assert_eq!(query_hash(&request), query_hash(&relocated));
        relocated.year_hint = Some("2025".into());
        assert_ne!(query_hash(&request), query_hash(&relocated));
        relocated.year_hint = request.year_hint.clone();
        relocated.country_hint = Some("JP".into());
        assert_ne!(query_hash(&request), query_hash(&relocated));
        relocated.country_hint = request.country_hint.clone();
        relocated.tracks[0].title = Some("Different".into());
        assert_ne!(query_hash(&request), query_hash(&relocated));
    }

    #[test]
    fn musicbrainz_provider_album_maps_without_losing_track_identity() {
        let candidate = musicbrainz_candidate(ProviderAlbum {
            id: "release-id".into(),
            title: "Album".into(),
            artist: Some("Artist".into()),
            artists: vec!["Artist".into()],
            artist_id: Some("artist-id".into()),
            year: Some("2004".into()),
            genre: None,
            tracks: vec![crate::state::providers::ProviderTrack {
                title: Some("Track".into()),
                match_titles: vec!["Recording title".into()],
                artist: Some("Artist feat. Guest".into()),
                artists: vec!["Artist".into(), "Guest".into()],
                track_number: Some(1),
                track_total: None,
                disc_number: Some(2),
                recording_id: Some("recording-id".into()),
                length: Some(123000.0),
            }],
            ..ProviderAlbum::default()
        });

        assert_eq!(candidate.source, LookupSource::Musicbrainz);
        assert_eq!(
            candidate.musicbrainz_album_id.as_deref(),
            Some("release-id")
        );
        assert_eq!(
            candidate.musicbrainz_artist_id.as_deref(),
            Some("artist-id")
        );
        assert_eq!(candidate.tracks[0].disc_number, Some(2));
        assert_eq!(
            candidate.tracks[0].musicbrainz_track_id.as_deref(),
            Some("recording-id")
        );
        assert_eq!(
            candidate.tracks[0].artist.as_deref(),
            Some("Artist feat. Guest")
        );
    }

    #[test]
    fn discogs_provider_album_maps_release_genre_and_track_totals() {
        let candidate = discogs_candidate(ProviderAlbum {
            id: "42".into(),
            title: "Album".into(),
            artist: Some("Artist".into()),
            artists: vec!["Artist".into()],
            artist_id: Some("7".into()),
            year: Some("2004".into()),
            genre: Some("Rock, Indie Rock".into()),
            tracks: vec![crate::state::providers::ProviderTrack {
                title: Some("Track".into()),
                match_titles: Vec::new(),
                artist: Some("Artist".into()),
                artists: vec!["Artist".into()],
                track_number: Some(1),
                track_total: Some(1),
                disc_number: None,
                recording_id: None,
                length: Some(202.0),
            }],
            ..ProviderAlbum::default()
        });

        assert_eq!(candidate.source, LookupSource::Discogs);
        assert_eq!(candidate.discogs_release_id.as_deref(), Some("42"));
        assert_eq!(candidate.discogs_artist_id.as_deref(), Some("7"));
        assert_eq!(candidate.genre.as_deref(), Some("Rock, Indie Rock"));
        assert_eq!(candidate.tracks[0].track_total, Some(1));
        assert_eq!(candidate.tracks[0].length, Some(202.0));
    }

    #[test]
    fn lookup_request_keeps_embedded_album_separate_from_folder_identity() {
        let root = temp_root();
        let album = root.join("Folder Artist/2004 - Folder Album [FLAC]");
        fs::create_dir_all(&album).unwrap();
        fs::copy(corpus_mp3(), album.join("01.mp3")).unwrap();
        fs::copy(corpus_mp3(), album.join("02.mp3")).unwrap();

        let request = build_lookup_request(&album).unwrap();

        assert_eq!(request.artist_hint.as_deref(), Some("Corpus Artist"));
        assert_eq!(request.tagged_artist_hint.as_deref(), Some("Corpus Artist"));
        assert_eq!(request.folder_artist_hint.as_deref(), Some("Folder Artist"));
        assert_eq!(request.album_hint.as_deref(), Some("Corpus Album"));
        assert_eq!(request.tagged_album_hint.as_deref(), Some("Corpus Album"));
        assert_eq!(request.folder_album_hint.as_deref(), Some("Folder Album"));
        assert_eq!(request.year_hint.as_deref(), Some("2004"));
        assert_eq!(
            request.musicbrainz_album_id.as_deref(),
            Some("corpus-mb-album")
        );
        assert_eq!(request.discogs_release_id.as_deref(), Some("67890"));
        assert_eq!(request.tracks.len(), 2);
        assert_eq!(request.tracks[0].title.as_deref(), Some("Corpus MP3"));
        assert_eq!(request.tracks[0].track_total, Some(2));
        assert_eq!(
            request.tracks[0].musicbrainz_track_id.as_deref(),
            Some("corpus-mb-track")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lookup_request_uses_album_and_artist_above_cd_subfolder() {
        let root = temp_root();
        let disc = root.join("Folder Artist/2004 - Folder Album/CD 2");
        fs::create_dir_all(&disc).unwrap();
        fs::copy(corpus_mp3(), disc.join("01.mp3")).unwrap();

        let request = build_lookup_request(&disc).unwrap();

        assert_eq!(request.artist_hint.as_deref(), Some("Corpus Artist"));
        assert_eq!(request.folder_artist_hint.as_deref(), Some("Folder Artist"));
        assert_eq!(request.album_hint.as_deref(), Some("Corpus Album"));
        assert_eq!(request.folder_album_hint.as_deref(), Some("Folder Album"));
        assert_eq!(request.year_hint.as_deref(), Some("2004"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folder_candidate_normalizes_compilation_album_artist_and_keeps_ids() {
        let request = LookupRequest {
            artist_hint: Some("Various Artists".into()),
            album_hint: Some("Sampler".into()),
            musicbrainz_album_id: Some("mbid".into()),
            discogs_release_id: Some("42".into()),
            tracks: vec![track("Song", "Per-track Artist")],
            ..LookupRequest::default()
        };

        let candidate = folder_candidate(&request);

        assert_eq!(candidate.source, LookupSource::Folder);
        assert_eq!(candidate.album_artist.as_deref(), Some("Various Artists"));
        assert_eq!(candidate.album_artists, vec!["Various Artists"]);
        assert_eq!(candidate.musicbrainz_album_id.as_deref(), Some("mbid"));
        assert_eq!(candidate.discogs_release_id.as_deref(), Some("42"));
        assert_eq!(
            candidate.tracks[0].artist.as_deref(),
            Some("Per-track Artist")
        );
    }

    #[test]
    fn folder_candidate_splits_collaborative_track_artists() {
        let request = LookupRequest {
            artist_hint: Some("陶晶莹".into()),
            album_hint: Some("你又复活了".into()),
            tracks: vec![track("执着", "陶晶莹&张雨生")],
            ..LookupRequest::default()
        };

        let candidate = folder_candidate(&request);

        // Display credit is preserved on the ARTIST tag.
        assert_eq!(candidate.tracks[0].artist.as_deref(), Some("陶晶莹&张雨生"));
        // ARTISTS list is split into individual collaborators.
        assert_eq!(candidate.tracks[0].artists, vec!["陶晶莹", "张雨生"]);
    }

    #[test]
    fn folder_candidate_splits_collaborative_album_artist_hint() {
        let request = LookupRequest {
            artist_hint: Some("陶晶莹&张雨生".into()),
            album_hint: Some("执着".into()),
            tracks: vec![TrackCandidate {
                title: Some("执着".into()),
                ..TrackCandidate::default()
            }],
            ..LookupRequest::default()
        };

        let candidate = folder_candidate(&request);

        assert_eq!(candidate.album_artist.as_deref(), Some("陶晶莹&张雨生"));
        assert_eq!(candidate.album_artists, vec!["陶晶莹", "张雨生"]);
        assert_eq!(candidate.artists, vec!["陶晶莹", "张雨生"]);
    }

    #[test]
    fn folder_candidate_preserves_explicit_multi_artist_list() {
        let request = LookupRequest {
            artist_hint: Some("陶晶莹".into()),
            album_hint: Some("你又复活了".into()),
            tracks: vec![TrackCandidate {
                title: Some("执着".into()),
                artist: Some("陶晶莹 & 张雨生".into()),
                artists: vec!["陶晶莹".into(), "张雨生".into()],
                ..TrackCandidate::default()
            }],
            ..LookupRequest::default()
        };

        let candidate = folder_candidate(&request);

        assert_eq!(candidate.tracks[0].artists, vec!["陶晶莹", "张雨生"]);
    }

    #[test]
    fn folder_candidate_keeps_solo_artist_unsplit() {
        let request = LookupRequest {
            artist_hint: Some("陶晶莹".into()),
            album_hint: Some("你又复活了".into()),
            tracks: vec![track("爱情悲喜剧", "陶晶莹")],
            ..LookupRequest::default()
        };

        let candidate = folder_candidate(&request);

        assert_eq!(candidate.tracks[0].artist.as_deref(), Some("陶晶莹"));
        assert_eq!(candidate.tracks[0].artists, vec!["陶晶莹"]);
    }

    #[test]
    fn folder_candidate_splits_collaborative_artist_when_artists_list_empty() {
        // `artist` credit set, `artists` empty: derive the list from `artist`.
        let request = LookupRequest {
            artist_hint: Some("陶晶莹".into()),
            album_hint: Some("你又复活了".into()),
            tracks: vec![TrackCandidate {
                title: Some("执着".into()),
                artist: Some("陶晶莹&张雨生".into()),
                ..TrackCandidate::default()
            }],
            ..LookupRequest::default()
        };

        let candidate = folder_candidate(&request);

        assert_eq!(candidate.tracks[0].artist.as_deref(), Some("陶晶莹&张雨生"));
        assert_eq!(candidate.tracks[0].artists, vec!["陶晶莹", "张雨生"]);
    }

    #[test]
    fn folder_candidate_derives_solo_artist_list_from_display_credit() {
        // `artist` is solo and `artists` is empty: list is derived from `artist`.
        let request = LookupRequest {
            artist_hint: Some("陶晶莹".into()),
            album_hint: Some("你又复活了".into()),
            tracks: vec![TrackCandidate {
                title: Some("爱情悲喜剧".into()),
                artist: Some("陶晶莹".into()),
                ..TrackCandidate::default()
            }],
            ..LookupRequest::default()
        };

        let candidate = folder_candidate(&request);

        assert_eq!(candidate.tracks[0].artist.as_deref(), Some("陶晶莹"));
        assert_eq!(candidate.tracks[0].artists, vec!["陶晶莹"]);
    }

    #[test]
    fn selected_disc_filters_multi_disc_provider_candidate() {
        // Build a 28-track (2×14) provider candidate mimicking the 挑信 release.
        let cd1_local: Vec<TrackCandidate> = (0..14u32)
            .map(|i| TrackCandidate {
                title: Some(format!("CD1 Track {}", i + 1)),
                track_number: Some(i + 1),
                disc_number: None,
                ..TrackCandidate::default()
            })
            .collect();
        let cd1_provider: Vec<TrackCandidate> = cd1_local
            .iter()
            .enumerate()
            .map(|(i, _)| TrackCandidate {
                title: Some(format!("CD1 Track {}", i + 1)),
                musicbrainz_track_id: Some(format!("mb-cd1-{}", i + 1)),
                track_number: Some(i as u32 + 1),
                disc_number: Some(1),
                ..TrackCandidate::default()
            })
            .collect();
        let cd2_provider: Vec<TrackCandidate> = (0..14u32)
            .map(|i| TrackCandidate {
                title: Some(format!("CD2 Track {}", i + 1)),
                musicbrainz_track_id: Some(format!("mb-cd2-{}", i + 1)),
                track_number: Some(i + 1),
                disc_number: Some(2),
                ..TrackCandidate::default()
            })
            .collect();
        let mut all_provider = cd1_provider.clone();
        all_provider.extend(cd2_provider);

        let request = LookupRequest {
            selected_disc_number: Some(1),
            tracks: cd1_local.clone(),
            ..LookupRequest::default()
        };
        let candidate = AlbumCandidate {
            artist: Some("Test Artist".into()),
            tracks: all_provider,
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };

        let protected = protect_candidate_tracks(&request, &candidate);
        assert_eq!(protected.tracks.len(), 14, "should produce 14 CD1 tracks");
        for (i, track) in protected.tracks.iter().enumerate() {
            assert_eq!(
                track.title,
                Some(format!("CD1 Track {}", i + 1)),
                "CD1 title at {}",
                i
            );
            assert_eq!(
                track.musicbrainz_track_id,
                Some(format!("mb-cd1-{}", i + 1)),
                "CD1 MB track ID at {}",
                i
            );
        }
    }

    #[test]
    fn selected_disc_cd2_produces_correct_tracks() {
        let local_14: Vec<TrackCandidate> = (0..14u32)
            .map(|i| TrackCandidate {
                title: Some(format!("CD2 Track {}", i + 1)),
                track_number: Some(i + 1),
                ..TrackCandidate::default()
            })
            .collect();
        let cd1_provider: Vec<TrackCandidate> = (0..14u32)
            .map(|i| TrackCandidate {
                title: Some(format!("CD1 Track {}", i + 1)),
                disc_number: Some(1),
                ..TrackCandidate::default()
            })
            .collect();
        let cd2_provider: Vec<TrackCandidate> = (0..14u32)
            .map(|i| TrackCandidate {
                title: Some(format!("CD2 Track {}", i + 1)),
                musicbrainz_track_id: Some(format!("mb-cd2-{}", i + 1)),
                track_number: Some(i + 1),
                disc_number: Some(2),
                ..TrackCandidate::default()
            })
            .collect();
        let mut all_provider = cd1_provider;
        all_provider.extend(cd2_provider);

        let request = LookupRequest {
            selected_disc_number: Some(2),
            tracks: local_14,
            ..LookupRequest::default()
        };
        let candidate = AlbumCandidate {
            tracks: all_provider,
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };

        let protected = protect_candidate_tracks(&request, &candidate);
        assert_eq!(protected.tracks.len(), 14);
        for (i, track) in protected.tracks.iter().enumerate() {
            assert_eq!(
                track.musicbrainz_track_id,
                Some(format!("mb-cd2-{}", i + 1))
            );
        }
    }

    #[test]
    fn selected_disc_does_not_filter_when_provider_has_no_disc_numbers() {
        let local: Vec<TrackCandidate> = (0..3u32)
            .map(|i| TrackCandidate {
                title: Some(format!("Track {}", i + 1)),
                ..TrackCandidate::default()
            })
            .collect();
        let provider: Vec<TrackCandidate> = (0..3u32)
            .map(|i| TrackCandidate {
                title: Some(format!("Remote Track {}", i + 1)),
                disc_number: None, // no disc numbers
                ..TrackCandidate::default()
            })
            .collect();

        let request = LookupRequest {
            selected_disc_number: Some(1),
            tracks: local,
            ..LookupRequest::default()
        };
        let candidate = AlbumCandidate {
            tracks: provider,
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };
        // Provider has no disc numbers → should not filter (keep all 3).
        let protected = protect_candidate_tracks(&request, &candidate);
        assert_eq!(protected.tracks.len(), 3);
    }

    #[test]
    fn selected_disc_does_not_filter_on_count_mismatch() {
        let local: Vec<TrackCandidate> = (0..3u32)
            .map(|i| TrackCandidate {
                title: Some(format!("Track {}", i + 1)),
                ..TrackCandidate::default()
            })
            .collect();
        // Provider has 5 CD1 tracks, but local has 3 → mismatch, don't filter.
        let provider: Vec<TrackCandidate> = (0..5u32)
            .map(|i| TrackCandidate {
                title: Some(format!("CD1 Track {}", i + 1)),
                disc_number: Some(1),
                ..TrackCandidate::default()
            })
            .collect();

        let request = LookupRequest {
            selected_disc_number: Some(1),
            tracks: local.clone(),
            ..LookupRequest::default()
        };
        let candidate = AlbumCandidate {
            tracks: provider,
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };
        // Count doesn't match → discard scoping. Matching still runs,
        // which anchors on the local track count.
        let protected = protect_candidate_tracks(&request, &candidate);
        assert_eq!(protected.tracks.len(), 3); // matching aligns to local
    }

    #[test]
    fn provider_credibility_scopes_selected_disc_before_allowing_extras() {
        let request = LookupRequest {
            artist_hint: Some("Artist".into()),
            album_hint: Some("Album".into()),
            selected_disc_number: Some(1),
            tracks: vec![track("Song One", "Artist"), track("Song Two", "Artist")],
            ..LookupRequest::default()
        };
        let candidate = AlbumCandidate {
            artist: Some("Artist".into()),
            album_artist: Some("Artist".into()),
            album: Some("Album".into()),
            tracks: vec![
                TrackCandidate {
                    title: Some("Song One".into()),
                    disc_number: Some(1),
                    ..TrackCandidate::default()
                },
                TrackCandidate {
                    title: Some("Song Two".into()),
                    disc_number: Some(1),
                    ..TrackCandidate::default()
                },
                TrackCandidate {
                    title: Some("Bonus Song".into()),
                    disc_number: Some(1),
                    ..TrackCandidate::default()
                },
                TrackCandidate {
                    title: Some("Song One".into()),
                    disc_number: Some(2),
                    ..TrackCandidate::default()
                },
            ],
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };

        assert!(provider_candidate_credibility(&request, &candidate).is_ok());
    }

    #[test]
    fn provider_credibility_rejects_when_selected_disc_is_missing() {
        let request = LookupRequest {
            artist_hint: Some("Artist".into()),
            album_hint: Some("Album".into()),
            selected_disc_number: Some(1),
            tracks: vec![track("Repeated Song", "Artist")],
            ..LookupRequest::default()
        };
        let candidate = AlbumCandidate {
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            tracks: vec![TrackCandidate {
                title: Some("Repeated Song".into()),
                disc_number: Some(2),
                ..TrackCandidate::default()
            }],
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };

        assert!(provider_candidate_credibility(&request, &candidate).is_err());
    }

    #[test]
    fn build_lookup_request_parses_selected_disc_number() {
        // "挑信 CD1" should produce selected_disc_number=1
        let root = temp_root();
        let disc = root.join("挑信 CD1");
        fs::create_dir_all(&disc).unwrap();
        fs::copy(corpus_mp3(), disc.join("01.mp3")).unwrap();
        fs::copy(corpus_mp3(), disc.join("02.mp3")).unwrap();

        let request = build_lookup_request(&disc).unwrap();
        assert_eq!(request.selected_disc_number, Some(1));

        // Bare "CD1" should also work.
        let bare = root.join("CD2");
        fs::create_dir_all(&bare).unwrap();
        fs::copy(corpus_mp3(), bare.join("01.mp3")).unwrap();
        let request2 = build_lookup_request(&bare).unwrap();
        assert_eq!(request2.selected_disc_number, Some(2));

        // No disc suffix → None.
        let plain = root.join("Plain Album");
        fs::create_dir_all(&plain).unwrap();
        fs::copy(corpus_mp3(), plain.join("01.mp3")).unwrap();
        let request3 = build_lookup_request(&plain).unwrap();
        assert_eq!(request3.selected_disc_number, None);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fresh_provider_precedes_stale_cache_and_controls_cache_replacement() {
        let cached = AlbumCandidate {
            album: Some("Stale Album".into()),
            musicbrainz_album_id: Some("stale-id".into()),
            tracks: vec![track("Stale Title", "Stale Artist")],
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };
        let fresh = AlbumCandidate {
            album: Some("Canonical Album".into()),
            musicbrainz_album_id: Some("fresh-id".into()),
            tracks: vec![track("Canonical Title", "Canonical Artist")],
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };
        let folder = AlbumCandidate {
            album: Some("Folder Album".into()),
            source: LookupSource::Folder,
            ..AlbumCandidate::default()
        };

        let selected = combine_candidate_sources(vec![fresh.clone()], vec![cached], folder);

        assert_eq!(selected[0].album.as_deref(), Some("Canonical Album"));
        assert_eq!(
            selected[0].musicbrainz_album_id.as_deref(),
            Some("fresh-id")
        );
        assert_eq!(
            selected[0].tracks[0].title.as_deref(),
            Some("Canonical Title")
        );
        assert!(should_replace_lookup_cache(&[fresh], true));
        assert!(!should_replace_lookup_cache(
            &[AlbumCandidate {
                source: LookupSource::Folder,
                ..AlbumCandidate::default()
            }],
            true
        ));
    }

    #[test]
    fn album_filter_rejects_unrelated_provider_result_but_accepts_chinese_variant() {
        let candidates = vec![
            AlbumCandidate {
                album: Some("无限".into()),
                source: LookupSource::Musicbrainz,
                ..AlbumCandidate::default()
            },
            AlbumCandidate {
                album: Some("Unrelated".into()),
                source: LookupSource::Discogs,
                ..AlbumCandidate::default()
            },
        ];

        let filtered = filter_candidates_for_album(Some("無限"), candidates, None);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].album.as_deref(), Some("无限"));
    }

    #[test]
    fn album_filter_exempts_direct_mb_id_even_when_name_mismatches() {
        // When a candidate's musicbrainz_album_id matches the request's
        // requested ID, it should NOT be filtered by album name — the user
        // explicitly set that ID in the file tags, so the MB album name
        // (e.g. "SACD Best Collection") may differ from the folder name
        // (e.g. "精选 Collection").
        let candidates = vec![
            AlbumCandidate {
                album: Some("SACD Best Collection".into()),
                musicbrainz_album_id: Some("1976db43-24bc-432f-8437-d0adf1de1b0d".into()),
                source: LookupSource::Musicbrainz,
                ..AlbumCandidate::default()
            },
            AlbumCandidate {
                album: Some("Some Other Album".into()),
                source: LookupSource::Discogs,
                ..AlbumCandidate::default()
            },
        ];

        // With the matched request ID, the SACD candidate survives.
        let filtered = filter_candidates_for_album(
            Some("精选 Collection"),
            candidates.clone(),
            Some("1976db43-24bc-432f-8437-d0adf1de1b0d"),
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(
            filtered[0].musicbrainz_album_id.as_deref(),
            Some("1976db43-24bc-432f-8437-d0adf1de1b0d")
        );

        // Without the matched request ID (None), the SACD candidate is
        // rejected by album name mismatch.
        let filtered = filter_candidates_for_album(Some("精选 Collection"), candidates, None);
        assert_eq!(filtered.len(), 0);
    }

    #[test]
    fn artist_release_ranking_filters_title_and_prefers_requested_year() {
        let releases = vec![
            ProviderReleaseSummary {
                id: "wrong".into(),
                title: "Different Album".into(),
                year: Some(2024),
                kind: Some("release".into()),
                artist_name: Some("Artist".into()),
                ..ProviderReleaseSummary::default()
            },
            ProviderReleaseSummary {
                id: "old".into(),
                title: "無限".into(),
                year: Some(2004),
                kind: Some("release".into()),
                artist_name: Some("Artist".into()),
                ..ProviderReleaseSummary::default()
            },
            ProviderReleaseSummary {
                id: "requested".into(),
                title: "无限".into(),
                year: Some(2005),
                kind: Some("release".into()),
                artist_name: Some("Artist".into()),
                ..ProviderReleaseSummary::default()
            },
        ];

        let ranked = rank_artist_releases(releases, Some("無限"), Some("2005"));

        assert_eq!(
            ranked
                .iter()
                .map(|release| release.id.as_str())
                .collect::<Vec<_>>(),
            vec!["requested", "old"]
        );
    }

    #[test]
    fn detailed_release_ranking_prefers_track_identity_over_list_order() {
        let request = LookupRequest {
            tracks: vec![track("First", "Artist"), track("Second", "Artist")],
            ..LookupRequest::default()
        };
        let wrong = AlbumCandidate {
            album: Some("Album".into()),
            tracks: vec![
                track("Unrelated A", "Artist"),
                track("Unrelated B", "Artist"),
            ],
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };
        let matching = AlbumCandidate {
            album: Some("Album".into()),
            tracks: vec![track("First", "Artist"), track("Second", "Artist")],
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };

        let ranked = rank_candidate_details(vec![wrong, matching.clone()], &request, "musicbrainz");

        assert_eq!(ranked[0], matching);
    }

    #[test]
    fn chinese_write_conversion_updates_album_and_per_track_text_only() {
        let candidate = AlbumCandidate {
            artist: Some("音乐".into()),
            artists: vec!["音乐".into()],
            album: Some("无限".into()),
            musicbrainz_album_id: Some("release-id".into()),
            tracks: vec![TrackCandidate {
                title: Some("后来".into()),
                artist: Some("音乐".into()),
                track_number: Some(1),
                ..Default::default()
            }],
            ..Default::default()
        };

        let converted = convert_candidate_chinese(&candidate, Some("traditional"));

        assert_eq!(converted.artist.as_deref(), Some("音樂"));
        assert_eq!(converted.album.as_deref(), Some("無限"));
        assert_eq!(converted.tracks[0].title.as_deref(), Some("後來"));
        assert_eq!(converted.tracks[0].track_number, Some(1));
        assert_eq!(
            converted.musicbrainz_album_id.as_deref(),
            Some("release-id")
        );
    }

    #[test]
    fn chinese_write_conversion_simplified_converts_traditional_to_simplified_only() {
        let candidate = AlbumCandidate {
            artist: Some("音樂".into()),
            artists: vec!["音樂".into()],
            album: Some("無限".into()),
            musicbrainz_album_id: Some("release-id".into()),
            tracks: vec![TrackCandidate {
                title: Some("後來".into()),
                artist: Some("音樂".into()),
                track_number: Some(1),
                ..Default::default()
            }],
            ..Default::default()
        };

        let converted = convert_candidate_chinese(&candidate, Some("simplified"));

        assert_eq!(converted.artist.as_deref(), Some("音乐"));
        assert_eq!(converted.album.as_deref(), Some("无限"));
        assert_eq!(converted.tracks[0].title.as_deref(), Some("后来"));
        // Non-text fields must be preserved unchanged
        assert_eq!(converted.tracks[0].track_number, Some(1));
        assert_eq!(
            converted.musicbrainz_album_id.as_deref(),
            Some("release-id")
        );
    }

    #[test]
    fn chinese_write_conversion_no_target_leaves_unchanged() {
        let candidate = AlbumCandidate {
            artist: Some("音樂".into()),
            tracks: vec![TrackCandidate {
                title: Some("後來".into()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let converted = convert_candidate_chinese(&candidate, Some("simplified"));

        // Simplified should convert traditional characters
        assert_eq!(converted.artist.as_deref(), Some("音乐"));
        assert_eq!(converted.tracks[0].title.as_deref(), Some("后来"));

        // With no target, no conversion happens
        let unchanged = convert_candidate_chinese(&candidate, None);
        assert_eq!(unchanged.artist.as_deref(), Some("音樂"));
        assert_eq!(unchanged.tracks[0].title.as_deref(), Some("後來"));
        let unchanged = convert_candidate_chinese(&candidate, Some(""));
        assert_eq!(unchanged.artist.as_deref(), Some("音樂"));
        let unchanged = convert_candidate_chinese(&candidate, Some("unknown"));
        assert_eq!(unchanged.artist.as_deref(), Some("音樂"));
    }

    #[test]
    fn track_protection_rejects_unrelated_provider_titles_but_allows_polluted_match() {
        let request = LookupRequest {
            tracks: vec![track("01 - Local Song (Remastered)", "Local Artist")],
            ..Default::default()
        };
        let unrelated = AlbumCandidate {
            source: LookupSource::Musicbrainz,
            tracks: vec![track("Different Song", "Wrong Artist")],
            ..Default::default()
        };
        let matching = AlbumCandidate {
            source: LookupSource::Musicbrainz,
            tracks: vec![track("Local Song", "Canonical Artist")],
            ..Default::default()
        };

        let protected = protect_candidate_tracks(&request, &unrelated);
        let accepted = protect_candidate_tracks(&request, &matching);

        assert_eq!(
            protected.tracks[0].title.as_deref(),
            Some("01 - Local Song (Remastered)")
        );
        assert_eq!(protected.tracks[0].artist.as_deref(), Some("Local Artist"));
        assert_eq!(accepted.tracks[0].title.as_deref(), Some("Local Song"));
        assert_eq!(
            accepted.tracks[0].artist.as_deref(),
            Some("Canonical Artist")
        );
    }

    #[test]
    fn track_protection_aligns_reordered_provider_tracks_by_title() {
        let request = LookupRequest {
            tracks: vec![track("First", "Local One"), track("Second", "Local Two")],
            ..Default::default()
        };
        let candidate = AlbumCandidate {
            source: LookupSource::Musicbrainz,
            tracks: vec![
                TrackCandidate {
                    musicbrainz_track_id: Some("second-id".into()),
                    ..track("Second", "Remote Two")
                },
                TrackCandidate {
                    musicbrainz_track_id: Some("first-id".into()),
                    ..track("First", "Remote One")
                },
            ],
            ..Default::default()
        };

        let protected = protect_candidate_tracks(&request, &candidate);

        assert_eq!(protected.tracks[0].title.as_deref(), Some("First"));
        assert_eq!(protected.tracks[0].artist.as_deref(), Some("Remote One"));
        assert_eq!(
            protected.tracks[0].musicbrainz_track_id.as_deref(),
            Some("first-id")
        );
        assert_eq!(protected.tracks[1].title.as_deref(), Some("Second"));
        assert_eq!(protected.tracks[1].artist.as_deref(), Some("Remote Two"));
        assert_eq!(
            protected.tracks[1].musicbrainz_track_id.as_deref(),
            Some("second-id")
        );
    }

    #[test]
    fn track_protection_rejects_same_title_with_incompatible_duration() {
        let request = LookupRequest {
            tracks: vec![TrackCandidate {
                length: Some(200.0),
                ..track("Song", "Local Artist")
            }],
            ..Default::default()
        };
        let candidate = AlbumCandidate {
            source: LookupSource::Musicbrainz,
            tracks: vec![TrackCandidate {
                length: Some(300_000.0),
                musicbrainz_track_id: Some("wrong-recording".into()),
                ..track("Song", "Wrong Artist")
            }],
            ..Default::default()
        };

        let protected = protect_candidate_tracks(&request, &candidate);

        assert_eq!(protected.tracks[0].artist.as_deref(), Some("Local Artist"));
        assert_eq!(protected.tracks[0].musicbrainz_track_id, None);
    }

    #[test]
    fn genre_fill_requires_nonempty_high_confidence_value() {
        assert_eq!(
            genre_from_value(&serde_json::json!({
                "genre": "Rock, Indie Rock",
                "confidence": 0.6
            })),
            Some("Rock, Indie Rock".into())
        );
        assert_eq!(
            genre_from_value(&serde_json::json!({
                "genre": "Rock",
                "confidence": 0.59
            })),
            None
        );
        assert_eq!(
            genre_from_value(&serde_json::json!({
                "genre": "unknown",
                "confidence": 0.99
            })),
            None
        );
        assert_eq!(
            genre_from_value(&serde_json::json!({
                "genre": "Mandopop, Pop",
                "confidence": "0.85"
            })),
            Some("Mandopop, Pop".into()),
            "a numeric confidence serialized as text must not discard a valid genre"
        );
    }

    #[test]
    fn genre_fill_outcome_distinguishes_applied_rejected_and_failed_attempts() {
        assert_eq!(
            genre_fill_outcome(Ok(serde_json::json!({
                "genre": "Mandopop, Pop",
                "confidence": 0.85
            }))),
            GenreFillOutcome::Applied("Mandopop, Pop".into())
        );
        assert_eq!(
            genre_fill_outcome(Ok(serde_json::json!({
                "genre": "Pop",
                "confidence": 0.4
            }))),
            GenreFillOutcome::Rejected {
                genre: Some("Pop".into()),
                confidence: Some(0.4),
            }
        );
        assert_eq!(
            genre_fill_outcome(Err("LLM returned malformed JSON".into())),
            GenreFillOutcome::Failed("LLM returned malformed JSON".into())
        );
    }

    #[test]
    fn auto_tag_event_matches_renderer_contract() {
        let event = auto_tag_event(
            "auto-tag-1",
            "completed",
            "Complete",
            9,
            Some(serde_json::json!({"artist": "Artist"})),
        );

        assert_eq!(
            serde_json::to_value(event).unwrap(),
            serde_json::json!({
                "taskId": "auto-tag-1",
                "type": "completed",
                "message": "Complete",
                "progress": 9,
                "total": 9,
                "data": {"artist": "Artist"}
            })
        );
    }

    #[test]
    fn completion_message_discloses_when_genre_remains_missing() {
        assert_eq!(
            auto_tag_completion_message(&AlbumCandidate::default()),
            "Complete — genre remains missing"
        );
        assert_eq!(
            auto_tag_completion_message(&AlbumCandidate {
                genre: Some("Mandopop, Pop".into()),
                ..AlbumCandidate::default()
            }),
            "Complete"
        );
    }

    #[test]
    fn rank_artist_releases_filters_super_girl_releases() {
        // Regression: when filtering 蕭亞軒's artist releases with the folder hint
        // "Super Girl 爱无畏（精歌+精选）", both MusicBrainz releases must survive.
        let releases = vec![
            ProviderReleaseSummary {
                id: "23058dc0-d399-447a-8c75-798fee8af9c4".into(),
                title: "Super Girl 愛 無畏 新歌＋精選".into(),
                year: Some(2012),
                kind: Some("release".into()),
                artist_name: Some("蕭亞軒".into()),
                ..ProviderReleaseSummary::default()
            },
            ProviderReleaseSummary {
                id: "a9746022-a1f5-478f-9480-6ffe9e846b40".into(),
                title: "Super Girl 爱无畏".into(),
                year: Some(2012),
                kind: Some("release".into()),
                artist_name: Some("蕭亞軒".into()),
                ..ProviderReleaseSummary::default()
            },
            ProviderReleaseSummary {
                id: "unrelated".into(),
                title: "Unrelated Album".into(),
                year: Some(2004),
                kind: Some("release".into()),
                artist_name: Some("蕭亞軒".into()),
                ..ProviderReleaseSummary::default()
            },
        ];

        let ranked = rank_artist_releases(
            releases,
            Some("Super Girl 爱无畏（精歌+精选）"),
            Some("2012"),
        );

        // Only the two Super Girl releases should pass the filter
        assert_eq!(ranked.len(), 2);
        assert!(ranked
            .iter()
            .any(|r| r.id == "a9746022-a1f5-478f-9480-6ffe9e846b40"));
        assert!(ranked
            .iter()
            .any(|r| r.id == "23058dc0-d399-447a-8c75-798fee8af9c4"));
    }

    #[tokio::test]
    async fn candidate_apply_writes_album_and_per_track_fields_through_safe_queue() {
        let root = temp_root();
        let album = root.join("Artist/Album");
        fs::create_dir_all(&album).unwrap();
        fs::copy(corpus_mp3(), album.join("01.mp3")).unwrap();
        fs::copy(corpus_mp3(), album.join("02.mp3")).unwrap();
        let candidate = AlbumCandidate {
            artist: Some("Album Artist".into()),
            artists: vec!["Album Artist".into()],
            album: Some("Canonical Album".into()),
            album_artist: Some("Album Artist".into()),
            album_artists: vec!["Album Artist".into()],
            year: Some("2004".into()),
            genre: Some("Rock".into()),
            musicbrainz_album_id: Some("release-id".into()),
            tracks: vec![
                track("First", "Album Artist"),
                track("Second", "Album Artist feat. Guest"),
            ],
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };
        let queue = crate::state::write_queue::WriteQueue::default();

        let written = apply_candidate_tags(&album, &candidate, &queue)
            .await
            .unwrap();

        assert_eq!(written, 2);
        let first = crate::commands::tracks::read_track_metadata(&album.join("01.mp3")).unwrap();
        let second = crate::commands::tracks::read_track_metadata(&album.join("02.mp3")).unwrap();
        assert_eq!(first.album.as_deref(), Some("Canonical Album"));
        assert_eq!(first.album_artist.as_deref(), Some("Album Artist"));
        assert_eq!(first.musicbrainz_album_id.as_deref(), Some("release-id"));
        assert_eq!(first.title.as_deref(), Some("First"));
        assert_eq!(second.title.as_deref(), Some("Second"));
        assert_eq!(second.artist.as_deref(), Some("Album Artist feat. Guest"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn folder_candidate_apply_splits_collaborative_artists_into_multi_value_flac() {
        // Regression: a collaborative track whose ARTIST is a single concatenated
        // credit ("陶晶莹&张雨生") must be written as two ARTISTS entries on the
        // fallback (no-release) path, while the ARTIST display credit is kept.
        let root = temp_root();
        let album = root.join("陶晶莹/1999-你又复活了[flac]");
        fs::create_dir_all(&album).unwrap();
        let track_path = album.join("陶晶莹_张雨生-执着.flac");
        fs::copy(corpus_flac(), &track_path).unwrap();

        let request = LookupRequest {
            path: album.to_string_lossy().into_owned(),
            artist_hint: Some("陶晶莹".into()),
            album_hint: Some("你又复活了".into()),
            tracks: vec![TrackCandidate {
                title: Some("执着".into()),
                artist: Some("陶晶莹&张雨生".into()),
                artists: vec!["陶晶莹&张雨生".into()],
                ..TrackCandidate::default()
            }],
            ..LookupRequest::default()
        };
        let candidate = folder_candidate(&request);
        // Sanity: the fallback candidate already split the per-track list.
        assert_eq!(candidate.tracks[0].artists, vec!["陶晶莹", "张雨生"]);

        let written = apply_candidate_tags(&album, &candidate, &WriteQueue::default())
            .await
            .unwrap();
        assert_eq!(written, 1);

        // The ARTIST display credit is preserved.
        let read = crate::commands::tracks::read_track_metadata(&track_path).unwrap();
        assert_eq!(read.artist.as_deref(), Some("陶晶莹&张雨生"));
        // The ARTISTS multi-value field now carries one entry per collaborator.
        let artists = crate::commands::tracks::read_extra_tags(&track_path)
            .into_iter()
            .filter(|row| row.key == "ARTISTS")
            .map(|row| row.value)
            .collect::<Vec<_>>();
        assert_eq!(artists, vec!["陶晶莹", "张雨生"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn candidate_apply_reports_partial_write_failure_after_attempting_all_tracks() {
        let root = temp_root();
        let album = root.join("Artist/Album");
        fs::create_dir_all(&album).unwrap();
        fs::copy(corpus_mp3(), album.join("01.mp3")).unwrap();
        fs::copy(corpus_aiff(), album.join("02.aiff")).unwrap();
        let candidate = AlbumCandidate {
            album: Some("Canonical Album".into()),
            album_artists: vec!["Artist".into()],
            tracks: vec![track("First", "Artist"), track("Second", "Artist")],
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };

        let error = apply_candidate_tags(&album, &candidate, &WriteQueue::default())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("02.aiff"));
        let first = crate::commands::tracks::read_track_metadata(&album.join("01.mp3")).unwrap();
        assert_eq!(first.title.as_deref(), Some("First"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn candidate_apply_all_tracks_applies_album_fields_to_empty_track_candidate() {
        // Full auto-tag applies album metadata to every file even when a
        // positional candidate has no per-track fields. It must preserve that
        // file's existing per-track tags rather than clearing them.
        let root = temp_root();
        let album = root.join("Artist/Album");
        fs::create_dir_all(&album).unwrap();
        let track_path = album.join("01.mp3");
        fs::copy(corpus_mp3(), &track_path).unwrap();
        let seed: TrackPatch = serde_json::from_value(serde_json::json!({
            "title": "Original Title",
            "artist": "Original Artist",
            "trackNumber": 5,
            "trackTotal": 10,
            "discNumber": 1,
            "discTotal": 2,
        }))
        .unwrap();
        crate::commands::mutations::write_track_dispatch(&track_path, &seed).unwrap();

        let candidate = AlbumCandidate {
            album: Some("Canonical Album".into()),
            album_artists: vec!["Artist".into()],
            tracks: vec![TrackCandidate::default()],
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };

        let written = apply_candidate_tags(&album, &candidate, &WriteQueue::default())
            .await
            .unwrap();

        assert_eq!(written, 1);
        let read = crate::commands::tracks::read_track_metadata(&track_path).unwrap();
        assert_eq!(read.album.as_deref(), Some("Canonical Album"));
        assert_eq!(read.title.as_deref(), Some("Original Title"));
        assert_eq!(read.artist.as_deref(), Some("Original Artist"));
        assert_eq!(read.track_number, Some(5));
        assert_eq!(read.disc_number, Some(1));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn candidate_apply_preserves_existing_year_when_candidate_year_missing() {
        // A candidate with no year must not remove an existing YEAR tag
        // (previously `year: null` became Patch::Null and deleted it).
        let root = temp_root();
        let album = root.join("Artist/Album");
        fs::create_dir_all(&album).unwrap();
        let track_path = album.join("01.mp3");
        fs::copy(corpus_mp3(), &track_path).unwrap();
        let seed: TrackPatch = serde_json::from_value(serde_json::json!({
            "title": "Original Title",
            "year": "2004",
        }))
        .unwrap();
        crate::commands::mutations::write_track_dispatch(&track_path, &seed).unwrap();

        let candidate = AlbumCandidate {
            album: Some("Canonical Album".into()),
            album_artists: vec!["Artist".into()],
            year: None,
            tracks: vec![track("First", "Artist")],
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };

        let written = apply_candidate_tags(&album, &candidate, &WriteQueue::default())
            .await
            .unwrap();

        assert_eq!(written, 1);
        let read = crate::commands::tracks::read_track_metadata(&track_path).unwrap();
        assert_eq!(read.year.as_deref(), Some("2004"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn candidate_apply_updates_existing_year_when_candidate_year_present() {
        let root = temp_root();
        let album = root.join("Artist/Album");
        fs::create_dir_all(&album).unwrap();
        let track_path = album.join("01.mp3");
        fs::copy(corpus_mp3(), &track_path).unwrap();
        let seed: TrackPatch = serde_json::from_value(serde_json::json!({
            "title": "Original Title",
            "year": "1999",
        }))
        .unwrap();
        crate::commands::mutations::write_track_dispatch(&track_path, &seed).unwrap();

        let candidate = AlbumCandidate {
            album: Some("Canonical Album".into()),
            album_artists: vec!["Artist".into()],
            year: Some("2008".into()),
            tracks: vec![track("First", "Artist")],
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };

        let written = apply_candidate_tags(&album, &candidate, &WriteQueue::default())
            .await
            .unwrap();

        assert_eq!(written, 1);
        let read = crate::commands::tracks::read_track_metadata(&track_path).unwrap();
        assert_eq!(read.year.as_deref(), Some("2008"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn disabled_provider_cache_cannot_authorize_a_write() {
        let root = temp_root();
        let album = root.join("张卫健/张卫健-《1993-真挚的朋友精选》[WAV 分轨]");
        fs::create_dir_all(&album).unwrap();
        let track_path = album.join("(03) [刘德华] 重赐我生命.wav");
        fs::copy(corpus_wav(), &track_path).unwrap();

        let cache = CacheState::new(root.clone());
        assert!(cache.initialize(Some(root.join("cache.db").to_str().unwrap())));
        let request = build_lookup_request(&album).unwrap();
        let cached = AlbumCandidate {
            artist: Some("张卫健".into()),
            album: request.album_hint.clone(),
            year: None,
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };
        cache
            .set_lookup(
                &query_hash(&request),
                &serde_json::to_value(&request).unwrap(),
                &serde_json::to_value(vec![cached]).unwrap(),
                "musicbrainz",
            )
            .unwrap();

        let config = AutoTagConfig {
            remote_lookup_enabled: Some(false),
            discogs_enabled: Some(false),
            ..AutoTagConfig::default()
        };
        let queue = WriteQueue::default();
        let cancelled = AtomicBool::new(false);
        let mut reports = Vec::new();
        let result = resolve_and_apply_album(
            &album,
            &config,
            AutoTagServices {
                providers: &ProviderState::new(),
                cache: &cache,
                queue: &queue,
                alias_file: &root.join("artist-aliases.json"),
            },
            &cancelled,
            |_, _| {},
            |kind, message, data| reports.push((kind.to_string(), message, data)),
        )
        .await
        .unwrap();

        assert_eq!(result.outcome, AutoTagOutcome::NeedsReview);
        assert_eq!(result.written, 0);
        assert!(reports.iter().any(|(kind, _, data)| {
            kind == "source"
                && data.as_ref().and_then(|data| data.get("source"))
                    == Some(&serde_json::json!("cache"))
        }));
        let written = crate::commands::tracks::read_track_metadata(&track_path).unwrap();
        assert_ne!(written.year.as_deref(), Some("1993"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn disabled_discogs_cache_cannot_authorize_alias_rewrites() {
        let root = temp_root();
        let album = root.join("郑少秋/1996.01-天地男儿-新歌精选[wav]");
        fs::create_dir_all(&album).unwrap();
        let local_tracks = [
            ("从不放弃", 1),
            ("男儿着眼天地间", 2),
            ("男儿无泪", 3),
            ("心中雨", 4),
            ("做个自由人", 5),
            ("Oh Gal", 6),
            ("徘徊在这段路上", 7),
            ("文秋郎", 8),
            ("冬连", 9),
            ("男儿志在四方", 10),
            ("游子恨", 11),
        ];
        for (title, track_number) in local_tracks {
            let path = album.join(format!("{title}.wav"));
            fs::copy(corpus_wav(), &path).unwrap();
            let seed: TrackPatch = serde_json::from_value(serde_json::json!({
                "title": title,
                "artist": "郑少秋",
                "artists": ["???", "郑少秋"],
                "album": "天地男儿-新歌精选",
                "albumArtist": "郑少秋",
                "albumArtists": ["郑少秋"],
                "year": "1996",
                "trackNumber": track_number,
                "trackTotal": 11,
                "musicbrainzAlbumId": null,
                "musicbrainzArtistId": null,
                "discogsReleaseId": null,
                "discogsArtistId": "3156508"
            }))
            .unwrap();
            crate::commands::mutations::write_track_dispatch(&path, &seed).unwrap();
        }

        let cache = CacheState::new(root.clone());
        assert!(cache.initialize(Some(root.join("cache.db").to_str().unwrap())));
        let request = build_lookup_request(&album).unwrap();
        let remote_titles = [
            "從不放棄 (天地男兒主題曲)",
            "男兒着眼天地間 (天地男兒插曲)",
            "男兒無淚 (天地男兒插曲)",
            "心中雨",
            "做個自由人",
            "Oh Gal",
            "排徊在這段路上",
            "文秋郎",
            "冬戀",
            "男兒志在四方",
            "遊子恨",
        ];
        let cached = AlbumCandidate {
            artist: Some("Adam Cheng".into()),
            artists: vec!["Adam Cheng".into()],
            album: Some("天地男兒新歌精選".into()),
            album_artist: Some("Adam Cheng".into()),
            album_artists: vec!["Adam Cheng".into()],
            year: Some("1996".into()),
            genre: Some("Pop, Stage & Screen, Cantopop".into()),
            discogs_release_id: Some("32867382".into()),
            discogs_artist_id: Some("3156508".into()),
            tracks: remote_titles
                .iter()
                .enumerate()
                .map(|(index, title)| TrackCandidate {
                    title: Some((*title).into()),
                    artist: Some("Adam Cheng".into()),
                    artists: vec!["Adam Cheng".into()],
                    track_number: u32::try_from(index + 1).ok(),
                    track_total: Some(11),
                    ..TrackCandidate::default()
                })
                .collect(),
            source: LookupSource::Discogs,
            ..AlbumCandidate::default()
        };
        cache
            .set_lookup(
                &query_hash(&request),
                &serde_json::to_value(&request).unwrap(),
                &serde_json::to_value(vec![cached]).unwrap(),
                "discogs",
            )
            .unwrap();

        let config = AutoTagConfig {
            remote_lookup_enabled: Some(false),
            discogs_enabled: Some(false),
            ..AutoTagConfig::default()
        };
        let result = resolve_and_apply_album(
            &album,
            &config,
            AutoTagServices {
                providers: &ProviderState::new(),
                cache: &cache,
                queue: &WriteQueue::default(),
                alias_file: &root.join("artist-aliases.json"),
            },
            &AtomicBool::new(false),
            |_, _| {},
            |_, _, _| {},
        )
        .await
        .unwrap();

        assert_eq!(result.outcome, AutoTagOutcome::NeedsReview);
        assert_eq!(result.written, 0);
        for path in collect_audio_files(&album) {
            let written = crate::commands::tracks::read_track_metadata(Path::new(&path)).unwrap();
            assert_eq!(written.artist.as_deref(), Some("郑少秋"), "{path}");
            assert_eq!(written.artists, vec!["???", "郑少秋"], "{path}");
            assert_eq!(written.album_artist.as_deref(), Some("郑少秋"), "{path}");
            assert_eq!(written.album_artists, vec!["郑少秋"], "{path}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn folder_only_runner_updates_progress_caches_and_writes_without_network() {
        let root = temp_root();
        let album = root.join("Folder Artist/Folder Album");
        fs::create_dir_all(&album).unwrap();
        let track_path = album.join("01.mp3");
        fs::copy(corpus_mp3(), &track_path).unwrap();
        let clear_ids: TrackPatch = serde_json::from_value(serde_json::json!({
            "musicbrainzAlbumId": null,
            "musicbrainzArtistId": null,
            "discogsReleaseId": null,
            "discogsArtistId": null
        }))
        .unwrap();
        crate::commands::mutations::write_track_dispatch(&track_path, &clear_ids).unwrap();
        let cache = CacheState::new(root.clone());
        assert!(cache.initialize(Some(root.join("cache.db").to_str().unwrap())));
        let queue = WriteQueue::default();
        let cancelled = AtomicBool::new(false);
        let config = AutoTagConfig {
            remote_lookup_enabled: Some(false),
            discogs_enabled: Some(false),
            ..AutoTagConfig::default()
        };
        let mut updates = Vec::new();
        let mut reports = Vec::new();

        let result = resolve_and_apply_album(
            &album,
            &config,
            AutoTagServices {
                providers: &ProviderState::new(),
                cache: &cache,
                queue: &queue,
                alias_file: &root.join("artist-aliases.json"),
            },
            &cancelled,
            |step, message| updates.push((step, message.to_string())),
            |kind, message, data| reports.push((kind.to_string(), message, data)),
        )
        .await
        .unwrap();

        assert_eq!(result.outcome, AutoTagOutcome::NeedsReview);
        assert_eq!(result.written, 0);
        assert_eq!(updates.first().unwrap().0, 1);
        assert_eq!(updates.last().unwrap().0, 9);
        assert!(reports.iter().any(|(kind, _, _)| kind == "needs_review"));
        assert!(!reports.iter().any(|(kind, _, _)| kind == "write"));
        let request = build_lookup_request(&album).unwrap();
        assert!(cache.lookup(&query_hash(&request)).is_none());
        let written = crate::commands::tracks::read_track_metadata(&track_path).unwrap();
        assert_eq!(written.artist.as_deref(), Some("Corpus Artist"));
        assert_eq!(written.album_artist.as_deref(), Some("Corpus Album Artist"));
        assert_eq!(written.album.as_deref(), Some("Corpus Album"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn english_edition_folder_keeps_tagged_album_and_extracts_country() {
        let evidence = parse_folder_album_evidence(
            "1991 - Emotions [Japanese Edition]-tracks",
            Some("Emotions"),
        );

        assert_eq!(evidence.search_album.as_deref(), Some("Emotions"));
        assert_eq!(evidence.folder_album.as_deref(), Some("Emotions"));
        assert_eq!(evidence.tagged_album.as_deref(), Some("Emotions"));
        assert_eq!(evidence.country.as_deref(), Some("JP"));
        assert_eq!(evidence.year.as_deref(), Some("1991"));
    }

    #[test]
    fn release_summary_ranking_prefers_explicit_edition_country() {
        let ranked = rank_artist_releases_with_country(
            vec![
                ProviderReleaseSummary {
                    id: "european".into(),
                    title: "Emotions".into(),
                    year: Some(1991),
                    kind: Some("release".into()),
                    artist_name: Some("Mariah Carey".into()),
                    country: None,
                    formats: Vec::new(),
                    catalog_number: None,
                    barcode: None,
                },
                ProviderReleaseSummary {
                    id: "japanese".into(),
                    title: "Emotions".into(),
                    year: Some(1991),
                    kind: Some("release".into()),
                    artist_name: Some("Mariah Carey".into()),
                    country: Some("JP".into()),
                    formats: vec!["CD".into()],
                    catalog_number: Some("SRCS 5672".into()),
                    barcode: Some("4988009606729".into()),
                },
            ],
            Some("Emotions"),
            Some("1991"),
            Some("JP"),
        );

        assert_eq!(ranked[0].id, "japanese");
    }

    #[test]
    fn provider_credibility_rejects_emotions_compilation() {
        let request = LookupRequest {
            artist_hint: Some("Mariah Carey".into()),
            album_hint: Some("Emotions".into()),
            tagged_album_hint: Some("Emotions".into()),
            folder_album_hint: Some("Emotions".into()),
            year_hint: Some("1991".into()),
            country_hint: Some("JP".into()),
            tracks: (1..=10)
                .map(|index| track(&format!("Song {index}"), "Mariah Carey"))
                .collect(),
            ..LookupRequest::default()
        };
        let compilation = AlbumCandidate {
            artist: Some("Mariah Carey".into()),
            album: Some("Emotions / Rainbow / Butterfly".into()),
            year: Some("2010".into()),
            country: Some("GB".into()),
            discogs_release_id: Some("2642561".into()),
            tracks: (1..=36)
                .map(|index| track(&format!("Song {index}"), "Mariah Carey"))
                .collect(),
            source: LookupSource::Discogs,
            ..AlbumCandidate::default()
        };

        assert!(provider_candidate_credibility(&request, &compilation).is_err());
    }

    #[test]
    fn emotions_japanese_release_is_credible_with_numeric_track_five_tag() {
        let root = temp_root();
        let album_path = root.join("Mariah Carey/1991 - Emotions [Japanese Edition]-tracks");
        fs::create_dir_all(&album_path).unwrap();
        let titles = [
            "Emotions",
            "And You Don't Remember",
            "Can't Let Go",
            "Make It Happen",
            "If It's Over",
            "You're So Cold",
            "So Blessed",
            "To Be Around You",
            "Till The End Of Time",
            "The Wind",
        ];
        let durations = [
            248.826, 266.0, 267.0, 307.0, 278.106, 305.0, 253.0, 277.0, 334.0, 282.0,
        ];
        let request = LookupRequest {
            path: album_path.to_string_lossy().into_owned(),
            artist_hint: Some("Mariah Carey".into()),
            album_hint: Some("Emotions".into()),
            tagged_album_hint: Some("Emotions".into()),
            folder_album_hint: Some("Emotions".into()),
            year_hint: Some("1991".into()),
            country_hint: Some("JP".into()),
            tracks: titles
                .iter()
                .enumerate()
                .map(|(index, title)| TrackCandidate {
                    title: Some(if index == 4 { "5".into() } else { (*title).into() }),
                    filename: Some(format!("{:02}. {title}", index + 1)),
                    track_number: Some(u32::try_from(index + 1).unwrap()),
                    length: Some(durations[index]),
                    ..TrackCandidate::default()
                })
                .collect(),
            ..LookupRequest::default()
        };
        for (index, title) in titles.iter().enumerate() {
            fs::write(
                album_path.join(format!("{:02}. {title}.flac", index + 1)),
                [],
            )
            .unwrap();
        }
        let candidate = AlbumCandidate {
            artist: Some("Mariah Carey".into()),
            artists: vec!["Mariah Carey".into()],
            album: Some("Emotions".into()),
            album_artist: Some("Mariah Carey".into()),
            album_artists: vec!["Mariah Carey".into()],
            year: Some("1991".into()),
            country: Some("JP".into()),
            musicbrainz_album_id: Some("e01b7fc8-7ead-3ee0-afbf-daabef5f0d04".into()),
            linked_discogs_release_id: Some("1521689".into()),
            tracks: titles
                .iter()
                .enumerate()
                .map(|(index, title)| TrackCandidate {
                    title: Some((*title).into()),
                    artist: Some("Mariah Carey".into()),
                    artists: vec!["Mariah Carey".into()],
                    track_number: Some(u32::try_from(index + 1).unwrap()),
                    length: Some(durations[index] * 1000.0),
                    ..TrackCandidate::default()
                })
                .collect(),
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };

        let selected = select_credible_provider_candidate(&request, vec![candidate]).unwrap();

        assert_eq!(
            selected.musicbrainz_album_id.as_deref(),
            Some("e01b7fc8-7ead-3ee0-afbf-daabef5f0d04")
        );
        assert_eq!(selected.discogs_release_id.as_deref(), Some("1521689"));
        assert_eq!(selected.tracks[4].title.as_deref(), Some("If It's Over"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn provider_diagnostics_aggregate_safe_candidate_rejection_codes() {
        let request = LookupRequest {
            artist_hint: Some("Mariah Carey".into()),
            album_hint: Some("Emotions".into()),
            tagged_album_hint: Some("Emotions".into()),
            tracks: vec![track("Emotions", "Mariah Carey")],
            ..LookupRequest::default()
        };
        let candidate = AlbumCandidate {
            artist: Some("Mariah Carey".into()),
            album: Some("Emotions / Rainbow / Butterfly".into()),
            source: LookupSource::Discogs,
            ..AlbumCandidate::default()
        };
        let diagnostics = provider_selection_diagnostics(
            &request,
            &[candidate],
            &[ProviderAttempt {
                provider: "discogs",
                status: ProviderAttemptStatus::NoMatch,
                diagnostic: None,
                retry_count: 0,
                retry_after_seconds: None,
            }],
        );

        assert_eq!(diagnostics[0]["candidateCounts"]["discogs"], 1);
        assert_eq!(diagnostics[0]["credibleCounts"]["discogs"], 0);
        assert_eq!(diagnostics[0]["rejectionCodes"]["title_conflict"], 1);
    }

    #[test]
    fn auto_tag_llm_output_budget_scales_with_album_track_count() {
        assert_eq!(auto_tag_llm_max_tokens(0), 2_048);
        assert_eq!(auto_tag_llm_max_tokens(8), 2_048);
        assert_eq!(auto_tag_llm_max_tokens(34), 5_376);
        assert_eq!(auto_tag_llm_max_tokens(100), 8_192);
    }

    #[test]
    fn edition_ranking_selects_japanese_musicbrainz_release_and_linked_discogs_id() {
        let tracks = (1..=10)
            .map(|index| track(&format!("Song {index}"), "Mariah Carey"))
            .collect::<Vec<_>>();
        let request = LookupRequest {
            artist_hint: Some("Mariah Carey".into()),
            album_hint: Some("Emotions".into()),
            tagged_album_hint: Some("Emotions".into()),
            year_hint: Some("1991".into()),
            country_hint: Some("JP".into()),
            tracks: tracks.clone(),
            ..LookupRequest::default()
        };
        let european = AlbumCandidate {
            artist: Some("Mariah Carey".into()),
            album: Some("Emotions".into()),
            year: Some("1991".into()),
            country: None,
            musicbrainz_album_id: Some("60b06354-f3b3-4d68-bea3-6331768608bb".into()),
            tracks: tracks.clone(),
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };
        let japanese = AlbumCandidate {
            country: Some("JP".into()),
            musicbrainz_album_id: Some("e01b7fc8-7ead-3ee0-afbf-daabef5f0d04".into()),
            linked_discogs_release_id: Some("1521689".into()),
            ..european.clone()
        };

        let selected =
            select_credible_provider_candidate(&request, vec![european, japanese]).unwrap();

        assert_eq!(
            selected.musicbrainz_album_id.as_deref(),
            Some("e01b7fc8-7ead-3ee0-afbf-daabef5f0d04")
        );
        assert_eq!(selected.discogs_release_id.as_deref(), Some("1521689"));
    }

    #[test]
    fn validated_artist_alias_can_pass_provider_credibility_gate() {
        let request = LookupRequest {
            artist_hint: Some("郑少秋".into()),
            artist_aliases: vec!["Adam Cheng".into()],
            album_hint: Some("Album".into()),
            tracks: vec![track("Song", "郑少秋")],
            ..LookupRequest::default()
        };
        let candidate = AlbumCandidate {
            artist: Some("Adam Cheng".into()),
            album: Some("Album".into()),
            tracks: vec![track("Song", "Adam Cheng")],
            source: LookupSource::Discogs,
            ..AlbumCandidate::default()
        };

        assert!(provider_candidate_credibility(&request, &candidate).is_ok());
    }

    #[test]
    fn provider_authority_matrix_applies_a_match_and_blocks_ai_on_outage() {
        let request = LookupRequest {
            artist_hint: Some("Artist".into()),
            album_hint: Some("Album".into()),
            tracks: vec![track("Song", "Artist")],
            ..LookupRequest::default()
        };
        let credible = AlbumCandidate {
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            tracks: vec![track("Song", "Artist")],
            musicbrainz_album_id: Some("release".into()),
            source: LookupSource::Musicbrainz,
            ..AlbumCandidate::default()
        };
        let outage = ProviderAttempt {
            provider: "discogs",
            status: ProviderAttemptStatus::Unavailable,
            diagnostic: Some("network failure".into()),
            retry_count: 0,
            retry_after_seconds: None,
        };

        assert!(matches!(
            provider_authority_decision(
                &request,
                vec![credible],
                std::slice::from_ref(&outage),
            ),
            ProviderAuthorityDecision::Selected(_)
        ));
        assert_eq!(
            provider_authority_decision(&request, Vec::new(), &[outage]),
            ProviderAuthorityDecision::Unavailable
        );
        assert_eq!(
            provider_authority_decision(
                &request,
                Vec::new(),
                &[ProviderAttempt {
                    provider: "musicbrainz",
                    status: ProviderAttemptStatus::NoMatch,
                    diagnostic: None,
                    retry_count: 0,
                    retry_after_seconds: None,
                }],
            ),
            ProviderAuthorityDecision::Providerless
        );
        assert_eq!(
            provider_authority_decision(&request, Vec::new(), &[]),
            ProviderAuthorityDecision::Providerless,
            "disabled providers allow validated AI fallback"
        );
    }

    #[tokio::test]
    async fn musicbrainz_scoped_detail_failure_is_unavailable() {
        let root = temp_root();
        let cache = CacheState::new(root.clone());
        assert!(cache.initialize(Some(root.join("cache.db").to_str().unwrap())));
        let request = LookupRequest {
            path: root.to_string_lossy().into_owned(),
            artist_hint: Some("Artist".into()),
            album_hint: Some("Album".into()),
            musicbrainz_artist_id: Some("mb-artist".into()),
            tracks: vec![track("Song", "Artist")],
            ..LookupRequest::default()
        };
        let base = scoped_detail_failure_server("musicbrainz");
        let client = MusicBrainzClient::at(ProviderState::new().http(), &base);

        assert!(musicbrainz_artist_candidates(&client, &cache, &request)
            .await
            .is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn discogs_scoped_detail_failure_is_unavailable() {
        let root = temp_root();
        let cache = CacheState::new(root.clone());
        assert!(cache.initialize(Some(root.join("cache.db").to_str().unwrap())));
        let request = LookupRequest {
            path: root.to_string_lossy().into_owned(),
            artist_hint: Some("Artist".into()),
            album_hint: Some("Album".into()),
            discogs_artist_id: Some("7".into()),
            tracks: vec![track("Song", "Artist")],
            ..LookupRequest::default()
        };
        let base = scoped_detail_failure_server("discogs");
        let client = DiscogsClient::at(ProviderState::new().http(), None, &base);

        assert!(discogs_artist_candidates(&client, &cache, &request)
            .await
            .is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn authoritative_ai_write_clears_all_stale_provider_ids() {
        let root = temp_root();
        let album = root.join("Artist/Album");
        fs::create_dir_all(&album).unwrap();
        let path = album.join("01.mp3");
        fs::copy(corpus_mp3(), &path).unwrap();
        let candidate = AlbumCandidate {
            artist: Some("Correct Artist".into()),
            artists: vec!["Correct Artist".into()],
            album_artist: Some("Correct Artist".into()),
            album_artists: vec!["Correct Artist".into()],
            album: Some("Correct Album".into()),
            tracks: vec![TrackCandidate {
                title: Some("Correct Title".into()),
                artist: Some("Correct Artist".into()),
                artists: vec!["Correct Artist".into()],
                track_number: Some(1),
                track_total: Some(1),
                disc_number: Some(1),
                disc_total: Some(1),
                ..TrackCandidate::default()
            }],
            source: LookupSource::Llm,
            ..AlbumCandidate::default()
        };

        apply_candidate_tags(&album, &candidate, &WriteQueue::default())
            .await
            .unwrap();

        let written = crate::commands::tracks::read_track_metadata(&path).unwrap();
        assert_eq!(written.musicbrainz_album_id, None);
        assert_eq!(written.musicbrainz_artist_id, None);
        assert_eq!(written.musicbrainz_track_id, None);
        assert_eq!(written.discogs_release_id, None);
        assert_eq!(written.discogs_artist_id, None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn authoritative_ai_requires_complete_tracks_and_threshold() {
        let request = LookupRequest {
            tracks: vec![track("Old One", "Old Artist"), track("Old Two", "Old Artist")],
            ..LookupRequest::default()
        };
        let valid = serde_json::json!({
            "artist": "Correct Artist",
            "artists": ["Correct Artist"],
            "albumArtist": "Correct Artist",
            "albumArtists": ["Correct Artist"],
            "album": "Correct Album",
            "year": "1991",
            "genre": "Pop, R&B",
            "confidence": 0.85,
            "tracks": [
                {"index": 0, "title": "One", "artist": "Correct Artist", "artists": ["Correct Artist"], "trackNumber": 1, "trackTotal": 2, "discNumber": 1, "discTotal": 1},
                {"index": 1, "title": "Two", "artist": "Correct Artist", "artists": ["Correct Artist"], "trackNumber": 2, "trackTotal": 2, "discNumber": 1, "discTotal": 1}
            ]
        });

        let accepted = validated_ai_candidate(&request, &valid).unwrap();
        assert_eq!(accepted.source, LookupSource::Llm);
        assert_eq!(accepted.album.as_deref(), Some("Correct Album"));
        assert_eq!(accepted.musicbrainz_album_id, None);
        assert_eq!(accepted.discogs_release_id, None);

        let mut numeric_string = valid.clone();
        numeric_string["confidence"] = serde_json::json!("0.95");
        assert!(validated_ai_candidate(&request, &numeric_string).is_err());

        let mut duplicate = valid.clone();
        duplicate["tracks"][1]["index"] = serde_json::json!(0);
        assert!(validated_ai_candidate(&request, &duplicate).is_err());

        let mut invalid_count = valid.clone();
        invalid_count["tracks"][0]["trackTotal"] = serde_json::json!(1);
        assert!(validated_ai_candidate(&request, &invalid_count).is_err());

        let mut invalid_number = valid.clone();
        invalid_number["tracks"][1]["trackNumber"] = serde_json::json!(99);
        assert!(validated_ai_candidate(&request, &invalid_number).is_err());

        let mut empty_artist = valid.clone();
        empty_artist["tracks"][0]["artist"] = serde_json::json!("");
        assert!(validated_ai_candidate(&request, &empty_artist).is_err());

        let mut low = valid;
        low["confidence"] = serde_json::json!(0.849);
        assert_eq!(
            validated_ai_candidate(&request, &low).unwrap_err().code,
            "ai_low_confidence"
        );
    }

    #[test]
    fn ai_error_diagnostics_do_not_include_provider_bodies_or_model_output() {
        let http = OpenRouterError::Http {
            status: 429,
            body: "secret-response-body".into(),
        };
        let malformed = OpenRouterError::MalformedJson {
            finish_reason: "stop".into(),
            message: "model-private-output".into(),
        };

        let http_failure = ai_validation_failure_from_error(&http);
        let malformed_failure = ai_validation_failure_from_error(&malformed);

        assert_eq!(http_failure.code, "ai_failed");
        assert_eq!(http_failure.detail, "AI provider returned HTTP 429");
        assert!(!http_failure.detail.contains("secret-response-body"));
        assert_eq!(malformed_failure.code, "ai_malformed");
        assert_eq!(malformed_failure.detail, "AI response was malformed JSON");
        assert!(!malformed_failure.detail.contains("model-private-output"));
    }

}
