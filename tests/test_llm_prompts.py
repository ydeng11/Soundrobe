"""Tests for compact LLM prompt builders."""

from pathlib import Path


def test_selection_prompt_contains_candidate_summary_without_raw_paths():
    """Selection prompt includes hints and candidates but avoids full paths."""
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest, LookupSource
    from auto_tagger.llm.prompts import build_selection_messages

    messages = build_selection_messages(
        LookupRequest(
            path=Path("/very/long/Artist/Album"),
            artist_hint="Artist",
            album_hint="Album",
        ),
        [
            AlbumCandidate(
                artist="Artist",
                album="Album",
                year="2024",
                musicbrainz_albumid="album-id",
                source=LookupSource.BEETS,
            )
        ],
    )
    content = "\n".join(message["content"] for message in messages)

    assert "selected_index" in content
    assert "album-id" in content
    assert "/very/long" not in content


def test_fallback_prompt_warns_not_to_invent_musicbrainz_ids():
    """Fallback prompt explicitly forbids invented MusicBrainz IDs."""
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest, LookupSource
    from auto_tagger.llm.prompts import build_fallback_messages

    messages = build_fallback_messages(
        LookupRequest(path=Path("/music/Artist/Album"), artist_hint="Artist", album_hint="Album"),
        AlbumCandidate(artist="Artist", album="Album", source=LookupSource.FOLDER),
        current_metadata=[],
    )
    content = "\n".join(message["content"] for message in messages)

    assert "Do not invent MusicBrainz IDs" in content
    assert "tracks" in content
