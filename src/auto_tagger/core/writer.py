"""Write normalized metadata to audio files."""

from __future__ import annotations

from pathlib import Path

from auto_tagger.core.audio import load_audio_file
from auto_tagger.core.formats import write_tags
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.exceptions import FileProcessingError, TaggingError


def write_metadata(path: Path, metadata: TrackMetadata, dry_run: bool = False) -> TrackMetadata:
    """Write metadata to a supported audio file or return it unchanged in dry-run mode."""
    normalized = metadata.normalized()
    try:
        audio_file = load_audio_file(path)
        if dry_run:
            return normalized

        write_tags(audio_file.format, audio_file.mutagen_file, normalized)
        audio_file.mutagen_file.save()
        return normalized
    except FileProcessingError:
        raise
    except Exception as exc:
        raise TaggingError(f"Could not write metadata to {path}: {exc}") from exc
