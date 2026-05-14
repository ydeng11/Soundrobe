"""Lookup orchestration across cache, Beets, and folder fallback."""

from __future__ import annotations

from pathlib import Path

import opencc

from auto_tagger.config import Settings
from auto_tagger.exceptions import TaggingError
from auto_tagger.integrations.aliases import get_aliases
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


# ── SC/TC Chinese variant helpers ────────────────────────────────────


_t2s_converter: opencc.OpenCC | None = None
_s2t_converter: opencc.OpenCC | None = None


def _get_converters() -> tuple[opencc.OpenCC, opencc.OpenCC]:
    """Lazy-init OpenCC converters (singleton)."""
    global _t2s_converter, _s2t_converter
    if _t2s_converter is None:
        _t2s_converter = opencc.OpenCC("t2s")
        _s2t_converter = opencc.OpenCC("s2t")
    return _t2s_converter, _s2t_converter


def _sc_variant(text: str) -> str:
    """Convert text to Simplified Chinese (if it contains Chinese)."""
    t2s, _ = _get_converters()
    return t2s.convert(text)


def _tc_variant(text: str) -> str:
    """Convert text to Traditional Chinese (if it contains Chinese)."""
    _, s2t = _get_converters()
    return s2t.convert(text)


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
        seen: set[tuple[str | None, str | None]] = set()
        results: list[AlbumCandidate] = []

        def _try_search(
            artist_hint: str | None, album_hint: str | None
        ) -> list[AlbumCandidate]:
            key = (artist_hint, album_hint)
            if key in seen:
                return []
            seen.add(key)
            try:
                variant_request = LookupRequest(
                    path=request.path,
                    artist_hint=artist_hint,
                    album_hint=album_hint,
                    year_hint=request.year_hint,
                    tracks=request.tracks,
                )
                return self.beets_client.lookup_album(variant_request)
            except TaggingError:
                return []

        # 1. Try original + SC/TC variants
        variants = self._hint_variants(request.artist_hint, request.album_hint)
        for artist_hint, album_hint in variants:
            candidates = _try_search(artist_hint, album_hint)
            results.extend(candidates)
            if candidates:
                return results

        # 2. Try known aliases of the artist
        for alias in get_aliases(request.artist_hint):
            candidates = _try_search(alias, request.album_hint)
            results.extend(candidates)
            if candidates:
                return results

        # 3. Album-only fallback (no artist constraint)
        candidates = _try_search(None, request.album_hint)
        results.extend(candidates)
        return results

    def _lookup_discogs(self, request: LookupRequest) -> list[AlbumCandidate]:
        """Search Discogs for album candidates.

        Tries in order:
          1. Original artist + album + SC/TC variants
          2. Known aliases of the artist hint
          3. Album-only search (catch cross-script mismatches like 蔡健雅 vs Tanya Chua)
        """
        if not request.artist_hint or not request.album_hint:
            return []

        client = DiscogsClient(
            token=self.settings.discogs_token,
            max_candidates=self.settings.discogs_max_candidates,
            timeout_seconds=self.settings.discogs_timeout_seconds,
        )
        seen: set[tuple[str | None, str | None]] = set()
        results: list[AlbumCandidate] = []

        # 1. Try original + SC/TC variants
        variants = self._hint_variants(request.artist_hint, request.album_hint)
        for artist_hint, album_hint in variants:
            key = (artist_hint, album_hint)
            if key in seen:
                continue
            seen.add(key)
            try:
                candidates = client.search_album(
                    artist_hint or "", album_hint or ""
                )
                results.extend(candidates)
            except DiscogsError as exc:
                self._record_warning(f"Discogs lookup failed: {exc}")
                continue
            if candidates:
                return results

        # 2. Try known aliases of the artist (e.g. 蔡健雅 → tanya chua)
        for alias in get_aliases(request.artist_hint):
            key = (alias, request.album_hint)
            if key in seen:
                continue
            seen.add(key)
            try:
                candidates = client.search_album(alias, request.album_hint)
                results.extend(candidates)
            except DiscogsError:
                continue
            if candidates:
                return results

        # 3. Album-only fallback — catches cases where the artist name
        #    is in a completely different script (Chinese vs Latin).
        #    The LLM selection step will filter out wrong artists.
        try:
            candidates = client.search_album("", request.album_hint)
            results.extend(candidates)
        except DiscogsError:
            pass
        return results

    @staticmethod
    def _hint_variants(
        artist: str | None, album: str | None
    ) -> list[tuple[str | None, str | None]]:
        """Yield (artist, album) pairs trying original and SC/TC variants.

        Returns: [(original, original), (sc, sc), (tc, tc)] with dupes removed.
        """
        pairs: list[tuple[str | None, str | None]] = [(artist, album)]
        if artist:
            sc_a, tc_a = _sc_variant(artist), _tc_variant(artist)
        else:
            sc_a = tc_a = None
        if album:
            sc_b, tc_b = _sc_variant(album), _tc_variant(album)
        else:
            sc_b = tc_b = None
        for a, b in [(sc_a, sc_b), (tc_a, tc_b)]:
            if (a, b) not in pairs:
                pairs.append((a, b))
        return pairs

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
