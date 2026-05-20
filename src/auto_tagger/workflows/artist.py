"""Artist artwork workflow — fetches artist images from Discogs for Navidrome."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.features.artist_artwork import (
    ArtistArtworkOutcome,
    ArtistArtworkStatus,
    ArtistArtworkSummary,
    discover_artist_directories,
    find_local_artist_image,
    save_artist_image,
)
from auto_tagger.features.cover_art import CoverArtStatus
from auto_tagger.integrations.discogs_client import DiscogsClient, DiscogsError


@dataclass(frozen=True)
class ArtistWorkflowConfig:
    """Per-run configuration for the artist artwork workflow."""

    dry_run: bool = False
    force: bool = False
    skip_dirs: set[str] | None = None
    parallel: int = 1
    discogs_token: str | None = None

    def discogs_client(self, settings: Settings) -> DiscogsClient:
        """Create a DiscogsClient configured for artist lookups."""
        return DiscogsClient(
            token=self.discogs_token or settings.discogs_token,
            timeout_seconds=settings.artist_artwork_timeout_seconds,
            max_candidates=5,
        )


class ArtistWorkflow:
    """Coordinate artist artwork discovery and fetching for a library."""

    def __init__(self, settings: Settings):
        self.settings = settings

    def run(
        self,
        library_path: Path,
        dry_run: bool = False,
        force: bool = False,
        parallel: int = 1,
    ) -> ArtistArtworkSummary:
        """Run the artist artwork workflow.

        Scans top-level directories under *library_path* for artist folders,
        then for each artist:
          1. Check for existing valid ``artist.{jpg,png}``
          2. If missing or ``--force``, fetch from Discogs
          3. Save to artist directory

        Args:
            library_path: Root of the music library (e.g. ``/Volumes/downloads/music``).
            dry_run: If True, only report what would be done, don't fetch or save.
            force: If True, re-fetch even when a valid local image exists.
            parallel: Number of parallel fetches (currently unused; sequential only).

        Returns:
            An ``ArtistArtworkSummary`` with counts and per-artist outcomes.
        """
        if not library_path.is_dir():
            return ArtistArtworkSummary(
                errors=[f"Not a directory: {library_path}"],
            )

        # Use the first non-Compilations artist directory as a quick parse
        # sanity check, then discover all artist directories.
        skip = set(self.settings.artist_artwork_skip_dirs)
        artist_dirs = discover_artist_directories(library_path, skip_dirs=skip)

        if not artist_dirs:
            return ArtistArtworkSummary(
                errors=[f"No artist directories found under: {library_path}"],
            )

        config = ArtistWorkflowConfig(
            dry_run=dry_run,
            force=force,
            skip_dirs=skip,
            parallel=parallel,
            discogs_token=self.settings.discogs_token,
        )
        discogs = config.discogs_client(self.settings)

        outcomes: list[ArtistArtworkOutcome] = []
        errors: list[str] = []

        for artist_dir in artist_dirs:
            outcome = self._process_artist(
                artist_dir, config, discogs,
            )
            outcomes.append(outcome)

        missing_count = sum(1 for o in outcomes if o.status == ArtistArtworkStatus.MISSING)
        failed_count = sum(1 for o in outcomes if o.status == ArtistArtworkStatus.FAILED)
        already_count = sum(1 for o in outcomes if o.status == ArtistArtworkStatus.ALREADY_EXISTS)
        fetched_count = sum(1 for o in outcomes if o.status == ArtistArtworkStatus.FETCHED)
        skipped_count = sum(1 for o in outcomes if o.status == ArtistArtworkStatus.SKIPPED)

        return ArtistArtworkSummary(
            found_local=already_count,
            fetched=fetched_count,
            skipped=skipped_count,
            missing=missing_count,
            failed=failed_count,
            total=len(artist_dirs),
            outcomes=outcomes,
            errors=errors,
        )

    def _process_artist(
        self,
        artist_dir: Path,
        config: ArtistWorkflowConfig,
        discogs: DiscogsClient,
    ) -> ArtistArtworkOutcome:
        """Process a single artist directory: check, fetch, save.

        Returns the outcome for this artist.
        """
        artist_name = artist_dir.name

        # 1. Check for existing valid local image
        local_image = find_local_artist_image(artist_dir)
        if local_image is not None and not config.force:
            return ArtistArtworkOutcome(
                artist_name=artist_name,
                artist_path=artist_dir,
                status=ArtistArtworkStatus.ALREADY_EXISTS,
                image_path=local_image.path,
                message="Valid artist.jpg already present",
            )

        if config.dry_run:
            if local_image is None:
                return ArtistArtworkOutcome(
                    artist_name=artist_name,
                    artist_path=artist_dir,
                    status=ArtistArtworkStatus.MISSING,
                    message="Would fetch artist image from Discogs",
                )
            return ArtistArtworkOutcome(
                artist_name=artist_name,
                artist_path=artist_dir,
                status=ArtistArtworkStatus.SKIPPED,
                image_path=local_image.path,
                message="Would re-fetch (--force); existing image would be overwritten",
            )

        if not self.settings.artist_artwork_enabled:
            return ArtistArtworkOutcome(
                artist_name=artist_name,
                artist_path=artist_dir,
                status=ArtistArtworkStatus.SKIPPED,
                message="Artist artwork disabled in settings",
            )

        # 2. Fetch from Discogs
        try:
            result = discogs.fetch_artist_image(artist_name)
        except DiscogsError as exc:
            return ArtistArtworkOutcome(
                artist_name=artist_name,
                artist_path=artist_dir,
                status=ArtistArtworkStatus.FAILED,
                message=f"Discogs error: {exc}",
            )

        if result.status != CoverArtStatus.FETCHED_REMOTE or result.image is None:
            return ArtistArtworkOutcome(
                artist_name=artist_name,
                artist_path=artist_dir,
                status=ArtistArtworkStatus.MISSING,
                message=result.message or "No image found on Discogs",
            )

        # 3. Save to artist directory
        try:
            saved_path = save_artist_image(artist_dir, result.image)
        except OSError as exc:
            return ArtistArtworkOutcome(
                artist_name=artist_name,
                artist_path=artist_dir,
                status=ArtistArtworkStatus.FAILED,
                message=f"Could not save artist image: {exc}",
            )

        return ArtistArtworkOutcome(
            artist_name=artist_name,
            artist_path=artist_dir,
            status=ArtistArtworkStatus.FETCHED,
            image_path=saved_path,
            message=f"Fetched from Discogs ({result.image.source})",
        )
