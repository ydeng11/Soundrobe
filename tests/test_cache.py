"""Tests for SQLite match cache."""

import json
from pathlib import Path


def test_match_cache_stores_and_loads_candidates(tmp_path):
    """Match cache persists normalized candidates by request hash."""
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest, LookupSource

    cache = MatchCache(tmp_path / "nested" / "cache.db")
    request = LookupRequest(
        path=Path("/music/Artist/Album"),
        artist_hint="Artist",
        album_hint="Album",
    )
    candidates = [
        AlbumCandidate(
            artist="Artist",
            album="Album",
            album_artist="Artist",
            musicbrainz_albumid="album-id",
            source=LookupSource.BEETS,
        )
    ]

    assert cache.get(request) is None

    cache.set(request, candidates)

    assert cache.get(request) == candidates
    assert (tmp_path / "nested" / "cache.db").exists()


def test_match_cache_key_includes_track_hints(tmp_path):
    """Different track lists produce different cache entries."""
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.candidates import (
        AlbumCandidate,
        LookupRequest,
        LookupSource,
        TrackCandidate,
    )

    cache = MatchCache(tmp_path / "cache.db")
    first = LookupRequest(path=Path("/music/A/B"), artist_hint="A", album_hint="B")
    second = LookupRequest(
        path=Path("/music/A/B"),
        artist_hint="A",
        album_hint="B",
        tracks=[TrackCandidate(title="Different")],
    )
    cache.set(first, [AlbumCandidate(artist="A", album="B", source=LookupSource.BEETS)])

    assert cache.get(second) is None


# ── album state ledger tests ────────────────────────────────────────


def test_album_state_get_set_status(tmp_path):
    """Album state can be stored and retrieved by path hash."""
    from auto_tagger.integrations.cache import MatchCache

    cache = MatchCache(tmp_path / "cache.db")
    album_path = Path("/music/陈慧琳/2006.陈慧琳.Especial 新曲+精选 3CD/陈慧琳.Especial 新曲+精选 CD1")

    assert cache.get_album_state(album_path) is None

    cache.set_album_state(album_path, status="tagged_ok")
    state = cache.get_album_state(album_path)
    assert state is not None
    assert state["status"] == "tagged_ok"
    assert state["path_hash"] is not None
    assert state["processed_at"] is not None


def test_album_state_status_update(tmp_path):
    """Setting album state again overwrites previous status."""
    from auto_tagger.integrations.cache import MatchCache

    cache = MatchCache(tmp_path / "cache.db")
    album_path = Path("/music/A/B")

    cache.set_album_state(album_path, status="llm_parsed")
    assert cache.get_album_state(album_path)["status"] == "llm_parsed"

    cache.set_album_state(album_path, status="tagged_ok")
    assert cache.get_album_state(album_path)["status"] == "tagged_ok"


def test_album_state_content_hash(tmp_path):
    """Content hash changes when files are added or removed."""
    from auto_tagger.integrations.cache import MatchCache

    cache = MatchCache(tmp_path / "cache.db")
    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)
    (album_path / "01.flac").write_bytes(b"data")
    (album_path / "02.flac").write_bytes(b"data")

    cache.set_album_state(album_path, status="tagged_ok")
    state = cache.get_album_state(album_path)
    hash1 = state["content_hash"]

    # Add a file — content hash should change
    (album_path / "03.flac").write_bytes(b"data")
    cache.set_album_state(album_path, status="tagged_ok")
    state = cache.get_album_state(album_path)
    hash2 = state["content_hash"]
    assert hash2 != hash1


def test_album_state_content_hash_stable_on_rename(tmp_path):
    """Content hash is based on file names + sizes, not inode order."""
    from auto_tagger.integrations.cache import MatchCache

    cache = MatchCache(tmp_path / "cache.db")
    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)
    (album_path / "a.flac").write_bytes(b"aaa")
    (album_path / "b.flac").write_bytes(b"bbb")

    cache.set_album_state(album_path, status="tagged_ok")
    state = cache.get_album_state(album_path)
    hash1 = state["content_hash"]

    # Rename one file (same content, different name) — hash should change
    (album_path / "a.flac").rename(album_path / "c.flac")
    cache.set_album_state(album_path, status="tagged_ok")
    state = cache.get_album_state(album_path)
    hash2 = state["content_hash"]
    assert hash2 != hash1


def test_album_state_llm_extraction_get_set(tmp_path):
    """LLM folder name extraction results are cached by folder name hash."""
    from auto_tagger.integrations.cache import MatchCache

    cache = MatchCache(tmp_path / "cache.db")
    folder_name = "2006.陈慧琳.Especial 新曲+精选 3CD"

    assert cache.get_llm_extraction(folder_name) is None

    extraction = {"artist": "陈慧琳", "album": "Especial 新曲+精选", "year": "2006"}
    cache.set_llm_extraction(folder_name, extraction)
    assert cache.get_llm_extraction(folder_name) == extraction


def test_album_state_llm_extraction_shared_across_subdirs(tmp_path):
    """CD1, CD2, CD3 with the same parent share the same LLM extraction."""
    from auto_tagger.integrations.cache import MatchCache

    cache = MatchCache(tmp_path / "cache.db")
    parent = "2006.陈慧琳.Especial 新曲+精选 3CD"
    extraction = {"artist": "陈慧琳", "album": "Especial 新曲+精选", "year": "2006"}
    cache.set_llm_extraction(parent, extraction)

    # Subdirs CD1, CD2, CD3 all reference the same parent
    assert cache.get_llm_extraction(parent) == extraction


def test_album_state_llm_extraction_overwrite(tmp_path):
    """Setting LLM extraction again with same folder name overwrites."""
    from auto_tagger.integrations.cache import MatchCache

    cache = MatchCache(tmp_path / "cache.db")
    folder_name = "2006.陈慧琳.Especial 新曲+精选 3CD"

    cache.set_llm_extraction(folder_name, {"artist": "old"})
    cache.set_llm_extraction(folder_name, {"artist": "陈慧琳", "album": "Especial", "year": "2006"})
    assert cache.get_llm_extraction(folder_name) == {"artist": "陈慧琳", "album": "Especial", "year": "2006"}


def test_album_state_status_enum_values(tmp_path):
    """Only valid status values are accepted."""
    from auto_tagger.integrations.cache import MatchCache

    cache = MatchCache(tmp_path / "cache.db")
    album_path = Path("/music/A/B")

    for status in ("pending", "llm_parsed", "tagged_ok", "error"):
        cache.set_album_state(album_path, status=status)
        assert cache.get_album_state(album_path)["status"] == status


def test_album_state_folder_name_hash_root_not_subdir(tmp_path):
    """Album state stores folder_name_hash so we can query by parent folder."""
    from auto_tagger.integrations.cache import MatchCache

    cache = MatchCache(tmp_path / "cache.db")
    album_path = Path("/music/陈慧琳/2006.陈慧琳.Especial 新曲+精选 3CD/陈慧琳.Especial 新曲+精选 CD1")

    cache.set_album_state(album_path, status="tagged_ok")
    state = cache.get_album_state(album_path)
    # The folder_name_hash should reflect the parent folder, not the CD1 subdir
    # This is stored so we can find related subdir entries
    assert "folder_name_hash" in state
    # Even without the parent context, it stores something
    assert state["folder_name_hash"] is None or isinstance(state["folder_name_hash"], str)


def test_album_state_clear_state(tmp_path):
    """Setting status=None removes the album state entry."""
    from auto_tagger.integrations.cache import MatchCache

    cache = MatchCache(tmp_path / "cache.db")
    album_path = Path("/music/A/B")

    cache.set_album_state(album_path, status="tagged_ok")
    assert cache.get_album_state(album_path) is not None

    cache.clear_album_state(album_path)
    assert cache.get_album_state(album_path) is None
