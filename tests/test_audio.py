"""Tests for audio file discovery and loading."""

from pathlib import Path

import pytest

from auto_tagger.exceptions import FileProcessingError


def test_detect_audio_format_by_extension():
    """Supported extensions map to normalized audio formats."""
    from auto_tagger.core.audio import AudioFormat, detect_audio_format

    assert detect_audio_format(Path("track.mp3")) is AudioFormat.MP3
    assert detect_audio_format(Path("track.FLAC")) is AudioFormat.FLAC
    assert detect_audio_format(Path("track.m4a")) is AudioFormat.M4A
    assert detect_audio_format(Path("track.mp4")) is AudioFormat.M4A


def test_detect_audio_format_rejects_unknown_extension():
    """Unsupported extensions raise a project-level file error."""
    from auto_tagger.core.audio import detect_audio_format

    with pytest.raises(FileProcessingError, match="Unsupported audio format"):
        detect_audio_format(Path("cover.jpg"))


def test_iter_audio_files_returns_sorted_supported_files(tmp_path):
    """Album discovery returns deterministic supported file paths only."""
    from auto_tagger.core.audio import iter_audio_files

    album = tmp_path / "album"
    album.mkdir()
    (album / "02.flac").write_bytes(b"")
    (album / "01.mp3").write_bytes(b"")
    (album / "cover.jpg").write_bytes(b"")
    (album / "nested").mkdir()
    (album / "nested" / "03.m4a").write_bytes(b"")

    assert [path.name for path in iter_audio_files(album)] == ["01.mp3", "02.flac"]
    assert [path.name for path in iter_audio_files(album, recursive=True)] == [
        "01.mp3",
        "02.flac",
        "03.m4a",
    ]


def test_iter_audio_files_raises_when_no_audio_found(tmp_path):
    """Discovery fails clearly when a path contains no supported files."""
    from auto_tagger.core.audio import iter_audio_files

    (tmp_path / "notes.txt").write_text("not audio")

    with pytest.raises(FileProcessingError, match="No supported audio files"):
        iter_audio_files(tmp_path)
