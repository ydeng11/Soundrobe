"""Tests for single-album workflow orchestration."""

from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.features.cover_art import CoverArtResult, CoverArtStatus
from auto_tagger.workflows.album import AlbumWorkflow


def test_album_workflow_dry_run_collects_preview(monkeypatch, tmp_path: Path):
    """Album workflow reads metadata and returns a dry-run preview result."""
    audio = tmp_path / "01.flac"
    audio.touch()

    monkeypatch.setattr(
        "auto_tagger.workflows.album.iter_audio_files",
        lambda path, recursive=False: [audio],
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.read_metadata",
        lambda path: TrackMetadata(title="Song", artist="Artist", album="Album", track_number=1),
    )

    result = AlbumWorkflow(Settings()).run(tmp_path, dry_run=True)

    assert result.audio_files == [audio]
    assert result.dry_run is True
    assert result.applied_writes == 0
    assert result.planned_writes == 1
    assert result.metadata_by_path[audio].title == "Song"


def test_album_workflow_yolo_blocks_writes_on_health_errors(monkeypatch, tmp_path: Path):
    """YOLO mode tries to fix via lookup, but falls back when lookup fails."""
    from auto_tagger.quality.health import AlbumHealthReport, HealthIssue, HealthSeverity

    audio = tmp_path / "01.flac"
    audio.touch()

    monkeypatch.setattr(
        "auto_tagger.workflows.album.iter_audio_files",
        lambda path, recursive=False: [audio],
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.read_metadata",
        lambda path: TrackMetadata(title="Song", artist="Artist", album="Album", track_number=1),
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.build_album_health_report",
        lambda album_path, audio_files, metadata_by_path, settings: AlbumHealthReport(
            album_path=album_path,
            tracks_checked=1,
            lrc_files_checked=0,
            issues=[
                HealthIssue("audio", HealthSeverity.ERROR, audio, "audio.bad", "Bad audio")
            ],
        ),
    )
    # Mock _fix_metadata to simulate lookup failure
    monkeypatch.setattr(
        AlbumWorkflow,
        "_fix_metadata",
        lambda self, path, af, mbp, artist_mbid_map=None, artist_genre_map=None: (False, "", "No candidates"),
    )

    result = AlbumWorkflow(Settings(yolo=True)).run(tmp_path, dry_run=False)

    assert result.applied_writes == 0
    assert result.skipped_writes == 1


# ── cover art fix ──────────────────────────────────────────────


def test_cover_art_fix_embeds_local_cover(monkeypatch, tmp_path: Path):
    """YOLO mode embeds local cover art into all audio files."""
    audio_a = tmp_path / "01.flac"
    audio_b = tmp_path / "02.flac"
    audio_a.touch()
    audio_b.touch()

    # Create a local cover image
    jpeg_data = bytes([
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01,
        0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43,
        0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08, 0x07, 0x07, 0x07, 0x09,
        0x09, 0x08, 0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12,
        0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C, 0x1C, 0x20,
        0x24, 0x2E, 0x27, 0x20, 0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29,
        0x2C, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1F, 0x27, 0x39, 0x3D, 0x38, 0x32,
        0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01,
        0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00, 0x1F, 0x00, 0x00,
        0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
        0x09, 0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03,
        0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D,
        0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06,
        0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08,
        0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72,
        0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28,
        0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45,
        0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
        0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75,
        0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
        0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3,
        0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6,
        0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9,
        0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2,
        0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4,
        0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01,
        0x00, 0x00, 0x3F, 0x00, 0x7B, 0x94, 0x28, 0xA2, 0x8A, 0x28, 0xA2, 0x8A,
        0x28, 0xA2, 0x8A, 0x28, 0xA2, 0x8A, 0x28, 0xA2, 0x8A, 0x28, 0xA2, 0x8A,
        0x28, 0xA2, 0xBF, 0xFF, 0xD9,
    ])
    (tmp_path / "cover.jpg").write_bytes(jpeg_data)

    audio_files = [audio_a, audio_b]
    metadata = TrackMetadata(title="T", artist="A", album="Album",
                            album_artist="A", track_number=1)

    monkeypatch.setattr(
        "auto_tagger.workflows.album.iter_audio_files",
        lambda path, recursive=False: audio_files,
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.read_metadata",
        lambda path: metadata,
    )

    result = AlbumWorkflow(Settings(yolo=True)).run(tmp_path, dry_run=False)

    assert result.cover_art_fixed is True
    assert result.cover_art_status == CoverArtStatus.FOUND_LOCAL


def test_cover_art_fix_no_local_no_mbid(monkeypatch, tmp_path: Path):
    """Without local cover or MusicBrainz ID, cover art fix reports missing."""
    audio = tmp_path / "01.flac"
    audio.touch()

    monkeypatch.setattr(
        "auto_tagger.workflows.album.iter_audio_files",
        lambda path, recursive=False: [audio],
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.read_metadata",
        lambda path: TrackMetadata(title="T", artist="A", album="Album",
                                   album_artist="A", track_number=1),
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.write_metadata",
        lambda path, metadata, dry_run=False: metadata,
    )
    # Mock Discogs to prevent live API call
    monkeypatch.setattr(
        "auto_tagger.integrations.discogs_client.DiscogsClient.fetch_cover_art",
        lambda self, artist, album: CoverArtResult(
            CoverArtStatus.MISSING, message="No cover art"
        ),
    )

    result = AlbumWorkflow(Settings(yolo=True)).run(tmp_path, dry_run=False)

    assert result.cover_art_fixed is False
    assert result.cover_art_status == CoverArtStatus.MISSING


def test_write_candidate_handles_collaboration(monkeypatch, tmp_path: Path):
    """Collaboration single (We Are The World) gets group name as album_artist
    and individual performers in artists list, not mislabeled as compilation.
    """
    from auto_tagger.integrations.candidates import AlbumCandidate, TrackCandidate, LookupSource

    audio = tmp_path / "01.flac"
    audio.touch()

    candidate = AlbumCandidate(
        artist="U.S.A. For Africa",
        artists=["U.S.A. For Africa"],
        album="We Are The World",
        album_artist="U.S.A. For Africa",
        album_artists=["U.S.A. For Africa"],
        tracks=[
            TrackCandidate(
                title="We Are The World",
                artist="U.S.A. For Africa, Michael Jackson, Lionel Richie, Stevie Wonder",
                artists=[],
                track_number=1,
            ),
        ],
        source=LookupSource.BEETS,
    )

    existing_meta = TrackMetadata(
        title="We Are The World",
        artist="U.S.A. For Africa",
        album="We Are The World",
        album_artist="U.S.A. For Africa",
        track_number=1,
    )

    written: list[TrackMetadata] = []
    monkeypatch.setattr(
        "auto_tagger.workflows.album.write_metadata",
        lambda p, m, dry_run=False: written.append(m),
    )

    workflow = AlbumWorkflow(Settings())
    monkeypatch.setattr(workflow, "_enrich_genre_from_discogs", lambda c: None)
    monkeypatch.setattr(workflow, "_enrich_genre_from_llm", lambda a, b, known_genres=None: None)

    fixed, source_label, message = workflow._write_candidate_metadata(
        audio_files=[audio],
        metadata_by_path={audio: existing_meta},
        candidate=candidate,
    )

    assert fixed is True
    assert len(written) == 1
    meta = written[0]
    # Group name as album_artist, not Various Artists
    assert meta.album_artist == "U.S.A. For Africa"
    assert meta.compilation is False
    # Individual performers populated in artists (plural) list
    assert "Michael Jackson" in meta.artists
    assert "Lionel Richie" in meta.artists
    assert "Stevie Wonder" in meta.artists


def test_write_candidate_handles_classical(monkeypatch, tmp_path: Path):
    """Classical album with same primary performer across tracks
    is not mislabeled as Various Artists.
    """
    from auto_tagger.integrations.candidates import AlbumCandidate, TrackCandidate, LookupSource

    audio = tmp_path / "01.flac"
    audio.touch()

    candidate = AlbumCandidate(
        artist="Anne‑Sophie Mutter",
        artists=["Anne‑Sophie Mutter"],
        album="East Meets West",
        album_artist="Anne‑Sophie Mutter",
        album_artists=["Anne‑Sophie Mutter"],
        tracks=[
            TrackCandidate(title="Likoo", artist="Anne‑Sophie Mutter", artists=[], track_number=1),
            TrackCandidate(title="Studie II", artist="Anne‑Sophie Mutter, Yo-Yo Ma", artists=[], track_number=3),
            TrackCandidate(title="Air-Homage", artist="Anne‑Sophie Mutter, LSO", artists=[], track_number=14),
        ],
        source=LookupSource.BEETS,
    )

    existing_meta = TrackMetadata(
        title="Likoo", artist="Anne‑Sophie Mutter",
        album="East Meets West", album_artist="Anne‑Sophie Mutter",
        track_number=1,
    )

    written: list[TrackMetadata] = []
    monkeypatch.setattr(
        "auto_tagger.workflows.album.write_metadata",
        lambda p, m, dry_run=False: written.append(m),
    )

    workflow = AlbumWorkflow(Settings())
    monkeypatch.setattr(workflow, "_enrich_genre_from_discogs", lambda c: None)
    monkeypatch.setattr(workflow, "_enrich_genre_from_llm", lambda a, b, known_genres=None: None)

    fixed, source_label, message = workflow._write_candidate_metadata(
        audio_files=[audio],
        metadata_by_path={audio: existing_meta},
        candidate=candidate,
    )

    assert fixed is True
    assert len(written) == 1
    meta = written[0]
    # Primary performer as album_artist, NOT Various Artists
    assert meta.album_artist == "Anne‑Sophie Mutter"
    assert meta.compilation is False


def test_write_candidate_handles_compilation(monkeypatch, tmp_path: Path):
    """True compilation (different artists per track) still gets Various Artists."""
    from auto_tagger.integrations.candidates import AlbumCandidate, TrackCandidate, LookupSource

    audio = tmp_path / "01.flac"
    audio.touch()

    candidate = AlbumCandidate(
        artist="Various Artists",
        artists=["Various Artists"],
        album="Now That's What I Call Music",
        album_artist="Various Artists",
        album_artists=["Various Artists"],
        tracks=[
            TrackCandidate(title="A", artist="Artist One", artists=["Artist One"], track_number=1),
            TrackCandidate(title="B", artist="Artist Two", artists=["Artist Two"], track_number=2),
            TrackCandidate(title="C", artist="Artist Three", artists=["Artist Three"], track_number=3),
        ],
        source=LookupSource.BEETS,
    )

    existing_meta = TrackMetadata(
        title="A", artist="Artist One",
        album="Now That's What I Call Music",
        track_number=1,
    )

    written: list[TrackMetadata] = []
    monkeypatch.setattr(
        "auto_tagger.workflows.album.write_metadata",
        lambda p, m, dry_run=False: written.append(m),
    )

    workflow = AlbumWorkflow(Settings())
    monkeypatch.setattr(workflow, "_enrich_genre_from_discogs", lambda c: None)
    monkeypatch.setattr(workflow, "_enrich_genre_from_llm", lambda a, b, known_genres=None: None)

    fixed, source_label, message = workflow._write_candidate_metadata(
        audio_files=[audio],
        metadata_by_path={audio: existing_meta},
        candidate=candidate,
    )

    assert fixed is True
    assert len(written) == 1
    meta = written[0]
    assert meta.album_artist == "Various Artists"
    assert meta.compilation is True
    # Per-track artist preserved
    assert meta.artist == "Artist One"


def test_write_candidate_handles_ampersand_duo(monkeypatch, tmp_path: Path):
    """A & B duo album keeps combined artist, populates ARTISTS, not a compilation."""
    from auto_tagger.integrations.candidates import AlbumCandidate, TrackCandidate, LookupSource

    audio = tmp_path / "01.flac"
    audio.touch()

    candidate = AlbumCandidate(
        artist="Beyoncé & Jay-Z",
        artists=["Beyoncé", "Jay-Z"],
        album="The Album",
        album_artist="Beyoncé & Jay-Z",
        album_artists=["Beyoncé & Jay-Z"],
        tracks=[
            TrackCandidate(
                title="Crazy in Love",
                artist="Beyoncé & Jay-Z",
                artists=[],
                track_number=1,
            ),
        ],
        source=LookupSource.BEETS,
    )

    existing_meta = TrackMetadata(
        title="Crazy in Love",
        artist="Beyoncé & Jay-Z",
        album="The Album",
        album_artist="Beyoncé & Jay-Z",
        track_number=1,
    )

    written: list[TrackMetadata] = []
    monkeypatch.setattr(
        "auto_tagger.workflows.album.write_metadata",
        lambda p, m, dry_run=False: written.append(m),
    )

    workflow = AlbumWorkflow(Settings())
    monkeypatch.setattr(workflow, "_enrich_genre_from_discogs", lambda c: None)
    monkeypatch.setattr(workflow, "_enrich_genre_from_llm", lambda a, b, known_genres=None: None)

    fixed, source_label, message = workflow._write_candidate_metadata(
        audio_files=[audio],
        metadata_by_path={audio: existing_meta},
        candidate=candidate,
    )

    assert fixed is True
    assert len(written) == 1
    meta = written[0]
    # Combined duo name preserved as artist
    assert meta.artist == "Beyoncé & Jay-Z"
    # Individual artists in ARTISTS list
    assert "Beyoncé" in meta.artists
    assert "Jay-Z" in meta.artists
    # Not a compilation
    assert meta.compilation is False
    assert meta.album_artist == "Beyoncé & Jay-Z"


def test_write_candidate_handles_single_artist(monkeypatch, tmp_path: Path):
    """Normal single-artist album is unchanged — not a compilation."""
    from auto_tagger.integrations.candidates import AlbumCandidate, TrackCandidate, LookupSource

    audio = tmp_path / "01.flac"
    audio.touch()

    candidate = AlbumCandidate(
        artist="Tanya Chua",
        artists=["Tanya Chua"],
        album="Goodbye & Hello",
        album_artist="Tanya Chua",
        album_artists=["Tanya Chua"],
        tracks=[
            TrackCandidate(title="Darwin", artist="Tanya Chua", artists=["Tanya Chua"], track_number=1),
            TrackCandidate(title="Goodbye & Hello", artist="Tanya Chua", artists=["Tanya Chua"], track_number=2),
        ],
        source=LookupSource.BEETS,
    )

    existing_meta = TrackMetadata(
        title="Darwin", artist="Tanya Chua",
        album="Goodbye & Hello", album_artist="Tanya Chua",
        track_number=1,
    )

    written: list[TrackMetadata] = []
    monkeypatch.setattr(
        "auto_tagger.workflows.album.write_metadata",
        lambda p, m, dry_run=False: written.append(m),
    )

    workflow = AlbumWorkflow(Settings())
    monkeypatch.setattr(workflow, "_enrich_genre_from_discogs", lambda c: None)
    monkeypatch.setattr(workflow, "_enrich_genre_from_llm", lambda a, b, known_genres=None: None)

    fixed, source_label, message = workflow._write_candidate_metadata(
        audio_files=[audio],
        metadata_by_path={audio: existing_meta},
        candidate=candidate,
    )

    assert fixed is True
    assert len(written) == 1
    meta = written[0]
    assert meta.album_artist == "Tanya Chua"
    assert meta.compilation is False
    assert meta.artist == "Tanya Chua"


def test_cover_art_fix_skipped_in_dry_run(monkeypatch, tmp_path: Path):
    """Dry-run mode does not attempt cover art fix."""
    audio = tmp_path / "01.flac"
    audio.touch()
    (tmp_path / "cover.jpg").write_bytes(b"\xff\xd8\xff")  # partial JPEG header only

    monkeypatch.setattr(
        "auto_tagger.workflows.album.iter_audio_files",
        lambda path, recursive=False: [audio],
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.read_metadata",
        lambda path: TrackMetadata(title="T", artist="A", album="Album",
                                   album_artist="A", track_number=1),
    )

    result = AlbumWorkflow(Settings(yolo=True)).run(tmp_path, dry_run=True)

    assert result.cover_art_fixed is False
    assert result.cover_art_status == ""
