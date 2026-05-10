"""Lyrics sidecar discovery and embedding helpers."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

from auto_tagger.core.audio import AudioFormat
from auto_tagger.quality.lrc import validate_lrc_file


@dataclass(frozen=True)
class LyricsPayload:
    """Lyrics text discovered for an audio file."""

    text: str
    source_path: Path
    synchronized: bool
    encoding: str | None


def discover_lyrics(audio_path: Path) -> LyricsPayload | None:
    """Discover matching LRC or TXT lyrics for an audio file."""
    lrc_path = audio_path.with_suffix(".lrc")
    if lrc_path.exists():
        result = validate_lrc_file(lrc_path)
        if result.text:
            return LyricsPayload(result.text, lrc_path, True, result.encoding)

    txt_path = audio_path.with_suffix(".txt")
    if txt_path.exists():
        return LyricsPayload(txt_path.read_text(encoding="utf-8"), txt_path, False, "utf-8")
    return None


def embed_lyrics(audio_format: AudioFormat, tags: Any, lyrics: str) -> None:
    """Embed unsynchronized lyrics into a format-specific tag object."""
    if audio_format is AudioFormat.MP3:
        from mutagen.id3 import USLT

        tags.delall("USLT")
        tags.add(USLT(encoding=3, lang="eng", desc="", text=lyrics))
    elif audio_format is AudioFormat.M4A:
        tags["\xa9lyr"] = [lyrics]
    else:
        tags["LYRICS"] = [lyrics]
