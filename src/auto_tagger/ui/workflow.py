"""Subprocess management for auto-tag and audit operations."""

from __future__ import annotations

import asyncio
import json
import sys
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from auto_tagger.ui.screens.main_screen import MainScreen

_running_proc: asyncio.subprocess.Process | None = None


def cancel_running() -> None:
    """Cancel the currently running subprocess (if any)."""
    global _running_proc
    if _running_proc is not None and _running_proc.returncode is None:
        try:
            _running_proc.terminate()
        except ProcessLookupError:
            pass


async def run_auto_tag(screen: MainScreen, album_path: Path | None = None) -> None:
    """Run auto-tag as a subprocess with JSON streaming.

    If *album_path* is provided, tags only that single album directory.
    Otherwise, tags the entire loaded library (``state.library_path``).

    Launches ``auto-tag batch <path> --json-stream --yolo``, reads
    incremental JSON lines, and updates the UI as results arrive.
    """
    global _running_proc

    state = screen.app.state  # type: ignore[attr-defined]
    target = album_path or state.library_path

    if not target or (not album_path and not state.library_path):
        screen.notify("No library loaded", severity="error")
        return

    if album_path and album_path not in state.albums:
        screen.notify("Album not found in library", severity="error")
        return

    state.auto_tagging = True
    screen.refresh_bindings()
    label = album_path.name if album_path else (state.library_path.name if state.library_path else "library")
    screen.notify(f"Starting auto-tag: {label}...", severity="information")

    # Save pre-state snapshots for undo (affected albums only)
    if album_path:
        snapshots = _build_snapshots(state, {album_path})
        undo_label = f"Auto-Tag Album: {album_path.name}"
    else:
        snapshots = _build_snapshots(state)
        undo_label = "Auto-Tag: " + (state.library_path.name if state.library_path else "library")
    undo_manager = screen.app.undo_manager  # type: ignore[attr-defined]
    undo_manager.push(undo_label, snapshots)

    try:
        args = [sys.executable, "-m", "auto_tagger", "batch", str(target), "--json-stream", "--yolo"]
        if getattr(state, "debug_enabled", False):
            args.append("--debug")

        _running_proc = await asyncio.create_subprocess_exec(
            *args,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )

        if _running_proc.stdout is None:
            screen.notify("Failed to launch auto-tag", severity="error")
            state.auto_tagging = False
            screen.refresh_bindings()
            return

        # Read JSON lines incrementally with cancellation support
        reader_task = asyncio.create_task(
            _read_json_stream(_running_proc.stdout, screen, _handle_event)
        )

        # Also read stderr to avoid buffer deadlock
        stderr_task = asyncio.create_task(
            _read_stderr(_running_proc.stderr, screen)
        )

        # Wait for either completion or cancellation
        done, pending = await asyncio.wait(
            [reader_task, stderr_task],
            return_when=asyncio.FIRST_COMPLETED,
        )

        for task in pending:
            task.cancel()

        await _running_proc.wait()

    except asyncio.CancelledError:
        screen.notify("Auto-tag cancelled", severity="warning")
    except Exception as exc:
        screen.notify(f"Auto-tag failed: {exc}", severity="error")
    finally:
        # Save return code before clearing global so we can detect
        # user-initiated stop (SIGTERM = -15).
        was_stopped_by_user = (
            _running_proc is not None
            and _running_proc.returncode == -15
        ) if _running_proc is not None else False

        _running_proc = None
        state.auto_tagging = False
        screen.refresh_bindings()

        # Only start audit when tagging the whole library (not a single album)
        # and only if the user didn't explicitly stop the operation.
        if (not album_path and not was_stopped_by_user
                and state.auto_audit_enabled):
            screen.notify("Starting audit...", severity="information")
            await run_audit(screen)


async def run_audit(screen: MainScreen) -> None:
    """Run LLM audit on the library after auto-tag completes."""
    global _running_proc

    state = screen.app.state  # type: ignore[attr-defined]
    if not state.library_path:
        return

    state.auditing = True
    screen.refresh_bindings()

    try:
        _running_proc = await asyncio.create_subprocess_exec(
            sys.executable,
            "-m", "auto_tagger",
            "audit",
            str(state.library_path),
            "--json-stream",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )

        if _running_proc.stdout is None:
            state.auditing = False
            screen.refresh_bindings()
            return

        reader_task = asyncio.create_task(
            _read_json_stream(_running_proc.stdout, screen, _handle_audit_event)
        )
        stderr_task = asyncio.create_task(
            _read_stderr(_running_proc.stderr, screen)
        )

        done, pending = await asyncio.wait(
            [reader_task, stderr_task],
            return_when=asyncio.FIRST_COMPLETED,
        )

        for task in pending:
            task.cancel()

        await _running_proc.wait()

    except Exception as exc:
        screen.notify(f"Audit failed: {exc}", severity="warning")
    finally:
        _running_proc = None
        state.auditing = False
        screen.refresh_bindings()
        screen.notify("Audit complete", severity="information")


async def _read_json_stream(
    stream: asyncio.StreamReader,
    screen: MainScreen,
    handler,
) -> None:
    """Read JSON lines from a subprocess stream and hand them to a handler."""
    while True:
        line = await stream.readline()
        if not line:
            break

        raw = line.decode("utf-8", errors="replace").strip()
        if not raw:
            continue

        try:
            event = json.loads(raw)
            await handler(screen, event)
        except json.JSONDecodeError:
            pass


async def _read_stderr(
    stream: asyncio.StreamReader | None,
    screen: MainScreen,
) -> None:
    """Read and log stderr output from the subprocess."""
    if stream is None:
        return
    while True:
        line = await stream.readline()
        if not line:
            break
        raw = line.decode("utf-8", errors="replace").strip()
        if raw:
            screen.notify(f"[stderr] {raw}", severity="warning", timeout=2)


def _build_snapshots(state, album_paths: set[Path] | None = None) -> list:
    """Build undo snapshots for all (or a subset of) loaded tracks.

    If *album_paths* is given, only snapshots tracks in those albums.
    """
    from auto_tagger.ui.undo import TrackSnapshot

    snapshots = []
    for album_path, album in state.albums.items():
        if album_paths is not None and album_path not in album_paths:
            continue
        for track in album.tracks:
            snapshots.append(
                TrackSnapshot(path=track.path, metadata=track.metadata)
            )
    return snapshots


async def _handle_event(screen: MainScreen, event: dict) -> None:
    """Process a JSON event from the auto-tag subprocess."""
    state = screen.app.state  # type: ignore[attr-defined]
    event_type = event.get("type")

    if event_type == "album":
        album_path = Path(event.get("path", ""))
        if album_path in state.albums:
            album = state.albums[album_path]
            es = event.get("status", "ok")
            changes = event.get("changes", 0)
            album.status = "ok" if es == "ok" else ("error" if es == "error" else album.status)

            # Re-read metadata from disk when changes were applied
            if changes > 0 and album.tracks:
                _reload_tracks(album)

            # Update per-track changed state
            for tdata in event.get("tracks", []):
                tp = Path(tdata["path"])
                track = next((t for t in album.tracks if t.path == tp), None)
                if track:
                    track.status = "ok"

            # Refresh display
            screen.refresh_bindings()

    elif event_type == "progress":
        state.auto_tag_progress = event.get("current", 0)
        state.auto_tag_total = event.get("total", 0)
        screen.refresh_bindings()

    elif event_type == "summary":
        screen.notify(
            f"Processed: {event.get('processed', '?')} albums, "
            f"{event.get('failed', 0)} failed",
            severity="information",
        )


def _reload_tracks(album) -> None:
    """Re-read track metadata from disk after auto-tag changes."""
    from auto_tagger.core.audio import load_audio_file
    from auto_tagger.core.formats import read_tags
    from auto_tagger.ui.state import TrackData

    for track in album.tracks:
        try:
            af = load_audio_file(track.path)
            meta = read_tags(af.format, af.mutagen_file)
            track.metadata = meta
        except Exception:
            pass


async def _handle_audit_event(screen: MainScreen, event: dict) -> None:
    """Process a JSON event from the audit subprocess."""
    state = screen.app.state  # type: ignore[attr-defined]
    event_type = event.get("type")

    if event_type == "progress":
        state.audit_progress = event.get("current", 0)
        state.audit_total = event.get("total", 0)
        screen.refresh_bindings()
        return

    if event_type == "audit":
        album_path = Path(event.get("path", ""))
        if album_path not in state.albums:
            return

        album = state.albums[album_path]
        results = event.get("tracks", [])

        from auto_tagger.ui.state import TrackAuditResult

        for result_data in results:
            result = TrackAuditResult(
                track_index=result_data.get("index", 0),
                field=result_data.get("field", ""),
                status=result_data.get("status", "correct"),
                message=result_data.get("message"),
                suggestion=result_data.get("suggestion"),
            )
            album.audit_results.append(result)

            # Update the corresponding track
            tracks = album.tracks
            if result.track_index < len(tracks):
                if result.status == "error":
                    tracks[result.track_index].status = "error"
                elif result.status == "warning" and tracks[result.track_index].status != "error":
                    tracks[result.track_index].status = "warning"

        # Refresh display
        if hasattr(screen, "refresh_bindings"):
            screen.refresh_bindings()
