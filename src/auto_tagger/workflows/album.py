"""Single-album tagging workflow."""

from __future__ import annotations

import json
import re
from collections import defaultdict
from dataclasses import dataclass, field, replace
from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.core import iter_audio_files, load_audio_file, read_metadata, write_metadata
from auto_tagger.core.parse_filename import parse_track_filename
from auto_tagger.core.audio import AudioFormat
from auto_tagger.core.formats import strip_wav_list_chunks
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

# Regex matching CD/Disc subfolder names (e.g. "Artist - Album CD1").
_CD_SUBFOLDER_RE = re.compile(r"(?:[Cc][Dd]|[Dd][Ii][Ss][CcKk]|ディスク)\s*\d+\s*$")
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


def _stem_track_number(stem: str) -> int | None:
    """Extract track number from a filename stem via ``parse_track_filename``."""
    return parse_track_filename(stem).track_number


def _clean_stem(stem: str) -> str:
    """Strip track-number and artist prefix from a filename stem."""
    return parse_track_filename(stem).title or stem


class AlbumWorkflow:
    """Coordinate single-album preview and safe apply behavior."""

    def __init__(self, settings: Settings):
        self.settings = settings

    def run(
        self,
        path: Path,
        dry_run: bool,
        interactive: bool = False,
        force: bool = False,
        artist_mbid_map: dict[str, str] | None = None,
        artist_genre_map: dict[str, list[str]] | None = None,
    ) -> AlbumWorkflowResult:
        """Run album workflow in dry-run, interactive, or YOLO mode.

        Args:
            path: Album directory to process.
            dry_run: If True, only preview changes without writing.
            interactive: If True, prompt before applying changes.
            force: If True, ignore album state cache and reprocess even if tagged.
            artist_mbid_map: Optional mutable dict shared across batch runs.
                Maps all script variants of artist name -> MusicBrainz artist ID.
                Enables cross-album MBID propagation: all albums under the
                same artist folder inherit the same album artist MBID.
            artist_genre_map: Optional mutable dict shared across batch runs.
                Maps artist name -> [genre strings] discovered in previous albums.
                Enables cross-album genre enrichment using known genres as LLM context.
        """
        # ── Album state check: skip if already tagged with same content ──
        if not force and not dry_run:
            from auto_tagger.integrations.cache import MatchCache, _content_hash
            cache = MatchCache(self.settings.cache_path)
            state = cache.get_album_state(path)
            if state is not None and state["status"] == "tagged_ok":
                current_ch = _content_hash(path)
                if state["content_hash"] == current_ch:
                    return AlbumWorkflowResult(
                        album_path=path,
                        audio_files=[],
                        metadata_by_path={},
                        health_report=build_album_health_report(
                            path, [], {}, self.settings
                        ),
                        dry_run=dry_run,
                        planned_writes=0,
                        applied_writes=0,
                        skipped_writes=0,
                        messages=["Skipped (already tagged, content unchanged)"],
                    )

        audio_files = iter_audio_files(path, recursive=self.settings.recursive)
        metadata_by_path = {audio_file: read_metadata(audio_file) for audio_file in audio_files}
        health_report = build_album_health_report(
            path,
            audio_files,
            metadata_by_path,
            self.settings,
        )

        fix_messages: list[str] = []

        # ── Fix duplicate track numbers before exclusion checks ──
        # Run early so that Pattern 3 (discard strays from duplicate groups)
        # does not false-positive on all-track-1 albums: once duplicates are
        # resolved, each file has a unique track number and no duplicate
        # groups exist to trigger the stray-detection heuristic.
        dup_fixed, metadata_by_path, health_report = self._fix_duplicate_track_numbers(
            audio_files, metadata_by_path, health_report, dry_run,
        )
        if dup_fixed:
            fix_messages.append(
                "Renumbered tracks (filename prefixes or sequential)"
            )

        # ── Fix track_number from filename prefix ──────────────────────
        # Some files come with pre-existing metadata where track_number is
        # wrong (e.g. TRACK=11 in the tag but the file is actually track 6
        # per the filename "06.一首歌,让你带回去"). Fix these before exclusion
        # so they aren't flagged as "exceeds total" and deleted.
        #
        # Heuristic: if the filename has a clear track number prefix and it
        # differs from the metadata track_number, use the filename number.
        track_num_fixed = False
        for af in audio_files:
            meta = metadata_by_path.get(af)
            if meta is None:
                continue
            fn_track = _stem_track_number(af.stem)
            if fn_track is not None and meta.track_number is not None and fn_track != meta.track_number:
                # Use the filename track number — it's more reliable
                fixed = replace(meta, track_number=fn_track)
                if not dry_run:
                    try:
                        write_metadata(af, fixed, dry_run=False)
                        metadata_by_path[af] = fixed
                        track_num_fixed = True
                    except Exception:
                        continue

        if track_num_fixed:
            fix_messages.append("Fixed track_number from filename prefix")
            health_report = build_album_health_report(
                path, audio_files, metadata_by_path, self.settings
            )

        # ── Exclude tracks that don't belong to this album ───────────────
        # Several patterns indicate a file does not belong to the album:
        #   1. track_number > track_total (stray from another release)
        #   2. disc_number > disc_total
        #   3. duplicate track number where one file has disc_number=None
        #      and another has a real disc number (the disc=None one is stray)
        #   4. missing track_number on a single-track album — auto-assign
        #
        # These tracks are excluded from processing and deleted in YOLO mode.
        excluded_paths: set[Path] = set()

        # Pattern 3: duplicate track number caused by missing disc number.
        # When two files share the same track number (same disc) and one
        # has disc_number=None while the other has a real disc number,
        # the disc=None file is likely a stray from another disc/release.
        #
        # BUT: only apply this heuristic when the album actually uses
        # multiple discs (any file has disc > 1). When all files are on
        # disc 1, a missing disc_number just means incomplete metadata —
        # the file is NOT a stray and should not be deleted.
        dup_code = "metadata.duplicate_track_number"
        # Check if this album has multiple discs (disc > 1 anywhere)
        album_has_multi_disc = any(
            meta.disc_number is not None and meta.disc_number > 1
            for meta in metadata_by_path.values()
        )
        for issue in health_report.issues:
            if issue.code != dup_code:
                continue
            dup_paths = [Path(p) for p in (issue.details.get("paths") or [])]
            if len(dup_paths) < 2:
                continue
            # Check if some have disc_number=None while others have a real disc
            has_disc: set[Path] = set()
            no_disc: set[Path] = set()
            for dp in dup_paths:
                meta = metadata_by_path.get(dp)
                if meta is not None and meta.disc_number is not None:
                    has_disc.add(dp)
                else:
                    no_disc.add(dp)
            # If some have disc and some don't, the disc=None ones are stray
            # ONLY when this is a multi-disc album. Single-disc albums
            # with incomplete disc_number metadata are not strays.
            if has_disc and no_disc and album_has_multi_disc:
                excluded_paths.update(no_disc)

        # Pattern 4: missing track number. For single-track folders,
        # auto-assign track_number=1 immediately so the lookup path
        # doesn't have to re-discover it.
        missing_code = "metadata.missing_track_number"
        for th in health_report.track_health:
            for issue in th.issues:
                if issue.code != missing_code:
                    continue
                if len(audio_files) == 1 and not dry_run:
                    meta = metadata_by_path.get(th.path)
                    if meta is not None:
                        fixed = replace(meta, track_number=1, track_total=1)
                        write_metadata(th.path, fixed, dry_run=False)
                        metadata_by_path[th.path] = fixed
                    break

        if excluded_paths:
            audio_files = [f for f in audio_files if f not in excluded_paths]
            for ep in excluded_paths:
                metadata_by_path.pop(ep, None)
            # Rebuild health report without excluded tracks
            health_report = build_album_health_report(
                path,
                audio_files,
                metadata_by_path,
                self.settings,
            )

        planned_writes = len(audio_files)
        applied_writes = 0
        wrote_all = False  # tracks whether we already wrote metadata via the fix path

        if excluded_paths:
            excluded_names = ", ".join(p.name for p in sorted(excluded_paths))
            if not dry_run and self.settings.yolo and not interactive:
                deleted = 0
                for ep in sorted(excluded_paths):
                    try:
                        ep.unlink()
                        deleted += 1
                    except Exception:
                        pass
                fix_messages.append(
                    f"Deleted {deleted} track(s) not belonging to album: {excluded_names}"
                )
            else:
                fix_messages.append(
                    f"Would delete {len(excluded_paths)} track(s) not belonging to album: {excluded_names}"
                )

        # YOLO mode: fix tags via lookup cascade, or write existing metadata
        if not dry_run and self.settings.yolo and not interactive:
            # Check if tags need fixing (health errors, missing MBID, bad genre,
            # or artist mismatch)
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
                health_report.has_blocking_errors
                or self._artist_mismatches_folder(path, metadata_by_path)
                or has_bad_genre
                or has_missing_mbid
            )

            if needs_fix:
                fixed, source, msg, strays = self._fix_metadata(
                    path, audio_files, metadata_by_path,
                    artist_mbid_map=artist_mbid_map,
                    artist_genre_map=artist_genre_map,
                )
                if fixed:
                    fix_messages.append(msg)

                    # ── Remove stray files not belonging to the album ──
                    # Database is ground truth. Files that didn't match any
                    # candidate track are strays — delete in YOLO mode.
                    if strays:
                        strays_sorted = sorted(strays)
                        stray_names = ", ".join(p.name for p in strays_sorted)
                        audio_files = [f for f in audio_files if f not in strays]
                        for s in strays:
                            metadata_by_path.pop(s, None)
                            try:
                                s.unlink()
                            except Exception:
                                pass
                        fix_messages.append(
                            f"Deleted {len(strays)} stray track(s) not belonging to album: {stray_names}"
                        )

                    # Re-read metadata and rebuild health after fix
                    metadata_by_path = {
                        audio_file: read_metadata(audio_file) for audio_file in audio_files
                    }
                    health_report = build_album_health_report(
                        path, audio_files, metadata_by_path, self.settings
                    )
                    applied_writes = len(audio_files)
                    wrote_all = True

            if not wrote_all:
                # ── No database match found — fallback: bump stale_total ──
                # When no MusicBrainz/Discogs candidate (and no LLM result)
                # exists, fall back to existing metadata. Apply the stale
                # track_total heuristic to avoid false-positive "exceeds total"
                # on bonus tracks.
                tracks_by_disc: dict[int, list[Path]] = {}
                for af in audio_files:
                    meta = metadata_by_path.get(af)
                    disc = (meta.disc_number if meta is not None and meta.disc_number is not None else 1)
                    tracks_by_disc.setdefault(disc, []).append(af)

                stale_fixed = False
                for disc, disc_files in tracks_by_disc.items():
                    n_files = len(disc_files)
                    first_meta = metadata_by_path.get(disc_files[0])
                    current_total = (first_meta.track_total if first_meta is not None else None)
                    if current_total is not None and n_files > current_total:
                        for af in disc_files:
                            meta = metadata_by_path.get(af)
                            if meta is None:
                                continue
                            fixed = replace(meta, track_total=n_files)
                            try:
                                write_metadata(af, fixed, dry_run=False)
                                metadata_by_path[af] = fixed
                                stale_fixed = True
                            except Exception:
                                continue

                if stale_fixed:
                    fix_messages.append(
                        f"Corrected track_total to {n_files} (was {current_total}) — fallback (no database match)"
                    )

                # Write/enrich whatever metadata we have
                first_meta = next(iter(metadata_by_path.values()), None)
                needs_enrich = False
                if first_meta:
                    if (
                        first_meta.genre is None
                        or first_meta.genre in bad_genres
                    ):
                        needs_enrich = True
                    elif not first_meta.year:
                        needs_enrich = True

                if needs_enrich:
                    try:
                        enriched = self._enrich_genre_from_lookup(path, metadata_by_path, discogs_token=self.settings.discogs_token)
                        if enriched:
                            metadata_by_path = enriched
                            fix_messages.append("Enriched genre/year from Discogs")
                    except Exception:
                        pass

                for audio_file, metadata in metadata_by_path.items():
                    write_metadata(audio_file, metadata, dry_run=False)
                    applied_writes += 1
                wrote_all = True

        # ── Post-YOLO: enforce filename-based track numbers ────────────
        # The YOLO fix (lookup cascade) may have written candidate track
        # numbers that disagree with filename prefixes. This happens when
        # the candidate has a different track ordering or the file has
        # pre-existing metadata with a wrong track number. Re-apply the
        # filename-derived track number as the authoritative value.
        if not dry_run and self.settings.yolo and not interactive:
            post_fixed = False
            for af in audio_files:
                meta = metadata_by_path.get(af)
                if meta is None:
                    continue
                fn_track = _stem_track_number(af.stem)
                if fn_track is not None and meta.track_number is not None and fn_track != meta.track_number:
                    fixed = replace(meta, track_number=fn_track)
                    try:
                        write_metadata(af, fixed, dry_run=False)
                        metadata_by_path[af] = fixed
                        post_fixed = True
                    except Exception:
                        continue
            if post_fixed:
                fix_messages.append("Fixed track_number from filename (post-YOLO)")
                health_report = build_album_health_report(
                    path, audio_files, metadata_by_path, self.settings
                )

        skipped_writes = planned_writes - applied_writes if not dry_run else 0

        # Cover art fix: run in yolo mode regardless of metadata health
        cover_art_fixed = False
        cover_art_status = ""
        cover_art_message = ""
        if not dry_run and self.settings.yolo and not interactive:
            cover_art_fixed, cover_art_status, cover_art_message = self._fix_cover_art(
                path, audio_files, metadata_by_path
            )

        # ── Update album state after successful tagging ────────────────
        if not dry_run and self.settings.yolo and not interactive and applied_writes > 0:
            from auto_tagger.integrations.cache import MatchCache
            disc_count = 0
            try:
                # Count sibling directories in parent that look like CD subdirs
                parent = path.parent
                if parent and parent.is_dir():
                    disc_count = sum(
                        1 for d in parent.iterdir()
                        if d.is_dir() and any(
                            pat in d.name for pat in ("CD", "Disc", "disc", "cd")
                        )
                    )
            except Exception:
                pass
            try:
                MatchCache(self.settings.cache_path).set_album_state(
                    path, status="tagged_ok", disc_count=disc_count,
                )
            except Exception:
                pass

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

    def _fix_duplicate_track_numbers(
        self,
        audio_files: list[Path],
        metadata_by_path: dict[Path, TrackMetadata],
        health_report: AlbumHealthReport,
        dry_run: bool,
    ) -> tuple[bool, dict[Path, TrackMetadata], AlbumHealthReport]:
        """Fix duplicate track numbers using two strategies:

        **Strategy 1 (prefix-based):** Extract track numbers from leading
        digits in filenames (e.g. ``01 Song.flac`` → 1) and apply if the
        filename-based numbers are present on all duplicate files, unique
        within the group, and don't conflict with non-duplicate tracks.

        **Strategy 2 (sequential fallback):** When Strategy 1 resolves nothing
        and all files on a disc share the SAME track number (e.g. all
        track_number=1), assign them sequentially 1..N based on filename
        order. This is safe because all-tracks-same-number is unambiguously
        a metadata error.

        Returns (fixed, updated_metadata_by_path, updated_health_report).
        """
        dup_code = "metadata.duplicate_track_number"
        has_any_dup = any(i.code == dup_code for i in health_report.issues)
        if not has_any_dup:
            return False, metadata_by_path, health_report

        # ── Strategy 1: Prefix-based renumbering ──────────────────────

        # Build map of existing track numbers for non-duplicate conflict checking
        existing_numbers: dict[Path, int | None] = {}
        for af in audio_files:
            meta = metadata_by_path.get(af)
            if meta is not None and meta.track_number is not None:
                existing_numbers[af] = meta.track_number

        fixed = False
        fixed_paths: set[Path] = set()

        for issue in health_report.issues:
            if issue.code != dup_code:
                continue

            dup_paths = [Path(p) for p in (issue.details.get("paths") or [])]
            if len(dup_paths) < 2:
                continue

            # Extract stem numbers for these paths
            stem_map: dict[Path, int | None] = {}
            for dp in dup_paths:
                stem_map[dp] = _stem_track_number(dp.stem)

            # All must have stem numbers
            if any(v is None for v in stem_map.values()):
                continue

            # Stem numbers must be unique within the group
            nums = [v for v in stem_map.values() if v is not None]
            if len(nums) != len(set(nums)):
                continue

            # Check stem numbers don't conflict with non-duplicate tracks on same disc
            non_dup_paths = set(existing_numbers.keys()) - set(dup_paths)
            existing_non_dup_numbers = {
                existing_numbers[p] for p in non_dup_paths
                if existing_numbers[p] is not None
            }
            if any(n in existing_non_dup_numbers for n in nums):
                continue

            # Apply renumbering
            for dp in dup_paths:
                if dp in fixed_paths:
                    continue
                stem_num = stem_map[dp]
                if stem_num is None:
                    continue
                meta = metadata_by_path.get(dp)
                if meta is None:
                    continue

                fixed_meta = TrackMetadata(
                    title=meta.title, artist=meta.artist,
                    artists=meta.artists,
                    album=meta.album, album_artist=meta.album_artist,
                    album_artists=meta.album_artists,
                    track_number=stem_num,
                    track_total=meta.track_total,
                    disc_number=meta.disc_number,
                    disc_total=meta.disc_total,
                    year=meta.year, genre=meta.genre,
                    musicbrainz_trackid=meta.musicbrainz_trackid,
                    musicbrainz_albumid=meta.musicbrainz_albumid,
                    musicbrainz_artistid=meta.musicbrainz_artistid,
                    lyrics=meta.lyrics,
                    compilation=meta.compilation,
                    replaygain=meta.replaygain,
                )
                if not dry_run:
                    try:
                        write_metadata(dp, fixed_meta, dry_run=False)
                        metadata_by_path[dp] = fixed_meta
                        fixed_paths.add(dp)
                        fixed = True
                    except Exception:
                        # Skip files that can't be written (corrupt, unsupported)
                        continue

        # ── Strategy 2: Sequential fallback ───────────────────────────
        # If Strategy 1 didn't fix everything, check if there are discs where
        # ALL files share the same track number (e.g. all track=1). This is
        # clearly a metadata error — assign sequential numbers 1..N.

        if not fixed or any(i.code == dup_code for i in health_report.issues):
            seq_fixed = self._fix_duplicate_tracks_sequential(
                audio_files, metadata_by_path, health_report, dry_run,
            )
            if seq_fixed:
                fixed = True

        if not fixed:
            return False, metadata_by_path, health_report

        updated_health = build_album_health_report(
            health_report.album_path,
            audio_files,
            metadata_by_path,
            self.settings,
        )
        return True, metadata_by_path, updated_health

    def _fix_duplicate_tracks_sequential(
        self,
        audio_files: list[Path],
        metadata_by_path: dict[Path, TrackMetadata],
        health_report: AlbumHealthReport,
        dry_run: bool,
    ) -> bool:
        """Sequential track renumbering fallback.

        When all files on a given effective disc (treating disc=None as
        disc=1) share the same track number (e.g. all track_number=1),
        assign them sequentially 1..N based on filename sort order. This
        is safe because all-tracks-same-number on the same disc is
        unambiguously a metadata error.

        Normalises disc_number: files with disc=None are treated as
        disc=1 for grouping, and are also written with disc=1 so the
        health check (which defaults None to 1) doesn't create
        overlapping duplicate groups.

        Returns True if any tracks were renumbered.
        """
        dup_code = "metadata.duplicate_track_number"

        # Normalise disc_number: treat None as 1 for grouping
        def _effective_disc(path: Path) -> int:
            meta = metadata_by_path.get(path)
            return (meta.disc_number if meta is not None and meta.disc_number is not None else 1)

        # Group files by effective disc number
        files_by_disc: dict[int, list[Path]] = defaultdict(list)
        for af in audio_files:
            files_by_disc[_effective_disc(af)].append(af)

        # Collect all paths that appear in duplicate-track-number issues
        existing_dup_paths: set[Path] = set()
        for issue in health_report.issues:
            if issue.code != dup_code:
                continue
            for p in (issue.details.get("paths") or []):
                existing_dup_paths.add(Path(p))

        any_fixed = False

        for disc, disc_files in files_by_disc.items():
            if len(disc_files) < 2:
                continue

            # Check if ALL files on this effective disc share the same
            # track number (treating disc=None as disc=1)
            track_numbers: set[int | None] = set()
            for af in disc_files:
                meta = metadata_by_path.get(af)
                tn = meta.track_number if meta is not None else None
                track_numbers.add(tn)

            if len(track_numbers) != 1:
                continue
            common_tn = next(iter(track_numbers))
            if common_tn is None:
                continue

            # Verify at least one of these files is still in a
            # duplicate-track-number issue
            if not any(af in existing_dup_paths for af in disc_files):
                continue

            # Sort files by filename for consistent sequential numbering
            sorted_files = sorted(disc_files, key=lambda p: p.name)

            for track_num, af in enumerate(sorted_files, start=1):
                meta = metadata_by_path.get(af)
                if meta is None:
                    continue

                # Normalise disc_number to 1 when it was None
                new_disc = meta.disc_number if meta.disc_number is not None else 1

                fixed_meta = TrackMetadata(
                    title=meta.title, artist=meta.artist,
                    artists=meta.artists,
                    album=meta.album, album_artist=meta.album_artist,
                    album_artists=meta.album_artists,
                    track_number=track_num,
                    track_total=len(sorted_files),
                    disc_number=new_disc,
                    disc_total=meta.disc_total,
                    year=meta.year, genre=meta.genre,
                    musicbrainz_trackid=meta.musicbrainz_trackid,
                    musicbrainz_albumid=meta.musicbrainz_albumid,
                    musicbrainz_artistid=meta.musicbrainz_artistid,
                    lyrics=meta.lyrics,
                    compilation=meta.compilation,
                    replaygain=meta.replaygain,
                )
                if not dry_run:
                    try:
                        write_metadata(af, fixed_meta, dry_run=False)
                        metadata_by_path[af] = fixed_meta
                        any_fixed = True
                    except Exception:
                        continue

        return any_fixed

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
                token=self.settings.discogs_token,
                timeout_seconds=self.settings.cover_art_timeout_seconds,
            )
            result = discogs.fetch_cover_art(artist_name, album_name)
            if result.status == CoverArtStatus.FETCHED_REMOTE and result.image is not None:
                self._embed_into_all(audio_files, result.image)
                return (
                    True,
                    CoverArtStatus.FETCHED_REMOTE,
                    "Fetched and embedded cover from Discogs",
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
    ) -> tuple[bool, str, str, list[Path]]:
        """Fix missing metadata by looking up the best candidate and writing tags.

        Falls back to LLM tag generation when no database candidate matches.
        Propagates MusicBrainz artist IDs across albums via artist_mbid_map.
        Enriches genre from Discogs / LLM with cross-album genre context.
        Enforces the folder artist as the canonical album artist.

        Returns (fixed, source_label, message, stray_paths).
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
    ) -> tuple[bool, str, str, list[Path]]:
        """Use LLM to generate tags when no database candidate matches correctly.

        Enforces the folder artist as the canonical album artist and propagates
        MB artist IDs from the cross-album map. Enriches genre with LLM context
        from known-genre albums by the same artist.

        Files that match an LLM track are tagged. Files that do NOT match
        any LLM track are returned as strays — they don't belong to this
        album and should be excluded (deleted in YOLO mode).

        Returns (fixed, source_label, message, stray_paths).
        """
        if not self.settings.llm_api_key:
            return False, "", "No verified candidate and no LLM API key configured", []

        from auto_tagger.integrations.fallback import candidate_from_folder
        from auto_tagger.llm.fallback import FallbackTagGenerationService

        folder_candidate = candidate_from_folder(request)
        current_metadata = list(metadata_by_path.values())

        try:
            client = OpenRouterClient(self.settings)
            service = FallbackTagGenerationService(client, self.settings)
            result = service.generate_tags(request, folder_candidate, current_metadata)
        except Exception as exc:
            return False, "", f"LLM fallback failed: {exc}", []

        if not result.tracks:
            return False, "", f"LLM returned no tracks: {result.reason}", []

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

        # Match LLM tracks to audio files using multi-signal scoring
        match_map = self._match_tracks_to_files(
            audio_files, metadata_by_path, result.tracks,
        )

        # ── Partition files into matched and stray ──────────────────
        # LLM tracks are the ground truth. Matched files get tagged.
        # Unmatched files are strays — they don't belong to this album.
        matched_files: list[Path] = []
        strays: list[Path] = []
        for f in audio_files:
            if match_map.get(f) is not None:
                matched_files.append(f)
            else:
                strays.append(f)

        track_total = max(
            (t.track_number or 0) for t in result.tracks if t.track_number
        ) if any(t.track_number for t in result.tracks) else len(matched_files)
        llm_album_genre = result.tracks[0].genre if result.tracks else None

        for idx, audio_file in enumerate(matched_files):
            matched_track = match_map.get(audio_file)
            metadata = metadata_by_path.get(audio_file)
            if metadata is None:
                continue

            title = (
                getattr(matched_track, "title", None)
                if matched_track
                else None
            ) or metadata.title
            track_number = (
                getattr(matched_track, "track_number", None)
                if matched_track
                else None
            ) or metadata.track_number
            track_artist = (
                getattr(matched_track, "artist", None)
                if matched_track
                else None
            ) or metadata.artist
            if matched_track:
                matched_artists = getattr(matched_track, "artists", None)
            else:
                matched_artists = None
            track_artists = (
                list(matched_artists)
                if matched_artists
                else metadata.artists or ([track_artist] if track_artist else [])
            )

            # For collaborations, use smart-tagged per-track artist/artists
            if is_collaboration and smart_tagged_tracks and idx < len(smart_tagged_tracks):
                st = smart_tagged_tracks[idx]
                track_artist = st.artist or track_artist
                track_artists = st.artists or track_artists

            # Fall back to filename stem when LLM didn't provide a title
            if not title:
                title = _clean_stem(audio_file.stem)

            # For non-compilation albums, track artist matches the normalized
            # album artist (folder name), not the raw LLM output.
            if is_collaboration:
                # For collaborations, preserve the smart-tagged artist/artists
                llm_artist_normalized = track_artist
                llm_artists_normalized = track_artists
            elif not analysis.is_compilation:
                # For non-compilation multi-artist albums, preserve per-track artist
                # when the LLM returned distinct values for each track
                if track_artist and effective_album_artist and track_artist.strip() != effective_album_artist.strip():
                    llm_artist_normalized = track_artist
                    llm_artists_normalized = track_artists or [track_artist]
                else:
                    llm_artist_normalized = effective_album_artist
                    llm_artists_normalized = (
                        [effective_album_artist] if effective_album_artist else []
                    )
            else:
                llm_artist_normalized = track_artist
                llm_artists_normalized = track_artists

            new_metadata = TrackMetadata(
                title=title,
                artist=llm_artist_normalized,
                artists=llm_artists_normalized,
                album=metadata.album,
                album_artist=effective_album_artist,
                album_artists=effective_album_artists,
                track_number=track_number,
                track_total=track_total,
                year=metadata.year,
                genre=llm_album_genre or metadata.genre,
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
                for audio_file in matched_files:
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

        return True, "llm", f"Generated via LLM ({result.confidence:.0%} confidence): {result.tracks[0].album}", strays

    def _write_candidate_metadata(
        self,
        audio_files: list[Path],
        metadata_by_path: dict[Path, TrackMetadata],
        candidate: AlbumCandidate,
        artist_mbid_map: dict[str, str] | None = None,
        folder_artist: str | None = None,
        artist_genre_map: dict[str, list[str]] | None = None,
    ) -> tuple[bool, str, str, list[Path]]:
        """Write candidate metadata to matched audio files.

        Uses the database (MusicBrainz / Discogs) candidate as ground truth.
        Files that match a candidate track are tagged with the candidate's
        metadata. Files that do NOT match any candidate track are returned
        as strays — they don't belong to this album and should be excluded
        (deleted in YOLO mode).

        ``track_total`` is set to the max track number from the candidate's
        track list (database ground truth), never bumped to match file count.

        Enforces the folder artist as the canonical album artist (or "Various Artists"
        for compilations) and propagates MB artist IDs across albums via
        artist_mbid_map. Enriches genre from Discogs, then LLM with cross-album
        genre context.

        Returns (fixed, source_label, message, stray_paths).
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
            enriched = self._enrich_genre_from_discogs(candidate, discogs_token=self.settings.discogs_token)
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

        match_map = self._match_tracks_to_files(
            audio_files, metadata_by_path, candidate.tracks,
        )

        # ── Partition files into matched and stray ──────────────────
        # Database candidate is ground truth. Matched files get tagged.
        # Unmatched files are strays — they don't belong to this album.
        matched_files: list[Path] = []
        strays: list[Path] = []
        for f in audio_files:
            if match_map.get(f) is not None:
                matched_files.append(f)
            else:
                strays.append(f)

        track_total = max(
            (int(t.track_number) for t in candidate.tracks if t.track_number is not None)
        ) if any(t.track_number is not None for t in candidate.tracks) else len(matched_files)

        for idx, audio_file in enumerate(matched_files):
            matched_track = match_map.get(audio_file)
            metadata = metadata_by_path.get(audio_file)
            if metadata is None:
                continue

            new_metadata = self._merge_candidate_metadata(
                metadata, candidate, matched_track,
                track_number=(
                    getattr(matched_track, "track_number", None)
                    if matched_track
                    else None
                ) or metadata.track_number or 1,
                track_total=track_total,
                force=True,
            )

            # Fall back to filename stem when no candidate matched
            title = new_metadata.title
            if not title:
                title = _clean_stem(audio_file.stem)

            # For collaborations, use smart-tagged per-track artist/artists
            if is_collaboration and smart_tagged_tracks and idx < len(smart_tagged_tracks):
                st = smart_tagged_tracks[idx]
                track_artist = st.artist or new_metadata.artist
                track_artists = st.artists or new_metadata.artists
            elif not analysis.is_compilation:
                # For non-compilation albums, check if the matched track has
                # a distinct per-track artist. If so, preserve it — this handles
                # multi-artist albums where different tracks have different performers
                # (e.g. 拉阔演奏厅: 陈慧琳 on tracks 1-6, 陈小春 on tracks 7+).
                matched_track_artist = (
                    getattr(matched_track, "artist", None)
                    if matched_track
                    else None
                )
                if (
                    matched_track_artist
                    and candidate.artist
                    and matched_track_artist.strip() != candidate.artist.strip()
                ):
                    track_artist = matched_track_artist
                    matched_track_artists = (
                        getattr(matched_track, "artists", None)
                        if matched_track
                        else None
                    )
                    track_artists = matched_track_artists or [matched_track_artist]
                else:
                    # Single-artist album: track artist matches the normalized
                    # album artist (folder name), not the raw candidate artist.
                    track_artist = effective_album_artist
                    track_artists = [effective_album_artist] if effective_album_artist else new_metadata.artists
            else:
                track_artist = new_metadata.artist
                track_artists = new_metadata.artists

            # Enforce album artist based on compilation analysis
            new_metadata = TrackMetadata(
                title=title,
                artist=track_artist,
                artists=track_artists,
                album=new_metadata.album,
                album_artist=effective_album_artist,
                album_artists=effective_album_artists,
                track_number=new_metadata.track_number,
                track_total=track_total,
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
        return True, source_label, f"Fixed via {candidate.source.value}: {candidate.artist} — {candidate.album}", strays

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
    def _match_tracks_to_files(
        audio_files: list[Path],
        metadata_by_path: dict[Path, TrackMetadata],
        candidates: list,
        min_confidence: float = 0.25,
    ) -> dict[Path, object | None]:
        """Match audio files to candidate tracks using multi-signal scoring.

        Scoring signals (in priority order):
          1. Exact filename match (short-circuit) — if cleaned file stem equals
             candidate title, score=1.0 immediately, no greedy matrix needed.
          2. Filename similarity (weight 0.6) — token overlap (Jaccard).
          3. Track number match (weight 0.2) — exact match of track numbers,
             falling back to track number parsed from filename prefix.
          4. Duration proximity (weight 0.2) — tiebreaker, decays to 0 at 15s.

        Track numbers are extracted from filename prefixes when no existing
        tag is present (e.g. "01 爷爷.wav" → track_number=1). The prefix is
        also stripped before filename-vs-title comparison.

        Greedy assignment: the best (file, candidate) pair is locked first,
        then both are removed, and the process repeats. Files below
        *min_confidence* return None (no match / keep existing title).

        Works with both TrackCandidate and TrackMetadata objects — uses
        getattr for the optional ``length`` field (only TrackCandidate has it).

        Returns dict mapping each audio file to its best-matching candidate
        or None when no match meets the confidence threshold.
        """
        if not candidates:
            return {f: None for f in audio_files}

        import re
        import unicodedata

        def _sc(text: str) -> str:
            """Convert text to Simplified Chinese for token matching.
            Gracefully degrades if opencc is not installed.
            """
            try:
                import opencc  # type: ignore[import-untyped]
                return opencc.OpenCC("t2s").convert(text)  # type: ignore[no-any-return]
            except Exception:
                return text

        # ── helpers ──────────────────────────────────────────────
        def _normalize(text: str) -> str:
            """Normalize: NFKC, SC conversion, strip punctuation, lowercase."""
            t = unicodedata.normalize("NFKC", text)
            t = _sc(t)
            t = re.sub(r"[^\w\s\u4e00-\u9fff]", " ", t)
            return re.sub(r"\s+", " ", t).strip().lower()

        # _clean_stem and _stem_track_number are now module-level helpers

        def _filename_similarity(file_stem: str, candidate_title: str) -> float:
            """Token overlap (Jaccard) between filename stem and candidate title (0-1)."""
            a_tokens = set(_normalize(file_stem).split())
            b_tokens = set(_normalize(candidate_title).split())
            if not a_tokens or not b_tokens:
                return 0.0
            intersection = a_tokens & b_tokens
            union = a_tokens | b_tokens
            return len(intersection) / len(union) if union else 0.0

        def _duration_similarity(file_dur: float | None, cand_len: object) -> float:
            """Duration proximity score (0-1). 1.0 = exact match, decays to 0 at 15s diff.

            Defensive against string-typed lengths from some lookup clients.
            """
            if file_dur is None or cand_len is None:
                return 0.0
            try:
                cl = float(str(cand_len))
            except (TypeError, ValueError):
                return 0.0
            if cl <= 0:
                return 0.0
            return max(0.0, 1.0 - abs(file_dur - cl) / 15.0)

        # ── Pre-compute file metadata ────────────────────────────
        file_durations: dict[Path, float | None] = {}
        file_stems: dict[Path, str] = {}  # cleaned stem (track prefix removed)
        file_track_numbers: dict[Path, int | None] = {}  # from filename prefix
        for f in audio_files:
            try:
                af = load_audio_file(f)
                dur = getattr(af.mutagen_file.info, "length", None)
                file_durations[f] = float(dur) if dur is not None else None
            except Exception:
                file_durations[f] = None
            file_stems[f] = _clean_stem(f.stem)
            file_track_numbers[f] = _stem_track_number(f.stem)

        # Build candidate lookup by track number for exact match
        cand_by_tracknum: dict[int, object] = {}
        for c in candidates:
            tn = getattr(c, "track_number", None)
            if tn is not None:
                cand_by_tracknum[tn] = c

        # ── Phase 1: Exact filename match (short-circuit) ────────
        # If cleaned filename stem equals a candidate title after
        # normalization, lock immediately. This is the most reliable signal.
        assignment: dict[int, int] = {}  # file_idx -> candidate_idx
        used_candidates: set[int] = set()

        for fi, fpath in enumerate(audio_files):
            cleaned = _normalize(file_stems[fpath])
            if not cleaned:
                continue
            for ci, cand in enumerate(candidates):
                if ci in used_candidates:
                    continue
                cand_title = _normalize(getattr(cand, "title", None) or "")
                if cleaned == cand_title:
                    assignment[fi] = ci
                    used_candidates.add(ci)
                    break

        # ── Phase 2: Track number match (from filename prefix) ───
        # If file has a track number prefix AND a candidate has the same
        # track number, lock it (unless already assigned).
        for fi, fpath in enumerate(audio_files):
            if fi in assignment:
                continue
            fn_track = file_track_numbers[fpath]
            if fn_track is not None and fn_track in cand_by_tracknum:
                cand = cand_by_tracknum[fn_track]
                for ci, c in enumerate(candidates):
                    if ci in used_candidates:
                        continue
                    if c is cand:
                        # Score the match first — only lock if plausible
                        meta = metadata_by_path.get(fpath)
                        dur = file_durations.get(fpath)
                        cand_dur = getattr(cand, "length", None)
                        dur_score = _duration_similarity(dur, cand_dur)
                        fn_score = _filename_similarity(
                            file_stems[fpath],
                            getattr(cand, "title", None) or "",
                        )
                        # Combined score: filename (0.6) + duration (0.2) + track (0.2)
                        score = (fn_score * 0.6 + dur_score * 0.2 + 1.0 * 0.2) / 1.0
                        if score >= min_confidence:
                            assignment[fi] = ci
                            used_candidates.add(ci)
                        break

        # ── Phase 3: Matrix-based greedy for remaining files ─────
        remaining_files = [
            (fi, f) for fi, f in enumerate(audio_files) if fi not in assignment
        ]
        remaining_candidates = [
            (ci, c) for ci, c in enumerate(candidates) if ci not in used_candidates
        ]

        while remaining_files and remaining_candidates:
            best_score = -1.0
            best_fi = best_ci = -1

            for ri, (orig_fi, fpath) in enumerate(remaining_files):
                meta = metadata_by_path.get(fpath)
                dur = file_durations.get(fpath)
                cleaned_stem = file_stems[fpath]
                fn_track = file_track_numbers[fpath]
                existing_track = meta.track_number if meta else None

                for ci, (orig_ci, cand) in enumerate(remaining_candidates):
                    cand_dur = getattr(cand, "length", None)
                    cand_title = getattr(cand, "title", None) or ""
                    cand_tn = getattr(cand, "track_number", None)

                    # Filename vs title: primary signal (weight 0.6)
                    fn_score = _filename_similarity(cleaned_stem, cand_title)

                    # Duration: tiebreaker (weight 0.2)
                    dur_score = _duration_similarity(dur, cand_dur)

                    # Track number: exact (weight 0.2)
                    track_score = 0.0
                    if (
                        existing_track is not None
                        and cand_tn is not None
                        and existing_track == cand_tn
                    ):
                        track_score = 1.0
                    elif (
                        fn_track is not None
                        and cand_tn is not None
                        and fn_track == cand_tn
                    ):
                        track_score = 1.0

                    score = fn_score * 0.6 + dur_score * 0.2 + track_score * 0.2

                    if score > best_score:
                        best_score = score
                        best_fi = ri
                        best_ci = ci

            if best_score < min_confidence or best_fi < 0:
                break

            orig_fi, _ = remaining_files.pop(best_fi)
            orig_ci, _ = remaining_candidates.pop(best_ci)
            assignment[orig_fi] = orig_ci

        result: dict[Path, object | None] = {}
        for i, f in enumerate(audio_files):
            result[f] = candidates[assignment[i]] if i in assignment else None

        return result

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
                if af.format == AudioFormat.WAV:
                    strip_wav_list_chunks(audio_file)
            except Exception:
                continue

    @staticmethod
    def _enrich_genre_from_discogs(
        candidate: AlbumCandidate,
        discogs_token: str | None = None,
    ) -> AlbumCandidate | None:
        """Try to get genre data from Discogs when candidate has none."""
        if not candidate.artist or not candidate.album:
            return None
        try:
            client = DiscogsClient(
                token=discogs_token,
                max_candidates=3,
            )
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
            client = DiscogsClient(
                token=self.settings.discogs_token,
                max_candidates=3,
            )
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
        discogs_token: str | None = None,
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
            enriched = AlbumWorkflow._enrich_genre_from_discogs(best, discogs_token=discogs_token)
            if enriched and enriched.genre:
                genre = enriched.genre
                year = enriched.year

        if not genre:
            # Try Discogs search directly using the original hint
            try:
                if request.artist_hint and request.album_hint:
                    discogs = DiscogsClient(
                        token=discogs_token,
                        max_candidates=3,
                    )
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

        For CD subfolder paths (e.g. "Album CD1"), the grandparent folder
        is used as the artist reference instead of the immediate parent.
        """
        if (
            album_path.name
            and _CD_SUBFOLDER_RE.search(album_path.name)
            and album_path.parent.parent
        ):
            folder_artist = album_path.parent.parent.name
        else:
            folder_artist = album_path.parent.name
        for metadata in metadata_by_path.values():
            # Check track artist — the most common signal
            if metadata.artist:
                if not artist_matches_any(metadata.artist, folder_artist):
                    return True
            # Also check album artist — a previous auto-tag run may have
            # written the album bundle folder name ("Artist.Album 2CD")
            # instead of just the artist name.  The fuzzy alias matching
            # in artist_matches_any may miss this because "陈慧琳.GRACE"
            # shares many characters with "陈慧琳".  Do a direct check
            # for CD/Disc bundle patterns, which never appear in a
            # legitimate artist name.
            if metadata.album_artist:
                if not artist_matches_any(metadata.album_artist, folder_artist):
                    return True
                # Direct heuristic: if folder_artist is a prefix of
                # album_artist followed by a delimiter and extra text,
                # the album_artist is likely the bundle folder name.
                aa_norm = metadata.album_artist.casefold().strip()
                fa_norm = folder_artist.casefold().strip()
                if aa_norm != fa_norm and aa_norm.startswith(fa_norm):
                    suffix = aa_norm[len(fa_norm):]
                    if suffix and suffix[0] in ". _-":
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
