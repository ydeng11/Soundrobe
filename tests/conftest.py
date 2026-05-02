"""Pytest configuration and fixtures."""

import pytest
from pathlib import Path
from auto_tagger.config import Settings


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