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


def test_folder_name_extraction_prompt_includes_folder_and_parent(tmp_path):
    """Extraction prompt includes the album folder name and parent context."""
    from auto_tagger.llm.prompts import build_folder_extraction_messages

    messages = build_folder_extraction_messages(
        folder_name="陈慧琳.Especial 新曲+精选 CD1",
        parent_name="2006.陈慧琳.Especial 新曲+精选 3CD",
    )
    content = "\n".join(message["content"] for message in messages)

    assert "folder_name" in content
    assert "陈慧琳.Especial" in content
    assert "parent_name" in content
    assert "2006.陈慧琳" in content


def test_folder_name_extraction_prompt_requests_json_output():
    """Prompt asks for structured JSON with artist, album, year, disc."""
    from auto_tagger.llm.prompts import build_folder_extraction_messages

    messages = build_folder_extraction_messages(
        folder_name="陈慧琳.Especial 新曲+精选 CD1",
        parent_name="2006.陈慧琳.Especial 新曲+精选 3CD",
    )
    content = "\n".join(message["content"] for message in messages)

    assert "artist" in content
    assert "album" in content
    assert "year" in content
    assert "disc" in content


def test_folder_name_extraction_without_parent(tmp_path):
    """Parent name is optional — extraction works on folder name alone."""
    from auto_tagger.llm.prompts import build_folder_extraction_messages

    messages = build_folder_extraction_messages(folder_name="2006 - Greatest Hits")
    content = "\n".join(message["content"] for message in messages)

    assert "2006 - Greatest Hits" in content
    assert "parent" not in content or "parent_name" not in content or "null" in content


def test_folder_name_extraction_multi_artist():    
    """Prompt handles multi-artist folder like 陈慧琳.陈小春.拉阔演奏厅."""
    from auto_tagger.llm.prompts import build_folder_extraction_messages

    messages = build_folder_extraction_messages(
        folder_name="2006.陈慧琳.陈小春.拉阔演奏厅",
    )
    content = "\n".join(message["content"] for message in messages)

    assert "2006.陈慧琳.陈小春.拉阔演奏厅" in content
    assert "artist" in content
    assert "album" in content


def test_folder_name_extraction_cd_subfolder_disc_hint():
    """CD1 in the folder name should hint at disc number."""
    from auto_tagger.llm.prompts import build_folder_extraction_messages

    messages = build_folder_extraction_messages(
        folder_name="陈慧琳.Especial 新曲+精选 CD1",
        parent_name="2006.陈慧琳.Especial 新曲+精选 3CD",
    )
    content = "\n".join(message["content"] for message in messages)

    assert "disc" in content
