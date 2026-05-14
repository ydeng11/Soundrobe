"""Batch library workflow."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from auto_tagger.config import Settings
from auto_tagger.core.audio import SUPPORTED_EXTENSIONS
from auto_tagger.workflows.album import AlbumWorkflow


@dataclass(frozen=True)
class BatchSummary:
    """Summary of a batch run."""

    processed: int = 0
    applied: int = 0
    skipped: int = 0
    failed: int = 0
    cover_art_fixed: int = 0
    errors: list[str] = field(default_factory=list)
    health_reports: list[Any] = field(default_factory=list)


def discover_album_paths(library_path: Path) -> list[Path]:
    """Discover album directories containing supported audio files."""
    albums = {
        candidate.parent
        for candidate in library_path.rglob("*")
        if candidate.is_file() and candidate.suffix.lower() in SUPPORTED_EXTENSIONS
    }
    return sorted(albums)


class BatchWorkflow:
    """Process a library by running the album workflow per album."""

    def __init__(
        self,
        settings: Settings,
        album_workflow_factory: Callable[[Settings], AlbumWorkflow] = AlbumWorkflow,
    ):
        self.settings = settings
        self.album_workflow_factory = album_workflow_factory

    def run(self, path: Path, dry_run: bool, parallel: int = 1) -> BatchSummary:
        """Run batch processing with deterministic sequential execution.

        Maintains a cross-album MusicBrainz artist ID map so that MBIDs
        discovered in one album can propagate to other albums by the same artist
        that lack MBIDs in the lookup results.
        """
        albums = discover_album_paths(path)
        processed = applied = skipped = failed = cover_art_fixed = 0
        errors: list[str] = []
        health_reports: list[Any] = []

        # Shared mutable maps for cross-album propagation
        artist_mbid_map: dict[str, str] = {}
        artist_genre_map: dict[str, list[str]] = {}

        for album in albums:
            processed += 1
            try:
                result = self.album_workflow_factory(self.settings).run(
                    album, dry_run=dry_run,
                    artist_mbid_map=artist_mbid_map,
                    artist_genre_map=artist_genre_map,
                )
            except Exception as exc:
                failed += 1
                errors.append(f"{album}: {exc}")
                continue
            applied += result.applied_writes
            skipped += result.skipped_writes
            if result.cover_art_fixed:
                cover_art_fixed += 1
            if result.health_report is not None:
                health_reports.append(result.health_report.to_dict())

        return BatchSummary(
            processed, applied, skipped, failed, cover_art_fixed, errors, health_reports
        )
