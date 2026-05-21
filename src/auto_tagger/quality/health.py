"""Health report models and aggregation helpers."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
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
    def has_blocking_errors(self) -> bool:
        """Return whether this track has blocking health errors.

        This is a post-hoc diagnostic — not a pre-tagging gate.
        Tags may still have been written successfully even when
        this returns True (e.g. bonus tracks with track_number > track_total).
        """
        return any(issue.severity == HealthSeverity.ERROR for issue in self.issues)

    def to_dict(self) -> dict[str, Any]:
        """Return a JSON-safe representation."""
        return {
            "path": str(self.path),
            "has_blocking_errors": self.has_blocking_errors,
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
    def has_blocking_errors(self) -> bool:
        """Return whether the album has blocking health errors.

        This is a post-hoc diagnostic — not a pre-tagging gate.
        Tags may still have been written successfully even when
        this returns True (e.g. bonus tracks with track_number > track_total).
        """
        return any(issue.severity == HealthSeverity.ERROR for issue in self.issues)

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
            "has_blocking_errors": self.has_blocking_errors,
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
    """Render a health report summary as a Rich table."""
    summary = report.summary
    table = Table(title=f"Health report: {report.album_path.name}")
    table.add_column("Checked")
    table.add_column("Errors")
    table.add_column("Warnings")
    table.add_column("Info")
    table.add_column("Blocking errors")
    table.add_row(
        f"{report.tracks_checked} audio / {report.lrc_files_checked} LRC",
        str(summary["errors"]),
        str(summary["warnings"]),
        str(summary["info"]),
        "yes" if report.has_blocking_errors else "no",
    )
    return table


def _dict_to_issue(i: dict[str, Any]) -> HealthIssue:
    """Convert a JSON-safe dict back to a HealthIssue."""
    return HealthIssue(
        category=i.get("category", ""),
        severity=HealthSeverity(i.get("severity", "info")),
        path=Path(i["path"]) if i.get("path") else None,
        code=i.get("code", ""),
        message=i.get("message", ""),
        details=i.get("details", {}),
    )


def _severity_icon(severity: HealthSeverity) -> str:
    """Return a human-readable icon + label for a severity level."""
    if severity == HealthSeverity.ERROR:
        return "🔴 ERROR"
    if severity == HealthSeverity.WARNING:
        return "⚠ WARNING"
    return "ℹ INFO"


def _artist_album_from_path(album_path: Path) -> tuple[str, str]:
    """Derive (artist, album) from a path for report directory nesting.

    Uses the last two path components: parent name as artist, leaf as album.
    Falls back to ('_', leaf) when there's no useful parent.
    """
    name = album_path.name or "_"
    parent_name = album_path.parent.name
    if parent_name not in ("", "/", "."):
        return parent_name, name
    return "_", name


def health_report_paths(album_path: Path, report_dir: Path) -> tuple[Path, Path]:
    """Return (md_path, json_path) for a report on *album_path* under *report_dir*.

    Nests reports as ``{report_dir}/{artist}/{album}.md`` and
    ``{report_dir}/{artist}/{album}.json`` so different albums never overwrite.
    """
    artist, album = _artist_album_from_path(album_path)
    out = report_dir / artist / album
    return out.with_suffix(".md"), out.with_suffix(".json")


def render_health_report_markdown(report: AlbumHealthReport) -> str:
    """Render a health report as a human-readable Markdown string."""
    artist, album = _artist_album_from_path(report.album_path)
    lines: list[str] = []
    lines.append(f"# Health Report: {artist} — {album}")
    lines.append("")
    lines.append(f"> Generated: {datetime.now(timezone.utc).isoformat()}")
    lines.append("")
    lines.append("## Summary")
    lines.append("")
    s = report.summary
    lines.append("| Checked | Errors | Warnings | Info | Blocking errors |")
    lines.append("|---------|--------|----------|------|-----------------")
    tag_icon = "❌ yes" if report.has_blocking_errors else "✅ no"
    lines.append(f"| {report.tracks_checked} audio / {report.lrc_files_checked} LRC | {s['errors']} | {s['warnings']} | {s['info']} | {tag_icon} |")
    lines.append("")

    # ── Album-level issues ──────────────────────────────────
    album_issues = report.issues
    if album_issues:
        lines.append("## Album-level Issues")
        lines.append("")
        lines.append("| Category | Severity | Code | Message |")
        lines.append("|----------|----------|------|---------|")
        for issue in album_issues:
            lines.append(f"| {issue.category} | {_severity_icon(issue.severity)} | `{issue.code}` | {issue.message} |")
        lines.append("")

    # ── Per-track issues ────────────────────────────────────
    tracks_with_issues = [t for t in report.track_health if t.issues]
    tracks_clean = report.tracks_checked - len(tracks_with_issues)

    if tracks_with_issues:
        lines.append("## Per-track Issues")
        lines.append("")
        for track in tracks_with_issues:
            lines.append(f"### {track.path.name}")
            lines.append("")
            lines.append("| Category | Severity | Code | Message |")
            lines.append("|----------|----------|------|---------|")
            for issue in track.issues:
                lines.append(f"| {issue.category} | {_severity_icon(issue.severity)} | `{issue.code}` | {issue.message} |")
            lines.append("")

    if tracks_clean:
        lines.append("### Clean Tracks")
        lines.append("")
        lines.append(f"✅ {tracks_clean} track(s) with no issues.")
        lines.append("")

    # ── LRC files ───────────────────────────────────────────
    lrc_issues = [i for i in album_issues if i.category == "lrc"]
    if lrc_issues:
        lines.append("## LRC Files")
        lines.append("")
        lines.append("| File | Severity | Code | Message |")
        lines.append("|------|----------|------|---------|")
        for issue in lrc_issues:
            fn = issue.path.name if issue.path else "?"
            lines.append(f"| {fn} | {_severity_icon(issue.severity)} | `{issue.code}` | {issue.message} |")
        lines.append("")
    elif report.lrc_files_checked:
        lines.append("## LRC Files")
        lines.append("")
        lines.append(f"✅ {report.lrc_files_checked} LRC file(s) valid.")
        lines.append("")

    return "\n".join(lines) + "\n"


def render_combined_health_report_markdown(
    album_reports: list[dict[str, Any]],
    library_path: Path,
    cross_album_issues: list[dict[str, Any]] | None = None,
) -> str:
    """Render a combined batch health report as Markdown.

    *album_reports* items are the ``to_dict()`` output from each album.

    *cross_album_issues* are ``to_dict()`` outputs from the cross-album
    consistency check, e.g. inconsistent album_artist across albums
    under the same artist directory.
    """
    lines: list[str] = []
    lines.append(f"# Batch Health Report: {library_path}")
    lines.append("")
    lines.append(f"> Generated: {datetime.now(timezone.utc).isoformat()}")
    lines.append(f"> Albums checked: {len(album_reports)}")
    lines.append("")

    # ── Cross-album summary ────────────────────────────────
    albums_with_errors = sum(1 for r in album_reports if r.get("has_blocking_errors", False))
    total_errors = sum(r["summary"]["errors"] for r in album_reports)
    total_warnings = sum(r["summary"]["warnings"] for r in album_reports)

    # Add cross-album issues to totals
    if cross_album_issues:
        for issue in cross_album_issues:
            sev = issue.get("severity", "info")
            if sev == "error":
                total_errors += 1
            elif sev == "warning":
                total_warnings += 1

    lines.append("## Cross-album Summary")
    lines.append("")
    lines.append("| Albums with errors | Total errors | Total warnings |")
    lines.append("|---------------|--------------|----------------|")
    lines.append(f"| {albums_with_errors} | {total_errors} | {total_warnings} |")
    lines.append("")

    # ── Cross-album issues section ──────────────────────────
    if cross_album_issues:
        lines.append("## Cross-album Issues")
        lines.append("")
        lines.append("| Category | Severity | Code | Message |")
        lines.append("|----------|----------|------|---------|")
        for issue in cross_album_issues:
            sev = issue.get("severity", "info")
            icon = _severity_icon(HealthSeverity(sev))
            lines.append(f"| {issue.get('category', '')} | {icon} | `{issue.get('code', '')}` | {issue.get('message', '')} |")
        lines.append("")

    # ── Per-album summary table ─────────────────────────────
    lines.append("## Per-album Overview")
    lines.append("")
    lines.append("| Artist / Album | Errors | Warnings | Blocking errors |")
    lines.append("|----------------|--------|----------|---------|")
    for report_dict in sorted(album_reports, key=lambda r: r.get("album_path", "")):
        ap = Path(report_dict.get("album_path", ""))
        artist, album_name = _artist_album_from_path(ap)
        label = f"{artist} — {album_name}"
        s = report_dict.get("summary", {})
        errs = s.get("errors", 0)
        warns = s.get("warnings", 0)
        has_errors = report_dict.get("has_blocking_errors", False)
        tag_icon = "❌" if has_errors else "✅"
        lines.append(f"| {label} | {errs} | {warns} | {tag_icon} |")
    lines.append("")

    # ── Per-album details ───────────────────────────────────
    for report_dict in album_reports:
        ap = Path(report_dict.get("album_path", ""))
        artist, album_name = _artist_album_from_path(ap)
        s = report_dict.get("summary", {})
        issues = report_dict.get("issues", [])
        track_health = report_dict.get("track_health", [])
        has_errors = report_dict.get("has_blocking_errors", False)
        tag_icon = "❌" if has_errors else "✅"

        lines.append("---")
        lines.append("")
        lines.append(f"### {tag_icon} {artist} — {album_name}")
        lines.append("")
        lines.append("| Errors | Warnings | Info | Tracks checked |")
        lines.append("|--------|----------|------|----------------|")
        lines.append(f"| {s.get('errors', 0)} | {s.get('warnings', 0)} | {s.get('info', 0)} | {report_dict.get('tracks_checked', 0)} |")
        lines.append("")

        if issues:
            lines.append("| Category | Severity | Code | Message |")
            lines.append("|----------|----------|------|---------|")
            for issue in issues:
                sev = issue.get("severity", "info")
                icon = _severity_icon(HealthSeverity(sev))
                lines.append(f"| {issue.get('category', '')} | {icon} | `{issue.get('code', '')}` | {issue.get('message', '')} |")
            lines.append("")

        tracks_with = [t for t in track_health if t.get("issues")]
        if tracks_with:
            for t in tracks_with:
                lines.append(f"  - {Path(t.get('path', '')).name}: {len(t.get('issues', []))} issue(s)")
            lines.append("")

    return "\n".join(lines) + "\n"


def report_dict_to_markdown(report_dict: dict, album_path: Path) -> str:
    """Render a ``to_dict()`` output as Markdown via ``render_health_report_markdown``."""
    issues = [_dict_to_issue(i) for i in report_dict.get("issues", [])]

    track_health: list[TrackHealth] = []
    for t in report_dict.get("track_health", []):
        t_issues = [_dict_to_issue(i) for i in t.get("issues", [])]
        track_health.append(TrackHealth(path=Path(t["path"]), issues=t_issues))

    report = AlbumHealthReport(
        album_path=album_path,
        tracks_checked=report_dict.get("tracks_checked", 0),
        lrc_files_checked=report_dict.get("lrc_files_checked", 0),
        issues=issues,
        track_health=track_health,
    )
    return render_health_report_markdown(report)
