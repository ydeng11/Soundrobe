"""Folder-structure fallback lookup helpers."""

from __future__ import annotations

import re
from pathlib import Path

from auto_tagger.core import iter_audio_files, read_metadata
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.core.parse_filename import parse_album_folder_name, parse_track_filename
from auto_tagger.exceptions import AutoTaggerError
from auto_tagger.integrations.candidates import (
    AlbumCandidate,
    LookupRequest,
    LookupSource,
    TrackCandidate,
)

# Common patterns in folder names that aren't part of the album name
_DATE_PREFIX_RE = re.compile(r"^(\d{4})[-.](?:0[1-9]|1[0-2])\s*")  # "2003-06" or "2005.08"
_YEAR_PREFIX_RE = re.compile(r"^(\d{4})[.-]\s*")  # "2017-" or "2018." alone
_YEAR_FROM_PREFIX_RE = re.compile(r"^(\d{4})[-.]")  # capture year from date prefix (e.g. "2003" from "2003-04《挚爱》")
_BOOKMARKS_RE = re.compile(r"[《》「」【】\[\]]")  # Chinese/Western bookmarks
_EXTRA_SUFFIX_RE = re.compile(r"\s*\([^)]*\)\s*$")  # trailing "(FLAC分轨)" etc.
_FORMAT_SUFFIX_RE = re.compile(r"\[?(flac|mp3|wav|aac|ogg|m4a|wma|ape|flac\s*分轨|wav\s*分轨)\]?\s*$", re.IGNORECASE)  # trailing [flac] etc.
_CD_SUBFOLDER_RE = re.compile(r"(?:[Cc][Dd]|[Dd][Ii][Ss][CcKk]|ディスク)\s*\d+\s*$")  # CD1, Disc 1, Disk1, ディスク1


def extract_year_from_name(name: str) -> str | None:
    """Extract a 4-digit year from a folder name.

    Tries, in order:
      1. Leading date prefix: "2003-04《挚爱》" → "2003"
      2. Inside bookmark brackets: "Artist-《2011-重译》" → "2011"
      3. Inside parentheses: "Artist (2011) Album" → "2011"

    "2005.08 Album" → "2005"
    "2017- Album" → "2017"
    "Album Name" → None

    Returns the year string (e.g. "2003") or None.
    """
    # 1. Leading date prefix
    match = _YEAR_FROM_PREFIX_RE.match(name)
    if match:
        return match.group(1)
    # 2. Inside Chinese bookmarks: 《2011-重译》
    m = re.search(r'[《（（\[]\s*(\d{4})\s*[-.]', name)
    if m:
        return m.group(1)
    # 3. Parenthesized year anywhere: (2011) or [2011]
    m = re.search(r'[\[(（]\s*(\d{4})\s*[\])）]', name)
    if m:
        return m.group(1)
    return None


def clean_folder_name(name: str) -> str:
    """Clean a folder name for use as a lookup hint.

    Strips common metadata prefixes/suffixes that aren't part of the
    actual album or artist name:
      - Date prefixes: "2003-04《挚爱》" → "挚爱"
      - Bookmarks: "《挚爱》" → "挚爱"
      - Extra suffixes: "Hello (Bonus)" → "Hello"

    For folder names containing paired bookmarks (e.g. "Artist-《2011-Album》[FLAC]"),
    the content inside the bookmarks is extracted first, then cleaned.
    This ensures the album hint is just the album name without artist/year prefixes.

    Returns the cleaned name, or the original if nothing to strip.
    """
    # First try extracting content from inside Chinese bookmarks:
    # "Artist-《2011-Album》[FLAC]" → extract "2011-Album" → clean → "Album"
    bracketed = re.search(r'《([^》]+)》', name)
    if bracketed:
        inner = bracketed.group(1)
        cleaned = _DATE_PREFIX_RE.sub("", inner)
        cleaned = _YEAR_PREFIX_RE.sub("", cleaned)
        cleaned = cleaned.strip()
        if cleaned:
            return cleaned

    # Fallback: standard cleanup on full name
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

    # Detect CD subfolder pattern (e.g. "Artist - Album CD1" or
    # "Artist.Album CD2").  When inside a CD subfolder the immediate
    # parent is the album bundle folder ("Album (2CD)"), and the
    # grandparent is the true artist folder.
    if album_path.name and _CD_SUBFOLDER_RE.search(album_path.name):
        artist_hint = (
            clean_folder_name(album_path.parent.parent.name)
            if album_path.parent.parent and album_path.parent.parent.name
            else None
        )
    else:
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
    """Build a low-confidence fallback candidate from folder and file hints.

    Enriches ``LookupRequest`` hints with structured folder-name parsing.
    Compilation detection is handled upstream by the workflow.
    """
    tracks = request.tracks or _track_hints_from_path(request.path)

    # Enrich hints using structured folder-name parsing
    album_path = request.path.parent if request.path.is_file() else request.path
    parsed_folder = parse_album_folder_name(album_path.name) if album_path.name else None

    # Use parsed values as fallback when request hints are missing
    artist = request.artist_hint or (parsed_folder.artist if parsed_folder else None)
    album = request.album_hint or (parsed_folder.album if parsed_folder else None)
    year = request.year_hint or (parsed_folder.year if parsed_folder else None)
    album_artist = artist

    return AlbumCandidate(
        artist=artist,
        artists=[artist] if artist else [],
        album=album,
        album_artist=album_artist,
        album_artists=[album_artist] if album_artist else [],
        year=year,
        tracks=tracks,
        source=LookupSource.FOLDER,
    )


def _tag_or_parsed(meta_value, parsed_value, fallback=None):
    """Return *meta_value* if truthy, else *parsed_value*, else *fallback*."""
    return meta_value if meta_value else (parsed_value if parsed_value else fallback)


def _track_hints_from_path(path: Path) -> list[TrackCandidate]:
    """Build track candidates from audio file metadata and filename parsing.

    Priority: embedded tags > filename parsing > raw file stem / index.
    """
    try:
        audio_paths = iter_audio_files(path)
    except AutoTaggerError:
        return []

    tracks: list[TrackCandidate] = []
    for index, audio_path in enumerate(audio_paths, start=1):
        m = _read_metadata_or_none(audio_path)
        p = parse_track_filename(audio_path.stem)

        tracks.append(
            TrackCandidate(
                title=_tag_or_parsed(m.title if m else None, p.title, audio_path.stem),
                artist=_tag_or_parsed(m.artist if m else None, p.artist),
                artists=(m.artists if m and m.artists else p.artists),
                track_number=_tag_or_parsed(m.track_number if m else None, p.track_number, index),
                track_total=len(audio_paths),
                disc_number=_tag_or_parsed(m.disc_number if m else None, p.disc_number),
                disc_total=m.disc_total if m else None,
                musicbrainz_trackid=m.musicbrainz_trackid if m else None,
            )
        )
    return tracks


def _read_metadata_or_none(path: Path) -> TrackMetadata | None:
    try:
        return read_metadata(path)
    except AutoTaggerError:
        return None


