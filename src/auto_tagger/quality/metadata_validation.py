"""Metadata completeness and consistency validation."""

from __future__ import annotations

import re
from collections import Counter, defaultdict
from pathlib import Path

from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.quality.health import HealthIssue, HealthSeverity

GAIN_RE = re.compile(r"^[+-]?\d+(?:\.\d+)?\s+dB$")
PEAK_RE = re.compile(r"^(?:0(?:\.\d+)?|1(?:\.0+)?)$")


def validate_track_metadata(path: Path, metadata: TrackMetadata) -> list[HealthIssue]:
    """Validate one track's normalized metadata."""
    issues: list[HealthIssue] = []
    required = {
        "title": metadata.title,
        "artist": metadata.artist,
        "album": metadata.album,
        "album_artist": metadata.album_artist,
        "track_number": metadata.track_number,
    }
    for field_name, value in required.items():
        if value is None or (isinstance(value, str) and not value.strip()):
            issues.append(
                HealthIssue(
                    "metadata",
                    HealthSeverity.ERROR,
                    path,
                    f"metadata.missing_{field_name}",
                    f"Missing required metadata field: {field_name}",
                )
            )

    _validate_position(
        issues,
        path,
        metadata.track_number,
        metadata.track_total,
        "track",
    )
    _validate_position(
        issues,
        path,
        metadata.disc_number,
        metadata.disc_total,
        "disc",
    )
    _validate_replaygain(issues, path, metadata)
    return issues


def validate_album_metadata(metadata_by_path: dict[Path, TrackMetadata]) -> list[HealthIssue]:
    """Validate album-level consistency across tracks."""
    if not metadata_by_path:
        return []

    issues: list[HealthIssue] = []
    metadatas = list(metadata_by_path.items())
    _check_consistent_text(issues, metadatas, "album")
    _check_consistent_text(issues, metadatas, "album_artist")
    _check_duplicate_tracks(issues, metadatas)
    _check_track_gaps(issues, metadatas)
    _check_total_consistency(issues, metadatas, "track_total")
    _check_total_consistency(issues, metadatas, "disc_total")
    return issues


def _validate_position(
    issues: list[HealthIssue],
    path: Path,
    number: int | None,
    total: int | None,
    name: str,
) -> None:
    if number is not None and number <= 0:
        issues.append(
            HealthIssue(
                "metadata",
                HealthSeverity.ERROR,
                path,
                f"metadata.invalid_{name}_number",
                f"{name.title()} number must be positive",
            )
        )
    if total is not None and total <= 0:
        issues.append(
            HealthIssue(
                "metadata",
                HealthSeverity.ERROR,
                path,
                f"metadata.invalid_{name}_total",
                f"{name.title()} total must be positive",
            )
        )
    if number is not None and total is not None and number > total:
        issues.append(
            HealthIssue(
                "metadata",
                HealthSeverity.ERROR,
                path,
                f"metadata.{name}_exceeds_total",
                f"{name.title()} number exceeds {name} total",
            )
        )


def _validate_replaygain(
    issues: list[HealthIssue],
    path: Path,
    metadata: TrackMetadata,
) -> None:
    for value in (metadata.replaygain.track_gain, metadata.replaygain.album_gain):
        if value and not GAIN_RE.match(value):
            issues.append(
                HealthIssue(
                    "metadata",
                    HealthSeverity.WARNING,
                    path,
                    "metadata.invalid_replaygain_gain",
                    f"ReplayGain gain should include dB suffix: {value}",
                )
            )
    for value in (metadata.replaygain.track_peak, metadata.replaygain.album_peak):
        if value and not PEAK_RE.match(value):
            issues.append(
                HealthIssue(
                    "metadata",
                    HealthSeverity.WARNING,
                    path,
                    "metadata.invalid_replaygain_peak",
                    f"ReplayGain peak should be a float between 0 and 1: {value}",
                )
            )


def _check_consistent_text(
    issues: list[HealthIssue],
    metadatas: list[tuple[Path, TrackMetadata]],
    field_name: str,
) -> None:
    values = {
        str(getattr(metadata, field_name)).strip()
        for _, metadata in metadatas
        if getattr(metadata, field_name)
    }
    if len(values) > 1:
        issues.append(
            HealthIssue(
                "metadata",
                HealthSeverity.ERROR,
                None,
                f"metadata.inconsistent_{field_name}",
                f"Inconsistent {field_name} values across album",
                {"values": sorted(values)},
            )
        )


def _check_duplicate_tracks(
    issues: list[HealthIssue],
    metadatas: list[tuple[Path, TrackMetadata]],
) -> None:
    paths_by_position: dict[tuple[int, int], list[str]] = defaultdict(list)
    for path, metadata in metadatas:
        if metadata.track_number is None:
            continue
        disc = metadata.disc_number or 1
        paths_by_position[(disc, metadata.track_number)].append(str(path))

    for (disc, track), paths in paths_by_position.items():
        if len(paths) > 1:
            issues.append(
                HealthIssue(
                    "metadata",
                    HealthSeverity.ERROR,
                    None,
                    "metadata.duplicate_track_number",
                    f"Duplicate track number {track} on disc {disc}",
                    {"paths": paths},
                )
            )


def _check_track_gaps(
    issues: list[HealthIssue],
    metadatas: list[tuple[Path, TrackMetadata]],
) -> None:
    tracks_by_disc: dict[int, list[int]] = defaultdict(list)
    for _, metadata in metadatas:
        if metadata.track_number is not None:
            tracks_by_disc[metadata.disc_number or 1].append(metadata.track_number)

    for disc, tracks in tracks_by_disc.items():
        unique_tracks = sorted(set(tracks))
        if not unique_tracks:
            continue
        expected = set(range(1, max(unique_tracks) + 1))
        missing = sorted(expected - set(unique_tracks))
        if missing:
            issues.append(
                HealthIssue(
                    "metadata",
                    HealthSeverity.WARNING,
                    None,
                    "metadata.track_sequence_gap",
                    f"Track sequence has gaps on disc {disc}",
                    {"disc": disc, "missing": missing},
                )
            )


def _check_total_consistency(
    issues: list[HealthIssue],
    metadatas: list[tuple[Path, TrackMetadata]],
    field_name: str,
) -> None:
    values = [
        getattr(metadata, field_name)
        for _, metadata in metadatas
        if getattr(metadata, field_name) is not None
    ]
    if len(set(values)) > 1:
        counts = Counter(values)
        issues.append(
            HealthIssue(
                "metadata",
                HealthSeverity.WARNING,
                None,
                f"metadata.inconsistent_{field_name}",
                f"Inconsistent {field_name} values across album",
                {"values": dict(counts)},
            )
        )


def check_cross_album_artist_consistency(
    albums: list[tuple[Path, str | None]],
) -> list[HealthIssue]:
    """Check album_artist consistency across albums in the same artist directory.

    Groups albums by their parent directory (artist folder). If a directory
    contains albums with multiple different album_artist values, an issue is
    raised for that directory. Albums with ``None`` album_artist are excluded
    from comparison (no data yet) but still listed in the per-album breakdown.

    Args:
        albums: List of ``(album_path, album_artist)`` tuples, one per album
            in the library.

    Returns:
        A list of cross-album ``HealthIssue`` instances, one per artist
        directory with inconsistencies.
    """
    issues: list[HealthIssue] = []

    # Group by parent directory (artist folder)
    by_parent: dict[Path, list[tuple[Path, str | None]]] = defaultdict(list)
    for album_path, artist_name in albums:
        by_parent[album_path.parent].append((album_path, artist_name))

    for parent_dir, album_list in by_parent.items():
        unique_artists: set[str] = set()
        for album_path, artist_name in album_list:
            if artist_name and artist_name.strip():
                unique_artists.add(artist_name.strip())

        if len(unique_artists) <= 1:
            continue

        albums_detail = {
            ap.name: (an or "(missing)")
            for ap, an in album_list
        }
        issues.append(
            HealthIssue(
                "cross_album",
                HealthSeverity.WARNING,
                None,
                "cross_album.inconsistent_album_artist",
                f"Inconsistent album_artist across albums in '{parent_dir.name}': "
                f"{', '.join(sorted(unique_artists))}",
                {
                    "artist_directory": str(parent_dir),
                    "values": sorted(unique_artists),
                    "albums": dict(sorted(albums_detail.items())),
                },
            )
        )

    return issues
