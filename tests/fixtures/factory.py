"""Generate synthetic audio files with Chinese metadata for testing.

Uses ffmpeg (subprocess) for silent audio generation and mutagen for tag writing.
All files are created at generation time if the output directory is empty.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path
from typing import Any

_TRACKS: list[dict[str, Any]] = [
    {"num": 1, "title": "反轉地球"},
    {"num": 2, "title": "著迷"},
    {"num": 3, "title": "戴上我的愛"},
    {"num": 4, "title": "機會"},
    {"num": 5, "title": "來電"},
    {"num": 6, "title": "我想更懂你"},
    {"num": 7, "title": "無所不在"},
    {"num": 8, "title": "街頭詩人"},
    {"num": 9, "title": "戲如人生"},
    {"num": 10, "title": "謝謝"},
    {"num": 11, "title": "Pan@sonic"},
]

_ARTIST = "潘玮柏"
_ALBUM = "反转地球"
_YEAR = "2006"
_TRACK_TOTAL = "11"


class FixtureFactory:
    """Generates synthetic audio fixture trees."""

    def __init__(self, output_dir: Path) -> None:
        self.output_dir = Path(output_dir)
        self._ffmpeg: str | None = None
        self._have_ffmpeg: bool | None = None

    @property
    def have_ffmpeg(self) -> bool:
        if self._have_ffmpeg is None:
            self._ffmpeg = shutil.which("ffmpeg")
            self._have_ffmpeg = self._ffmpeg is not None
        return self._have_ffmpeg

    def generate_all(self) -> None:
        """Generate all fixture trees. Idempotent — skips if already exists."""
        self.generate_album()
        self.generate_compilation()
        self.generate_formats()
        self.generate_edge_cases()

    # ── album fixture ──────────────────────────────────────────

    def generate_album(self) -> Path:
        """Generate a synthetic 潘玮柏/2006-反转地球 album with 11 FLAC + LRC + cover."""
        album_dir = self.output_dir / "album" / _ARTIST / _ALBUM
        if album_dir.exists():
            return album_dir
        album_dir.mkdir(parents=True, exist_ok=True)

        for track in _TRACKS:
            filename = f"{_ARTIST} - {track['num']:02d}.{track['title']}"
            flac_path = album_dir / f"{filename}.flac"
            self._create_silent_flac(flac_path, num=track["num"], title=track["title"])
            self._create_lrc(album_dir / f"{filename}.lrc", num=track["num"], title=track["title"])

        self._create_cover_jpeg(album_dir / "cover.jpg")
        return album_dir

    # ── compilation fixture ────────────────────────────────────

    def generate_compilation(self) -> Path:
        """Generate a multi-artist compilation album."""
        comp_dir = self.output_dir / "compilation" / "Various Artists" / "Greatest Hits"
        if comp_dir.exists():
            return comp_dir
        comp_dir.mkdir(parents=True, exist_ok=True)

        comp_tracks = [
            ("01", "First Song", "Artist One"),
            ("02", "Second Song", "Artist Two"),
            ("03", "Duet", "Artist One feat. Artist Two"),
        ]
        for num, title, artist in comp_tracks:
            flac_path = comp_dir / f"{num} - {title}.flac"
            self._create_silent_flac(
                flac_path,
                num=int(num),
                title=title,
                artist=artist,
                album="Greatest Hits",
                album_artist="Various Artists",
            )
        return comp_dir

    # ── format fixtures ────────────────────────────────────────

    def generate_formats(self) -> Path:
        """Generate one file per supported format (FLAC, MP3, M4A)."""
        fmt_dir = self.output_dir / "formats"
        if fmt_dir.exists():
            return fmt_dir
        fmt_dir.mkdir(parents=True, exist_ok=True)

        meta = {"title": "Test Track", "artist": "Test Artist", "album": "Test Album"}

        self._create_silent_flac(fmt_dir / "test.flac", num=1, **meta)
        self._create_silent_mp3(fmt_dir / "test.mp3", num=1, **meta)
        self._create_silent_m4a(fmt_dir / "test.m4a", num=1, **meta)

        return fmt_dir

    # ── edge case fixtures ─────────────────────────────────────

    def generate_edge_cases(self) -> Path:
        """Generate edge-case fixture trees."""
        edge_dir = self.output_dir / "edge_cases"
        if edge_dir.exists():
            return edge_dir
        edge_dir.mkdir(parents=True, exist_ok=True)

        # Empty tags — a FLAC with no Vorbis comments
        empty_dir = edge_dir / "empty_tags" / "Unknown" / "Untitled"
        empty_dir.mkdir(parents=True, exist_ok=True)
        self._create_silent_flac(empty_dir / "01.flac")  # no tags

        # Corrupt — non-audio file renamed as .flac
        corrupt_dir = edge_dir / "corrupt" / "Bad" / "Broken"
        corrupt_dir.mkdir(parents=True, exist_ok=True)
        (corrupt_dir / "01.flac").write_text("not audio data", encoding="utf-8")

        # Missing cover — album with no cover art
        missing_dir = edge_dir / "missing_cover" / "Artist" / "No Cover"
        missing_dir.mkdir(parents=True, exist_ok=True)
        self._create_silent_flac(missing_dir / "01.flac", num=1, title="Lonely Track")

        # Unicode path — Japanese directory names
        unicode_dir = edge_dir / "unicode" / "アーティスト" / "アルバム"
        unicode_dir.mkdir(parents=True, exist_ok=True)
        self._create_silent_flac(unicode_dir / "01.flac", num=1, title="日本語")

        return edge_dir

    # ── low-level generators ───────────────────────────────────

    def _create_silent_flac(
        self,
        path: Path,
        num: int | None = None,
        title: str | None = None,
        artist: str | None = None,
        album: str | None = None,
        album_artist: str | None = None,
    ) -> None:
        """Create a minimal silent FLAC file with optional Vorbis comments."""
        if not self.have_ffmpeg:
            raise RuntimeError("ffmpeg is required to generate fixture FLAC files")

        subprocess.run(
            [
                self._ffmpeg,  # type: ignore[arg-type]
                "-y",
                "-f", "lavfi",
                "-i", "anullsrc=r=44100:cl=stereo",
                "-t", "0.1",
                "-c:a", "flac",
                "-sample_fmt", "s16",
                str(path),
            ],
            check=True,
            capture_output=True,
        )

        if any([title is not None, artist is not None, album is not None,
                album_artist is not None, num is not None]):
            from mutagen.flac import FLAC

            audio = FLAC(path)
            if title is not None:
                audio["title"] = title
            if artist is not None:
                audio["artist"] = artist
            if album is not None:
                audio["album"] = album
            if album_artist is not None:
                audio["albumartist"] = album_artist
            if num is not None:
                audio["tracknumber"] = str(num)
                audio["tracktotal"] = _TRACK_TOTAL
                audio["date"] = _YEAR
            audio.save()

    def _create_silent_mp3(
        self,
        path: Path,
        num: int,
        title: str,
        artist: str,
        album: str,
    ) -> None:
        """Create a minimal silent MP3 with ID3 tags."""
        if not self.have_ffmpeg:
            raise RuntimeError("ffmpeg is required to generate fixture MP3 files")

        subprocess.run(
            [
                self._ffmpeg,  # type: ignore[arg-type]
                "-y",
                "-f", "lavfi",
                "-i", "anullsrc=r=44100:cl=stereo",
                "-t", "0.05",
                "-c:a", "libmp3lame",
                "-q:a", "9",
                str(path),
            ],
            check=True,
            capture_output=True,
        )

        from mutagen.mp3 import EasyMP3

        audio = EasyMP3(path)
        audio["title"] = title
        audio["artist"] = artist
        audio["album"] = album
        audio["tracknumber"] = str(num)
        audio.save()

    def _create_silent_m4a(
        self,
        path: Path,
        num: int,
        title: str,
        artist: str,
        album: str,
    ) -> None:
        """Create a minimal silent M4A with MP4 tags."""
        if not self.have_ffmpeg:
            raise RuntimeError("ffmpeg is required to generate fixture M4A files")

        subprocess.run(
            [
                self._ffmpeg,  # type: ignore[arg-type]
                "-y",
                "-f", "lavfi",
                "-i", "anullsrc=r=44100:cl=stereo",
                "-t", "0.05",
                "-c:a", "aac",
                "-b:a", "16k",
                str(path),
            ],
            check=True,
            capture_output=True,
        )

        from mutagen.mp4 import MP4, MP4Tags

        audio = MP4(path)
        tags: Any = audio.tags or MP4Tags()
        tags["\xa9nam"] = title
        tags["\xa9ART"] = artist
        tags["\xa9alb"] = album
        tags["trkn"] = [(num, 0)]
        audio.tags = tags
        audio.save()

    @staticmethod
    def _create_lrc(path: Path, num: int, title: str) -> None:
        """Create a minimal LRC file with sample lyric timing."""
        lines = [
            "[ver:v1.0]",
            f"[ti:{title}]",
            f"[ar:{_ARTIST}]",
            f"[al:{_ALBUM}]",
            "[by:auto-tagger]",
            "[offset:0]",
        ]
        # Add a few timed lyric lines with realistic offsets
        base_seconds = (num - 1) * 3
        for i in range(3):
            seconds = base_seconds + i * 1.1
            m = int(seconds // 60)
            s = int(seconds % 60)
            ms = int((seconds % 1) * 1000)
            lines.append(f"[{m:02d}:{s:02d}.{ms:03d}]Sample lyric line {i + 1} for {title}")
        path.write_text("\n".join(lines), encoding="utf-8")

    @staticmethod
    def _create_cover_jpeg(path: Path) -> None:
        """Create a minimal 1x1 red JPEG without PIL."""
        # Minimal valid JPEG: 1x1 red pixel (hand-crafted)
        jpeg_data = bytes([
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01,
            0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43,
            0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08, 0x07, 0x07, 0x07, 0x09,
            0x09, 0x08, 0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12,
            0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C, 0x1C, 0x20,
            0x24, 0x2E, 0x27, 0x20, 0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29,
            0x2C, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1F, 0x27, 0x39, 0x3D, 0x38, 0x32,
            0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01,
            0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00, 0x1F, 0x00, 0x00,
            0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
            0x09, 0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03,
            0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D,
            0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06,
            0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08,
            0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72,
            0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28,
            0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45,
            0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
            0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75,
            0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
            0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3,
            0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6,
            0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9,
            0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2,
            0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4,
            0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01,
            0x00, 0x00, 0x3F, 0x00, 0x7B, 0x94, 0x28, 0xA2, 0x8A, 0x28, 0xA2, 0x8A,
            0x28, 0xA2, 0x8A, 0x28, 0xA2, 0x8A, 0x28, 0xA2, 0x8A, 0x28, 0xA2, 0x8A,
            0x28, 0xA2, 0xBF, 0xFF, 0xD9,
        ])
        path.write_bytes(jpeg_data)
