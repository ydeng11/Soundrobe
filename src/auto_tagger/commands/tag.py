"""Tag command implementation."""

from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.core import iter_audio_files, read_metadata
from auto_tagger.exceptions import FileProcessingError
from auto_tagger.utils import console, print_info, print_success, print_table


def execute(settings: Settings, path: Path, dry_run: bool) -> None:
    """Execute tag command.

    Args:
        settings: Application settings
        path: Path to album or file
        dry_run: Preview without changes
    """
    print_info(f"Tagging: {path}")
    console.print(f"  Dry run: {dry_run}")
    console.print(f"  YOLO mode: {settings.yolo}")
    console.print(f"  Output format: {settings.output_format}")

    try:
        audio_files = iter_audio_files(path, recursive=settings.recursive)
    except FileProcessingError as exc:
        console.print(f"[yellow]{exc}[/yellow]")
        return

    if not dry_run:
        print_info("Phase 2 supports metadata reading only; use --dry-run to preview tags")
        return

    for audio_file in audio_files:
        metadata = read_metadata(audio_file)
        rows = metadata.to_display_rows()
        if rows:
            print_table(f"Metadata preview: {audio_file.name}", ["Field", "Value"], rows)
        else:
            console.print(f"[yellow]No metadata tags found:[/yellow] {audio_file}")

    print_success(f"Previewed metadata for {len(audio_files)} audio file(s)")
