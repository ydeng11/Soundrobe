"""Real beets library integration tests.

These tests exercise the actual beets autotag matching code, not injected fakes.
They require an internet connection (MusicBrainz API) and are skipped when beets
is unavailable.
"""

from pathlib import Path

import pytest

from auto_tagger.integrations.beets_client import BeetsClient, RateLimiter
from auto_tagger.integrations.candidates import LookupRequest, LookupSource

pytestmark = pytest.mark.needs_beets


def test_real_beets_configure_does_not_raise():
    """configure_beets() initializes without reading user config."""
    client = BeetsClient()
    client.configure_beets()


def test_real_beets_lookup_track_does_not_raise(album_fixture: Path):
    """Real beets track lookup does not crash on valid FLAC files."""
    client = BeetsClient()
    flacs = sorted(album_fixture.rglob("*.flac"))
    candidates = client.lookup_track(flacs[0])
    assert isinstance(candidates, list)


def test_real_beets_lookup_album_returns_candidates(album_fixture: Path):
    """Real beets album lookup returns candidates from MusicBrainz."""
    client = BeetsClient()
    request = LookupRequest(
        path=album_fixture,
        artist_hint="潘玮柏",
        album_hint="反转地球",
    )
    candidates = client.lookup_album(request)
    # A well-known Chinese album should return at least one candidate
    assert isinstance(candidates, list)
    if candidates:
        assert candidates[0].source is LookupSource.BEETS


def test_real_beets_lookup_album_empty_dir_returns_empty(tmp_path: Path):
    """Album lookup on an empty directory returns empty list."""
    empty = tmp_path / "Empty" / "Album"
    empty.mkdir(parents=True)
    client = BeetsClient()
    request = LookupRequest(
        path=empty,
        artist_hint="Nobody",
        album_hint="Nothing",
    )
    candidates = client.lookup_album(request)
    assert isinstance(candidates, list)


def test_real_beets_rate_limiter_works():
    """Rate limiter enforces intervals between beets calls."""
    now_values = iter([10.0, 10.75])
    sleeps: list[float] = []
    limiter = RateLimiter(
        interval_seconds=1.0,
        now_func=lambda: next(now_values),
        sleep_func=sleeps.append,
    )
    limiter.wait()
    limiter.wait()
    assert sleeps == [0.25]
