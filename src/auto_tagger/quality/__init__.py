"""Quality assurance helpers for album health validation."""

from auto_tagger.quality.audio_validation import FFProbeValidator
from auto_tagger.quality.health import (
    AlbumHealthReport,
    HealthIssue,
    HealthSeverity,
    TrackHealth,
    build_album_health_report,
    render_health_report,
)
from auto_tagger.quality.lrc import convert_lrc_to_utf8, discover_lrc_files, validate_lrc_file
from auto_tagger.quality.metadata_validation import validate_album_metadata, validate_track_metadata
from auto_tagger.quality.replaygain import ReplayGainCalculator, apply_replaygain_tags

__all__ = [
    "AlbumHealthReport",
    "FFProbeValidator",
    "HealthIssue",
    "HealthSeverity",
    "ReplayGainCalculator",
    "TrackHealth",
    "apply_replaygain_tags",
    "build_album_health_report",
    "convert_lrc_to_utf8",
    "discover_lrc_files",
    "render_health_report",
    "validate_album_metadata",
    "validate_lrc_file",
    "validate_track_metadata",
]
