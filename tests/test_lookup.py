"""Tests for lookup orchestration."""



def test_lookup_service_uses_cache_before_beets(tmp_path):
    """Cached lookup candidates are returned without calling beets."""
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupSource
    from auto_tagger.integrations.lookup import LookupService

    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)
    cached = AlbumCandidate(artist="Artist", album="Album", source=LookupSource.BEETS)
    cache = MatchCache(tmp_path / "cache.db")

    service = LookupService(beets_client=None, cache=cache)
    request = service.request_from_path(album_path)
    cache.set(request, [cached])

    result = service.lookup_album(album_path)
    assert len(result) == 1
    assert result[0].artist == cached.artist
    assert result[0].album == cached.album
    assert result[0].source == cached.source


def test_lookup_service_caches_beets_candidates(tmp_path):
    """Beets candidates are cached after a cache miss."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupSource
    from auto_tagger.integrations.lookup import LookupService

    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)
    candidate = AlbumCandidate(artist="Artist", album="Album", source=LookupSource.BEETS)

    class Client:
        def __init__(self):
            self.calls = 0

        def lookup_album(self, request):
            self.calls += 1
            return [candidate]

    client = Client()
    service = LookupService(
        beets_client=client,
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path, discogs_enabled=False),
    )

    r1 = service.lookup_album(album_path)
    assert len(r1) == 1
    assert r1[0].artist == "Artist"
    assert r1[0].album == "Album"
    assert r1[0].verification == "match"

    r2 = service.lookup_album(album_path)
    assert len(r2) == 1
    assert r2[0].artist == "Artist"
    assert r2[0].album == "Album"
    assert r2[0].verification == "match"
    assert client.calls == 1


def test_lookup_service_falls_through_dataset_when_no_index(tmp_path):
    """When no dataset index exists, lookup falls through without error."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.lookup import LookupService

    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)

    service = LookupService(
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path, discogs_enabled=False),
    )

    candidates = service.lookup_album(album_path)

    # Falls through dataset (no index) -> beets (text search) or folder
    assert len(candidates) >= 1


def test_lookup_service_records_missing_dataset_warning_then_uses_beets(tmp_path):
    """Missing local dataset index records a warning and still falls back to Beets."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupSource
    from auto_tagger.integrations.lookup import LookupService

    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)
    remote_candidate = AlbumCandidate(artist="Artist", album="Remote", source=LookupSource.BEETS)

    class BeetsClient:
        def lookup_album(self, request):
            return [remote_candidate]

    service = LookupService(
        beets_client=BeetsClient(),
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path, discogs_enabled=False),
    )

    result = service.lookup_album(album_path)
    # Mismatched beets result + folder fallback
    assert len(result) == 2
    assert result[0].artist == "Artist"
    assert result[0].album == "Remote"
    assert result[0].verification == "mismatch"
    assert result[1].source is LookupSource.FOLDER
    assert result[1].verification == "match"
    assert len(service.warnings) == 1
    assert "Local dataset index not found" in service.warnings[0]


def test_lookup_service_can_disable_remote_lookup(tmp_path):
    """Remote lookup can be disabled after cache and dataset miss."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.candidates import LookupSource
    from auto_tagger.integrations.lookup import LookupService

    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)
    (album_path / "01 Song.flac").write_bytes(b"")

    class BeetsClient:
        def lookup_album(self, request):
            raise AssertionError("remote lookup should not run")

    service = LookupService(
        beets_client=BeetsClient(),
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path, remote_lookup_enabled=False, discogs_enabled=False),
    )

    candidates = service.lookup_album(album_path)

    assert len(candidates) == 1
    assert candidates[0].source is LookupSource.FOLDER


def test_lookup_service_falls_back_to_folder_candidate(tmp_path):
    """No Beets matches produces a folder fallback candidate."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.candidates import LookupSource
    from auto_tagger.integrations.lookup import LookupService

    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)
    (album_path / "01 Song.flac").write_bytes(b"")

    class Client:
        def lookup_album(self, request):
            return []

    service = LookupService(
        beets_client=Client(),
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path, discogs_enabled=False),
    )

    candidates = service.lookup_album(album_path)

    assert len(candidates) == 1
    assert candidates[0].source is LookupSource.FOLDER
    assert candidates[0].album == "Album"


# ── LLM hint enhancement tests ─────────────────────────────────────────


def test_lookup_deterministic_parse_ambiguous_dot_convention(tmp_path):
    """Folder names with Year.Artist.Album convention are detected as ambiguous."""
    from auto_tagger.integrations.lookup import LookupService
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.config import Settings

    album_path = tmp_path / "陈慧琳" / "2006.陈慧琳.Especial 新曲+精选 3CD" / "陈慧琳.Especial 新曲+精选 CD1"
    album_path.mkdir(parents=True)
    (album_path / "01 歌.flac").write_bytes(b"data")

    service = LookupService(
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path, discogs_enabled=False, llm_api_key=None),
    )
    request = service.request_from_path(album_path)

    # The deterministic parse should fail to extract clean hints
    assert request.year_hint is None or request.album_hint == "陈慧琳.Especial 新曲+精选 CD1"
    ambiguous = service._hints_are_ambiguous(request)
    assert ambiguous is True


def test_lookup_deterministic_parse_clean_hints(tmp_path):
    """Standard Artist/Year-Album structure is not ambiguous."""
    from auto_tagger.integrations.lookup import LookupService
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.config import Settings

    album_path = tmp_path / "Artist" / "2006-Album"
    album_path.mkdir(parents=True)
    (album_path / "01 Song.flac").write_bytes(b"data")

    service = LookupService(
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path, discogs_enabled=False, llm_api_key=None),
    )
    request = service.request_from_path(album_path)
    ambiguous = service._hints_are_ambiguous(request)
    assert ambiguous is False


def test_lookup_enhancement_caches_and_reuses_llm_result(tmp_path):
    """After LLM extraction, the same folder name reuses cached result."""
    from auto_tagger.integrations.lookup import LookupService
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.config import Settings

    album_path = tmp_path / "陈慧琳" / "2006.陈慧琳.Especial 新曲+精选 3CD" / "陈慧琳.Especial 新曲+精选 CD1"
    album_path.mkdir(parents=True)
    (album_path / "01 歌.flac").write_bytes(b"data")

    extracted = {"artist": "陈慧琳", "album": "Especial 新曲+精选", "year": "2006", "disc": "1"}

    service = LookupService(
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path, discogs_enabled=False, llm_api_key=None),
    )
    request = service.request_from_path(album_path)

    # Store the extraction manually (simulating LLM call)
    folder_name = "2006.陈慧琳.Especial 新曲+精选 3CD"
    service.cache.set_llm_extraction(folder_name, extracted)

    enhanced = service._enhance_hints_with_llm(request)
    assert enhanced is not None
    assert enhanced.artist_hint == "陈慧琳"
    assert enhanced.album_hint == "Especial 新曲+精选"
    assert enhanced.year_hint == "2006"


def test_lookup_enhancement_returns_none_when_no_llm_key(tmp_path):
    """Without an LLM API key, enhancement returns None gracefully."""
    from auto_tagger.integrations.lookup import LookupService
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.config import Settings

    album_path = tmp_path / "A" / "Album"
    album_path.mkdir(parents=True)

    service = LookupService(
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path, llm_api_key=None),
    )
    request = service.request_from_path(album_path)
    enhanced = service._enhance_hints_with_llm(request)
    assert enhanced is None


def test_lookup_enhancement_not_called_when_hints_are_clean(tmp_path):
    """Enhancement is skipped entirely when deterministic hints are clean."""
    from auto_tagger.integrations.lookup import LookupService
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.config import Settings

    album_path = tmp_path / "Artist" / "2006 - Album"
    album_path.mkdir(parents=True)
    (album_path / "01 Song.flac").write_bytes(b"data")

    service = LookupService(
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path, discogs_enabled=False, llm_api_key=None),
    )
    request = service.request_from_path(album_path)
    assert not service._hints_are_ambiguous(request)


def test_lookup_enhancement_preserves_per_track_artist(tmp_path):
    """Enhanced hints don't lose per-track artist info from filenames."""
    from auto_tagger.integrations.lookup import LookupService
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.config import Settings

    album_path = tmp_path / "陈慧琳" / "2006.陈慧琳.陈小春.拉阔演奏厅"
    album_path.mkdir(parents=True)
    (album_path / "陈慧琳 - 01.毫无保留.flac").write_bytes(b"data")
    (album_path / "陈小春 - 02.斗苦.flac").write_bytes(b"data")

    service = LookupService(
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path, discogs_enabled=False, llm_api_key=None),
    )
    request = service.request_from_path(album_path)
    
    folder_name = "2006.陈慧琳.陈小春.拉阔演奏厅"
    extracted = {"artist": "陈慧琳,陈小春", "album": "拉阔演奏厅", "year": "2006"}
    service.cache.set_llm_extraction(folder_name, extracted)

    enhanced = service._enhance_hints_with_llm(request)
    assert enhanced is not None
    assert enhanced.artist_hint == "陈慧琳,陈小春"
    assert enhanced.album_hint == "拉阔演奏厅"
    # Per-track hints from filenames should be preserved
    assert len(enhanced.tracks) >= 2
