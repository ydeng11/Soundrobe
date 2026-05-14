"""Real ReplayGain calculation integration tests.

When rgain3/loudgain is not installed, calculate_album returns warning
issues instead of raising. These tests verify both paths.
"""

from pathlib import Path

from auto_tagger.quality.replaygain import ReplayGainCalculator


def test_replaygain_calculator_handles_missing_command(album_fixture: Path):
    """When rgain3/loudgain is missing, calculate_album returns warnings."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    calculator = ReplayGainCalculator(command="rgain3")  # may not be installed
    results = calculator.calculate_album(flacs)

    assert len(results) == len(flacs)
    for path, result in results.items():
        assert path in flacs
        # Either we got valid tags or a warning about missing command
        assert result.tags is not None
        assert result.tags.track_gain is not None or len(result.issues) >= 1


def test_replaygain_calculator_loudgain_also_handled(album_fixture: Path):
    """loudgain (alternative command) also returns results."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    calculator = ReplayGainCalculator(command="loudgain")  # may not be installed
    results = calculator.calculate_album(flacs)
    assert len(results) == len(flacs)


def test_replaygain_calculator_empty_paths():
    """Empty path list returns empty dict."""
    calculator = ReplayGainCalculator()
    results = calculator.calculate_album([])
    assert results == {}
