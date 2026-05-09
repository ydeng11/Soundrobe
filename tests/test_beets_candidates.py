"""Tests for Beets lookup candidate models."""

from pathlib import Path


def test_album_candidate_round_trips_through_json():
    """Candidate models serialize to stable JSON-compatible dictionaries."""
    from auto_tagger.integrations.candidates import (
        AlbumCandidate,
        LookupSource,
        TrackCandidate,
    )

    candidate = AlbumCandidate(
        artist="Artist",
        artists=["Artist"],
        album="Album",
        album_artist="Artist",
        album_artists=["Artist"],
        year="2024",
        genre="Pop",
        musicbrainz_albumid="album-id",
        musicbrainz_artistid="artist-id",
        distance=0.12,
        source=LookupSource.BEETS,
        tracks=[
            TrackCandidate(
                title="Song",
                artist="Artist",
                artists=["Artist"],
                track_number=1,
                track_total=1,
                musicbrainz_trackid="track-id",
                length=180.5,
            )
        ],
    )

    restored = AlbumCandidate.from_dict(candidate.to_dict())

    assert restored == candidate
    assert restored.to_display_row() == [
        "beets",
        "Artist",
        "Album",
        "2024",
        "0.12",
        "album-id",
    ]


def test_lookup_request_hash_is_stable_for_same_query():
    """Request hashes depend on lookup hints and track hints, not object identity."""
    from auto_tagger.integrations.candidates import LookupRequest, TrackCandidate

    request = LookupRequest(
        path=Path("/music/Artist/Album"),
        artist_hint="Artist",
        album_hint="Album",
        tracks=[TrackCandidate(title="Song", track_number=1)],
    )
    same_request = LookupRequest.from_dict(request.to_dict())

    assert same_request.query_hash() == request.query_hash()
    assert same_request.path == request.path
