"""Folder-structure fallback lookup helpers."""

from __future__ import annotations

import re
from pathlib import Path

from auto_tagger.core import iter_audio_files, read_metadata
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.exceptions import AutoTaggerError
from auto_tagger.integrations.candidates import (
    AlbumCandidate,
    LookupRequest,
    LookupSource,
    TrackCandidate,
)

COMPILATION_HINTS = {"various artists", "va", "soundtrack", "ost"}

# Common patterns in folder names that aren't part of the album name
_DATE_PREFIX_RE = re.compile(r"^\d{4}[-.]\d{2}\s*")  # "2003-04" or "2005.08"
_BOOKMARKS_RE = re.compile(r"[《》「」【】\[\]]")  # Chinese/Western bookmarks
_EXTRA_SUFFIX_RE = re.compile(r"\s*\([^)]*\)\s*$")  # trailing "(FLAC分轨)" etc.


def clean_folder_name(name: str) -> str:
    """Clean a folder name for use as a lookup hint.

    Strips common metadata prefixes/suffixes that aren't part of the
    actual album or artist name:
      - Date prefixes: "2003-04《挚爱》" → "挚爱"
      - Bookmarks: "《挚爱》" → "挚爱"
      - Extra suffixes: "Hello (Bonus)" → "Hello"

    Returns the cleaned name, or the original if nothing to strip.
    """
    cleaned = _DATE_PREFIX_RE.sub("", name)
    cleaned = _BOOKMARKS_RE.sub("", cleaned)
    cleaned = _EXTRA_SUFFIX_RE.sub("", cleaned)
    cleaned = cleaned.strip()
    return cleaned or name


def parse_album_path(path: Path) -> LookupRequest:
    """Parse artist and album hints from an Artist/Album path.

    Folder names are cleaned of date prefixes, bookmarks, and extra
    suffixes before being used as lookup hints.
    """
    album_path = path.parent if path.is_file() else path
    album_hint = clean_folder_name(album_path.name) if album_path.name else None
    artist_hint = clean_folder_name(album_path.parent.name) if album_path.parent.name else None
    return LookupRequest(path=path, artist_hint=artist_hint, album_hint=album_hint)


def candidate_from_folder(request: LookupRequest) -> AlbumCandidate:
    """Build a low-confidence fallback candidate from folder and file hints."""
    tracks = request.tracks or _track_hints_from_path(request.path)
    album_artist = request.artist_hint
    if _is_compilation_hint(request.artist_hint) or _is_compilation_hint(request.album_hint):
        album_artist = "Various Artists"

    return AlbumCandidate(
        artist=request.artist_hint,
        artists=[request.artist_hint] if request.artist_hint else [],
        album=request.album_hint,
        album_artist=album_artist,
        album_artists=[album_artist] if album_artist else [],
        tracks=tracks,
        source=LookupSource.FOLDER,
    )


def _track_hints_from_path(path: Path) -> list[TrackCandidate]:
    try:
        audio_paths = iter_audio_files(path)
    except AutoTaggerError:
        return []

    tracks: list[TrackCandidate] = []
    for index, audio_path in enumerate(audio_paths, start=1):
        metadata = _read_metadata_or_none(audio_path)
        title = metadata.title if metadata and metadata.title else audio_path.stem
        track_number = metadata.track_number if metadata else index
        tracks.append(
            TrackCandidate(
                title=title,
                artist=metadata.artist if metadata else None,
                artists=metadata.artists if metadata else [],
                track_number=track_number or index,
                track_total=len(audio_paths),
                disc_number=metadata.disc_number if metadata else None,
                disc_total=metadata.disc_total if metadata else None,
                musicbrainz_trackid=metadata.musicbrainz_trackid if metadata else None,
            )
        )
    return tracks


def _read_metadata_or_none(path: Path) -> TrackMetadata | None:
    try:
        return read_metadata(path)
    except AutoTaggerError:
        return None


def _is_compilation_hint(value: str | None) -> bool:
    return bool(value and value.strip().lower() in COMPILATION_HINTS)
