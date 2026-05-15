"""Clean command — strip junk tags from audio files."""

from __future__ import annotations

from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.core.audio import iter_audio_files, load_audio_file
from auto_tagger.core.formats import JUNK_TAG_NAMES
from auto_tagger.exceptions import FileProcessingError
from auto_tagger.utils import console, print_info, print_success, print_table


def execute(
    settings: Settings,
    path: Path,
    dry_run: bool = False,
) -> None:
    """Execute clean command.

    Scans audio files under *path*, finds any junk tags (description,
    comment, c — aka watermarks from Chinese download sources), and
    removes them.

    Args:
        settings: Application settings
        path: Path to an audio file or album/library directory
        dry_run: If True, only preview what would be removed
    """
    print_info(f"Cleaning junk tags: {path}")
    console.print(f"  Dry run: {dry_run}")
    console.print(f"  Tags to remove: {', '.join(sorted(JUNK_TAG_NAMES))}")

    # Always scan recursively when given a directory, since junk tags can
    # appear in any subdirectory.  Non-recursive scan is used only for single
    # file paths (iter_audio_files handles file paths correctly either way).
    recursive = path.is_dir() or settings.recursive
    try:
        audio_files = iter_audio_files(path, recursive=recursive)
    except FileProcessingError as exc:
        console.print(f"[yellow]{exc}[/yellow]")
        return

    total_files = len(audio_files)
    removed_count = 0
    removed_map: dict[str, int] = {name: 0 for name in JUNK_TAG_NAMES}
    report_rows: list[list[str]] = []

    for audio_file_path in audio_files:
        try:
            audio_file = load_audio_file(audio_file_path)
        except FileProcessingError:
            continue

        tags = audio_file.mutagen_file
        found_in_file: list[str] = []

        for junk_key in JUNK_TAG_NAMES:
            if junk_key in tags:
                found_in_file.append(junk_key)
                removed_map[junk_key] += 1

        if found_in_file:
            rel_path = audio_file_path.relative_to(path) if path != audio_file_path else audio_file_path.name
            report_rows.append([str(rel_path), ", ".join(found_in_file)])
            removed_count += 1

            if not dry_run:
                for junk_key in found_in_file:
                    try:
                        del tags[junk_key]
                    except (KeyError, TypeError):
                        pass
                try:
                    tags.save()
                except Exception as exc:
                    console.print(f"[red]Error saving {audio_file_path}: {exc}[/red]")

    # Summary
    if dry_run:
        mode = "[yellow]DRY RUN[/yellow] — no changes applied"
    else:
        mode = "[green]Applied[/green]"

    console.print()
    print_info(
        f"Scanned {total_files} file(s), found junk tags in {removed_count} file(s) — {mode}"
    )
    for junk_key in sorted(JUNK_TAG_NAMES):
        count = removed_map[junk_key]
        if count > 0:
            console.print(f"  Removed '{junk_key}': {count} occurrence(s)")

    if report_rows:
        console.print()
        print_table(
            "Files with junk tags",
            ["File", "Tags removed"],
            report_rows,
        )
    else:
        print_success("No junk tags found")

    if removed_count > 0 and dry_run:
        print_info("Run without --dry-run to apply these changes")
