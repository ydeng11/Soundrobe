//! Auto-tag candidate contracts and, once complete, task orchestration.

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    commands::{
        covers::download_album_artwork_at,
        library::collect_audio_files,
        lyrics::{apply_album_lyrics_at, DEFAULT_BASE_URL},
        mutations::{write_track_queued, TrackPatch},
        tracks::read_album,
    },
    error::ApiError,
    infra::openrouter::{ChatMessage, OpenRouterClient},
    state::{
        config::AutoTagConfig,
        providers::{
            album_names_match, convert_chinese_text, DiscogsClient, MusicBrainzClient,
            ProviderAlbum, ProviderReleaseSummary, ProviderState, RemoteArtworkClient,
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LookupRequest {
    pub path: String,
    pub artist_hint: Option<String>,
    pub album_hint: Option<String>,
    pub year_hint: Option<String>,
    pub musicbrainz_album_id: Option<String>,
    pub musicbrainz_artist_id: Option<String>,
    pub discogs_release_id: Option<String>,
    pub discogs_artist_id: Option<String>,
    pub tracks: Vec<TrackCandidate>,
}

pub fn build_lookup_request(album_path: &Path) -> Result<LookupRequest, ApiError> {
    let detail = read_album(album_path)?;
    let supplied_folder = album_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let cd_subfolder = Regex::new(r"(?i)^(?:cd|disc|disk|ディスク)\s*\d+\s*$")
        .expect("valid CD subfolder regex")
        .is_match(supplied_folder);
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
    let year_hint = extract_folder_year(folder_name);
    let tagged_artist = detail
        .tracks
        .iter()
        .find_map(|track| track.artist.clone().or_else(|| track.album_artist.clone()));
    let tagged_album = detail.tracks.iter().find_map(|track| track.album.clone());
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
    let artist_hint = if is_compilation_folder(&folder_artist) {
        Some("Various Artists".to_string())
    } else if tagged_artist
        .as_deref()
        .is_some_and(|artist| !artist.eq_ignore_ascii_case(&folder_artist))
    {
        non_empty(folder_artist)
    } else {
        tagged_artist.or_else(|| non_empty(folder_artist))
    };
    let total = u32::try_from(detail.tracks.len()).ok();
    let tracks = detail
        .tracks
        .into_iter()
        .enumerate()
        .map(|(index, track)| {
            let filename_number = Path::new(&track.path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(filename_track_number);
            TrackCandidate {
                title: track.title,
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
        album_hint: non_empty(folder_album).or(tagged_album),
        year_hint: year_hint.or(tagged_year),
        musicbrainz_album_id,
        musicbrainz_artist_id,
        discogs_release_id,
        discogs_artist_id,
        tracks,
    })
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn extract_folder_year(name: &str) -> Option<String> {
    Regex::new(r"^((?:19|20)\d{2})(?:\s*[-.]|[^\d]|$)")
        .expect("valid folder year regex")
        .captures(name)
        .and_then(|captures| captures.get(1))
        .map(|year| year.as_str().to_string())
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

pub fn folder_candidate(request: &LookupRequest) -> AlbumCandidate {
    let artist = request.artist_hint.clone();
    let album_artist = if artist.as_deref().is_some_and(is_compilation_folder) {
        Some("Various Artists".to_string())
    } else {
        artist
    };
    let album_artists = album_artist.iter().cloned().collect::<Vec<_>>();
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
        tracks: request.tracks.clone(),
        source: LookupSource::Folder,
        ..AlbumCandidate::default()
    }
}

fn candidate_priority(candidate: &AlbumCandidate) -> i8 {
    if candidate.musicbrainz_album_id.is_some() || candidate.discogs_release_id.is_some() {
        return -1;
    }
    match candidate.source {
        LookupSource::Musicbrainz => 0,
        LookupSource::Discogs => 1,
        LookupSource::Llm => 2,
        LookupSource::Folder => 3,
        _ => 10,
    }
}

pub fn merge_candidate_fields(candidates: Vec<AlbumCandidate>) -> Vec<AlbumCandidate> {
    let Some((preferred_index, preferred)) = candidates
        .iter()
        .enumerate()
        .min_by_key(|(_, candidate)| candidate_priority(candidate))
    else {
        return Vec::new();
    };
    let mut merged = AlbumCandidate {
        source: preferred.source,
        verification: preferred.verification.clone(),
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
    let old_album_artist = candidate
        .artist
        .clone()
        .or_else(|| candidate.album_artist.clone());
    candidate.artist = Some(canonical_name.clone());
    candidate.artists = vec![canonical_name.clone()];
    candidate.album_artist = Some(canonical_name.clone());
    candidate.album_artists = vec![canonical_name.clone()];
    for track in &mut candidate.tracks {
        let is_solo = old_album_artist
            .as_ref()
            .is_none_or(|artist| track.artist.as_ref() == Some(artist));
        if is_solo {
            track.artist = Some(canonical_name.clone());
            track.artists = vec![canonical_name.clone()];
        }
    }
    candidate
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
    album_hint: &'a Option<String>,
    musicbrainz_album_id: &'a Option<String>,
    musicbrainz_artist_id: &'a Option<String>,
    discogs_release_id: &'a Option<String>,
    discogs_artist_id: &'a Option<String>,
    tracks: Vec<HashTrack<'a>>,
    track_count: usize,
}

pub fn query_hash(request: &LookupRequest) -> String {
    let query = HashQuery {
        cache_version: 3,
        artist_hint: &request.artist_hint,
        album_hint: &request.album_hint,
        musicbrainz_album_id: &request.musicbrainz_album_id,
        musicbrainz_artist_id: &request.musicbrainz_artist_id,
        discogs_release_id: &request.discogs_release_id,
        discogs_artist_id: &request.discogs_artist_id,
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
    let matched = match_remote_candidate_tracks(
        &request.tracks,
        &filenames,
        &candidate.tracks,
        source,
        &artist_hints,
        &[],
    );
    let mut protected = candidate.clone();
    protected.tracks = matched.tracks;
    protected
}

pub fn combine_candidate_sources(
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
    merge_candidate_fields(fresh)
}

pub fn filter_candidates_for_album(
    album_hint: Option<&str>,
    candidates: Vec<AlbumCandidate>,
) -> Vec<AlbumCandidate> {
    candidates
        .into_iter()
        .filter(|candidate| {
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
    let mut ranked = releases
        .into_iter()
        .filter(|release| album_hint.is_none_or(|hint| album_names_match(hint, &release.title)))
        .map(|release| {
            let year_score = match (year_hint, release.year) {
                (Some(hint), Some(year)) if hint == year.to_string() => 1,
                _ => 0,
            };
            (year_score, release)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| right.year.cmp(&left.year))
            .then_with(|| left.title.cmp(&right.title))
    });
    ranked.into_iter().map(|(_, release)| release).collect()
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

async fn musicbrainz_artist_candidates(
    client: &MusicBrainzClient,
    cache: &CacheState,
    request: &LookupRequest,
) -> Vec<AlbumCandidate> {
    let Some(artist_id) = request.musicbrainz_artist_id.as_deref() else {
        return Vec::new();
    };
    let releases = cached_artist_releases(cache, "musicbrainz", artist_id).unwrap_or_default();
    let releases = if releases.is_empty() {
        let fetched = client.artist_release_page(artist_id, 1, 100).await;
        if let Ok(value) = serde_json::to_value(&fetched) {
            let _ = cache.set_artist_releases("musicbrainz", artist_id, 1, &value);
        }
        fetched
    } else {
        releases
    };
    let mut candidates = Vec::new();
    for release in rank_artist_releases(
        releases,
        request.album_hint.as_deref(),
        request.year_hint.as_deref(),
    )
    .into_iter()
    .take(3)
    {
        let album = cached_release_detail(cache, "musicbrainz-v3", &release.id);
        let album = match album {
            Some(album) => Some(album),
            None => {
                let fetched = client.release_by_id(&release.id).await;
                if let Some(album) = &fetched {
                    if let Ok(value) = serde_json::to_value(album) {
                        let _ = cache.set_release_detail("musicbrainz-v3", &release.id, &value);
                    }
                }
                fetched
            }
        };
        if let Some(album) = album {
            candidates.push(musicbrainz_candidate(album));
        }
    }
    rank_candidate_details(candidates, request, "musicbrainz")
}

async fn discogs_artist_candidates(
    client: &DiscogsClient,
    cache: &CacheState,
    request: &LookupRequest,
) -> Vec<AlbumCandidate> {
    let Some(artist_id) = request.discogs_artist_id.as_deref() else {
        return Vec::new();
    };
    let releases = cached_artist_releases(cache, "discogs", artist_id).unwrap_or_default();
    let releases = if releases.is_empty() {
        let fetched = client.artist_release_page(artist_id, 1, 100).await;
        if let Ok(value) = serde_json::to_value(&fetched) {
            let _ = cache.set_artist_releases("discogs", artist_id, 1, &value);
        }
        fetched
    } else {
        releases
    };
    let mut candidates = Vec::new();
    for release in rank_artist_releases(
        releases,
        request.album_hint.as_deref(),
        request.year_hint.as_deref(),
    )
    .into_iter()
    .take(3)
    {
        let album = cached_release_detail(cache, "discogs-v2", &release.id);
        let album = match album {
            Some(album) => Some(album),
            None => {
                let fetched = client.release_metadata(&release.id).await;
                if let Some(album) = &fetched {
                    if let Ok(value) = serde_json::to_value(album) {
                        let _ = cache.set_release_detail("discogs-v2", &release.id, &value);
                    }
                }
                fetched
            }
        };
        if let Some(album) = album {
            candidates.push(discogs_candidate(album));
        }
    }
    rank_candidate_details(candidates, request, "discogs")
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

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoTagRunResult {
    pub candidate: AlbumCandidate,
    pub written: usize,
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

#[derive(Clone, Debug, PartialEq)]
pub struct LlmTagResolution {
    pub corrected_request: LookupRequest,
    pub fallback: AlbumCandidate,
}

pub fn genre_from_value(value: &serde_json::Value) -> Option<String> {
    let confidence = value.get("confidence")?.as_f64()?;
    if confidence < 0.6 {
        return None;
    }
    llm_string(value.get("genre"))
}

pub fn llm_resolution_from_value(
    request: &LookupRequest,
    value: &serde_json::Value,
) -> LlmTagResolution {
    let corrected_artist = llm_string(value.get("artist")).or_else(|| request.artist_hint.clone());
    let corrected_album = llm_string(value.get("album")).or_else(|| request.album_hint.clone());
    let corrected_year = llm_string(value.get("year")).or_else(|| request.year_hint.clone());
    let album_artist = llm_string(value.get("albumArtist")).or_else(|| corrected_artist.clone());
    let llm_tracks = normalize_llm_tracks(value.get("tracks"), request.tracks.len());
    let tracks: Vec<TrackCandidate> = request
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let correction = llm_tracks.get(index).copied();
            let mut track = track.clone();
            if let Some(title) = correction.and_then(|track| llm_string(track.get("title"))) {
                track.title = Some(title);
            }
            if let Some(artist) = correction
                .and_then(|track| llm_string(track.get("artist")))
                .or_else(|| track.artist.clone())
                .or_else(|| corrected_artist.clone())
            {
                track.artist = Some(artist);
            }
            track
        })
        .collect();
    let mut corrected_request = request.clone();
    corrected_request.artist_hint = corrected_artist.clone();
    corrected_request.album_hint = corrected_album.clone();
    corrected_request.year_hint = corrected_year.clone();
    corrected_request.tracks = tracks.clone();
    let album_artists = album_artist.iter().cloned().collect::<Vec<_>>();

    LlmTagResolution {
        corrected_request,
        fallback: AlbumCandidate {
            artist: corrected_artist.clone(),
            artists: corrected_artist.iter().cloned().collect(),
            album: corrected_album,
            album_artist,
            album_artists,
            year: corrected_year,
            genre: llm_string(value.get("genre")),
            musicbrainz_album_id: request.musicbrainz_album_id.clone(),
            musicbrainz_artist_id: request.musicbrainz_artist_id.clone(),
            discogs_release_id: request.discogs_release_id.clone(),
            discogs_artist_id: request.discogs_artist_id.clone(),
            tracks,
            source: LookupSource::Llm,
            ..AlbumCandidate::default()
        },
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

fn normalize_llm_tracks(
    value: Option<&serde_json::Value>,
    expected_count: usize,
) -> Vec<&serde_json::Map<String, serde_json::Value>> {
    let mut tracks = value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_object)
        .collect::<Vec<_>>();
    let indices = tracks
        .iter()
        .filter_map(|track| track.get("index").and_then(serde_json::Value::as_i64))
        .collect::<Vec<_>>();
    let unique = indices
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    let contiguous_one_based = indices.iter().min() == Some(&1)
        && indices.iter().max() == i64::try_from(expected_count).ok().as_ref()
        && unique == expected_count;
    let contiguous_zero_based =
        indices.iter().min() == Some(&0) && unique >= tracks.len().min(expected_count);
    if contiguous_one_based || contiguous_zero_based {
        tracks.sort_by_key(|track| {
            track
                .get("index")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(i64::MAX)
        });
    }
    tracks.truncate(expected_count);
    tracks
}

async fn resolve_tags_via_llm(
    request: &LookupRequest,
    config: &AutoTagConfig,
    cancelled: &AtomicBool,
) -> Option<LlmTagResolution> {
    let api_key = config
        .llm_api_key
        .as_deref()
        .filter(|key| !key.is_empty())?;
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
            "artist": track.artist,
            "track_number": track.track_number,
            "genre": track.genre,
        })).collect::<Vec<_>>(),
    });
    let messages = vec![
        ChatMessage::system(concat!(
            "Resolve correct music metadata from folder structure, parser hints, and existing tags. ",
            "Return only JSON with artist, albumArtist, album, year, genre, tracks, and confidence. ",
            "Strip year and format annotations from album names. Preserve uncertain fields as null. ",
            "Use Various Artists only for true compilations. Per-track entries use index, title, artist. ",
            "Do not invent provider IDs. Genre should use conservative Discogs-style comma-separated tags."
        )),
        ChatMessage::user(payload.to_string()),
    ];
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "artist": {"type": ["string", "null"]},
            "albumArtist": {"type": ["string", "null"]},
            "album": {"type": ["string", "null"]},
            "year": {"type": ["string", "null"]},
            "genre": {"type": ["string", "null"]},
            "tracks": {"type": "array", "items": {"type": "object", "properties": {
                "index": {"type": "number"},
                "title": {"type": ["string", "null"]},
                "artist": {"type": ["string", "null"]}
            }}},
            "confidence": {"type": "number"}
        },
        "required": ["artist", "albumArtist", "album", "year", "genre", "tracks", "confidence"]
    });
    OpenRouterClient::new(api_key, model)
        .complete_json(messages, "TagCorrectionResponse", schema, cancelled)
        .await
        .ok()
        .map(|response| llm_resolution_from_value(request, &response.data))
}

async fn fill_genre_if_missing(
    candidate: &AlbumCandidate,
    request: &LookupRequest,
    config: &AutoTagConfig,
    cancelled: &AtomicBool,
) -> AlbumCandidate {
    if candidate.genre.is_some() {
        return candidate.clone();
    }
    let Some(api_key) = config.llm_api_key.as_deref().filter(|key| !key.is_empty()) else {
        return candidate.clone();
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
    let Ok(response) = OpenRouterClient::new(api_key, model)
        .with_generation(0.2, 256)
        .complete_json(messages, "GenreFillResponse", schema, cancelled)
        .await
    else {
        return candidate.clone();
    };
    let Some(genre) = genre_from_value(&response.data) else {
        return candidate.clone();
    };
    let mut filled = candidate.clone();
    filled.genre = Some(genre);
    filled
}

pub async fn resolve_and_apply_album(
    album_path: &Path,
    config: &AutoTagConfig,
    providers: &ProviderState,
    cache: &CacheState,
    queue: &WriteQueue,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(u64, &str),
) -> Result<AutoTagRunResult, ApiError> {
    progress(1, "Parsing folder hints...");
    let mut request = build_lookup_request(album_path)?;
    check_cancelled(cancelled)?;

    progress(3, "Checking cache...");
    let hash = query_hash(&request);
    let cached = cache
        .lookup(&hash)
        .and_then(|value| serde_json::from_value::<Vec<AlbumCandidate>>(value).ok())
        .unwrap_or_default();
    let mut fresh = Vec::new();

    progress(4, "Direct provider ID lookup...");
    let musicbrainz = MusicBrainzClient::new(providers.http());
    if let Some(release_id) = request.musicbrainz_album_id.as_deref() {
        if let Some(album) = musicbrainz.release_by_id(release_id).await {
            fresh.push(musicbrainz_candidate(album));
        }
    }
    let has_direct_musicbrainz = fresh.iter().any(|candidate| {
        candidate.source == LookupSource::Musicbrainz
            && candidate.musicbrainz_album_id == request.musicbrainz_album_id
    });
    let discogs = DiscogsClient::new(providers.http(), config.discogs_token.clone());
    if let Some(release_id) = request.discogs_release_id.as_deref() {
        if let Some(album) = discogs.release_metadata(release_id).await {
            fresh.push(discogs_candidate(album));
        }
    }
    check_cancelled(cancelled)?;

    let ambiguous = hints_are_ambiguous(
        request.album_hint.as_deref(),
        request.artist_hint.as_deref(),
        &request.path,
        request.year_hint.as_deref(),
    );
    let mut llm_fallback = None;
    if !has_direct_musicbrainz && ambiguous {
        if let Some(resolution) = resolve_tags_via_llm(&request, config, cancelled).await {
            request = resolution.corrected_request;
            llm_fallback = Some(resolution.fallback);
        }
    }

    if !has_direct_musicbrainz && config.remote_lookup_enabled != Some(false) {
        progress(5, "Searching MusicBrainz...");
        let scoped = musicbrainz_artist_candidates(&musicbrainz, cache, &request).await;
        if scoped.is_empty() {
            if let (Some(artist), Some(album)) = (
                request.artist_hint.as_deref(),
                request.album_hint.as_deref(),
            ) {
                fresh.extend(
                    musicbrainz
                        .search_album(artist, album, 5)
                        .await
                        .into_iter()
                        .map(musicbrainz_candidate),
                );
            }
        } else {
            fresh.extend(scoped);
        }
    }
    check_cancelled(cancelled)?;

    if !has_direct_musicbrainz && config.discogs_enabled != Some(false) {
        progress(6, "Searching Discogs releases...");
        let scoped = discogs_artist_candidates(&discogs, cache, &request).await;
        if scoped.is_empty() {
            if let (Some(artist), Some(album)) = (
                request.artist_hint.as_deref(),
                request.album_hint.as_deref(),
            ) {
                fresh.extend(
                    discogs
                        .search_album(artist, album, 3)
                        .await
                        .into_iter()
                        .map(discogs_candidate),
                );
            }
        } else {
            fresh.extend(scoped);
        }
    }
    check_cancelled(cancelled)?;

    if !has_direct_musicbrainz && !ambiguous && fresh.is_empty() && cached.is_empty() {
        if let Some(resolution) = resolve_tags_via_llm(&request, config, cancelled).await {
            request = resolution.corrected_request;
            llm_fallback = Some(resolution.fallback);
        }
    }

    progress(7, "Building fallback...");
    if let Some(fallback) = llm_fallback {
        fresh.push(fallback);
    }
    let folder = folder_candidate(&request);
    fresh = filter_candidates_for_album(request.album_hint.as_deref(), fresh);
    let cached = filter_candidates_for_album(request.album_hint.as_deref(), cached);
    let mut cache_payload = fresh.clone();
    cache_payload.push(folder.clone());
    if should_replace_lookup_cache(&cache_payload, !cached.is_empty()) {
        if let (Ok(query), Ok(response)) = (
            serde_json::to_value(&request),
            serde_json::to_value(&cache_payload),
        ) {
            let source = cache_payload
                .first()
                .map(|candidate| candidate.source)
                .unwrap_or(LookupSource::Folder);
            let write_hash = query_hash(&request);
            let _ = cache.set_lookup(&write_hash, &query, &response, lookup_source_name(source));
        }
    }
    let fresh = fresh
        .into_iter()
        .map(|candidate| protect_candidate_tracks(&request, &candidate))
        .collect();
    let cached = cached
        .into_iter()
        .map(|candidate| protect_candidate_tracks(&request, &candidate))
        .collect();
    let candidates = combine_candidate_sources(fresh, cached, folder);
    let candidate = candidates
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::Message("No auto-tag candidate available".to_string()))?;
    check_cancelled(cancelled)?;
    progress(8, "Resolving genre...");
    let candidate = protect_candidate_tracks(&request, &candidate);
    let candidate = fill_genre_if_missing(&candidate, &request, config, cancelled).await;
    check_cancelled(cancelled)?;
    progress(9, "Applying tags...");
    let candidate = convert_candidate_chinese(&candidate, config.chinese_script.as_deref());
    let written = apply_candidate_tags(album_path, &candidate, queue).await?;
    if config.remote_lookup_enabled != Some(false)
        || config.discogs_enabled != Some(false)
        || config.theaudiodb_api_key.is_some()
    {
        let remote = RemoteArtworkClient::new(
            providers.http(),
            config.discogs_token.clone(),
            config.theaudiodb_api_key.clone(),
        );
        let _ = download_album_artwork_at(album_path, &remote, queue).await;
    }
    check_cancelled(cancelled)?;
    let lyrics_url = if config.lyrics_download_enabled == Some(true) {
        Some(config.lyrics_api_url.as_deref().unwrap_or(DEFAULT_BASE_URL))
    } else {
        None
    };
    let _ = apply_album_lyrics_at(album_path, lyrics_url, queue).await;
    Ok(AutoTagRunResult { candidate, written })
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
        let config = app.state::<crate::state::config::ConfigState>().raw();
        let providers = app.state::<ProviderState>();
        let cache = app.state::<CacheState>();
        let queue = app.state::<WriteQueue>();
        let progress_app = app.clone();
        let progress_task_id = spawned_task_id.clone();
        let operation = resolve_and_apply_album(
            &path,
            &config,
            &providers,
            &cache,
            &queue,
            &cancelled,
            move |step, message| {
                let tasks = progress_app.state::<TaskRegistry>();
                if tasks.update(&progress_task_id, step, message) {
                    let _ = progress_app.emit(
                        "auto-tag:event",
                        auto_tag_event(&progress_task_id, "progress", message, step, None),
                    );
                }
            },
        )
        .await;

        match operation {
            Ok(result) => {
                let data = serde_json::to_value(&result.candidate).unwrap_or_default();
                tasks.finish(
                    &spawned_task_id,
                    TaskStatus::Completed,
                    "Complete",
                    data.clone(),
                );
                let _ = app.emit(
                    "auto-tag:event",
                    auto_tag_event(&spawned_task_id, "completed", "Complete", 9, Some(data)),
                );
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
    insert_option(&mut album_fields, "year", &candidate.year);
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
        let mut fields = album_fields.clone();
        if let Some(track) = candidate.tracks.get(index) {
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
        let patch: TrackPatch = serde_json::from_value(fields.into())
            .map_err(|error| ApiError::WriteTask(error.to_string()))?;
        match write_track_queued(queue, file_path.clone().into(), patch).await {
            Ok(()) => written += 1,
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
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "auto-tagger-auto-tag-{}-{}",
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

    fn track(title: &str, artist: &str) -> TrackCandidate {
        TrackCandidate {
            title: Some(title.into()),
            artist: Some(artist.into()),
            artists: vec![artist.into()],
            ..TrackCandidate::default()
        }
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
    fn query_hash_is_stable_and_ignores_path_and_year() {
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
        relocated.year_hint = Some("2025".into());

        assert_eq!(query_hash(&request), query_hash(&relocated));
        assert_eq!(
            query_hash(&request),
            "9c500ff3beebc2b3057c76ec6e97a04127e6e3c34fc4b91cc39f449ae3567939"
        );
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
        });

        assert_eq!(candidate.source, LookupSource::Discogs);
        assert_eq!(candidate.discogs_release_id.as_deref(), Some("42"));
        assert_eq!(candidate.discogs_artist_id.as_deref(), Some("7"));
        assert_eq!(candidate.genre.as_deref(), Some("Rock, Indie Rock"));
        assert_eq!(candidate.tracks[0].track_total, Some(1));
        assert_eq!(candidate.tracks[0].length, Some(202.0));
    }

    #[test]
    fn lookup_request_reads_real_tags_but_keeps_mismatching_folder_identity() {
        let root = temp_root();
        let album = root.join("Folder Artist/2004 - Folder Album [FLAC]");
        fs::create_dir_all(&album).unwrap();
        fs::copy(corpus_mp3(), album.join("01.mp3")).unwrap();
        fs::copy(corpus_mp3(), album.join("02.mp3")).unwrap();

        let request = build_lookup_request(&album).unwrap();

        assert_eq!(request.artist_hint.as_deref(), Some("Folder Artist"));
        assert_eq!(request.album_hint.as_deref(), Some("Folder Album"));
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

        assert_eq!(request.artist_hint.as_deref(), Some("Folder Artist"));
        assert_eq!(request.album_hint.as_deref(), Some("Folder Album"));
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

        let filtered = filter_candidates_for_album(Some("無限"), candidates);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].album.as_deref(), Some("无限"));
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
            },
            ProviderReleaseSummary {
                id: "old".into(),
                title: "無限".into(),
                year: Some(2004),
                kind: Some("release".into()),
                artist_name: Some("Artist".into()),
            },
            ProviderReleaseSummary {
                id: "requested".into(),
                title: "无限".into(),
                year: Some(2005),
                kind: Some("release".into()),
                artist_name: Some("Artist".into()),
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
    fn llm_resolution_normalizes_one_based_tracks_and_null_sentinels() {
        let request = LookupRequest {
            artist_hint: Some("Parsed Artist".into()),
            album_hint: Some("Parsed Album".into()),
            year_hint: Some("2000".into()),
            tracks: vec![
                track("Old One", "Parsed Artist"),
                track("Old Two", "Parsed Artist"),
            ],
            ..LookupRequest::default()
        };
        let value = serde_json::json!({
            "artist": "Correct Artist",
            "albumArtist": "Correct Artist",
            "album": "Correct Album",
            "year": "unknown",
            "genre": "Rock, Indie Rock",
            "tracks": [
                {"index": 2, "title": "Second", "artist": "Correct Artist feat. Guest"},
                {"index": 1, "title": "First", "artist": null}
            ],
            "confidence": 0.9
        });

        let resolution = llm_resolution_from_value(&request, &value);

        assert_eq!(
            resolution.corrected_request.artist_hint.as_deref(),
            Some("Correct Artist")
        );
        assert_eq!(
            resolution.corrected_request.year_hint.as_deref(),
            Some("2000")
        );
        assert_eq!(
            resolution.corrected_request.tracks[0].title.as_deref(),
            Some("First")
        );
        assert_eq!(resolution.fallback.source, LookupSource::Llm);
        assert_eq!(
            resolution.fallback.genre.as_deref(),
            Some("Rock, Indie Rock")
        );
        assert_eq!(
            resolution.fallback.tracks[0].title.as_deref(),
            Some("First")
        );
        assert_eq!(
            resolution.fallback.tracks[0].artist.as_deref(),
            Some("Parsed Artist")
        );
        assert_eq!(
            resolution.fallback.tracks[1].title.as_deref(),
            Some("Second")
        );
        assert_eq!(
            resolution.fallback.tracks[1].artist.as_deref(),
            Some("Correct Artist feat. Guest")
        );
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

        let result = resolve_and_apply_album(
            &album,
            &config,
            &ProviderState::new(),
            &cache,
            &queue,
            &cancelled,
            |step, message| updates.push((step, message.to_string())),
        )
        .await
        .unwrap();

        assert_eq!(result.candidate.source, LookupSource::Folder);
        assert_eq!(result.written, 1);
        assert_eq!(updates.first().unwrap().0, 1);
        assert_eq!(updates.last().unwrap().0, 9);
        let request = build_lookup_request(&album).unwrap();
        assert!(cache.lookup(&query_hash(&request)).is_some());
        let written = crate::commands::tracks::read_track_metadata(&track_path).unwrap();
        assert_eq!(written.artist.as_deref(), Some("Corpus Artist"));
        assert_eq!(written.album_artist.as_deref(), Some("Folder Artist"));
        assert_eq!(written.album.as_deref(), Some("Folder Album"));
        fs::remove_dir_all(root).unwrap();
    }
}
