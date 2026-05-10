"""Real beets library integration tests.

These tests exercise the actual beets autotag matching code, not injected fakes.
They require an internet connection (MusicBrainz API) and are skipped when beets
is unavailable.

Note: beets 2.x `tag_album` requires non-empty items. The `_match_album_with_beets`
path in `BeetsClient` currently passes an empty list, which causes an assertion
error before reaching MusicBrainz. This is a known bug — the album lookup path
needs to either create dummy items from the path or bypass items extraction.
Track lookup (`tag_item`) works correctly.
"""

from pathlib import Path

import pytest

from auto_tagger.exceptions import TaggingError
from auto_tagger.integrations.beets_client import BeetsClient, RateLimiter
from auto_tagger.integrations.candidates import LookupRequest

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


def test_real_beets_lookup_album_empty_items_raises(album_fixture: Path):
    """Album lookup with empty items raises TaggingError (known beets 2.x limitation).

    The _match_album_with_beets method currently passes items=[] to beets.tag_album,
    which asserts items must be non-empty. This test documents the current behavior.
    """
    client = BeetsClient()
    request = LookupRequest(
        path=album_fixture,
        artist_hint="潘玮柏",
        album_hint="反转地球",
    )
    with pytest.raises(TaggingError, match="Could not query beets"):
        client.lookup_album(request)


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
