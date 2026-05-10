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

    assert service.lookup_album(album_path) == [cached]


def test_lookup_service_caches_beets_candidates(tmp_path):
    """Beets candidates are cached after a cache miss."""
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
    service = LookupService(beets_client=client, cache=MatchCache(tmp_path / "cache.db"))

    assert service.lookup_album(album_path) == [candidate]
    assert service.lookup_album(album_path) == [candidate]
    assert client.calls == 1


def test_lookup_service_uses_dataset_before_beets(tmp_path):
    """Dataset candidates are preferred over remote Beets/API lookup."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupSource
    from auto_tagger.integrations.lookup import LookupService

    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)
    dataset_candidate = AlbumCandidate(
        artist="Artist",
        album="Album",
        source=LookupSource.DATASET,
    )

    class DatasetClient:
        def __init__(self):
            self.calls = 0

        def lookup_album(self, request):
            self.calls += 1
            return [dataset_candidate]

    class BeetsClient:
        def __init__(self):
            self.calls = 0

        def lookup_album(self, request):
            self.calls += 1
            return [AlbumCandidate(artist="Artist", album="Remote", source=LookupSource.BEETS)]

    dataset_client = DatasetClient()
    beets_client = BeetsClient()
    service = LookupService(
        beets_client=beets_client,
        dataset_client=dataset_client,
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path),
    )

    assert service.lookup_album(album_path) == [dataset_candidate]
    assert dataset_client.calls == 1
    assert beets_client.calls == 0


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
        settings=Settings(data_dir=tmp_path),
    )

    assert service.lookup_album(album_path) == [remote_candidate]
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
        settings=Settings(data_dir=tmp_path, remote_lookup_enabled=False),
    )

    candidates = service.lookup_album(album_path)

    assert len(candidates) == 1
    assert candidates[0].source is LookupSource.FOLDER


def test_lookup_service_falls_back_to_folder_candidate(tmp_path):
    """No Beets matches produces a folder fallback candidate."""
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.candidates import LookupSource
    from auto_tagger.integrations.lookup import LookupService

    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)
    (album_path / "01 Song.flac").write_bytes(b"")

    class Client:
        def lookup_album(self, request):
            return []

    service = LookupService(beets_client=Client(), cache=MatchCache(tmp_path / "cache.db"))

    candidates = service.lookup_album(album_path)

    assert len(candidates) == 1
    assert candidates[0].source is LookupSource.FOLDER
    assert candidates[0].album == "Album"
