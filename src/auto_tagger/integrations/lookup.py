"""Lookup orchestration across cache, Beets, and folder fallback."""

from __future__ import annotations

import json
import logging
import re
from dataclasses import replace
from functools import cache
from pathlib import Path

import opencc

from auto_tagger.config import Settings
from auto_tagger.exceptions import TaggingError

logger = logging.getLogger(__name__)
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


@cache
def _get_converters() -> tuple[opencc.OpenCC, opencc.OpenCC]:
    """Lazy-init OpenCC converters (singleton via functools.cache)."""
    return opencc.OpenCC("t2s"), opencc.OpenCC("s2t")


def _sc_variant(text: str) -> str:
    """Convert text to Simplified Chinese."""
    return _get_converters()[0].convert(text)


def _tc_variant(text: str) -> str:
    """Convert text to Traditional Chinese."""
    return _get_converters()[1].convert(text)


class LookupService:
    """Coordinate candidate lookup with caching and fallback."""

    def __init__(
        self,
        beets_client: BeetsClient | None = None,
        cache: MatchCache | None = None,
        settings: Settings | None = None,
    ):
        self.settings = settings or Settings()
        self.beets_client = beets_client if beets_client is not None else BeetsClient()
        self.cache = cache if cache is not None else MatchCache(self.settings.cache_path)
        self.warnings: list[str] = []

    def request_from_path(self, path: Path) -> LookupRequest:
        """Build a lookup request using both folder names and existing file tags."""
        return parse_album_with_tags(path)

    def lookup_album(self, path: Path) -> list[AlbumCandidate]:
        """Return album candidates from cache, Beets, or folder fallback."""
        request = self.request_from_path(path)

        if self.settings.debug:
            logger.debug(
                "Lookup request from %s: artist=%s album=%s year=%s tracks=%d",
                path.name, request.artist_hint or "?",
                request.album_hint or "?", request.year_hint or "?",
                len(request.tracks) if request.tracks else 0,
            )

        # Try LLM hint enhancement when deterministic parsing is ambiguous
        if self._hints_are_ambiguous(request):
            enhanced = self._enhance_hints_with_llm(request)
            if enhanced is not None:
                if self.settings.debug:
                    logger.debug(
                        "LLM enhanced hints: artist=%s album=%s year=%s",
                        enhanced.artist_hint or "?",
                        enhanced.album_hint or "?",
                        enhanced.year_hint or "?",
                    )
                request = enhanced

        cached = self.cache.get(request)
        if cached is not None:
            if self.settings.debug:
                logger.debug(
                    "Cache hit for %s — %d candidate(s)",
                    path.name, len(cached),
                )
            return cached

        if self.settings.debug:
            logger.debug("Cache miss — running lookups")

        candidates = self._lookup_dataset(request)
        if self.settings.debug:
            logger.debug("Dataset candidates: %d", len(candidates))

        if not candidates and self.settings.remote_lookup_enabled:
            candidates = self._lookup_beets(request)
            if self.settings.debug:
                logger.debug("Beets/MusicBrainz candidates: %d", len(candidates))

        # Always try Discogs — merge with existing results
        if self.settings.discogs_enabled:
            discogs_candidates = self._lookup_discogs(request)
            if self.settings.debug:
                logger.debug("Discogs candidates: %d", len(discogs_candidates))
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
            if self.settings.debug:
                logger.debug(
                    "No matching candidates — added folder fallback: %s — %s",
                    folder.artist or "?", folder.album or "?",
                )

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
        return replace(candidate, verification=status)

    def _record_warning(self, warning: str) -> None:
        if warning not in self.warnings:
            self.warnings.append(warning)

    # ── LLM hint enhancement ────────────────────────────────────────

    def _hints_are_ambiguous(self, request: LookupRequest) -> bool:
        """Check if deterministic parsing produced ambiguous hints.

        Returns True when the folder name contains patterns that suggest
        the raw metadata was not fully extracted by deterministic parsing:
        - Bracket annotations: ``[香港首版]``, ``[FLAC]``, ``[引进版]``
        - Chinese bookmark separators: ``《》「」【】``
        - Chinese dots between CJK characters (multi-artist or Year.Artist.Album)
        - ASCII dots in CJK contexts
        - No album/artist hint at all

        Checks the original folder name (not the cleaned hint) so that
        even after deterministic ``clean_folder_name()`` processes the
        name, the LLM still has a chance to extract from the raw form.
        """
        album_hint = request.album_hint or ""
        artist_hint = request.artist_hint or ""

        # If we couldn't get any useful hint, definitely ambiguous
        if not album_hint or not artist_hint:
            return True

        # Check the original folder name for metadata annotation markers
        # that deterministic cleanup may not have fully resolved.
        folder_name = request.path.name if request.path else ""
        if re.search(r"[\[\]《》「」【】]", folder_name):
            return True

        # Chinese dot between CJK characters → likely multi-artist/album
        # separator that deterministic parsing doesn't handle
        if re.search(r"[\u4e00-\u9fff]\.[\u4e00-\u9fff]", folder_name):
            return True
        if "。" in folder_name:
            return True

        # Year prefix without a clean album: "1997-大件事[香港首版]" still has
        # annotations that the LLM can extract better than regex
        # (only flag when deterministic cleanup clearly left artifacts)
        if request.year_hint and album_hint:
            # If the cleaned album hint looks like it has concatenated
            # metadata (no natural word boundaries), try LLM
            if len(album_hint) > 3 and album_hint.isalnum() and album_hint.isascii():
                clean = album_hint.replace(" ", "").replace("-", "")
                if clean.isalnum() and len(clean) > 10:
                    return True
            return False

        # CD subdirectory without year in album name → year likely in parent
        if not request.year_hint and not album_hint:
            return True

        # Dot convention: Year.Artist.Album with ASCII dots
        if "." in album_hint and not request.year_hint:
            return True
        if "." in artist_hint and any(ord(c) > 127 for c in artist_hint):
            return True

        return False

    def _enhance_hints_with_llm(
        self,
        request: LookupRequest,
    ) -> LookupRequest | None:
        """Try to enhance ambiguous hints using cached or fresh LLM extraction.

        Checks the album state cache first (regardless of LLM availability),
        and only calls the LLM if no cached extraction exists.

        Returns an updated LookupRequest with clean hints, or None if
        no enhancement was possible.
        """
        # Determine the album folder name to use as the cache/call key
        album_path = request.path.parent if request.path.is_file() else request.path
        folder_name = album_path.name if album_path.name else ""
        parent_name = album_path.parent.name if album_path.parent else None

        if not folder_name:
            return None

        if self.settings.debug:
            logger.debug(
                "LLM hint enhancement: folder=%s parent=%s ambiguous=True",
                folder_name, parent_name or "-",
            )

        # Check cache first (works even without LLM key)
        cached = self.cache.get_llm_extraction(folder_name)
        if cached is not None:
            if self.settings.debug:
                logger.debug(
                    "Cache hit for LLM extraction: folder=%s → %s",
                    folder_name, cached,
                )
            return self._apply_extraction(request, cached)

        # Also try with parent name as the key (CD subdirs share parent)
        if parent_name and parent_name != folder_name:
            cached = self.cache.get_llm_extraction(parent_name)
            if cached is not None:
                if self.settings.debug:
                    logger.debug(
                        "Cache hit for LLM extraction: parent=%s → %s",
                        parent_name, cached,
                    )
                return self._apply_extraction(request, cached)

        # No cache hit → need to call LLM
        if not self.settings.llm_api_key:
            if self.settings.debug:
                logger.debug(
                    "No cached LLM extraction and no API key — skipping hint enhancement"
                )
            return None

        try:
            from auto_tagger.llm.client import OpenRouterClient
            from auto_tagger.llm.prompts import build_folder_extraction_messages
            from auto_tagger.llm.schemas import FolderExtractionResponse

            client = OpenRouterClient(self.settings)
            messages = build_folder_extraction_messages(folder_name, parent_name)

            response = client.complete_json(
                messages=messages,
                schema=FolderExtractionResponse,
            )

            extraction = response.data
            if not isinstance(extraction, dict):
                return None

            # Normalize keys
            normalized = {
                "artist": extraction.get("artist") or None,
                "album": extraction.get("album") or None,
                "year": str(extraction["year"]) if extraction.get("year") else None,
                "disc": str(extraction["disc"]) if extraction.get("disc") else None,
            }

            if self.settings.debug:
                logger.debug(
                    "LLM extracted: folder=%s → artist=%s album=%s year=%s disc=%s",
                    folder_name,
                    normalized["artist"] or "?",
                    normalized["album"] or "?",
                    normalized["year"] or "?",
                    normalized["disc"] or "?",
                )

            # Cache the result under the folder name
            self.cache.set_llm_extraction(folder_name, normalized)
            # Also cache under parent name so CD subdirs can share
            if parent_name and parent_name != folder_name:
                self.cache.set_llm_extraction(parent_name, normalized)

            return self._apply_extraction(request, normalized)

        except Exception:
            if self.settings.verbose or self.settings.debug:
                self._record_warning(f"LLM folder extraction failed for '{folder_name}'")
            return None

    @staticmethod
    def _apply_extraction(
        request: LookupRequest,
        extraction: dict[str, str | None],
    ) -> LookupRequest:
        """Apply LLM extraction results to a lookup request."""
        artist = extraction.get("artist") or request.artist_hint
        album = extraction.get("album") or request.album_hint
        year = extraction.get("year") or request.year_hint

        return LookupRequest(
            path=request.path,
            artist_hint=artist,
            album_hint=album,
            year_hint=year,
            tracks=request.tracks,
        )
