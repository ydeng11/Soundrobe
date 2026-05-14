"""Normalized metadata models for audio tags."""

from __future__ import annotations

from dataclasses import dataclass, field, replace
from typing import Any


def parse_position(value: Any) -> tuple[int | None, int | None]:
    """Parse a track or disc position from ``N`` or ``N/TOTAL`` values."""
    if value is None:
        return None, None

    if isinstance(value, (list, tuple)):
        if not value:
            return None, None
        value = value[0]

    text = str(value).strip()
    if not text:
        return None, None

    current_text, _, total_text = text.partition("/")
    current = _parse_int(current_text)
    total = _parse_int(total_text) if total_text else None
    return current, total


def format_position(current: int | None, total: int | None = None) -> str | None:
    """Format a track or disc position for tag storage."""
    if current is None:
        return None
    if total is None:
        return str(current)
    return f"{current}/{total}"


def _parse_int(value: str) -> int | None:
    value = value.strip()
    if not value:
        return None
    try:
        return int(value)
    except ValueError:
        return None


def _clean_list(values: list[str]) -> list[str]:
    return [value.strip() for value in values if value and value.strip()]


@dataclass(frozen=True)
class ReplayGainTags:
    """ReplayGain tag values stored as normalized strings."""

    track_gain: str | None = None
    track_peak: str | None = None
    album_gain: str | None = None
    album_peak: str | None = None

    def is_empty(self) -> bool:
        """Return whether all ReplayGain fields are empty."""
        return not any((self.track_gain, self.track_peak, self.album_gain, self.album_peak))


@dataclass(frozen=True)
class TrackMetadata:
    """Normalized track metadata independent of the underlying audio format."""

    title: str | None = None
    artist: str | None = None
    artists: list[str] = field(default_factory=list)
    album: str | None = None
    album_artist: str | None = None
    album_artists: list[str] = field(default_factory=list)
    track_number: int | None = None
    track_total: int | None = None
    disc_number: int | None = None
    disc_total: int | None = None
    year: str | None = None
    genre: str | None = None
    musicbrainz_trackid: str | None = None
    musicbrainz_albumid: str | None = None
    musicbrainz_artistid: str | None = None
    lyrics: str | None = None
    composer: str | None = None
    compilation: bool | None = None
    replaygain: ReplayGainTags = field(default_factory=ReplayGainTags)

    def normalized(self) -> TrackMetadata:
        """Return a copy with cleaned list fields and display fallbacks."""
        artists = _clean_list(self.artists)
        if not artists and self.artist:
            artists = [self.artist.strip()]

        album_artists = _clean_list(self.album_artists)
        if not album_artists and self.album_artist:
            album_artists = [self.album_artist.strip()]

        return replace(
            self,
            artists=artists,
            album_artists=album_artists,
        )

    def to_display_rows(self) -> list[list[str]]:
        """Return populated metadata fields as label/value rows for CLI display."""
        rows: list[list[str]] = []
        fields = [
            ("title", self.title),
            ("artist", self.artist),
            ("artists", ", ".join(self.artists)),
            ("album", self.album),
            ("album_artist", self.album_artist),
            ("album_artists", ", ".join(self.album_artists)),
            ("track", format_position(self.track_number, self.track_total)),
            ("disc", format_position(self.disc_number, self.disc_total)),
            ("year", self.year),
            ("genre", self.genre),
            ("musicbrainz_trackid", self.musicbrainz_trackid),
            ("musicbrainz_albumid", self.musicbrainz_albumid),
            ("musicbrainz_artistid", self.musicbrainz_artistid),
            ("lyrics", "embedded" if self.lyrics else None),
            ("composer", self.composer),
            ("compilation", "1" if self.compilation else None),
            ("replaygain_track_gain", self.replaygain.track_gain),
            ("replaygain_track_peak", self.replaygain.track_peak),
            ("replaygain_album_gain", self.replaygain.album_gain),
            ("replaygain_album_peak", self.replaygain.album_peak),
        ]
        for label, value in fields:
            if value:
                rows.append([label, str(value)])
        return rows
