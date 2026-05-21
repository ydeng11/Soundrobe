"""Parse metadata from file and folder naming conventions.

This module extracts structured metadata (track number, title, artist,
album, year, etc.) from filenames and folder names according to
observed Chinese and Western naming conventions.  The parsed values
serve as *fallback* hints when embedded audio tags are missing or
unreliable.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path

# ── regular expressions (ordered by priority, left-to-right) ──────────

# Pattern 1:  "(01) [Artist] Title.ext"
_TRACK_PAREN_BRACKET_RE = re.compile(
    r"^\((\d{1,3})\)\s*\[([^\]]+)\]\s*(.+)$"
)

# Pattern 2:  "Artist - NN.Title.ext"  or  "Artist - NN Title.ext"
# The artist part must contain at least one non-digit to avoid matching
# "01 - Title" as artist="0", track=1.
_TRACK_ARTIST_DASH_NUMBER_RE = re.compile(
    r"^(.+?\D)\s*[-—]\s*(\d{1,3})\s*[.\-\s_]\s*(.+)$"
)

# Pattern 3:  "NN. Title.ext"
_TRACK_NUMBER_DOT_RE = re.compile(
    r"^(\d{1,3})\.\s*(.+)$"
)

# Pattern 4:  "NN Title.ext"
# DO NOT match if the title part starts with a dash/hyphen — that's a
# different naming convention (e.g. "01 - Title" should not match here).
_TRACK_NUMBER_SPACE_RE = re.compile(
    r"^(\d{1,3})\s+(?![-—])(.+)$"
)

# Pattern 4b:  "NN-SeparatorTitle.ext"  (no space, e.g. "01-Song", "01_Song")
# The separator is a single dash, dot, or underscore immediately after the number.
# This must NOT match "Artist-Title" (no number) which is handled later.
_TRACK_NUMBER_SEP_RE = re.compile(
    r"^(\d{1,3})[-._]\s*(?![-—])(.+)$"
)

# Pattern 4c:  "NNSongTitle.ext"  (no separator at all, e.g. "01Song")
# Ambiguous — only match when the first character after the number is a letter.
_TRACK_NUMBER_NOSEP_RE = re.compile(
    r"^(\d{1,3})([A-Za-z].*)$"
)

# Pattern 5:  "Artist — Title.ext"  (em dash or hyphen)
# The artist part must contain at least one non-digit.
_TRACK_ARTIST_DASH_TITLE_RE = re.compile(
    r"^(.+?\D)\s+[-—]\s+(.+)$"
)

# Pattern 6:  Vinyl side+track  "A1. Title.ext", "B2. Title.ext"
_VINYL_SIDE_TRACK_RE = re.compile(
    r"^([A-D])(\d{1,2})\.\s*(.+)$"
)

# Pattern 7:  "Title - Suffix YYYY - Artist.ext"
# Tried early (before single-dash patterns) because it has two dashes
# separated by a 4-digit year, making it very specific.
_TRACK_TITLE_SUFFIX_ARTIST_RE = re.compile(
    r"^(.+?)\s*[-—]\s*(.+?)\s*(\d{4})\s*[-—]\s*(.+)$"
)

# Pattern 8:  "(NN) Title.ext"  (no bracketed artist)
_TRACK_PAREN_NO_ARTIST_RE = re.compile(
    r"^\((\d{1,3})\)\s*(.+)$"
)

# ── Album folder patterns ─────────────────────────────────────────────

# Pattern A:  "Artist-《Year-Album》[Format]"
_ALBUM_ARTIST_BOOKMARK_RE = re.compile(
    r"^(.+?)-《(\d{4})-([^》]+)》"
)

# Pattern B:  "[Year] Album (Edition)"
_ALBUM_BRACKET_YEAR_RE = re.compile(
    r"^\[(\d{4})\]\s*(.+)$"
)

# Pattern C:  "Year - Album"  or  "Year-Album"
_ALBUM_YEAR_DASH_RE = re.compile(
    r"^(\d{4})\s*[-—]\s*(.+)$"
)

# Pattern D:  "Artist - Album (Year) [Info]"
_ALBUM_ARTIST_DASH_RE = re.compile(
    r"^(.+?)\s*-\s*(.+?)\s*\((\d{4})\)"
)

# Strip trailing format/edition markers:  [FLAC], [WAV 分轨], (2011 Remaster), etc.
_FORMAT_SUFFIX_RE = re.compile(
    r"""\[[^\]]*\]       # [FLAC], [WAV 分轨], [LP]
    |\([^)]*(?:Remaster|Deluxe|Edition|LP|CD|Box|Bonus).*?\)  # (2011 Remaster), (Deluxe Edition)
    """,
    re.VERBOSE | re.IGNORECASE,
)


# ── helpers ───────────────────────────────────────────────────────────

def _strip_format_suffixes(text: str) -> str:
    """Remove trailing format/edition markers like ``[FLAC]``, ``(2011 Remaster)``."""
    result = text.strip()
    prev = None
    while prev != result:
        prev = result
        result = _FORMAT_SUFFIX_RE.sub("", result).strip()
    return result


def _remove_leading_punctuation(text: str) -> str:
    """Remove leading whitespace, dashes, or dots from a cleaned stem."""
    return re.sub(r"^[\s.、，,。\-—]+", "", text).strip()


# ── Public dataclass ──────────────────────────────────────────────────


@dataclass
class ParsedFilename:
    """Structured metadata extracted from a file or folder name.

    All fields are optional — only the values that could be confidently
    parsed are populated.
    """

    title: str | None = None
    artist: str | None = None
    artists: list[str] = field(default_factory=list)
    album: str | None = None
    album_artist: str | None = None
    track_number: int | None = None
    disc_number: int | None = None
    year: str | None = None


# ── Track-level parsing ───────────────────────────────────────────────


def parse_track_filename(stem: str) -> ParsedFilename:
    """Parse a file stem (without extension) into structured metadata.

    Tries patterns in priority order.  The first pattern that matches
    wins.  Patterns range from most specific (bracketed artist + numbered)
    to least specific (plain title only).

    Examples::

        "(01) [陈洁仪] 心痛"   → ParsedFilename(title="心痛", artist="陈洁仪", track_number=1)
        "01. 崇拜"             → ParsedFilename(title="崇拜", track_number=1)
        "蔡健雅 - 01.呼吸"     → ParsedFilename(title="呼吸", artist="蔡健雅", track_number=1)
        "陈洁仪 - 最好的年纪"  → ParsedFilename(title="最好的年纪", artist="陈洁仪")
        "A1. Rolling In The Deep" → ParsedFilename(title="Rolling In The Deep",
                                                     track_number=1, disc_number=1)
    """
    # Pattern 1:  "(01) [Artist] Title"
    m = _TRACK_PAREN_BRACKET_RE.match(stem)
    if m:
        return ParsedFilename(
            title=m.group(3).strip(),
            artist=m.group(2).strip(),
            artists=_split_artists(m.group(2).strip()),
            track_number=int(m.group(1)),
        )

    # Pattern 7 (early):  "Title - Suffix YYYY - Artist"
    # Tried before single-dash patterns because the double-dash + year
    # structure is very specific and unambiguous.
    # Matches e.g. "Bohemian Rhapsody - Remastered 2011 - Queen"
    m = _TRACK_TITLE_SUFFIX_ARTIST_RE.match(stem)
    if m:
        return ParsedFilename(
            title=m.group(1).strip(),
            artist=m.group(4).strip(),
            artists=_split_artists(m.group(4).strip()),
            year=m.group(3),
        )

    # Pattern 2:  "Artist - NN.Title"  or  "Artist - NN Title"
    # Artist part must contain at least one non-digit character.
    m = _TRACK_ARTIST_DASH_NUMBER_RE.match(stem)
    if m:
        title = _remove_leading_punctuation(m.group(3))
        return ParsedFilename(
            title=title,
            artist=m.group(1).strip(),
            artists=_split_artists(m.group(1).strip()),
            track_number=int(m.group(2)),
        )

    # Pattern 3:  "NN. Title"
    m = _TRACK_NUMBER_DOT_RE.match(stem)
    if m:
        return _numbered_title(int(m.group(1)), m.group(2))

    # Pattern 4:  "NN Title"
    m = _TRACK_NUMBER_SPACE_RE.match(stem)
    if m:
        return _numbered_title(int(m.group(1)), m.group(2))

    # Pattern 4b:  "NN-SeparatorTitle" (no space, e.g. "01-Song", "01_Song")
    m = _TRACK_NUMBER_SEP_RE.match(stem)
    if m:
        return _numbered_title(int(m.group(1)), m.group(2))

    # Pattern 4c:  "NNSongTitle" (no separator, e.g. "01Song")
    m = _TRACK_NUMBER_NOSEP_RE.match(stem)
    if m:
        return _numbered_title(int(m.group(1)), m.group(2))

    # Pattern 5:  "Artist — Title"  (em dash or hyphen)
    # Artist part must contain at least one non-digit character.
    m = _TRACK_ARTIST_DASH_TITLE_RE.match(stem)
    if m:
        artist = m.group(1).strip()
        title = _remove_leading_punctuation(m.group(2))
        return ParsedFilename(
            title=title,
            artist=artist,
            artists=_split_artists(artist),
        )

    # Pattern 6:  Vinyl side+track  "A1. Title"
    m = _VINYL_SIDE_TRACK_RE.match(stem)
    if m:
        side = m.group(1)
        track = int(m.group(2))
        disc_number = ord(side.upper()) - ord("A") + 1  # A→1, B→2, C→3, D→4
        title = _remove_leading_punctuation(m.group(3))
        return ParsedFilename(
            title=title,
            track_number=track,
            disc_number=disc_number,
        )

    # Pattern 8:  "(NN) Title"  (no bracketed artist)
    m = _TRACK_PAREN_NO_ARTIST_RE.match(stem)
    if m:
        return _numbered_title(int(m.group(1)), m.group(2))

    # No pattern matched — use the stem as-is as the title
    return ParsedFilename(title=stem.strip())


def _numbered_title(track_number: int, raw_title: str) -> ParsedFilename:
    """Build a ParsedFilename from a track number and raw title text."""
    return ParsedFilename(
        title=_remove_leading_punctuation(raw_title),
        track_number=track_number,
    )


def _split_artists(text: str) -> list[str]:
    """Split a single artist string on collaboration markers.

    Handles::

        "陈洁仪＋苏永康" → ["陈洁仪", "苏永康"]
        "陈洁仪+苏永康"  → ["陈洁仪", "苏永康"]
        "陈洁仪, 苏永康" → ["陈洁仪", "苏永康"]
        "陈洁仪 & 苏永康" → ["陈洁仪", "苏永康"]
        "陈洁仪"         → ["陈洁仪"]
    """
    separators = re.compile(r"\s*[＋+&,/&]\s*")
    parts = separators.split(text)
    return [p.strip() for p in parts if p.strip()]


# ── Album-level parsing ───────────────────────────────────────────────


def parse_album_folder_name(name: str) -> ParsedFilename:
    """Parse an album folder name into structured metadata.

    Tries patterns in priority order.  Returns the first match.

    Examples::

        "陈洁仪-《1994-心痛》[WAV 分轨]"  → ParsedFilename(album="心痛",
                                                  artist="陈洁仪", year="1994")
        "1993-Karen"                      → ParsedFilename(album="Karen", year="1993")
        "[1975] A Night At The Opera"     → ParsedFilename(album="A Night At The Opera",
                                                  year="1975")
        "Adele - 21 (2011) [LP] [flac]"  → ParsedFilename(album="21",
                                                  artist="Adele", year="2011")
        "Album Name"                      → ParsedFilename(album="Album Name")
    """
    stripped = name.strip()

    # Pattern A:  "Artist-《Year-Album》[Format]"
    m = _ALBUM_ARTIST_BOOKMARK_RE.match(stripped)
    if m:
        album = _strip_format_suffixes(m.group(3).strip())
        return ParsedFilename(
            album=album or m.group(3).strip(),
            artist=m.group(1).strip(),
            album_artist=m.group(1).strip(),
            year=m.group(2),
        )

    # Pattern D (try before B/C because it's more specific):
    # "Artist - Album (Year) [Info]"
    m = _ALBUM_ARTIST_DASH_RE.match(stripped)
    if m:
        album = _strip_format_suffixes(m.group(2).strip())
        return ParsedFilename(
            album=album or m.group(2).strip(),
            artist=m.group(1).strip(),
            album_artist=m.group(1).strip(),
            year=m.group(3),
        )

    # Pattern B:  "[Year] Album (Edition)"
    m = _ALBUM_BRACKET_YEAR_RE.match(stripped)
    if m:
        album = _strip_format_suffixes(m.group(2).strip())
        return ParsedFilename(
            album=album or m.group(2).strip(),
            year=m.group(1),
        )

    # Pattern C:  "Year-Album"  or  "Year - Album"
    m = _ALBUM_YEAR_DASH_RE.match(stripped)
    if m:
        album = _strip_format_suffixes(m.group(2).strip())
        return ParsedFilename(
            album=album or m.group(2).strip(),
            year=m.group(1),
        )

    # No pattern matched — stripped folder name is the album name
    cleaned = _strip_format_suffixes(stripped)
    return ParsedFilename(album=cleaned or stripped)


# ── Path-level parsing ────────────────────────────────────────────────


_SKIP_GRANDPARENT = frozenset({
    "Loose", "Various", "Various Artists", "Compilations",
    "downloads", "music", "Volumes",
})


def _looks_like_artist_name(name: str) -> bool:
    """Heuristic check: does *name* plausibly name a real artist?

    Rejects:
    * temp/test directories with hyphens or underscores (e.g. ``pytest-123``)
    * known non-artist folder names
    * single-component UUID or hash-like names
    """
    if not name:
        return False
    if name in _SKIP_GRANDPARENT:
        return False
    # Reject paths that look like temp/test dirs (containing hyphens/underscores
    # mixed with digits in a non-name-like way)
    if re.match(r"^[a-z]+[-_][a-z0-9]+", name, re.IGNORECASE):
        return False
    # Reject purely numeric or UUID-like names
    if re.match(r"^[0-9a-f]{8,}$", name, re.IGNORECASE):
        return False
    return True


def metadata_from_path(audio_path: Path) -> ParsedFilename:
    """Parse metadata from a complete audio file path.

    Combines file-level parsing (stem) with parent folder (album) and
    grandparent folder (artist) information.

    The parse order is:

    1. File stem → track_number, title, track_artist
    2. Parent folder → album, year
    3. Grandparent folder → artist (if not already found at file level)

    Values from closer to the file take priority: file > parent > grandparent.

    Album-level info is only applied when the file-level parse did not
    already provide a value for that field.
    """
    file_parsed = parse_track_filename(audio_path.stem)

    album_parsed = ParsedFilename()
    if audio_path.parent and audio_path.parent.name:
        album_parsed = parse_album_folder_name(audio_path.parent.name)

    # If file-level parsing didn't find an artist, use the grandparent folder name
    artist_from_grandparent: str | None = None
    if (not file_parsed.artist
            and not album_parsed.artist
            and audio_path.parent.parent
            and audio_path.parent.parent.name):
        gp = audio_path.parent.parent.name
        if _looks_like_artist_name(gp):
            artist_from_grandparent = gp

    return ParsedFilename(
        title=file_parsed.title,
        artist=file_parsed.artist or album_parsed.artist or artist_from_grandparent,
        artists=file_parsed.artists or album_parsed.artists or (
            [artist_from_grandparent] if artist_from_grandparent else []
        ),
        album=file_parsed.album or album_parsed.album,
        album_artist=file_parsed.album_artist or album_parsed.album_artist or artist_from_grandparent,
        track_number=file_parsed.track_number,
        disc_number=file_parsed.disc_number,
        year=file_parsed.year or album_parsed.year,
    )
