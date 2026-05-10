"""ffprobe-backed audio file validation."""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Protocol

from auto_tagger.quality.health import HealthIssue, HealthSeverity


class CommandResult(Protocol):
    """Subset of subprocess result fields used by validators."""

    stdout: str
    stderr: str
    returncode: int


class CommandRunner(Protocol):
    """Callable command runner protocol."""

    def __call__(self, command: list[str], timeout: int) -> CommandResult:
        """Run a command and return a completed process-like object."""


@dataclass(frozen=True)
class AudioValidationResult:
    """Validation result for one audio file."""

    path: Path
    duration_seconds: float | None = None
    format_name: str | None = None
    codec_name: str | None = None
    issues: list[HealthIssue] = field(default_factory=list)

    @property
    def is_valid(self) -> bool:
        """Return whether no blocking audio validation errors were found."""
        return not any(issue.severity == HealthSeverity.ERROR for issue in self.issues)


def _default_runner(command: list[str], timeout: int) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        capture_output=True,
        check=False,
        text=True,
        timeout=timeout,
    )


class FFProbeValidator:
    """Validate audio structure through an injectable ffprobe boundary."""

    def __init__(
        self,
        ffprobe_path: str = "ffprobe",
        timeout_seconds: int = 20,
        runner: CommandRunner = _default_runner,
    ):
        self.ffprobe_path = ffprobe_path
        self.timeout_seconds = timeout_seconds
        self.runner = runner

    def validate(self, path: Path) -> AudioValidationResult:
        """Validate one audio file and return structured issues."""
        command = [
            self.ffprobe_path,
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-print_format",
            "json",
            str(path),
        ]
        try:
            completed = self.runner(command, self.timeout_seconds)
        except FileNotFoundError:
            return AudioValidationResult(
                path=path,
                issues=[
                    HealthIssue(
                        "audio",
                        HealthSeverity.WARNING,
                        path,
                        "audio.ffprobe_missing",
                        f"ffprobe command not found: {self.ffprobe_path}",
                    )
                ],
            )
        except subprocess.TimeoutExpired:
            return AudioValidationResult(
                path=path,
                issues=[
                    HealthIssue(
                        "audio",
                        HealthSeverity.ERROR,
                        path,
                        "audio.ffprobe_timeout",
                        f"ffprobe timed out after {self.timeout_seconds} seconds",
                    )
                ],
            )

        if completed.returncode != 0:
            message = completed.stderr.strip() or "ffprobe failed"
            return AudioValidationResult(
                path=path,
                issues=[
                    HealthIssue(
                        "audio",
                        HealthSeverity.ERROR,
                        path,
                        "audio.ffprobe_failed",
                        message,
                        {"returncode": completed.returncode},
                    )
                ],
            )

        try:
            payload = json.loads(completed.stdout or "{}")
        except json.JSONDecodeError:
            return AudioValidationResult(
                path=path,
                issues=[
                    HealthIssue(
                        "audio",
                        HealthSeverity.ERROR,
                        path,
                        "audio.invalid_ffprobe_json",
                        "ffprobe returned invalid JSON",
                    )
                ],
            )

        streams = payload.get("streams") or []
        audio_stream = next(
            (stream for stream in streams if stream.get("codec_type") == "audio"),
            None,
        )
        if audio_stream is None:
            return AudioValidationResult(
                path=path,
                issues=[
                    HealthIssue(
                        "audio",
                        HealthSeverity.ERROR,
                        path,
                        "audio.no_audio_stream",
                        "ffprobe found no audio stream",
                    )
                ],
            )

        format_data = payload.get("format") or {}
        duration = _parse_duration(audio_stream.get("duration")) or _parse_duration(
            format_data.get("duration")
        )
        issues: list[HealthIssue] = []
        if duration is None or duration <= 0:
            issues.append(
                HealthIssue(
                    "audio",
                    HealthSeverity.WARNING,
                    path,
                    "audio.missing_duration",
                    "Audio duration is missing or not positive",
                )
            )

        return AudioValidationResult(
            path=path,
            duration_seconds=duration,
            format_name=format_data.get("format_name"),
            codec_name=audio_stream.get("codec_name"),
            issues=issues,
        )


def _parse_duration(value: object) -> float | None:
    if value is None:
        return None
    try:
        return float(str(value))
    except ValueError:
        return None
