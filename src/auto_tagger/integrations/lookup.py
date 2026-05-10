"""Lookup orchestration across cache, Beets, and folder fallback."""

from __future__ import annotations

from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.exceptions import TaggingError
from auto_tagger.integrations.beets_client import BeetsClient
from auto_tagger.integrations.cache import MatchCache
from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest
from auto_tagger.integrations.dataset import DatasetIndexClient
from auto_tagger.integrations.fallback import candidate_from_folder, parse_album_path


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
        self.dataset_client = (
            dataset_client
            if dataset_client is not None
            else DatasetIndexClient(
                self.settings.dataset_index_path,
                max_candidates=self.settings.dataset_max_candidates,
            )
        )
        self.cache = cache if cache is not None else MatchCache(self.settings.cache_path)
        self.warnings: list[str] = []

    def request_from_path(self, path: Path) -> LookupRequest:
        """Build a lookup request from a file or album path."""
        return parse_album_path(path)

    def lookup_album(self, path: Path) -> list[AlbumCandidate]:
        """Return album candidates from cache, Beets, or folder fallback."""
        request = self.request_from_path(path)
        cached = self.cache.get(request)
        if cached is not None:
            return cached

        candidates = self._lookup_dataset(request)
        if not candidates and self.settings.remote_lookup_enabled:
            candidates = self._lookup_beets(request)
        if not candidates:
            candidates = [candidate_from_folder(request)]

        self.cache.set(request, candidates)
        return candidates

    def _lookup_dataset(self, request: LookupRequest) -> list[AlbumCandidate]:
        if not self.settings.dataset_lookup_enabled or self.dataset_client is None:
            return []
        candidates = self.dataset_client.lookup_album(request)
        warning = getattr(self.dataset_client, "last_warning", None)
        if warning and self.settings.dataset_warn_when_unavailable:
            self._record_warning(warning)
        return candidates

    def _lookup_beets(self, request: LookupRequest) -> list[AlbumCandidate]:
        if self.beets_client is None:
            return []
        try:
            return self.beets_client.lookup_album(request)
        except TaggingError:
            return []

    def _record_warning(self, warning: str) -> None:
        if warning not in self.warnings:
            self.warnings.append(warning)
