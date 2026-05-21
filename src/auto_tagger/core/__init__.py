"""Core audio metadata services."""

from auto_tagger.core.audio import (
    AudioFile,
    AudioFormat,
    detect_audio_format,
    iter_audio_files,
    load_audio_file,
)
from auto_tagger.core.metadata import ReplayGainTags, TrackMetadata
from auto_tagger.core.parse_filename import (
    ParsedFilename,
    metadata_from_path,
    parse_album_folder_name,
    parse_track_filename,
)
from auto_tagger.core.reader import read_album_metadata, read_metadata
from auto_tagger.core.writer import write_metadata

__all__ = [
    "AudioFile",
    "AudioFormat",
    "ParsedFilename",
    "ReplayGainTags",
    "TrackMetadata",
    "detect_audio_format",
    "iter_audio_files",
    "load_audio_file",
    "metadata_from_path",
    "parse_album_folder_name",
    "parse_track_filename",
    "read_album_metadata",
    "read_metadata",
    "write_metadata",
]
