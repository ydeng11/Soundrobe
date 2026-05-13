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
_DATE_PREFIX_RE = re.compile(r"^(\d{4})[-.]\d{2}\s*")  # "2003-04" or "2005.08"
_YEAR_PREFIX_RE = re.compile(r"^(\d{4})[.-]\s*")  # "2017-" or "2018." alone
_YEAR_FROM_PREFIX_RE = re.compile(r"^(\d{4})[-.]")  # capture year from date prefix (e.g. "2003" from "2003-04《挚爱》")
_BOOKMARKS_RE = re.compile(r"[《》「」【】\[\]]")  # Chinese/Western bookmarks
_EXTRA_SUFFIX_RE = re.compile(r"\s*\([^)]*\)\s*$")  # trailing "(FLAC分轨)" etc.
_FORMAT_SUFFIX_RE = re.compile(r"\[?(flac|mp3|wav|aac|ogg|m4a|wma|ape|flac分轨|wav分轨)\]?\s*$", re.IGNORECASE)  # trailing [flac] etc.


def extract_year_from_name(name: str) -> str | None:
    """Extract a 4-digit year from a folder name's leading date prefix.

    "2003-04《挚爱》" → "2003"
    "2005.08 Album" → "2005"
    "2017- Album" → "2017"
    "Album Name" → None

    Returns the year string (e.g. "2003") or None.
    """
    match = _YEAR_FROM_PREFIX_RE.match(name)
    return match.group(1) if match else None


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
    cleaned = _YEAR_PREFIX_RE.sub("", cleaned)
    cleaned = _BOOKMARKS_RE.sub("", cleaned)
    # Strip format suffix first so parenthetical suffix can be matched
    cleaned = _FORMAT_SUFFIX_RE.sub("", cleaned)
    cleaned = _EXTRA_SUFFIX_RE.sub("", cleaned)
    cleaned = cleaned.strip()
    return cleaned or name


def parse_album_path(path: Path) -> LookupRequest:
    """Parse artist, album, and year hints from an Artist/Album path.

    Folder names are cleaned of date prefixes, bookmarks, and extra
    suffixes before being used as lookup hints. The year is extracted
    from the album folder's leading date prefix if present.
    """
    album_path = path.parent if path.is_file() else path
    album_hint = clean_folder_name(album_path.name) if album_path.name else None
    artist_hint = clean_folder_name(album_path.parent.name) if album_path.parent.name else None
    year_hint = extract_year_from_name(album_path.name) if album_path.name else None
    return LookupRequest(
        path=path,
        artist_hint=artist_hint,
        album_hint=album_hint,
        year_hint=year_hint,
    )


def parse_album_with_tags(path: Path) -> LookupRequest:
    """Build a LookupRequest using both folder names and existing file tags.

    Existing tag values take priority over folder names — a human (or
    previous tool) put those tags there intentionally. Folder names are
    only used as fallback hints when tags are missing.

    Exception: if tag artist clearly differs from the folder artist name,
    the folder name wins (tags may have been written incorrectly by a
    previous auto-tag run).
    """
    folder_request = parse_album_path(path)
    tag_hints = _read_album_tags_from_first_file(path)

    # Use tag hints, but verify artist matches folder structure
    tag_artist = tag_hints.get("artist")
    folder_artist = folder_request.artist_hint

    if tag_artist and folder_artist and tag_artist.casefold() != folder_artist.casefold():
        # Tag artist doesn't match folder — folder structure is more trustworthy.
        # Also distrust the album tag since it may have been written by a wrong run.
        artist_hint = folder_artist
        album_hint = folder_request.album_hint
    else:
        artist_hint = tag_artist or folder_artist
        # Prefer cleaned folder album hint over tag — folder names are more
        # recently trustworthy and may have been cleaned of suffixes.
        album_hint = folder_request.album_hint or tag_hints.get("album")
    tracks = folder_request.tracks or _track_hints_from_path(path)

    return LookupRequest(
        path=path,
        artist_hint=artist_hint,
        album_hint=album_hint,
        year_hint=folder_request.year_hint or tag_hints.get("year"),
        tracks=tracks,
    )


def _read_album_tags_from_first_file(path: Path) -> dict[str, str | None]:
    """Read album-level tags from the first audio file in the directory."""
    try:
        audio_paths = iter_audio_files(path)
    except AutoTaggerError:
        return {}
    for audio_path in audio_paths:
        metadata = _read_metadata_or_none(audio_path)
        if metadata is not None and (metadata.album or metadata.artist):
            return {
                "artist": metadata.artist or metadata.album_artist,
                "album": metadata.album,
                "year": metadata.year,
            }
    return {}


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
        year=request.year_hint,
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
