"""Tests for the isolated Beets client boundary."""

from pathlib import Path
from types import SimpleNamespace

import pytest


def test_beets_client_normalizes_album_candidates():
    """Injected beets album matcher results become project candidates."""
    from auto_tagger.integrations.beets_client import BeetsClient
    from auto_tagger.integrations.candidates import LookupRequest, LookupSource

    proposal = SimpleNamespace(
        distance=0.2,
        info=SimpleNamespace(
            artist="Artist",
            artists=["Artist"],
            album="Album",
            albumartist="Artist",
            albumartists=["Artist"],
            year=2024,
            genre="Pop",
            album_id="album-id",
            artist_id="artist-id",
            tracks=[
                SimpleNamespace(
                    title="Song",
                    artist="Artist",
                    artists=["Artist"],
                    track=1,
                    tracktotal=1,
                    disc=1,
                    disctotal=1,
                    track_id="track-id",
                    length=180.0,
                )
            ],
        ),
    )
    client = BeetsClient(match_album_func=lambda request: [proposal])

    candidates = client.lookup_album(
        LookupRequest(path=Path("/music/Artist/Album"), artist_hint="Artist", album_hint="Album")
    )

    assert len(candidates) == 1
    assert candidates[0].source is LookupSource.BEETS
    assert candidates[0].album == "Album"
    assert candidates[0].musicbrainz_albumid == "album-id"
    assert candidates[0].tracks[0].musicbrainz_trackid == "track-id"


def test_beets_client_sorts_and_caps_album_candidates():
    """Candidates are returned best-distance first and capped."""
    from auto_tagger.integrations.beets_client import BeetsClient
    from auto_tagger.integrations.candidates import LookupRequest

    proposals = [
        SimpleNamespace(distance=0.9, info=SimpleNamespace(artist="A", album="Bad")),
        SimpleNamespace(distance=0.1, info=SimpleNamespace(artist="A", album="Good")),
    ]
    client = BeetsClient(match_album_func=lambda request: proposals, max_candidates=1)

    candidates = client.lookup_album(LookupRequest(path=Path("/music/A/Album")))

    assert [candidate.album for candidate in candidates] == ["Good"]


def test_beets_client_wraps_lookup_errors():
    """Lookup failures become project tagging errors with context."""
    from auto_tagger.exceptions import TaggingError
    from auto_tagger.integrations.beets_client import BeetsClient
    from auto_tagger.integrations.candidates import LookupRequest

    def fail(_request):
        raise RuntimeError("network down")

    client = BeetsClient(match_album_func=fail)

    with pytest.raises(TaggingError, match="Could not query beets"):
        client.lookup_album(LookupRequest(path=Path("/music/A/B"), artist_hint="A", album_hint="B"))


def test_rate_limiter_sleeps_for_remaining_interval():
    """Rate limiter enforces a minimum interval between calls."""
    from auto_tagger.integrations.beets_client import RateLimiter

    now_values = iter([10.0, 10.25])
    sleeps: list[float] = []
    limiter = RateLimiter(
        interval_seconds=1.0,
        now_func=lambda: next(now_values),
        sleep_func=sleeps.append,
    )

    limiter.wait()
    limiter.wait()

    assert sleeps == [0.75]
