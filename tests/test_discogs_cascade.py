"""Tests for Discogs integration in the lookup cascade."""


from auto_tagger.integrations.candidates import AlbumCandidate, LookupSource
from auto_tagger.integrations.discogs_client import DiscogsError


def test_discogs_used_when_beets_returns_empty(monkeypatch, tmp_path):
    """Discogs kicks in after a cache miss and empty beets result."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.lookup import LookupService

    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)
    (album_path / "01.flac").write_bytes(b"")

    class EmptyBeetsClient:
        def lookup_album(self, request):
            return []

    discogs_candidate = AlbumCandidate(
        artist="Artist", album="Album", source=LookupSource.DISCOGS
    )

    def fake_search(self, artist, album):
        return [discogs_candidate]

    # Patch DiscogsClient.search_album
    import auto_tagger.integrations.discogs_client as dc
    monkeypatch.setattr(dc.DiscogsClient, "search_album", fake_search)

    service = LookupService(
        beets_client=EmptyBeetsClient(),
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path),
    )

    candidates = service.lookup_album(album_path)
    assert len(candidates) == 1
    assert candidates[0].source is LookupSource.DISCOGS
    assert candidates[0].artist == "Artist"
    assert candidates[0].album == "Album"
    assert candidates[0].verification == "match"


def test_discogs_disabled_skips_discogs(monkeypatch, tmp_path):
    """When discogs_enabled=False, Discogs is not called."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.lookup import LookupService

    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)
    (album_path / "01.flac").write_bytes(b"")

    class EmptyBeetsClient:
        def lookup_album(self, request):
            return []

    class DiscogsClientSpy:
        def __init__(self):
            self.called = False

        def search_album(self, artist, album):  # type: ignore[override]
            self.called = True
            return []

    spy = DiscogsClientSpy()

    import auto_tagger.integrations.discogs_client as dc
    monkeypatch.setattr(dc, "DiscogsClient", lambda **kw: spy)

    service = LookupService(
        beets_client=EmptyBeetsClient(),
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path, discogs_enabled=False),
    )

    candidates = service.lookup_album(album_path)
    assert not spy.called
    assert len(candidates) == 1
    assert candidates[0].source is LookupSource.FOLDER


def test_discogs_error_produces_warning_and_falls_back(monkeypatch, tmp_path):
    """A Discogs error is recorded as a warning and falls back to folder."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.lookup import LookupService

    album_path = tmp_path / "Artist" / "Album"
    album_path.mkdir(parents=True)
    (album_path / "01.flac").write_bytes(b"")

    class EmptyBeetsClient:
        def lookup_album(self, request):
            return []

    def failing_search(self, artist, album):
        raise DiscogsError("Rate limit exceeded")

    import auto_tagger.integrations.discogs_client as dc
    monkeypatch.setattr(dc.DiscogsClient, "search_album", failing_search)

    service = LookupService(
        beets_client=EmptyBeetsClient(),
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path),
    )

    candidates = service.lookup_album(album_path)
    # Falls back to folder
    assert len(candidates) == 1
    assert candidates[0].source is LookupSource.FOLDER
    # Warning recorded
    assert len(service.warnings) == 2  # dataset not found + discogs error
    assert any("Discogs" in w for w in service.warnings)


def test_5566_lookup_cascade_with_discogs(monkeypatch, tmp_path):
    """A 5566 album lookup that misses dataset+beets finds it on Discogs."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.lookup import LookupService

    album_path = tmp_path / "5566" / "挚爱"
    album_path.mkdir(parents=True)
    (album_path / "01.flac").write_bytes(b"")

    class EmptyBeetsClient:
        def lookup_album(self, request):
            return []

    def discogs_search(self, artist, album):
        assert artist == "5566"
        assert album == "挚爱"
        return [
            AlbumCandidate(
                artist="5566",
                album="挚爱",
                year="2004",
                source=LookupSource.DISCOGS,
            )
        ]

    import auto_tagger.integrations.discogs_client as dc
    monkeypatch.setattr(dc.DiscogsClient, "search_album", discogs_search)

    service = LookupService(
        beets_client=EmptyBeetsClient(),
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path),
    )

    candidates = service.lookup_album(album_path)
    assert len(candidates) == 1
    assert candidates[0].artist == "5566"
    assert candidates[0].album == "挚爱"
    assert candidates[0].source is LookupSource.DISCOGS
    assert candidates[0].verification == "match"


def test_5566_digit_album_on_discogs(monkeypatch, tmp_path):
    """A digit album name like '2001' by Dr. Dre is found on Discogs."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.cache import MatchCache
    from auto_tagger.integrations.lookup import LookupService

    album_path = tmp_path / "Dr. Dre" / "2001"
    album_path.mkdir(parents=True)
    (album_path / "01.flac").write_bytes(b"")

    class EmptyBeetsClient:
        def lookup_album(self, request):
            return []

    def discogs_search(self, artist, album):
        return [
            AlbumCandidate(
                artist="Dr. Dre",
                album="2001",
                year="1999",
                source=LookupSource.DISCOGS,
            )
        ]

    import auto_tagger.integrations.discogs_client as dc
    monkeypatch.setattr(dc.DiscogsClient, "search_album", discogs_search)

    service = LookupService(
        beets_client=EmptyBeetsClient(),
        cache=MatchCache(tmp_path / "cache.db"),
        settings=Settings(data_dir=tmp_path),
    )

    candidates = service.lookup_album(album_path)
    assert len(candidates) == 1
    assert candidates[0].artist == "Dr. Dre"
    assert candidates[0].album == "2001"
    assert candidates[0].source is LookupSource.DISCOGS
    assert candidates[0].verification == "match"
