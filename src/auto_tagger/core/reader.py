"""Read audio metadata into normalized models."""

from __future__ import annotations

from pathlib import Path

from auto_tagger.core.audio import iter_audio_files, load_audio_file
from auto_tagger.core.formats import read_tags
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.exceptions import FileProcessingError, TaggingError


def read_metadata(path: Path) -> TrackMetadata:
    """Read metadata from a supported audio file."""
    try:
        audio_file = load_audio_file(path)
        return read_tags(audio_file.format, audio_file.mutagen_file)
    except FileProcessingError:
        raise
    except Exception as exc:
        raise TaggingError(f"Could not read metadata from {path}: {exc}") from exc


def read_album_metadata(path: Path, recursive: bool = False) -> dict[Path, TrackMetadata]:
    """Read metadata for all supported audio files under a path."""
    return {
        audio_path: read_metadata(audio_path)
        for audio_path in iter_audio_files(path, recursive=recursive)
    }
