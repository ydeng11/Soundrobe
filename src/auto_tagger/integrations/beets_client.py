"""Isolated Beets library integration."""

from __future__ import annotations

import tempfile
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

from auto_tagger.exceptions import TaggingError
from auto_tagger.integrations.candidates import (
    AlbumCandidate,
    LookupRequest,
    LookupSource,
    TrackCandidate,
)


class RateLimiter:
    """Simple interval-based rate limiter."""

    def __init__(
        self,
        interval_seconds: float = 1.0,
        now_func: Callable[[], float] | None = None,
        sleep_func: Callable[[float], None] | None = None,
    ):
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


class BeetsClient:
    """Lookup wrapper that keeps beets details out of app code."""

    def __init__(
        self,
        match_album_func: Callable[[LookupRequest], list[Any]] | None = None,
        match_track_func: Callable[[Path], list[Any]] | None = None,
        max_candidates: int = 5,
        rate_limiter: RateLimiter | None = None,
        library_path: Path | None = None,
    ):
        self.match_album_func = match_album_func or self._match_album_with_beets
        self.match_track_func = match_track_func or self._match_track_with_beets
        self.max_candidates = max_candidates
        self.rate_limiter = rate_limiter or RateLimiter()
        self.library_path = library_path

    def lookup_album(self, request: LookupRequest) -> list[AlbumCandidate]:
        """Return normalized album candidates from beets."""
        try:
            self.rate_limiter.wait()
            proposals = self.match_album_func(request)
        except Exception as exc:
            raise TaggingError(f"Could not query beets for {request.path}: {exc}") from exc

        candidates = [_album_candidate_from_proposal(proposal) for proposal in proposals]
        candidates.sort(key=_candidate_distance)
        return candidates[: self.max_candidates]

    def lookup_track(self, path: Path) -> list[TrackCandidate]:
        """Return normalized track candidates from beets."""
        try:
            self.rate_limiter.wait()
            proposals = self.match_track_func(path)
        except Exception as exc:
            raise TaggingError(f"Could not query beets for {path}: {exc}") from exc

        return [_track_candidate_from_info(_proposal_info(proposal)) for proposal in proposals]

    def configure_beets(self) -> None:
        """Initialize beets config without reading the user's beets config."""
        try:
            from beets import config
        except ImportError as exc:
            raise TaggingError("beets is required for MusicBrainz lookup") from exc

        library_path = self.library_path or Path(tempfile.gettempdir()) / "auto-tagger-beets.db"
        config.clear()
        config.read(user=False)
        config["library"] = str(library_path)
        config["threaded"] = False
        config["import"]["copy"] = False
        config["import"]["move"] = False
        config["import"]["write"] = False

    def _match_album_with_beets(self, request: LookupRequest) -> list[Any]:
        self.configure_beets()
        try:
            from beets.autotag.match import tag_album
        except ImportError as exc:
            raise TaggingError("beets autotag album lookup is unavailable") from exc

        items = self._items_from_album_path(request.path)
        if not items:
            return []  # no audio files to extract hints from
        _, _, proposal = tag_album(
            items,
            search_artist=request.artist_hint,
            search_name=request.album_hint,
        )
        return [proposal] if proposal else []

    @staticmethod
    def _items_from_album_path(path: Path) -> list[Any]:
        """Create beets Item objects from audio files in an album directory."""
        from beets.library import Item

        supported = {".flac", ".mp3", ".m4a", ".mp4", ".ogg", ".wma", ".aiff", ".wav"}
        audio_files = sorted(
            candidate
            for candidate in path.iterdir()
            if candidate.is_file() and candidate.suffix.lower() in supported
        )
        if not audio_files:
            audio_files = sorted(
                candidate
                for candidate in path.rglob("*")
                if candidate.is_file() and candidate.suffix.lower() in supported
            )
        items = []
        for file_path in audio_files:
            try:
                items.append(Item.from_path(str(file_path)))
            except Exception:
                continue
        return items

    def _match_track_with_beets(self, path: Path) -> list[Any]:
        self.configure_beets()
        try:
            from beets.autotag.match import tag_item
            from beets.library import Item
        except ImportError as exc:
            raise TaggingError("beets autotag track lookup is unavailable") from exc

        item = Item.from_path(path)
        proposal = tag_item(item, search_name=path.stem)
        return [proposal] if proposal else []


def _album_candidate_from_proposal(proposal: Any) -> AlbumCandidate:
    info = _proposal_info(proposal)
    tracks = [_track_candidate_from_info(track) for track in _attr(info, "tracks", []) or []]
    artist = _text_attr(info, "artist")
    album_artist = _text_attr(info, "albumartist") or artist
    return AlbumCandidate(
        artist=artist,
        artists=_list_attr(info, "artists"),
        album=_text_attr(info, "album"),
        album_artist=album_artist,
        album_artists=_list_attr(info, "albumartists") or ([album_artist] if album_artist else []),
        year=_text_attr(info, "year"),
        genre=_text_attr(info, "genre"),
        musicbrainz_albumid=_text_attr(info, "album_id"),
        musicbrainz_artistid=_text_attr(info, "artist_id"),
        tracks=tracks,
        distance=_float_attr(proposal, "distance"),
        source=LookupSource.BEETS,
    )


def _candidate_distance(candidate: AlbumCandidate) -> float:
    return candidate.distance if candidate.distance is not None else 1.0


def _track_candidate_from_info(info: Any) -> TrackCandidate:
    return TrackCandidate(
        title=_text_attr(info, "title"),
        artist=_text_attr(info, "artist"),
        artists=_list_attr(info, "artists"),
        track_number=_int_attr(info, "track"),
        track_total=_int_attr(info, "tracktotal"),
        disc_number=_int_attr(info, "disc"),
        disc_total=_int_attr(info, "disctotal"),
        musicbrainz_trackid=_text_attr(info, "track_id"),
        length=_float_attr(info, "length"),
    )


def _proposal_info(proposal: Any) -> Any:
    return _attr(proposal, "info", proposal)


def _attr(obj: Any, name: str, default: Any = None) -> Any:
    if isinstance(obj, dict):
        return obj.get(name, default)
    return getattr(obj, name, default)


def _text_attr(obj: Any, name: str) -> str | None:
    value = _attr(obj, name)
    return None if value is None else str(value)


def _list_attr(obj: Any, name: str) -> list[str]:
    value = _attr(obj, name)
    if value is None:
        return []
    if isinstance(value, str):
        return [value]
    return [str(item) for item in value if item]


def _int_attr(obj: Any, name: str) -> int | None:
    value = _attr(obj, name)
    if value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _float_attr(obj: Any, name: str) -> float | None:
    value = _attr(obj, name)
    if value is None:
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None
