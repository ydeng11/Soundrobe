"""Lookup orchestration across cache, Beets, and folder fallback."""

from __future__ import annotations

from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.exceptions import TaggingError
from auto_tagger.integrations.beets_client import BeetsClient
from auto_tagger.integrations.cache import MatchCache
from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest
from auto_tagger.integrations.fallback import candidate_from_folder, parse_album_path


class LookupService:
    """Coordinate candidate lookup with caching and fallback."""

    def __init__(
        self,
        beets_client: BeetsClient | None = None,
        cache: MatchCache | None = None,
        settings: Settings | None = None,
    ):
        settings = settings or Settings()
        self.beets_client = beets_client if beets_client is not None else BeetsClient()
        self.cache = cache if cache is not None else MatchCache(settings.cache_path)

    def request_from_path(self, path: Path) -> LookupRequest:
        """Build a lookup request from a file or album path."""
        return parse_album_path(path)

    def lookup_album(self, path: Path) -> list[AlbumCandidate]:
        """Return album candidates from cache, Beets, or folder fallback."""
        request = self.request_from_path(path)
        cached = self.cache.get(request)
        if cached is not None:
            return cached

        candidates = self._lookup_beets(request)
        if not candidates:
            candidates = [candidate_from_folder(request)]

        self.cache.set(request, candidates)
        return candidates

    def _lookup_beets(self, request: LookupRequest) -> list[AlbumCandidate]:
        if self.beets_client is None:
            return []
        try:
            return self.beets_client.lookup_album(request)
        except TaggingError:
            return []
