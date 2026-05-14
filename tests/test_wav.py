"""Tests for WAV file support."""

from pathlib import Path

from auto_tagger.core.audio import AudioFormat, detect_audio_format, iter_audio_files


def test_detect_wav_format(tmp_path: Path):
    """WAV files are detected as AudioFormat.WAV."""
    wav = tmp_path / "test.wav"
    wav.touch()
    assert detect_audio_format(wav) is AudioFormat.WAV


def test_iter_audio_files_finds_wav(tmp_path: Path):
    """WAV files are discovered by iter_audio_files."""
    (tmp_path / "01.flac").touch()
    (tmp_path / "02.wav").touch()
    (tmp_path / "03.txt").touch()

    found = list(iter_audio_files(tmp_path))
    suffixes = {f.suffix for f in found}
    assert ".wav" in suffixes
    assert ".flac" in suffixes
    assert len(found) == 2
