"""Pytest configuration and fixtures."""

import shutil
from pathlib import Path

import pytest

from auto_tagger.config import Settings


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
    """Path to a synthetic 潘玮柏/2006-反转地球 album directory."""
    return fixtures_dir / "album" / "潘玮柏" / "反转地球"


@pytest.fixture(scope="session")
def compilation_fixture(fixtures_dir: Path) -> Path:
    """Path to a synthetic multi-artist compilation album directory."""
    return fixtures_dir / "compilation" / "Various Artists" / "Greatest Hits"


@pytest.fixture(scope="session")
def format_fixtures(fixtures_dir: Path) -> Path:
    """Path to format test files (flac, mp3, m4a)."""
    return fixtures_dir / "formats"


@pytest.fixture(scope="session")
def edge_case_fixtures(fixtures_dir: Path) -> Path:
    """Path to edge case fixtures (empty tags, corrupt, missing cover)."""
    return fixtures_dir / "edge_cases"


@pytest.fixture
def tmp_album(tmp_path: Path) -> Path:
    """Create a temporary album directory structure."""
    album_dir = tmp_path / "Test Artist" / "Test Album"
    album_dir.mkdir(parents=True)

    (album_dir / "01 - Track One.mp3").touch()
    (album_dir / "02 - Track Two.mp3").touch()

    return album_dir


@pytest.fixture
def tmp_library(tmp_path: Path) -> Path:
    """Create a temporary library structure."""
    library = tmp_path / "Music"

    albums = [
        ("Artist One", "Album One", ["01.mp3", "02.mp3"]),
        ("Artist Two", "Album Two", ["01.mp3", "02.mp3", "03.mp3"]),
    ]

    for artist, album, tracks in albums:
        album_dir = library / artist / album
        album_dir.mkdir(parents=True)
        for track in tracks:
            (album_dir / track).touch()

    return library


@pytest.fixture
def settings() -> Settings:
    """Create default settings instance."""
    return Settings()


@pytest.fixture
def verbose_settings() -> Settings:
    """Create verbose settings instance."""
    return Settings(verbose=True)


@pytest.fixture
def yolo_settings() -> Settings:
    """Create YOLO mode settings instance."""
    return Settings(yolo=True)
