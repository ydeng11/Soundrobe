"""Tests for single-album workflow orchestration."""

from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.features.cover_art import CoverArtResult, CoverArtStatus
from auto_tagger.workflows.album import AlbumWorkflow, _clean_stem, _stem_track_number


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


# (merged from pi-wt branch)
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
# ── duplicate track number fix ──────────────────────────────────


def test_fix_duplicate_track_numbers_renumbers_from_filenames(monkeypatch, tmp_path: Path):
    """Duplicate track numbers are fixed by renumbering from filename prefixes."""
    from auto_tagger.quality.health import AlbumHealthReport, HealthIssue, HealthSeverity

    # Mock write_metadata to avoid touching real audio files
    write_calls: list[tuple[Path, TrackMetadata]] = []
    monkeypatch.setattr(
        "auto_tagger.workflows.album.write_metadata",
        lambda path, metadata, dry_run=False: write_calls.append((path, metadata)),
    )
    # Mock build_album_health_report for the rebuild to return a clean report
    monkeypatch.setattr(
        "auto_tagger.workflows.album.build_album_health_report",
        lambda album_path, audio_files, metadata_by_path, settings: AlbumHealthReport(
            album_path=album_path,
            tracks_checked=len(audio_files),
            lrc_files_checked=0,
        ),
    )

    # Create audio files with unique filename prefixes
    audio_a = tmp_path / "01 SongA.flac"
    audio_b = tmp_path / "02 SongB.flac"
    audio_c = tmp_path / "03 SongC.flac"
    audio_a.touch()
    audio_b.touch()
    audio_c.touch()
    audio_files = [audio_a, audio_b, audio_c]

    # Both SongA and SongB have track_number=1 (the duplicate)
    metadata_by_path = {
        audio_a: TrackMetadata(title="SongA", artist="A", album="Album", track_number=1),
        audio_b: TrackMetadata(title="SongB", artist="A", album="Album", track_number=1),
        audio_c: TrackMetadata(title="SongC", artist="A", album="Album", track_number=3),
    }

    # Health report with duplicate track number error
    health_report = AlbumHealthReport(
        album_path=tmp_path,
        tracks_checked=3,
        lrc_files_checked=0,
        issues=[
            HealthIssue(
                "metadata", HealthSeverity.ERROR, None,
                "metadata.duplicate_track_number",
                "Duplicate track number 1 on disc 1",
                {"paths": [str(audio_a), str(audio_b)]},
            ),
            HealthIssue(
                "metadata", HealthSeverity.WARNING, None,
                "metadata.track_sequence_gap",
                "Track sequence has gaps on disc 1",
            ),
        ],
    )

    workflow = AlbumWorkflow(Settings())
    fixed, updated_meta, updated_health = workflow._fix_duplicate_track_numbers(
        audio_files, metadata_by_path, health_report, dry_run=False,
    )

    assert fixed is True
    # SongA should still be track 1 (filename says 01)
    assert updated_meta[audio_a].track_number == 1
    # SongB should now be track 2 (filename says 02)
    assert updated_meta[audio_b].track_number == 2
    # SongC should still be track 3 (filename says 03)
    assert updated_meta[audio_c].track_number == 3
    # Health report should no longer have duplicate error
    assert updated_health.can_tag is True
    # write_metadata was called for the two fixed files
    assert len(write_calls) == 2


def test_fix_duplicate_track_numbers_skipped_when_no_duplicates(monkeypatch, tmp_path: Path):
    """No-op when health report has no duplicate track number issues."""
    from auto_tagger.quality.health import AlbumHealthReport

    monkeypatch.setattr(
        "auto_tagger.workflows.album.write_metadata",
        lambda path, metadata, dry_run=False: None,
    )

    audio = tmp_path / "01.flac"
    audio.touch()
    metadata = {audio: TrackMetadata(title="Song", artist="A", album="Album", track_number=1)}
    health_report = AlbumHealthReport(
        album_path=tmp_path, tracks_checked=1, lrc_files_checked=0,
    )

    workflow = AlbumWorkflow(Settings())
    fixed, meta, hr = workflow._fix_duplicate_track_numbers(
        [audio], metadata, health_report, dry_run=False,
    )

    assert fixed is False
    assert meta is metadata
    assert hr is health_report


def test_fix_duplicate_track_numbers_renumbers_sequentially_when_filenames_lack_prefix(monkeypatch, tmp_path: Path):
    """Sequential fallback fixes tracks when all files share same track number
    and filenames have no leading prefix."""
    from auto_tagger.quality.health import AlbumHealthReport, HealthIssue, HealthSeverity

    monkeypatch.setattr(
        "auto_tagger.workflows.album.write_metadata",
        lambda path, metadata, dry_run=False: None,
    )

    audio_a = tmp_path / "SongA.flac"
    audio_b = tmp_path / "SongB.flac"
    audio_a.touch()
    audio_b.touch()

    metadata = {
        audio_a: TrackMetadata(title="SongA", artist="A", album="Album", track_number=1),
        audio_b: TrackMetadata(title="SongB", artist="A", album="Album", track_number=1),
    }
    health_report = AlbumHealthReport(
        album_path=tmp_path,
        tracks_checked=2,
        lrc_files_checked=0,
        issues=[
            HealthIssue(
                "metadata", HealthSeverity.ERROR, None,
                "metadata.duplicate_track_number",
                "Duplicate track number 1 on disc 1",
                {"paths": [str(audio_a), str(audio_b)]},
            ),
        ],
    )

    workflow = AlbumWorkflow(Settings())
    fixed, updated_meta, _ = workflow._fix_duplicate_track_numbers(
        [audio_a, audio_b], metadata, health_report, dry_run=False,
    )

    assert fixed is True
    # Sequential: SongA → track 1, SongB → track 2
    assert updated_meta[audio_a].track_number == 1
    assert updated_meta[audio_b].track_number == 2


def test_fix_duplicate_track_numbers_skipped_when_stem_numbers_not_unique(monkeypatch, tmp_path: Path):
    """Strategy 1 is skipped when stem numbers aren't unique, but Strategy 2
    sequential fallback handles it when all files share the same track number."""
    from auto_tagger.quality.health import AlbumHealthReport, HealthIssue, HealthSeverity

    monkeypatch.setattr(
        "auto_tagger.workflows.album.write_metadata",
        lambda path, metadata, dry_run=False: None,
    )

    audio_a = tmp_path / "01 Intro.flac"
    audio_b = tmp_path / "01 Song.flac"  # same prefix
    audio_a.touch()
    audio_b.touch()

    metadata = {
        audio_a: TrackMetadata(title="Intro", artist="A", album="Album", track_number=1),
        audio_b: TrackMetadata(title="Song", artist="A", album="Album", track_number=1),
    }
    health_report = AlbumHealthReport(
        album_path=tmp_path,
        tracks_checked=2,
        lrc_files_checked=0,
        issues=[
            HealthIssue(
                "metadata", HealthSeverity.ERROR, None,
                "metadata.duplicate_track_number",
                "Duplicate track number 1 on disc 1",
                {"paths": [str(audio_a), str(audio_b)]},
            ),
        ],
    )

    workflow = AlbumWorkflow(Settings())
    fixed, updated_meta, _ = workflow._fix_duplicate_track_numbers(
        [audio_a, audio_b], metadata, health_report, dry_run=False,
    )

    assert fixed is True
    # Sequential fallback: 01 Intro → track 1, 01 Song → track 2
    assert updated_meta[audio_a].track_number == 1
    assert updated_meta[audio_b].track_number == 2


def test_fix_duplicate_track_numbers_end_to_end_via_workflow_yolo(monkeypatch, tmp_path: Path):
    """YOLO workflow fixes duplicate track numbers before the lookup cascade."""
    from auto_tagger.quality.health import AlbumHealthReport, HealthIssue, HealthSeverity

    audio_a = tmp_path / "01 SongA.flac"
    audio_b = tmp_path / "02 SongB.flac"
    audio_c = tmp_path / "03 SongC.flac"
    audio_a.touch()
    audio_b.touch()
    audio_c.touch()
    audio_files = [audio_a, audio_b, audio_c]

    # Two tracks share track_number=1
    def _mock_read(path: Path) -> TrackMetadata:
        mapping = {
            audio_a: TrackMetadata(title="SongA", artist="Artist", album="Album", track_number=1),
            audio_b: TrackMetadata(title="SongB", artist="Artist", album="Album", track_number=1),
            audio_c: TrackMetadata(title="SongC", artist="Artist", album="Album", track_number=3),
        }
        return mapping[path]

    # Track whether we're in the initial call or rebuild call
    health_call_count: list[int] = [0]

    def _mock_health(album_path, af, mbp, settings):
        health_call_count[0] += 1
        if health_call_count[0] == 2:
            # Second call (rebuild after fix) returns clean report
            return AlbumHealthReport(
                album_path=album_path,
                tracks_checked=len(af),
                lrc_files_checked=0,
            )
        # First call returns report with duplicates
        return AlbumHealthReport(
            album_path=album_path,
            tracks_checked=3,
            lrc_files_checked=0,
            issues=[
                HealthIssue(
                    "metadata", HealthSeverity.ERROR, None,
                    "metadata.duplicate_track_number",
                    "Duplicate track number 1 on disc 1",
                    {"paths": [str(audio_a), str(audio_b)]},
                ),
                HealthIssue(
                    "metadata", HealthSeverity.WARNING, None,
                    "metadata.track_sequence_gap",
                    "Track sequence has gaps on disc 1",
                ),
            ],
        )

    monkeypatch.setattr(
        "auto_tagger.workflows.album.iter_audio_files",
        lambda path, recursive=False: audio_files,
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.read_metadata",
        _mock_read,
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.build_album_health_report",
        _mock_health,
    )
    # Prevent actual disk writes from interfering
    monkeypatch.setattr(
        "auto_tagger.workflows.album.write_metadata",
        lambda path, metadata, dry_run=False: None,
    )
    # Prevent _fix_metadata lookup from running (it would fail without real data)
    monkeypatch.setattr(
        AlbumWorkflow,
        "_fix_metadata",
        lambda self, path, af, mbp, artist_mbid_map=None, artist_genre_map=None: (False, "", "No candidates"),
    )

    result = AlbumWorkflow(Settings(yolo=True)).run(tmp_path, dry_run=False)

    # The duplicate fix should have been applied before can_write
    assert "Renumbered tracks (filename prefixes or sequential)" in result.messages
    # Health report should allow tagging
    assert result.health_report.can_tag is True


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


# ── _stem_track_number with (NN) parenthesized prefix ───────────


def test_stem_track_number_handles_parenthesized_prefix():
    """_stem_track_number extracts leading digits from parenthesized prefixes.

    Regression test: the regex ``r"^(\\d{1,2})"`` did not match filenames
    starting with ``(`` such as ``(01) Song.flac``, causing Strategy 1 to
    silently skip all parenthesized-prefix files.
    """
    # (NN) format — newly supported
    assert _stem_track_number("(01) Song") == 1
    assert _stem_track_number("(12) [Artist] Title") == 12
    assert _stem_track_number("(99) Song") == 99

    # Bare NN prefix — existing behavior, must still work
    assert _stem_track_number("01 Song") == 1
    assert _stem_track_number("01.Song") == 1
    assert _stem_track_number("01-Song") == 1
    assert _stem_track_number("01_Song") == 1
    assert _stem_track_number("01Song") == 1

    # No prefix
    assert _stem_track_number("Song") is None
    assert _stem_track_number("Artist - Song") is None

    # Three-digit prefix matches the first two digits (\d{1,2})
    assert _stem_track_number("100 Songs") == 10

    # Edge: empty stem
    assert _stem_track_number("") is None


# ── _clean_stem with (NN) parenthesized prefix ─────────────────


def test_clean_stem_cleans_parenthesized_prefix():
    """_clean_stem strips parenthesized track-number prefixes.

    Regression test: the regex only matched bare ``NN`` at start, missing
    ``(NN)`` prefixed filenames and leaving the raw prefix in the title.
    """
    # (NN) format — newly supported
    assert _clean_stem("(01) Song") == "Song"
    assert _clean_stem("(01) [Artist] Title") == "[Artist] Title"
    assert _clean_stem("(12) Artist - Title") == "Artist - Title"

    # Bare NN prefix — existing behavior, must still work
    assert _clean_stem("01 Song") == "Song"
    assert _clean_stem("01.Song") == "Song"
    assert _clean_stem("01-Song") == "Song"
    assert _clean_stem("01_Song") == "Song"
    assert _clean_stem("01Song") == "Song"

    # No prefix — returned unchanged
    assert _clean_stem("Song Title") == "Song Title"
    assert _clean_stem("Artist - Song") == "Artist - Song"

    # Edge: empty stem
    assert _clean_stem("") == ""


# ── Pattern 3 exclusion (duplicate + disc number heuristic) ────


def _mock_health_with_duplicates_and_meta(
    album_path: Path,
    audio_files: list[Path],
    metadata_by_path: dict[Path, TrackMetadata],
    settings: Settings,
):
    """Build a health report with duplicate track number issues based on
    the actual metadata, so Pattern 3's disc-number check uses real values.
    """
    from auto_tagger.quality.health import AlbumHealthReport, HealthIssue, HealthSeverity
    from collections import defaultdict

    paths_by_pos: dict[tuple[int, int], list[str]] = defaultdict(list)
    for f in audio_files:
        meta = metadata_by_path.get(f)
        if meta is not None and meta.track_number is not None:
            disc = meta.disc_number or 1
            paths_by_pos[(disc, meta.track_number)].append(str(f))

    issues: list[HealthIssue] = []
    for (disc, track), paths in paths_by_pos.items():
        if len(paths) > 1:
            issues.append(
                HealthIssue(
                    "metadata", HealthSeverity.ERROR, None,
                    "metadata.duplicate_track_number",
                    f"Duplicate track number {track} on disc {disc}",
                    {"paths": paths},
                )
            )

    return AlbumHealthReport(
        album_path=album_path,
        tracks_checked=len(audio_files),
        lrc_files_checked=0,
        issues=issues,
    )


def test_pattern_3_does_not_exclude_disc_none_on_single_disc(
    monkeypatch,
    tmp_path: Path,
):
    """Pattern 3 must NOT exclude disc=None files when the album has no
    files with disc > 1. On single-disc albums, missing disc_number is
    incomplete metadata, not a stray from another disc.

    Regression test: the original heuristic unconditionally excluded
    disc=None files in any duplicate group, deleting legitimate tracks
    from albums like 陈洁仪-2018-A Time For Everything.
    """
    from auto_tagger.quality.health import AlbumHealthReport

    # 4 files, 2 share track=1 (one disc=1, one disc=None), others disc=1
    audio_a = tmp_path / "SongA.flac"
    audio_b = tmp_path / "SongB.flac"
    audio_c = tmp_path / "SongC.flac"
    audio_d = tmp_path / "SongD.flac"
    for f in (audio_a, audio_b, audio_c, audio_d):
        f.touch()
    all_files = [audio_a, audio_b, audio_c, audio_d]

    metadata = {
        audio_a: TrackMetadata(title="A", artist="X", album="Album",
                               album_artist="X", track_number=1, disc_number=1),
        audio_b: TrackMetadata(title="B", artist="X", album="Album",
                               album_artist="X", track_number=1, disc_number=None),
        audio_c: TrackMetadata(title="C", artist="X", album="Album",
                               album_artist="X", track_number=3, disc_number=1),
        audio_d: TrackMetadata(title="D", artist="X", album="Album",
                               album_artist="X", track_number=4, disc_number=None),
    }

    monkeypatch.setattr(
        "auto_tagger.workflows.album.iter_audio_files",
        lambda path, recursive=False: all_files,
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.read_metadata",
        lambda path: metadata[path],
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.build_album_health_report",
        _mock_health_with_duplicates_and_meta,
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.write_metadata",
        lambda path, md, dry_run=False: None,
    )

    # Stub _fix_metadata so the lookup cascade doesn't run
    monkeypatch.setattr(
        AlbumWorkflow,
        "_fix_metadata",
        lambda self, path, af, mbp, artist_mbid_map=None, artist_genre_map=None: (
            False, "", "No candidates"
        ),
    )

    # Stub _fix_cover_art so it doesn't run
    monkeypatch.setattr(
        AlbumWorkflow,
        "_fix_cover_art",
        lambda self, path, af, mbp: (False, "", ""),
    )

    result = AlbumWorkflow(Settings(yolo=True)).run(tmp_path, dry_run=False)

    # All 4 files should still be present — Pattern 3 must not exclude
    assert len(result.audio_files) == 4, (
        f"Expected 4 files, got {len(result.audio_files)} — "
        "Pattern 3 wrongly excluded disc=None files on single-disc album"
    )
    # No "Deleted" message should appear
    assert not any("Deleted" in m for m in result.messages), (
        f"Unexpected deletion messages: {result.messages}"
    )


def test_pattern_3_excludes_disc_none_on_multi_disc(monkeypatch, tmp_path: Path):
    """Pattern 3 SHOULD exclude disc=None files when the album has files
    with disc > 1. The disc=None file may be a stray from another disc.

    This preserves the existing heuristic for legitimate multi-disc scenarios.
    """
    # 4 files on a multi-disc album (has disc > 1 elsewhere):
    #   - audio_a: disc 1, track 1 — legitimate track
    #   - audio_b: disc None, track 1 — stray that leaked into disc 1's group
    #   - audio_c: disc 1, track 3 — another legitimate track
    #   - audio_d: disc 2, track 1 — proves this is a multi-disc album
    #
    # audio_a and audio_b both normalize to (disc=1, track=1) so they
    # appear in the same duplicate group. Since album_has_multi_disc=True
    # (audio_d has disc=2), Pattern 3 should exclude the disc=None stray.
    audio_a = tmp_path / "SongA.flac"
    audio_b = tmp_path / "Stray.flac"
    audio_c = tmp_path / "SongC.flac"
    audio_d = tmp_path / "2-01 SongD.flac"
    for f in (audio_a, audio_b, audio_c, audio_d):
        f.touch()
    all_files = [audio_a, audio_b, audio_c, audio_d]

    metadata = {
        audio_a: TrackMetadata(title="SongA", artist="X", album="Album",
                               album_artist="X", track_number=1, disc_number=1),
        audio_b: TrackMetadata(title="Stray", artist="X", album="Album",
                               album_artist="X", track_number=1, disc_number=None),
        audio_c: TrackMetadata(title="SongC", artist="X", album="Album",
                               album_artist="X", track_number=3, disc_number=1),
        audio_d: TrackMetadata(title="SongD", artist="X", album="Album",
                               album_artist="X", track_number=1, disc_number=2),
    }

    monkeypatch.setattr(
        "auto_tagger.workflows.album.iter_audio_files",
        lambda path, recursive=False: all_files,
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.read_metadata",
        lambda path: metadata[path],
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.build_album_health_report",
        _mock_health_with_duplicates_and_meta,
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.write_metadata",
        lambda path, md, dry_run=False: None,
    )
    monkeypatch.setattr(
        AlbumWorkflow,
        "_fix_metadata",
        lambda self, path, af, mbp, artist_mbid_map=None, artist_genre_map=None: (
            False, "", "No candidates"
        ),
    )
    monkeypatch.setattr(
        AlbumWorkflow,
        "_fix_cover_art",
        lambda self, path, af, mbp: (False, "", ""),
    )

    result = AlbumWorkflow(Settings(yolo=True)).run(tmp_path, dry_run=False)

    # The disc=None file should have been excluded + deleted
    # Pattern 3 should see disc=2 exists and mark the disc=None file as stray
    messages_str = " ".join(result.messages)
    assert "Deleted" in messages_str, (
        f"Expected Pattern 3 to exclude stray on multi-disc, messages: {result.messages}"
    )
    assert audio_b not in result.audio_files, (
        "Stray disc=None file should have been excluded on multi-disc album"
    )
