"""Artist command implementations."""

from __future__ import annotations

from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.features.artist_artwork import ArtistArtworkStatus, ArtistArtworkSummary
from auto_tagger.utils import console, print_info, print_success, print_warning
from auto_tagger.workflows.artist import ArtistWorkflow


def execute_artwork(
    settings: Settings,
    library_path: Path,
    dry_run: bool,
    force: bool,
    parallel: int,
) -> None:
    """Execute the ``artist artwork`` command.

    Discovers artist directories, fetches artwork from Discogs, and saves
    ``artist.jpg`` to each artist folder.
    """
    print_info("Artist artwork")
    console.print(f"  Library: {library_path}")
    console.print(f"  Dry run: {dry_run}")
    console.print(f"  Force:   {force}")
    console.print(f"  Parallel: {parallel}")

    workflow = ArtistWorkflow(settings)
    summary = workflow.run(
        library_path,
        dry_run=dry_run,
        force=force,
        parallel=parallel,
    )

    _render_summary(summary, dry_run=dry_run)


def _render_summary(summary: ArtistArtworkSummary, dry_run: bool) -> None:
    """Print a summary table of the artist artwork run."""
    console.print()

    if summary.errors:
        for err in summary.errors:
            console.print(f"[red]✗[/red] {err}")

    if not summary.total:
        console.print("[yellow]No artist directories found.[/yellow]")
        return

    # Group outcomes by status for display
    already = [o for o in summary.outcomes if o.status == ArtistArtworkStatus.ALREADY_EXISTS]
    fetched = [o for o in summary.outcomes if o.status == ArtistArtworkStatus.FETCHED]
    missing = [o for o in summary.outcomes if o.status == ArtistArtworkStatus.MISSING]
    failed = [o for o in summary.outcomes if o.status == ArtistArtworkStatus.FAILED]
    skipped = [o for o in summary.outcomes if o.status == ArtistArtworkStatus.SKIPPED]

    if dry_run:
        status_line = (
            f"[cyan]{summary.total}[/cyan] artist(s) scanned — "
            f"[green]{summary.found_local}[/green] already present, "
            f"[yellow]{len(missing)}[/yellow] would fetch, "
        )
        if summary.failed:
            status_line += f"[red]{summary.failed} errors, [/red]"
        status_line += f"[dim]{len(skipped)} skipped[/dim]"
        console.print(status_line)
    else:
        status_line = (
            f"[cyan]{summary.total}[/cyan] artist(s) processed — "
            f"[green]{summary.found_local}[/green] already present, "
            f"[green]{summary.fetched}[/green] fetched, "
        )
        if summary.missing:
            status_line += f"[yellow]{summary.missing} not found on Discogs, [/yellow]"
        if summary.failed:
            status_line += f"[red]{summary.failed} failed, [/red]"
        status_line += f"[dim]{len(skipped)} skipped[/dim]"
        console.print(status_line)

    # Show artists that failed or were not found
    for outcome in failed:
        console.print(f"  [red]✗[/red] {outcome.artist_name}: {outcome.message}")

    for outcome in missing:
        console.print(f"  [yellow]?[/yellow] {outcome.artist_name}: {outcome.message}")

    for outcome in skipped:
        console.print(f"  [dim]—[/dim] {outcome.artist_name}: {outcome.message}")

    if summary.fetched:
        print_success(f"Fetched artist artwork for {summary.fetched} artist(s)")
