"""Tests for Discogs cover art fetching."""

import pytest

from auto_tagger.features.cover_art import CoverArtStatus
from auto_tagger.integrations.discogs_client import DiscogsClient


def test_fetch_cover_art_returns_missing_when_no_results(monkeypatch):
    """When Discogs search returns nothing, cover art result is MISSING."""
    client = DiscogsClient()

    # Mock search to return empty
    def fake_search(*args, **kwargs):
        return []

    monkeypatch.setattr(client, "search_album", fake_search)
    monkeypatch.setattr(client, "_search", lambda *a, **kw: [])

    result = client.fetch_cover_art("Nonexistent", "Album")

    assert result.status == CoverArtStatus.MISSING


def test_fetch_cover_art_returns_missing_when_no_images(monkeypatch):
    """When Discogs release has no images, cover art result is MISSING."""
    client = DiscogsClient()

    def fake_search(*args, **kwargs):
        return [{"api": "discogs"}]

    class FakeResponse:
        def json(self):
            return {"images": []}
        @property
        def status_code(self):
            return 200
        @property
        def content(self):
            return b""

    def fake_get(path):
        return FakeResponse().json()

    monkeypatch.setattr(client, "search_album", fake_search)
    monkeypatch.setattr(client, "_search", lambda *a, **kw: [{"id": 123}])
    monkeypatch.setattr(client, "_get", lambda path: {"images": []})

    result = client.fetch_cover_art("Artist", "Album")

    assert result.status == CoverArtStatus.MISSING


def test_fetch_cover_art_downloads_image(monkeypatch):
    """When Discogs has a primary image, it's downloaded and returned."""
    client = DiscogsClient()

    monkeypatch.setattr(client, "search_album", lambda *a, **kw: [{"api": "discogs"}])
    monkeypatch.setattr(client, "_search", lambda *a, **kw: [{"id": 123}])
    monkeypatch.setattr(client, "_get", lambda path: {
        "images": [
            {"type": "primary", "uri": "https://example.com/cover.jpg", "width": 600, "height": 600},
        ]
    })

    import httpx

    def fake_httpx_get(url, **kwargs):
        class FakeResp:
            status_code = 200
            content = b"fake-image-data"
        return FakeResp()

    monkeypatch.setattr(httpx, "get", fake_httpx_get)

    result = client.fetch_cover_art("Artist", "Album")

    assert result.status == CoverArtStatus.FETCHED_REMOTE
    assert result.image is not None
    assert result.image.data == b"fake-image-data"
    assert result.image.source == "discogs"


def test_fetch_cover_art_prefers_primary_image(monkeypatch):
    """Discogs primary image is preferred over secondary."""
    client = DiscogsClient()

    monkeypatch.setattr(client, "search_album", lambda *a, **kw: [{"api": "discogs"}])
    monkeypatch.setattr(client, "_search", lambda *a, **kw: [{"id": 123}])
    monkeypatch.setattr(client, "_get", lambda path: {
        "images": [
            {"type": "secondary", "uri": "https://example.com/secondary.jpg"},
            {"type": "primary", "uri": "https://example.com/primary.jpg"},
        ]
    })

    import httpx
    urls_downloaded = []

    def fake_httpx_get(url, **kwargs):
        urls_downloaded.append(url)
        class FakeResp:
            status_code = 200
            content = b"data"
        return FakeResp()

    monkeypatch.setattr(httpx, "get", fake_httpx_get)

    client.fetch_cover_art("Artist", "Album")
    assert "primary.jpg" in urls_downloaded[0]
