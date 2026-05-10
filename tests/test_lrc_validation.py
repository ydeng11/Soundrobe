"""Tests for LRC file validation and UTF-8 conversion."""

from pathlib import Path

from auto_tagger.quality.health import HealthSeverity
from auto_tagger.quality.lrc import convert_lrc_to_utf8, discover_lrc_files, validate_lrc_file


def test_discover_lrc_files_finds_album_and_adjacent_lyrics(tmp_path: Path):
    """LRC discovery returns sorted unique .lrc files near audio files."""
    audio = tmp_path / "01.flac"
    audio.touch()
    album_lrc = tmp_path / "album.lrc"
    adjacent_lrc = tmp_path / "01.lrc"
    album_lrc.write_text("[ti:Album]\n", encoding="utf-8")
    adjacent_lrc.write_text("[00:01.00]Line\n", encoding="utf-8")

    assert discover_lrc_files(tmp_path, [audio]) == [adjacent_lrc, album_lrc]


def test_validate_lrc_file_accepts_utf8_timing_and_metadata(tmp_path: Path):
    """Valid UTF-8 LRC content with metadata and timing tags has no issues."""
    lrc = tmp_path / "song.lrc"
    lrc.write_text("[ar:Artist]\n[ti:Song]\n[00:01.20]Lyric\n", encoding="utf-8")

    result = validate_lrc_file(lrc)

    assert result.encoding == "utf-8"
    assert result.needs_conversion is False
    assert result.issues == []


def test_validate_lrc_file_warns_for_legacy_encoding(tmp_path: Path):
    """Legacy encoded LRC files decode but request UTF-8 conversion."""
    lrc = tmp_path / "song.lrc"
    lrc.write_bytes("[00:01.00]café\n".encode("cp1252"))

    result = validate_lrc_file(lrc)

    assert result.needs_conversion is True
    assert result.encoding in {"cp1252", "latin-1"}
    assert result.issues[0].severity == HealthSeverity.WARNING
    assert result.issues[0].code == "lrc.non_utf8"


def test_validate_lrc_file_flags_malformed_timing(tmp_path: Path):
    """Broken bracket timing tags are reported as LRC format warnings."""
    lrc = tmp_path / "song.lrc"
    lrc.write_text("[00:xx.00]Lyric\n", encoding="utf-8")

    result = validate_lrc_file(lrc)

    assert result.issues[0].code == "lrc.malformed_tag"


def test_convert_lrc_to_utf8_writes_only_when_not_dry_run(tmp_path: Path):
    """Conversion returns a dry-run plan first and writes UTF-8 only in apply mode."""
    lrc = tmp_path / "song.lrc"
    lrc.write_bytes("[00:01.00]café\n".encode("cp1252"))

    dry_run = convert_lrc_to_utf8(lrc, dry_run=True)
    assert dry_run.needs_conversion is True
    assert lrc.read_bytes() != "[00:01.00]café\n".encode()

    applied = convert_lrc_to_utf8(lrc, dry_run=False)
    assert applied.needs_conversion is False
    assert lrc.read_text(encoding="utf-8") == "[00:01.00]café\n"
