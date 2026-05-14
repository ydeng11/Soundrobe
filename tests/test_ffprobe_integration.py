"""Real ffprobe integration tests for audio validation."""

from pathlib import Path

import pytest

from auto_tagger.quality.audio_validation import FFProbeValidator

pytestmark = pytest.mark.needs_ffmpeg


def test_ffprobe_accepts_valid_flac(album_fixture: Path):
    """ffprobe validates a properly formed FLAC file."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    validator = FFProbeValidator()
    result = validator.validate(flacs[0])
    assert result.is_valid


def test_ffprobe_reports_corrupt_file(edge_case_fixtures: Path):
    """ffprobe reports issues for a text file disguised as FLAC."""
    corrupt = edge_case_fixtures / "corrupt" / "Bad" / "Broken" / "01.flac"
    validator = FFProbeValidator()
    result = validator.validate(corrupt)
    # Non-audio content produces at least a warning (e.g., missing duration)
    assert len(result.issues) >= 1
    assert result.duration_seconds is None


def test_ffprobe_validates_all_album_tracks(album_fixture: Path):
    """All 11 synthetic FLAC files pass ffprobe validation."""
    validator = FFProbeValidator()
    flacs = sorted(album_fixture.rglob("*.flac"))
    for flac in flacs:
        result = validator.validate(flac)
        assert result.is_valid, f"{flac.name} failed validation: {result.issues}"
