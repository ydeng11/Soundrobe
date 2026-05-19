"""Tests for the Discogs API client."""

from unittest.mock import MagicMock, PropertyMock

import httpx
import pytest

from auto_tagger.integrations.candidates import LookupSource
from auto_tagger.integrations.discogs_client import DiscogsClient, DiscogsError


def _mock_client(json_data: dict, status_code: int = 200) -> httpx.Client:
    """Build a mock httpx.Client that returns a fixed JSON response."""
    mock_response = MagicMock(spec=httpx.Response)
    type(mock_response).status_code = PropertyMock(return_value=status_code)
    mock_response.json.return_value = json_data
    mock_response.text = str(json_data)
    mock_response.content = b""

    mock = MagicMock(spec=httpx.Client)
    mock.request.return_value = mock_response

    return mock


def _make_mock_response(status_code: int, json_data: dict | None = None) -> MagicMock:
    """Build a mock httpx.Response with the given status and optional JSON data."""
    resp = MagicMock(spec=httpx.Response)
    type(resp).status_code = PropertyMock(return_value=status_code)
    if json_data is not None:
        resp.json.return_value = json_data
    resp.text = str(json_data) if json_data else ""
    resp.content = b""
    return resp


class _CountingClient:
    """A fake httpx.Client that counts calls and returns configurable responses."""

    def __init__(self, responses: list | None = None):
        self.call_count = 0
        self._responses = responses or []

    def set_responses(self, responses: list) -> None:
        self._responses = responses

    def request(self, method: str, url: str, **kwargs) -> MagicMock:
        idx = self.call_count
        self.call_count += 1
        if idx < len(self._responses):
            return self._responses[idx]
        # Default: 200 with empty results
        return _make_mock_response(200, {"results": []})


# ── Basic tests ─────────────────────────────────────────────────


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


def test_discogs_search_returns_candidates():
    """Search results are transformed into AlbumCandidate objects."""
    mock = _mock_client({
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

    client = DiscogsClient(http_client=mock)
    candidates = client.search_album("Dr. Dre", "2001")

    assert len(candidates) == 1
    assert candidates[0].source is LookupSource.DISCOGS
    assert candidates[0].artist == "Dr. Dre"
    assert candidates[0].album == "2001"
    assert candidates[0].year == "1999"
    assert "Hip Hop" in (candidates[0].genre or "")
    assert candidates[0].musicbrainz_albumid is None


def test_discogs_search_handles_empty_results():
    """Empty search results return an empty list."""
    mock = _mock_client({"results": []})

    client = DiscogsClient(http_client=mock)
    candidates = client.search_album("Nonexistent", "Album")

    assert candidates == []


def test_discogs_search_handles_http_error():
    """HTTP 429 from Discogs raises DiscogsError from _search.

    (``search_album`` catches individual DiscogsErrors and moves to the next
    name variant, but ``_search`` propagates them.)
    """
    mock = _mock_client({"results": []}, status_code=429)

    client = DiscogsClient(http_client=mock, cache_ttl_seconds=0, max_retries=0)
    with pytest.raises(DiscogsError, match="rate limit"):
        client._search("test query")


def test_discogs_full_release_returns_tracklist():
    """Full release endpoint returns an AlbumCandidate with tracklist."""
    mock = _mock_client({
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

    client = DiscogsClient(http_client=mock)
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


def test_discogs_client_5566_search():
    """A search for 5566 (Taiwanese boy band) returns album candidates."""
    mock = _mock_client({
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

    client = DiscogsClient(http_client=mock)
    candidates = client.search_album("5566", "挚爱")

    assert len(candidates) == 1
    assert candidates[0].artist == "5566"
    assert candidates[0].album == "挚爱"
    assert candidates[0].source is LookupSource.DISCOGS


# ── New tests for caching, retry, and proxy rotation ──────────────


def test_in_memory_cache_hits_on_repeated_search():
    """The same search query returns cached results on second call."""
    counting = _CountingClient([
        _make_mock_response(200, {
            "results": [{"title": "Artist - Album", "year": "2020", "id": 1}]
        }),
    ])

    client = DiscogsClient(http_client=counting)  # type: ignore[arg-type]
    first = client.search_album("Artist", "Album")
    second = client.search_album("Artist", "Album")

    assert len(first) == 1
    assert len(second) == 1
    # Only 1 real HTTP call — second hit the cache
    assert counting.call_count == 1


def test_retry_on_429_then_succeed():
    """Client retries on 429 with backoff and succeeds on third attempt."""
    counting = _CountingClient([
        _make_mock_response(429),
        _make_mock_response(429),
        _make_mock_response(200, {
            "results": [{"title": "Artist - Album", "id": 1}]
        }),
    ])

    client = DiscogsClient(http_client=counting, cache_ttl_seconds=0, max_retries=2)
    candidates = client.search_album("Artist", "Album")

    assert len(candidates) == 1
    assert counting.call_count == 3  # 2 retries + 1 success


def test_retry_exhausted_raises():
    """Client raises DiscogsError after exhausting all retries on 429."""
    counting = _CountingClient([
        _make_mock_response(429),
        _make_mock_response(429),  # max_retries=1 means 2 attempts
    ])

    client = DiscogsClient(http_client=counting, cache_ttl_seconds=0, max_retries=1)
    with pytest.raises(DiscogsError, match="rate limit"):
        client._search("test query")


def test_cache_ttl_expiry():
    """Cache entries expire after their TTL (TTL=0 forces re-fetch)."""
    counting = _CountingClient([
        _make_mock_response(200, {
            "results": [{"title": "Artist - Album", "id": 1}]
        }),
        _make_mock_response(200, {
            "results": [{"title": "Artist - Album", "id": 2}]
        }),
    ])

    # TTL = 0 means immediate expiry
    client = DiscogsClient(http_client=counting, cache_ttl_seconds=0)
    client.search_album("Artist", "Album")
    client.search_album("Artist", "Album")

    # Both calls hit the network because TTL=0
    assert counting.call_count == 2


def test_search_artist_returns_results():
    """Artist search returns raw result dicts."""
    mock = _mock_client({
        "results": [
            {"id": 123, "title": "Test Artist", "type": "artist"},
        ]
    })

    client = DiscogsClient(http_client=mock)
    results = client.search_artist("Test Artist")

    assert len(results) == 1
    assert results[0]["id"] == 123
    assert results[0]["title"] == "Test Artist"


def test_get_artist_returns_details():
    """Artist GET returns full artist details."""
    mock = _mock_client({
        "id": 123,
        "name": "Test Artist",
        "images": [{"type": "primary", "uri": "https://example.com/img.jpg"}],
    })

    client = DiscogsClient(http_client=mock)
    data = client.get_artist(123)

    assert data["id"] == 123
    assert data["name"] == "Test Artist"


def test_search_album_429_graceful_degradation():
    """When all Discogs searches return 429, search_album returns empty list.

    ``search_album`` catches individual DiscogsErrors from each name variant
    and gracefully returns no candidates rather than propagating the error.
    """
    mock = _mock_client({"results": []}, status_code=429)

    client = DiscogsClient(http_client=mock, cache_ttl_seconds=0, max_retries=0)
    candidates = client.search_album("Artist", "Album")

    assert candidates == []
