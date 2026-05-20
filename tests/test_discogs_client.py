"""Tests for the Discogs API client."""

import pytest

from auto_tagger.integrations.candidates import LookupSource
from auto_tagger.integrations.discogs_client import DiscogsClient, DiscogsError


class FakeResponse:
    """Minimal fake httpx.Response."""

    def __init__(self, status_code: int, json_data: dict | None = None, text: str = ""):
        self.status_code = status_code
        self._json_data = json_data or {}
        self.text = text

    def json(self) -> dict:
        return self._json_data

    def raise_for_status(self) -> None:
        if self.status_code >= 400:
            import httpx
            raise httpx.HTTPStatusError("error", request=object(), response=self)  # type: ignore[arg-type]


def test_discogs_client_splits_artist_album_title():
    """The Discogs title 'Artist - Album' is split into artist and album."""
    client = DiscogsClient()
    artist, album = client._split_title("Dr. Dre - 2001")
    assert artist == "Dr. Dre"
    assert album == "2001"


def test_discogs_client_handles_title_without_dash():
    """A title without ' - ' is treated as album-only."""
    client = DiscogsClient()
    artist, album = client._split_title("2001")
    assert artist is None
    assert album == "2001"


def test_discogs_client_parses_position():
    """Track positions are parsed from Discogs format strings."""
    client = DiscogsClient()
    assert client._parse_position("A1") == 1
    assert client._parse_position("1") == 1
    assert client._parse_position("CD-12") == 12
    assert client._parse_position("B3") == 3
    assert client._parse_position("") is None
    assert client._parse_position("Vinyl") is None


def test_discogs_client_parses_duration():
    """Duration strings like '3:28' are converted to seconds."""
    client = DiscogsClient()
    assert client._parse_duration("3:28") == 208.0
    assert client._parse_duration("0:40") == 40.0
    assert client._parse_duration("") is None
    assert client._parse_duration("invalid") is None


def test_discogs_search_returns_candidates(monkeypatch):
    """Search results are transformed into AlbumCandidate objects."""
    def fake_get(url, params=None, headers=None, timeout=None):
        return FakeResponse(200, {
            "results": [
                {
                    "title": "Dr. Dre - 2001",
                    "year": "1999",
                    "genre": ["Hip Hop"],
                    "style": ["Gangsta"],
                    "id": 3201905,
                }
            ]
        })

    import httpx
    monkeypatch.setattr(httpx, "get", fake_get)

    client = DiscogsClient()
    candidates = client.search_album("Dr. Dre", "2001")

    assert len(candidates) == 1
    assert candidates[0].source is LookupSource.DISCOGS
    assert candidates[0].artist == "Dr. Dre"
    assert candidates[0].album == "2001"
    assert candidates[0].year == "1999"
    assert "Hip Hop" in (candidates[0].genre or "")
    assert candidates[0].musicbrainz_albumid is None  # Discogs has no MBIDs


def test_discogs_search_handles_empty_results(monkeypatch):
    """Empty search results return an empty list."""
    def fake_get(url, params=None, headers=None, timeout=None):
        return FakeResponse(200, {"results": []})

    import httpx
    monkeypatch.setattr(httpx, "get", fake_get)

    client = DiscogsClient()
    candidates = client.search_album("Nonexistent", "Album")

    assert candidates == []


def test_discogs_search_handles_http_error(monkeypatch):
    """HTTP 429 is retried once with backoff, then raises DiscogsError from _search."""
    import logging

    # Capture warnings to verify retry behavior
    logs: list[str] = []

    class LogHandler(logging.Handler):
        def emit(self, record: logging.LogRecord) -> None:
            logs.append(record.getMessage())

    logger = logging.getLogger("auto_tagger.integrations.discogs_client")
    logger.addHandler(LogHandler())
    original_level = logger.level
    logger.setLevel(logging.WARNING)

    call_count: int = 0

    def fake_get(url, params=None, headers=None, timeout=None):
        nonlocal call_count
        call_count += 1
        return FakeResponse(429, text="Rate limit exceeded")

    import httpx
    monkeypatch.setattr(httpx, "get", fake_get)

    client = DiscogsClient()
    # search_album catches DiscogsError internally, returns empty list
    result = client.search_album("Artist", "Album")
    assert result == []

    # The rate limit warning was logged during retry
    assert any("rate limit hit" in msg for msg in logs), logs
    # httpx.get was called at least twice per _search call (initial + 1 retry)
    assert call_count >= 2

    logger.removeHandler(logger.handlers[-1])
    logger.setLevel(original_level)


def test_discogs_rate_limited_get_raises_after_retries(monkeypatch):
    """_rate_limited_get raises DiscogsError after retries are exhausted."""
    def fake_get(url, params=None, headers=None, timeout=None):
        return FakeResponse(429, text="Rate limit exceeded")

    import httpx
    monkeypatch.setattr(httpx, "get", fake_get)

    client = DiscogsClient()
    with pytest.raises(DiscogsError, match="rate limit exceeded after 2 attempts"):
        client._rate_limited_get(client.BASE_URL + "/database/search")


def test_discogs_full_release_returns_tracklist(monkeypatch):
    """Full release endpoint returns an AlbumCandidate with tracklist."""
    def fake_get(url, params=None, headers=None, timeout=None):
        return FakeResponse(200, {
            "title": "Dr. Dre - 2001",
            "year": 1999,
            "genres": ["Hip Hop"],
            "styles": ["Gangsta"],
            "artists": [{"name": "Dr. Dre"}],
            "tracklist": [
                {"position": "1", "title": "Lolo (Intro)", "duration": "0:40"},
                {"position": "2", "title": "The Watcher", "duration": "3:28"},
            ],
        })

    import httpx
    monkeypatch.setattr(httpx, "get", fake_get)

    client = DiscogsClient()
    candidate = client.get_release(3201905)

    assert candidate is not None
    assert candidate.album == "2001"
    assert candidate.artist == "Dr. Dre"
    assert candidate.artists == ["Dr. Dre"]
    assert len(candidate.tracks) == 2
    assert candidate.tracks[0].title == "Lolo (Intro)"
    assert candidate.tracks[0].length == 40.0
    assert candidate.tracks[1].title == "The Watcher"
    assert candidate.tracks[1].length == 208.0


def test_discogs_client_5566_search(monkeypatch):
    """A search for 5566 (Taiwanese boy band) returns album candidates."""
    def fake_get(url, params=None, headers=None, timeout=None):
        return FakeResponse(200, {
            "results": [
                {
                    "title": "5566 - 挚爱",
                    "year": "2004",
                    "genre": ["Pop"],
                    "style": ["Mandopop"],
                    "id": 123456,
                }
            ]
        })

    import httpx
    monkeypatch.setattr(httpx, "get", fake_get)

    client = DiscogsClient()
    candidates = client.search_album("5566", "挚爱")

    assert len(candidates) == 1
    assert candidates[0].artist == "5566"
    assert candidates[0].album == "挚爱"
    assert candidates[0].source is LookupSource.DISCOGS
