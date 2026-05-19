"""Tests for Discogs cover art fetching."""

from pathlib import Path
from unittest.mock import MagicMock, PropertyMock

import httpx
import pytest

from auto_tagger.features.cover_art import CoverArtStatus
from auto_tagger.integrations.discogs_client import DiscogsClient


def test_fetch_cover_art_returns_missing_when_no_results(monkeypatch):
    """When Discogs search returns nothing, cover art result is MISSING."""
    client = DiscogsClient()

    def fake_search(*args, **kwargs):
        return []

    monkeypatch.setattr(client, "search_album", fake_search)
    monkeypatch.setattr(client, "_search", lambda *a, **kw: [])

    result = client.fetch_cover_art("Nonexistent", "Album")

    assert result.status == CoverArtStatus.MISSING


def test_fetch_cover_art_returns_missing_when_no_images(monkeypatch):
    """When Discogs release has no images, cover art result is MISSING."""
    client = DiscogsClient()

    monkeypatch.setattr(client, "search_album", lambda *a, **kw: [{"api": "discogs"}])
    monkeypatch.setattr(client, "_search", lambda *a, **kw: [{"id": 123}])
    monkeypatch.setattr(client, "_get", lambda path: {"images": []})

    result = client.fetch_cover_art("Artist", "Album")

    assert result.status == CoverArtStatus.MISSING


def test_fetch_cover_art_downloads_image(tmp_path, monkeypatch):
    """When Discogs has a primary image, it's downloaded and returned."""
    mock_client = MagicMock(spec=httpx.Client)
    mock_client.request.return_value = _response(200, b"data")

    client = DiscogsClient(
        http_client=mock_client,
        image_cache_dir=tmp_path / "discogs-images",
    )

    monkeypatch.setattr(client, "search_album", lambda *a, **kw: [{"api": "discogs"}])
    monkeypatch.setattr(client, "_search", lambda *a, **kw: [{"id": 123}])
    monkeypatch.setattr(client, "_get", lambda path: {
        "images": [
            {"type": "primary", "uri": "https://example.com/cover.jpg", "width": 600, "height": 600},
        ]
    })

    result = client.fetch_cover_art("Artist", "Album")

    assert result.status == CoverArtStatus.FETCHED_REMOTE
    assert result.image is not None
    assert result.image.data == b"data"
    assert result.image.source == "discogs"


def test_fetch_cover_art_prefers_primary_image(tmp_path):
    """Discogs primary image is preferred over secondary."""
    urls_downloaded = []

    def tracking_request(method, url, **kwargs):
        urls_downloaded.append(url)
        return _response(200, b"data")

    mock_client = MagicMock(spec=httpx.Client)
    mock_client.request = tracking_request

    client = DiscogsClient(
        http_client=mock_client,
        image_cache_dir=tmp_path / "discogs-images",
    )

    # We only need to set _search and _get since those are called by fetch_cover_art
    client._search = lambda *a, **kw: [{"id": 123}]  # type: ignore[method-assign]
    client._get = lambda path: {  # type: ignore[method-assign]
        "images": [
            {"type": "secondary", "uri": "https://example.com/secondary.jpg"},
            {"type": "primary", "uri": "https://example.com/primary.jpg"},
        ]
    }

    client.fetch_cover_art("Artist", "Album")
    assert "primary.jpg" in urls_downloaded[0]


def _response(status_code: int, content: bytes) -> MagicMock:
    """Build a minimal mock httpx.Response."""
    resp = MagicMock(spec=httpx.Response)
    type(resp).status_code = PropertyMock(return_value=status_code)
    resp.content = content
    resp.text = content.decode("utf-8", errors="replace")
    return resp
