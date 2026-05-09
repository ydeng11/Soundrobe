"""Core audio metadata services."""

from auto_tagger.core.audio import AudioFile, AudioFormat, detect_audio_format, iter_audio_files
from auto_tagger.core.metadata import ReplayGainTags, TrackMetadata
from auto_tagger.core.reader import read_album_metadata, read_metadata
from auto_tagger.core.writer import write_metadata

__all__ = [
    "AudioFile",
    "AudioFormat",
    "ReplayGainTags",
    "TrackMetadata",
    "detect_audio_format",
    "iter_audio_files",
    "read_album_metadata",
    "read_metadata",
    "write_metadata",
]
