"""In-memory application state."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from auto_tagger.core.metadata import TrackMetadata


def _has_embedded_cover(audio_format, mutagen_file) -> bool:
    """Check if a mutagen file object has embedded cover art."""
    from auto_tagger.core.audio import AudioFormat

    if audio_format in (AudioFormat.MP3, AudioFormat.WAV):
        if hasattr(mutagen_file, "tags") and mutagen_file.tags:
            try:
                apic = mutagen_file.tags.getall("APIC")
                return len(apic) > 0
            except Exception:
                return False
        return False
    elif audio_format is AudioFormat.M4A:
        return bool(mutagen_file.get("covr", []))
    else:
        # FLAC / Ogg Vorbis
        if hasattr(mutagen_file, "pictures"):
            try:
                return len(mutagen_file.pictures) > 0
            except Exception:
                return False
        return False


def _extract_embedded_cover(audio_format, mutagen_file) -> bytes | None:
    """Extract embedded cover art bytes from a mutagen file object.

    Returns the raw image data or None if no cover is embedded or the
    format is unsupported.
    """
    from auto_tagger.core.audio import AudioFormat

    if audio_format in (AudioFormat.MP3, AudioFormat.WAV):
        try:
            if hasattr(mutagen_file, "tags") and mutagen_file.tags:
                apic_list = mutagen_file.tags.getall("APIC")
                if apic_list:
                    return apic_list[0].data  # type: ignore[no-any-return]
        except Exception:
            return None
        return None
    elif audio_format is AudioFormat.M4A:
        try:
            covr = mutagen_file.get("covr", [])
            if covr:
                return covr[0]  # type: ignore[no-any-return]
        except Exception:
            return None
        return None
    else:
        # FLAC / Ogg Vorbis
        try:
            if hasattr(mutagen_file, "pictures"):
                pics = mutagen_file.pictures
                if pics:
                    return pics[0].data  # type: ignore[no-any-return]
        except Exception:
            return None
        return None


@dataclass
class TrackAuditResult:
    """Result of an LLM audit check on a single track field."""

    track_index: int
    field: str
    status: str  # "correct" | "warning" | "error"
    message: str | None = None
    suggestion: str | None = None


@dataclass
class AlbumData:
    """In-memory data for one album directory."""

    path: Path
    artist_hint: str = ""
    album_hint: str = ""
    tracks: list[TrackData] = field(default_factory=list)
    audio_file_paths: list[Path] = field(default_factory=list)
    _tracks_loaded: bool = False
    status: str = "pending"  # "pending" | "ok" | "warning" | "error"
    cover_path: Path | None = None
    cover_source: str = ""  # "external" | "embedded" | "missing"
    audit_results: list[TrackAuditResult] = field(default_factory=list)

    @property
    def tracks_loaded(self) -> bool:
        return self._tracks_loaded

    def ensure_tracks_loaded(self) -> None:
        """Lazily load track metadata from disk."""
        if self._tracks_loaded:
            return

        from auto_tagger.core.audio import load_audio_file
        from auto_tagger.core.formats import read_tags

        for audio_path in self.audio_file_paths:
            try:
                af = load_audio_file(audio_path)
                meta = read_tags(af.format, af.mutagen_file)
                has_cover = _has_embedded_cover(af.format, af.mutagen_file)
                self.tracks.append(
                    TrackData(
                        path=audio_path,
                        metadata=meta,
                        has_cover=has_cover,
                        size_bytes=audio_path.stat().st_size,
                    )
                )
            except Exception:
                # Skip unreadable files
                pass

        # Detect external cover file in album directory
        self._detect_external_cover()
        self._tracks_loaded = True

    def _detect_external_cover(self) -> None:
        """Detect external cover.jpg/png in the album directory."""
        from auto_tagger.features.cover_art import COVER_NAMES, COVER_SUFFIXES

        for name in list(COVER_NAMES) + [self.album_hint]:
            for suffix in COVER_SUFFIXES:
                candidate = self.path / f"{name}{suffix}"
                if candidate.exists():
                    self.cover_path = candidate
                    self.cover_source = "external"
                    return

    def get_cover_bytes(self) -> bytes | None:
        """Return cover art image bytes (external file or embedded).

        Priority:
        1. External cover file in album directory
        2. Embedded cover from first audio track with cover art
        """
        # External file (fast path)
        if self.cover_path is not None and self.cover_path.exists():
            try:
                return self.cover_path.read_bytes()
            except Exception:
                return None

        # Embedded cover from tracks (needs lazy load)
        if not self._tracks_loaded:
            self.ensure_tracks_loaded()

        for track in self.tracks:
            if track.has_cover:
                try:
                    from auto_tagger.core.audio import load_audio_file

                    af = load_audio_file(track.path)
                    return _extract_embedded_cover(af.format, af.mutagen_file)
                except Exception:
                    continue

        return None

    @property
    def track_count(self) -> int:
        if self._tracks_loaded:
            return len(self.tracks)
        return len(self.audio_file_paths)

    @property
    def error_count(self) -> int:
        return sum(1 for t in self.tracks if t.status == "error")

    @property
    def warning_count(self) -> int:
        return sum(1 for t in self.tracks if t.status == "warning")


@dataclass
class TrackData:
    """In-memory data for one audio track."""

    path: Path
    metadata: TrackMetadata
    status: str = "pending"  # "pending" | "ok" | "warning" | "error"
    changed_fields: set[str] = field(default_factory=set)
    has_cover: bool = False
    size_bytes: int = 0
    bitrate: int | None = None
    sample_rate: int | None = None
    codec: str = ""


@dataclass
class AppState:
    """Top-level application state."""

    library_path: Path | None = None
    albums: dict[Path, AlbumData] = field(default_factory=dict)
    selected_album_path: Path | None = None
    selected_track_paths: set[Path] = field(default_factory=set)
    loaded: bool = False
    auto_tagging: bool = False
    auto_tag_progress: int = 0
    auto_tag_total: int = 0
    auditing: bool = False
    audit_progress: int = 0
    audit_total: int = 0
    show_album_browser: bool = True
    filter_text: str = ""
    audio_file_count: int = 0
    total_duration_seconds: float = 0.0
    auto_audit_enabled: bool = True
    parallel_jobs: int = 4
    llm_model: str = ""
    output_format: str = "table"
    recent_workspaces: list[Path] = field(default_factory=list)
    debug_enabled: bool = False

    @property
    def selected_album(self) -> AlbumData | None:
        if self.selected_album_path and self.selected_album_path in self.albums:
            return self.albums[self.selected_album_path]
        return None

    @property
    def selected_tracks(self) -> list[TrackData]:
        if not self.selected_album:
            return []
        return [
            t for t in self.selected_album.tracks
            if t.path in self.selected_track_paths
        ]

    def clear(self) -> None:
        """Reset all state."""
        self.library_path = None
        self.albums.clear()
        self.selected_album_path = None
        self.selected_track_paths.clear()
        self.loaded = False
        self.auto_tagging = False
        self.auto_tag_progress = 0
        self.auto_tag_total = 0
        self.auditing = False
        self.audit_progress = 0
        self.audit_total = 0
        self.show_album_browser = True
        self.filter_text = ""
        self.audio_file_count = 0
        self.total_duration_seconds = 0.0
