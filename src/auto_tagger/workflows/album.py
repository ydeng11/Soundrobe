"""Single-album tagging workflow."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.core import iter_audio_files, load_audio_file, read_metadata, write_metadata
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.features.cover_art import (
    CoverArtArchiveClient,
    CoverArtImage,
    CoverArtStatus,
    discover_local_cover_art,
    embed_cover_art,
)
from auto_tagger.quality import AlbumHealthReport, build_album_health_report


@dataclass(frozen=True)
class AlbumWorkflowResult:
    """Structured result for one album run."""

    album_path: Path
    audio_files: list[Path]
    metadata_by_path: dict[Path, TrackMetadata]
    health_report: AlbumHealthReport
    dry_run: bool
    planned_writes: int = 0
    applied_writes: int = 0
    skipped_writes: int = 0
    cover_art_fixed: bool = False
    cover_art_status: str = ""
    cover_art_message: str = ""
    messages: list[str] = field(default_factory=list)


class AlbumWorkflow:
    """Coordinate single-album preview and safe apply behavior."""

    def __init__(self, settings: Settings):
        self.settings = settings

    def run(
        self,
        path: Path,
        dry_run: bool,
        interactive: bool = False,
    ) -> AlbumWorkflowResult:
        """Run album workflow in dry-run, interactive, or YOLO mode."""
        audio_files = iter_audio_files(path, recursive=self.settings.recursive)
        metadata_by_path = {audio_file: read_metadata(audio_file) for audio_file in audio_files}
        health_report = build_album_health_report(
            path,
            audio_files,
            metadata_by_path,
            self.settings,
        )
        planned_writes = len(audio_files)
        can_write = (
            not dry_run and self.settings.yolo and health_report.can_tag and not interactive
        )
        applied_writes = 0

        if can_write:
            for audio_file, metadata in metadata_by_path.items():
                write_metadata(audio_file, metadata, dry_run=False)
                applied_writes += 1

        skipped_writes = planned_writes - applied_writes if not dry_run else 0

        # Cover art fix: run in yolo mode regardless of metadata health
        cover_art_fixed = False
        cover_art_status = ""
        cover_art_message = ""
        if not dry_run and self.settings.yolo and not interactive:
            cover_art_fixed, cover_art_status, cover_art_message = self._fix_cover_art(
                path, audio_files, metadata_by_path
            )

        return AlbumWorkflowResult(
            album_path=path,
            audio_files=audio_files,
            metadata_by_path=metadata_by_path,
            health_report=health_report,
            dry_run=dry_run,
            planned_writes=planned_writes,
            applied_writes=applied_writes,
            skipped_writes=skipped_writes,
            cover_art_fixed=cover_art_fixed,
            cover_art_status=cover_art_status,
            cover_art_message=cover_art_message,
        )

    def _fix_cover_art(
        self,
        album_path: Path,
        audio_files: list[Path],
        metadata_by_path: dict[Path, TrackMetadata],
    ) -> tuple[bool, str, str]:
        """Fix missing cover art: local first, then Cover Art Archive, then embed.

        Returns (fixed, status, message).
        """
        if not audio_files:
            return False, CoverArtStatus.MISSING, "No audio files to embed into"

        # 1. Try local cover art (album-name first, then generic)
        album_name = next(
            (m.album for m in metadata_by_path.values() if m.album), None
        )
        image = discover_local_cover_art(album_path, album_name)
        if image is not None:
            self._embed_into_all(audio_files, image)
            return True, CoverArtStatus.FOUND_LOCAL, "Embedded local cover art"

        # 2. Try Cover Art Archive (requires MusicBrainz album ID)
        musicbrainz_albumid = self._find_musicbrainz_albumid(metadata_by_path)
        if musicbrainz_albumid is None:
            return False, CoverArtStatus.MISSING, "No local cover and no MusicBrainz album ID"

        client = CoverArtArchiveClient(timeout_seconds=self.settings.cover_art_timeout_seconds)
        result = client.fetch_front_cover(musicbrainz_albumid)

        if result.status == CoverArtStatus.FETCHED_REMOTE and result.image is not None:
            self._embed_into_all(audio_files, result.image)
            return (
                True,
                CoverArtStatus.FETCHED_REMOTE,
                "Fetched and embedded cover from Cover Art Archive",
            )

        return False, result.status, result.message

    @staticmethod
    def _find_musicbrainz_albumid(metadata_by_path: dict[Path, TrackMetadata]) -> str | None:
        """Find the first non-None MusicBrainz album ID in the metadata set."""
        for metadata in metadata_by_path.values():
            if metadata.musicbrainz_albumid:
                return metadata.musicbrainz_albumid
        return None

    @staticmethod
    def _embed_into_all(audio_files: list[Path], image: CoverArtImage) -> None:
        """Embed a cover art image into every audio file."""
        for audio_file in audio_files:
            try:
                af = load_audio_file(audio_file)
                embed_cover_art(af.format, af.mutagen_file, image)
                af.mutagen_file.save()
            except Exception:
                continue
