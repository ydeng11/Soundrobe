"""Batch library workflow."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Protocol

from auto_tagger.config import Settings
from auto_tagger.core.audio import SUPPORTED_EXTENSIONS
from auto_tagger.quality.metadata_validation import check_cross_album_artist_consistency
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
    cross_album_issues: list[dict] = field(default_factory=list)


def discover_album_paths(library_path: Path) -> list[Path]:
    """Discover album directories containing supported audio files."""
    albums = {
        candidate.parent
        for candidate in library_path.rglob("*")
        if candidate.is_file() and candidate.suffix.lower() in SUPPORTED_EXTENSIONS
    }
    return sorted(albums)


class ProgressCallback(Protocol):
    """Protocol for a progress callback: (current, total) -> None."""

    def __call__(self, current: int, total: int) -> None: ...


class BatchWorkflow:
    """Process a library by running the album workflow per album."""

    def __init__(
        self,
        settings: Settings,
        album_workflow_factory: Callable[[Settings], AlbumWorkflow] = AlbumWorkflow,
    ):
        self.settings = settings
        self.album_workflow_factory = album_workflow_factory

    def run(
        self,
        path: Path,
        dry_run: bool,
        parallel: int = 1,
        force: bool = False,
        progress_callback: ProgressCallback | None = None,
    ) -> BatchSummary:
        """Run batch processing with deterministic sequential execution.

        Maintains a cross-album MusicBrainz artist ID map so that MBIDs
        discovered in one album can propagate to other albums by the same artist
        that lack MBIDs in the lookup results.

        Args:
            path: Music library root directory.
            dry_run: If True, preview only.
            parallel: Number of parallel workers (unused; always sequential).
            force: If True, ignore album state cache.
            progress_callback: Optional callback invoked after each album with
                (current: int, total: int).
        """
        albums = discover_album_paths(path)
        total = len(albums)
        processed = applied = skipped = failed = cover_art_fixed = 0
        errors: list[str] = []
        health_reports: list[Any] = []

        # Shared mutable maps for cross-album propagation
        artist_mbid_map: dict[str, str] = {}
        artist_genre_map: dict[str, list[str]] = {}

        # Collect album_artist values for cross-album consistency check
        album_artist_data: list[tuple[Path, str | None]] = []

        for album in albums:
            processed += 1
            if progress_callback is not None:
                progress_callback(processed, total)
            try:
                result = self.album_workflow_factory(self.settings).run(
                    album, dry_run=dry_run, force=force,
                    artist_mbid_map=artist_mbid_map,
                    artist_genre_map=artist_genre_map,
                )
            except Exception as exc:
                failed += 1
                errors.append(f"{album}: {exc}")
                album_artist_data.append((album, None))
                continue
            applied += result.applied_writes
            skipped += result.skipped_writes
            if result.cover_art_fixed:
                cover_art_fixed += 1
            if result.health_report is not None:
                health_reports.append(result.health_report.to_dict())
            # Extract album_artist from first track's metadata
            first_meta = next(iter(result.metadata_by_path.values()), None)
            aa = first_meta.album_artist if first_meta is not None else None
            album_artist_data.append((album, aa))

        # Run cross-album consistency check
        cross_album_issues = check_cross_album_artist_consistency(album_artist_data)

        return BatchSummary(
            processed, applied, skipped, failed, cover_art_fixed, errors,
            health_reports,
            [issue.to_dict() for issue in cross_album_issues],
        )
