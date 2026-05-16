"""Discogs API integration for album metadata lookup.

Discogs provides a free, unauthenticated API with:
- Database search (release, master, artist)
- Release details (full tracklist, artists, year, genres)
- Master release (consolidated tracklist across versions)
- 19M+ releases vs. MusicBrainz's ~3M

Rate limit: 25 req/min unauthenticated, 60 req/min with token.
"""

from __future__ import annotations

from typing import Any

import httpx

from auto_tagger.features.cover_art import CoverArtImage, CoverArtResult, CoverArtStatus
from auto_tagger.integrations.aliases import get_all_name_variants, get_aliases
from auto_tagger.integrations.candidates import (
    AlbumCandidate,
    LookupSource,
    TrackCandidate,
)


class DiscogsError(Exception):
    """Raised when the Discogs API returns an error."""


class DiscogsClient:
    """Thin wrapper around the free Discogs API."""

    BASE_URL = "https://api.discogs.com"

    def __init__(
        self,
        token: str | None = None,
        user_agent: str = "auto-tagger/0.1.0",
        max_candidates: int = 3,
        timeout_seconds: int = 20,
    ):
        self.token = token
        self.user_agent = user_agent
        self.max_candidates = max_candidates
        self.timeout_seconds = timeout_seconds

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

    # ── internal ─────────────────────────────────────────────

    def _search(self, query: str, per_page: int = 5, search_type: str = "release") -> list[dict[str, Any]]:
        """Search Discogs database. Returns list of raw result dicts."""
        params: dict[str, Any] = {
            "q": query,
            "type": search_type,
            "per_page": per_page,
        }
        if self.token:
            params["token"] = self.token

        headers = {"User-Agent": self.user_agent}
        try:
            response = httpx.get(
                f"{self.BASE_URL}/database/search",
                params=params,
                headers=headers,
                timeout=self.timeout_seconds,
            )
        except httpx.HTTPError as exc:
            raise DiscogsError(f"Discogs search request failed: {exc}") from exc

        if response.status_code == 429:
            raise DiscogsError("Discogs rate limit exceeded. Wait before retrying.")
        if response.status_code != 200:
            raise DiscogsError(
                f"Discogs search returned HTTP {response.status_code}: {response.text[:200]}"
            )

        data = response.json()
        return list(data.get("results", []))

    def _get(self, path: str) -> dict[str, Any]:
        """GET a Discogs API endpoint."""
        headers = {"User-Agent": self.user_agent}
        if self.token:
            path = f"{path}?token={self.token}" if "?" not in path else f"{path}&token={self.token}"

        try:
            response = httpx.get(
                f"{self.BASE_URL}{path}" if not path.startswith("http") else path,
                headers=headers,
                timeout=self.timeout_seconds,
            )
        except httpx.HTTPError as exc:
            raise DiscogsError(f"Discogs request failed: {exc}") from exc

        if response.status_code == 429:
            raise DiscogsError("Discogs rate limit exceeded. Wait before retrying.")
        if response.status_code != 200:
            raise DiscogsError(
                f"Discogs endpoint returned HTTP {response.status_code}"
            )

        return response.json()

    def _release_to_candidate(self, item: dict[str, Any]) -> AlbumCandidate | None:
        """Convert a search result item to an AlbumCandidate."""
        title = item.get("title", "")
        artist, album = self._split_title(title)
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

    def _full_release_to_candidate(self, data: dict[str, Any]) -> AlbumCandidate | None:
        """Convert a full release/master response to an AlbumCandidate."""
        title = data.get("title", "")
        artist, album = self._split_title(title)
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
                track_number=self._parse_position(t.get("position", "")),
                length=self._parse_duration(t.get("duration", "")),
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
        import re
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

    def _download_image_bytes(self, url: str) -> tuple[bytes, str] | None:
        """Download image bytes from *url*.

        Returns ``(image_bytes, mime_type)`` on success or ``None`` on
        any HTTP error, non-200 status, or empty response body.
        """
        try:
            response = httpx.get(
                url, timeout=self.timeout_seconds, follow_redirects=True
            )
        except httpx.HTTPError:
            return None
        if response.status_code != 200 or not response.content:
            return None
        mime_type = self._mime_from_url(url) or "image/jpeg"
        return response.content, mime_type

    # ── artist search & image fetch ─────────────────────────────────

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

    def _search_artist(self, query: str, per_page: int = 5) -> list[dict[str, Any]]:
        """Search Discogs for an artist. Returns raw results."""
        return self._search(query, per_page=per_page, search_type="artist")

    def fetch_cover_art(self, artist: str, album: str) -> CoverArtResult:
        """Fetch cover art for an album from Discogs.

        Tries in order:
          1. Each name variant (English aliases, TC, SC, etc.) + album
          2. Album-only (catches cross-script mismatches)

        For each query, checks the search result's cover_image field first.
        If missing, fetches the full release detail for images.

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
