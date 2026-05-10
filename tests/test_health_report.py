"""Tests for health report models and aggregation."""

from pathlib import Path

from auto_tagger.quality.health import (
    AlbumHealthReport,
    HealthIssue,
    HealthSeverity,
    TrackHealth,
)


def test_health_issue_serializes_plain_values(tmp_path: Path):
    """Health issues serialize paths, enums, and details to JSON-safe values."""
    issue = HealthIssue(
        category="audio",
        severity=HealthSeverity.ERROR,
        path=tmp_path / "01.flac",
        code="audio.unreadable",
        message="File could not be decoded",
        details={"exit_code": 1},
    )

    assert issue.to_dict() == {
        "category": "audio",
        "severity": "error",
        "path": str(tmp_path / "01.flac"),
        "code": "audio.unreadable",
        "message": "File could not be decoded",
        "details": {"exit_code": 1},
    }


def test_album_health_report_summarizes_errors_and_warnings(tmp_path: Path):
    """Album reports summarize issue counts and block tagging on errors."""
    report = AlbumHealthReport(
        album_path=tmp_path,
        tracks_checked=2,
        lrc_files_checked=1,
        issues=[
            HealthIssue("audio", HealthSeverity.ERROR, tmp_path / "bad.flac", "bad", "Bad file"),
            HealthIssue("lrc", HealthSeverity.WARNING, tmp_path / "song.lrc", "warn", "Warn"),
        ],
        track_health=[
            TrackHealth(
                path=tmp_path / "bad.flac",
                issues=[
                    HealthIssue(
                        "audio",
                        HealthSeverity.ERROR,
                        tmp_path / "bad.flac",
                        "bad",
                        "Bad file",
                    )
                ],
            )
        ],
    )

    assert report.can_tag is False
    assert report.summary == {"errors": 1, "warnings": 1, "info": 0}
    assert report.to_dict()["can_tag"] is False


def test_track_health_allows_tagging_without_error(tmp_path: Path):
    """Track health only blocks tagging when an error issue is attached."""
    track = TrackHealth(
        path=tmp_path / "01.flac",
        issues=[
            HealthIssue(
                "metadata",
                HealthSeverity.WARNING,
                tmp_path / "01.flac",
                "metadata.track_gap",
                "Track sequence has a gap",
            )
        ],
    )

    assert track.can_tag is True
