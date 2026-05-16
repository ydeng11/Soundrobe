"""Navidrome-specific enrichment features."""

from auto_tagger.features.artist_artwork import (
    ArtistArtworkOutcome,
    ArtistArtworkStatus,
    ArtistArtworkSummary,
    discover_artist_directories,
    find_local_artist_image,
    save_artist_image,
)
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
    "ArtistArtworkOutcome",
    "ArtistArtworkStatus",
    "ArtistArtworkSummary",
    "CompilationAnalysis",
    "CoverArtArchiveClient",
    "CoverArtImage",
    "CoverArtResult",
    "CoverArtStatus",
    "LyricsPayload",
    "analyze_compilation",
    "apply_compilation_tags",
    "discover_artist_directories",
    "discover_local_cover_art",
    "discover_lyrics",
    "embed_cover_art",
    "embed_lyrics",
    "find_local_artist_image",
    "save_artist_image",
]
