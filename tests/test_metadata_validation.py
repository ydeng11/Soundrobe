"""Tests for metadata quality validation."""

from pathlib import Path

from auto_tagger.core.metadata import ReplayGainTags, TrackMetadata
from auto_tagger.quality.metadata_validation import validate_album_metadata, validate_track_metadata


def complete_metadata(**overrides) -> TrackMetadata:
    """Build valid test metadata with optional overrides."""
    values = {
        "title": "Song",
        "artist": "Artist",
        "album": "Album",
        "album_artist": "Artist",
        "track_number": 1,
        "track_total": 2,
        "disc_number": 1,
        "disc_total": 1,
    }
    values.update(overrides)
    return TrackMetadata(**values)


def test_validate_track_metadata_accepts_complete_metadata(tmp_path: Path):
    """Complete Navidrome-oriented metadata produces no issues."""
    assert validate_track_metadata(tmp_path / "01.flac", complete_metadata()) == []


def test_validate_track_metadata_reports_missing_required_fields(tmp_path: Path):
    """Missing required fields are focused metadata errors."""
    metadata = complete_metadata(title=None, album_artist=None, track_number=None)

    issues = validate_track_metadata(tmp_path / "01.flac", metadata)

    assert [issue.code for issue in issues] == [
        "metadata.missing_title",
        "metadata.missing_album_artist",
        "metadata.missing_track_number",
    ]


def test_validate_track_metadata_reports_invalid_positions(tmp_path: Path):
    """Invalid track and disc numbers are errors."""
    metadata = complete_metadata(track_number=3, track_total=2, disc_number=0)

    issues = validate_track_metadata(tmp_path / "01.flac", metadata)

    assert "metadata.track_exceeds_total" in {issue.code for issue in issues}
    assert "metadata.invalid_disc_number" in {issue.code for issue in issues}


def test_validate_track_metadata_warns_for_bad_replaygain_shape(tmp_path: Path):
    """Malformed ReplayGain values are warnings instead of hard blockers."""
    metadata = complete_metadata(
        replaygain=ReplayGainTags(track_gain="-6.2", track_peak="loud")
    )

    issues = validate_track_metadata(tmp_path / "01.flac", metadata)

    assert {issue.code for issue in issues} == {
        "metadata.invalid_replaygain_gain",
        "metadata.invalid_replaygain_peak",
    }


def test_validate_album_metadata_reports_inconsistent_album_fields(tmp_path: Path):
    """Album-level validation catches inconsistent album data and track gaps."""
    tracks = {
        tmp_path / "01.flac": complete_metadata(track_number=1, album="Album"),
        tmp_path / "03.flac": complete_metadata(track_number=3, album="Other"),
    }

    issues = validate_album_metadata(tracks)

    assert "metadata.inconsistent_album" in {issue.code for issue in issues}
    assert "metadata.track_sequence_gap" in {issue.code for issue in issues}


def test_validate_album_metadata_reports_duplicate_track_numbers(tmp_path: Path):
    """Duplicate track numbers on the same disc are reported."""
    tracks = {
        tmp_path / "a.flac": complete_metadata(title="A", track_number=1),
        tmp_path / "b.flac": complete_metadata(title="B", track_number=1),
    }

    issues = validate_album_metadata(tracks)

    assert "metadata.duplicate_track_number" in {issue.code for issue in issues}
