"""Tests for lyrics discovery and embedding."""

from pathlib import Path

from auto_tagger.core.audio import AudioFormat
from auto_tagger.features.lyrics import discover_lyrics, embed_lyrics


def test_discover_lyrics_prefers_matching_lrc(tmp_path: Path):
    """Matching LRC sidecar is discovered as synchronized lyrics."""
    audio = tmp_path / "01.flac"
    audio.touch()
    lrc = tmp_path / "01.lrc"
    lrc.write_text("[00:01.00]Line\n", encoding="utf-8")

    payload = discover_lyrics(audio)

    assert payload is not None
    assert payload.synchronized is True
    assert payload.source_path == lrc
    assert "Line" in payload.text


def test_discover_lyrics_falls_back_to_txt(tmp_path: Path):
    """Matching TXT sidecar is discovered as unsynchronized lyrics."""
    audio = tmp_path / "01.flac"
    audio.touch()
    txt = tmp_path / "01.txt"
    txt.write_text("Plain lyrics\n", encoding="utf-8")

    payload = discover_lyrics(audio)

    assert payload is not None
    assert payload.synchronized is False
    assert payload.text == "Plain lyrics\n"


def test_embed_lyrics_writes_vorbis_lyrics():
    """Vorbis-style tags receive LYRICS text."""
    tags: dict[str, list[str]] = {}

    embed_lyrics(AudioFormat.FLAC, tags, "Plain lyrics")

    assert tags["LYRICS"] == ["Plain lyrics"]


def test_embed_lyrics_writes_mp4_lyrics():
    """MP4-like tags receive the lyrics atom."""
    tags: dict[str, list[str]] = {}

    embed_lyrics(AudioFormat.M4A, tags, "Plain lyrics")

    assert tags["\xa9lyr"] == ["Plain lyrics"]
