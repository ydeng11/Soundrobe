"""Compilation album detection and metadata transforms."""

from __future__ import annotations

from dataclasses import dataclass, replace

from auto_tagger.core.metadata import TrackMetadata


@dataclass(frozen=True)
class CompilationAnalysis:
    """Explainable compilation detection result."""

    is_compilation: bool
    confidence: float
    reasons: list[str]


def analyze_compilation(
    tracks: list[TrackMetadata],
    album_path_hint: str = "",
) -> CompilationAnalysis:
    """Analyze album metadata for compilation signals."""
    reasons: list[str] = []
    hint = album_path_hint.lower()
    if "various artists" in hint:
        reasons.append("Folder path mentions Various Artists")
    if any(token in hint for token in ("soundtrack", "ost", "compilation")):
        reasons.append("Folder path suggests soundtrack or compilation")

    artists = {track.artist.strip() for track in tracks if track.artist and track.artist.strip()}
    album_artists = {
        track.album_artist.strip().lower()
        for track in tracks
        if track.album_artist and track.album_artist.strip()
    }
    if "various artists" in album_artists:
        reasons.append("Album artist is Various Artists")
    if len(artists) >= 3 or (len(tracks) > 1 and len(artists) == len(tracks)):
        reasons.append("Track artists vary across the album")
    if any(track.compilation for track in tracks):
        reasons.append("Existing compilation tag is present")

    confidence = min(1.0, 0.4 * len(reasons))
    is_compilation = confidence >= 0.65
    return CompilationAnalysis(is_compilation, confidence, reasons)


def apply_compilation_tags(tracks: list[TrackMetadata]) -> list[TrackMetadata]:
    """Return tracks tagged as a Various Artists compilation."""
    return [
        replace(
            track,
            album_artist="Various Artists",
            album_artists=["Various Artists"],
            compilation=True,
        ).normalized()
        for track in tracks
    ]
