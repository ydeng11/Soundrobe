"""Audit command — LLM-based metadata quality audit."""

from __future__ import annotations

from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.core.audio import SUPPORTED_EXTENSIONS, load_audio_file
from auto_tagger.core.formats import read_tags
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.core.writer import write_metadata
from auto_tagger.exceptions import TaggingError
from auto_tagger.llm.client import OpenRouterClient
from auto_tagger.llm.prompts import build_audit_messages
from auto_tagger.llm.schemas import AuditResponse
from auto_tagger.utils import console, print_info


def execute(
    settings: Settings,
    path: Path,
    fix: bool = False,
) -> None:
    """Run LLM audit on albums in a library path.

    Args:
        settings: Application settings
        path: Path to library or single album
        fix: Apply LLM-suggested fixes to files
    """
    if not settings.llm_api_key:
        raise TaggingError(
            "LLM API key is required for audit. "
            "Set AUTO_TAG_LLM_API_KEY environment variable or configure in config."
        )

    print_info(f"Auditing: {path}")

    # Discover albums
    audio_extensions = set(SUPPORTED_EXTENSIONS.keys())
    album_map: dict[Path, list[Path]] = {}

    audio_paths = [path] if path.is_file() else list(path.rglob("*"))
    for p in audio_paths:
        if p.is_file() and p.suffix.lower() in audio_extensions:
            parent = p.parent
            if parent not in album_map:
                album_map[parent] = []
            album_map[parent].append(p)

    if not album_map:
        console.print("[yellow]No audio files found[/yellow]")
        return

    albums = sorted(album_map.items())
    total = len(albums)
    console.print(f"  Albums to audit: {total}")
    console.print()

    client = OpenRouterClient(settings)

    total_issues = 0
    total_fixed = 0
    fixed_albums = 0

    from rich.progress import (
        BarColumn,
        Progress,
        SpinnerColumn,
        TaskProgressColumn,
        TextColumn,
        TimeRemainingColumn,
    )

    with Progress(
        SpinnerColumn(),
        TextColumn("[progress.description]{task.description}"),
        BarColumn(),
        TaskProgressColumn(),
        TimeRemainingColumn(),
        console=console,
        transient=True,
    ) as progress:
        task = progress.add_task(
            description="Auditing albums…",
            total=total,
        )

        for i, (album_path, audio_files) in enumerate(albums, 1):
            completed = i - 1
            progress.update(
                task,
                description=f"Auditing [bold]{album_path.name}[/bold]…",
                completed=completed,
            )

            try:
                result, fixed_count = _audit_album(settings, client, album_path, audio_files, fix=fix)
            except Exception as exc:
                progress.update(task, completed=i, description="Auditing albums…")
                console.print(f"  [red]Error auditing {album_path.name}:[/red] {exc}")
                continue

            total_issues += len(result)
            if fixed_count:
                total_fixed += fixed_count
                fixed_albums += 1

            progress.update(task, completed=i, description="Auditing albums…")
            _show_results(album_path, result)

    if fix and total_fixed:
        print_info(
            f"Fixed {total_fixed} issue(s) across {fixed_albums} album(s) "
            f"out of {total_issues} issue(s) found"
        )
    elif fix:
        print_info("No issues fixed — LLM didn't provide suggestions for any flagged items")

    print_info("Audit complete")


def _audit_album(
    settings: Settings,
    client: OpenRouterClient,
    album_path: Path,
    audio_files: list[Path],
    fix: bool = False,
) -> tuple[list[dict], int]:
    """Audit a single album: read tags, call LLM, apply fixes.

    Returns (results, fixed_count) where results are non-correct findings.
    """
    tracks_meta: list[TrackMetadata] = []
    filenames: list[str] = []

    for audio_path in sorted(audio_files):
        try:
            af = load_audio_file(audio_path)
            meta = read_tags(af.format, af.mutagen_file)
        except Exception:
            meta = TrackMetadata()
        tracks_meta.append(meta)
        filenames.append(audio_path.name)

    # Get album hints from directory
    artist_hint = album_path.parent.name
    album_hint = album_path.name

    # Skip if no tracks have meaningful metadata
    if not any(t.title or t.artist for t in tracks_meta):
        return [], 0

    messages = build_audit_messages(artist_hint, album_hint, tracks_meta, filenames)

    response = client.complete_json(messages, AuditResponse)
    audit_data = response.data
    raw_tracks = audit_data.get("tracks", [])

    # Only return non-correct results
    # Preserve 'corrected' so --fix can apply complete metadata overrides
    # Also attach current_value so the display can distinguish real changes
    # from confirmations where the suggestion matches the current value.
    _field_getter = {
        "title": lambda m: m.title,
        "artist": lambda m: m.artist,
        "artists": lambda m: (
            ", ".join(m.artists)
            if m.artists
            else m.artist
        ),
        "album": lambda m: m.album,
        "album_artist": lambda m: m.album_artist,
        "path": lambda idx, _: filenames[idx] if idx < len(filenames) else None,
    }

    results = []
    for t in raw_tracks:
        if t.get("status") not in ("warning", "error"):
            continue
        idx = t["index"]
        field = t["field"]
        getter = _field_getter.get(field)
        if getter is None:
            current_value = None
        elif field == "path":
            current_value = getter(idx, tracks_meta)
        else:
            meta = tracks_meta[idx] if idx < len(tracks_meta) else None
            current_value = getter(meta) if meta is not None else None
        results.append({
            "index": idx,
            "field": field,
            "status": t["status"],
            "message": t.get("message", ""),
            "suggestion": t.get("suggestion"),
            "corrected": t.get("corrected"),
            "current_value": current_value,
        })

    # ── Apply fixes ─────────────────────────────────────────────
    fixed_count = 0
    if fix:
        sorted_files = sorted(audio_files)
        for r in results:
            corrected = r.get("corrected")
            # Prefer full corrected metadata, fall back to per-field suggestion
            if not corrected and not r.get("suggestion"):
                continue
            field = r["field"]
            if field == "path":
                # Can't fix filenames via metadata
                continue
            idx = r["index"]
            if idx < 0 or idx >= len(sorted_files):
                continue
            audio_path = sorted_files[idx]
            try:
                af = load_audio_file(audio_path)
                meta = read_tags(af.format, af.mutagen_file)

                # Start with current metadata
                meta_kwargs: dict = {
                    "title": meta.title,
                    "artist": meta.artist,
                    "artists": meta.artists,
                    "album": meta.album,
                    "album_artist": meta.album_artist,
                    "album_artists": meta.album_artists,
                    "track_number": meta.track_number,
                    "track_total": meta.track_total,
                    "disc_number": meta.disc_number,
                    "disc_total": meta.disc_total,
                    "year": meta.year,
                    "genre": meta.genre,
                    "compilation": meta.compilation,
                    "musicbrainz_albumid": meta.musicbrainz_albumid,
                    "musicbrainz_artistid": meta.musicbrainz_artistid,
                }

                # Apply corrected metadata (LLM provides full corrected values)
                if corrected:
                    for corr_field in ("title", "artist", "album", "album_artist", "year", "genre"):
                        corr_val = corrected.get(corr_field)
                        if corr_val is not None:
                            meta_kwargs[corr_field] = corr_val
                    corr_artists = corrected.get("artists")
                    if corr_artists is not None:
                        meta_kwargs["artists"] = corr_artists
                else:
                    # Fallback: per-field suggestion
                    simple_map = {
                        "title": "title", "artist": "artist", "artists": "artists",
                        "album": "album", "album_artist": "album_artist",
                        "year": "year", "genre": "genre",
                    }
                    attr = simple_map.get(field)
                    if attr is None:
                        continue
                    suggestion = r["suggestion"]
                    meta_kwargs[attr] = suggestion if attr != "artists" else [suggestion]

                new_meta = TrackMetadata(**meta_kwargs)
                write_metadata(audio_path, new_meta, dry_run=False, chinese_script=settings.chinese_script)
                fixed_count += 1
            except Exception:
                continue

    return results, fixed_count


def _show_results(album_path: Path, results: list[dict]) -> None:
    """Display audit results for an album in human-readable format.

    Only shows the suggestion arrow (→) when the suggested value differs
    from the current value. When the suggestion matches the current value
    the entry is shown as a note without an arrow to avoid confusion.
    """
    if not results:
        return

    console.print(f"\n  [bold]{album_path.name}[/bold]")
    for r in results:
        icon = "❌" if r["status"] == "error" else "⚠️"
        suggestion = r.get("suggestion")
        current = r.get("current_value")

        if suggestion and current is not None and str(suggestion) == str(current):
            # Suggestion matches current value — show as note, not as a change
            console.print(
                f"    {icon} Track #{r['index']} {r['field']}: "
                f"{r['message']}  ([dim]current: {current}[/dim])"
            )
        elif suggestion:
            console.print(
                f"    {icon} Track #{r['index']} {r['field']}: "
                f"{r['message']}  [green]→ {suggestion}[/green]"
            )
        else:
            console.print(
                f"    {icon} Track #{r['index']} {r['field']}: "
                f"{r['message']}"
            )



