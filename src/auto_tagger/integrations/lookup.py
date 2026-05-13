"""Lookup orchestration across cache, Beets, and folder fallback."""

from __future__ import annotations

from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.exceptions import TaggingError
from auto_tagger.integrations.beets_client import BeetsClient
from auto_tagger.integrations.cache import MatchCache
from auto_tagger.integrations.candidates import (
    AlbumCandidate,
    LookupRequest,
    verify_album_name,
)
from auto_tagger.integrations.dataset_raw import query_album as raw_query_album
from auto_tagger.integrations.discogs_client import DiscogsClient, DiscogsError
from auto_tagger.integrations.fallback import (
    candidate_from_folder,
    parse_album_with_tags,
)


class LookupService:
    """Coordinate candidate lookup with caching and fallback."""

    def __init__(
        self,
        beets_client: BeetsClient | None = None,
        dataset_client: DatasetIndexClient | None = None,
        cache: MatchCache | None = None,
        settings: Settings | None = None,
    ):
        self.settings = settings or Settings()
        self.beets_client = beets_client if beets_client is not None else BeetsClient()
        self.dataset_client = dataset_client  # kept for backward compat
        self.cache = cache if cache is not None else MatchCache(self.settings.cache_path)
        self.warnings: list[str] = []

    def request_from_path(self, path: Path) -> LookupRequest:
        """Build a lookup request using both folder names and existing file tags."""
        return parse_album_with_tags(path)

    def lookup_album(self, path: Path) -> list[AlbumCandidate]:
        """Return album candidates from cache, Beets, or folder fallback."""
        request = self.request_from_path(path)
        cached = self.cache.get(request)
        if cached is not None:
            return cached

        candidates = self._lookup_dataset(request)
        if not candidates and self.settings.remote_lookup_enabled:
            candidates = self._lookup_beets(request)
        # Always try Discogs — merge with existing results
        if self.settings.discogs_enabled:
            discogs_candidates = self._lookup_discogs(request)
            if discogs_candidates:
                candidates = list(candidates) + discogs_candidates

        # Annotate verification status for all candidates
        candidates = [
            self._annotate_verification(request, candidate)
            for candidate in candidates
        ]

        # If no candidates at all, or all database candidates are mismatches,
        # always include the folder fallback as a trusted option.
        if not candidates or all(
            c.verification == "mismatch" for c in candidates
        ):
            folder = self._annotate_verification(
                request, candidate_from_folder(request)
            )
            # Don't duplicate if the folder candidate is already present
            candidates = list(candidates) + [folder]

        self.cache.set(request, candidates)
        return candidates

    def _lookup_dataset(self, request: LookupRequest) -> list[AlbumCandidate]:
        if not self.settings.dataset_lookup_enabled:
            return []
        if not self.settings.dataset_index_path.exists():
            if self.settings.dataset_warn_when_unavailable:
                self._record_warning(
                    f"Local dataset index not found at {self.settings.dataset_index_path}"
                )
            return []
        try:
            return raw_query_album(
                self.settings.dataset_index_path,
                request.artist_hint,
                request.album_hint,
                max_candidates=self.settings.dataset_max_candidates,
                services=self.settings.dataset_services,
            )
        except Exception as exc:
            if self.settings.dataset_warn_when_unavailable:
                self._record_warning(f"Local dataset query failed: {exc}")
            return []

    def _lookup_beets(self, request: LookupRequest) -> list[AlbumCandidate]:
        if self.beets_client is None:
            return []
        try:
            return self.beets_client.lookup_album(request)
        except TaggingError:
            return []

    def _lookup_discogs(self, request: LookupRequest) -> list[AlbumCandidate]:
        """Search Discogs for album candidates."""
        if not request.artist_hint or not request.album_hint:
            return []
        try:
            client = DiscogsClient(
                token=self.settings.discogs_token,
                max_candidates=self.settings.discogs_max_candidates,
                timeout_seconds=self.settings.discogs_timeout_seconds,
            )
            return client.search_album(request.artist_hint, request.album_hint)
        except DiscogsError as exc:
            self._record_warning(f"Discogs lookup failed: {exc}")
            return []

    @staticmethod
    def _annotate_verification(
        request: LookupRequest, candidate: AlbumCandidate
    ) -> AlbumCandidate:
        """Add verification status to a candidate."""
        status = verify_album_name(request.album_hint, candidate)
        return AlbumCandidate(
            artist=candidate.artist,
            artists=candidate.artists,
            album=candidate.album,
            album_artist=candidate.album_artist,
            album_artists=candidate.album_artists,
            year=candidate.year,
            genre=candidate.genre,
            musicbrainz_albumid=candidate.musicbrainz_albumid,
            musicbrainz_artistid=candidate.musicbrainz_artistid,
            tracks=candidate.tracks,
            distance=candidate.distance,
            source=candidate.source,
            verification=status,
        )

    def _record_warning(self, warning: str) -> None:
        if warning not in self.warnings:
            self.warnings.append(warning)
