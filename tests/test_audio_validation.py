"""Tests for ffprobe-backed audio validation."""

from pathlib import Path

from auto_tagger.quality.audio_validation import FFProbeValidator
from auto_tagger.quality.health import HealthSeverity


class FakeRunner:
    """Callable ffprobe runner for tests."""

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


def test_ffprobe_validator_accepts_audio_stream(tmp_path: Path):
    """Valid ffprobe JSON with an audio stream returns a taggable result."""
    audio = tmp_path / "01.flac"
    audio.touch()
    runner = FakeRunner(
        stdout=(
            '{"streams":[{"codec_type":"audio","codec_name":"flac","duration":"12.5"}],'
            '"format":{"format_name":"flac","duration":"12.5"}}'
        )
    )

    result = FFProbeValidator(ffprobe_path="ffprobe", timeout_seconds=3, runner=runner).validate(
        audio
    )

    assert result.is_valid is True
    assert result.duration_seconds == 12.5
    assert result.codec_name == "flac"
    assert result.issues == []
    assert runner.calls[0][0][-1] == str(audio)
    assert runner.calls[0][1] == 3


def test_ffprobe_validator_reports_nonzero_exit(tmp_path: Path):
    """A failed ffprobe run produces an error issue and invalid result."""
    audio = tmp_path / "bad.flac"
    audio.touch()
    runner = FakeRunner(stderr="Invalid data found", returncode=1)

    result = FFProbeValidator(runner=runner).validate(audio)

    assert result.is_valid is False
    assert result.issues[0].severity == HealthSeverity.ERROR
    assert result.issues[0].code == "audio.ffprobe_failed"
    assert "Invalid data" in result.issues[0].message


def test_ffprobe_validator_reports_missing_binary(tmp_path: Path):
    """Missing ffprobe binary is reported as a warning so dry-run can continue."""
    audio = tmp_path / "01.flac"
    audio.touch()

    def missing_runner(command: list[str], timeout: int):
        raise FileNotFoundError("ffprobe")

    result = FFProbeValidator(runner=missing_runner).validate(audio)

    assert result.is_valid is True
    assert result.issues[0].severity == HealthSeverity.WARNING
    assert result.issues[0].code == "audio.ffprobe_missing"


def test_ffprobe_validator_reports_no_audio_stream(tmp_path: Path):
    """ffprobe output without audio streams is an error."""
    audio = tmp_path / "image.mp3"
    audio.touch()
    runner = FakeRunner(stdout='{"streams":[{"codec_type":"video"}],"format":{}}')

    result = FFProbeValidator(runner=runner).validate(audio)

    assert result.is_valid is False
    assert result.issues[0].code == "audio.no_audio_stream"
