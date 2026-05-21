"""Tests for health report models and aggregation."""

from pathlib import Path

from auto_tagger.quality.health import (
    AlbumHealthReport,
    HealthIssue,
    HealthSeverity,
    TrackHealth,
    _artist_album_from_path,
    health_report_paths,
    render_combined_health_report_markdown,
    render_health_report_markdown,
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

    assert report.has_blocking_errors is True
    assert report.summary == {"errors": 1, "warnings": 1, "info": 0}
    assert report.to_dict()["has_blocking_errors"] is True


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

    assert track.has_blocking_errors is False


# ── Path derivation ──────────────────────────────────────────

class TestArtistAlbumFromPath:
    def test_parent_and_leaf(self):
        """Extracts parent as artist, leaf as album."""
        artist, album = _artist_album_from_path(Path("/Music/Adele/21"))
        assert artist == "Adele"
        assert album == "21"

    def test_deep_path(self):
        """Deep paths still use immediate parent and leaf."""
        artist, album = _artist_album_from_path(Path("/a/b/c/d"))
        assert artist == "c"
        assert album == "d"

    def test_single_component(self):
        """Single-component paths fall back to ('_', leaf)."""
        artist, album = _artist_album_from_path(Path("album_dir"))
        assert artist == "_"
        assert album == "album_dir"

    def test_current_dir(self):
        """Dot path falls back to ('_', '_')."""
        artist, album = _artist_album_from_path(Path("."))
        assert artist == "_"
        assert album == "_"

    def test_root(self):
        """Root path falls back to ('_', '_')."""
        artist, album = _artist_album_from_path(Path("/"))
        assert artist == "_"
        assert album == "_"


class TestHealthReportPaths:
    def test_nested_under_report_dir(self):
        """Paths nest as {report_dir}/{artist}/{album}.md/.json."""
        md, js = health_report_paths(Path("/Music/Adele/21"), Path("/tmp/hr"))
        assert md == Path("/tmp/hr/Adele/21.md")
        assert js == Path("/tmp/hr/Adele/21.json")

    def test_special_chars_in_names(self):
        """Special characters in names pass through to the filesystem."""
        md, js = health_report_paths(
            Path("/Music/Taylor Swift/1989 (Taylor's Version)"),
            Path("/tmp/hr"),
        )
        assert md == Path("/tmp/hr/Taylor Swift/1989 (Taylor's Version).md")
        assert js == Path("/tmp/hr/Taylor Swift/1989 (Taylor's Version).json")

    def test_unicode_names(self):
        """Unicode artist/album names pass through."""
        md, js = health_report_paths(
            Path("/Music/蔡健雅/失语者"),
            Path("/tmp/hr"),
        )
        assert md == Path("/tmp/hr/蔡健雅/失语者.md")
        assert js == Path("/tmp/hr/蔡健雅/失语者.json")


# ── Markdown rendering ───────────────────────────────────────

class TestRenderHealthReportMarkdown:
    def test_summary_line(self):
        """Markdown output starts with the summary table."""
        report = AlbumHealthReport(
            album_path=Path("/Music/Adele/21"),
            tracks_checked=2,
            lrc_files_checked=1,
            issues=[],
            track_health=[
                TrackHealth(Path("01.flac"), []),
                TrackHealth(Path("02.flac"), []),
            ],
        )
        md = render_health_report_markdown(report)
        assert "# Health Report: Adele — 21" in md
        assert "| 2 audio / 1 LRC" in md
        assert "✅ no" in md

    def test_album_level_issues_listed(self):
        """Album-level issues appear under an Album-level Issues section."""
        report = AlbumHealthReport(
            album_path=Path("/Music/Adele/21"),
            tracks_checked=1,
            lrc_files_checked=0,
            issues=[
                HealthIssue(
                    "metadata", HealthSeverity.ERROR, None,
                    "metadata.inconsistent_album",
                    "Inconsistent album values across album",
                ),
            ],
            track_health=[TrackHealth(Path("01.flac"), [])],
        )
        md = render_health_report_markdown(report)
        assert "## Album-level Issues" in md
        assert "🔴 ERROR" in md
        assert "metadata.inconsistent_album" in md

    def test_per_track_issues_listed(self):
        """Per-track issues appear under Per-track Issues."""
        report = AlbumHealthReport(
            album_path=Path("/Music/Adele/21"),
            tracks_checked=2,
            lrc_files_checked=0,
            issues=[],
            track_health=[
                TrackHealth(Path("01.flac"), [
                    HealthIssue(
                        "audio", HealthSeverity.WARNING, Path("01.flac"),
                        "audio.missing_duration",
                        "Audio duration is missing or not positive",
                    ),
                ]),
                TrackHealth(Path("02.flac"), []),
            ],
        )
        md = render_health_report_markdown(report)
        assert "## Per-track Issues" in md
        assert "01.flac" in md
        assert "⚠ WARNING" in md
        assert "audio.missing_duration" in md
        assert "Clean Tracks" in md
        assert "✅ 1 track(s) with no issues." in md

    def test_clean_report(self):
        """A clean report shows all-ok markers."""
        report = AlbumHealthReport(
            album_path=Path("/Music/Adele/21"),
            tracks_checked=2,
            lrc_files_checked=2,
            issues=[],
            track_health=[
                TrackHealth(Path("01.flac"), []),
                TrackHealth(Path("02.flac"), []),
            ],
        )
        md = render_health_report_markdown(report)
        assert "✅ no" in md
        assert "✅ 2 track(s) with no issues." in md
        assert "✅ 2 LRC file(s) valid." in md
        assert "## Album-level Issues" not in md
        assert "## Per-track Issues" not in md
        assert "🔴" not in md
        assert "⚠" not in md

    def test_lrc_section_with_issues(self):
        """LRC issues render in their own section."""
        report = AlbumHealthReport(
            album_path=Path("/Music/Adele/21"),
            tracks_checked=1,
            lrc_files_checked=1,
            issues=[
                HealthIssue(
                    "lrc", HealthSeverity.WARNING, Path("01.lrc"),
                    "lrc.non_utf8",
                    "Detected non-UTF-8 encoding: gb18030",
                ),
            ],
            track_health=[TrackHealth(Path("01.flac"), [])],
        )
        md = render_health_report_markdown(report)
        assert "## LRC Files" in md
        assert "01.lrc" in md
        assert "lrc.non_utf8" in md


class TestRenderCombinedHealthReportMarkdown:
    def test_cross_album_summary(self):
        """Combined report has cross-album summary table."""
        reports = [
            {
                "album_path": "/Music/Adele/21",
                "tracks_checked": 2,
                "lrc_files_checked": 0,
                "has_blocking_errors": False,
                "summary": {"errors": 0, "warnings": 1, "info": 0},
                "issues": [],
                "track_health": [],
            },
        ]
        md = render_combined_health_report_markdown(reports, Path("/Music"))
        assert "# Batch Health Report: /Music" in md
        assert "| 0 | 1 |" in md  # albums with errors | total warnings

    def test_per_album_overview(self):
        """Combined report lists each album in a table."""
        reports = [
            {
                "album_path": "/Music/Adele/21",
                "tracks_checked": 2,
                "lrc_files_checked": 0,
                "has_blocking_errors": False,
                "summary": {"errors": 0, "warnings": 0, "info": 0},
                "issues": [],
                "track_health": [],
            },
        ]
        md = render_combined_health_report_markdown(reports, Path("/Music"))
        assert "Adele — 21" in md

    def test_empty_reports(self):
        """Empty reports list produces a valid minimal report."""
        md = render_combined_health_report_markdown([], Path("/Music"))
        assert "Albums checked: 0" in md

    def test_issues_included(self):
        """Albums with issues show their issues in the combined report."""
        reports = [
            {
                "album_path": "/Music/Adele/21",
                "tracks_checked": 1,
                "lrc_files_checked": 0,
                "has_blocking_errors": True,
                "summary": {"errors": 1, "warnings": 0, "info": 0},
                "issues": [
                    {
                        "category": "metadata",
                        "severity": "error",
                        "path": "/Music/Adele/21/01.flac",
                        "code": "metadata.missing_album_artist",
                        "message": "Missing required metadata field: album_artist",
                        "details": {},
                    },
                ],
                "track_health": [],
            },
        ]
        md = render_combined_health_report_markdown(reports, Path("/Music"))
        assert "🔴 ERROR" in md
        assert "metadata.missing_album_artist" in md
        assert "❌" in md

    def test_cross_album_issues_section(self):
        """Cross-album issues render under a Cross-album Issues heading."""
        cross = [
            {
                "category": "cross_album",
                "severity": "warning",
                "path": None,
                "code": "cross_album.inconsistent_album_artist",
                "message": "Inconsistent album_artist across albums in 'Pink Floyd': Pink Floyd, Pink Flyod",
                "details": {
                    "artist_directory": "/Music/Pink Floyd",
                    "values": ["Pink Floyd", "Pink Flyod"],
                    "albums": {"Dark Side": "Pink Floyd", "Wall": "Pink Flyod"},
                },
            },
        ]
        md = render_combined_health_report_markdown(
            [], Path("/Music"), cross_album_issues=cross,
        )
        assert "## Cross-album Issues" in md
        assert "cross_album.inconsistent_album_artist" in md
        assert "Pink Floyd, Pink Flyod" in md
        assert "⚠ WARNING" in md

    def test_cross_album_issues_counted_in_totals(self):
        """Cross-album warnings are reflected in summary totals."""
        cross = [
            {
                "category": "cross_album",
                "severity": "warning",
                "path": None,
                "code": "cross_album.inconsistent_album_artist",
                "message": "Inconsistent album_artist in 'Pink Floyd'",
                "details": {},
            },
        ]
        md = render_combined_health_report_markdown(
            [], Path("/Music"), cross_album_issues=cross,
        )
        # Total warnings should be 1 (from the cross-album issue)
        assert "| 0 | 0 | 1 |" in md  # albums blocked | total errors | total warnings

    def test_no_cross_album_issues_omits_section(self):
        """Without cross-album issues, the section is omitted."""
        md = render_combined_health_report_markdown([], Path("/Music"))
        assert "## Cross-album Issues" not in md
