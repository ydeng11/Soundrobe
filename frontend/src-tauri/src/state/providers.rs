//! Shared provider foundation for remote metadata and artwork.
//!
//! Providers own no mutable request state; the managed client is cheaply cloneable,
//! so no state lock is held while a request is in flight.

use opencc_fmmseg::OpenCC;
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

const USER_AGENT: &str = "auto-tagger/0.1.0";
const DISCOGS_BASE: &str = "https://api.discogs.com";
static OPENCC: OnceLock<OpenCC> = OnceLock::new();
static DISCOGS_LIMITER: OnceLock<Arc<DiscogsRateLimiter>> = OnceLock::new();
static MUSICBRAINZ_LAST_REQUEST: OnceLock<tokio::sync::Mutex<Option<Instant>>> = OnceLock::new();

pub fn convert_chinese_text(value: &str, target: &str) -> String {
    let converter = OPENCC.get_or_init(OpenCC::new);
    match target {
        "traditional" => converter.convert(value, "s2t", false),
        "simplified" => converter.convert(value, "t2s", false),
        _ => value.to_string(),
    }
}

struct DiscogsRateLimiter {
    timestamps: tokio::sync::Mutex<Vec<Instant>>,
    maximum: AtomicUsize,
}

impl DiscogsRateLimiter {
    fn shared(token_present: bool) -> Arc<Self> {
        let limiter = Arc::clone(DISCOGS_LIMITER.get_or_init(|| {
            Arc::new(Self {
                timestamps: tokio::sync::Mutex::new(Vec::new()),
                maximum: AtomicUsize::new(25),
            })
        }));
        if token_present {
            limiter.maximum.store(60, Ordering::Relaxed);
        }
        limiter
    }

    async fn wait(&self) {
        loop {
            let delay = {
                let mut timestamps = self.timestamps.lock().await;
                let now = Instant::now();
                timestamps
                    .retain(|timestamp| now.duration_since(*timestamp) < Duration::from_secs(60));
                if timestamps.len() < self.maximum.load(Ordering::Relaxed) {
                    timestamps.push(now);
                    return;
                }
                Duration::from_secs(60).saturating_sub(now.duration_since(timestamps[0]))
                    + Duration::from_millis(100)
            };
            tokio::time::sleep(delay).await;
        }
    }
}

pub struct ProviderState {
    http: Client,
}

impl ProviderState {
    pub fn new() -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(USER_AGENT)
            .build()
            .expect("reqwest RustLS client configuration is valid");
        Self { http }
    }

    pub fn http(&self) -> Client {
        self.http.clone()
    }
}

impl Default for ProviderState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTrack {
    pub title: Option<String>,
    pub match_titles: Vec<String>,
    pub artist: Option<String>,
    pub artists: Vec<String>,
    pub track_number: Option<u32>,
    pub track_total: Option<u32>,
    pub disc_number: Option<u32>,
    pub recording_id: Option<String>,
    /// Raw provider duration, preserving Electron's provider-specific units.
    pub length: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAlbum {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub artists: Vec<String>,
    pub artist_id: Option<String>,
    pub year: Option<String>,
    pub genre: Option<String>,
    pub tracks: Vec<ProviderTrack>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderReleaseSummary {
    pub id: String,
    pub title: String,
    pub year: Option<u32>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub artist_name: Option<String>,
}

pub struct MusicBrainzClient {
    base_url: String,
    http: Client,
}

impl MusicBrainzClient {
    pub fn new(http: Client) -> Self {
        Self::at(http, "https://musicbrainz.org/ws/2")
    }

    pub fn at(http: Client, base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        }
    }

    pub async fn release_by_id(&self, release_id: &str) -> Option<ProviderAlbum> {
        wait_for_musicbrainz().await;
        let response = self
            .http
            .get(format!("{}/release/{release_id}", self.base_url))
            .query(&[("fmt", "json"), ("inc", "recordings+artist-credits")])
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .json::<serde_json::Value>()
            .await
            .ok()?;
        parse_musicbrainz_release(&response, release_id)
    }

    pub async fn search_album(
        &self,
        artist: &str,
        album: &str,
        max_candidates: usize,
    ) -> Vec<ProviderAlbum> {
        if artist.is_empty() || album.is_empty() || max_candidates == 0 {
            return Vec::new();
        }
        wait_for_musicbrainz().await;
        let query = format!(
            "artist:\"{}\" AND release:\"{}\"",
            escape_musicbrainz_query(artist),
            escape_musicbrainz_query(album)
        );
        let response = self
            .http
            .get(format!("{}/release", self.base_url))
            .query(&[
                ("query", query),
                ("fmt", "json".to_string()),
                ("limit", max_candidates.min(25).to_string()),
            ])
            .send()
            .await;
        let Ok(response) = response.and_then(reqwest::Response::error_for_status) else {
            return Vec::new();
        };
        let Ok(body) = response.json::<serde_json::Value>().await else {
            return Vec::new();
        };
        let releases = body
            .get("releases")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .take(max_candidates)
            .filter_map(|release| {
                let id = release.get("id")?.as_str()?.to_string();
                Some((id, parse_musicbrainz_search_release(release)))
            })
            .collect::<Vec<_>>();
        let mut albums = Vec::new();
        for (release_id, fallback) in releases {
            if let Some(album) = self.release_by_id(&release_id).await.or(fallback) {
                albums.push(album);
            }
        }
        albums
    }

    pub async fn artist_release_page(
        &self,
        artist_id: &str,
        page: u32,
        limit: u32,
    ) -> Vec<ProviderReleaseSummary> {
        wait_for_musicbrainz().await;
        let offset = page.saturating_sub(1).saturating_mul(limit);
        let response = self
            .http
            .get(format!("{}/release", self.base_url))
            .query(&[
                ("artist", artist_id.to_string()),
                ("limit", limit.to_string()),
                ("offset", offset.to_string()),
                ("fmt", "json".to_string()),
                ("inc", "artist-credits".to_string()),
            ])
            .send()
            .await;
        let Ok(response) = response.and_then(reqwest::Response::error_for_status) else {
            return Vec::new();
        };
        let Ok(body) = response.json::<serde_json::Value>().await else {
            return Vec::new();
        };
        body.get("releases")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|release| {
                let id = release.get("id")?.as_str()?.to_string();
                let title = release.get("title")?.as_str()?.to_string();
                let year = release
                    .get("date")
                    .and_then(serde_json::Value::as_str)
                    .filter(|date| date.len() >= 4)
                    .and_then(|date| date[..4].parse().ok());
                let artist_name = release
                    .get("artist-credit")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|credit| credit.first())
                    .and_then(|credit| credit.get("name"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                Some(ProviderReleaseSummary {
                    id,
                    title,
                    year,
                    kind: Some("release".to_string()),
                    artist_name,
                })
            })
            .collect()
    }
}

fn escape_musicbrainz_query(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn parse_musicbrainz_search_release(value: &serde_json::Value) -> Option<ProviderAlbum> {
    let id = value.get("id")?.as_str()?.to_string();
    let title = value.get("title")?.as_str()?.to_string();
    let credit = value
        .get("artist-credit")
        .and_then(serde_json::Value::as_array)
        .and_then(|credit| credit.first());
    let artist = credit
        .and_then(|credit| credit.get("name"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    Some(ProviderAlbum {
        id,
        title,
        artists: artist.iter().cloned().collect(),
        artist,
        artist_id: credit
            .and_then(|credit| credit.get("artist"))
            .and_then(|artist| artist.get("id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        year: value
            .get("date")
            .and_then(serde_json::Value::as_str)
            .filter(|date| date.len() >= 4)
            .map(|date| date[..4].to_string()),
        genre: None,
        tracks: Vec::new(),
    })
}

fn parse_musicbrainz_release(
    value: &serde_json::Value,
    fallback_id: &str,
) -> Option<ProviderAlbum> {
    let title = value.get("title")?.as_str()?.to_string();
    let release_credit = value
        .get("artist-credit")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice);
    let artist = release_credit
        .and_then(|credit| credit.first())
        .and_then(|credit| credit.get("name"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let artists = artist.iter().cloned().collect::<Vec<_>>();
    let artist_id = release_credit
        .and_then(|credit| credit.first())
        .and_then(|credit| credit.get("artist"))
        .and_then(|artist| artist.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let media = value
        .get("media")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut tracks = Vec::new();
    for medium in media {
        let disc_number = positive_integer(medium.get("position"));
        let medium_tracks = medium
            .get("tracks")
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for track in medium_tracks {
            let recording = track.get("recording");
            let recording_title = recording
                .and_then(|recording| recording.get("title"))
                .and_then(serde_json::Value::as_str);
            let title = track
                .get("title")
                .and_then(serde_json::Value::as_str)
                .or(recording_title)
                .map(str::to_string);
            let credit = non_empty_credit(track.get("artist-credit")).or_else(|| {
                non_empty_credit(recording.and_then(|value| value.get("artist-credit")))
            });
            let track_artist = credit
                .map(format_artist_credit)
                .filter(|name| !name.is_empty())
                .or_else(|| artist.clone());
            let track_artists = if credit.is_some() {
                artist_names(credit)
            } else {
                artists.clone()
            };
            tracks.push(ProviderTrack {
                match_titles: recording_title
                    .filter(|recording_title| Some(*recording_title) != title.as_deref())
                    .map(|title| vec![title.to_string()])
                    .unwrap_or_default(),
                title,
                artist: track_artist,
                artists: track_artists,
                track_number: positive_integer(track.get("number"))
                    .or_else(|| positive_integer(track.get("position"))),
                track_total: None,
                disc_number,
                recording_id: recording
                    .and_then(|recording| recording.get("id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                length: recording
                    .and_then(|recording| recording.get("length"))
                    .and_then(serde_json::Value::as_f64),
            });
        }
    }

    Some(ProviderAlbum {
        id: value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(fallback_id)
            .to_string(),
        title,
        artist,
        artists,
        artist_id,
        year: value
            .get("date")
            .and_then(serde_json::Value::as_str)
            .filter(|date| date.len() >= 4)
            .map(|date| date[..4].to_string()),
        genre: None,
        tracks,
    })
}

fn non_empty_credit(value: Option<&serde_json::Value>) -> Option<&[serde_json::Value]> {
    value
        .and_then(serde_json::Value::as_array)
        .filter(|credit| !credit.is_empty())
        .map(Vec::as_slice)
}

fn format_artist_credit(credit: &[serde_json::Value]) -> String {
    credit
        .iter()
        .map(|item| {
            format!(
                "{}{}",
                item.get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
                item.get("joinphrase")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
            )
        })
        .collect()
}

fn artist_names(credit: Option<&[serde_json::Value]>) -> Vec<String> {
    credit
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("name").and_then(serde_json::Value::as_str))
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn positive_integer(value: Option<&serde_json::Value>) -> Option<u32> {
    match value? {
        serde_json::Value::Number(number) => number
            .as_u64()
            .filter(|number| *number > 0)
            .and_then(|number| u32::try_from(number).ok()),
        serde_json::Value::String(number) => number
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|number| *number > 0),
        _ => None,
    }
}

fn parse_discogs_release(value: &serde_json::Value, fallback_id: &str) -> Option<ProviderAlbum> {
    let title = value.get("title")?.as_str()?.to_string();
    let artists = discogs_artists(value.get("artists"), None);
    let artist = artist_display_name(&artists, None);
    let raw_tracks = value
        .get("tracklist")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let track_total = u32::try_from(
        raw_tracks
            .iter()
            .filter(|track| {
                track
                    .get("position")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|position| !position.trim().is_empty())
            })
            .count(),
    )
    .ok();
    let mut tracks = Vec::new();
    for (index, track) in raw_tracks
        .iter()
        .filter(|track| {
            track
                .get("position")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|position| !position.trim().is_empty())
        })
        .enumerate()
    {
        let position = track
            .get("position")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let (disc_number, parsed_track_number) = parse_discogs_position(position);
        let track_artists = discogs_artists(track.get("artists"), artist.as_deref());
        tracks.push(ProviderTrack {
            title: track
                .get("title")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            match_titles: Vec::new(),
            artist: artist_display_name(&track_artists, artist.as_deref()),
            artists: if track_artists.is_empty() {
                artists.clone()
            } else {
                track_artists
            },
            track_number: parsed_track_number.or_else(|| u32::try_from(index + 1).ok()),
            track_total,
            disc_number,
            recording_id: None,
            length: track
                .get("duration")
                .and_then(serde_json::Value::as_str)
                .and_then(parse_discogs_duration),
        });
    }

    Some(ProviderAlbum {
        id: value
            .get("id")
            .and_then(|id| {
                id.as_u64()
                    .map(|id| id.to_string())
                    .or_else(|| id.as_str().map(str::to_string))
            })
            .unwrap_or_else(|| fallback_id.to_string()),
        title,
        artist,
        artists,
        artist_id: None,
        year: value.get("year").and_then(|year| {
            year.as_u64()
                .map(|year| year.to_string())
                .or_else(|| year.as_str().map(str::to_string))
        }),
        genre: merge_genre_style(value.get("genres"), value.get("styles")),
        tracks,
    })
}

fn discogs_artists(value: Option<&serde_json::Value>, fallback: Option<&str>) -> Vec<String> {
    let raw_names = value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|artist| artist.get("name").and_then(serde_json::Value::as_str))
        .map(clean_discogs_artist)
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    let names = if raw_names.is_empty() {
        fallback.into_iter().map(str::to_string).collect()
    } else {
        raw_names
    };
    split_artist_names(&names)
}

fn clean_discogs_artist(name: &str) -> String {
    Regex::new(r"\s+\(\d+\)$")
        .expect("valid Discogs artist suffix regex")
        .replace(name, "")
        .trim()
        .to_string()
}

fn split_artist_names(names: &[String]) -> Vec<String> {
    let separator = Regex::new(r"(?i)\s+(?:feat\.?|ft\.?|featuring)\s+|\s*[&/;,＋+、，；·‧]\s*")
        .expect("valid multi-artist regex");
    let mut output = Vec::new();
    for name in names {
        let name = replace_cjk_dots(name);
        for part in separator.split(&name) {
            let part = part.trim();
            if !part.is_empty()
                && !output
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(part))
            {
                output.push(part.to_string());
            }
        }
    }
    output
}

fn replace_cjk_dots(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    characters
        .iter()
        .enumerate()
        .map(|(index, character)| {
            if *character == '.'
                && index > 0
                && index + 1 < characters.len()
                && is_cjk(characters[index - 1])
                && is_cjk(characters[index + 1])
            {
                ';'
            } else {
                *character
            }
        })
        .collect()
}

fn artist_display_name(artists: &[String], fallback: Option<&str>) -> Option<String> {
    if artists.is_empty() {
        return fallback.map(str::to_string);
    }
    Some(artists.join(" & "))
}

fn parse_discogs_position(position: &str) -> (Option<u32>, Option<u32>) {
    let compact = position.trim();
    let cd = Regex::new(r"(?i)^CD\s*(\d+)[-. ]*(\d+)$").expect("valid Discogs CD position regex");
    if let Some(captures) = cd.captures(compact) {
        return (
            captures
                .get(1)
                .and_then(|value| value.as_str().parse().ok()),
            captures
                .get(2)
                .and_then(|value| value.as_str().parse().ok()),
        );
    }
    let trailing = Regex::new(r"(\d+)$").expect("valid Discogs position regex");
    (
        None,
        trailing
            .captures(compact)
            .and_then(|captures| captures.get(1))
            .and_then(|value| value.as_str().parse().ok()),
    )
}

fn parse_discogs_duration(duration: &str) -> Option<f64> {
    let parts = duration
        .split(':')
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    match parts.as_slice() {
        [minutes, seconds] => Some(minutes * 60.0 + seconds),
        [hours, minutes, seconds] => Some(hours * 3600.0 + minutes * 60.0 + seconds),
        _ => None,
    }
}

fn merge_genre_style(
    genres: Option<&serde_json::Value>,
    styles: Option<&serde_json::Value>,
) -> Option<String> {
    let mut values = Vec::new();
    for value in [genres, styles]
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !values.contains(&value) {
            values.push(value);
        }
    }
    (!values.is_empty()).then(|| values.join(", "))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteImage {
    pub source: &'static str,
    pub bytes: Vec<u8>,
    pub mime: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscogsAliasResolution {
    Direct,
    Alias(String),
    Unresolved,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<DiscogsSearchResult>,
}

#[derive(Debug, Deserialize)]
struct ArtistReleasesResponse {
    #[serde(default)]
    releases: Vec<ArtistRelease>,
    pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize)]
struct Pagination {
    pages: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ArtistRelease {
    id: Option<u64>,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscogsSearchResult {
    id: Option<u64>,
    title: Option<String>,
    cover_image: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ArtistDetail {
    #[serde(default)]
    images: Vec<DiscogsImage>,
}

#[derive(Debug, Deserialize)]
struct ReleaseDetail {
    #[serde(default)]
    images: Vec<DiscogsImage>,
}

#[derive(Debug, Deserialize)]
struct DiscogsImage {
    #[serde(rename = "type")]
    kind: Option<String>,
    uri: Option<String>,
}

pub struct DiscogsClient {
    base_url: String,
    token: Option<String>,
    http: Client,
    limiter: Arc<DiscogsRateLimiter>,
}

impl DiscogsClient {
    pub fn new(http: Client, token: Option<String>) -> Self {
        Self::at(http, token, DISCOGS_BASE)
    }

    pub fn at(http: Client, token: Option<String>, base_url: &str) -> Self {
        let limiter =
            DiscogsRateLimiter::shared(token.as_deref().is_some_and(|value| !value.is_empty()));
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            http,
            limiter,
        }
    }

    /// Exact Electron order: known release → known artist releases → validated search.
    pub async fn album_cover(
        &self,
        artist: &str,
        album: &str,
        release_id: Option<&str>,
        artist_id: Option<&str>,
    ) -> Option<RemoteImage> {
        if let Some(release_id) = release_id.filter(|id| !id.is_empty()) {
            if let Some(image) = self.release_cover(release_id).await {
                return Some(image);
            }
        }
        if let Some(artist_id) = artist_id.filter(|id| !id.is_empty()) {
            if let Some(image) = self.artist_release_cover(artist_id, album).await {
                return Some(image);
            }
        }
        self.search_cover(artist, album).await
    }

    pub async fn artist_image(&self, artist_id: &str) -> Option<RemoteImage> {
        let artist: ArtistDetail = self.get_json(&format!("artists/{artist_id}")).await?;
        let image_url = preferred_image_url(&artist.images)?;
        self.fetch_image("discogs", &image_url).await
    }

    pub async fn release_metadata(&self, release_id: &str) -> Option<ProviderAlbum> {
        let release: serde_json::Value = self.get_json(&format!("releases/{release_id}")).await?;
        parse_discogs_release(&release, release_id)
    }

    pub async fn search_album(
        &self,
        artist: &str,
        album: &str,
        max_candidates: usize,
    ) -> Vec<ProviderAlbum> {
        if (artist.is_empty() && album.is_empty()) || max_candidates == 0 {
            return Vec::new();
        }
        let releases = self
            .search_album_type(artist, album, max_candidates, "release")
            .await;
        if releases.is_empty() {
            self.search_album_type(artist, album, max_candidates, "master")
                .await
        } else {
            releases
        }
    }

    async fn search_album_type(
        &self,
        artist: &str,
        album: &str,
        max_candidates: usize,
        search_type: &str,
    ) -> Vec<ProviderAlbum> {
        let Some(body) = self
            .get_json_with_query::<serde_json::Value>(
                "database/search",
                &[
                    ("q", format!("{artist} {album}").trim().to_string()),
                    ("type", search_type.to_string()),
                    ("per_page", max_candidates.saturating_mul(3).to_string()),
                ],
            )
            .await
        else {
            return Vec::new();
        };
        let results = body
            .get("results")
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut albums = Vec::new();
        for result in results.iter().take(max_candidates) {
            let title = result
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let (result_artist, result_album) = title
                .split_once(" - ")
                .map(|(artist, album)| (artist.trim(), album.trim()))
                .unwrap_or((artist, title));
            if !artist.is_empty() && !artist_search_matches(result_artist, artist) {
                continue;
            }
            let release_id = result.get("id").and_then(serde_json::Value::as_u64);
            if let Some(release_id) = release_id {
                let path = format!("{search_type}s/{release_id}");
                if let Some(detail) = self
                    .get_json::<serde_json::Value>(&path)
                    .await
                    .and_then(|detail| parse_discogs_release(&detail, &release_id.to_string()))
                {
                    albums.push(detail);
                    continue;
                }
            }
            let artists = split_artist_names(&[result_artist.to_string()]);
            albums.push(ProviderAlbum {
                id: release_id.map(|id| id.to_string()).unwrap_or_default(),
                title: result_album.to_string(),
                artist: artist_display_name(&artists, Some(result_artist)),
                artists,
                artist_id: None,
                year: result.get("year").and_then(|year| {
                    year.as_u64()
                        .map(|year| year.to_string())
                        .or_else(|| year.as_str().map(str::to_string))
                }),
                genre: merge_genre_style(result.get("genre"), result.get("style")),
                tracks: Vec::new(),
            });
        }
        albums
    }

    pub async fn artist_release_page(
        &self,
        artist_id: &str,
        page: u32,
        per_page: u32,
    ) -> Vec<ProviderReleaseSummary> {
        let Some(body) = self
            .get_json::<serde_json::Value>(&format!(
                "artists/{artist_id}/releases?per_page={per_page}&page={page}&sort=year&sort_order=desc"
            ))
            .await
        else {
            return Vec::new();
        };
        body.get("releases")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|release| {
                let id = release
                    .get("main_release")
                    .and_then(serde_json::Value::as_u64)
                    .or_else(|| release.get("id").and_then(serde_json::Value::as_u64))?
                    .to_string();
                let title = release.get("title")?.as_str()?.to_string();
                let kind = release
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .filter(|kind| matches!(*kind, "master" | "release"))
                    .map(str::to_string);
                Some(ProviderReleaseSummary {
                    id,
                    title,
                    year: release
                        .get("year")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|year| u32::try_from(year).ok()),
                    kind,
                    artist_name: release
                        .get("artist")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                })
            })
            .collect()
    }

    pub async fn search_artist_exact(&self, name: &str) -> Option<String> {
        for key in ["artist", "q"] {
            let Some(response) = self
                .get_json_with_query::<SearchResponse>(
                    "database/search",
                    &[
                        ("type", "artist".to_string()),
                        (key, name.to_string()),
                        ("per_page", "5".to_string()),
                    ],
                )
                .await
            else {
                continue;
            };
            for result in response.results {
                let (Some(id), Some(title)) = (result.id, result.title.as_deref()) else {
                    continue;
                };
                if artist_exact_match(title, name) {
                    return Some(id.to_string());
                }
            }
        }
        None
    }

    pub async fn release_cover(&self, release_id: &str) -> Option<RemoteImage> {
        let release: ReleaseDetail = self.get_json(&format!("releases/{release_id}")).await?;
        let image_url = preferred_image_url(&release.images)?;
        self.fetch_image("discogs", &image_url).await
    }

    async fn artist_release_cover(&self, artist_id: &str, album: &str) -> Option<RemoteImage> {
        let mut best = None;
        for page in 1..=3 {
            let response: ArtistReleasesResponse = self
                .get_json(&format!(
                    "artists/{artist_id}/releases?per_page=50&page={page}"
                ))
                .await?;
            for release in &response.releases {
                if release
                    .title
                    .as_deref()
                    .is_some_and(|title| album_title_score(album, title) >= 75)
                {
                    best = release.id;
                    break;
                }
            }
            if best.is_some()
                || response
                    .pagination
                    .and_then(|p| p.pages)
                    .is_some_and(|pages| page >= pages)
            {
                break;
            }
        }
        self.release_cover(&best?.to_string()).await
    }

    async fn search_cover(&self, artist: &str, album: &str) -> Option<RemoteImage> {
        if artist.is_empty() || album.is_empty() {
            return None;
        }
        let response: SearchResponse = self
            .get_json_with_query(
                "database/search",
                &[
                    ("q", format!("{artist} {album}")),
                    ("type", "release".to_string()),
                    ("per_page", "10".to_string()),
                ],
            )
            .await?;
        for candidate in response.results {
            let Some(title) = candidate.title.as_deref() else {
                continue;
            };
            if !discogs_candidate_matches(title, artist, album) {
                continue;
            }
            if let Some(url) = candidate.cover_image.filter(|url| !url.is_empty()) {
                if let Some(image) = self.fetch_image("discogs", &url).await {
                    return Some(image);
                }
            } else if let Some(id) = candidate.id {
                if let Some(image) = self.release_cover(&id.to_string()).await {
                    return Some(image);
                }
            }
        }
        None
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Option<T> {
        self.limiter.wait().await;
        let url = format!("{}/{}", self.base_url, path);
        let request = self.authorized(self.http.get(url));
        let response = request.send().await.ok()?.error_for_status().ok()?;
        response.json().await.ok()
    }

    async fn get_json_with_query<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Option<T> {
        self.limiter.wait().await;
        let url = format!("{}/{}", self.base_url, path);
        let request = self.authorized(self.http.get(url)).query(query);
        let response = request.send().await.ok()?.error_for_status().ok()?;
        response.json().await.ok()
    }

    fn authorized(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.token.as_deref().filter(|token| !token.is_empty()) {
            Some(token) => request.header("Authorization", format!("Discogs token={token}")),
            None => request,
        }
    }

    pub async fn fetch_image(&self, source: &'static str, url: &str) -> Option<RemoteImage> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("image/jpeg")
            .to_string();
        let bytes = response.bytes().await.ok()?.to_vec();
        (!bytes.is_empty()).then_some(RemoteImage {
            source,
            bytes,
            mime,
            url: url.to_string(),
        })
    }
}

fn artist_search_matches(candidate: &str, hint: &str) -> bool {
    let candidate = candidate.to_lowercase();
    let hint = hint.to_lowercase();
    candidate.contains(hint.trim()) || hint.contains(candidate.trim())
}

fn preferred_image_url(images: &[DiscogsImage]) -> Option<String> {
    images
        .iter()
        .find(|image| image.kind.as_deref() == Some("primary"))
        .or_else(|| images.first())?
        .uri
        .clone()
        .filter(|url| !url.is_empty())
}

pub fn discogs_candidate_matches(title: &str, artist: &str, album: &str) -> bool {
    let Some((candidate_artist, candidate_album)) = title.split_once(" - ") else {
        return false;
    };
    artist_matches(candidate_artist, artist) && album_matches(candidate_album, album, Some(artist))
}

fn artist_exact_match(candidate: &str, query: &str) -> bool {
    let candidate_variants = normalized_variants(candidate);
    let query_variants = normalized_variants(query);
    candidate_variants
        .iter()
        .any(|value| query_variants.contains(value))
        || normalize(candidate).replace(' ', "") == normalize(query).replace(' ', "")
}

fn artist_matches(candidate: &str, query: &str) -> bool {
    if candidate
        .split_whitespace()
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("various"))
    {
        return false;
    }
    let candidate = candidate.trim_end_matches('*').trim();
    candidate.split(" = ").any(|part| {
        let candidate_variants = normalized_variants(part);
        let query_variants = normalized_variants(query);
        candidate_variants.iter().any(|candidate| {
            query_variants.iter().any(|query| {
                candidate == query
                    || (candidate.len() >= 3
                        && query.len() >= 3
                        && (candidate.contains(query) || query.contains(candidate)))
            })
        })
    })
}

fn album_matches(candidate: &str, query: &str, query_artist: Option<&str>) -> bool {
    let candidate_variants = normalized_variants(candidate);
    if candidate_variants.contains(&normalize(query)) {
        return true;
    }
    let candidate_normalized = normalize(candidate);
    matches!(candidate_normalized.as_str(), "同名专辑" | "同名")
        && query_artist.is_some_and(|artist| normalize(query) == normalize(artist))
}

pub fn album_names_match(hint: &str, candidate: &str) -> bool {
    let hint_forms = lookup_variants(hint);
    let candidate_forms = lookup_variants(candidate);
    hint_forms.iter().any(|hint| {
        candidate_forms.iter().any(|candidate| {
            hint == candidate
                || hint.contains(candidate)
                || candidate.contains(hint)
                || ((contains_cjk(hint) || contains_cjk(candidate))
                    && fuzzy_similarity(hint, candidate) >= 80)
        })
    })
}

fn album_title_score(local: &str, remote: &str) -> u32 {
    let local_forms = lookup_variants(local);
    let remote_forms = lookup_variants(remote);
    let mut score = 0;
    for local in &local_forms {
        for remote in &remote_forms {
            if local == remote {
                score = score.max(100);
            } else if remote.contains(local) && containment_is_specific(local) {
                score = score.max(85);
            } else if local.contains(remote) && containment_is_specific(remote) {
                score = score.max(70);
            } else if contains_cjk(local) || contains_cjk(remote) {
                let similarity = fuzzy_similarity(local, remote);
                if similarity >= 80 {
                    score = score.max((similarity * 85 + 50) / 100);
                }
            }
        }
    }
    score
}

fn containment_is_specific(value: &str) -> bool {
    if contains_cjk(value) {
        value
            .chars()
            .filter(|character| !character.is_whitespace())
            .count()
            >= 4
    } else {
        value
            .split_whitespace()
            .any(|token| token.chars().count() >= 3)
    }
}

fn contains_cjk(value: &str) -> bool {
    value
        .chars()
        .any(|character| matches!(character, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}'))
}

fn fuzzy_similarity(left: &str, right: &str) -> u32 {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    if left.is_empty() && right.is_empty() {
        return 100;
    }
    let mut previous = vec![0_u32; right.len() + 1];
    let mut current = vec![0_u32; right.len() + 1];
    for left_character in left.iter().copied() {
        for (index, right_character) in right.iter().copied().enumerate() {
            current[index + 1] = if left_character == right_character {
                previous[index] + 1
            } else {
                previous[index + 1].max(current[index])
            };
        }
        std::mem::swap(&mut previous, &mut current);
        current.fill(0);
    }
    (200 * previous[right.len()] + (left.len() + right.len()) as u32 / 2)
        / (left.len() + right.len()) as u32
}

fn lookup_variants(input: &str) -> BTreeSet<String> {
    let converter = OPENCC.get_or_init(OpenCC::new);
    [
        input.to_string(),
        converter.convert(input, "s2t", false),
        converter.convert(input, "t2s", false),
    ]
    .into_iter()
    .map(|value| normalize_lookup(&value))
    .collect()
}

fn normalize_lookup(input: &str) -> String {
    let decomposed = input
        .replace('斉', "齊")
        .replace('妳', "你")
        .nfkd()
        .filter(|character| !is_combining_mark(*character))
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    let tokens = decomposed
        .split_whitespace()
        .map(|token| match token {
            "viii" => "8",
            "vii" => "7",
            "vi" => "6",
            "iv" => "4",
            "iii" => "3",
            "ii" => "2",
            "ix" => "9",
            "v" => "5",
            "x" => "10",
            "i" => "1",
            other => other,
        })
        .collect::<Vec<_>>();
    let joined = tokens.join(" ");
    let mut output = String::with_capacity(joined.len());
    let mut characters = joined.chars().peekable();
    while let Some(character) = characters.next() {
        if character == ' '
            && output.chars().last().is_some_and(is_cjk)
            && characters.peek().copied().is_some_and(is_cjk)
        {
            continue;
        }
        output.push(character);
    }
    output
}

fn is_cjk(character: char) -> bool {
    matches!(character, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}')
}

fn normalized_variants(input: &str) -> BTreeSet<String> {
    let converter = OPENCC.get_or_init(OpenCC::new);
    [
        input.to_string(),
        converter.convert(input, "s2t", false),
        converter.convert(input, "t2s", false),
    ]
    .into_iter()
    .map(|value| normalize(&value))
    .collect()
}

fn normalize(input: &str) -> String {
    input
        .nfkc()
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

#[derive(Debug, Clone)]
pub struct ArtworkEndpoints {
    pub cover_art_archive: String,
    pub the_audio_db: String,
    pub wikidata_api: String,
    pub wikidata_entity: String,
    pub commons_file: String,
    pub musicbrainz: String,
}

impl Default for ArtworkEndpoints {
    fn default() -> Self {
        Self {
            cover_art_archive: "https://coverartarchive.org".to_string(),
            the_audio_db: "https://theaudiodb.com/api/v1/json".to_string(),
            wikidata_api: "https://www.wikidata.org/w/api.php".to_string(),
            wikidata_entity: "https://www.wikidata.org/wiki/Special:EntityData".to_string(),
            commons_file: "https://commons.wikimedia.org/wiki/Special:FilePath".to_string(),
            musicbrainz: "https://musicbrainz.org/ws/2".to_string(),
        }
    }
}

pub struct RemoteArtworkClient {
    http: Client,
    discogs: DiscogsClient,
    endpoints: ArtworkEndpoints,
    the_audio_db_key: Option<String>,
}

impl RemoteArtworkClient {
    pub fn new(
        http: Client,
        discogs_token: Option<String>,
        the_audio_db_key: Option<String>,
    ) -> Self {
        Self::at(
            http,
            discogs_token,
            the_audio_db_key,
            DISCOGS_BASE,
            ArtworkEndpoints::default(),
        )
    }

    pub fn at(
        http: Client,
        discogs_token: Option<String>,
        the_audio_db_key: Option<String>,
        discogs_base: &str,
        endpoints: ArtworkEndpoints,
    ) -> Self {
        Self {
            discogs: DiscogsClient::at(http.clone(), discogs_token, discogs_base),
            http,
            endpoints,
            the_audio_db_key,
        }
    }

    /// Remote album order after the local provider: CAA → Discogs → TheAudioDB.
    pub async fn album_cover(
        &self,
        artist: Option<&str>,
        album: Option<&str>,
        musicbrainz_album_id: Option<&str>,
        discogs_artist_id: Option<&str>,
        discogs_release_id: Option<&str>,
    ) -> Option<RemoteImage> {
        if let Some(mbid) = musicbrainz_album_id.filter(|value| !value.is_empty()) {
            if let Some(image) = self.cover_art_archive(mbid).await.filter(valid_image) {
                return Some(image);
            }
        }
        if let Some(image) = self
            .discogs
            .album_cover(
                artist.unwrap_or_default(),
                album.unwrap_or_default(),
                discogs_release_id,
                discogs_artist_id,
            )
            .await
            .filter(valid_image)
        {
            return Some(image);
        }
        if let (Some(artist), Some(album)) = (artist, album) {
            if let Some(image) = self.the_audio_db(artist, album).await.filter(valid_image) {
                return Some(image);
            }
        }
        None
    }

    /// Remote artist order after local: Discogs direct/identity → Wikimedia.
    pub async fn artist_image(
        &self,
        artist: &str,
        discogs_artist_id: Option<&str>,
    ) -> Option<RemoteImage> {
        if let Some(id) = discogs_artist_id.filter(|value| !value.is_empty()) {
            if let Some(image) = self.discogs.artist_image(id).await.filter(valid_image) {
                return Some(image);
            }
        }
        let mut resolved_id = self.discogs.search_artist_exact(artist).await;
        if resolved_id.is_none() {
            for alias in self.musicbrainz_aliases(artist).await {
                if let Some(id) = self.discogs.search_artist_exact(&alias).await {
                    resolved_id = Some(id);
                    break;
                }
            }
        }
        if let Some(id) = resolved_id.filter(|id| Some(id.as_str()) != discogs_artist_id) {
            if let Some(image) = self.discogs.artist_image(&id).await.filter(valid_image) {
                return Some(image);
            }
        }
        self.wikimedia(artist).await.filter(valid_image)
    }

    /// Resolve a Latin lookup alias only when the original name does not
    /// already resolve and Discogs independently validates the alias.
    pub async fn validated_discogs_alias(&self, artist: &str) -> DiscogsAliasResolution {
        if self.discogs.search_artist_exact(artist).await.is_some() {
            return DiscogsAliasResolution::Direct;
        }
        for alias in self.musicbrainz_aliases(artist).await {
            if self.discogs.search_artist_exact(&alias).await.is_some() {
                return DiscogsAliasResolution::Alias(alias);
            }
        }
        DiscogsAliasResolution::Unresolved
    }

    pub async fn validate_discogs_artist_name(&self, name: &str) -> bool {
        self.discogs.search_artist_exact(name).await.is_some()
    }

    async fn cover_art_archive(&self, mbid: &str) -> Option<RemoteImage> {
        #[derive(Deserialize)]
        struct ArchiveResponse {
            #[serde(default)]
            images: Vec<ArchiveImage>,
        }
        #[derive(Deserialize)]
        struct ArchiveImage {
            image: Option<String>,
            #[serde(default)]
            types: Vec<String>,
        }
        let url = format!(
            "{}/release/{mbid}",
            self.endpoints.cover_art_archive.trim_end_matches('/')
        );
        let response = self.send_with_one_transport_retry(&url).await?;
        let archive: ArchiveResponse = response.json().await.ok()?;
        let image_url = archive
            .images
            .iter()
            .find(|image| image.types.iter().any(|kind| kind == "Front"))
            .or_else(|| archive.images.first())?
            .image
            .as_deref()?;
        self.fetch_image_with_retry("cover-art-archive", image_url)
            .await
    }

    async fn the_audio_db(&self, artist: &str, album: &str) -> Option<RemoteImage> {
        #[derive(Deserialize)]
        struct AudioDbResponse {
            album: Option<Vec<AudioDbAlbum>>,
        }
        #[derive(Deserialize)]
        struct AudioDbAlbum {
            #[serde(rename = "strAlbumThumb")]
            thumbnail: Option<String>,
        }
        let key = self
            .the_audio_db_key
            .as_deref()
            .filter(|value| !value.is_empty())?;
        let url = format!(
            "{}/{key}/searchalbum.php",
            self.endpoints.the_audio_db.trim_end_matches('/')
        );
        let response = self
            .http
            .get(url)
            .query(&[("s", artist), ("a", album)])
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        let body: AudioDbResponse = response.json().await.ok()?;
        let albums = body.album?;
        let image_url = albums.first()?.thumbnail.as_deref()?;
        self.fetch_image("theaudiodb", image_url).await
    }

    async fn musicbrainz_aliases(&self, artist: &str) -> Vec<String> {
        wait_for_musicbrainz().await;
        let query = format!("artist:\"{}\"", artist.replace('"', "\\\""));
        let response = self
            .http
            .get(format!(
                "{}/artist/",
                self.endpoints.musicbrainz.trim_end_matches('/')
            ))
            .query(&[
                ("query", query),
                ("fmt", "json".to_string()),
                ("limit", "5".to_string()),
            ])
            .send()
            .await;
        let Ok(response) = response.and_then(reqwest::Response::error_for_status) else {
            return Vec::new();
        };
        let Ok(body) = response.json::<serde_json::Value>().await else {
            return Vec::new();
        };
        body.get("artists")
            .and_then(serde_json::Value::as_array)
            .and_then(|artists| {
                artists
                    .iter()
                    .find(|candidate| {
                        candidate.get("name").and_then(serde_json::Value::as_str) == Some(artist)
                            || candidate
                                .get("sort-name")
                                .and_then(serde_json::Value::as_str)
                                == Some(artist)
                    })
                    .or_else(|| artists.first())
            })
            .and_then(|candidate| candidate.get("aliases"))
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|alias| alias.get("name").and_then(serde_json::Value::as_str))
            .filter(|alias| alias.is_ascii())
            .map(str::to_string)
            .collect()
    }

    async fn wikimedia(&self, artist: &str) -> Option<RemoteImage> {
        let mut search_url = reqwest::Url::parse(&self.endpoints.wikidata_api).ok()?;
        search_url
            .query_pairs_mut()
            .append_pair("action", "wbsearchentities")
            .append_pair("search", artist)
            .append_pair("language", "en")
            .append_pair("limit", "5")
            .append_pair("format", "json");
        let search = self
            .send_with_one_transport_retry(search_url.as_str())
            .await?
            .json::<serde_json::Value>()
            .await
            .ok()?;
        let entity_id = search
            .get("search")?
            .as_array()?
            .first()?
            .get("id")?
            .as_str()?;
        let entity_url = format!(
            "{}/{entity_id}.json",
            self.endpoints.wikidata_entity.trim_end_matches('/')
        );
        let entity = self
            .send_with_one_transport_retry(&entity_url)
            .await?
            .json::<serde_json::Value>()
            .await
            .ok()?;
        let filename = entity
            .get("entities")?
            .get(entity_id)?
            .get("claims")?
            .get("P18")?
            .as_array()?
            .first()?
            .get("mainsnak")?
            .get("datavalue")?
            .get("value")?
            .as_str()?;
        let image_url = format!(
            "{}/{}",
            self.endpoints.commons_file.trim_end_matches('/'),
            filename.replace(' ', "_")
        );
        self.fetch_image_with_retry("wikimedia", &image_url).await
    }

    async fn send_with_one_transport_retry(&self, url: &str) -> Option<reqwest::Response> {
        match self.http.get(url).send().await {
            Ok(response) => response.error_for_status().ok(),
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(250)).await;
                self.http
                    .get(url)
                    .send()
                    .await
                    .ok()?
                    .error_for_status()
                    .ok()
            }
        }
    }

    async fn fetch_image(&self, source: &'static str, url: &str) -> Option<RemoteImage> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("image/jpeg")
            .to_string();
        let bytes = response.bytes().await.ok()?.to_vec();
        (!bytes.is_empty()).then_some(RemoteImage {
            source,
            bytes,
            mime,
            url: url.to_string(),
        })
    }

    async fn fetch_image_with_retry(&self, source: &'static str, url: &str) -> Option<RemoteImage> {
        let response = self.send_with_one_transport_retry(url).await?;
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("image/jpeg")
            .to_string();
        let bytes = response.bytes().await.ok()?.to_vec();
        (!bytes.is_empty()).then_some(RemoteImage {
            source,
            bytes,
            mime,
            url: url.to_string(),
        })
    }
}

async fn wait_for_musicbrainz() {
    let limiter = MUSICBRAINZ_LAST_REQUEST.get_or_init(|| tokio::sync::Mutex::new(None));
    let delay = {
        let mut last_request = limiter.lock().await;
        let now = Instant::now();
        let scheduled = last_request
            .map(|last| last + Duration::from_secs(1))
            .filter(|next| *next > now)
            .unwrap_or(now);
        *last_request = Some(scheduled);
        scheduled.saturating_duration_since(now)
    };
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
}

fn valid_image(image: &RemoteImage) -> bool {
    image::load_from_memory(&image.bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    type Route = fn(&str, &str) -> (&'static str, String, &'static str);

    fn retry_server() -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server_base = base.clone();
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let (mut dropped, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let count = dropped.read(&mut request).unwrap();
            let _ = send.send(String::from_utf8_lossy(&request[..count]).into_owned());
            drop(dropped);

            let (mut metadata, _) = listener.accept().unwrap();
            let count = metadata.read(&mut request).unwrap();
            let _ = send.send(String::from_utf8_lossy(&request[..count]).into_owned());
            let body =
                format!(r#"{{"images":[{{"image":"{server_base}/image","types":["Front"]}}]}}"#);
            write!(metadata, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();

            let (mut image_stream, _) = listener.accept().unwrap();
            let count = image_stream.read(&mut request).unwrap();
            let _ = send.send(String::from_utf8_lossy(&request[..count]).into_owned());
            let image = image::DynamicImage::new_rgb8(1, 1);
            let mut body = Vec::new();
            image::codecs::jpeg::JpegEncoder::new(&mut body)
                .encode_image(&image)
                .unwrap();
            write!(image_stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: image/jpeg\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            image_stream.write_all(&body).unwrap();
        });
        (base, receive)
    }

    fn server(count: usize, route: Route) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server_base = base.clone();
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            for _ in 0..count {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 4096];
                let count = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..count]).into_owned();
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let (status, body, content_type) = route(path, &server_base);
                let body = if body == "VALID_IMAGE" {
                    let image = image::DynamicImage::new_rgb8(1, 1);
                    let mut bytes = Vec::new();
                    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
                        .encode_image(&image)
                        .unwrap();
                    bytes
                } else {
                    body.into_bytes()
                };
                let _ = send.send(request);
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        (base, receive)
    }

    fn generic_route(path: &str, base: &str) -> (&'static str, String, &'static str) {
        if path.starts_with("/database/search?") {
            return (
                "200 OK",
                format!(
                    r#"{{"results":[{{"title":"Wrong - Album","cover_image":"{base}/bad"}},{{"title":"F.I.R. = 飛兒樂團 - 無限","cover_image":"{base}/image"}}]}}"#
                ),
                "application/json",
            );
        }
        ("200 OK", "VALID_IMAGE".to_string(), "image/jpeg")
    }

    fn direct_route(path: &str, base: &str) -> (&'static str, String, &'static str) {
        if path == "/releases/42" {
            return (
                "200 OK",
                format!(r#"{{"images":[{{"type":"primary","uri":"{base}/image"}}]}}"#),
                "application/json",
            );
        }
        ("200 OK", "VALID_IMAGE".to_string(), "image/jpeg")
    }

    fn fallback_route(path: &str, base: &str) -> (&'static str, String, &'static str) {
        if path == "/releases/404" {
            return ("404 Not Found", "{}".to_string(), "application/json");
        }
        if path.starts_with("/artists/7/releases?") {
            return ("200 OK", "not-json".to_string(), "application/json");
        }
        if path.starts_with("/database/search?") {
            return (
                "200 OK",
                format!(
                    r#"{{"results":[{{"title":"Artist - Album","cover_image":"{base}/fallback"}}]}}"#
                ),
                "application/json",
            );
        }
        ("200 OK", "VALID_IMAGE".to_string(), "image/jpeg")
    }

    fn album_provider_route(path: &str, base: &str) -> (&'static str, String, &'static str) {
        if path == "/caa/release/mbid" {
            return (
                "200 OK",
                format!(r#"{{"images":[{{"image":"{base}/invalid","types":["Front"]}}]}}"#),
                "application/json",
            );
        }
        if path == "/invalid" {
            return ("200 OK", "not-an-image".to_string(), "image/jpeg");
        }
        if path.starts_with("/discogs/database/search?") {
            return (
                "200 OK",
                r#"{"results":[]}"#.to_string(),
                "application/json",
            );
        }
        if path.starts_with("/audio/key/searchalbum.php?") {
            return (
                "200 OK",
                format!(r#"{{"album":[{{"strAlbumThumb":"{base}/audio-image"}}]}}"#),
                "application/json",
            );
        }
        ("200 OK", "VALID_IMAGE".to_string(), "image/jpeg")
    }

    fn artist_provider_route(path: &str, base: &str) -> (&'static str, String, &'static str) {
        if path == "/discogs/artists/9" {
            return ("200 OK", r#"{"images":[]}"#.to_string(), "application/json");
        }
        if path.starts_with("/discogs/database/search?") && path.contains("artist=Alias") {
            return (
                "200 OK",
                r#"{"results":[{"id":7,"title":"Alias"}]}"#.to_string(),
                "application/json",
            );
        }
        if path.starts_with("/discogs/database/search?") {
            return (
                "200 OK",
                r#"{"results":[]}"#.to_string(),
                "application/json",
            );
        }
        if path.starts_with("/mb/artist/?") {
            return (
                "200 OK",
                r#"{"artists":[{"id":"mb","name":"原名","aliases":[{"name":"Alias"}]}]}"#
                    .to_string(),
                "application/json",
            );
        }
        if path == "/discogs/artists/7" {
            return (
                "200 OK",
                format!(r#"{{"images":[{{"type":"primary","uri":"{base}/artist-image"}}]}}"#),
                "application/json",
            );
        }
        ("200 OK", "VALID_IMAGE".to_string(), "image/jpeg")
    }

    fn wikimedia_route(path: &str, _base: &str) -> (&'static str, String, &'static str) {
        if path.starts_with("/wiki/search?") {
            return (
                "200 OK",
                r#"{"search":[{"id":"Q1"}]}"#.to_string(),
                "application/json",
            );
        }
        if path == "/wiki/entity/Q1.json" {
            return (
                "200 OK",
                r#"{"entities":{"Q1":{"claims":{"P18":[{"mainsnak":{"datavalue":{"value":"Artist Photo.jpg"}}}]}}}}"#.to_string(),
                "application/json",
            );
        }
        ("200 OK", "VALID_IMAGE".to_string(), "image/jpeg")
    }

    fn endpoints(base: &str) -> ArtworkEndpoints {
        ArtworkEndpoints {
            cover_art_archive: format!("{base}/caa"),
            the_audio_db: format!("{base}/audio"),
            wikidata_api: format!("{base}/wiki/search"),
            wikidata_entity: format!("{base}/wiki/entity"),
            commons_file: format!("{base}/wiki/file"),
            musicbrainz: format!("{base}/mb"),
        }
    }

    fn musicbrainz_release_route(path: &str, _base: &str) -> (&'static str, String, &'static str) {
        assert!(path.starts_with("/release/release-id?"));
        (
            "200 OK",
            r#"{
              "id":"release-id","title":"Canonical Album","date":"2004-08-01",
              "artist-credit":[{"name":"Album Artist","artist":{"id":"artist-id"}}],
              "media":[
                {"position":1,"track-count":1,"tracks":[{
                  "position":1,
                  "recording":{"id":"track-1","title":"Solo","length":181000,"artist-credit":[{"name":"Album Artist"}]}
                }]},
                {"position":2,"track-count":1,"tracks":[{
                  "position":1,
                  "recording":{"id":"track-2","title":"Featured","length":202000,"artist-credit":[
                    {"name":"Album Artist","joinphrase":" feat. "},{"name":"Guest"}
                  ]}
                }]}
              ]
            }"#
            .to_string(),
            "application/json",
        )
    }

    fn discogs_metadata_route(path: &str, _base: &str) -> (&'static str, String, &'static str) {
        assert_eq!(path, "/releases/42");
        (
            "200 OK",
            r#"{
              "id":42,"title":"Canonical Album","year":2004,
              "artists":[{"name":"Album Artist (2)"}],
              "genres":["Rock","Pop"],"styles":["Pop","Indie Rock"],
              "tracklist":[
                {"position":"CD1-01","title":"Solo","duration":"3:01"},
                {"position":"CD2-03","title":"Featured","duration":"3:22",
                 "artists":[{"name":"Album Artist (2)"},{"name":"Guest"}]}
              ]
            }"#
            .to_string(),
            "application/json",
        )
    }

    fn musicbrainz_search_route(path: &str, _base: &str) -> (&'static str, String, &'static str) {
        if path.starts_with("/release?query=") {
            return (
                "200 OK",
                r#"{"releases":[{"id":"wrong"},{"id":"best"}]}"#.to_string(),
                "application/json",
            );
        }
        let (id, title) = if path.starts_with("/release/wrong?") {
            ("wrong", "Wrong Album")
        } else {
            assert!(path.starts_with("/release/best?"));
            ("best", "Right Album")
        };
        (
            "200 OK",
            format!(
                r#"{{"id":"{id}","title":"{title}","artist-credit":[{{"name":"Artist","artist":{{"id":"artist-id"}}}}],"media":[]}}"#
            ),
            "application/json",
        )
    }

    fn musicbrainz_search_detail_failure_route(
        path: &str,
        _base: &str,
    ) -> (&'static str, String, &'static str) {
        if path.starts_with("/release?query=") {
            return (
                "200 OK",
                r#"{"releases":[{"id":"fallback","title":"Search Album","date":"2003-01-01","artist-credit":[{"name":"Search Artist","artist":{"id":"artist-id"}}]}]}"#.to_string(),
                "application/json",
            );
        }
        ("503 Unavailable", "{}".to_string(), "application/json")
    }

    fn musicbrainz_artist_page_route(
        path: &str,
        _base: &str,
    ) -> (&'static str, String, &'static str) {
        assert!(path.starts_with(
            "/release?artist=artist-id&limit=100&offset=100&fmt=json&inc=artist-credits"
        ));
        (
            "200 OK",
            r#"{"releases":[
              {"id":"one","title":"Album One","date":"2001-02-03","artist-credit":[{"name":"Artist"}]},
              {"id":"two","title":"Album Two","artist-credit":[]}
            ]}"#
            .to_string(),
            "application/json",
        )
    }

    fn discogs_artist_page_route(path: &str, _base: &str) -> (&'static str, String, &'static str) {
        assert_eq!(
            path,
            "/artists/7/releases?per_page=100&page=2&sort=year&sort_order=desc"
        );
        (
            "200 OK",
            r#"{"releases":[
              {"id":11,"main_release":42,"title":"Master Album","artist":"Artist","year":2004,"type":"master"},
              {"id":12,"title":"Release Album","type":"release"},
              {"id":13,"title":"Invalid Kind","type":"label"}
            ]}"#
            .to_string(),
            "application/json",
        )
    }

    fn discogs_search_metadata_route(
        path: &str,
        _base: &str,
    ) -> (&'static str, String, &'static str) {
        if path.starts_with("/database/search?") {
            return (
                "200 OK",
                r#"{"results":[
                  {"id":1,"title":"Other - Album"},
                  {"id":42,"title":"Artist - Album","year":2004}
                ]}"#
                .to_string(),
                "application/json",
            );
        }
        assert_eq!(path, "/releases/42");
        (
            "200 OK",
            r#"{"id":42,"title":"Album","artists":[{"name":"Artist"}],"tracklist":[]}"#.to_string(),
            "application/json",
        )
    }

    #[tokio::test]
    async fn discogs_name_search_rejects_artist_mismatch_and_loads_release_detail() {
        let (base, requests) = server(2, discogs_search_metadata_route);
        let client = DiscogsClient::at(ProviderState::new().http(), None, &base);

        let albums = client.search_album("Artist", "Album", 3).await;

        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].id, "42");
        assert_eq!(albums[0].artist.as_deref(), Some("Artist"));
        let search = requests.recv().unwrap();
        assert!(search.contains("q=Artist+Album"));
        assert!(search.contains("type=release"));
        assert!(requests.recv().unwrap().contains("GET /releases/42 "));
    }

    #[tokio::test]
    async fn discogs_artist_release_page_prefers_main_release_and_sanitizes_type() {
        let (base, _requests) = server(1, discogs_artist_page_route);
        let client = DiscogsClient::at(ProviderState::new().http(), None, &base);

        let releases = client.artist_release_page("7", 2, 100).await;

        assert_eq!(releases.len(), 3);
        assert_eq!(releases[0].id, "42");
        assert_eq!(releases[0].year, Some(2004));
        assert_eq!(releases[0].kind.as_deref(), Some("master"));
        assert_eq!(releases[0].artist_name.as_deref(), Some("Artist"));
        assert_eq!(releases[1].id, "12");
        assert_eq!(releases[2].kind, None);
    }

    #[tokio::test]
    async fn musicbrainz_artist_release_page_matches_electron_cache_shape() {
        let (base, _requests) = server(1, musicbrainz_artist_page_route);
        let client = MusicBrainzClient::at(ProviderState::new().http(), &base);

        let releases = client.artist_release_page("artist-id", 2, 100).await;

        assert_eq!(releases.len(), 2);
        assert_eq!(releases[0].id, "one");
        assert_eq!(releases[0].year, Some(2001));
        assert_eq!(releases[0].kind.as_deref(), Some("release"));
        assert_eq!(releases[0].artist_name.as_deref(), Some("Artist"));
        assert_eq!(releases[1].year, None);
    }

    #[tokio::test]
    async fn musicbrainz_name_search_loads_release_details_in_api_order() {
        let (base, requests) = server(3, musicbrainz_search_route);
        let client = MusicBrainzClient::at(ProviderState::new().http(), &base);

        let albums = client.search_album("Artist", "Right Album", 2).await;

        assert_eq!(albums.len(), 2);
        assert_eq!(albums[0].id, "wrong");
        assert_eq!(albums[1].id, "best");
        let search = requests.recv().unwrap();
        assert!(search
            .contains("GET /release?query=artist%3A%22Artist%22+AND+release%3A%22Right+Album%22"));
        assert!(requests.recv().unwrap().contains("GET /release/wrong?"));
        assert!(requests.recv().unwrap().contains("GET /release/best?"));
    }

    #[tokio::test]
    async fn musicbrainz_name_search_keeps_search_metadata_when_track_detail_fails() {
        let (base, _requests) = server(2, musicbrainz_search_detail_failure_route);
        let client = MusicBrainzClient::at(ProviderState::new().http(), &base);

        let albums = client
            .search_album("Search Artist", "Search Album", 1)
            .await;

        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].id, "fallback");
        assert_eq!(albums[0].title, "Search Album");
        assert_eq!(albums[0].artist.as_deref(), Some("Search Artist"));
        assert_eq!(albums[0].artist_id.as_deref(), Some("artist-id"));
        assert_eq!(albums[0].year.as_deref(), Some("2003"));
        assert!(albums[0].tracks.is_empty());
    }

    #[tokio::test]
    async fn discogs_direct_release_parses_genres_positions_and_clean_artists() {
        let (base, requests) = server(1, discogs_metadata_route);
        let client = DiscogsClient::at(ProviderState::new().http(), Some("secret".into()), &base);

        let album = client.release_metadata("42").await.unwrap();

        assert_eq!(album.title, "Canonical Album");
        assert_eq!(album.artist.as_deref(), Some("Album Artist"));
        assert_eq!(album.year.as_deref(), Some("2004"));
        assert_eq!(album.genre.as_deref(), Some("Rock, Pop, Indie Rock"));
        assert_eq!(album.tracks[0].track_number, Some(1));
        assert_eq!(album.tracks[0].disc_number, Some(1));
        assert_eq!(album.tracks[1].track_number, Some(3));
        assert_eq!(album.tracks[1].disc_number, Some(2));
        assert_eq!(album.tracks[1].length, Some(202.0));
        assert_eq!(
            album.tracks[1].artist.as_deref(),
            Some("Album Artist & Guest")
        );
        assert!(requests
            .recv()
            .unwrap()
            .to_ascii_lowercase()
            .contains("authorization: discogs token=secret"));
    }

    #[tokio::test]
    async fn musicbrainz_direct_release_preserves_discs_and_recording_artist_credit() {
        let (base, requests) = server(1, musicbrainz_release_route);
        let client = MusicBrainzClient::at(ProviderState::new().http(), &base);

        let album = client.release_by_id("release-id").await.unwrap();

        assert_eq!(album.title, "Canonical Album");
        assert_eq!(album.artist.as_deref(), Some("Album Artist"));
        assert_eq!(album.artist_id.as_deref(), Some("artist-id"));
        assert_eq!(album.year.as_deref(), Some("2004"));
        assert_eq!(album.tracks.len(), 2);
        assert_eq!(album.tracks[0].disc_number, Some(1));
        assert_eq!(album.tracks[1].disc_number, Some(2));
        assert_eq!(
            album.tracks[1].artist.as_deref(),
            Some("Album Artist feat. Guest")
        );
        assert_eq!(album.tracks[1].length, Some(202000.0));
        let request = requests.recv().unwrap();
        assert!(
            request.contains("GET /release/release-id?fmt=json&inc=recordings%2Bartist-credits")
        );
    }

    #[tokio::test]
    async fn cover_art_archive_retries_one_transport_failure() {
        let (base, requests) = retry_server();
        let client = RemoteArtworkClient::at(
            ProviderState::new().http(),
            None,
            None,
            &base,
            ArtworkEndpoints {
                cover_art_archive: base.clone(),
                ..ArtworkEndpoints::default()
            },
        );
        let image = client.cover_art_archive("retry").await.unwrap();
        assert!(valid_image(&image));
        let paths = (0..3)
            .map(|_| requests.recv().unwrap().lines().next().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(paths[0].contains("/release/retry"));
        assert!(paths[1].contains("/release/retry"));
        assert!(paths[2].contains("/image"));
    }

    #[test]
    fn validation_rejects_various_and_matches_punctuation_and_chinese_variants() {
        assert!(!discogs_candidate_matches(
            "Various Artists - Album",
            "Artist",
            "Album"
        ));
        assert!(discogs_candidate_matches(
            "F.I.R. = 飛兒樂團 - 無限",
            "飞儿乐团",
            "无限"
        ));
        assert!(discogs_candidate_matches(
            "Artist* - My Album",
            "artist",
            "my album"
        ));
        assert!(!discogs_candidate_matches(
            "Other - Album",
            "Artist",
            "Album"
        ));
        assert!(discogs_candidate_matches(
            "Artist - 同名专辑",
            "Artist",
            "Artist"
        ));
        assert_eq!(album_title_score("無限", "无限"), 100);
        assert!(album_title_score("十年紀念專輯收藏版", "十年记念专辑收藏版") >= 75);
        assert!(album_title_score("Great Album", "Great Album Deluxe") >= 75);
        assert_eq!(album_title_score("Café IV", "Cafe 4"), 100);
        assert_eq!(album_title_score("红 光 辉", "红光辉"), 100);
        assert!(album_title_score("A", "Another") < 75);
    }

    #[tokio::test]
    async fn validated_generic_search_skips_mismatch_and_uses_authorized_candidate() {
        let (base, requests) = server(2, generic_route);
        let client = DiscogsClient::at(ProviderState::new().http(), Some("secret".into()), &base);
        let image = client
            .album_cover("飞儿乐团", "无限", None, None)
            .await
            .unwrap();
        assert!(valid_image(&image));
        let search = requests.recv().unwrap();
        assert!(search.contains("GET /database/search?"));
        assert!(search.contains("type=release"));
        assert!(search
            .to_ascii_lowercase()
            .contains("authorization: discogs token=secret"));
        assert!(requests.recv().unwrap().contains("GET /image "));
    }

    #[tokio::test]
    async fn direct_release_precedes_artist_and_generic_search() {
        let (base, requests) = server(2, direct_route);
        let client = DiscogsClient::at(ProviderState::new().http(), None, &base);
        let image = client
            .album_cover("Artist", "Album", Some("42"), Some("7"))
            .await
            .unwrap();
        assert!(valid_image(&image));
        assert!(requests.recv().unwrap().contains("GET /releases/42 "));
        assert!(requests.recv().unwrap().contains("GET /image "));
    }

    #[tokio::test]
    async fn non_ok_and_malformed_json_fall_through_to_validated_search() {
        let (base, requests) = server(4, fallback_route);
        let client = DiscogsClient::at(ProviderState::new().http(), None, &base);
        let image = client
            .album_cover("Artist", "Album", Some("404"), Some("7"))
            .await
            .unwrap();
        assert!(valid_image(&image));
        let paths = (0..4)
            .map(|_| requests.recv().unwrap().lines().next().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(paths[0].contains("/releases/404"));
        assert!(paths[1].contains("/artists/7/releases"));
        assert!(paths[2].contains("/database/search"));
        assert!(paths[3].contains("/fallback"));
    }

    #[tokio::test]
    async fn remote_album_falls_through_caa_and_discogs_to_theaudiodb() {
        let (base, requests) = server(5, album_provider_route);
        let client = RemoteArtworkClient::at(
            ProviderState::new().http(),
            None,
            Some("key".into()),
            &format!("{base}/discogs"),
            endpoints(&base),
        );
        let image = client
            .album_cover(Some("Artist"), Some("Album"), Some("mbid"), None, None)
            .await
            .unwrap();
        assert_eq!(image.source, "theaudiodb");
        assert!(valid_image(&image));
        let paths = (0..5)
            .map(|_| requests.recv().unwrap().lines().next().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(paths[0].contains("/caa/release/mbid"));
        assert!(paths[1].contains("/invalid"));
        assert!(paths[2].contains("/discogs/database/search"));
        assert!(paths[3].contains("/audio/key/searchalbum.php"));
        assert!(paths[4].contains("/audio-image"));
    }

    #[tokio::test]
    async fn wikimedia_resolves_search_entity_and_commons_image() {
        let (base, requests) = server(3, wikimedia_route);
        let client = RemoteArtworkClient::at(
            ProviderState::new().http(),
            None,
            None,
            &format!("{base}/discogs"),
            endpoints(&base),
        );
        let image = client.wikimedia("Artist").await.unwrap();
        assert_eq!(image.source, "wikimedia");
        assert!(valid_image(&image));
        let paths = (0..3)
            .map(|_| requests.recv().unwrap().lines().next().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(paths[0].contains("/wiki/search?"));
        assert!(paths[1].contains("/wiki/entity/Q1.json"));
        assert!(paths[2].contains("/wiki/file/Artist_Photo.jpg"));
    }

    #[tokio::test]
    async fn artist_uses_direct_then_musicbrainz_alias_discogs_identity() {
        let (base, requests) = server(7, artist_provider_route);
        let client = RemoteArtworkClient::at(
            ProviderState::new().http(),
            None,
            None,
            &format!("{base}/discogs"),
            endpoints(&base),
        );
        let image = client.artist_image("原名", Some("9")).await.unwrap();
        assert_eq!(image.source, "discogs");
        assert!(valid_image(&image));
        let paths = (0..7)
            .map(|_| requests.recv().unwrap().lines().next().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(paths[0].contains("/discogs/artists/9"));
        assert!(paths[1].contains("artist=%E5%8E%9F%E5%90%8D"));
        assert!(paths[2].contains("q=%E5%8E%9F%E5%90%8D"));
        assert!(paths[3].contains("/mb/artist/"));
        assert!(paths[4].contains("artist=Alias"));
        assert!(paths[5].contains("/discogs/artists/7"));
        assert!(paths[6].contains("/artist-image"));
    }

    #[tokio::test]
    async fn audit_alias_is_returned_only_after_exact_discogs_validation() {
        let (base, requests) = server(4, artist_provider_route);
        let client = RemoteArtworkClient::at(
            ProviderState::new().http(),
            None,
            None,
            &format!("{base}/discogs"),
            endpoints(&base),
        );

        let alias = client.validated_discogs_alias("原名").await;

        assert_eq!(alias, DiscogsAliasResolution::Alias("Alias".into()));
        let paths = (0..4)
            .map(|_| requests.recv().unwrap().lines().next().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(paths[0].contains("artist=%E5%8E%9F%E5%90%8D"));
        assert!(paths[1].contains("q=%E5%8E%9F%E5%90%8D"));
        assert!(paths[2].contains("/mb/artist/"));
        assert!(paths[3].contains("artist=Alias"));
    }
}
