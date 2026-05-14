"""Health report models and aggregation helpers."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path
from typing import Any

from rich.table import Table

from auto_tagger.config import Settings
from auto_tagger.core.metadata import TrackMetadata


class HealthSeverity(str, Enum):
    """Severity for health report issues."""

    INFO = "info"
    WARNING = "warning"
    ERROR = "error"


@dataclass(frozen=True)
class HealthIssue:
    """A structured issue found during validation."""

    category: str
    severity: HealthSeverity
    path: Path | None
    code: str
    message: str
    details: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        """Return a JSON-safe representation."""
        return {
            "category": self.category,
            "severity": self.severity.value,
            "path": str(self.path) if self.path is not None else None,
            "code": self.code,
            "message": self.message,
            "details": self.details,
        }


@dataclass(frozen=True)
class TrackHealth:
    """Health issues for one audio file."""

    path: Path
    issues: list[HealthIssue] = field(default_factory=list)

    @property
    def can_tag(self) -> bool:
        """Return whether this track has no blocking health errors."""
        return not any(issue.severity == HealthSeverity.ERROR for issue in self.issues)

    def to_dict(self) -> dict[str, Any]:
        """Return a JSON-safe representation."""
        return {
            "path": str(self.path),
            "can_tag": self.can_tag,
            "issues": [issue.to_dict() for issue in self.issues],
        }


@dataclass(frozen=True)
class AlbumHealthReport:
    """Aggregate health report for an album or directory."""

    album_path: Path
    tracks_checked: int
    lrc_files_checked: int
    issues: list[HealthIssue] = field(default_factory=list)
    track_health: list[TrackHealth] = field(default_factory=list)

    @property
    def can_tag(self) -> bool:
        """Return whether the album has no blocking health errors."""
        return not any(issue.severity == HealthSeverity.ERROR for issue in self.issues)

    @property
    def summary(self) -> dict[str, int]:
        """Return issue counts by severity."""
        counts = {"errors": 0, "warnings": 0, "info": 0}
        for issue in self.issues:
            if issue.severity == HealthSeverity.ERROR:
                counts["errors"] += 1
            elif issue.severity == HealthSeverity.WARNING:
                counts["warnings"] += 1
            else:
                counts["info"] += 1
        return counts

    def to_dict(self) -> dict[str, Any]:
        """Return a JSON-safe representation."""
        return {
            "album_path": str(self.album_path),
            "tracks_checked": self.tracks_checked,
            "lrc_files_checked": self.lrc_files_checked,
            "can_tag": self.can_tag,
            "summary": self.summary,
            "issues": [issue.to_dict() for issue in self.issues],
            "track_health": [track.to_dict() for track in self.track_health],
        }


def build_album_health_report(
    album_path: Path,
    audio_files: list[Path],
    metadata_by_path: dict[Path, TrackMetadata],
    settings: Settings,
) -> AlbumHealthReport:
    """Build a health report by running Phase 5 validators."""
    from auto_tagger.features.cover_art import discover_local_cover_art
    from auto_tagger.quality.audio_validation import FFProbeValidator
    from auto_tagger.quality.lrc import discover_lrc_files, validate_lrc_file
    from auto_tagger.quality.metadata_validation import (
        validate_album_metadata,
        validate_track_metadata,
    )

    issues: list[HealthIssue] = []
    track_health: list[TrackHealth] = []
    audio_validator = FFProbeValidator(
        ffprobe_path=settings.ffprobe_path,
        timeout_seconds=settings.ffprobe_timeout_seconds,
    )

    for audio_file in audio_files:
        track_issues: list[HealthIssue] = []
        audio_result = audio_validator.validate(audio_file)
        track_issues.extend(audio_result.issues)

        metadata = metadata_by_path.get(audio_file)
        if metadata is not None:
            track_issues.extend(validate_track_metadata(audio_file, metadata))

        issues.extend(track_issues)
        track_health.append(TrackHealth(audio_file, track_issues))

    album_issues = validate_album_metadata(metadata_by_path)
    issues.extend(album_issues)

    lrc_files = discover_lrc_files(album_path, audio_files)
    for lrc_file in lrc_files:
        lrc_result = validate_lrc_file(lrc_file)
        issues.extend(lrc_result.issues)

    # Cover art check — prefer album-name cover, then generic names
    album_name = next(
        (m.album for m in metadata_by_path.values() if m.album), None
    )
    cover = discover_local_cover_art(album_path, album_name)
    if cover is None:
        issues.append(
            HealthIssue(
                "cover_art",
                HealthSeverity.WARNING,
                album_path,
                "missing_local",
                "No local cover art found (cover.jpg, folder.jpg, front.jpg, etc.)",
            )
        )

    return AlbumHealthReport(
        album_path=album_path,
        tracks_checked=len(audio_files),
        lrc_files_checked=len(lrc_files),
        issues=issues,
        track_health=track_health,
    )


def render_health_report(report: AlbumHealthReport) -> Table:
    """Render a health report as a Rich table."""
    summary = report.summary
    table = Table(title="Health report")
    table.add_column("Checked")
    table.add_column("Errors")
    table.add_column("Warnings")
    table.add_column("Info")
    table.add_column("Can tag")
    table.add_row(
        f"{report.tracks_checked} audio / {report.lrc_files_checked} LRC",
        str(summary["errors"]),
        str(summary["warnings"]),
        str(summary["info"]),
        "yes" if report.can_tag else "no",
    )
    return table
