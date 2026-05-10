"""Navidrome-specific enrichment features."""

from auto_tagger.features.compilations import (
    CompilationAnalysis,
    analyze_compilation,
    apply_compilation_tags,
)
from auto_tagger.features.cover_art import (
    CoverArtArchiveClient,
    CoverArtImage,
    CoverArtResult,
    CoverArtStatus,
    discover_local_cover_art,
    embed_cover_art,
)
from auto_tagger.features.lyrics import LyricsPayload, discover_lyrics, embed_lyrics

__all__ = [
    "CompilationAnalysis",
    "CoverArtArchiveClient",
    "CoverArtImage",
    "CoverArtResult",
    "CoverArtStatus",
    "LyricsPayload",
    "analyze_compilation",
    "apply_compilation_tags",
    "discover_local_cover_art",
    "discover_lyrics",
    "embed_cover_art",
    "embed_lyrics",
]
