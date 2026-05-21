"""Batch command implementation."""

import json
from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.quality import (
    health_report_paths,
    render_combined_health_report_markdown,
    report_dict_to_markdown,
)
from auto_tagger.utils import console, print_info, print_success
from auto_tagger.workflows.batch import BatchWorkflow


def execute(
    settings: Settings,
    path: Path,
    dry_run: bool,
    parallel: int,
    interactive: bool = False,
    health_report_path: Path | None = None,
    force: bool = False,
) -> None:
    """Execute batch command.

    Args:
        settings: Application settings
        path: Path to music library
        dry_run: Preview without changes
        parallel: Number of parallel processes
        health_report_path: Optional path to write combined health report JSON
        force: Ignore album state cache
    """
    print_info(f"Batch processing: {path}")
    console.print(f"  Dry run: {dry_run}")
    console.print(f"  Parallel jobs: {parallel}")
    console.print(f"  YOLO mode: {settings.yolo}")
    console.print(f"  Force: {force}")
    console.print(f"  Interactive: {interactive or settings.interactive_default}")
    console.print(f"  Output format: {settings.output_format}")

    summary = BatchWorkflow(settings).run(path, dry_run=dry_run, parallel=parallel, force=force)
    console.print(f"  Albums processed: {summary.processed}")
    console.print(f"  Applied writes: {summary.applied}")
    console.print(f"  Skipped writes: {summary.skipped}")
    console.print(f"  Failed albums: {summary.failed}")
    if summary.errors:
        for err in summary.errors:
            console.print(f"  [red]Error:[/red] {err}")
    if summary.cover_art_fixed:
        console.print(f"  Cover art fixed: {summary.cover_art_fixed}")

    # Write per-album health reports
    for report_dict in summary.health_reports:
        album_path = Path(report_dict.get("album_path", ""))
        if album_path:
            _write_health_reports(album_path, report_dict, settings)

    # Write combined batch report (MD + JSON)
    if summary.health_reports:
        _write_combined_batch_report(
            path, summary.health_reports, settings,
            explicit_path=health_report_path,
            cross_album_issues=summary.cross_album_issues,
        )
    elif summary.cross_album_issues:
        # No per-album reports but cross-album issues exist — still write them
        _write_combined_batch_report(
            path, [], settings,
            explicit_path=health_report_path,
            cross_album_issues=summary.cross_album_issues,
        )

    print_success("Batch processing complete")


def _write_health_reports(
    album_path: Path,
    report_dict: dict,
    settings: Settings,
) -> None:
    """Write per-album health report MD + JSON to the default directory."""
    md_path, json_path = health_report_paths(album_path, settings.health_report_dir)
    md_path.parent.mkdir(parents=True, exist_ok=True)

    md_content = report_dict_to_markdown(report_dict, album_path)
    md_path.write_text(md_content, encoding="utf-8")

    json_path.write_text(
        json.dumps(report_dict, indent=2),
        encoding="utf-8",
    )


def _write_combined_batch_report(
    library_path: Path,
    album_reports: list[dict],
    settings: Settings,
    explicit_path: Path | None = None,
    cross_album_issues: list[dict] | None = None,
) -> None:
    """Write combined batch report for the whole library.

    Writes JSON always. Also writes Markdown when no explicit_path is given.
    """
    report = _build_batch_report_dict(library_path, album_reports, cross_album_issues)

    if explicit_path is not None:
        explicit_path.parent.mkdir(parents=True, exist_ok=True)
        explicit_path.write_text(
            json.dumps(report, indent=2, ensure_ascii=False),
            encoding="utf-8",
        )
        print_info(f"Wrote combined health report: {explicit_path}")
        return

    batch_name = f"batch-{library_path.name}"
    md_path = settings.health_report_dir / f"{batch_name}.md"
    json_path = settings.health_report_dir / f"{batch_name}.json"
    settings.health_report_dir.mkdir(parents=True, exist_ok=True)

    md_content = render_combined_health_report_markdown(
        album_reports, library_path, cross_album_issues=cross_album_issues,
    )
    md_path.write_text(md_content, encoding="utf-8")
    json_path.write_text(
        json.dumps(report, indent=2, ensure_ascii=False),
        encoding="utf-8",
    )
    print_info(f"Batch health report: {md_path}")


_SEVERITY_KEY: dict[str, str] = {
    "error": "errors",
    "warning": "warnings",
}


def _build_batch_report_dict(
    library_path: Path,
    album_reports: list[dict],
    cross_album_issues: list[dict] | None = None,
) -> dict:
    """Build the combined batch report dict (JSON structure)."""
    summary = {
        "errors": sum(r["summary"]["errors"] for r in album_reports),
        "warnings": sum(r["summary"]["warnings"] for r in album_reports),
        "info": sum(r["summary"]["info"] for r in album_reports),
    }
    if cross_album_issues:
        for issue in cross_album_issues:
            key = _SEVERITY_KEY.get(issue.get("severity", "info"), "info")
            summary[key] += 1

    result: dict = {
        "library_path": str(library_path),
        "albums_checked": len(album_reports),
        "summary": summary,
        "albums": album_reports,
    }
    if cross_album_issues:
        result["cross_album_issues"] = cross_album_issues
    return result
