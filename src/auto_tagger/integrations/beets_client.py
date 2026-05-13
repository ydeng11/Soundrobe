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
            # Fall back to direct musicbrainzngs text search
            return self._match_album_via_musicbrainzngs(request)
        _, _, proposal = tag_album(
            items,
            search_artist=request.artist_hint,
            search_name=request.album_hint,
        )
        if proposal and getattr(proposal, "info", None) is not None:
            return [proposal]
        # beets returned empty proposal — try direct musicbrainzngs search
        return self._match_album_via_musicbrainzngs(request)

    @staticmethod
    def _match_album_via_musicbrainzngs(request: LookupRequest) -> list[Any]:
        """Search MusicBrainz directly when beets has no audio items.

        Returns proposals with full track listings by fetching release details
        for the top search result.
        """
        try:
            import musicbrainzngs as mb
        except ImportError:
            return []

        try:
            mb.set_useragent("auto-tagger", "0.1.0", "https://github.com/auto-tagger/auto-tagger")
        except Exception:
            pass  # already set

        try:
            result = mb.search_releases(
                artist=request.artist_hint or "",
                release=request.album_hint or "",
                limit=5,
            )
        except Exception:
            return []

        proposals = []
        for release in result.get("release-list", []):
            # Try to fetch full release with tracks and genre
            release_id = release.get("id")
            genre_str = _get_release_genre(release)
            tracks_data, release_genre = _fetch_release_tracks(release_id)

            if release_genre and not genre_str:
                genre_str = release_genre

            info: dict[str, Any] = {
                "album": release.get("title"),
                "album_id": release_id,
                "artist": _artist_name_from_release(release),
                "artists": _artist_list_from_release(release),
                "year": release.get("date", "")[:4] if release.get("date") else None,
                "genre": genre_str,
                "tracks": tracks_data,
            }
            proposals.append(_MusicBrainzProposal(info))
        return proposals

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


class _MusicBrainzProposal:
    """Minimal adapter so musicbrainzngs results work with _album_candidate_from_proposal."""

    def __init__(self, info: dict[str, Any]) -> None:
        self.info: dict[str, Any] = info
        self.distance: float | None = None


def _artist_name_from_release(release: dict[str, Any]) -> str | None:
    """Extract the primary artist name from a MusicBrainz release dict."""
    ac = release.get("artist-credit", [])
    if ac:
        return ac[0].get("artist", {}).get("name")
    return None


def _artist_list_from_release(release: dict[str, Any]) -> list[str]:
    """Extract all artist names from a MusicBrainz release dict."""
    names: list[str] = []
    for item in release.get("artist-credit", []):
        name = item.get("artist", {}).get("name")
        if name:
            names.append(name)
    return names


def _fetch_release_tracks(release_id: str | None) -> tuple[list[dict[str, Any]], str | None]:
    """Fetch full track listing and genre for a MusicBrainz release by ID.

    Returns (tracks, genre_string).
    """
    if not release_id:
        return [], None
    try:
        import musicbrainzngs as mb

        result = mb.get_release_by_id(release_id, includes=["recordings", "tags"])
        release = result.get("release", {})
    except Exception:
        return [], None

    # Extract genre from release tag-list
    genre = _extract_genre_from_tags(release.get("tag-list", []))

    tracks: list[dict[str, Any]] = []
    track_num = 0
    for medium in release.get("medium-list", []):
        disc_num = int(medium.get("position", 1) or 1)
        for track in medium.get("track-list", []):
            track_num += 1
            recording = track.get("recording", {})
            length_ms = track.get("length")
            length_s = int(length_ms) / 1000.0 if length_ms else None
            tracks.append({
                "title": recording.get("title"),
                "artist": _artist_name_from_recording(recording),
                "artists": _artist_list_from_recording(recording),
                "track_number": int(track.get("position", track_num) or track_num),
                "track_total": len(medium.get("track-list", [])),
                "disc_number": disc_num,
                "disc_total": len(release.get("medium-list", [])),
                "musicbrainz_trackid": recording.get("id"),
                "length": length_s,
            })
    return tracks, genre


def _get_release_genre(release: dict[str, Any]) -> str | None:
    """Extract genre from a MusicBrainz search result release dict.

    Search results may have a 'tag-list' with user tags.
    """
    return _extract_genre_from_tags(release.get("tag-list", []))


def _extract_genre_from_tags(tag_list: list[dict[str, Any]]) -> str | None:
    """Extract genre-like tags from MusicBrainz tag-list.

    Filters out non-genre tags (like 'seen live', 'favorite') and
    returns the most popular genre tag. Falls back to 'unknown' if
    no genre tag found with sufficient count.
    """
    non_genre = {"seen live", "favourite", "favorite", "owned", "bootleg",
                 "live", "compilation", "greatest hits", "favourites",
                 "want", "have", "in my collection"}

    tags_with_count = []
    for tag in tag_list:
        name = tag.get("name", "").strip().lower() if isinstance(tag, dict) else str(tag).strip().lower()
        count = int(tag.get("count", 1)) if isinstance(tag, dict) else 1
        if name and name not in non_genre:
            tags_with_count.append((name, count))

    # Sort by count descending, return the most popular
    if tags_with_count:
        tags_with_count.sort(key=lambda x: x[1], reverse=True)
        return tags_with_count[0][0]
    return None


def _artist_name_from_recording(recording: dict[str, Any]) -> str | None:
    """Extract the primary artist name from a recording dict."""
    ac = recording.get("artist-credit", [])
    if ac:
        return ac[0].get("artist", {}).get("name")
    return None


def _artist_list_from_recording(recording: dict[str, Any]) -> list[str]:
    """Extract all artist names from a recording dict."""
    names: list[str] = []
    for item in recording.get("artist-credit", []):
        name = item.get("artist", {}).get("name")
        if name:
            names.append(name)
    return names
