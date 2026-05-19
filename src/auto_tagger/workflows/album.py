"""Single-album tagging workflow."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.core import iter_audio_files, load_audio_file, read_metadata, write_metadata
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.features.compilations import analyze_compilation, apply_smart_album_tags
from auto_tagger.features.cover_art import (
    CoverArtArchiveClient,
    CoverArtImage,
    CoverArtStatus,
    discover_local_cover_art,
    embed_cover_art,
)
from auto_tagger.integrations import LookupService
from auto_tagger.integrations.aliases import artist_matches_any, get_aliases, save_alias
from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest, LookupSource
from auto_tagger.integrations.discogs_client import DiscogsClient, DiscogsError
from auto_tagger.llm.client import OpenRouterClient
from auto_tagger.llm.schemas import GenreEnrichmentResponse
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


def _artist_variant_keys(name: str) -> list[str]:
    """Generate all variant keys for an artist name for map lookups/storage.

    Returns the original casefolded name plus all script variants (SC, TC, etc.)
    and any known aliases. Ensures lookups match regardless of the script
    variant used by album folders vs lookup results.
    """
    import unicodedata
    keys: list[str] = []
    raw = name.strip()
    if not raw:
        return keys

    norm = unicodedata.normalize("NFKC", raw.casefold())
    keys.append(norm)

    try:
        import opencc
        for cfg in ("s2t", "t2s", "s2tw", "tw2s", "s2hk", "hk2s"):
            try:
                conv = opencc.OpenCC(cfg)
                converted = unicodedata.normalize("NFKC", conv.convert(raw).casefold().strip())
                if converted and converted not in keys:
                    keys.append(converted)
            except Exception:
                continue
    except Exception:
        pass

    # Add known aliases
    for alias in get_aliases(raw):
        alias_norm = unicodedata.normalize("NFKC", alias.casefold().strip())
        if alias_norm not in keys:
            keys.append(alias_norm)

    return keys


def _store_mbid_in_map(
    artist_mbid_map: dict[str, str],
    artist_name: str,
    mbid: str,
) -> None:
    """Store an MB artist ID in the map under all script-variant keys.

    Stores the MBID under every variant of *artist_name* (original, SC, TC,
    HK, TW, known aliases) so lookups always succeed regardless of which
    script variant the next album uses.
    """
    for key in _artist_variant_keys(artist_name):
        if key:
            artist_mbid_map.setdefault(key, mbid)


def _lookup_mbid_in_map(
    artist_mbid_map: dict[str, str] | None,
    artist_name: str | None,
) -> str | None:
    """Look up an MB artist ID using any script-variant key of *artist_name*.

    Checks every variant (original, SC, TC, HK, TW, known aliases) against
    the map so that e.g. a folder named 久石让 finds the MBID stored under
    久石譲.
    """
    if artist_mbid_map is None or not artist_name:
        return None
    for key in _artist_variant_keys(artist_name):
        mbid = artist_mbid_map.get(key)
        if mbid:
            return mbid
    return None



def _store_genre_in_map(
    artist_genre_map: dict[str, list[str]],
    artist_name: str,
    genre: str,
) -> None:
    """Store a genre string for an artist in the cross-album genre map.

    Deduplicates genres — if the same genre string is already stored
    for this artist, it is not added again.
    """
    if not genre or not artist_name:
        return
    genres = artist_genre_map.setdefault(artist_name.casefold().strip(), [])
    if genre not in genres:
        genres.append(genre)


def _get_context_genres(
    artist_genre_map: dict[str, list[str]] | None,
    artist_name: str | None,
) -> list[str]:
    """Get deduplicated genres known for an artist from the cross-album map.

    Returns an empty list if the artist has no known genres or the map
    is not available.
    """
    if artist_genre_map is None or not artist_name:
        return []
    return artist_genre_map.get(artist_name.casefold().strip(), [])


class AlbumWorkflow:
    """Coordinate single-album preview and safe apply behavior."""

    def __init__(self, settings: Settings):
        self.settings = settings

    def run(
        self,
        path: Path,
        dry_run: bool,
        interactive: bool = False,
        artist_mbid_map: dict[str, str] | None = None,
        artist_genre_map: dict[str, list[str]] | None = None,
    ) -> AlbumWorkflowResult:
        """Run album workflow in dry-run, interactive, or YOLO mode.

        Args:
            path: Album directory to process.
            dry_run: If True, only preview changes without writing.
            interactive: If True, prompt before applying changes.
            artist_mbid_map: Optional mutable dict shared across batch runs.
                Maps all script variants of artist name -> MusicBrainz artist ID.
                Enables cross-album MBID propagation: all albums under the
                same artist folder inherit the same album artist MBID.
            artist_genre_map: Optional mutable dict shared across batch runs.
                Maps artist name -> [genre strings] discovered in previous albums.
                Enables cross-album genre enrichment using known genres as LLM context.
        """
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
        has_missing_mbid = any(
            m.musicbrainz_artistid is None or m.musicbrainz_albumid is None
            for m in metadata_by_path.values()
        )
        needs_fix = (
            not health_report.can_tag
            or self._artist_mismatches_folder(path, metadata_by_path)
            or has_bad_genre
            or has_missing_mbid
        )
        if not dry_run and self.settings.yolo and not interactive and needs_fix:
            fixed, source, msg = self._fix_metadata(
                path, audio_files, metadata_by_path,
                artist_mbid_map=artist_mbid_map,
                artist_genre_map=artist_genre_map,
            )
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
            # Check if genre or year is missing — do a quick lookup to enrich
            first_meta = next(iter(metadata_by_path.values()), None)
            needs_enrich = False
            if first_meta:
                if (
                    first_meta.genre is None
                    or first_meta.genre in ("未知流派", "unknown", "Unknown", "?")
                ):
                    needs_enrich = True
                elif not first_meta.year:
                    # Year is missing but genre exists — still try to enrich
                    # since Discogs also provides year data.
                    needs_enrich = True

            if needs_enrich:
                try:
                    enriched = self._enrich_genre_from_lookup(path, metadata_by_path)
                    if enriched:
                        metadata_by_path = enriched
                        fix_messages.append("Enriched genre/year from Discogs")
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
        artist_mbid_map: dict[str, str] | None = None,
        artist_genre_map: dict[str, list[str]] | None = None,
    ) -> tuple[bool, str, str]:
        """Fix missing metadata by looking up the best candidate and writing tags.

        Falls back to LLM tag generation when no database candidate matches.
        Propagates MusicBrainz artist IDs across albums via artist_mbid_map.
        Enriches genre from Discogs / LLM with cross-album genre context.
        Enforces the folder artist as the canonical album artist.

        Returns (fixed, source_label, message).
        """
        lookup = LookupService(settings=self.settings)
        candidates = lookup.lookup_album(album_path)
        request = lookup.request_from_path(album_path)

        # The folder name is the definitive artist for this album
        folder_artist = request.artist_hint

        # Find the best verified candidate (match > close > first available)
        best = self._select_best_candidate(candidates, folder_artist)
        if best is None:
            # Fall back to LLM generation when no candidate matches
            return self._fix_via_llm(
                album_path, audio_files, metadata_by_path, request,
                artist_mbid_map=artist_mbid_map,
                artist_genre_map=artist_genre_map,
            )

        # Inject MB artist ID if the candidate is missing one.
        # Try the cross-album map first, then resolve via MusicBrainz API.
        if not best.musicbrainz_artistid:
            mapped_id = self._ensure_artist_mbid(folder_artist, artist_mbid_map)
            if not mapped_id:
                mapped_id = self._ensure_artist_mbid(best.artist, artist_mbid_map)
            if mapped_id:
                best = AlbumCandidate(
                    artist=best.artist,
                    artists=best.artists,
                    album=best.album,
                    album_artist=best.album_artist,
                    album_artists=best.album_artists,
                    year=best.year,
                    genre=best.genre,
                    musicbrainz_albumid=best.musicbrainz_albumid,
                    musicbrainz_artistid=mapped_id,
                    tracks=best.tracks,
                    distance=best.distance,
                    source=best.source,
                    verification=best.verification,
                )

        # Write tags from database candidate, enforcing folder artist
        return self._write_candidate_metadata(
            audio_files, metadata_by_path, best,
            artist_mbid_map=artist_mbid_map,
            folder_artist=folder_artist,
            artist_genre_map=artist_genre_map,
        )

    def _fix_via_llm(
        self,
        album_path: Path,
        audio_files: list[Path],
        metadata_by_path: dict[Path, TrackMetadata],
        request: LookupRequest,
        artist_mbid_map: dict[str, str] | None = None,
        artist_genre_map: dict[str, list[str]] | None = None,
    ) -> tuple[bool, str, str]:
        """Use LLM to generate tags when no database candidate matches correctly.

        Enforces the folder artist as the canonical album artist and propagates
        MB artist IDs from the cross-album map. Enriches genre with LLM context
        from known-genre albums by the same artist.
        """
        if not self.settings.llm_api_key:
            return False, "", "No verified candidate and no LLM API key configured"

        from auto_tagger.integrations.fallback import candidate_from_folder
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

        folder_artist = request.artist_hint
        llm_artist = result.tracks[0].artist or result.tracks[0].album_artist

        # Learn artist alias if the LLM used a different name than the hint
        if llm_artist and folder_artist:
            save_alias(folder_artist, llm_artist)

        # Resolve MB artist ID: check map first, then MusicBrainz API
        folder_mbid = self._ensure_artist_mbid(folder_artist, artist_mbid_map)

        # Store MBID in map for cross-album propagation
        if folder_mbid and folder_artist and artist_mbid_map is not None:
            _store_mbid_in_map(artist_mbid_map, folder_artist, folder_mbid)

        # Determine if this is a compilation based on track-level artists
        analysis = analyze_compilation(result.tracks, album_path_hint=str(request.path))

        if analysis.is_compilation:
            effective_album_artist = "Various Artists"
            effective_album_artists = ["Various Artists"]
            is_collaboration = False
            smart_tagged_tracks = None
        elif analysis.is_collaboration:
            is_collaboration = True
            smart_tagged_tracks = apply_smart_album_tags(result.tracks, analysis)
            effective_album_artist = smart_tagged_tracks[0].album_artist or (
                folder_artist or llm_artist or ""
            )
            effective_album_artists = smart_tagged_tracks[0].album_artists or (
                [effective_album_artist] if effective_album_artist else []
            )
        else:
            is_collaboration = False
            smart_tagged_tracks = None
            effective_album_artist = folder_artist or llm_artist or ""
            effective_album_artists = [effective_album_artist] if effective_album_artist else []

        # Sort audio files by filename for stable track ordering
        sorted_files = sorted(audio_files, key=lambda p: p.name)

        for i, audio_file in enumerate(sorted_files):
            llm_track = result.tracks[i] if i < len(result.tracks) else result.tracks[-1] if result.tracks else None
            metadata = metadata_by_path.get(audio_file)
            if metadata is None or llm_track is None:
                continue

            # For collaborations, use smart-tagged per-track artist/artists
            if is_collaboration and smart_tagged_tracks and i < len(smart_tagged_tracks):
                st = smart_tagged_tracks[i]
                track_artist = st.artist or llm_track.artist or metadata.artist
                track_artists = st.artists or llm_track.artists or metadata.artists or (
                    [track_artist] if track_artist else []
                )
            else:
                track_artist = llm_track.artist or metadata.artist
                track_artists = llm_track.artists or metadata.artists or (
                    [track_artist] if track_artist else []
                )

            new_metadata = TrackMetadata(
                title=llm_track.title or metadata.title,
                artist=track_artist,
                artists=track_artists,
                album=llm_track.album or metadata.album,
                album_artist=effective_album_artist,
                album_artists=effective_album_artists,
                track_number=llm_track.track_number or i + 1,
                track_total=result.tracks[-1].track_number or len(sorted_files),
                year=metadata.year,
                genre=llm_track.genre or metadata.genre,
                compilation=analysis.is_compilation,
                musicbrainz_artistid=folder_mbid or metadata.musicbrainz_artistid,
            )
            try:
                write_metadata(audio_file, new_metadata, dry_run=False)
            except Exception:
                continue

        # Store genre in cross-album map for enrichment of subsequent albums
        genre = result.tracks[0].genre if result.tracks else None
        if genre and artist_genre_map is not None and folder_artist:
            _store_genre_in_map(artist_genre_map, folder_artist, genre)

        # Enrich genre from Discogs/LLM if the LLM output didn't include one
        if not any(t.genre for t in result.tracks):
            known_genres = _get_context_genres(artist_genre_map, folder_artist)
            enriched_genre = self._enrich_genre_fallback(
                folder_artist or llm_artist or "",
                result.tracks[0].album or "",
                known_genres=known_genres,
            )
            if enriched_genre:
                if artist_genre_map is not None and folder_artist:
                    _store_genre_in_map(artist_genre_map, folder_artist, enriched_genre)
                for audio_file in sorted_files:
                    meta = read_metadata(audio_file)
                    if meta and meta.genre is None:
                        enriched = TrackMetadata(
                            title=meta.title, artist=meta.artist, artists=meta.artists,
                            album=meta.album,
                            album_artist=effective_album_artist,
                            album_artists=effective_album_artists,
                            track_number=meta.track_number, track_total=meta.track_total,
                            disc_number=meta.disc_number, disc_total=meta.disc_total,
                            year=meta.year, genre=enriched_genre,
                            compilation=analysis.is_compilation,
                            musicbrainz_albumid=meta.musicbrainz_albumid,
                            musicbrainz_artistid=meta.musicbrainz_artistid,
                        )
                        try:
                            write_metadata(audio_file, enriched, dry_run=False)
                        except Exception:
                            continue

        return True, "llm", f"Generated via LLM ({result.confidence:.0%} confidence): {result.tracks[0].album}"

    def _write_candidate_metadata(
        self,
        audio_files: list[Path],
        metadata_by_path: dict[Path, TrackMetadata],
        candidate: AlbumCandidate,
        artist_mbid_map: dict[str, str] | None = None,
        folder_artist: str | None = None,
        artist_genre_map: dict[str, list[str]] | None = None,
    ) -> tuple[bool, str, str]:
        """Write candidate metadata to all audio files.

        Enforces the folder artist as the canonical album artist (or "Various Artists"
        for compilations) and propagates MB artist IDs across albums via
        artist_mbid_map. Enriches genre from Discogs, then LLM with cross-album
        genre context.

        Returns (fixed, source_label, message).
        """
        # Update cross-album MB artist ID map with all script variants
        if artist_mbid_map is not None and candidate.musicbrainz_artistid:
            # Store under the candidate artist name
            if candidate.artist:
                _store_mbid_in_map(artist_mbid_map, candidate.artist, candidate.musicbrainz_artistid)
            # Also store under the folder artist if different (handles SC/TC variants)
            if folder_artist and candidate.artist and not artist_matches_any(candidate.artist, folder_artist):
                _store_mbid_in_map(artist_mbid_map, folder_artist, candidate.musicbrainz_artistid)

        # Analyze compilation pattern (compilation, collaboration, classical, etc.)
        tracks_for_analysis = [
            TrackMetadata(
                title=t.title or "",
                artist=t.artist,
                artists=t.artists,
                album=candidate.album or "",
                album_artist=candidate.album_artist,
                album_artists=candidate.album_artists,
                track_number=t.track_number,
            )
            for t in candidate.tracks
        ]
        analysis = analyze_compilation(
            tracks_for_analysis,
            album_path_hint=candidate.album or "",
        )

        if analysis.is_compilation:
            effective_album_artist = "Various Artists"
            effective_album_artists = ["Various Artists"]
            is_collaboration = False
            smart_tagged_tracks = None
        elif analysis.is_collaboration:
            is_collaboration = True
            smart_tagged_tracks = apply_smart_album_tags(tracks_for_analysis, analysis)
            effective_album_artist = smart_tagged_tracks[0].album_artist or (
                folder_artist or candidate.album_artist or candidate.artist or ""
            )
            effective_album_artists = smart_tagged_tracks[0].album_artists or (
                [effective_album_artist] if effective_album_artist else []
            )
        else:
            is_collaboration = False
            smart_tagged_tracks = None
            effective_album_artist = folder_artist or candidate.album_artist or candidate.artist or ""
            effective_album_artists = [effective_album_artist] if effective_album_artist else []

        # Enrich genre from Discogs if candidate has none
        if not candidate.genre:
            enriched = self._enrich_genre_from_discogs(candidate)
            if enriched:
                candidate = enriched

        # If still no genre, fall back to LLM genre suggestion with context
        if not candidate.genre:
            known_genres = _get_context_genres(artist_genre_map, folder_artist)
            llm_genre = self._enrich_genre_from_llm(
                candidate.artist or "", candidate.album or "",
                known_genres=known_genres,
            )
            if llm_genre:
                candidate = AlbumCandidate(
                    artist=candidate.artist,
                    artists=candidate.artists,
                    album=candidate.album,
                    album_artist=candidate.album_artist,
                    album_artists=candidate.album_artists,
                    year=candidate.year,
                    genre=llm_genre,
                    musicbrainz_albumid=candidate.musicbrainz_albumid,
                    musicbrainz_artistid=candidate.musicbrainz_artistid,
                    tracks=candidate.tracks,
                    distance=candidate.distance,
                    source=candidate.source,
                    verification=candidate.verification,
                )

        # Store genre in cross-album map for enrichment of subsequent albums
        if candidate.genre and artist_genre_map is not None and folder_artist:
            _store_genre_in_map(artist_genre_map, folder_artist, candidate.genre)

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

            # For collaborations, use smart-tagged per-track artist/artists
            if is_collaboration and smart_tagged_tracks and i < len(smart_tagged_tracks):
                st = smart_tagged_tracks[i]
                track_artist = st.artist or new_metadata.artist
                track_artists = st.artists or new_metadata.artists
            else:
                track_artist = new_metadata.artist
                track_artists = new_metadata.artists

            # Enforce album artist based on compilation analysis
            new_metadata = TrackMetadata(
                title=new_metadata.title,
                artist=track_artist,
                artists=track_artists,
                album=new_metadata.album,
                album_artist=effective_album_artist,
                album_artists=effective_album_artists,
                track_number=new_metadata.track_number,
                track_total=new_metadata.track_total,
                disc_number=new_metadata.disc_number,
                disc_total=new_metadata.disc_total,
                year=new_metadata.year,
                genre=new_metadata.genre,
                compilation=analysis.is_compilation,
                musicbrainz_albumid=new_metadata.musicbrainz_albumid,
                musicbrainz_artistid=(
                    _lookup_mbid_in_map(artist_mbid_map, folder_artist)
                    or new_metadata.musicbrainz_artistid
                ),
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
        """Select the best candidate: match with artist > match > close > first non-folder.

        Artist matching checks both direct name match and known aliases
        (e.g. hint "蔡健雅" matches candidate artist "Tanya Chua" via alias).
        """
        def _artist_matches(c: AlbumCandidate) -> bool:
            return artist_matches_any(c.artist, artist_hint)

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
                # Prefer the Discogs year over the candidate year (Discogs data
                # is often richer, especially for folder-derived candidates).
                return AlbumCandidate(
                    artist=candidate.artist,
                    artists=candidate.artists,
                    album=candidate.album,
                    album_artist=candidate.album_artist,
                    album_artists=candidate.album_artists,
                    year=dc.year or candidate.year,
                    genre=dc.genre,
                    musicbrainz_albumid=candidate.musicbrainz_albumid,
                    musicbrainz_artistid=candidate.musicbrainz_artistid,
                    tracks=candidate.tracks,
                    distance=candidate.distance,
                    source=candidate.source,
                    verification=candidate.verification,
                )
        return None

    def _enrich_genre_from_llm(
        self, artist: str, album: str,
        known_genres: list[str] | None = None,
    ) -> str | None:
        """Ask the LLM to suggest genre tags for an album.

        When *known_genres* is provided (from other albums by the same artist
        processed earlier in the batch), the LLM selects the best match from
        that list rather than generating from scratch. This improves accuracy
        and reduces cost.

        Returns a Discogs-style genre string (e.g. 'Electronic, Ambient,
        Modern Classical') or None if the API is unavailable or uncertain.
        """
        if not self.settings.llm_api_key:
            return None

        payload: dict[str, object] = {
            "artist": artist,
            "album": album,
        }
        if known_genres:
            payload["known_genres"] = known_genres

        if known_genres:
            system_prompt = (
                "Suggest a genre for this album. You are given known genres "
                "from other albums by the same artist. Select the best matching "
                "genre from the known list, or return null if none fit. "
                "Output Discogs-style comma-separated tags "
                "(e.g. 'Electronic, House, Deep House'). "
                "Return JSON with a single 'genre' field (string or null)."
            )
        else:
            system_prompt = (
                "Suggest a genre for this album. Output Discogs-style "
                "comma-separated tags (e.g. 'Electronic, House, Deep House' "
                "or 'Stage & Screen, Score, Contemporary Classical'). "
                "Return JSON with a single 'genre' field (string or null). "
                "Return null if you are not confident."
            )

        try:
            client = OpenRouterClient(self.settings)
            response = client.complete_json(
                [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": json.dumps(payload, ensure_ascii=False)},
                ],
                GenreEnrichmentResponse,
            )
            parsed = GenreEnrichmentResponse.model_validate(response.data)
            return parsed.genre if parsed.genre else None
        except Exception:
            return None

    def _enrich_genre_fallback(
        self,
        artist: str,
        album: str,
        known_genres: list[str] | None = None,
    ) -> str | None:
        """Try to get genre from Discogs, then fall back to LLM.

        When falling back to the LLM, provides known genres from other albums
        by the same artist as context for more accurate selection.

        Returns a genre string or None.
        """
        # Try Discogs first
        try:
            client = DiscogsClient(max_candidates=3)
            dc_results = client.search_album(artist, album)
            for dc in dc_results:
                if dc.genre:
                    return dc.genre
        except Exception:
            pass

        # Fall back to LLM genre suggestion with known-genre context
        return self._enrich_genre_from_llm(artist, album, known_genres=known_genres)

    @staticmethod
    def _enrich_genre_from_lookup(
        album_path: Path,
        metadata_by_path: dict[Path, TrackMetadata],
    ) -> dict[Path, TrackMetadata] | None:
        """Look up genre from Discogs and enrich metadata if genre is missing.

        Falls back to LLM genre suggestion when Discogs returns nothing.
        """
        if not metadata_by_path:
            return None

        lookup = LookupService()
        candidates = lookup.lookup_album(album_path)
        request = lookup.request_from_path(album_path)
        best = AlbumWorkflow._select_best_candidate(candidates, request.artist_hint)

        genre = None
        year = None
        if best and best.genre:
            genre = best.genre
            year = best.year
        elif best:
            enriched = AlbumWorkflow._enrich_genre_from_discogs(best)
            if enriched and enriched.genre:
                genre = enriched.genre
                year = enriched.year

        if not genre:
            # Try Discogs search directly using the original hint
            try:
                if request.artist_hint and request.album_hint:
                    discogs = DiscogsClient(max_candidates=3)
                    for dc in discogs.search_album(request.artist_hint, request.album_hint):
                        if dc.genre:
                            genre = dc.genre
                            year = dc.year or year
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
                year=year or meta.year,
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
        Uses artist_matches_any to handle SC/TC and alias matching.
        """
        folder_artist = album_path.parent.name
        for metadata in metadata_by_path.values():
            if metadata.artist:
                if not artist_matches_any(metadata.artist, folder_artist):
                    return True
        return False

    @staticmethod
    def _ensure_artist_mbid(
        artist_name: str | None,
        artist_mbid_map: dict[str, str] | None,
    ) -> str | None:
        """Resolve a MusicBrainz artist ID for an artist name.

        Checks the cross-album map first. If not found, queries the
        MusicBrainz API via musicbrainzngs with the artist name and
        stores the result in the map for future lookups.

        For Chinese artists with no learned aliases, also harvests
        alias data from MusicBrainz (English names, Pinyin, TC)
        and persists them via `save_alias()` so downstream lookups
        (Discogs, etc.) can use the English/Latin name.

        Returns the MBID (UUID string) or None if the artist is not
        found in MusicBrainz.
        """
        if not artist_name:
            return None

        # Check cross-album map first
        if artist_mbid_map is not None:
            mapped = _lookup_mbid_in_map(artist_mbid_map, artist_name)
            if mapped:
                return mapped

        # Query MusicBrainz API via musicbrainzngs
        try:
            import musicbrainzngs as mb  # type: ignore[import-untyped]

            try:
                mb.set_useragent(
                    "auto-tagger", "0.1.0",
                    "https://github.com/auto-tagger/auto-tagger",
                )
            except Exception:
                pass

            result = mb.search_artists(artist=artist_name, limit=1)
            artist_list = result.get("artist-list") or []
            if artist_list:
                mbid = artist_list[0].get("id")
                if mbid and artist_mbid_map is not None:
                    _store_mbid_in_map(artist_mbid_map, artist_name, mbid)

                # Seed aliases from MusicBrainz for Chinese artists with no existing aliases.
                # This bridges Chinese→English name gap for Discogs lookups.
                from auto_tagger.integrations.aliases import (
                    get_aliases,
                    is_chinese_name,
                    save_alias,
                )

                if is_chinese_name(artist_name) and not get_aliases(artist_name):
                    alias_list = artist_list[0].get("alias-list") or []
                    for alias_entry in alias_list:
                        alias_text = alias_entry.get("alias") or alias_entry.get("name")
                        if alias_text and alias_text.strip():
                            save_alias(artist_name, alias_text.strip())

                return mbid  # type: ignore[no-any-return]
        except Exception:
            pass

        return None
