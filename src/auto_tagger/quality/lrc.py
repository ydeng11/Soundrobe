"""LRC discovery, encoding validation, and conversion."""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path

from auto_tagger.quality.health import HealthIssue, HealthSeverity

TIMING_RE = re.compile(r"\[(\d{1,3}):([0-5]\d)(?:\.(\d{1,3}))?\]")
TAG_RE = re.compile(r"\[([A-Za-z]+):(.*)\]")
KNOWN_METADATA_TAGS = {"ar", "al", "ti", "by", "offset", "length", "re"}
LEGACY_ENCODINGS = ("gb18030", "big5", "cp1252", "latin-1")


@dataclass(frozen=True)
class LRCValidationResult:
    """Validation result for one LRC file."""

    path: Path
    encoding: str | None
    text: str = ""
    needs_conversion: bool = False
    issues: list[HealthIssue] = field(default_factory=list)


def discover_lrc_files(album_path: Path, audio_files: list[Path]) -> list[Path]:
    """Discover unique LRC files in album and adjacent to audio files."""
    discovered: set[Path] = set(album_path.glob("*.lrc")) if album_path.is_dir() else set()
    for audio_file in audio_files:
        adjacent = audio_file.with_suffix(".lrc")
        if adjacent.exists():
            discovered.add(adjacent)
    return sorted(discovered)


def validate_lrc_file(path: Path) -> LRCValidationResult:
    """Validate LRC encoding and timing structure."""
    encoding, text, encoding_issue = _decode_lrc(path)
    issues: list[HealthIssue] = []
    if encoding_issue is not None:
        issues.append(encoding_issue)
    if text is None:
        return LRCValidationResult(path=path, encoding=encoding, issues=issues)

    issues.extend(_validate_lrc_text(path, text))
    return LRCValidationResult(
        path=path,
        encoding=encoding,
        text=text,
        needs_conversion=encoding not in {None, "utf-8"},
        issues=issues,
    )


def convert_lrc_to_utf8(path: Path, dry_run: bool = True) -> LRCValidationResult:
    """Convert an LRC file to UTF-8 when possible."""
    result = validate_lrc_file(path)
    if dry_run or not result.needs_conversion or not result.text:
        return result

    path.write_text(result.text, encoding="utf-8")
    return validate_lrc_file(path)


def _decode_lrc(path: Path) -> tuple[str | None, str | None, HealthIssue | None]:
    data = path.read_bytes()
    if data.startswith(b"\xef\xbb\xbf"):
        return "utf-8", data.decode("utf-8-sig"), None
    try:
        return "utf-8", data.decode("utf-8"), None
    except UnicodeDecodeError:
        pass

    for encoding in LEGACY_ENCODINGS:
        try:
            text = data.decode(encoding)
        except UnicodeDecodeError:
            continue
        return (
            encoding,
            text,
            HealthIssue(
                "lrc",
                HealthSeverity.WARNING,
                path,
                "lrc.non_utf8",
                f"LRC file is encoded as {encoding}; convert to UTF-8",
                {"encoding": encoding},
            ),
        )

    return (
        None,
        None,
        HealthIssue(
            "lrc",
            HealthSeverity.ERROR,
            path,
            "lrc.undecodable",
            "LRC file could not be decoded as UTF-8 or a supported legacy encoding",
        ),
    )


def _validate_lrc_text(path: Path, text: str) -> list[HealthIssue]:
    issues: list[HealthIssue] = []
    for line_number, line in enumerate(text.splitlines(), start=1):
        stripped = line.strip()
        if not stripped:
            continue
        if TIMING_RE.search(stripped):
            continue
        metadata_match = TAG_RE.fullmatch(stripped)
        if metadata_match and metadata_match.group(1).lower() in KNOWN_METADATA_TAGS:
            continue
        if stripped.startswith("[") and "]" in stripped:
            issues.append(
                HealthIssue(
                    "lrc",
                    HealthSeverity.WARNING,
                    path,
                    "lrc.malformed_tag",
                    f"Malformed LRC tag on line {line_number}",
                    {"line": line_number, "content": stripped},
                )
            )
    return issues
