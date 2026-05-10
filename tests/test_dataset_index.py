"""Tests for the local dataset SQLite index."""

from pathlib import Path


def test_dataset_index_client_matches_normalized_artist_album(tmp_path: Path):
    """SQLite dataset rows become normalized lookup candidates."""
    from auto_tagger.integrations.candidates import LookupRequest, LookupSource
    from auto_tagger.integrations.dataset import DatasetIndexClient, DatasetIndexWriter

    index_path = tmp_path / "dataset-index.sqlite"
    writer = DatasetIndexWriter(index_path)
    writer.add_album(
        source="musicbrainz",
        artist="The Artist",
        album="Album!",
        album_artist="The Artist",
        year="2024",
        genre="Pop",
        musicbrainz_albumid="album-id",
        musicbrainz_artistid="artist-id",
        tracks=[
            {
                "title": "Song One",
                "artist": "The Artist",
                "track_number": 1,
                "track_total": 1,
                "musicbrainz_trackid": "track-id",
                "length": 181.5,
            }
        ],
    )
    writer.close()

    client = DatasetIndexClient(index_path, max_candidates=3)
    candidates = client.lookup_album(
        LookupRequest(
            path=tmp_path / "THE ARTIST" / "Album",
            artist_hint="THE ARTIST",
            album_hint="Album",
        )
    )

    assert len(candidates) == 1
    assert candidates[0].source is LookupSource.DATASET
    assert candidates[0].artist == "The Artist"
    assert candidates[0].album == "Album!"
    assert candidates[0].musicbrainz_albumid == "album-id"
    assert candidates[0].tracks[0].title == "Song One"
    assert candidates[0].tracks[0].musicbrainz_trackid == "track-id"


def test_dataset_asset_parser_selects_latest_torrent():
    """GitHub repository asset metadata is parsed without downloading archives."""
    from auto_tagger.integrations.dataset import parse_dataset_assets

    assets = parse_dataset_assets(
        [
            {
                "name": "MusicBrainz Tidal Spotify Dataset 07 June 2025.7z.torrent",
                "download_url": "https://example.invalid/june.torrent",
            },
            {
                "name": "MusicBrainz Tidal Spotify Deezer Dataset 22 Feb 2026.torrent",
                "download_url": "https://example.invalid/feb.torrent",
            },
        ]
    )

    assert assets[0].version == "22 Feb 2026"
    assert assets[0].services == ["musicbrainz", "tidal", "spotify", "deezer"]
    assert assets[0].download_url == "https://example.invalid/feb.torrent"
