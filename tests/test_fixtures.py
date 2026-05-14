"""Validate that synthetic fixtures are correctly generated.

All tests use the session-scoped fixtures from conftest.py, which are
generated once and reused across the entire test suite.
"""

from pathlib import Path

import pytest
from mutagen.flac import FLAC

from auto_tagger.core.audio import detect_audio_format, iter_audio_files
from auto_tagger.core.reader import read_metadata

# ── album fixture ──────────────────────────────────────────────

def test_album_fixture_has_eleven_flac_files(album_fixture: Path):
    """Synthetic album fixture contains 11 FLAC tracks."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    assert len(flacs) == 11


def test_album_fixture_has_eleven_lrc_files(album_fixture: Path):
    """Synthetic album fixture contains 11 LRC files."""
    lrcs = sorted(album_fixture.rglob("*.lrc"))
    assert len(lrcs) == 11


def test_album_fixture_has_cover_image(album_fixture: Path):
    """Synthetic album fixture contains a cover.jpg."""
    assert (album_fixture / "cover.jpg").exists()


def test_album_fixture_flac_has_correct_tags(album_fixture: Path):
    """First FLAC track has expected Vorbis comment tags."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    audio = FLAC(flacs[0])
    tags = dict(audio.tags or {})
    assert tags["artist"] == ["潘玮柏"]
    assert tags["album"] == ["反转地球"]
    assert tags["date"] == ["2006"]
    assert tags["tracktotal"] == ["11"]


def test_album_fixture_tracks_have_unique_titles(album_fixture: Path):
    """All 11 tracks have distinct titles."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    titles = {FLAC(f).tags["title"][0] for f in flacs}
    assert len(titles) == 11
    assert "反轉地球" in titles
    assert "Pan@sonic" in titles


def test_album_fixture_iter_audio_files_works(album_fixture: Path):
    """iter_audio_files finds all 11 FLAC files."""
    found = list(iter_audio_files(album_fixture))
    assert len(found) == 11


def test_album_fixture_read_metadata_works(album_fixture: Path):
    """read_metadata returns correct metadata for the first track."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    meta = read_metadata(flacs[0])
    assert meta.title == "反轉地球"
    assert meta.artist == "潘玮柏"
    assert meta.album == "反转地球"
    assert meta.track_number == 1
    assert meta.year == "2006"


def test_album_fixture_detect_audio_format(album_fixture: Path):
    """detect_audio_format identifies FLAC files correctly."""
    flacs = sorted(album_fixture.rglob("*.flac"))
    fmt = detect_audio_format(flacs[0])
    assert fmt is not None
    assert fmt.name == "FLAC"


# ── compilation fixture ────────────────────────────────────────

def test_compilation_fixture_has_three_tracks(compilation_fixture: Path):
    """Multi-artist compilation has 3 tracks."""
    flacs = sorted(compilation_fixture.rglob("*.flac"))
    assert len(flacs) == 3


def test_compilation_fixture_has_different_artists(compilation_fixture: Path):
    """Compilation tracks have different artists."""
    flacs = sorted(compilation_fixture.rglob("*.flac"))
    artists = {FLAC(f).tags["artist"][0] for f in flacs}
    assert len(artists) >= 2


# ── format fixtures ────────────────────────────────────────────

def test_format_fixtures_has_all_formats(format_fixtures: Path):
    """Formats directory contains .flac, .mp3, .m4a."""
    assert (format_fixtures / "test.flac").exists()
    assert (format_fixtures / "test.mp3").exists()
    assert (format_fixtures / "test.m4a").exists()


def test_format_fixtures_tags_readable(format_fixtures: Path):
    """All format files have readable metadata."""
    for ext in ("flac", "mp3", "m4a"):
        meta = read_metadata(format_fixtures / f"test.{ext}")
        assert meta.title == "Test Track"
        assert meta.artist == "Test Artist"


# ── edge case fixtures ─────────────────────────────────────────

def test_edge_case_empty_tags_dir_exists(edge_case_fixtures: Path):
    """Empty tags fixture directory is populated."""
    empty_dir = edge_case_fixtures / "empty_tags"
    assert list(empty_dir.rglob("*.flac"))


def test_edge_case_corrupt_file_exists(edge_case_fixtures: Path):
    """Corrupt fixture has a .flac file that isn't valid audio."""
    corrupt = edge_case_fixtures / "corrupt" / "Bad" / "Broken" / "01.flac"
    assert corrupt.exists()
    # Should not be valid FLAC — mutagen will raise
    with pytest.raises(Exception):
        FLAC(corrupt)


def test_edge_case_missing_cover_dir_has_no_jpg(edge_case_fixtures: Path):
    """Missing cover fixture has no cover image."""
    jpgs = list((edge_case_fixtures / "missing_cover").rglob("*.jpg"))
    assert len(jpgs) == 0


def test_edge_case_unicode_path_exists(edge_case_fixtures: Path):
    """Unicode path fixture contains a FLAC file."""
    unicode_dir = edge_case_fixtures / "unicode"
    assert list(unicode_dir.rglob("*.flac"))
