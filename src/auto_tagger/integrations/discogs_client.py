"""Discogs API integration for album metadata lookup.

Discogs provides a free, unauthenticated API with:
- Database search (release, master, artist)
- Release details (full tracklist, artists, year, genres)
- Master release (consolidated tracklist across versions)
- 19M+ releases vs. MusicBrainz's ~3M

Rate limit: 25 req/min unauthenticated, 60 req/min with token.
"""

from __future__ import annotations

import hashlib
import itertools
import re
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

import httpx

from auto_tagger.features.cover_art import CoverArtImage, CoverArtResult, CoverArtStatus
from auto_tagger.integrations.aliases import get_all_name_variants, get_aliases
from auto_tagger.integrations.candidates import (
    AlbumCandidate,
    LookupSource,
    TrackCandidate,
)

# ── Module-level proxy pool (fetched once per process) ───────────────

_PROXY_URL: str | None = None
_PROXY_POOL: list[str] | None = None
_PROXY_CYCLE: itertools.cycle | None = None


def _load_proxy_list(proxy_url: str | None) -> list[str]:
    """Fetch and parse the Webshare proxy-list URL.

    Returns a list of ``user:pass@host:port`` strings.
    Caches the result in a module-level variable so it is fetched
    only once per process lifetime.
    """
    global _PROXY_URL, _PROXY_POOL, _PROXY_CYCLE

    if not proxy_url:
        return []

    # Return cached pool if the URL hasn't changed
    if _PROXY_URL == proxy_url and _PROXY_POOL is not None:
        return _PROXY_POOL

    try:
        response = httpx.get(proxy_url, timeout=15)
        if response.status_code == 200 and response.text.strip():
            proxies = [
                line.strip()
                for line in response.text.strip().splitlines()
                if line.strip() and ":" in line
            ]
            _PROXY_URL = proxy_url
            _PROXY_POOL = proxies
            _PROXY_CYCLE = itertools.cycle(proxies) if proxies else None
            return proxies
    except httpx.HTTPError:
        pass

    _PROXY_URL = proxy_url
    _PROXY_POOL = []
    _PROXY_CYCLE = None
    return []


def _next_proxy() -> str | None:
    """Return the next proxy from the round-robin pool, or None."""
    if _PROXY_CYCLE is None:
        return None
    return next(_PROXY_CYCLE)


def _backoff_sleep(attempt: int) -> None:
    """Sleep with exponential backoff: 0.5s, 1s, 2s, ..."""
    time.sleep(0.5 * (2 ** attempt))


# ── In-memory Discogs API response cache ────────────────────────────


class DiscogsCache:
    """In-memory TTL cache for individual Discogs API responses.

    Keys are ``{http_method}:{endpoint_path}:{sorted_params}``.
    Values are ``(expiry_timestamp, data)`` tuples.
    Scoped to process lifetime — cleared on exit.
    """

    def __init__(self, ttl_seconds: int = 3600):
        self._ttl = ttl_seconds
        self._store: dict[str, tuple[float, Any]] = {}

    def get(self, key: str) -> Any | None:
        """Return cached data if present and not expired."""
        entry = self._store.get(key)
        if entry is None:
            return None
        expiry, data = entry
        if time.monotonic() > expiry:
            del self._store[key]
            return None
        return data

    def set(self, key: str, data: Any) -> None:
        """Store data with a TTL expiry."""
        self._store[key] = (time.monotonic() + self._ttl, data)

    def clear(self) -> None:
        """Clear all cached entries."""
        self._store.clear()


# ── Rate limiter (reused from beets_client pattern) ─────────────────


class _RateLimiter:
    """Simple interval-based rate limiter."""

    def __init__(
        self,
        interval_seconds: float = 2.5,
        now_func: Callable[[], float] | None = None,
        sleep_func: Callable[[float], None] | None = None,
    ):
        # 25 req/min = 2.4s interval; use 2.5 for safety.
        # With a token (60 req/min), 1.0s. Default to the safe side.
        self.interval_seconds = interval_seconds
        self.now_func = now_func or time.monotonic
        self.sleep_func = sleep_func or time.sleep
        self._last_call: float | None = None

    def wait(self) -> None:
        """Sleep if needed to maintain the configured interval."""
        now = self.now_func()
        if self._last_call is not None:
            elapsed = now - self._last_call
            remaining = self.interval_seconds - elapsed
            if remaining > 0:
                self.sleep_func(remaining)
                now += remaining
        self._last_call = now


# ── Image disk cache ────────────────────────────────────────────────


class _ImageDiskCache:
    """Persistent on-disk cache for downloaded Discogs images.

    Images are stored under *cache_dir* keyed by SHA-256 hash of
    the source URL (first 32 hex chars). Metadata (mime type) is stored
    alongside in a JSON sidecar.
    """

    def __init__(self, cache_dir: str | Path | None = None):
        if cache_dir is None:
            cache_dir = Path.home() / ".cache" / "auto-tagger" / "discogs-images"
        self._cache_dir = Path(cache_dir)
        self._cache_dir.mkdir(parents=True, exist_ok=True)

    def _key(self, url: str) -> str:
        return hashlib.sha256(url.encode("utf-8")).hexdigest()[:32]

    def get(self, url: str) -> tuple[bytes, str] | None:
        """Return ``(image_bytes, mime_type)`` from disk cache, or None."""
        key = self._key(url)
        img_path = self._cache_dir / f"{key}.img"
        meta_path = self._cache_dir / f"{key}.meta"
        if img_path.exists() and meta_path.exists():
            try:
                data = img_path.read_bytes()
                mime_type = meta_path.read_text().strip()
                return data, mime_type
            except OSError:
                return None
        return None

    def set(self, url: str, data: bytes, mime_type: str) -> None:
        """Store image bytes and MIME type to disk cache."""
        key = self._key(url)
        img_path = self._cache_dir / f"{key}.img"
        meta_path = self._cache_dir / f"{key}.meta"
        try:
            img_path.write_bytes(data)
            meta_path.write_text(mime_type)
        except OSError:
            pass


# ── Main client ─────────────────────────────────────────────────────


class DiscogsError(Exception):
    """Raised when the Discogs API returns an error."""


class DiscogsClient:
    """Thin wrapper around the free Discogs API with caching, retry, and proxy rotation."""

    BASE_URL = "https://api.discogs.com"
    MAX_RETRIES = 3

    def __init__(
        self,
        token: str | None = None,
        user_agent: str = "auto-tagger/0.1.0",
        max_candidates: int = 3,
        timeout_seconds: int = 20,
        proxy_url: str | None = None,
        cache_ttl_seconds: int = 3600,
        image_cache_dir: str | Path | None = None,
        http_client: httpx.Client | None = None,
        max_retries: int = 3,
    ):
        self.token = token
        self.user_agent = user_agent
        self.max_candidates = max_candidates
        self.timeout_seconds = timeout_seconds
        self.proxy_url = proxy_url
        self.MAX_RETRIES = max_retries
        self._cache = DiscogsCache(ttl_seconds=cache_ttl_seconds)
        self._image_cache = _ImageDiskCache(image_cache_dir)

        # Rate limiter: 60 req/min with token, 25 req/min without
        interval = 1.0 if token else 2.5
        self._rate_limiter = _RateLimiter(interval_seconds=interval)

        # Injectable httpx.Client for testing; otherwise lazily created.
        self._http_client = http_client

        # Shared httpx.Client kwargs for lazy creation
        self._client_kwargs: dict[str, Any] = {
            "timeout": httpx.Timeout(timeout_seconds),
            "follow_redirects": True,
        }

        # Load proxy pool if a proxy URL is configured and no token is set
        self._proxies: list[str] = []
        if not token and proxy_url:
            self._proxies = _load_proxy_list(proxy_url)

    # ── Public API ──────────────────────────────────────────────────

    def search_album(self, artist: str, album: str) -> list[AlbumCandidate]:
        """Search Discogs for an album by artist and album name.

        Iterates through name variants (English aliases, TC, SC, etc.)
        and album-only fallback to handle cross-script mismatches
        (e.g. Chinese 久石让 vs English "Joe Hisaishi").

        Returns up to max_candidates AlbumCandidate results with source=DISCOGS.
        """
        if not album:
            return []

        seen_ids: set[int] = set()
        candidates: list[AlbumCandidate] = []

        # 1. Try each name variant + album
        for name_variant in get_all_name_variants(artist):
            query = f"{name_variant} {album}".strip()
            try:
                results = self._search(query, per_page=self.max_candidates)
            except DiscogsError:
                continue
            for item in results:
                release_id = item.get("id")
                if release_id and release_id in seen_ids:
                    continue
                if release_id:
                    seen_ids.add(release_id)
                candidate = self._release_to_candidate(item)
                if candidate is not None:
                    candidates.append(candidate)
            if candidates:
                # Good enough — stop at first batch with results
                break

        # 2. Album-only fallback (catches cross-script mismatches)
        if not candidates:
            try:
                results = self._search(album, per_page=self.max_candidates)
            except DiscogsError:
                pass
            else:
                for item in results:
                    release_id = item.get("id")
                    if release_id and release_id in seen_ids:
                        continue
                    if release_id:
                        seen_ids.add(release_id)
                    candidate = self._release_to_candidate(item)
                    if candidate is not None:
                        candidates.append(candidate)

        return candidates[: self.max_candidates]

    def get_release(self, release_id: int) -> AlbumCandidate | None:
        """Fetch full release details by Discogs release ID."""
        data = self._get(f"/releases/{release_id}")
        return self._full_release_to_candidate(data)

    def get_master(self, master_id: int) -> AlbumCandidate | None:
        """Fetch master release details by Discogs master ID."""
        data = self._get(f"/masters/{master_id}")
        return self._full_release_to_candidate(data)

    def search_artist(self, artist_name: str) -> list[dict[str, Any]]:
        """Search for an artist on Discogs.

        Iterates through name variants (English aliases, TC, SC, etc.)
        to handle cross-script mismatches.

        Returns raw search result dicts with keys:
          id, title, type, thumb, cover_image, resource_url, uri, ...
        """
        if not artist_name:
            return []

        seen_ids: set[int] = set()
        results: list[dict[str, Any]] = []

        for name_variant in get_all_name_variants(artist_name):
            try:
                batch = self._search_artist(name_variant, per_page=5)
            except DiscogsError:
                continue
            for item in batch:
                artist_id = item.get("id")
                if artist_id and artist_id in seen_ids:
                    continue
                if artist_id:
                    seen_ids.add(artist_id)
                results.append(item)
            if results:
                break

        # Album-name-only fallback (unlikely for artist search but safe)
        if not results:
            try:
                results = self._search_artist(artist_name, per_page=5)
            except DiscogsError:
                pass

        return results[:5]

    def get_artist(self, artist_id: int) -> dict[str, Any]:
        """Fetch full artist details from Discogs, including images.

        Returns the raw API response dict with keys:
          id, name, images, profile, releases_url, resource_url, uri, urls, ...

        The images list contains dicts with:
          uri, uri150, height, width, type ("primary" | "secondary")
        """
        return self._get(f"/artists/{artist_id}")

    def fetch_artist_image(self, artist_name: str) -> CoverArtResult:
        """Fetch the primary artist image from Discogs.

        Searches for the artist by name (with alias variants) then fetches
        the first primary image. If no primary image exists, uses the first
        available image (secondary).

        Uses the on-disk image cache to avoid re-downloading.

        Returns CoverArtResult with FETCHED_REMOTE on success, or
        MISSING/FETCH_FAILED on failure.
        """
        if not artist_name:
            return CoverArtResult(CoverArtStatus.MISSING, message="No artist name provided")

        artist_results = self.search_artist(artist_name)
        if not artist_results:
            return CoverArtResult(
                CoverArtStatus.MISSING,
                message=f"No Discogs artist found for: {artist_name}",
            )

        # Try each result until we find an image
        for artist_item in artist_results:
            artist_id = artist_item.get("id")
            if not artist_id:
                continue

            # Check thumb from search result first (fast, avoids extra GET)
            thumb = artist_item.get("cover_image")
            if thumb:
                downloaded = self._download_image_bytes(thumb)
                if downloaded is not None:
                    data, mime_type = downloaded
                    return CoverArtResult(
                        CoverArtStatus.FETCHED_REMOTE,
                        CoverArtImage(data, mime_type, "discogs-artist"),
                    )

            # Full artist detail for images array
            try:
                artist_data = self.get_artist(artist_id)
            except DiscogsError:
                continue

            images = artist_data.get("images", [])
            if not images:
                continue

            primary = next(
                (img for img in images if img.get("type") == "primary"),
                None,
            )
            target = primary or images[0]
            image_url = target.get("uri")
            if not image_url:
                continue

            downloaded = self._download_image_bytes(image_url)
            if downloaded is None:
                continue
            data, mime_type = downloaded
            return CoverArtResult(
                CoverArtStatus.FETCHED_REMOTE,
                CoverArtImage(data, mime_type, "discogs-artist"),
            )

        return CoverArtResult(
            CoverArtStatus.MISSING,
            message=f"No image found for Discogs artist: {artist_name}",
        )

    def fetch_cover_art(self, artist: str, album: str) -> CoverArtResult:
        """Fetch cover art for an album from Discogs.

        Tries in order:
          1. Each name variant (English aliases, TC, SC, etc.) + album
          2. Album-only (catches cross-script mismatches)

        For each query, checks the search result's cover_image field first.
        If missing, fetches the full release detail for images.

        Uses the on-disk image cache to avoid re-downloading.

        Returns CoverArtResult with FETCHED_REMOTE on success, or
        MISSING/FETCH_FAILED on failure.
        """
        if not album:
            return CoverArtResult(CoverArtStatus.MISSING, message="No album name provided")

        # Build ordered list of search queries to try
        queries: list[str] = []

        # 1. Each name variant + album (English aliases, TC, SC, etc.)
        for name_variant in get_all_name_variants(artist):
            variant_query = f"{name_variant} {album}"
            if variant_query not in queries:
                queries.append(variant_query)

        # 2. Album-only fallback (catches cross-script mismatches)
        if album not in queries:
            queries.append(album)

        seen_ids: set[int] = set()

        for query in queries:
            try:
                raw_results = self._search(query, per_page=5)
            except DiscogsError:
                continue

            for item in raw_results:
                release_id = item.get("id")
                if not release_id or release_id in seen_ids:
                    continue
                seen_ids.add(release_id)

                # Check cover_image in search result first (avoids extra GET)
                cover_url = item.get("cover_image")
                if cover_url:
                    downloaded = self._download_image_bytes(cover_url)
                    if downloaded is not None:
                        data, mime_type = downloaded
                        return CoverArtResult(
                            CoverArtStatus.FETCHED_REMOTE,
                            CoverArtImage(data, mime_type, "discogs"),
                        )

                # Fall through to full release GET for images
                try:
                    release_data = self._get(f"/releases/{release_id}")
                except (DiscogsError, httpx.HTTPError):
                    continue

                images = release_data.get("images", [])
                primary = next((img for img in images if img.get("type") == "primary"), None)
                target = primary or (images[0] if images else None)
                if not target:
                    continue

                image_url = target.get("uri")
                if not image_url:
                    continue

                downloaded = self._download_image_bytes(image_url)
                if downloaded is None:
                    continue
                data, mime_type = downloaded
                return CoverArtResult(
                    CoverArtStatus.FETCHED_REMOTE,
                    CoverArtImage(data, mime_type, "discogs"),
                )

        return CoverArtResult(
            CoverArtStatus.MISSING,
            message="No cover art found in Discogs results",
        )

    # ── Internal HTTP layer ─────────────────────────────────────────

    def _request(
        self,
        method: str,
        url: str,
        *,
        params: dict[str, Any] | None = None,
        headers: dict[str, str] | None = None,
        cache_key: str | None = None,
    ) -> httpx.Response:
        """Execute an HTTP request with rate limiting, retry, and proxy rotation.

        Steps:
          1. Wait for rate limiter.
          2. Execute with retry + exponential backoff on 429.
          3. On 429 exhaustion (no token), rotate proxy and retry.

        Returns the ``httpx.Response`` on success.
        Raises ``DiscogsError`` on failure.
        """
        all_headers = {"User-Agent": self.user_agent}
        if headers:
            all_headers.update(headers)

        auth: httpx.BasicAuth | None = None

        last_exc: Exception | None = None
        proxy_attempted = False

        for attempt in range(self.MAX_RETRIES + 1):
            try:
                self._rate_limiter.wait()
                client = self._build_client()
                response = client.request(
                    method,
                    url,
                    params=params,
                    headers=all_headers,
                    auth=auth,
                )

                if response.status_code == 429:
                    if attempt < self.MAX_RETRIES:
                        _backoff_sleep(attempt)
                        continue
                    # Retries exhausted — try proxy rotation if no token
                    if not self.token and not proxy_attempted:
                        rotated = self._rotate_proxy()
                        if rotated:
                            proxy_attempted = True
                            retried = self._retry_with_new_proxy(
                                method, url, params, all_headers, auth,
                            )
                            if retried is not None:
                                return retried
                    raise DiscogsError(
                        "Discogs rate limit exceeded after retries and proxy rotation. "
                        "Wait before retrying."
                    )

                if response.status_code != 200:
                    raise DiscogsError(
                        f"Discogs returned HTTP {response.status_code}: {response.text[:200]}"
                    )

                return response

            except httpx.HTTPError as exc:
                last_exc = exc
                if attempt < self.MAX_RETRIES:
                    _backoff_sleep(attempt)
                    continue
                raise DiscogsError(f"Discogs request failed: {exc}") from exc

        raise DiscogsError(f"Discogs request failed after retries" +
                           (f": {last_exc}" if last_exc else ""))

    def _retry_with_new_proxy(
        self,
        method: str,
        url: str,
        params: dict[str, Any] | None,
        headers: dict[str, str],
        auth: httpx.BasicAuth | None,
    ) -> httpx.Response | None:
        """Retry a request with a rotated proxy after rate-limit exhaustion.

        Returns the response on success, or None if all retries fail.
        """
        for retry_attempt in range(self.MAX_RETRIES + 1):
            try:
                self._rate_limiter.wait()
                client = self._build_client()
                response = client.request(
                    method, url,
                    params=params, headers=headers, auth=auth,
                )
                if response.status_code == 429:
                    _backoff_sleep(retry_attempt)
                    continue
                if response.status_code == 200:
                    return response
                return None
            except httpx.HTTPError:
                continue
        return None

    def _build_client(self) -> httpx.Client:
        """Build (or rebuild) the httpx.Client, optionally with a proxy.

        If an ``http_client`` was injected at construction (for testing),
        return it directly instead of building a new one.
        """
        if self._http_client is not None:
            return self._http_client

        kwargs: dict[str, Any] = dict(self._client_kwargs)

        if self._proxies:
            proxy = _next_proxy()
            if proxy:
                kwargs["proxies"] = f"http://{proxy}"

        return httpx.Client(**kwargs)

    def _rotate_proxy(self) -> bool:
        """Advance the round-robin cycle to the next proxy.

        Returns True if proxies are available, False if the pool is empty.
        """
        if not self._proxies:
            return False
        _next_proxy()
        return True

    def _search(
        self, query: str, per_page: int = 5, search_type: str = "release"
    ) -> list[dict[str, Any]]:
        """Search Discogs database. Returns list of raw result dicts."""
        params: dict[str, Any] = {
            "q": query,
            "type": search_type,
            "per_page": per_page,
        }
        if self.token:
            params["token"] = self.token

        cache_key = self._cache_key("GET", "/database/search", params)

        # Check in-memory cache first
        cached = self._cache.get(cache_key)
        if cached is not None:
            return list(cached)

        try:
            response = self._request(
                "GET",
                f"{self.BASE_URL}/database/search",
                params=params,
            )
        except (DiscogsError, httpx.HTTPError) as exc:
            raise DiscogsError(f"Discogs search request failed: {exc}") from exc

        data = response.json()
        results = list(data.get("results", []))

        # Store parsed results in cache
        self._cache.set(cache_key, results)

        return results

    def _get(self, path: str) -> dict[str, Any]:
        """GET a Discogs API endpoint."""
        url = f"{self.BASE_URL}{path}" if not path.startswith("http") else path
        params: dict[str, Any] = {}
        if self.token:
            params["token"] = self.token

        cache_key = self._cache_key("GET", path, params)

        # Check in-memory cache first
        cached = self._cache.get(cache_key)
        if cached is not None:
            return dict(cached)

        try:
            response = self._request("GET", url, params=params or None)
        except (DiscogsError, httpx.HTTPError) as exc:
            raise DiscogsError(f"Discogs request failed: {exc}") from exc

        data = response.json()

        # Store parsed data in cache
        self._cache.set(cache_key, data)

        return data

    def _download_image_bytes(self, url: str) -> tuple[bytes, str] | None:
        """Download image bytes from *url*, using disk cache.

        Returns ``(image_bytes, mime_type)`` on success or ``None`` on
        any HTTP error, non-200 status, or empty response body.
        """
        # Check disk cache first
        cached = self._image_cache.get(url)
        if cached is not None:
            return cached

        try:
            response = self._request(
                "GET",
                url,
            )
        except (DiscogsError, httpx.HTTPError):
            return None

        if response.status_code != 200 or not response.content:
            return None

        mime_type = self._mime_from_url(url) or "image/jpeg"

        # Store in disk cache
        self._image_cache.set(url, response.content, mime_type)

        return response.content, mime_type

    def _search_artist(self, query: str, per_page: int = 5) -> list[dict[str, Any]]:
        """Search Discogs for an artist. Returns raw results."""
        return self._search(query, per_page=per_page, search_type="artist")

    # ── Cache helpers ──────────────────────────────────────────────

    @staticmethod
    def _cache_key(method: str, path: str, params: dict[str, Any] | None = None) -> str:
        """Generate a stable cache key for a Discogs API request."""
        if not params:
            return f"{method}:{path}"
        sorted_items = sorted(
            (k, str(v)) for k, v in params.items() if k != "token"
        )
        param_str = ":" + "&".join(f"{k}={v}" for k, v in sorted_items)
        return f"{method}:{path}{param_str}"

    # ── Data conversion helpers ────────────────────────────────────

    @staticmethod
    def _release_to_candidate(item: dict[str, Any]) -> AlbumCandidate | None:
        """Convert a search result item to an AlbumCandidate."""
        title = item.get("title", "")
        artist, album = DiscogsClient._split_title(title)
        if not album:
            return None

        year = item.get("year")
        genres = item.get("genre", [])
        styles = item.get("style", [])

        artist_list = [artist] if artist else []
        album_artist = artist or None

        return AlbumCandidate(
            artist=artist or None,
            artists=artist_list,
            album=album,
            album_artist=album_artist,
            album_artists=[album_artist] if album_artist else [],
            year=str(year) if year else None,
            genre=", ".join(genres + styles) if (genres or styles) else None,
            musicbrainz_albumid=None,  # Discogs does not provide MBIDs
            musicbrainz_artistid=None,
            source=LookupSource.DISCOGS,
        )

    @staticmethod
    def _full_release_to_candidate(data: dict[str, Any]) -> AlbumCandidate | None:
        """Convert a full release/master response to an AlbumCandidate."""
        title = data.get("title", "")
        artist, album = DiscogsClient._split_title(title)
        if not album:
            return None

        year = data.get("year")
        genres = data.get("genres", [])
        styles = data.get("styles", [])

        artists_data = data.get("artists", [])
        artists = [a.get("name", "") for a in artists_data if a.get("name")]
        album_artist = artists[0] if artists else artist

        tracklist = data.get("tracklist", [])
        tracks = [
            TrackCandidate(
                title=t.get("title"),
                track_number=DiscogsClient._parse_position(t.get("position", "")),
                length=DiscogsClient._parse_duration(t.get("duration", "")),
            )
            for t in tracklist
            if t.get("title")
        ]

        return AlbumCandidate(
            artist=artists[0] if artists else artist or None,
            artists=artists,
            album=album,
            album_artist=album_artist or None,
            album_artists=artists,
            year=str(year) if year else None,
            genre=", ".join(genres + styles) if (genres or styles) else None,
            musicbrainz_albumid=None,
            musicbrainz_artistid=None,
            tracks=tracks,
            source=LookupSource.DISCOGS,
        )

    @staticmethod
    def _split_title(title: str) -> tuple[str | None, str | None]:
        """Split 'Artist - Album' title into artist and album parts."""
        if " - " in title:
            parts = title.split(" - ", 1)
            return parts[0].strip() or None, parts[1].strip() or None
        return None, title.strip() or None

    @staticmethod
    def _parse_position(position: str) -> int | None:
        """Parse a Discogs track position (e.g., 'A1', '1', 'CD-1') to an integer."""
        if not position:
            return None
        # Strip common prefixes: side letters, disc markers
        match = re.search(r"(\d+)$", position)
        if match:
            return int(match.group(1))
        return None

    @staticmethod
    def _parse_duration(duration: str) -> float | None:
        """Parse a Discogs duration string like '3:28' to seconds as float."""
        if not duration:
            return None
        parts = duration.split(":")
        if len(parts) == 2:
            try:
                return int(parts[0]) * 60 + float(parts[1])
            except (ValueError, TypeError):
                return None
        return None

    @staticmethod
    def _mime_from_url(url: str) -> str | None:
        """Guess MIME type from an image URL based on extension."""
        lower = url.lower()
        if ".jpg" in lower or ".jpeg" in lower:
            return "image/jpeg"
        if ".png" in lower:
            return "image/png"
        if ".webp" in lower:
            return "image/webp"
        if ".gif" in lower:
            return "image/gif"
        return None
