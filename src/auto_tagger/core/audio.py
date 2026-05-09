"""Audio file discovery and loading helpers."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import Any

from auto_tagger.exceptions import FileProcessingError


class AudioFormat(Enum):
    """Supported audio metadata formats."""

    MP3 = "mp3"
    FLAC = "flac"
    M4A = "m4a"


SUPPORTED_EXTENSIONS = {
    ".mp3": AudioFormat.MP3,
    ".flac": AudioFormat.FLAC,
    ".m4a": AudioFormat.M4A,
    ".mp4": AudioFormat.M4A,
}


@dataclass(frozen=True)
class AudioFile:
    """Loaded audio file with normalized format information."""

    path: Path
    format: AudioFormat
    mutagen_file: Any


def detect_audio_format(path: Path) -> AudioFormat:
    """Detect supported audio format by extension."""
    try:
        return SUPPORTED_EXTENSIONS[path.suffix.lower()]
    except KeyError as exc:
        raise FileProcessingError(f"Unsupported audio format: {path}") from exc


def iter_audio_files(path: Path, recursive: bool = False) -> list[Path]:
    """Return sorted supported audio files under a file or directory path."""
    if path.is_file():
        detect_audio_format(path)
        return [path]

    if not path.exists():
        raise FileProcessingError(f"Path not found: {path}")
    if not path.is_dir():
        raise FileProcessingError(f"Path is not a file or directory: {path}")

    iterator = path.rglob("*") if recursive else path.iterdir()
    files = sorted(
        candidate
        for candidate in iterator
        if candidate.is_file() and candidate.suffix.lower() in SUPPORTED_EXTENSIONS
    )
    if not files:
        raise FileProcessingError(f"No supported audio files found: {path}")
    return files


def load_audio_file(path: Path) -> AudioFile:
    """Load an audio file through mutagen and return the normalized wrapper."""
    if not path.exists():
        raise FileProcessingError(f"Path not found: {path}")
    if not path.is_file():
        raise FileProcessingError(f"Path is not a file: {path}")

    audio_format = detect_audio_format(path)

    try:
        from mutagen import File
    except ImportError as exc:
        raise FileProcessingError("mutagen is required for audio metadata support") from exc

    try:
        mutagen_file = File(path, easy=False)
    except Exception as exc:
        raise FileProcessingError(f"Could not read audio file {path}: {exc}") from exc

    if mutagen_file is None:
        raise FileProcessingError(f"Could not read audio file {path}")

    return AudioFile(path=path, format=audio_format, mutagen_file=mutagen_file)
