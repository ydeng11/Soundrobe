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
from auto_tagger.integrations import LookupService
from auto_tagger.integrations.candidates import AlbumCandidate, LookupSource
from auto_tagger.integrations.discogs_client import DiscogsClient, DiscogsError
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
        metadata_fixed = False
        fix_source = ""
        fix_messages: list[str] = []

        # YOLO mode: if tags are missing/error, try to fix from lookup cascade
        bad_genres = {"未知流派", "unknown", "Unknown", "?"}
        has_bad_genre = any(
            m.genre is None or m.genre in bad_genres
            for m in metadata_by_path.values()
        )
        needs_fix = (
            not health_report.can_tag
            or self._artist_mismatches_folder(path, metadata_by_path)
            or has_bad_genre
        )
        if not dry_run and self.settings.yolo and not interactive and needs_fix:
            fixed, source, msg = self._fix_metadata(path, audio_files, metadata_by_path)
            if fixed:
                metadata_fixed = True
                fix_source = source
                fix_messages.append(msg)
                # Re-read metadata and rebuild health after fix
                metadata_by_path = {
                    audio_file: read_metadata(audio_file) for audio_file in audio_files
                }
                health_report = build_album_health_report(
                    path, audio_files, metadata_by_path, self.settings
                )
                applied_writes = len(audio_files)

        if can_write:
            # Check if genre is missing — do a quick lookup to enrich
            first_meta = next(iter(metadata_by_path.values()), None)
            if first_meta and (
                first_meta.genre is None
                or first_meta.genre in ("未知流派", "unknown", "Unknown", "?")
            ):
                try:
                    enriched = self._enrich_genre_from_lookup(path, metadata_by_path)
                    if enriched:
                        metadata_by_path = enriched
                        fix_messages.append("Enriched genre from Discogs")
                except Exception:
                    pass

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
            messages=fix_messages,
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
        if musicbrainz_albumid:
            client = CoverArtArchiveClient(
                timeout_seconds=self.settings.cover_art_timeout_seconds
            )
            result = client.fetch_front_cover(musicbrainz_albumid)
            if result.status == CoverArtStatus.FETCHED_REMOTE and result.image is not None:
                self._embed_into_all(audio_files, result.image)
                return (
                    True,
                    CoverArtStatus.FETCHED_REMOTE,
                    "Fetched and embedded cover from Cover Art Archive",
                )

        # 3. Try Discogs cover art
        artist_name = next(
            (m.artist for m in metadata_by_path.values() if m.artist), None
        )
        if artist_name and album_name:
            discogs = DiscogsClient(
                timeout_seconds=self.settings.cover_art_timeout_seconds
            )
            result = discogs.fetch_cover_art(artist_name, album_name)
            if result.status == CoverArtStatus.FETCHED_REMOTE and result.image is not None:
                self._embed_into_all(audio_files, result.image)
                return (
                    True,
                    CoverArtStatus.FETCHED_REMOTE,
                    f"Fetched and embedded cover from Discogs",
                )

        return False, CoverArtStatus.MISSING, "No local cover and no online cover found"

    @staticmethod
    def _find_musicbrainz_albumid(metadata_by_path: dict[Path, TrackMetadata]) -> str | None:
        """Find the first non-None MusicBrainz album ID in the metadata set."""
        for metadata in metadata_by_path.values():
            if metadata.musicbrainz_albumid:
                return metadata.musicbrainz_albumid
        return None

    # ── metadata fix via lookup cascade ──────────────────────

    def _fix_metadata(
        self,
        album_path: Path,
        audio_files: list[Path],
        metadata_by_path: dict[Path, TrackMetadata],
    ) -> tuple[bool, str, str]:
        """Fix missing metadata by looking up the best candidate and writing tags.

        Falls back to LLM tag generation when no database candidate matches.

        Returns (fixed, source_label, message).
        """
        lookup = LookupService(settings=self.settings)
        candidates = lookup.lookup_album(album_path)
        request = lookup.request_from_path(album_path)

        # Find the best verified candidate (match > close > first available)
        best = self._select_best_candidate(candidates, request.artist_hint)
        if best is None:
            # Fall back to LLM generation when no candidate matches
            return self._fix_via_llm(album_path, audio_files, metadata_by_path, request)

        # Write tags from database candidate
        return self._write_candidate_metadata(
            audio_files, metadata_by_path, best
        )

    def _fix_via_llm(
        self,
        album_path: Path,
        audio_files: list[Path],
        metadata_by_path: dict[Path, TrackMetadata],
        request: LookupRequest,
    ) -> tuple[bool, str, str]:
        """Use LLM to generate tags when no database candidate matches correctly."""
        if not self.settings.llm_api_key:
            return False, "", "No verified candidate and no LLM API key configured"

        from auto_tagger.integrations.fallback import candidate_from_folder
        from auto_tagger.llm.client import OpenRouterClient
        from auto_tagger.llm.fallback import FallbackTagGenerationService

        folder_candidate = candidate_from_folder(request)
        current_metadata = list(metadata_by_path.values())

        try:
            client = OpenRouterClient(self.settings)
            service = FallbackTagGenerationService(client, self.settings)
            result = service.generate_tags(request, folder_candidate, current_metadata)
        except Exception as exc:
            return False, "", f"LLM fallback failed: {exc}"

        if not result.tracks:
            return False, "", f"LLM returned no tracks: {result.reason}"

        # Sort audio files by filename for stable track ordering
        sorted_files = sorted(audio_files, key=lambda p: p.name)

        for i, audio_file in enumerate(sorted_files):
            llm_track = result.tracks[i] if i < len(result.tracks) else result.tracks[-1] if result.tracks else None
            metadata = metadata_by_path.get(audio_file)
            if metadata is None or llm_track is None:
                continue

            new_metadata = TrackMetadata(
                title=llm_track.title or metadata.title,
                artist=llm_track.artist or metadata.artist,
                artists=llm_track.artists or metadata.artists or (
                    [llm_track.artist] if llm_track.artist else []
                ),
                album=llm_track.album or metadata.album,
                album_artist=llm_track.album_artist or llm_track.artist or metadata.album_artist,
                album_artists=(
                    llm_track.album_artists
                    or [llm_track.album_artist]
                    if llm_track.album_artist else []
                ),
                track_number=llm_track.track_number or i + 1,
                track_total=result.tracks[-1].track_number or len(sorted_files) if result.tracks else len(sorted_files),
                year=metadata.year,
                genre=metadata.genre,
            )
            try:
                write_metadata(audio_file, new_metadata, dry_run=False)
            except Exception:
                continue

        return True, "llm", f"Generated via LLM ({result.confidence:.0%} confidence): {result.tracks[0].album}"

    def _write_candidate_metadata(
        self,
        audio_files: list[Path],
        metadata_by_path: dict[Path, TrackMetadata],
        candidate: AlbumCandidate,
    ) -> tuple[bool, str, str]:
        """Write candidate metadata to all audio files.

        Enriches the candidate with genre data from Discogs if missing.
        """
        # Enrich genre from Discogs if candidate has none
        if not candidate.genre:
            enriched = self._enrich_genre_from_discogs(candidate)
            if enriched:
                candidate = enriched

        sorted_files = sorted(audio_files, key=lambda p: p.name)

        for i, audio_file in enumerate(sorted_files):
            candidate_track = candidate.tracks[i] if i < len(candidate.tracks) else None
            metadata = metadata_by_path.get(audio_file)
            if metadata is None:
                continue

            new_metadata = self._merge_candidate_metadata(
                metadata, candidate, candidate_track, i + 1, len(sorted_files),
                force=True,
            )
            try:
                write_metadata(audio_file, new_metadata, dry_run=False)
            except Exception:
                continue

        source_label = f"{candidate.source.value}"
        if candidate.musicbrainz_albumid:
            source_label += f" (MBID: {candidate.musicbrainz_albumid[:8]}...)"
        return True, source_label, f"Fixed via {candidate.source.value}: {candidate.artist} — {candidate.album}"

    @staticmethod
    def _select_best_candidate(
        candidates: list[AlbumCandidate],
        artist_hint: str | None = None,
    ) -> AlbumCandidate | None:
        """Select the best candidate: match with artist > match > close > first non-folder."""
        # Normalize artist hint for comparison
        hint_normalized = artist_hint.casefold().strip() if artist_hint else None

        def _artist_matches(c: AlbumCandidate) -> bool:
            if not hint_normalized or not c.artist:
                return False
            return hint_normalized in c.artist.casefold()

        # Prefer verified match with matching artist
        for c in candidates:
            if c.verification == "match" and c.artist and _artist_matches(c):
                return c
        # Then verified close with matching artist
        for c in candidates:
            if c.verification == "close" and c.artist and _artist_matches(c):
                return c
        # Then any candidate with matching artist (non-folder)
        for c in candidates:
            if c.artist and _artist_matches(c) and c.source != LookupSource.FOLDER:
                return c
        # No candidate matches the artist hint — return None so LLM fallback triggers
        return None

    @staticmethod
    def _merge_candidate_metadata(
        existing: TrackMetadata,
        album_candidate: AlbumCandidate,
        track_candidate: object | None,
        track_number: int,
        track_total: int,
        force: bool = False,
    ) -> TrackMetadata:
        """Merge candidate metadata into a TrackMetadata.

        When force=True (YOLO fix mode), candidate data always takes priority
        over existing tags. When force=False, existing data is preserved.
        """
        from auto_tagger.integrations.candidates import TrackCandidate

        title = existing.title
        artist = existing.artist
        artists = existing.artists
        album = existing.album
        album_artist = existing.album_artist
        album_artists = existing.album_artists

        # Candidate always wins when forcing or when existing is empty
        if force or not artist:
            artist = album_candidate.artist
        if force or not artists:
            artists = album_candidate.artists if album_candidate.artists else (
                [album_candidate.artist] if album_candidate.artist else []
            )
        if force or not album:
            album = album_candidate.album
        if force or not album_artist:
            album_artist = album_candidate.album_artist or album_candidate.artist
        if force or not album_artists:
            album_artists = album_candidate.album_artists if album_candidate.album_artists else (
                [album_artist] if album_artist else []
            )

        if isinstance(track_candidate, TrackCandidate):
            if force or not title:
                title = track_candidate.title
            if force or not artist:
                artist = track_candidate.artist or album_candidate.artist
            if force or not artists:
                artists = (
                    track_candidate.artists
                    if track_candidate.artists
                    else [artist] if artist else []
                )
            track_num = track_candidate.track_number or track_number
            disc_num = track_candidate.disc_number
        else:
            track_num = track_number
            disc_num = None

        return TrackMetadata(
            title=title,
            artist=artist,
            artists=artists,
            album=album,
            album_artist=album_artist,
            album_artists=album_artists,
            track_number=track_num,
            track_total=track_total,
            disc_number=disc_num,
            disc_total=(
                album_candidate.tracks[0].disc_total
                if album_candidate.tracks
                else None
            ),
            year=album_candidate.year if force else (existing.year or album_candidate.year),
            genre=album_candidate.genre if force else (existing.genre or album_candidate.genre),
            musicbrainz_albumid=(
                album_candidate.musicbrainz_albumid
                if force or not existing.musicbrainz_albumid
                else existing.musicbrainz_albumid
            ),
            musicbrainz_artistid=(
                album_candidate.musicbrainz_artistid
                if force or not existing.musicbrainz_artistid
                else existing.musicbrainz_artistid
            ),
        )

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

    @staticmethod
    def _enrich_genre_from_discogs(
        candidate: AlbumCandidate,
    ) -> AlbumCandidate | None:
        """Try to get genre data from Discogs when candidate has none."""
        if not candidate.artist or not candidate.album:
            return None
        try:
            client = DiscogsClient(max_candidates=3)
            discogs_results = client.search_album(candidate.artist, candidate.album)
        except DiscogsError:
            return None

        for dc in discogs_results:
            if dc.genre:
                # Use genre from any Discogs result — even if album name doesn't
                # match exactly, the genre is likely relevant for the artist.
                return AlbumCandidate(
                    artist=candidate.artist,
                    artists=candidate.artists,
                    album=candidate.album,
                    album_artist=candidate.album_artist,
                    album_artists=candidate.album_artists,
                    year=candidate.year,
                    genre=dc.genre,
                    musicbrainz_albumid=candidate.musicbrainz_albumid,
                    musicbrainz_artistid=candidate.musicbrainz_artistid,
                    tracks=candidate.tracks,
                    distance=candidate.distance,
                    source=candidate.source,
                    verification=candidate.verification,
                )
        return None

    @staticmethod
    def _enrich_genre_from_lookup(
        album_path: Path,
        metadata_by_path: dict[Path, TrackMetadata],
    ) -> dict[Path, TrackMetadata] | None:
        """Look up genre from Discogs and enrich metadata if genre is missing."""
        if not metadata_by_path:
            return None

        lookup = LookupService()
        candidates = lookup.lookup_album(album_path)
        request = lookup.request_from_path(album_path)
        best = AlbumWorkflow._select_best_candidate(candidates, request.artist_hint)

        genre = None
        if best and best.genre:
            genre = best.genre
        elif best:
            enriched = AlbumWorkflow._enrich_genre_from_discogs(best)
            if enriched and enriched.genre:
                genre = enriched.genre

        if not genre:
            # Try Discogs search directly for any genre
            try:
                from auto_tagger.integrations.discogs_client import DiscogsClient

                client = DiscogsClient(max_candidates=3)
                if request.artist_hint and request.album_hint:
                    for dc in client.search_album(request.artist_hint, request.album_hint):
                        if dc.genre:
                            genre = dc.genre
                            break
            except Exception:
                pass

        if not genre:
            return None

        updated = {}
        for path, meta in metadata_by_path.items():
            updated[path] = TrackMetadata(
                title=meta.title,
                artist=meta.artist,
                artists=meta.artists,
                album=meta.album,
                album_artist=meta.album_artist,
                album_artists=meta.album_artists,
                track_number=meta.track_number,
                track_total=meta.track_total,
                disc_number=meta.disc_number,
                disc_total=meta.disc_total,
                year=meta.year,
                genre=genre,
                musicbrainz_albumid=meta.musicbrainz_albumid,
                musicbrainz_artistid=meta.musicbrainz_artistid,
                lyrics=meta.lyrics,
                compilation=meta.compilation,
                replaygain=meta.replaygain,
            )
        return updated

    @staticmethod
    def _artist_mismatches_folder(
        album_path: Path,
        metadata_by_path: dict[Path, TrackMetadata],
    ) -> bool:
        """Check if the tagged artist differs from the folder's artist name.

        This detects cases where a previous auto-tag run wrote wrong tags.
        """
        folder_artist = album_path.parent.name
        for metadata in metadata_by_path.values():
            if metadata.artist and metadata.artist.casefold() != folder_artist.casefold():
                return True
        return False
