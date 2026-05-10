"""Tests for ReplayGain calculation orchestration."""

from pathlib import Path

from auto_tagger.core.metadata import ReplayGainTags, TrackMetadata
from auto_tagger.quality.replaygain import (
    ReplayGainCalculator,
    ReplayGainResult,
    apply_replaygain_tags,
)


class FakeReplayGainRunner:
    """Callable command runner for ReplayGain tests."""

    def __init__(self, stdout: str = "", stderr: str = "", returncode: int = 0):
        self.stdout = stdout
        self.stderr = stderr
        self.returncode = returncode
        self.calls: list[tuple[list[str], int]] = []

    def __call__(self, command: list[str], timeout: int):
        self.calls.append((command, timeout))
        return type(
            "Completed",
            (),
            {"stdout": self.stdout, "stderr": self.stderr, "returncode": self.returncode},
        )()


def test_replaygain_calculator_parses_json_output(tmp_path: Path):
    """ReplayGain calculator normalizes JSON command output by file path."""
    audio = tmp_path / "01.flac"
    audio.touch()
    runner = FakeReplayGainRunner(
        stdout=(
            '{"files":[{"path":"'
            + str(audio)
            + '","track_gain":"-6.84 dB","track_peak":"0.987654",'
            '"album_gain":"-7.10 dB","album_peak":"0.999999"}]}'
        )
    )

    results = ReplayGainCalculator(command="rgain3", runner=runner).calculate_album([audio])

    assert results[audio].tags == ReplayGainTags(
        track_gain="-6.84 dB",
        track_peak="0.987654",
        album_gain="-7.10 dB",
        album_peak="0.999999",
    )
    assert runner.calls[0][0][0] == "rgain3"


def test_replaygain_calculator_reports_command_failure(tmp_path: Path):
    """ReplayGain command failures produce per-file issues."""
    audio = tmp_path / "01.flac"
    audio.touch()
    runner = FakeReplayGainRunner(stderr="boom", returncode=2)

    results = ReplayGainCalculator(runner=runner).calculate_album([audio])

    assert results[audio].tags.is_empty()
    assert results[audio].issues[0].code == "replaygain.command_failed"
    assert "boom" in results[audio].issues[0].message


def test_replaygain_calculator_reports_missing_command(tmp_path: Path):
    """Missing ReplayGain binary produces a warning result for each path."""
    audio = tmp_path / "01.flac"
    audio.touch()

    def missing_runner(command: list[str], timeout: int):
        raise FileNotFoundError("rgain3")

    results = ReplayGainCalculator(runner=missing_runner).calculate_album([audio])

    assert results[audio].issues[0].code == "replaygain.command_missing"


def test_apply_replaygain_tags_preserves_other_metadata_in_dry_run(tmp_path: Path):
    """Dry-run application returns metadata updates without writing files."""
    audio = tmp_path / "01.flac"
    existing = TrackMetadata(title="Song", artist="Artist")
    result = ReplayGainResult(
        path=audio,
        tags=ReplayGainTags(track_gain="-1.00 dB", track_peak="0.5"),
    )

    updated = apply_replaygain_tags({audio: result}, {audio: existing}, dry_run=True)

    assert updated[audio].title == "Song"
    assert updated[audio].replaygain.track_gain == "-1.00 dB"
