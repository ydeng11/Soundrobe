"""ReplayGain calculation and metadata application helpers."""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Protocol

from auto_tagger.core.metadata import ReplayGainTags, TrackMetadata
from auto_tagger.core.writer import write_metadata
from auto_tagger.quality.health import HealthIssue, HealthSeverity


class CommandResult(Protocol):
    """Subset of subprocess result fields used by ReplayGain helpers."""

    stdout: str
    stderr: str
    returncode: int


class CommandRunner(Protocol):
    """Callable command runner protocol."""

    def __call__(self, command: list[str], timeout: int) -> CommandResult:
        """Run a command and return a completed process-like object."""


@dataclass(frozen=True)
class ReplayGainResult:
    """ReplayGain result for one file."""

    path: Path
    tags: ReplayGainTags = field(default_factory=ReplayGainTags)
    issues: list[HealthIssue] = field(default_factory=list)


def _default_runner(command: list[str], timeout: int) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        capture_output=True,
        check=False,
        text=True,
        timeout=timeout,
    )


class ReplayGainCalculator:
    """Run a ReplayGain command and normalize the result."""

    def __init__(
        self,
        command: str = "rgain3",
        timeout_seconds: int = 600,
        runner: CommandRunner = _default_runner,
    ):
        self.command = command
        self.timeout_seconds = timeout_seconds
        self.runner = runner

    def calculate_album(self, paths: list[Path]) -> dict[Path, ReplayGainResult]:
        """Calculate ReplayGain values for an album."""
        if not paths:
            return {}

        command = [self.command, "--json", *[str(path) for path in paths]]
        try:
            completed = self.runner(command, self.timeout_seconds)
        except FileNotFoundError:
            return _issue_results(
                paths,
                "replaygain.command_missing",
                f"ReplayGain command not found: {self.command}",
                HealthSeverity.WARNING,
            )
        except subprocess.TimeoutExpired:
            return _issue_results(
                paths,
                "replaygain.command_timeout",
                f"ReplayGain command timed out after {self.timeout_seconds} seconds",
                HealthSeverity.ERROR,
            )

        if completed.returncode != 0:
            message = completed.stderr.strip() or "ReplayGain command failed"
            return _issue_results(
                paths,
                "replaygain.command_failed",
                message,
                HealthSeverity.ERROR,
            )

        try:
            payload = json.loads(completed.stdout or "{}")
        except json.JSONDecodeError:
            return _issue_results(
                paths,
                "replaygain.invalid_json",
                "ReplayGain command returned invalid JSON",
                HealthSeverity.ERROR,
            )

        return _parse_replaygain_payload(paths, payload)


def apply_replaygain_tags(
    results: dict[Path, ReplayGainResult],
    existing_metadata: dict[Path, TrackMetadata],
    dry_run: bool = True,
    chinese_script: str | None = None,
) -> dict[Path, TrackMetadata]:
    """Apply ReplayGain tags while preserving all other metadata fields."""
    updated: dict[Path, TrackMetadata] = {}
    for path, result in results.items():
        metadata = existing_metadata.get(path)
        if metadata is None or result.issues or result.tags.is_empty():
            continue
        new_metadata = replace(metadata, replaygain=result.tags)
        if dry_run:
            updated[path] = new_metadata.normalized()
        else:
            updated[path] = write_metadata(path, new_metadata, dry_run=False, chinese_script=chinese_script)
    return updated


def _issue_results(
    paths: list[Path],
    code: str,
    message: str,
    severity: HealthSeverity,
) -> dict[Path, ReplayGainResult]:
    return {
        path: ReplayGainResult(
            path=path,
            issues=[
                HealthIssue(
                    "replaygain",
                    severity,
                    path,
                    code,
                    message,
                )
            ],
        )
        for path in paths
    }


def _parse_replaygain_payload(
    paths: list[Path],
    payload: dict[str, object],
) -> dict[Path, ReplayGainResult]:
    by_path = {path: ReplayGainResult(path=path) for path in paths}
    files = payload.get("files")
    if not isinstance(files, list):
        return _issue_results(
            paths,
            "replaygain.missing_results",
            "ReplayGain output did not include file results",
            HealthSeverity.ERROR,
        )

    for item in files:
        if not isinstance(item, dict):
            continue
        item_path = Path(str(item.get("path", "")))
        if item_path not in by_path:
            continue
        by_path[item_path] = ReplayGainResult(
            path=item_path,
            tags=ReplayGainTags(
                track_gain=_normalize_gain(item.get("track_gain")),
                track_peak=_normalize_peak(item.get("track_peak")),
                album_gain=_normalize_gain(item.get("album_gain")),
                album_peak=_normalize_peak(item.get("album_peak")),
            ),
        )
    return by_path


def _normalize_gain(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    if not text:
        return None
    if text.endswith("dB"):
        return text
    return f"{text} dB"


def _normalize_peak(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None
