"""Batch command implementation."""

import json
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
    health_report_path: Path | None = None,
) -> None:
    """Execute batch command.

    Args:
        settings: Application settings
        path: Path to music library
        dry_run: Preview without changes
        parallel: Number of parallel processes
        health_report_path: Optional path to write combined health report JSON
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

    if health_report_path and summary.health_reports:
        total_errors = sum(r["summary"]["errors"] for r in summary.health_reports)
        total_warnings = sum(r["summary"]["warnings"] for r in summary.health_reports)
        total_info = sum(r["summary"]["info"] for r in summary.health_reports)
        report = {
            "library_path": str(path),
            "albums_checked": len(summary.health_reports),
            "summary": {
                "errors": total_errors,
                "warnings": total_warnings,
                "info": total_info,
            },
            "albums": summary.health_reports,
        }
        health_report_path.parent.mkdir(parents=True, exist_ok=True)
        health_report_path.write_text(
            json.dumps(report, indent=2, ensure_ascii=False),
            encoding="utf-8",
        )
        print_info(f"Wrote combined health report: {health_report_path}")

    print_success("Batch processing complete")
