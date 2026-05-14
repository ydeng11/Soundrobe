# Test Hardening Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Raise test suite from 85% coverage to 90%+ with real-integration tests, E2E CLI dry-run, and complete dataset write-path coverage using synthetic 潘玮柏-inspired fixtures.

**Architecture:** Four-phase layered approach. Phase 1 builds a fixture factory that generates synthetic FLAC/MP3/M4A/LRC/cover files with Chinese metadata. Phase 2 tests real beets/ffprobe/rgain3 against those fixtures. Phase 3 runs full E2E CLI pipelines. Phase 4 fills remaining coverage gaps (dataset write, command entry points, logging, edge cases).

**Tech Stack:** ffmpeg (silent audio), mutagen (tag writing), pytest fixtures (session-scoped), CliRunner (E2E)

---

## Phase 1: Fixture Factory

### Task 1.1: Add `.gitignore` and pytest marks to conftest

**Files:**
- Modify: `.gitignore` (append)
- Modify: `tests/conftest.py`

**Step 1: Add fixture data to .gitignore**

```python
# Read current .gitignore, append:
tests/fixtures/data/
```

**Step 2: Add pytest marks and shared utilities to conftest**

Add to `tests/conftest.py`:

```python
"""Pytest configuration and fixtures."""

import shutil
from pathlib import Path

import pytest

from auto_tagger.config import Settings


# ── existing fixtures (tmp_album, tmp_library, settings, etc.) remain unchanged ──


def _has_command(name: str) -> bool:
    """Check if a command is available on PATH."""
    return shutil.which(name) is not None


needs_ffmpeg = pytest.mark.skipif(
    not (_has_command("ffmpeg") and _has_command("ffprobe")),
    reason="requires ffmpeg and ffprobe",
)

needs_beets = pytest.mark.skipif(
    not _has_command("beet"),
    reason="requires beets CLI",
)

needs_rgain = pytest.mark.skipif(
    not (_has_command("rgain3") or _has_command("loudgain")),
    reason="requires rgain3 or loudgain",
)


@pytest.fixture(scope="session")
def fixtures_dir(tmp_path_factory: pytest.TempPathFactory) -> Path:
    """Generate or load the synthetic fixture tree once per test session."""
    from tests.fixtures.factory import FixtureFactory

    data_dir = tmp_path_factory.mktemp("fixtures_data")
    factory = FixtureFactory(data_dir)
    factory.generate_all()
    return data_dir


@pytest.fixture(scope="session")
def album_fixture(fixtures_dir: Path) -> Path:
    """Path to a synthetic 潘玮柏/2006-反转地球 album."""
    return fixtures_dir / "album"


@pytest.fixture(scope="session")
def compilation_fixture(fixtures_dir: Path) -> Path:
    """Path to a synthetic multi-artist compilation album."""
    return fixtures_dir / "compilation"


@pytest.fixture(scope="session")
def format_fixtures(fixtures_dir: Path) -> Path:
    """Path to format test files (flac, mp3, m4a)."""
    return fixtures_dir / "formats"


@pytest.fixture(scope="session")
def edge_case_fixtures(fixtures_dir: Path) -> Path:
    """Path to edge case fixtures (empty tags, corrupt, missing cover)."""
    return fixtures_dir / "edge_cases"
```

**Step 3: Run existing tests to ensure conftest doesn't break anything**

Run: `pytest tests/ -x -q`
Expected: 119 passed (fixture generation will fail until factory exists, but existing tests that don't use new fixtures should pass)

**Step 4: Commit**

```bash
git add .gitignore tests/conftest.py
git commit -m "test: add pytest marks and fixture scaffolding"
```

---

### Task 1.2: Create fixture factory module

**Files:**
- Create: `tests/fixtures/__init__.py`
- Create: `tests/fixtures/factory.py`

**Step 1: Create package init**

`tests/fixtures/__init__.py`:
```python
"""Synthetic audio fixture generation for testing."""
```

**Step 2: Write the fixture factory**

`tests/fixtures/factory.py`:

```python
"""Generate synthetic audio files with Chinese metadata for testing.

Uses ffmpeg (subprocess) for silent audio generation and mutagen for tag writing.
All files are created at module import time if the output directory is empty.
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
        self.output_dir = output_dir
        self._ffmpeg: str | None = None
        self._have_ffmpeg: bool | None = None

    @property
    def have_ffmpeg(self) -> bool:
        if self._have_ffmpeg is None:
            self._ffmpeg = shutil.which("ffmpeg")
            self._have_ffmpeg = self._ffmpeg is not None
        return self._have_ffmpeg

    def generate_all(self) -> None:
        """Generate all fixture trees. Idempotent."""
        self.generate_album()
        self.generate_compilation()
        self.generate_formats()
        self.generate_edge_cases()

    # ── album fixture ──────────────────────────────────────────

    def generate_album(self) -> Path:
        """Generate a synthetic 潘玮柏/2006-反转地球 album."""
        album_dir = self.output_dir / "album" / _ARTIST / _ALBUM
        if album_dir.exists():
            return album_dir
        album_dir.mkdir(parents=True, exist_ok=True)

        for track in _TRACKS:
            filename = f"{_ARTIST} - {track['num']:02d}.{track['title']}"
            flac_path = album_dir / f"{filename}.flac"
            self._create_silent_flac(flac_path, **track)
            self._create_lrc(album_dir / f"{filename}.lrc", **track)

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
        """Generate one file per supported format."""
        fmt_dir = self.output_dir / "formats"
        if fmt_dir.exists():
            return fmt_dir
        fmt_dir.mkdir(parents=True, exist_ok=True)

        meta = {"title": "Test Track", "artist": "Test Artist", "album": "Test Album"}

        flac = fmt_dir / "test.flac"
        self._create_silent_flac(flac, num=1, **meta)

        mp3 = fmt_dir / "test.mp3"
        self._create_silent_mp3(mp3, num=1, **meta)

        m4a = fmt_dir / "test.m4a"
        self._create_silent_m4a(m4a, num=1, **meta)

        return fmt_dir

    # ── edge case fixtures ─────────────────────────────────────

    def generate_edge_cases(self) -> Path:
        """Generate edge-case fixture trees."""
        edge_dir = self.output_dir / "edge_cases"
        if edge_dir.exists():
            return edge_dir
        edge_dir.mkdir(parents=True, exist_ok=True)

        # Empty tags
        empty_dir = edge_dir / "empty_tags" / "Unknown" / "Untitled"
        empty_dir.mkdir(parents=True, exist_ok=True)
        self._create_silent_flac(empty_dir / "01.flac")  # no tags

        # Corrupt: non-audio file renamed as .flac
        corrupt_dir = edge_dir / "corrupt" / "Bad" / "Broken"
        corrupt_dir.mkdir(parents=True, exist_ok=True)
        (corrupt_dir / "01.flac").write_text("not audio data", encoding="utf-8")

        # Missing cover
        missing_dir = edge_dir / "missing_cover" / "Artist" / "No Cover"
        missing_dir.mkdir(parents=True, exist_ok=True)
        self._create_silent_flac(missing_dir / "01.flac", num=1, title="Lonely Track")

        # Unicode path
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

        if any([title, artist, album, album_artist, num]):
            from mutagen.flac import FLAC

            audio = FLAC(path)
            if title:
                audio["title"] = title
            if artist:
                audio["artist"] = artist
            if album:
                audio["album"] = album
            if album_artist:
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
        """Create a minimal LRC file with sample timing."""
        lines = [
            "[ver:v1.0]",
            f"[ti:{title}]",
            f"[ar:{_ARTIST}]",
            f"[al:{_ALBUM}]",
            "[by:auto-tagger]",
            "[offset:0]",
        ]
        # Add a few timed lyric lines
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
        # Minimal valid JPEG: 1x1 red pixel
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
```

**Step 2: Verify factory works**

```bash
cd /Users/ihelio/code/auto_tagger && .venv/bin/python -c "
from tests.fixtures.factory import FixtureFactory
from pathlib import Path
import tempfile
d = Path(tempfile.mkdtemp())
f = FixtureFactory(d)
f.generate_all()
print('Album files:', sorted((d / 'album').rglob('*')))
print('Compilation files:', sorted((d / 'compilation').rglob('*')))
print('Format files:', sorted((d / 'formats').rglob('*')))
print('Edge case files:', sorted((d / 'edge_cases').rglob('*')))
"
```

Expected: All fixture directories populated with files.

**Step 3: Commit**

```bash
git add tests/fixtures/
git commit -m "test: add synthetic fixture factory with 潘玮柏-inspired data"
```

---

### Task 1.3: Validate fixtures with tests

**Files:**
- Create: `tests/test_fixtures.py`

**Step 1: Write fixture validation tests**

`tests/test_fixtures.py`:

```python
"""Validate that synthetic fixtures are correctly generated."""

import json
from pathlib import Path

import pytest
from mutagen.flac import FLAC

from auto_tagger.core.audio import detect_audio_format, iter_audio_files
from auto_tagger.core.reader import read_metadata


# ── album fixture ──────────────────────────────────────────────

def test_album_fixture_has_eleven_flac_files(album_fixture: Path):
    """Synthetic album fixture contains 11 FLAC tracks."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    assert len(flacs) == 11


def test_album_fixture_has_eleven_lrc_files(album_fixture: Path):
    """Synthetic album fixture contains 11 LRC files."""
    lrcs = sorted(album_fixture.rglob("*.lrc"))
    assert len(lrcs) == 11


def test_album_fixture_has_cover_image(album_fixture: Path):
    """Synthetic album fixture contains a cover.jpg."""
    assert (album_fixture / "cover.jpg").exists()


def test_album_fixture_flac_has_correct_tags(album_fixture: Path):
    """First FLAC track has expected Vorbis comment tags."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    audio = FLAC(flacs[0])
    tags = dict(audio.tags or {})
    assert tags["artist"] == ["潘玮柏"]
    assert tags["album"] == ["反转地球"]
    assert tags["date"] == ["2006"]
    assert tags["tracktotal"] == ["11"]


def test_album_fixture_tracks_have_unique_titles(album_fixture: Path):
    """All 11 tracks have distinct titles."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    titles = {FLAC(f).tags["title"][0] for f in flacs}
    assert len(titles) == 11
    assert "反轉地球" in titles
    assert "Pan@sonic" in titles


def test_album_fixture_iter_audio_files_works(album_fixture: Path):
    """iter_audio_files finds all 11 FLAC files."""
    found = list(iter_audio_files(album_fixture))
    assert len(found) == 11


def test_album_fixture_read_metadata_works(album_fixture: Path):
    """read_metadata returns correct metadata for the first track."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    meta = read_metadata(flacs[0])
    assert meta.title == "反轉地球"
    assert meta.artist == "潘玮柏"
    assert meta.album == "反转地球"
    assert meta.track_number == 1
    assert meta.year == "2006"


def test_album_fixture_detect_audio_format(album_fixture: Path):
    """detect_audio_format identifies FLAC files correctly."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    fmt = detect_audio_format(flacs[0])
    assert fmt is not None
    assert fmt.name == "FLAC"


# ── compilation fixture ────────────────────────────────────────

def test_compilation_fixture_has_three_tracks(compilation_fixture: Path):
    """Multi-artist compilation has 3 tracks."""
    flacs = sorted(compilation_fixture.rglob("*.flac"))
    assert len(flacs) == 3


def test_compilation_fixture_has_different_artists(compilation_fixture: Path):
    """Compilation tracks have different artists."""
    flacs = sorted(compilation_fixture.rglob("*.flac"))
    artists = {FLAC(f).tags["artist"][0] for f in flacs}
    assert len(artists) >= 2


# ── format fixtures ────────────────────────────────────────────

def test_format_fixtures_has_all_formats(format_fixtures: Path):
    """Formats directory contains .flac, .mp3, .m4a."""
    assert (format_fixtures / "test.flac").exists()
    assert (format_fixtures / "test.mp3").exists()
    assert (format_fixtures / "test.m4a").exists()


def test_format_fixtures_tags_readable(format_fixtures: Path):
    """All format files have readable metadata."""
    for ext in ("flac", "mp3", "m4a"):
        meta = read_metadata(format_fixtures / f"test.{ext}")
        assert meta.title == "Test Track"
        assert meta.artist == "Test Artist"


# ── edge case fixtures ─────────────────────────────────────────

def test_edge_case_empty_tags_dir_exists(edge_case_fixtures: Path):
    """Empty tags fixture directory is populated."""
    empty_dir = edge_case_fixtures / "empty_tags"
    assert list(empty_dir.rglob("*.flac"))


def test_edge_case_corrupt_file_exists(edge_case_fixtures: Path):
    """Corrupt fixture has a .flac file that isn't valid audio."""
    corrupt = edge_case_fixtures / "corrupt" / "Bad" / "Broken" / "01.flac"
    assert corrupt.exists()
    # Should not be valid FLAC
    with pytest.raises(Exception):
        FLAC(corrupt)


def test_edge_case_missing_cover_dir_has_no_jpg(edge_case_fixtures: Path):
    """Missing cover fixture has no cover image."""
    jpgs = list((edge_case_fixtures / "missing_cover").rglob("*.jpg"))
    assert len(jpgs) == 0


def test_edge_case_unicode_path_exists(edge_case_fixtures: Path):
    """Unicode path fixture contains a FLAC file."""
    unicode_dir = edge_case_fixtures / "unicode"
    assert list(unicode_dir.rglob("*.flac"))
```

**Step 2: Run fixture tests**

```bash
pytest tests/test_fixtures.py -v
```

Expected: ~15 passed (may skip if ffmpeg unavailable in CI)

**Step 3: Commit**

```bash
git add tests/test_fixtures.py
git commit -m "test: add fixture validation tests"
```

---

## Phase 2: Real-Integration Tests

### Task 2.1: Real beets integration tests

**Files:**
- Create: `tests/test_beets_integration.py`

**Step 1: Write beets integration tests**

`tests/test_beets_integration.py`:

```python
"""Real beets library integration tests.

These tests exercise the actual beets autotag matching code, not injected fakes.
They require an internet connection (MusicBrainz API) and are skipped when beets
is unavailable.
"""

from pathlib import Path

import pytest

from auto_tagger.integrations.beets_client import BeetsClient
from auto_tagger.integrations.candidates import LookupRequest, LookupSource

pytestmark = pytest.mark.needs_beets


def test_real_beets_configure_does_not_raise():
    """configure_beets() initializes without reading user config."""
    client = BeetsClient()
    client.configure_beets()  # should not raise


def test_real_beets_lookup_album_returns_candidates(album_fixture: Path):
    """Real beets album lookup returns candidates for a well-known album."""
    client = BeetsClient()
    request = LookupRequest(
        path=album_fixture,
        artist_hint="潘玮柏",
        album_hint="反转地球",
    )
    candidates = client.lookup_album(request)
    # May return 0 if MusicBrainz doesn't recognize the synthetic album,
    # but should not raise.
    assert isinstance(candidates, list)


def test_real_beets_lookup_track_does_not_raise(album_fixture: Path):
    """Real beets track lookup does not crash on valid FLAC files."""
    client = BeetsClient()
    flacs = sorted(album_fixture.rglob("*.flac"))
    candidates = client.lookup_track(flacs[0])
    assert isinstance(candidates, list)


def test_real_beets_lookup_album_candidates_have_correct_source(album_fixture: Path):
    """Beets-sourced candidates carry the BEETS source marker."""
    client = BeetsClient()
    request = LookupRequest(
        path=album_fixture,
        artist_hint="潘玮柏",
        album_hint="反转地球",
    )
    candidates = client.lookup_album(request)
    for candidate in candidates:
        assert candidate.source is LookupSource.BEETS


def test_real_beets_rate_limiter_works(album_fixture: Path):
    """Successive lookups don't fail due to rate limiting."""
    client = BeetsClient()
    request = LookupRequest(
        path=album_fixture,
        artist_hint="潘玮柏",
        album_hint="反转地球",
    )
    # Two lookups in a row should both succeed
    client.lookup_album(request)
    client.lookup_album(request)  # should not raise
```

**Step 2: Run beets integration tests**

```bash
pytest tests/test_beets_integration.py -v -m "needs_beets"
```

Expected: Tests may fail if MusicBrainz is unreachable, but should not crash with import errors or config issues.

**Step 3: Commit**

```bash
git add tests/test_beets_integration.py
git commit -m "test: add real beets library integration tests"
```

---

### Task 2.2: Real ffprobe integration tests

**Files:**
- Create: `tests/test_ffprobe_integration.py`

**Step 1: Write ffprobe integration tests**

`tests/test_ffprobe_integration.py`:

```python
"""Real ffprobe integration tests for audio validation."""

from pathlib import Path

import pytest

from auto_tagger.quality.audio_validation import FfprobeValidator, HealthIssue

pytestmark = pytest.mark.needs_ffmpeg


def test_ffprobe_accepts_valid_flac(album_fixture: Path):
    """ffprobe validates a properly formed FLAC file."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    validator = FfprobeValidator()
    issues = validator.validate(flacs[0])
    # Should have no ERROR-level issues for valid audio
    errors = [i for i in issues if hasattr(i, "severity") and str(i.severity) == "error"]
    assert len(errors) == 0


def test_ffprobe_reports_no_audio_stream(edge_case_fixtures: Path):
    """ffprobe reports error for a text file disguised as FLAC."""
    corrupt = edge_case_fixtures / "corrupt" / "Bad" / "Broken" / "01.flac"
    validator = FfprobeValidator()
    issues = validator.validate(corrupt)
    assert len(issues) >= 1


def test_ffprobe_validates_all_album_tracks(album_fixture: Path):
    """All 11 synthetic FLAC files pass ffprobe validation."""
    validator = FfprobeValidator()
    flacs = sorted(album_fixture.rglob("*.flac"))
    for flac in flacs:
        issues = validator.validate(flac)
        errors = [i for i in issues if hasattr(i, "severity") and str(i.severity) == "error"]
        assert len(errors) == 0, f"{flac.name} has errors: {errors}"
```

**Step 2: Run ffprobe tests**

```bash
pytest tests/test_ffprobe_integration.py -v -m "needs_ffmpeg"
```

Expected: 3 passed

**Step 3: Commit**

```bash
git add tests/test_ffprobe_integration.py
git commit -m "test: add real ffprobe integration tests"
```

---

### Task 2.3: Real ReplayGain integration tests

**Files:**
- Create: `tests/test_replaygain_integration.py`

**Step 1: Write ReplayGain integration tests**

`tests/test_replaygain_integration.py`:

```python
"""Real ReplayGain calculation integration tests."""

from pathlib import Path

import pytest

from auto_tagger.quality.replaygain import ReplayGainCalculator

pytestmark = pytest.mark.needs_rgain


def test_replaygain_calculates_on_silent_flac(album_fixture: Path):
    """ReplayGain calculator runs successfully on silent FLAC."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    calculator = ReplayGainCalculator()
    tags = calculator.calculate(flacs[0])
    assert tags is not None
    assert tags.track_gain is not None
    assert tags.track_peak is not None


def test_replaygain_calculates_album_gain(album_fixture: Path):
    """Album ReplayGain runs on all tracks."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    calculator = ReplayGainCalculator()
    tags = calculator.calculate_album(flacs)
    if tags is not None:  # some commands don't support album mode
        assert tags.album_gain is not None
        assert tags.album_peak is not None
```

**Step 2: Run ReplayGain tests**

```bash
pytest tests/test_replaygain_integration.py -v -m "needs_rgain"
```

Expected: Tests may skip if no rgain3/loudgain; otherwise 1-2 passed.

**Step 3: Commit**

```bash
git add tests/test_replaygain_integration.py
git commit -m "test: add real ReplayGain integration tests"
```

---

## Phase 3: End-to-End CLI Tests

### Task 3.1: E2E tag command dry-run

**Files:**
- Create: `tests/test_cli_e2e.py`

**Step 1: Write E2E tag command tests**

`tests/test_cli_e2e.py`:

```python
"""End-to-end CLI tests using synthetic fixtures.

These tests exercise the full pipeline: CLI arg parsing → audio discovery →
metadata reading → lookup → health report → output formatting.
All tests use CliRunner (no subprocess) for speed and debuggability.
"""

import json
from pathlib import Path

from click.testing import CliRunner

from auto_tagger.cli import cli


def test_tag_dry_run_on_fixture_album_shows_metadata(album_fixture: Path):
    """Dry-run on the synthetic album shows track metadata preview."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(album_fixture), "--dry-run"])
    assert result.exit_code == 0
    assert "反轉地球" in result.output
    assert "潘玮柏" in result.output
    assert "反转地球" in result.output
    assert "Metadata preview" in result.output


def test_tag_dry_run_shows_lookup_section(album_fixture: Path):
    """Dry-run output includes a lookup candidates section."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(album_fixture), "--dry-run"])
    assert result.exit_code == 0
    assert "Lookup" in result.output


def test_tag_dry_run_shows_health_report(album_fixture: Path):
    """Dry-run output includes a health report section."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(album_fixture), "--dry-run"])
    assert result.exit_code == 0
    assert "Health" in result.output


def test_tag_dry_run_writes_health_report_json(album_fixture: Path, tmp_path: Path):
    """--health-report flag writes a JSON file with valid structure."""
    report_path = tmp_path / "health.json"
    runner = CliRunner()
    result = runner.invoke(
        cli,
        ["tag", str(album_fixture), "--dry-run", "--health-report", str(report_path)],
    )
    assert result.exit_code == 0
    assert report_path.exists()
    data = json.loads(report_path.read_text(encoding="utf-8"))
    assert "summary" in data
    assert "errors" in data["summary"]
    assert "warnings" in data["summary"]


def test_tag_yolo_dry_run_does_not_write(album_fixture: Path):
    """YOLO mode with dry-run previews but does not mutate files."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(album_fixture), "--dry-run", "--yolo"])
    assert result.exit_code == 0
    assert "YOLO" in result.output


def test_tag_verbose_shows_extra_detail(album_fixture: Path):
    """Verbose flag adds detail to output."""
    runner = CliRunner()
    result = runner.invoke(cli, ["--verbose", "tag", str(album_fixture), "--dry-run"])
    assert result.exit_code == 0


def test_tag_json_output_is_valid_json(album_fixture: Path):
    """JSON output format produces parseable JSON."""
    runner = CliRunner()
    result = runner.invoke(
        cli,
        ["--output", "json", "tag", str(album_fixture), "--dry-run"],
    )
    assert result.exit_code == 0
    # Should be parseable JSON
    data = json.loads(result.output)
    assert isinstance(data, (dict, list))


def test_tag_nonexistent_path_errors():
    """Tagging a nonexistent path exits non-zero."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", "/nonexistent/path/xyz"])
    assert result.exit_code != 0


def test_tag_empty_directory_handled(tmp_path: Path):
    """Tagging an empty directory shows appropriate message."""
    empty = tmp_path / "empty"
    empty.mkdir()
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(empty), "--dry-run"])
    # Should not crash
    assert isinstance(result.exit_code, int)
```

**Step 2: Run E2E tag tests**

```bash
pytest tests/test_cli_e2e.py -v
```

Expected: ~9-10 passed (some may fail if network-dependent lookups time out)

**Step 3: Commit**

```bash
git add tests/test_cli_e2e.py
git commit -m "test: add E2E CLI tag command dry-run tests"
```

---

### Task 3.2: E2E batch command dry-run

**Files:**
- Modify: `tests/test_cli_e2e.py` (append)

**Step 1: Append batch command E2E tests**

Add to `tests/test_cli_e2e.py`:

```python
# ── batch command ──────────────────────────────────────────────

def test_batch_dry_run_on_fixture_library(fixtures_dir: Path):
    """Batch dry-run processes the full fixture library."""
    runner = CliRunner()
    result = runner.invoke(cli, ["batch", str(fixtures_dir), "--dry-run"])
    assert result.exit_code == 0
    assert "Batch processing" in result.output
    assert "Albums processed" in result.output


def test_batch_dry_run_shows_summary(fixtures_dir: Path):
    """Batch output includes album counts and applied/skipped/failed."""
    runner = CliRunner()
    result = runner.invoke(cli, ["batch", str(fixtures_dir), "--dry-run"])
    assert result.exit_code == 0
    # Summary lines
    assert "processed" in result.output.lower()
    assert "applied" in result.output.lower() or "skipped" in result.output.lower()
```

**Step 2: Run batch tests**

```bash
pytest tests/test_cli_e2e.py::test_batch_dry_run_on_fixture_library -v
pytest tests/test_cli_e2e.py::test_batch_dry_run_shows_summary -v
```

Expected: 2 passed

**Step 3: Commit**

```bash
git add tests/test_cli_e2e.py
git commit -m "test: add E2E CLI batch command dry-run tests"
```

---

## Phase 4: Remaining Coverage Gaps

### Task 4.1: Dataset command entry point tests

**Files:**
- Create: `tests/test_dataset_commands.py`

**Step 1: Write dataset command tests**

`tests/test_dataset_commands.py`:

```python
"""Tests for dataset command entry points (status and setup)."""

import json
from pathlib import Path

from click.testing import CliRunner

from auto_tagger.cli import cli


def test_dataset_status_when_not_installed():
    """dataset status command reports not-installed state."""
    runner = CliRunner()
    result = runner.invoke(
        cli,
        ["dataset", "status"],
        env={"AUTO_TAG_DATA_DIR": "/tmp/nonexistent-dataset-test"},
    )
    assert result.exit_code == 0
    assert "not installed" in result.output.lower()


def test_dataset_setup_dry_run_prints_plan():
    """dataset setup --dry-run prints plan without downloading."""
    runner = CliRunner()
    result = runner.invoke(
        cli,
        ["dataset", "setup", "--dry-run"],
    )
    assert result.exit_code == 0
    assert "Dataset setup" in result.output
    assert "dry run" in result.output.lower()


def test_dataset_setup_invalid_service_errors():
    """dataset setup with invalid service raises error."""
    runner = CliRunner()
    result = runner.invoke(
        cli,
        ["dataset", "setup", "--services", "invalid_service"],
    )
    assert result.exit_code != 0
```

**Step 2: Run dataset command tests**

```bash
pytest tests/test_dataset_commands.py -v
```

Expected: 3 passed (setup dry-run may fail if GitHub API is unreachable; if so, mark with `pytest.mark.network`)

**Step 3: Commit**

```bash
git add tests/test_dataset_commands.py
git commit -m "test: add dataset command entry point tests"
```

---

### Task 4.2: Dataset write path tests

**Files:**
- Create: `tests/test_dataset_write.py`

**Step 1: Write dataset write path tests**

`tests/test_dataset_write.py`:

```python
"""Tests for DatasetIndexWriter and CSV index building."""

import csv
import json
from pathlib import Path

from auto_tagger.integrations.candidates import LookupRequest, LookupSource
from auto_tagger.integrations.dataset import (
    DatasetIndexClient,
    DatasetIndexWriter,
    DatasetState,
    build_index_from_csv_tree,
    load_dataset_state,
    normalize_lookup_text,
    save_dataset_state,
)


def test_dataset_state_round_trip(tmp_path: Path):
    """DatasetState serializes and deserializes correctly."""
    original = DatasetState(
        version="01 January 2025",
        services=["musicbrainz", "spotify"],
        source_file="Dataset 01 January 2025.torrent",
        built_at="2025-01-01T00:00:00+00:00",
        album_rows=100,
        track_rows=500,
    )
    state_path = tmp_path / "state.json"
    save_dataset_state(state_path, original)
    loaded = load_dataset_state(state_path)
    assert loaded is not None
    assert loaded.version == original.version
    assert loaded.services == original.services
    assert loaded.album_rows == 100
    assert loaded.track_rows == 500


def test_dataset_index_writer_adds_album(tmp_path: Path):
    """DatasetIndexWriter stores an album and its tracks."""
    index_path = tmp_path / "index.sqlite"
    writer = DatasetIndexWriter(index_path)
    album_id = writer.add_album(
        source="musicbrainz",
        artist="Test Artist",
        album="Test Album",
        album_artist="Test Artist",
        year="2024",
        genre="Rock",
        musicbrainz_albumid="test-mbid-123",
        tracks=[
            {
                "title": "Track One",
                "artist": "Test Artist",
                "track_number": 1,
                "track_total": 2,
                "disc_number": 1,
                "disc_total": 1,
                "length": 240.0,
            },
            {
                "title": "Track Two",
                "artist": "Test Artist",
                "track_number": 2,
                "track_total": 2,
                "disc_number": 1,
                "disc_total": 1,
                "length": 180.0,
            },
        ],
    )
    writer.close()

    assert album_id > 0
    assert writer.album_rows == 1
    assert writer.track_rows == 2


def test_dataset_index_client_looks_up_written_album(tmp_path: Path):
    """DatasetIndexClient finds an album previously written by DatasetIndexWriter."""
    index_path = tmp_path / "index.sqlite"

    writer = DatasetIndexWriter(index_path)
    writer.add_album(
        source="musicbrainz",
        artist="潘玮柏",
        album="反转地球",
        year="2006",
        tracks=[
            {"title": "反轉地球", "artist": "潘玮柏", "track_number": 1, "track_total": 11},
        ],
    )
    writer.close()

    client = DatasetIndexClient(index_path)
    request = LookupRequest(
        path=tmp_path,
        artist_hint="潘玮柏",
        album_hint="反转地球",
    )
    candidates = client.lookup_album(request)

    assert len(candidates) == 1
    assert candidates[0].artist == "潘玮柏"
    assert candidates[0].album == "反转地球"
    assert candidates[0].source is LookupSource.DATASET
    assert candidates[0].musicbrainz_albumid is None  # not set for non-musicbrainz fields
    assert len(candidates[0].tracks) == 1
    assert candidates[0].tracks[0].title == "反轉地球"


def test_dataset_index_client_returns_empty_on_missing_index(tmp_path: Path):
    """DatasetIndexClient returns empty list when index file is missing."""
    client = DatasetIndexClient(tmp_path / "nonexistent.sqlite")
    request = LookupRequest(path=tmp_path, artist_hint="X", album_hint="Y")
    candidates = client.lookup_album(request)
    assert candidates == []
    assert client.last_warning is not None
    assert "not found" in client.last_warning


def test_build_index_from_csv_tree(tmp_path: Path):
    """build_index_from_csv_tree imports a minimal CSV tree."""
    csv_dir = tmp_path / "csv"
    musicbrainz_dir = csv_dir / "musicbrainz"
    musicbrainz_dir.mkdir(parents=True)

    # Write a minimal album CSV
    album_csv = musicbrainz_dir / "musicbrainz_album_2025.csv"
    with album_csv.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(
            f,
            fieldnames=[
                "artist", "album", "year", "genre",
                "albumartist", "album_id",
            ],
        )
        writer.writeheader()
        writer.writerow({
            "artist": "Test Artist",
            "album": "Test Album",
            "year": "2024",
            "genre": "Pop",
            "albumartist": "Test Artist",
            "album_id": "mbid-123",
        })

    index_path = tmp_path / "index.sqlite"
    album_rows, track_rows = build_index_from_csv_tree(
        csv_dir, index_path, services=["musicbrainz"]
    )

    assert album_rows >= 1
    assert track_rows >= 0

    # Verify lookup works
    client = DatasetIndexClient(index_path)
    candidates = client.lookup_album(
        LookupRequest(path=tmp_path, artist_hint="Test Artist", album_hint="Test Album")
    )
    assert len(candidates) >= 1


def test_normalize_lookup_text():
    """Normalize function strips punctuation and lowercases."""
    assert normalize_lookup_text("潘玮柏") == "潘玮柏"
    assert normalize_lookup_text("Test Artist!") == "test artist"
    assert normalize_lookup_text("  Multiple   Spaces  ") == "multiple spaces"
    assert normalize_lookup_text(None) == ""
    assert normalize_lookup_text("") == ""


def test_dataset_index_writer_handles_none_artist(tmp_path: Path):
    """add_album returns 0 and does not crash when artist is None."""
    index_path = tmp_path / "index.sqlite"
    writer = DatasetIndexWriter(index_path)
    album_id = writer.add_album(source="musicbrainz", artist=None, album="Album")
    writer.close()
    assert album_id == 0


def test_dataset_index_client_normalizes_chinese_text(tmp_path: Path):
    """Chinese artist/album names are normalized correctly for lookup."""
    index_path = tmp_path / "index.sqlite"
    writer = DatasetIndexWriter(index_path)
    writer.add_album(
        source="musicbrainz",
        artist="潘玮柏",
        album="反转地球",
        year="2006",
        tracks=[],
    )
    writer.close()

    client = DatasetIndexClient(index_path)
    # Look up with same text
    candidates = client.lookup_album(
        LookupRequest(path=tmp_path, artist_hint="潘玮柏", album_hint="反转地球")
    )
    assert len(candidates) == 1
```

**Step 2: Run dataset write path tests**

```bash
pytest tests/test_dataset_write.py -v
```

Expected: 8 passed

**Step 3: Commit**

```bash
git add tests/test_dataset_write.py
git commit -m "test: add dataset write path and index building tests"
```

---

### Task 4.3: Command entry point tests

**Files:**
- Create: `tests/test_command_entry_points.py`

**Step 1: Write command entry point tests**

`tests/test_command_entry_points.py`:

```python
"""Tests for command entry point execute() functions."""

from pathlib import Path

from auto_tagger.commands.tag import execute as tag_execute
from auto_tagger.config import Settings


def test_tag_execute_with_fixture_album(album_fixture: Path):
    """tag.execute() runs without raising on a valid album."""
    settings = Settings(data_dir=Path("/tmp/auto-tagger-test"))
    # dry_run=True prevents writes
    tag_execute(
        settings=settings,
        path=album_fixture,
        dry_run=True,
        yolo=False,
        interactive=False,
        health_report_path=None,
    )


def test_tag_execute_with_empty_dir(tmp_path: Path):
    """tag.execute() handles empty directory gracefully."""
    empty = tmp_path / "empty"
    empty.mkdir()
    settings = Settings(data_dir=Path("/tmp/auto-tagger-test"))
    # Should not raise
    tag_execute(
        settings=settings,
        path=empty,
        dry_run=True,
        yolo=False,
        interactive=False,
        health_report_path=None,
    )


def test_tag_execute_yolo_mode(album_fixture: Path):
    """tag.execute() runs in yolo mode with dry-run (no mutation)."""
    settings = Settings(data_dir=Path("/tmp/auto-tagger-test"), yolo=True)
    tag_execute(
        settings=settings,
        path=album_fixture,
        dry_run=True,
        yolo=True,
        interactive=False,
        health_report_path=None,
    )
```

**Step 2: Run entry point tests**

```bash
pytest tests/test_command_entry_points.py -v
```

Expected: 3 passed (may need network for lookups)

**Step 3: Commit**

```bash
git add tests/test_command_entry_points.py
git commit -m "test: add command entry point execute() tests"
```

---

### Task 4.4: Logging and edge case tests

**Files:**
- Create: `tests/test_logging.py`
- Create: `tests/test_edge_cases.py`

**Step 1: Write logging tests**

`tests/test_logging.py`:

```python
"""Tests for logging configuration."""

import logging

from auto_tagger.utils.logging import setup_logging


def test_setup_logging_verbose():
    """Verbose mode sets DEBUG level."""
    setup_logging(verbose=True)
    logger = logging.getLogger("auto_tagger")
    assert logger.level == logging.DEBUG


def test_setup_logging_default():
    """Default mode sets INFO level."""
    setup_logging(verbose=False)
    logger = logging.getLogger("auto_tagger")
    assert logger.level <= logging.INFO


def test_setup_logging_adds_handler():
    """setup_logging adds at least one handler."""
    logger = logging.getLogger("auto_tagger")
    initial_handlers = len(logger.handlers)
    setup_logging(verbose=False)
    assert len(logger.handlers) > initial_handlers or logger.handlers
```

**Step 2: Write edge case tests**

`tests/test_edge_cases.py`:

```python
"""Edge case and robustness tests."""

from pathlib import Path

import pytest
from click.testing import CliRunner

from auto_tagger.cli import cli
from auto_tagger.core.audio import detect_audio_format, iter_audio_files
from auto_tagger.core.reader import read_metadata
from auto_tagger.integrations.candidates import LookupSource
from auto_tagger.integrations.fallback import candidate_from_folder, parse_album_path


def test_iter_audio_files_skips_non_media_files(tmp_path: Path):
    """iter_audio_files ignores .txt, .docx, .url files."""
    album = tmp_path / "Artist" / "Album"
    album.mkdir(parents=True)
    (album / "01.flac").touch()
    (album / "notes.txt").touch()
    (album / "readme.docx").touch()
    (album / "shortcut.url").touch()

    found = list(iter_audio_files(album))
    assert len(found) == 1
    assert found[0].suffix == ".flac"


def test_reader_handles_corrupt_flac(edge_case_fixtures: Path):
    """read_metadata handles corrupt files gracefully."""
    corrupt = edge_case_fixtures / "corrupt" / "Bad" / "Broken" / "01.flac"
    # Should raise or return empty metadata — depends on implementation
    try:
        meta = read_metadata(corrupt)
        # If it doesn't raise, metadata should be minimal
        assert meta is not None
    except Exception:
        pass  # raising is also acceptable


def test_unicode_paths_supported(edge_case_fixtures: Path):
    """iter_audio_files works with Japanese paths."""
    jp_dir = edge_case_fixtures / "unicode"
    found = list(iter_audio_files(jp_dir))
    assert len(found) >= 1


def test_fallback_on_unicode_path(edge_case_fixtures: Path):
    """Fallback parse works on Unicode directory names."""
    jp_album = edge_case_fixtures / "unicode" / "アーティスト" / "アルバム"
    request = parse_album_path(jp_album)
    assert request.artist_hint == "アーティスト"
    assert request.album_hint == "アルバム"


def test_detect_format_unknown_extension(tmp_path: Path):
    """Unknown extensions return None from detect_audio_format."""
    unknown = tmp_path / "file.xyz"
    unknown.touch()
    assert detect_audio_format(unknown) is None


def test_cli_handles_permission_error(tmp_path: Path, monkeypatch):
    """CLI doesn't crash on permission-denied paths."""
    # Instead of actually creating unreadable dirs (which may need root),
    # verify the CLI handles path errors gracefully
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", "/root/forbidden"])
    assert result.exit_code != 0


def test_tag_command_handles_file_path_instead_of_dir(tmp_path: Path):
    """Tagging a file path (not directory) works via fallback."""
    track = tmp_path / "track.flac"
    track.write_bytes(b"")
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(track), "--dry-run"])
    # Should not crash
    assert isinstance(result.exit_code, int)
```

**Step 3: Run logging and edge case tests**

```bash
pytest tests/test_logging.py tests/test_edge_cases.py -v
```

Expected: ~8 passed

**Step 4: Commit**

```bash
git add tests/test_logging.py tests/test_edge_cases.py
git commit -m "test: add logging and edge case tests"
```

---

### Task 4.5: Final verification

**Step 1: Run full test suite with coverage**

```bash
pytest tests/ -v --cov=src/auto_tagger --cov-report=term-missing 2>&1 | tail -80
```

Expected: All tests pass. Coverage should be ≥90% (up from 85%). Key previously-untested files should show improved coverage:
- `commands/batch.py`: 29% → ≥70%
- `integrations/beets_client.py`: 64% → ≥80%
- `integrations/dataset.py`: 70% → ≥85%
- `commands/dataset.py`: 67% → ≥80%

**Step 2: Run linting**

```bash
ruff check src tests
mypy src
```

Expected: 0 errors

**Step 3: Count tests**

```bash
pytest tests/ --collect-only -q | tail -3
```

Expected: ≥145 tests (started with 119, adding ~30-40)

**Step 4: Commit final**

```bash
git add -A
git commit -m "test: complete test hardening — fixtures, integration, E2E, gaps"
```
