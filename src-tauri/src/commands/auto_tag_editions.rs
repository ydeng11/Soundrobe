//! Discogs edition fallback. Deliberately stricter about album coverage and
//! ordering than normal matching; its timing exception never reaches the matcher.
use super::*;
use crate::state::providers::clean_discogs_artist;
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedMatch {
    #[serde(skip)]
    pub tracks: Vec<TrackCandidate>,
    pub coverage: usize,
    pub local_year: Option<String>,
    pub release_year: Option<String>,
    pub duration_exception: Option<DurationException>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DurationException {
    pub track_number: u32,
    pub local_seconds: f64,
    pub provider_seconds: f64,
}

fn edition_title(title: &str) -> String {
    static BONUS: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let bonus = BONUS.get_or_init(|| {
        Regex::new(
            r"(?i)\s*[\[(](?:(?:japan|japanese|deluxe|international)\s+)?bonus\s+track[\])]\s*$",
        )
        .expect("valid bonus annotation regex")
    });
    normalized_track_title(&bonus.replace(title, ""))
}

fn normalized_track_title(title: &str) -> String {
    title
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn accept_edition(
    request: &LookupRequest,
    candidate: &AlbumCandidate,
) -> Result<AcceptedMatch, String> {
    let reject =
        || "Discogs edition lacks complete ordered title and duration evidence".to_string();
    let remote = candidate_tracks_for_request(request, candidate);
    if candidate.source != LookupSource::Discogs
        || request.tracks.is_empty()
        || remote.len() != request.tracks.len()
    {
        return Err(reject());
    }
    let mut by_title = HashMap::new();
    let mut positions = HashSet::new();
    for track in &remote {
        let title = edition_title(track.title.as_deref().ok_or_else(reject)?);
        let position = (
            track.disc_number.unwrap_or(1),
            track.track_number.ok_or_else(reject)?,
        );
        if title.is_empty()
            || by_title.insert(title, track).is_some()
            || !positions.insert(position)
        {
            return Err(reject());
        }
    }
    let mut seen = HashSet::new();
    let mut tracks = Vec::new();
    let mut duration_exception = None;
    for local in &request.tracks {
        // Use only an exact normalized title here, never containment or position.
        let title = edition_title(local.title.as_deref().ok_or_else(reject)?);
        let provider = by_title.get(&title).ok_or_else(reject)?;
        if !seen.insert(title)
            || local.track_number.is_none()
            || local.track_number != provider.track_number
            || local.disc_number.unwrap_or(1) != provider.disc_number.unwrap_or(1)
        {
            return Err(reject());
        }
        let local_seconds = local
            .length
            .filter(|n| n.is_finite() && *n > 0.0)
            .ok_or_else(reject)?;
        let provider_seconds = provider
            .length
            .filter(|n| n.is_finite() && *n > 0.0)
            .ok_or_else(reject)?;
        let difference = (local_seconds - provider_seconds).abs();
        if difference > 5.0_f64.max(local_seconds * 0.03) {
            if difference > 10.0 || duration_exception.is_some() {
                return Err(reject());
            }
            duration_exception = Some(DurationException {
                track_number: local.track_number.unwrap(),
                local_seconds,
                provider_seconds,
            });
        }
        let mut track = (*provider).clone();
        track.length = local.length;
        track.filename = local.filename.clone();
        // Keep the local spelling unless removing an explicit bonus annotation.
        track.title = if edition_title(local.title.as_deref().unwrap())
            == normalized_track_title(local.title.as_deref().unwrap())
        {
            local.title.clone()
        } else {
            provider.title.clone()
        };
        tracks.push(track);
    }
    preserve_collaborators(&request.tracks, &mut tracks);
    // Reuse all existing album/artist/country and strong coverage safeguards.
    let mut validated = candidate.clone();
    validated.year = request.year_hint.clone();
    validated.tracks = tracks.clone();
    strict_provider_candidate_credibility(request, &validated)?;
    Ok(AcceptedMatch {
        tracks,
        coverage: remote.len(),
        local_year: request.year_hint.clone(),
        release_year: candidate.year.clone(),
        duration_exception,
    })
}

pub(super) fn preserve_collaborators(local: &[TrackCandidate], mapped: &mut [TrackCandidate]) {
    for (local, mapped) in local.iter().zip(mapped) {
        let existing = split_collaborative_artists(&local.artist, &local.artists)
            .into_iter()
            .filter(|name| !is_placeholder_artist_identity(name))
            .collect::<Vec<_>>();
        let remote = split_collaborative_artists(&mapped.artist, &mapped.artists)
            .into_iter()
            .filter(|name| !is_placeholder_artist_identity(name))
            .collect::<Vec<_>>();
        let same_artist = |left: &str, right: &str| {
            exact_artist_identity(&clean_discogs_artist(left), &clean_discogs_artist(right))
        };
        // Retain the existing credit order when the provider adds no performers.
        if existing.len() > remote.len()
            && remote
                .iter()
                .all(|artist| existing.iter().any(|name| same_artist(name, artist)))
        {
            mapped.artist = Some(existing.join(" & "));
            mapped.artists = existing;
            continue;
        }
        let mut merged = remote;
        for artist in existing {
            if !merged.iter().any(|name| same_artist(name, &artist)) {
                merged.push(artist);
            }
        }
        if !merged.is_empty() {
            mapped.artist = Some(merged.join(" & "));
            mapped.artists = merged;
        }
    }
}

const DETAIL_LIMIT: usize = 40;
const MASTER_LIMIT: usize = 2;
const PAGE_LIMIT: u32 = 5;

pub(super) struct DiscogsDiscovery<'a> {
    client: &'a DiscogsClient,
    cache: &'a CacheState,
    cancelled: &'a AtomicBool,
    details: HashMap<String, AlbumCandidate>,
    attempted: HashSet<String>,
    pub diagnostics: Vec<serde_json::Value>,
}

impl<'a> DiscogsDiscovery<'a> {
    pub fn new(
        client: &'a DiscogsClient,
        cache: &'a CacheState,
        cancelled: &'a AtomicBool,
    ) -> Self {
        Self {
            client,
            cache,
            cancelled,
            details: HashMap::new(),
            attempted: HashSet::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn remember(&mut self, candidate: &AlbumCandidate) {
        if let Some(id) = &candidate.discogs_release_id {
            self.details
                .entry(id.clone())
                .or_insert_with(|| candidate.clone());
        }
    }

    pub fn candidates(&self) -> Vec<AlbumCandidate> {
        let mut candidates = self.details.values().cloned().collect::<Vec<_>>();
        candidates.sort_by(|a, b| a.discogs_release_id.cmp(&b.discogs_release_id));
        candidates
    }

    pub async fn detail(&mut self, id: &str) -> Result<AlbumCandidate, String> {
        check_cancelled(self.cancelled).map_err(|error| error.to_string())?;
        if let Some(candidate) = self.details.get(id) {
            return Ok(candidate.clone());
        }
        if let Some(album) = cached_release_detail(self.cache, "discogs-v4", id) {
            let candidate = discogs_candidate(album);
            self.remember(&candidate);
            return Ok(candidate);
        }
        if self.attempted.contains(id) {
            return Err("Discogs detail previously failed in this run".into());
        }
        if self.attempted.len() >= DETAIL_LIMIT {
            return Err("Discogs discovery incomplete: release detail limit reached".into());
        }
        self.attempted.insert(id.to_string());
        let album = self.client.release_metadata_result(id).await?;
        check_cancelled(self.cancelled).map_err(|error| error.to_string())?;
        if let Ok(value) = serde_json::to_value(&album) {
            let _ = self.cache.set_release_detail("discogs-v4", id, &value);
        }
        let candidate = discogs_candidate(album);
        self.remember(&candidate);
        Ok(candidate)
    }

    pub async fn initial_search(
        &mut self,
        request: &LookupRequest,
    ) -> Result<Vec<AlbumCandidate>, String> {
        let (Some(artist), Some(album)) = (
            request.artist_hint.as_deref(),
            request.album_hint.as_deref(),
        ) else {
            return Ok(Vec::new());
        };
        let mut params = vec![
            ("type", "release"),
            ("artist", artist),
            ("release_title", album),
        ];
        if let Some(year) = request.year_hint.as_deref() {
            params.push(("year", year));
        }
        if let Some(country) = request
            .country_hint
            .as_deref()
            .and_then(discogs_country_name)
        {
            params.push(("country", country));
        }
        let (mut summaries, _) = self.client.search_release_summaries(&params, 1, 10).await?;
        if summaries.is_empty() {
            // Preserve the prior generic-search fallback, but retain an explicit
            // country constraint and fetch only concrete releases through the budget.
            let query = format!("{artist} {album}");
            params.retain(|(key, _)| matches!(*key, "type" | "country"));
            params.push(("q", &query));
            check_cancelled(self.cancelled).map_err(|error| error.to_string())?;
            summaries = self
                .client
                .search_release_summaries(&params, 1, 10)
                .await?
                .0;
        }
        let mut found = Vec::new();
        let mut error = None;
        for summary in summaries.into_iter().take(10) {
            if summary.kind.as_deref() == Some("master") {
                continue;
            }
            match self.detail(&summary.id).await {
                Ok(candidate) => found.push(candidate),
                Err(reason) => error = Some(reason),
            }
        }
        if let Some(error) = error {
            // Successful details remain available to the expansion stage.
            return Err(error);
        }
        Ok(found)
    }

    pub async fn expand(&mut self, request: &LookupRequest) -> Result<Vec<AlbumCandidate>, String> {
        check_cancelled(self.cancelled).map_err(|error| error.to_string())?;
        let (Some(artist), Some(album)) = (
            request.artist_hint.as_deref(),
            request.album_hint.as_deref(),
        ) else {
            return Ok(Vec::new());
        };
        let key = query_hash(request);
        let cached = self
            .cache
            .artist_releases("discogs-edition-masters-v2", &key, 1)
            .and_then(|value| serde_json::from_value(value).ok());
        let (masters, total): (Vec<crate::state::providers::ReleaseSearchSummary>, u32) =
            match cached {
                Some(value) => value,
                None => {
                    // Country is checked on concrete editions; masters have no country identity.
                    let found = self
                        .client
                        .search_release_summaries(
                            &[
                                ("type", "master"),
                                ("artist", artist),
                                ("release_title", album),
                            ],
                            1,
                            100,
                        )
                        .await?;
                    check_cancelled(self.cancelled).map_err(|error| error.to_string())?;
                    let _ = self.cache.set_artist_releases(
                        "discogs-edition-masters-v2",
                        &key,
                        1,
                        &serde_json::json!(found),
                    );
                    found
                }
            };
        let summaries_count = masters.len();
        let mut masters = masters
            .into_iter()
            .filter(|master| {
                master.kind.as_deref() == Some("master")
                    && exact_album_identity(album, &master.title)
                    && master.artist.as_deref().is_some_and(|name| {
                        let name = clean_discogs_artist(name);
                        exact_artist_identity(artist, &name)
                            || request
                                .artist_aliases
                                .iter()
                                .any(|alias| exact_artist_identity(alias, &name))
                    })
            })
            .collect::<Vec<_>>();
        masters.sort_by(|a, b| a.id.cmp(&b.id));
        masters.dedup_by(|a, b| a.id == b.id);
        let mut incomplete = masters.len() > MASTER_LIMIT || total as usize > summaries_count;
        let mut versions = Vec::new();
        let mut failure = None;
        for master in masters.into_iter().take(MASTER_LIMIT) {
            for page in 1..=PAGE_LIMIT {
                check_cancelled(self.cancelled).map_err(|error| error.to_string())?;
                let cached = self
                    .cache
                    .artist_releases("discogs-master-versions-v1", &master.id, page)
                    .and_then(|value| serde_json::from_value(value).ok());
                let result = match cached {
                    Some(value) => Ok(value),
                    None => {
                        self.client
                            .master_versions_page_result(&master.id, page)
                            .await
                    }
                };
                match result {
                    Ok((releases, pages)) => {
                        check_cancelled(self.cancelled).map_err(|error| error.to_string())?;
                        let _ = self.cache.set_artist_releases(
                            "discogs-master-versions-v1",
                            &master.id,
                            page,
                            &serde_json::json!((&releases, pages)),
                        );
                        versions.extend(releases);
                        if pages <= page {
                            break;
                        }
                        if page == PAGE_LIMIT {
                            incomplete = true;
                        }
                    }
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
        }
        versions.retain(|version| {
            exact_album_identity(album, &version.title)
                && request.country_hint.as_deref().is_none_or(|hint| {
                    version
                        .country
                        .as_deref()
                        .is_none_or(|country| countries_match(hint, country))
                })
        });
        versions.sort_by(|a, b| {
            let country = |v: &ProviderReleaseSummary| {
                request
                    .country_hint
                    .as_deref()
                    .zip(v.country.as_deref())
                    .is_some_and(|(a, b)| countries_match(a, b))
            };
            let year = |v: &ProviderReleaseSummary| {
                request
                    .year_hint
                    .as_ref()
                    .zip(v.year)
                    .is_some_and(|(a, b)| a == &b.to_string())
            };
            country(b)
                .cmp(&country(a))
                .then_with(|| year(b).cmp(&year(a)))
                .then_with(|| a.id.cmp(&b.id))
        });
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        for version in versions {
            if !seen.insert(version.id.clone()) {
                continue;
            }
            match self.detail(&version.id).await {
                Ok(candidate) => {
                    let assessment = provider_candidate_credibility(request, &candidate);
                    self.diagnostics.push(serde_json::json!({"stage":"discogs_versions", "releaseId":version.id,
                        "rejection":assessment.as_ref().err(), "detailRequests":self.attempted.len()}));
                    let exact = assessment.is_ok_and(|score| score.exact_track_count);
                    found.push(candidate);
                    if exact {
                        return Ok(found);
                    }
                }
                Err(error) => {
                    failure = Some(error);
                    if self.cancelled.load(Ordering::Acquire)
                        || self.attempted.len() >= DETAIL_LIMIT
                    {
                        break;
                    }
                }
            }
        }
        if incomplete || failure.is_some() {
            let reason = failure.unwrap_or_else(|| "master/version page limit reached".into());
            self.diagnostics.push(
                serde_json::json!({"stage":"discogs_versions", "incomplete":true, "reason":reason}),
            );
            return Err(format!("Discogs discovery incomplete: {reason}"));
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collaborator_merge_preserves_overlapping_credits_and_spelling_variants() {
        for (existing, provider, expected) in [
            (vec!["???"], vec!["Artist"], vec!["Artist"]),
            (
                vec!["Artist", "Guest B"],
                vec!["Artist", "Guest C"],
                vec!["Artist", "Guest C", "Guest B"],
            ),
            (
                vec!["Artist", "Guest B", "Guest C"],
                vec!["Artist", "Guest B (2)"],
                vec!["Artist", "Guest B", "Guest C"],
            ),
            (
                vec!["Artist", "Unresolved Alias", "Guest C"],
                vec!["Artist", "Provider Name"],
                vec!["Artist", "Provider Name", "Unresolved Alias", "Guest C"],
            ),
        ] {
            let track = |names: Vec<&str>| TrackCandidate {
                artist: Some(names.join(" & ")),
                artists: names.into_iter().map(str::to_string).collect(),
                ..Default::default()
            };
            let mut mapped = vec![track(provider)];
            preserve_collaborators(&[track(existing)], &mut mapped);
            assert_eq!(mapped[0].artists, expected);
            assert_eq!(
                mapped[0].artist.as_deref(),
                Some(expected.join(" & ").as_str())
            );
        }
    }

    fn cache() -> (PathBuf, CacheState) {
        let root = std::env::temp_dir().join(format!("soundrobe-edition-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let cache = CacheState::new(root.clone());
        assert!(cache.initialize(Some(root.join("cache.db").to_str().unwrap())));
        (root, cache)
    }

    fn request() -> LookupRequest {
        LookupRequest {
            artist_hint: Some("Artist".into()),
            album_hint: Some("Album".into()),
            ..Default::default()
        }
    }

    fn masters(cache: &CacheState, count: usize) {
        let values = (0..count).map(|i| serde_json::json!({
            "provider":"discogs", "id":i.to_string(), "kind":"master", "title":"Album", "artist":"Artist", "formats":[]
        })).collect::<Vec<_>>();
        cache
            .set_artist_releases(
                "discogs-edition-masters-v2",
                &query_hash(&request()),
                1,
                &serde_json::json!((values, count)),
            )
            .unwrap();
    }

    #[tokio::test]
    async fn discovery_limits_are_incomplete_not_confirmed_no_match() {
        let (root, cache) = cache();
        let client = DiscogsClient::at(reqwest::Client::new(), None, "http://127.0.0.1:1");
        let cancelled = AtomicBool::new(false);
        masters(&cache, 3);
        for master in ["0", "1"] {
            for page in 1..=5 {
                cache
                    .set_artist_releases(
                        "discogs-master-versions-v1",
                        master,
                        page,
                        &serde_json::json!((Vec::<ProviderReleaseSummary>::new(), 6)),
                    )
                    .unwrap();
            }
        }
        let mut discovery = DiscogsDiscovery::new(&client, &cache, &cancelled);
        let error = discovery.expand(&request()).await.unwrap_err();
        assert!(error.contains("master/version page limit"), "{error}");
        assert_eq!(discovery.diagnostics.last().unwrap()["incomplete"], true);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn discovery_cancelled_before_network_and_detail_budget_shared_across_stages() {
        let (root, cache) = cache();
        let client = DiscogsClient::at(reqwest::Client::new(), None, "http://127.0.0.1:1");
        let cancelled = AtomicBool::new(true);
        let mut discovery = DiscogsDiscovery::new(&client, &cache, &cancelled);
        assert!(discovery
            .expand(&request())
            .await
            .unwrap_err()
            .to_lowercase()
            .contains("cancel"));
        assert!(discovery.attempted.is_empty());
        cancelled.store(false, Ordering::Release);
        discovery.attempted.extend((0..40).map(|id| id.to_string()));
        assert!(discovery
            .detail("41")
            .await
            .unwrap_err()
            .contains("detail limit"));
        assert_eq!(discovery.attempted.len(), 40);
        assert!(discovery
            .detail("1")
            .await
            .unwrap_err()
            .contains("previously failed"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn discovery_provider_failure_is_incomplete() {
        let (root, cache) = cache();
        masters(&cache, 1);
        let client = DiscogsClient::at(reqwest::Client::new(), None, "http://127.0.0.1:1");
        let cancelled = AtomicBool::new(false);
        let mut discovery = DiscogsDiscovery::new(&client, &cache, &cancelled);
        assert!(discovery
            .expand(&request())
            .await
            .unwrap_err()
            .contains("discovery incomplete"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn malformed_discovery_does_not_cache_no_match_or_reuse_legacy_empty_results() {
        use std::io::{Read, Write};
        let (root, cache) = cache();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                assert!(stream.read(&mut request).unwrap() > 0);
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
            }
        });
        let request = request();
        cache
            .set_artist_releases(
                "discogs-edition-masters-v1",
                &query_hash(&request),
                1,
                &serde_json::json!((Vec::<serde_json::Value>::new(), 0)),
            )
            .unwrap();
        let client = DiscogsClient::at(reqwest::Client::new(), None, &base);
        let cancelled = AtomicBool::new(false);
        let mut discovery = DiscogsDiscovery::new(&client, &cache, &cancelled);
        assert!(discovery.initial_search(&request).await.is_err());
        assert!(discovery.expand(&request).await.is_err());
        assert!(cache
            .artist_releases("discogs-edition-masters-v2", &query_hash(&request), 1)
            .is_none());
        server.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn discovery_cancellation_after_response_does_not_cache_detail() {
        use std::io::{Read, Write};
        let (root, cache) = cache();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let cancelled = Arc::new(AtomicBool::new(false));
        let server_cancelled = Arc::clone(&cancelled);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).unwrap() > 0);
            server_cancelled.store(true, Ordering::Release);
            let body = r#"{"id":42,"title":"Album","tracklist":[]}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let client = DiscogsClient::at(reqwest::Client::new(), None, &base);
        let mut discovery = DiscogsDiscovery::new(&client, &cache, &cancelled);
        assert!(discovery.detail("42").await.is_err());
        assert!(discovery.candidates().is_empty());
        assert!(cached_release_detail(&cache, "discogs-v4", "42").is_none());
        server.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn initial_search_retains_yearless_fallback_and_country_constraint() {
        use std::io::{Read, Write};
        let (root, cache) = cache();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0; 4096];
                let size = stream.read(&mut buffer).unwrap();
                let path = String::from_utf8_lossy(&buffer[..size]);
                assert!(path.contains("country=Japan"));
                assert_eq!(path.contains("year=2014"), index == 0);
                let body = if index == 0 {
                    r#"{"results":[]}"#
                } else {
                    r#"{"results":[{"id":42,"type":"release","title":"Artist - Album"}]}"#
                };
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let client = DiscogsClient::at(reqwest::Client::new(), None, &base);
        let cancelled = AtomicBool::new(false);
        let mut discovery = DiscogsDiscovery::new(&client, &cache, &cancelled);
        discovery.remember(&AlbumCandidate {
            discogs_release_id: Some("42".into()),
            ..Default::default()
        });
        let mut request = request();
        request.year_hint = Some("2014".into());
        request.country_hint = Some("JP".into());
        assert_eq!(discovery.initial_search(&request).await.unwrap().len(), 1);
        server.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
