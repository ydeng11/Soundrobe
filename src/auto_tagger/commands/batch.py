"""Batch command implementation."""

from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.utils import console, print_info, print_success
from auto_tagger.workflows.batch import BatchWorkflow


def execute(
    settings: Settings,
    path: Path,
    dry_run: bool,
    parallel: int,
    interactive: bool = False,
) -> None:
    """Execute batch command.

    Args:
        settings: Application settings
        path: Path to music library
        dry_run: Preview without changes
        parallel: Number of parallel processes
    """
    print_info(f"Batch processing: {path}")
    console.print(f"  Dry run: {dry_run}")
    console.print(f"  Parallel jobs: {parallel}")
    console.print(f"  YOLO mode: {settings.yolo}")
    console.print(f"  Interactive: {interactive or settings.interactive_default}")
    console.print(f"  Output format: {settings.output_format}")

    summary = BatchWorkflow(settings).run(path, dry_run=dry_run, parallel=parallel)
    console.print(f"  Albums processed: {summary.processed}")
    console.print(f"  Applied writes: {summary.applied}")
    console.print(f"  Skipped writes: {summary.skipped}")
    console.print(f"  Failed albums: {summary.failed}")
    print_success("Batch processing complete")
