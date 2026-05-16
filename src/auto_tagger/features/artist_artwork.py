"""Artist artwork discovery and fetching for Navidrome.

Navidrome reads artist images from ``artist.{jpg,jpeg,png}`` files located in
the artist's music directory. This module discovers artist directories,
checks for existing artwork, and fetches artist images from Discogs.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path

from auto_tagger.features.cover_art import CoverArtImage, _mime_type_for_bytes, _valid_image_data

ARTIST_IMAGE_NAMES = ("artist",)
ARTIST_IMAGE_SUFFIXES = (".jpg", ".jpeg", ".png")

# Default directories to skip when scanning for artist folders.
# These are aggregation directories (Compilations, Various Artists, etc.)
# that contain albums without a single canonical artist.
_DEFAULT_SKIP_DIRS: set[str] = {
    "compilations", "various artists", "va",
}


class ArtistArtworkStatus(str, Enum):
    """Status of an artist artwork fetch attempt."""

    ALREADY_EXISTS = "already_exists"
    FETCHED = "fetched"
    SKIPPED = "skipped"
    MISSING = "missing"
    FAILED = "failed"


@dataclass(frozen=True)
class ArtistArtworkOutcome:
    """Result of processing one artist directory."""

    artist_name: str
    artist_path: Path
    status: ArtistArtworkStatus
    image_path: Path | None = None
    message: str = ""


@dataclass(frozen=True)
class ArtistArtworkSummary:
    """Aggregated results from a full library run."""

    found_local: int = 0
    fetched: int = 0
    skipped: int = 0
    missing: int = 0
    failed: int = 0
    total: int = 0
    outcomes: list[ArtistArtworkOutcome] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)

    @property
    def successful(self) -> int:
        """Images already present or successfully fetched."""
        return self.found_local + self.fetched


def discover_artist_directories(
    library_path: Path,
    skip_dirs: set[str] | None = None,
) -> list[Path]:
    """Find artist directories that contain audio content.

    Scans the top-level directories under *library_path*. A directory is
    considered an artist directory if it meets either condition:

    1. **Standard layout**: Contains subdirectories (albums) that contain
       audio files (``Artist/Album/track.flac``).
    2. **Flat layout**: Directly contains audio files (``Artist/track.flac``).

    Skip directories listed in *skip_dirs* (case-insensitive). By default
    skips ``Compilations``, ``Various Artists``, and ``VA``.

    Returns a sorted list of artist directory paths.
    """
    if not library_path.is_dir():
        return []

    skip = _DEFAULT_SKIP_DIRS | {s.casefold() for s in (skip_dirs or set())}
    from auto_tagger.core.audio import SUPPORTED_EXTENSIONS

    artists: list[Path] = []
    for entry in sorted(library_path.iterdir()):
        if not entry.is_dir():
            continue
        if entry.name.casefold() in skip:
            continue

        has_audio = False

        # Check 1: Direct audio files in the artist dir (flat layout)
        for f in entry.iterdir():
            if f.is_file() and f.suffix.lower() in SUPPORTED_EXTENSIONS:
                has_audio = True
                break

        if not has_audio:
            # Check 2: Album subdirectories containing audio files (standard layout)
            for sub in entry.iterdir():
                if not sub.is_dir():
                    continue
                for f in sub.iterdir():
                    if f.is_file() and f.suffix.lower() in SUPPORTED_EXTENSIONS:
                        has_audio = True
                        break
                if has_audio:
                    break

        if has_audio:
            artists.append(entry)

    return artists


def find_local_artist_image(artist_dir: Path) -> CoverArtImage | None:
    """Check if ``artist.{jpg,jpeg,png}`` exists in *artist_dir*.

    Validates that the file contains real image data (not a placeholder).
    Returns a ``CoverArtImage`` if found and valid, or ``None``.
    """
    for name in ARTIST_IMAGE_NAMES:
        for suffix in ARTIST_IMAGE_SUFFIXES:
            candidate = artist_dir / f"{name}{suffix}"
            if not candidate.exists() or not candidate.is_file():
                continue
            data = candidate.read_bytes()
            if not _valid_image_data(data):
                continue
            mime_type = _mime_type_for_bytes(data)
            if mime_type is None:
                # Fallback: guess from extension
                if suffix in (".jpg", ".jpeg"):
                    mime_type = "image/jpeg"
                elif suffix == ".png":
                    mime_type = "image/png"
                else:
                    continue
            return CoverArtImage(data, mime_type, "local", candidate)
    return None


def save_artist_image(
    artist_dir: Path,
    image: CoverArtImage,
    suffix: str = ".jpg",
) -> Path:
    """Save *image* as ``artist.jpg`` (or custom *suffix*) inside *artist_dir*.

    If the image's MIME type is PNG, ``.png`` is used instead. Overwrites
    any existing file. Returns the path to the saved file.
    """
    if image.mime_type == "image/png":
        suffix = ".png"
    elif image.mime_type == "image/jpeg":
        suffix = ".jpg"
    dest = artist_dir / f"artist{suffix}"
    dest.write_bytes(image.data)
    return dest
