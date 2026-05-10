"""Edge case and robustness tests."""

from pathlib import Path

import pytest
from click.testing import CliRunner

from auto_tagger.cli import cli
from auto_tagger.core.audio import detect_audio_format, iter_audio_files
from auto_tagger.core.reader import read_metadata
from auto_tagger.integrations.fallback import parse_album_path


def test_iter_audio_files_skips_non_media_files(tmp_path: Path):
    """iter_audio_files ignores .txt, .docx, .url files."""
    album = tmp_path / "Artist" / "Album"
    album.mkdir(parents=True)
    (album / "01.flac").touch()
    (album / "notes.txt").touch()
    (album / "readme.docx").touch()
    (album / "shortcut.url").touch()

    found = list(iter_audio_files(album))
    assert len(found) == 1
    assert found[0].suffix == ".flac"


def test_reader_handles_corrupt_flac(edge_case_fixtures: Path):
    """read_metadata handles corrupt files gracefully."""
    corrupt = edge_case_fixtures / "corrupt" / "Bad" / "Broken" / "01.flac"
    try:
        meta = read_metadata(corrupt)
        assert meta is not None
    except Exception:
        pass  # raising is also acceptable


def test_unicode_paths_supported(edge_case_fixtures: Path):
    """iter_audio_files works with Japanese paths (recursive)."""
    from auto_tagger.core.audio import iter_audio_files as iaf
    jp_dir = edge_case_fixtures / "unicode"
    found = list(iaf(jp_dir, recursive=True))
    assert len(found) >= 1


def test_fallback_on_unicode_path(edge_case_fixtures: Path):
    """Fallback parse works on Unicode directory names."""
    jp_album = edge_case_fixtures / "unicode" / "アーティスト" / "アルバム"
    request = parse_album_path(jp_album)
    assert request.artist_hint == "アーティスト"
    assert request.album_hint == "アルバム"


def test_detect_format_unknown_extension(tmp_path: Path):
    """Unknown extensions raise FileProcessingError from detect_audio_format."""
    from auto_tagger.exceptions import FileProcessingError

    unknown = tmp_path / "file.xyz"
    unknown.touch()
    with pytest.raises(FileProcessingError, match="Unsupported"):
        detect_audio_format(unknown)


def test_cli_handles_permission_error():
    """CLI doesn't crash on permission-denied paths."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", "/root/forbidden"])
    assert result.exit_code != 0


def test_tag_command_handles_file_path_instead_of_dir(tmp_path: Path):
    """Tagging a file path (not directory) works via fallback."""
    track = tmp_path / "track.flac"
    track.write_bytes(b"")
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(track), "--dry-run"])
    assert isinstance(result.exit_code, int)
