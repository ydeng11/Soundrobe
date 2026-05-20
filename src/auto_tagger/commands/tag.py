"""Tag command implementation."""

import json
from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.core import iter_audio_files, read_metadata
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.exceptions import FileProcessingError
from auto_tagger.integrations import LookupService
from auto_tagger.llm import CandidateSelectionService, OpenRouterClient
from auto_tagger.quality import (
    build_album_health_report,
    health_report_paths,
    render_health_report,
    report_dict_to_markdown,
)
from auto_tagger.utils import console, print_info, print_success, print_table
from auto_tagger.workflows.album import AlbumWorkflow

_LOOKUP_WARNINGS_SHOWN: set[str] = set()


def execute(
    settings: Settings,
    path: Path,
    dry_run: bool,
    health_report_path: Path | None = None,
    interactive: bool = False,
) -> None:
    """Execute tag command.

    Args:
        settings: Application settings
        path: Path to album or file
        dry_run: Preview without changes
    """
    print_info(f"Tagging: {path}")
    console.print(f"  Dry run: {dry_run}")
    console.print(f"  YOLO mode: {settings.yolo}")
    console.print(f"  Interactive: {interactive or settings.interactive_default}")
    console.print(f"  Output format: {settings.output_format}")

    try:
        audio_files = iter_audio_files(path, recursive=settings.recursive)
    except FileProcessingError as exc:
        console.print(f"[yellow]{exc}[/yellow]")
        return

    if not dry_run:
        workflow = AlbumWorkflow(settings)
        result = workflow.run(path, dry_run=False, interactive=interactive)

        print_info(f"Album: {path.name}")
        print_success(f"Wrote tags to {result.applied_writes} file(s)")
        if result.skipped_writes:
            console.print(f"  Skipped: {result.skipped_writes} file(s)")

        if result.messages:
            for msg in result.messages:
                console.print(f"  [green]{msg}[/green]")

        if result.cover_art_fixed:
            print_success(
                f"Cover art: {result.cover_art_status} — {result.cover_art_message}")

        _write_health_reports(path, result.health_report.to_dict(), settings, health_report_path)
        return

    metadata_by_path: dict[Path, TrackMetadata] = {}
    for audio_file in audio_files:
        metadata = read_metadata(audio_file)
        metadata_by_path[audio_file] = metadata
        rows = metadata.to_display_rows()
        if rows:
            print_table(f"Metadata preview: {audio_file.name}", ["Field", "Value"], rows)
        else:
            console.print(f"[yellow]No metadata tags found:[/yellow] {audio_file}")

    health_report = build_album_health_report(path, audio_files, metadata_by_path, settings)
    console.print(render_health_report(health_report))
    _write_health_reports(path, health_report.to_dict(), settings, health_report_path)

    _print_lookup_candidates(settings, path)
    if not dry_run and settings.yolo and health_report.can_tag:
        print_success("YOLO apply path available for safe albums")
    elif interactive or settings.interactive_default:
        print_info("Interactive preview ready; apply flow will prompt before writing")
    print_success(f"Previewed metadata for {len(audio_files)} audio file(s)")


def _print_lookup_candidates(settings: Settings, path: Path) -> None:
    try:
        lookup_service = LookupService(settings=settings)
        candidates = lookup_service.lookup_album(path)
    except Exception as exc:
        console.print(f"[yellow]Lookup unavailable:[/yellow] {exc}")
        return

    for warning in getattr(lookup_service, "warnings", []):
        if warning not in _LOOKUP_WARNINGS_SHOWN:
            console.print(f"[yellow]Dataset lookup unavailable:[/yellow] {warning}")
            _LOOKUP_WARNINGS_SHOWN.add(warning)

    if not candidates:
        console.print("[yellow]No lookup candidates found[/yellow]")
        return

    print_table(
        "Lookup candidates",
        ["Source", "Artist", "Album", "Year", "Distance", "MusicBrainz Album ID", "Verify"],
        [candidate.to_display_row() for candidate in candidates],
    )

    if len(candidates) > 1 and not settings.llm_api_key:
        console.print("[yellow]LLM selection unavailable:[/yellow] missing API key")
        return

    if len(candidates) > 1 and settings.llm_api_key:
        result = CandidateSelectionService(
            OpenRouterClient(settings),
            settings,
        ).select_candidate(lookup_service.request_from_path(path), candidates)
        if result.selected_candidate is None:
            console.print(f"[yellow]LLM selection:[/yellow] none ({result.reason})")
            return
        print_table(
            "LLM selection",
            ["Artist", "Album", "Confidence", "Reason"],
            [
                [
                    result.selected_candidate.artist or "",
                    result.selected_candidate.album or "",
                    f"{result.confidence:.2f}",
                    result.reason,
                ]
            ],
        )


def _write_health_reports(
    album_path: Path,
    report_dict: dict,
    settings: Settings,
    explicit_path: Path | None = None,
) -> None:
    """Write health report files (MD + JSON) to the default directory.

    If *explicit_path* is provided, it is used as a single-file override
    (backwards-compatible with ``--health-report``).
    """
    if explicit_path is not None:
        explicit_path.parent.mkdir(parents=True, exist_ok=True)
        explicit_path.write_text(
            json.dumps(report_dict, indent=2),
            encoding="utf-8",
        )
        print_info(f"Wrote health report: {explicit_path}")
        return

    md_path, json_path = health_report_paths(album_path, settings.health_report_dir)
    md_path.parent.mkdir(parents=True, exist_ok=True)

    md_content = report_dict_to_markdown(report_dict, album_path)
    md_path.write_text(md_content, encoding="utf-8")

    json_path.write_text(
        json.dumps(report_dict, indent=2),
        encoding="utf-8",
    )

    print_info(f"Health report: {md_path}")



