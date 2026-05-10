"""Single-album tagging workflow."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.core import iter_audio_files, read_metadata, write_metadata
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.quality import AlbumHealthReport, build_album_health_report


@dataclass(frozen=True)
class AlbumWorkflowResult:
    """Structured result for one album run."""

    album_path: Path
    audio_files: list[Path]
    metadata_by_path: dict[Path, TrackMetadata]
    health_report: AlbumHealthReport
    dry_run: bool
    planned_writes: int = 0
    applied_writes: int = 0
    skipped_writes: int = 0
    messages: list[str] = field(default_factory=list)


class AlbumWorkflow:
    """Coordinate single-album preview and safe apply behavior."""

    def __init__(self, settings: Settings):
        self.settings = settings

    def run(
        self,
        path: Path,
        dry_run: bool,
        interactive: bool = False,
    ) -> AlbumWorkflowResult:
        """Run album workflow in dry-run, interactive, or YOLO mode."""
        audio_files = iter_audio_files(path, recursive=self.settings.recursive)
        metadata_by_path = {audio_file: read_metadata(audio_file) for audio_file in audio_files}
        health_report = build_album_health_report(
            path,
            audio_files,
            metadata_by_path,
            self.settings,
        )
        planned_writes = len(audio_files)
        can_write = not dry_run and self.settings.yolo and health_report.can_tag and not interactive
        applied_writes = 0

        if can_write:
            for audio_file, metadata in metadata_by_path.items():
                write_metadata(audio_file, metadata, dry_run=False)
                applied_writes += 1

        skipped_writes = planned_writes - applied_writes if not dry_run else 0
        return AlbumWorkflowResult(
            album_path=path,
            audio_files=audio_files,
            metadata_by_path=metadata_by_path,
            health_report=health_report,
            dry_run=dry_run,
            planned_writes=planned_writes,
            applied_writes=applied_writes,
            skipped_writes=skipped_writes,
        )
