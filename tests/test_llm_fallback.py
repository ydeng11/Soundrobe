"""Tests for LLM fallback tag generation."""

from pathlib import Path


class FakeFallbackClient:
    """LLM client double for fallback generation."""

    def __init__(self, data):
        from auto_tagger.llm.cost import TokenUsage

        self.data = data
        self.usage = TokenUsage(prompt_tokens=20, completion_tokens=10, total_tokens=30)

    def complete_json(self, messages, schema, model=None):
        from auto_tagger.llm.client import LLMResponse

        return LLMResponse(data=self.data, usage=self.usage, model=model or "test/model")


def test_fallback_generation_returns_track_metadata():
    """Fallback generation converts validated LLM JSON into TrackMetadata."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest, LookupSource
    from auto_tagger.llm.fallback import FallbackTagGenerationService

    service = FallbackTagGenerationService(
        FakeFallbackClient(
            {
                "artist": "Artist",
                "artists": ["Artist"],
                "album": "Album",
                "album_artist": "Artist",
                "album_artists": ["Artist"],
                "tracks": [{"title": "Song", "artist": "Artist", "track_number": 1}],
                "confidence": 0.75,
                "reason": "folder hints",
            }
        ),
        Settings(llm_api_key="key"),
    )

    result = service.generate_tags(
        LookupRequest(path=Path("/music/Artist/Album"), artist_hint="Artist", album_hint="Album"),
        AlbumCandidate(artist="Artist", album="Album", source=LookupSource.FOLDER),
        current_metadata=[],
    )

    assert result.tracks[0].title == "Song"
    assert result.tracks[0].album == "Album"
    assert result.confidence == 0.75


def test_fallback_generation_rejects_non_folder_candidate():
    """Fallback generation only runs for folder-source candidates."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest, LookupSource
    from auto_tagger.llm.fallback import FallbackTagGenerationService

    result = FallbackTagGenerationService(
        FakeFallbackClient({}),
        Settings(llm_api_key="key"),
    ).generate_tags(
        LookupRequest(path=Path("/music/Artist/Album")),
        AlbumCandidate(artist="Artist", album="Album", source=LookupSource.BEETS),
        current_metadata=[],
    )

    assert result.tracks == []
    assert "folder-source" in result.reason
