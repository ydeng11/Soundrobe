"""Tests for SQLite match cache."""

from pathlib import Path


def test_match_cache_stores_and_loads_candidates(tmp_path):
    """Match cache persists normalized candidates by request hash."""
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest, LookupSource

    cache = MatchCache(tmp_path / "nested" / "cache.db")
    request = LookupRequest(
        path=Path("/music/Artist/Album"),
        artist_hint="Artist",
        album_hint="Album",
    )
    candidates = [
        AlbumCandidate(
            artist="Artist",
            album="Album",
            album_artist="Artist",
            musicbrainz_albumid="album-id",
            source=LookupSource.BEETS,
        )
    ]

    assert cache.get(request) is None

    cache.set(request, candidates)

    assert cache.get(request) == candidates
    assert (tmp_path / "nested" / "cache.db").exists()


def test_match_cache_key_includes_track_hints(tmp_path):
    """Different track lists produce different cache entries."""
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.candidates import (
        AlbumCandidate,
        LookupRequest,
        LookupSource,
        TrackCandidate,
    )

    cache = MatchCache(tmp_path / "cache.db")
    first = LookupRequest(path=Path("/music/A/B"), artist_hint="A", album_hint="B")
    second = LookupRequest(
        path=Path("/music/A/B"),
        artist_hint="A",
        album_hint="B",
        tracks=[TrackCandidate(title="Different")],
    )
    cache.set(first, [AlbumCandidate(artist="A", album="B", source=LookupSource.BEETS)])

    assert cache.get(second) is None
