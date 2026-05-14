"""Tests that write_metadata with dry_run=False actually mutates files.

Each test copies a fixture file to a temp location, writes new metadata,
reads it back, and verifies the change. Original fixtures are never mutated.
"""

import shutil
from pathlib import Path

from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.core.reader import read_metadata
from auto_tagger.core.writer import write_metadata


def _copy_fixture_to_tmp(src: Path, tmp_path: Path) -> Path:
    """Copy a fixture file to a temp directory for safe mutation."""
    dest = tmp_path / src.name
    shutil.copy2(src, dest)
    return dest


def test_write_flac_persists_title_change(album_fixture: Path, tmp_path: Path):
    """Writing a new title to a FLAC file persists across re-reads."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    src = flacs[0]
    dest = _copy_fixture_to_tmp(src, tmp_path)

    # Read original
    original = read_metadata(dest)
    assert original.title == "反轉地球"

    # Write new metadata
    new_meta = TrackMetadata(
        title="New Title",
        artist="New Artist",
        album="New Album",
        track_number=1,
        year="2025",
    )
    write_metadata(dest, new_meta, dry_run=False)

    # Read back and verify
    reloaded = read_metadata(dest)
    assert reloaded.title == "New Title"
    assert reloaded.artist == "New Artist"
    assert reloaded.album == "New Album"
    assert reloaded.track_number == 1
    assert reloaded.year == "2025"


def test_write_flac_preserves_existing_tags(album_fixture: Path, tmp_path: Path):
    """Writing partial metadata preserves unmentioned existing tags."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    src = flacs[0]
    dest = _copy_fixture_to_tmp(src, tmp_path)

    original = read_metadata(dest)
    assert original.title is not None

    # Write only a new title, preserving everything else
    new_meta = TrackMetadata(title="Changed Title")
    write_metadata(dest, new_meta, dry_run=False)

    reloaded = read_metadata(dest)
    assert reloaded.title == "Changed Title"


def test_write_mp3_persists_changes(format_fixtures: Path, tmp_path: Path):
    """Writing to an MP3 file persists changes."""
    src = format_fixtures / "test.mp3"
    dest = _copy_fixture_to_tmp(src, tmp_path)

    new_meta = TrackMetadata(
        title="MP3 Title",
        artist="MP3 Artist",
        album="MP3 Album",
        track_number=5,
    )
    write_metadata(dest, new_meta, dry_run=False)

    reloaded = read_metadata(dest)
    assert reloaded.title == "MP3 Title"
    assert reloaded.artist == "MP3 Artist"
    assert reloaded.album == "MP3 Album"


def test_write_m4a_persists_changes(format_fixtures: Path, tmp_path: Path):
    """Writing to an M4A file persists changes."""
    src = format_fixtures / "test.m4a"
    dest = _copy_fixture_to_tmp(src, tmp_path)

    new_meta = TrackMetadata(
        title="M4A Title",
        artist="M4A Artist",
        album="M4A Album",
        track_number=3,
    )
    write_metadata(dest, new_meta, dry_run=False)

    reloaded = read_metadata(dest)
    assert reloaded.title == "M4A Title"
    assert reloaded.artist == "M4A Artist"
    assert reloaded.album == "M4A Album"


def test_write_dry_run_does_not_mutate(album_fixture: Path, tmp_path: Path):
    """dry_run=True returns new metadata but leaves file unchanged."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    src = flacs[0]
    dest = _copy_fixture_to_tmp(src, tmp_path)

    original = read_metadata(dest)
    original_title = original.title

    new_meta = TrackMetadata(title="Should Not Persist")
    result = write_metadata(dest, new_meta, dry_run=True)

    assert result.title == "Should Not Persist"  # returned to caller
    reloaded = read_metadata(dest)
    assert reloaded.title == original_title  # file unchanged


def test_write_all_album_tracks(album_fixture: Path, tmp_path: Path):
    """All 11 tracks in an album can be rewritten without corruption."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    for i, src in enumerate(flacs, start=1):
        dest = _copy_fixture_to_tmp(src, tmp_path)
        new_meta = TrackMetadata(
            title=f"Track {i:02d}",
            artist="Test Artist",
            album="Test Album",
            track_number=i,
            track_total=len(flacs),
            year="2025",
        )
        write_metadata(dest, new_meta, dry_run=False)
        reloaded = read_metadata(dest)
        assert reloaded.title == f"Track {i:02d}"
        assert reloaded.track_number == i


def test_write_roundtrip_all_formats(format_fixtures: Path, tmp_path: Path):
    """Round-trip write→read preserves metadata for all three formats."""
    tags = TrackMetadata(
        title="Roundtrip",
        artist="Test",
        album="Test Album",
        track_number=1,
        year="2025",
    )

    for ext in ("flac", "mp3", "m4a"):
        src = format_fixtures / f"test.{ext}"
        dest_dir = tmp_path / ext
        dest_dir.mkdir(exist_ok=True)
        dest = _copy_fixture_to_tmp(src, dest_dir)

        write_metadata(dest, tags, dry_run=False)
        reloaded = read_metadata(dest)
        assert reloaded.title == "Roundtrip", f"{ext} title mismatch"
        assert reloaded.artist == "Test", f"{ext} artist mismatch"
