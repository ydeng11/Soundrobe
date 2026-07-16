//! Shared provider foundation for remote metadata and artwork.
//!
//! Providers own no mutable request state; the managed client is cheaply cloneable,
//! so no state lock is held while a request is in flight.

use opencc_fmmseg::OpenCC;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteImage {
    pub source: &'static str,
    pub bytes: Vec<u8>,
    pub mime: String,
    pub url: String,
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
}
