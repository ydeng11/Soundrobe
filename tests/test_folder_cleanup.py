"""Tests for folder name cleanup in fallback lookup."""

from pathlib import Path

from auto_tagger.integrations.fallback import (
    clean_folder_name,
    extract_year_from_name,
    parse_album_path,
)


def test_extract_year_from_name():
    """Year is extracted from leading date prefix."""
    assert extract_year_from_name("2003-04《挚爱》") == "2003"
    assert extract_year_from_name("2005-08《好久不见》") == "2005"
    assert extract_year_from_name("2008-01《喝采》") == "2008"
    assert extract_year_from_name("2005.08 Album Name") == "2005"
    assert extract_year_from_name("2017- Album") == "2017"
    assert extract_year_from_name("Album Name") is None
    assert extract_year_from_name("5566") is None
    assert extract_year_from_name("挚爱") is None


def test_clean_date_prefix():
    """Date prefixes like '2003-04' are stripped."""
    assert clean_folder_name("2003-04《挚爱》") == "挚爱"
    assert clean_folder_name("2005-08《好久不见》") == "好久不见"
    assert clean_folder_name("2008-01《喝采》") == "喝采"


def test_clean_date_prefix_dot():
    """Date prefixes with dots like '2005.08' are stripped."""
    assert clean_folder_name("2005.08 Album Name") == "Album Name"


def test_clean_bookmarks():
    """Chinese bookmarks 《》「」【】 are stripped."""
    assert clean_folder_name("《挚爱》") == "挚爱"
    assert clean_folder_name("「Single」") == "Single"
    assert clean_folder_name("【Album】") == "Album"


def test_clean_extra_suffix():
    """Trailing parenthetical suffixes are stripped."""
    assert clean_folder_name("Album Name (FLAC分轨)") == "Album Name"
    assert clean_folder_name("Hello (Bonus Track Edition)") == "Hello"


def test_clean_combined():
    """Multiple patterns are cleaned together."""
    assert clean_folder_name("2003-04《挚爱》(FLAC)") == "挚爱"


def test_clean_no_change():
    """Names without patterns to strip are returned unchanged."""
    assert clean_folder_name("Album") == "Album"
    assert clean_folder_name("Artist Name") == "Artist Name"
    assert clean_folder_name("5566") == "5566"


def test_clean_empty_returns_original():
    """If everything is stripped, return the original."""
    assert clean_folder_name("( )") == "( )"


def test_parse_album_path_cleans_names(tmp_path: Path):
    """parse_album_path uses cleaned folder names for hints."""
    album = tmp_path / "5566" / "2003-04《挚爱》"
    album.mkdir(parents=True)
    (album / "01.wav").touch()

    request = parse_album_path(album)
    assert request.artist_hint == "5566"
    assert request.album_hint == "挚爱"
    assert request.year_hint == "2003"
    assert request.path == album


def test_parse_album_path_no_year(tmp_path: Path):
    """parse_album_path returns None year_hint when folder has no date prefix."""
    album = tmp_path / "Artist" / "Album Name"
    album.mkdir(parents=True)
    (album / "01.flac").touch()

    request = parse_album_path(album)
    assert request.artist_hint == "Artist"
    assert request.album_hint == "Album Name"
    assert request.year_hint is None
