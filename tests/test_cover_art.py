"""Tests for cover art discovery, fetching, and embedding."""

from pathlib import Path

from auto_tagger.core.audio import AudioFormat
from auto_tagger.features.cover_art import (
    CoverArtArchiveClient,
    CoverArtStatus,
    discover_local_cover_art,
    embed_cover_art,
)

JPEG_BYTES = b"\xff\xd8\xff\xe0cover"
PNG_BYTES = b"\x89PNG\r\n\x1a\ncover"


class FakeResponse:
    """Small HTTP response object for cover art tests."""

    def __init__(self, status_code: int, content: bytes, content_type: str = "image/jpeg"):
        self.status_code = status_code
        self.content = content
        self.headers = {"content-type": content_type}


class FakeHTTPClient:
    """Records requested URLs and returns a prepared response."""

    def __init__(self, response: FakeResponse):
        self.response = response
        self.urls: list[str] = []

    def get(self, url: str, timeout: int):
        self.urls.append(url)
        return self.response


def test_discover_local_cover_art_prefers_folder_cover(tmp_path: Path):
    """Local cover discovery prefers canonical folder artwork names."""
    (tmp_path / "front.png").write_bytes(PNG_BYTES)
    (tmp_path / "cover.jpg").write_bytes(JPEG_BYTES)

    image = discover_local_cover_art(tmp_path)

    assert image is not None
    assert image.path == tmp_path / "cover.jpg"
    assert image.mime_type == "image/jpeg"
    assert image.source == "local"


def test_cover_art_archive_client_fetches_front_cover():
    """Cover Art Archive client returns fetched image data for an MBID."""
    client = CoverArtArchiveClient(
        http_client=FakeHTTPClient(FakeResponse(200, JPEG_BYTES)),
        base_url="https://coverartarchive.org",
    )

    result = client.fetch_front_cover("release-id")

    assert result.status == CoverArtStatus.FETCHED_REMOTE
    assert result.image is not None
    assert result.image.mime_type == "image/jpeg"
    assert client.http_client.urls == ["https://coverartarchive.org/release/release-id/front"]


def test_cover_art_archive_client_reports_missing_cover():
    """404 responses become a non-error missing-cover result."""
    client = CoverArtArchiveClient(http_client=FakeHTTPClient(FakeResponse(404, b"")))

    result = client.fetch_front_cover("release-id")

    assert result.status == CoverArtStatus.MISSING
    assert result.image is None


def test_embed_cover_art_writes_vorbis_picture_block(tmp_path: Path):
    """Embedding cover art for FLAC-like tags writes a picture field."""
    cover = tmp_path / "cover.jpg"
    cover.write_bytes(JPEG_BYTES)
    image = discover_local_cover_art(tmp_path)
    tags: dict[str, list[str]] = {}

    assert image is not None
    embed_cover_art(AudioFormat.FLAC, tags, image)

    assert "METADATA_BLOCK_PICTURE" in tags
    assert tags["METADATA_BLOCK_PICTURE"][0]


def test_embed_cover_art_writes_mp4_cover_atom(tmp_path: Path):
    """Embedding cover art for MP4-like tags writes a covr atom."""
    cover = tmp_path / "cover.png"
    cover.write_bytes(PNG_BYTES)
    image = discover_local_cover_art(tmp_path)
    tags: dict[str, list[bytes]] = {}

    assert image is not None
    embed_cover_art(AudioFormat.M4A, tags, image)

    assert tags["covr"][0]
