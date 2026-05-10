"""Tests for DatasetIndexWriter and CSV index building."""

import csv
from pathlib import Path

from auto_tagger.integrations.candidates import LookupRequest, LookupSource
from auto_tagger.integrations.dataset import (
    DatasetIndexClient,
    DatasetIndexWriter,
    DatasetState,
    build_index_from_csv_tree,
    load_dataset_state,
    normalize_lookup_text,
    save_dataset_state,
)


def test_dataset_state_round_trip(tmp_path: Path):
    """DatasetState serializes and deserializes correctly."""
    original = DatasetState(
        version="01 January 2025",
        services=["musicbrainz", "spotify"],
        source_file="Dataset 01 January 2025.torrent",
        built_at="2025-01-01T00:00:00+00:00",
        album_rows=100,
        track_rows=500,
    )
    state_path = tmp_path / "state.json"
    save_dataset_state(state_path, original)
    loaded = load_dataset_state(state_path)
    assert loaded is not None
    assert loaded.version == original.version
    assert loaded.services == original.services
    assert loaded.album_rows == 100
    assert loaded.track_rows == 500


def test_dataset_index_writer_adds_album(tmp_path: Path):
    """DatasetIndexWriter stores an album and its tracks."""
    index_path = tmp_path / "index.sqlite"
    writer = DatasetIndexWriter(index_path)
    album_id = writer.add_album(
        source="musicbrainz",
        artist="Test Artist",
        album="Test Album",
        album_artist="Test Artist",
        year="2024",
        genre="Rock",
        musicbrainz_albumid="test-mbid-123",
        tracks=[
            {
                "title": "Track One",
                "artist": "Test Artist",
                "track_number": 1,
                "track_total": 2,
                "disc_number": 1,
                "disc_total": 1,
                "length": 240.0,
            },
            {
                "title": "Track Two",
                "artist": "Test Artist",
                "track_number": 2,
                "track_total": 2,
                "disc_number": 1,
                "disc_total": 1,
                "length": 180.0,
            },
        ],
    )
    writer.close()

    assert album_id > 0
    assert writer.album_rows == 1
    assert writer.track_rows == 2


def test_dataset_index_client_looks_up_written_album(tmp_path: Path):
    """DatasetIndexClient finds an album previously written by DatasetIndexWriter."""
    index_path = tmp_path / "index.sqlite"

    writer = DatasetIndexWriter(index_path)
    writer.add_album(
        source="musicbrainz",
        artist="潘玮柏",
        album="反转地球",
        year="2006",
        tracks=[
            {"title": "反轉地球", "artist": "潘玮柏", "track_number": 1, "track_total": 11},
        ],
    )
    writer.close()

    client = DatasetIndexClient(index_path)
    request = LookupRequest(
        path=tmp_path,
        artist_hint="潘玮柏",
        album_hint="反转地球",
    )
    candidates = client.lookup_album(request)

    assert len(candidates) == 1
    assert candidates[0].artist == "潘玮柏"
    assert candidates[0].album == "反转地球"
    assert candidates[0].source is LookupSource.DATASET
    assert len(candidates[0].tracks) == 1
    assert candidates[0].tracks[0].title == "反轉地球"


def test_dataset_index_client_returns_empty_on_missing_index(tmp_path: Path):
    """DatasetIndexClient returns empty list when index file is missing."""
    client = DatasetIndexClient(tmp_path / "nonexistent.sqlite")
    request = LookupRequest(path=tmp_path, artist_hint="X", album_hint="Y")
    candidates = client.lookup_album(request)
    assert candidates == []
    assert client.last_warning is not None
    assert "not found" in client.last_warning


def test_build_index_from_csv_tree(tmp_path: Path):
    """build_index_from_csv_tree imports a minimal CSV tree."""
    csv_dir = tmp_path / "csv"
    musicbrainz_dir = csv_dir / "musicbrainz"
    musicbrainz_dir.mkdir(parents=True)

    album_csv = musicbrainz_dir / "musicbrainz_album_2025.csv"
    with album_csv.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(
            f,
            fieldnames=[
                "artist", "album", "year", "genre",
                "albumartist", "album_id",
            ],
        )
        writer.writeheader()
        writer.writerow({
            "artist": "Test Artist",
            "album": "Test Album",
            "year": "2024",
            "genre": "Pop",
            "albumartist": "Test Artist",
            "album_id": "mbid-123",
        })

    index_path = tmp_path / "index.sqlite"
    album_rows, track_rows = build_index_from_csv_tree(
        csv_dir, index_path, services=["musicbrainz"]
    )

    assert album_rows >= 1
    assert track_rows >= 0

    client = DatasetIndexClient(index_path)
    candidates = client.lookup_album(
        LookupRequest(path=tmp_path, artist_hint="Test Artist", album_hint="Test Album")
    )
    assert len(candidates) >= 1


def test_normalize_lookup_text():
    """Normalize function strips punctuation and lowercases."""
    assert normalize_lookup_text("潘玮柏") == "潘玮柏"
    assert normalize_lookup_text("Test Artist!") == "test artist"
    assert normalize_lookup_text("  Multiple   Spaces  ") == "multiple spaces"
    assert normalize_lookup_text(None) == ""
    assert normalize_lookup_text("") == ""


def test_dataset_index_writer_handles_none_artist(tmp_path: Path):
    """add_album returns 0 and does not crash when artist is None."""
    index_path = tmp_path / "index.sqlite"
    writer = DatasetIndexWriter(index_path)
    album_id = writer.add_album(source="musicbrainz", artist=None, album="Album")
    writer.close()
    assert album_id == 0


def test_dataset_index_client_normalizes_chinese_text(tmp_path: Path):
    """Chinese artist/album names are normalized correctly for lookup."""
    index_path = tmp_path / "index.sqlite"
    writer = DatasetIndexWriter(index_path)
    writer.add_album(
        source="musicbrainz",
        artist="潘玮柏",
        album="反转地球",
        year="2006",
        tracks=[],
    )
    writer.close()

    client = DatasetIndexClient(index_path)
    candidates = client.lookup_album(
        LookupRequest(path=tmp_path, artist_hint="潘玮柏", album_hint="反转地球")
    )
    assert len(candidates) == 1
