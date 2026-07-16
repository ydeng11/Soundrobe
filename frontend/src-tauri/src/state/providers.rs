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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    type Route = fn(&str, &str) -> (&'static str, String, &'static str);

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
                let _ = send.send(request);
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
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
        ("200 OK", "jpeg-bytes".to_string(), "image/jpeg")
    }

    fn direct_route(path: &str, base: &str) -> (&'static str, String, &'static str) {
        if path == "/releases/42" {
            return (
                "200 OK",
                format!(r#"{{"images":[{{"type":"primary","uri":"{base}/image"}}]}}"#),
                "application/json",
            );
        }
        ("200 OK", "direct-image".to_string(), "image/jpeg")
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
        ("200 OK", "fallback-image".to_string(), "image/jpeg")
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
        assert_eq!(image.bytes, b"jpeg-bytes");
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
        assert_eq!(image.bytes, b"direct-image");
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
        assert_eq!(image.bytes, b"fallback-image");
        let paths = (0..4)
            .map(|_| requests.recv().unwrap().lines().next().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(paths[0].contains("/releases/404"));
        assert!(paths[1].contains("/artists/7/releases"));
        assert!(paths[2].contains("/database/search"));
        assert!(paths[3].contains("/fallback"));
    }
}
