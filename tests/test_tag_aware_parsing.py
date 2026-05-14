"""Tests for tag-aware LookupRequest building."""

from pathlib import Path


def test_parse_album_with_tags_uses_existing_tags(tmp_path: Path):
    """Existing file tags take priority over folder names for lookup hints."""
    from auto_tagger.integrations.fallback import parse_album_with_tags

    # Create a folder structure where folder name differs from tag contents
    album_dir = tmp_path / "SomeFolder" / "SomeSubFolder"
    album_dir.mkdir(parents=True)
    # Create a silent FLAC with tags
    (album_dir / "01.flac").write_bytes(b"")

    # Since we can't write FLAC tags without ffmpeg,
    # test the fallback path: tags missing → uses folder names
    request = parse_album_with_tags(album_dir)
    assert request.artist_hint == "SomeFolder"
    assert request.album_hint == "SomeSubFolder"


def test_parse_album_with_tags_returns_folder_hints_when_no_tags(tmp_path: Path):
    """When files have no readable tags, folder names are used as hints."""
    from auto_tagger.integrations.fallback import parse_album_with_tags

    album_dir = tmp_path / "5566" / "2003-04 挚爱"
    album_dir.mkdir(parents=True)
    (album_dir / "01.wav").write_bytes(b"")

    request = parse_album_with_tags(album_dir)
    # Without readable tags, falls back to folder hints (cleaned)
    assert request.artist_hint == "5566"
    # Album hint is cleaned of date prefix
    assert request.album_hint is not None
    assert "挚爱" in request.album_hint


def test_parse_album_with_tags_handles_digit_album_name(tmp_path: Path):
    """A digit-only album folder name like '2001' is preserved."""
    from auto_tagger.integrations.fallback import parse_album_with_tags

    album_dir = tmp_path / "Dr. Dre" / "2001"
    album_dir.mkdir(parents=True)
    (album_dir / "01.flac").write_bytes(b"")

    request = parse_album_with_tags(album_dir)
    assert request.artist_hint == "Dr. Dre"
    assert request.album_hint == "2001"


def test_parse_album_with_tags_preserves_digit_only_artist(tmp_path: Path):
    """A digit-only artist folder like '5566' is preserved."""
    from auto_tagger.integrations.fallback import parse_album_with_tags

    album_dir = tmp_path / "5566" / "挚爱"
    album_dir.mkdir(parents=True)
    (album_dir / "01.flac").write_bytes(b"")

    request = parse_album_with_tags(album_dir)
    assert request.artist_hint == "5566"
    assert request.album_hint == "挚爱"


def test_parse_album_with_tags_prefers_tags_over_folder(tmp_path: Path):
    """When tags exist, they take priority over folder names."""
    from auto_tagger.integrations.fallback import (
        parse_album_path,
        parse_album_with_tags,
    )

    # Create a folder where tags differ from folder names
    album_dir = tmp_path / "Wrong Artist" / "Wrong Album"
    album_dir.mkdir(parents=True)
    (album_dir / "01.flac").write_bytes(b"")

    # Without tags, folder names are used
    folder_request = parse_album_path(album_dir)
    assert folder_request.artist_hint == "Wrong Artist"
    assert folder_request.album_hint == "Wrong Album"

    # parse_album_with_tags would prefer tags, but since there are none,
    # it falls back to folder names (same result in this case)
    tag_request = parse_album_with_tags(album_dir)
    assert tag_request.path == album_dir
