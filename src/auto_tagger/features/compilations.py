"""Compilation album detection and metadata transforms.

Handles four distinct multi-artist patterns:

1. **Compilation** — different artists per track (e.g., *Now That's What I Call Music*).
   → album_artist = "Various Artists", compilation = True

2. **Collaboration single** — one track, many performers (e.g., *We Are The World*).
   → artist = group name, artists = [full list], album_artist = group name,
     compilation = False

3. **Classical / primary-performer album** — tracks vary by composer or ensemble
   but share one primary performer (e.g., Anne-Sophie Mutter).
   → artist = specific per-track performer(s), album_artist = primary performer,
     compilation = False

4. **Soundtrack / OST** — could be either a curated compilation (many artists,
   one per track) or a scored work (one composer, all tracks).
   → Uses composer-based heuristics to decide.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, replace

from auto_tagger.core.metadata import TrackMetadata

# ── helpers ────────────────────────────────────────────────────

_COMMA_SEPARATED_ARTIST_RE = re.compile(r"\s*[,;&]\s*")


def _extract_primary_performer(artist: str | None) -> str | None:
    """Extract the primary performer name before any conjunction or comma.

    For "Anne‐Sophie Mutter, Ye‐Eun Choi" → "Anne‐Sophie Mutter"
    For "Herbert Blomstedt, San Francisco Symphony" → "Herbert Blomstedt"
    Returns None if artist is empty.
    """
    if not artist or not artist.strip():
        return None
    parts = _COMMA_SEPARATED_ARTIST_RE.split(artist.strip())
    return parts[0].strip() if parts else artist.strip()


def _has_multi_artist_track(track: TrackMetadata) -> bool:
    """Check if a single track has multiple credited artists.

    Detects: populated artists list, or comma/ampersand/semicolon in artist
    field with **more than 2 distinct names**.

    Two-part names like "Herbert Blomstedt, San Francisco Symphony"
    (conductor + orchestra) are NOT considered multi-artist — they represent
    a single performing entity.  Three-plus names like
    "U.S.A. For Africa, Michael Jackson, Lionel Richie, ..." signal a true
    collaboration.
    """
    if len(track.artists) > 1:
        return True
    if track.artist and _COMMA_SEPARATED_ARTIST_RE.search(track.artist):
        parts = [p.strip() for p in _COMMA_SEPARATED_ARTIST_RE.split(track.artist) if p.strip()]
        return len(parts) >= 3
    return False


def _extract_album_path_artist(path: str) -> str | None:
    """Guess the artist from the folder path (second-to-last component).

    For "/Music/Artist Name/Album Name/" → "Artist Name"
    """
    parts = [p for p in path.split("/") if p]
    if len(parts) >= 2:
        return parts[-2]
    return None


# ── analysis result ───────────────────────────────────────────


@dataclass(frozen=True)
class CompilationAnalysis:
    """Explainable compilation detection result."""

    is_compilation: bool
    confidence: float
    reasons: list[str]
    suggested_album_artist: str | None = None
    is_collaboration: bool = False


# ── analysis ───────────────────────────────────────────────────


def analyze_compilation(
    tracks: list[TrackMetadata],
    album_path_hint: str = "",
) -> CompilationAnalysis:
    """Analyze album metadata for compilation signals.

    Returns a ``CompilationAnalysis`` with the decision, confidence,
    suggested album_artist, and whether it's a collaboration pattern.
    """
    reasons: list[str] = []
    hint = album_path_hint.lower()

    # ── folder-path signals ──
    path_artist = _extract_album_path_artist(album_path_hint)

    if "various artists" in hint:
        reasons.append("Folder path mentions Various Artists")
    if any(token in hint for token in ("soundtrack", "ost", "compilation", "原声带")):
        reasons.append("Folder path suggests soundtrack or compilation")

    # ── track-artist diversity ──
    raw_artists: set[str] = set()
    primary_performers: set[str] = set()
    multi_artist_track_count = 0
    for track in tracks:
        if track.artist and track.artist.strip():
            raw_artists.add(track.artist.strip())
        pp = _extract_primary_performer(track.artist)
        if pp:
            primary_performers.add(pp)
        if _has_multi_artist_track(track):
            multi_artist_track_count += 1

    album_artists_set: set[str] = {
        track.album_artist.strip().lower()
        for track in tracks
        if track.album_artist and track.album_artist.strip()
    }
    if "various artists" in album_artists_set:
        reasons.append("Album artist is Various Artists")

    # ── collaboration vs compilation detection ──
    # If ALL tracks have multi-artist, this is a collaboration album
    # (e.g., We Are The World — one track, but same pattern across all)
    is_collaboration = (
        len(tracks) > 0 and multi_artist_track_count == len(tracks)
    )
    if is_collaboration:
        reasons.append("All tracks feature multiple artists — collaboration pattern")

    # Classic compilation: artist varies across tracks
    if not is_collaboration:
        if len(raw_artists) >= 3:
            reasons.append("Track artists vary across the album (>= 3 distinct)")
        elif len(tracks) > 1 and len(raw_artists) == len(tracks):
            reasons.append("Every track has a different artist")

    # ── classical / primary-performer signal ──
    # If all tracks share the same primary performer, lower compilation confidence
    has_single_primary = (
        len(primary_performers) == 1
        and len(tracks) > 1
    )
    if has_single_primary:
        reasons.append(f"Tracks share primary performer: {next(iter(primary_performers))}")

    # ── existing tag signal ──
    if any(track.compilation for track in tracks):
        reasons.append("Existing compilation tag is present")

    # ── decision ──
    confidence = min(1.0, 0.4 * len(reasons))

    # Collaboration overrides compilation
    if is_collaboration:
        is_compilation = False
        confidence = min(1.0, 0.4 * len(reasons))
    # Single primary performer strongly suggests NOT a compilation
    elif has_single_primary and "collaboration" not in " ".join(reasons).lower():
        # Reduce compilation signal — primary performer shared means it's
        # a classical/performer-album, not a compilation
        confidence = max(0.0, confidence - 0.3)
        is_compilation = confidence >= 0.65
    else:
        is_compilation = confidence >= 0.65

    # ── determine suggested album_artist ──
    suggested = _suggest_album_artist(
        tracks, hint, album_artists_set, path_artist,
        is_compilation, is_collaboration, has_single_primary,
    )

    return CompilationAnalysis(
        is_compilation=is_compilation,
        confidence=confidence,
        reasons=reasons,
        suggested_album_artist=suggested,
        is_collaboration=is_collaboration,
    )


def _suggest_album_artist(
    tracks: list[TrackMetadata],
    hint: str,
    album_artists_set: set[str],
    path_artist: str | None,
    is_compilation: bool,
    is_collaboration: bool,
    has_single_primary: bool,
) -> str | None:
    """Suggest the best album_artist based on analysis signals."""
    # Compilation → Various Artists
    if is_compilation:
        return "Various Artists"

    # Collaboration → use the group name from artist field
    if is_collaboration and tracks:
        candidate = tracks[0].artist
        # If artist is a comma-separated list of all members,
        # use only the group name part (before the first comma)
        if candidate and _COMMA_SEPARATED_ARTIST_RE.search(candidate):
            first = _extract_primary_performer(candidate)
            if first:
                return first
        return candidate or "Various Artists"

    # Single primary performer → use that name
    if has_single_primary:
        pp = _extract_primary_performer(tracks[0].artist) if tracks else None
        if pp:
            return pp

    # Fallback: use the first track's album_artist or artist
    if tracks:
        return tracks[0].album_artist or tracks[0].artist

    return None


# ── tag application ───────────────────────────────────────────


def apply_compilation_tags(tracks: list[TrackMetadata]) -> list[TrackMetadata]:
    """Return tracks tagged as a Various Artists compilation.

    Preserves per-track artist information while setting:
    - album_artist = "Various Artists"
    - compilation = True
    """
    return [
        replace(
            track,
            album_artist="Various Artists",
            album_artists=["Various Artists"],
            compilation=True,
        ).normalized()
        for track in tracks
    ]


def apply_smart_album_tags(
    tracks: list[TrackMetadata],
    analysis: CompilationAnalysis,
) -> list[TrackMetadata]:
    """Apply album-level tags based on compilation analysis.

    This is smarter than ``apply_compilation_tags``: it handles collaborations,
    classical albums, and true compilations each according to their pattern.

    * Compilation → forces ``album_artist="Various Artists"``, ``compilation=True``
    * Collaboration → sets ``album_artist`` to group name, ``compilation=False``,
      populates ``artists`` (plural) from the comma-separated ``artist`` field
    * Classical / single-primary → sets ``album_artist`` to primary performer,
      ``compilation=False``
    * Other → preserves existing album_artist, ``compilation=False``
    """
    if analysis.is_compilation:
        return apply_compilation_tags(tracks)

    if analysis.is_collaboration:
        album_artist = analysis.suggested_album_artist or "Various Artists"
        result = []
        for track in tracks:
            # Parse the artist field to populate artists list
            artist_str = track.artist or ""
            parts = [
                p.strip()
                for p in _COMMA_SEPARATED_ARTIST_RE.split(artist_str)
                if p.strip()
            ]
            # If we have a proper group name + members list, the first part is the group
            if len(parts) > 1:
                # Group name is the first part, remaining are individual artists
                group_name = parts[0]
                individual_artists = parts
            else:
                group_name = artist_str
                individual_artists = [artist_str] if artist_str else []

            result.append(
                replace(
                    track,
                    album_artist=album_artist,
                    album_artists=[album_artist],
                    artists=individual_artists,
                    artist=group_name,  # primary = group name
                    compilation=False,
                ).normalized()
            )
        return result

    # Classical / single-primary / default: keep per-track artist,
    # set album_artist from suggestion
    album_artist = analysis.suggested_album_artist
    if album_artist:
        return [
            replace(
                track,
                album_artist=album_artist,
                album_artists=[album_artist],
                compilation=False,
            ).normalized()
            for track in tracks
        ]

    # No strong signal — leave as-is
    return tracks
