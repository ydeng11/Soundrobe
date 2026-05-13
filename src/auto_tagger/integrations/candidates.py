"""Normalized lookup candidate models."""

from __future__ import annotations

import json
import re
import unicodedata
from dataclasses import dataclass, field
from enum import Enum
from hashlib import sha256
from pathlib import Path
from typing import Any


class LookupSource(Enum):
    """Source for a lookup candidate."""

    BEETS = "beets"
    DATASET = "dataset"
    DISCOGS = "discogs"
    FOLDER = "folder"


@dataclass(frozen=True)
class TrackCandidate:
    """Potential metadata for one track from an external lookup."""

    title: str | None = None
    artist: str | None = None
    artists: list[str] = field(default_factory=list)
    track_number: int | None = None
    track_total: int | None = None
    disc_number: int | None = None
    disc_total: int | None = None
    musicbrainz_trackid: str | None = None
    length: float | None = None

    def to_dict(self) -> dict[str, Any]:
        """Serialize to JSON-compatible data."""
        return {
            "title": self.title,
            "artist": self.artist,
            "artists": self.artists,
            "track_number": self.track_number,
            "track_total": self.track_total,
            "disc_number": self.disc_number,
            "disc_total": self.disc_total,
            "musicbrainz_trackid": self.musicbrainz_trackid,
            "length": self.length,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> TrackCandidate:
        """Deserialize from JSON-compatible data."""
        return cls(
            title=data.get("title"),
            artist=data.get("artist"),
            artists=list(data.get("artists") or []),
            track_number=data.get("track_number"),
            track_total=data.get("track_total"),
            disc_number=data.get("disc_number"),
            disc_total=data.get("disc_total"),
            musicbrainz_trackid=data.get("musicbrainz_trackid"),
            length=data.get("length"),
        )


@dataclass(frozen=True)
class AlbumCandidate:
    """Potential metadata for an album lookup result."""

    artist: str | None = None
    artists: list[str] = field(default_factory=list)
    album: str | None = None
    album_artist: str | None = None
    album_artists: list[str] = field(default_factory=list)
    year: str | None = None
    genre: str | None = None
    musicbrainz_albumid: str | None = None
    musicbrainz_artistid: str | None = None
    tracks: list[TrackCandidate] = field(default_factory=list)
    distance: float | None = None
    source: LookupSource = LookupSource.BEETS
    verification: str | None = None

    def to_dict(self) -> dict[str, Any]:
        """Serialize to JSON-compatible data."""
        return {
            "artist": self.artist,
            "artists": self.artists,
            "album": self.album,
            "album_artist": self.album_artist,
            "album_artists": self.album_artists,
            "year": self.year,
            "genre": self.genre,
            "musicbrainz_albumid": self.musicbrainz_albumid,
            "musicbrainz_artistid": self.musicbrainz_artistid,
            "tracks": [track.to_dict() for track in self.tracks],
            "distance": self.distance,
            "source": self.source.value,
            "verification": self.verification,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> AlbumCandidate:
        """Deserialize from JSON-compatible data."""
        return cls(
            artist=data.get("artist"),
            artists=list(data.get("artists") or []),
            album=data.get("album"),
            album_artist=data.get("album_artist"),
            album_artists=list(data.get("album_artists") or []),
            year=data.get("year"),
            genre=data.get("genre"),
            musicbrainz_albumid=data.get("musicbrainz_albumid"),
            musicbrainz_artistid=data.get("musicbrainz_artistid"),
            tracks=[TrackCandidate.from_dict(item) for item in data.get("tracks", [])],
            distance=data.get("distance"),
            source=LookupSource(data.get("source", LookupSource.BEETS.value)),
            verification=data.get("verification"),
        )

    def to_display_row(self) -> list[str]:
        """Return a row for CLI candidate preview tables."""
        distance = "" if self.distance is None else f"{self.distance:.2f}"
        return [
            self.source.value,
            self.artist or "",
            self.album or "",
            self.year or "",
            distance,
            self.musicbrainz_albumid or "",
            self.verification or "",
        ]


@dataclass(frozen=True)
class LookupRequest:
    """Inputs used to look up album candidates."""

    path: Path
    artist_hint: str | None = None
    album_hint: str | None = None
    year_hint: str | None = None
    tracks: list[TrackCandidate] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Serialize to JSON-compatible data."""
        return {
            "path": str(self.path),
            "artist_hint": self.artist_hint,
            "album_hint": self.album_hint,
            "year_hint": self.year_hint,
            "tracks": [track.to_dict() for track in self.tracks],
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> LookupRequest:
        """Deserialize from JSON-compatible data."""
        return cls(
            path=Path(data["path"]),
            artist_hint=data.get("artist_hint"),
            album_hint=data.get("album_hint"),
            year_hint=data.get("year_hint"),
            tracks=[TrackCandidate.from_dict(item) for item in data.get("tracks", [])],
        )

    def query_hash(self) -> str:
        """Return stable hash for cache keys."""
        query = {
            "artist_hint": self.artist_hint,
            "album_hint": self.album_hint,
            "tracks": [
                {
                    "title": track.title,
                    "track_number": track.track_number,
                    "disc_number": track.disc_number,
                }
                for track in self.tracks
            ],
            "track_count": len(self.tracks),
        }
        payload = json.dumps(query, sort_keys=True, separators=(",", ":"))
        return sha256(payload.encode("utf-8")).hexdigest()


def candidates_to_json(candidates: list[AlbumCandidate]) -> str:
    """Serialize album candidates to JSON."""
    return json.dumps([candidate.to_dict() for candidate in candidates], sort_keys=True)


def candidates_from_json(payload: str) -> list[AlbumCandidate]:
    """Deserialize album candidates from JSON."""
    return [AlbumCandidate.from_dict(item) for item in json.loads(payload)]


def _chinese_variants_match(a: str, b: str) -> bool:
    """Check if two strings match after converting between simplified and traditional Chinese.

    Uses opencc if available. Falls back gracefully if not installed.
    """
    try:
        import opencc
    except ImportError:
        return False

    try:
        t2s = opencc.OpenCC("t2s")
        s2t = opencc.OpenCC("s2t")
    except Exception:
        return False

    a_norm = normalize_lookup_text(a)
    b_norm = normalize_lookup_text(b)

    # Try converting a to simplified, b to simplified
    try:
        if normalize_lookup_text(t2s.convert(a)) == normalize_lookup_text(t2s.convert(b)):
            return True
    except Exception:
        pass

    # Try converting both to traditional
    try:
        if normalize_lookup_text(s2t.convert(a)) == normalize_lookup_text(s2t.convert(b)):
            return True
    except Exception:
        pass

    # Try cross: a simplified vs b simplified, a traditional vs b traditional
    try:
        if normalize_lookup_text(t2s.convert(a)) == b_norm:
            return True
        if a_norm == normalize_lookup_text(t2s.convert(b)):
            return True
    except Exception:
        pass

    return False


def normalize_lookup_text(value: str | None) -> str:
    """Normalize lookup text for comparison.

    Applies Unicode NFKC normalization to handle fullwidth/halfwidth
    variants and canonical equivalences, then case-folds and strips
    punctuation.

    Note: This does NOT convert between simplified and traditional
    Chinese (e.g., 挚 → 摯). That requires a dedicated library like
    opencc. For now, those pairs will register as 'close' if one
    is a substring of the other, or 'mismatch' otherwise.
    """
    if not value:
        return ""
    text = unicodedata.normalize("NFKC", value.casefold())
    text = re.sub(r"[^\w\s]", " ", text)
    return re.sub(r"\s+", " ", text).strip()


def verify_album_name(hint: str | None, candidate: AlbumCandidate) -> str:
    """Compare a lookup hint against a candidate's album name.

    Handles simplified/traditional Chinese variants via opencc when available.

    Returns:
        "match" — hint and candidate album name are identical after normalization
        "close" — hint text is contained within candidate or vice versa
        "mismatch" — hint does not match candidate album name
        "match" — when either hint or candidate album is None (can't verify)
    """
    if not hint or not candidate.album:
        return "match"
    hint_norm = normalize_lookup_text(hint)
    cand_norm = normalize_lookup_text(candidate.album)
    if hint_norm == cand_norm:
        return "match"
    # Try simplified/traditional Chinese conversion
    if _chinese_variants_match(hint, candidate.album):
        return "match"
    if hint_norm in cand_norm or cand_norm in hint_norm:
        return "close"
    return "mismatch"
