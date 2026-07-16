//! Auto-tag candidate contracts and, once complete, task orchestration.

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::state::providers::ProviderAlbum;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
