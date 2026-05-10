"""Cover art discovery, fetching, and embedding helpers."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import Protocol

from auto_tagger.core.audio import AudioFormat
from auto_tagger.core.formats import embed_cover_art as embed_format_cover_art

COVER_NAMES = (
    "cover",
    "folder",
    "front",
    "album",
)
COVER_SUFFIXES = (".jpg", ".jpeg", ".png")


class CoverArtStatus(str, Enum):
    """Cover art lookup status."""

    FOUND_LOCAL = "found_local"
    FETCHED_REMOTE = "fetched_remote"
    MISSING = "missing"
    FETCH_FAILED = "fetch_failed"
    INVALID = "invalid"


@dataclass(frozen=True)
class CoverArtImage:
    """Image data ready to embed."""

    data: bytes
    mime_type: str
    source: str
    path: Path | None = None


@dataclass(frozen=True)
class CoverArtResult:
    """Result of a cover art lookup."""

    status: CoverArtStatus
    image: CoverArtImage | None = None
    message: str = ""


class HTTPResponse(Protocol):
    """Subset of HTTP response fields used by the cover client."""

    @property
    def status_code(self) -> int:
        """HTTP status code."""

    @property
    def content(self) -> bytes:
        """Response body."""

    @property
    def headers(self) -> dict[str, str]:
        """Response headers."""


class HTTPClient(Protocol):
    """Small HTTP client protocol."""

    def get(self, url: str, timeout: int) -> HTTPResponse:
        """Fetch a URL."""


class CoverArtArchiveClient:
    """Fetch front cover images from Cover Art Archive."""

    def __init__(
        self,
        http_client: HTTPClient | None = None,
        base_url: str = "https://coverartarchive.org",
        timeout_seconds: int = 20,
    ):
        self.http_client = http_client or _DefaultHTTPClient()
        self.base_url = base_url.rstrip("/")
        self.timeout_seconds = timeout_seconds

    def fetch_front_cover(self, musicbrainz_albumid: str) -> CoverArtResult:
        """Fetch the front cover for a MusicBrainz release ID."""
        url = f"{self.base_url}/release/{musicbrainz_albumid}/front"
        try:
            response = self.http_client.get(url, timeout=self.timeout_seconds)
        except Exception as exc:
            return CoverArtResult(CoverArtStatus.FETCH_FAILED, message=str(exc))

        if response.status_code == 404:
            return CoverArtResult(CoverArtStatus.MISSING, message="No cover art found")
        if response.status_code != 200:
            return CoverArtResult(
                CoverArtStatus.FETCH_FAILED,
                message=f"Cover Art Archive returned HTTP {response.status_code}",
            )

        content_type = response.headers.get("content-type", "").split(";")[0].strip()
        mime_type = content_type or _mime_type_for_bytes(response.content)
        if mime_type not in {"image/jpeg", "image/png"} or not _valid_image_data(response.content):
            return CoverArtResult(CoverArtStatus.INVALID, message="Cover response was not an image")

        return CoverArtResult(
            CoverArtStatus.FETCHED_REMOTE,
            CoverArtImage(response.content, mime_type, "cover-art-archive"),
        )


def discover_local_cover_art(album_path: Path) -> CoverArtImage | None:
    """Find the preferred local cover art image in an album folder."""
    for name in COVER_NAMES:
        for suffix in COVER_SUFFIXES:
            candidate = album_path / f"{name}{suffix}"
            if not candidate.exists():
                continue
            data = candidate.read_bytes()
            mime_type = _mime_type_for_bytes(data) or _mime_type_for_suffix(candidate.suffix)
            if mime_type and _valid_image_data(data):
                return CoverArtImage(data, mime_type, "local", candidate)
    return None


def embed_cover_art(audio_format: AudioFormat, tags: object, image: CoverArtImage) -> None:
    """Embed cover art in a format-specific tag object."""
    embed_format_cover_art(audio_format, tags, image.data, image.mime_type)


def _mime_type_for_suffix(suffix: str) -> str | None:
    if suffix.lower() in {".jpg", ".jpeg"}:
        return "image/jpeg"
    if suffix.lower() == ".png":
        return "image/png"
    return None


def _mime_type_for_bytes(data: bytes) -> str | None:
    if data.startswith(b"\xff\xd8\xff"):
        return "image/jpeg"
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        return "image/png"
    return None


def _valid_image_data(data: bytes) -> bool:
    return _mime_type_for_bytes(data) is not None


class _DefaultHTTPClient:
    def get(self, url: str, timeout: int) -> HTTPResponse:
        import httpx

        response = httpx.get(url, timeout=timeout)
        return _SimpleHTTPResponse(
            status_code=response.status_code,
            content=response.content,
            headers={key.lower(): value for key, value in response.headers.items()},
        )


@dataclass(frozen=True)
class _SimpleHTTPResponse:
    status_code: int
    content: bytes
    headers: dict[str, str]
