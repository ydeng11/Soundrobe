//! Auto-tag candidate contracts and, once complete, task orchestration.

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::{
    commands::{
        library::collect_audio_files,
        mutations::{write_track_queued, TrackPatch},
        tracks::read_album,
    },
    error::ApiError,
    state::{providers::ProviderAlbum, write_queue::WriteQueue},
};

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

#[derive(Clone, Debug, Default, PartialEq)]
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
            Err(error) => tracing::warn!(path = %file_path, %error, "auto-tag write failed"),
        }
    }
    Ok(written)
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
            artist_id: None,
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
}
