"""Health report integration tests — covers every issue type found in real libraries.

Issues covered (mirrors 蔡健雅 library findings):
  ERROR:   missing_album_artist, inconsistent_album
  WARNING: lrc.non_utf8, lrc.malformed_tag, cover_art.missing_local,
           metadata.track_sequence_gap

Fix-path tests verify issues disappear after remediation.
"""

import shutil
from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.core.audio import iter_audio_files
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.core.reader import read_metadata
from auto_tagger.quality.health import AlbumHealthReport, build_album_health_report

# ── missing album_artist ───────────────────────────────────────

def test_missing_album_artist_is_error(album_fixture: Path):
    """Every track without album_artist triggers a metadata error."""
    settings = Settings()
    audio_files = list(iter_audio_files(album_fixture))

    # Strip album_artist from metadata to simulate untagged files
    metadata_by_path = {}
    for af in audio_files:
        meta = read_metadata(af)
        # Clear album_artist
        meta = TrackMetadata(
            title=meta.title,
            artist=meta.artist,
            album=meta.album,
            track_number=meta.track_number,
            year=meta.year,
        )
        metadata_by_path[af] = meta

    report = build_album_health_report(album_fixture, audio_files, metadata_by_path, settings)
    assert report.has_blocking_errors
    missing = [i for i in report.issues if i.code == "metadata.missing_album_artist"]
    assert len(missing) == len(audio_files)


def test_fix_album_artist_clears_error(album_fixture: Path, tmp_path: Path):
    """Setting album_artist removes the missing_album_artist error."""
    # Copy one track to temp
    flacs = sorted(album_fixture.rglob("*.flac"))
    src = flacs[0]
    dest = tmp_path / src.name
    shutil.copy2(src, dest)

    settings = Settings()

    # Health report WITHOUT album_artist
    meta_no_aa = read_metadata(dest)
    meta_no_aa = TrackMetadata(
        title=meta_no_aa.title,
        artist=meta_no_aa.artist,
        album=meta_no_aa.album,
        track_number=meta_no_aa.track_number,
        year=meta_no_aa.year,
    )
    report_before = build_album_health_report(
        tmp_path, [dest], {dest: meta_no_aa}, settings
    )
    assert any(i.code == "metadata.missing_album_artist" for i in report_before.issues)

    # Fix: set album_artist
    meta_fixed = TrackMetadata(
        title=meta_no_aa.title,
        artist=meta_no_aa.artist,
        album=meta_no_aa.album,
        album_artist=meta_no_aa.artist,  # same as artist for solo album
        track_number=meta_no_aa.track_number,
        year=meta_no_aa.year,
    )
    report_after = build_album_health_report(
        tmp_path, [dest], {dest: meta_fixed}, settings
    )
    assert not report_after.has_blocking_errors
    assert not any(i.code == "metadata.missing_album_artist" for i in report_after.issues)


# ── lrc.non_utf8 ───────────────────────────────────────────────

def test_lrc_non_utf8_is_warning(tmp_path: Path):
    """GB18030-encoded LRC files trigger a non-UTF8 warning."""
    album = tmp_path / "Artist" / "Album"
    album.mkdir(parents=True)

    # Create a dummy FLAC file
    flac = album / "01.flac"
    flac.write_bytes(b"")

    # Create an LRC file with GB18030 encoding (simulates Chinese-sourced lyrics)
    gb18030_content = "[ti:测试]\n[ar:歌手]\n[00:01.00]歌词内容\n".encode("gb18030")
    lrc = album / "01.lrc"
    lrc.write_bytes(gb18030_content)

    settings = Settings()
    from auto_tagger.core.reader import read_metadata
    try:
        meta = read_metadata(flac)
    except Exception:
        meta = TrackMetadata(title="test")

    report = build_album_health_report(
        album, [flac], {flac: meta}, settings
    )
    non_utf8 = [i for i in report.issues if i.code == "lrc.non_utf8"]
    assert len(non_utf8) == 1
    assert "gb18030" in non_utf8[0].message.lower()


def test_lrc_utf8_is_clean(tmp_path: Path):
    """UTF-8 LRC files do not trigger encoding warnings."""
    album = tmp_path / "Artist" / "Album"
    album.mkdir(parents=True)

    flac = album / "01.flac"
    flac.write_bytes(b"")
    lrc = album / "01.lrc"
    lrc.write_text("[ti:Test]\n[ar:Artist]\n[00:01.00]Lyrics\n", encoding="utf-8")

    settings = Settings()
    from auto_tagger.core.reader import read_metadata
    try:
        meta = read_metadata(flac)
    except Exception:
        meta = TrackMetadata(title="test")

    report = build_album_health_report(album, [flac], {flac: meta}, settings)
    assert not any(i.code == "lrc.non_utf8" for i in report.issues)


# ── lrc.malformed_tag ─────────────────────────────────────────

def test_lrc_malformed_tag_is_warning(tmp_path: Path):
    """Lines with unrecognized [brackets] trigger malformed_tag warnings."""
    album = tmp_path / "Artist" / "Album"
    album.mkdir(parents=True)

    flac = album / "01.flac"
    flac.write_bytes(b"")
    # Line 1 has a malformed bracket: [ver:v1.0] — ver is not a known metadata tag
    lrc_content = "[ver:v1.0]\n[00:01.00]Valid timing\n[unknown:stuff]\n"
    lrc = album / "01.lrc"
    lrc.write_text(lrc_content, encoding="utf-8")

    settings = Settings()
    from auto_tagger.core.reader import read_metadata
    try:
        meta = read_metadata(flac)
    except Exception:
        meta = TrackMetadata(title="test")

    report = build_album_health_report(album, [flac], {flac: meta}, settings)
    malformed = [i for i in report.issues if i.code == "lrc.malformed_tag"]
    # [ver:v1.0] and [unknown:stuff] are both unrecognized
    assert len(malformed) == 2


# ── cover_art.missing_local ────────────────────────────────────

def test_missing_cover_art_is_warning(edge_case_fixtures: Path):
    """Albums without cover art get a cover_art.missing_local warning."""
    album = edge_case_fixtures / "missing_cover" / "Artist" / "No Cover"
    flacs = sorted(album.rglob("*.flac"))

    settings = Settings()
    metadata_by_path = {f: read_metadata(f) for f in flacs}
    report = build_album_health_report(album, flacs, metadata_by_path, settings)

    missing = [i for i in report.issues if i.code == "missing_local" and i.category == "cover_art"]
    assert len(missing) == 1


def test_present_cover_art_is_clean(album_fixture: Path):
    """Albums with cover.jpg do not get cover art warnings."""
    flacs = sorted(album_fixture.rglob("*.flac"))

    settings = Settings()
    metadata_by_path = {f: read_metadata(f) for f in flacs}
    report = build_album_health_report(album_fixture, flacs, metadata_by_path, settings)

    cover_issues = [i for i in report.issues if i.category == "cover_art"]
    assert len(cover_issues) == 0


# ── inconsistent album metadata ────────────────────────────────

def test_inconsistent_album_is_error(tmp_path: Path):
    """Tracks with different album names trigger inconsistent_album error."""
    album = tmp_path / "Artist" / "Album"
    album.mkdir(parents=True)

    flac1 = album / "01.flac"
    flac2 = album / "02.flac"
    flac1.write_bytes(b"")
    flac2.write_bytes(b"")

    settings = Settings()
    meta1 = TrackMetadata(
        title="Track 1", artist="Artist", album="Correct Name",
        album_artist="Artist", track_number=1,
    )
    meta2 = TrackMetadata(
        title="Track 2", artist="Artist", album="Wrong Name",  # different!
        album_artist="Artist", track_number=2,
    )
    metadata_by_path = {flac1: meta1, flac2: meta2}
    report = build_album_health_report(album, [flac1, flac2], metadata_by_path, settings)

    inconsistent = [i for i in report.issues if i.code == "metadata.inconsistent_album"]
    assert len(inconsistent) >= 1
    assert report.has_blocking_errors


def test_consistent_album_is_clean(album_fixture: Path):
    """All tracks with same album name do not trigger inconsistent_album."""
    flacs = sorted(album_fixture.rglob("*.flac"))

    settings = Settings()
    metadata_by_path = {f: read_metadata(f) for f in flacs}
    report = build_album_health_report(album_fixture, flacs, metadata_by_path, settings)

    assert not any(i.code == "metadata.inconsistent_album" for i in report.issues)


# ── track sequence gaps ────────────────────────────────────────

def test_track_sequence_gap_is_warning(tmp_path: Path):
    """Non-consecutive track numbers trigger a sequence gap warning."""
    album = tmp_path / "Artist" / "Album"
    album.mkdir(parents=True)

    flac1 = album / "01.flac"
    flac3 = album / "03.flac"  # missing track 2
    flac1.write_bytes(b"")
    flac3.write_bytes(b"")

    settings = Settings()
    meta1 = TrackMetadata(
        title="T1", artist="A", album="Album", album_artist="A", track_number=1,
    )
    meta3 = TrackMetadata(
        title="T3", artist="A", album="Album", album_artist="A", track_number=3,
    )
    audio_files = [flac1, flac3]
    metadata_by_path = {flac1: meta1, flac3: meta3}
    report = build_album_health_report(album, audio_files, metadata_by_path, settings)

    gaps = [i for i in report.issues if i.code == "metadata.track_sequence_gap"]
    assert len(gaps) >= 1


def test_consecutive_tracks_are_clean(album_fixture: Path):
    """Consecutive track numbers do not trigger sequence gap warnings."""
    flacs = sorted(album_fixture.rglob("*.flac"))

    settings = Settings()
    metadata_by_path = {f: read_metadata(f) for f in flacs}
    report = build_album_health_report(album_fixture, flacs, metadata_by_path, settings)

    assert not any(i.code == "metadata.track_sequence_gap" for i in report.issues)


# ── full health report structure ───────────────────────────────

def test_health_report_structure(album_fixture: Path):
    """AlbumHealthReport has correct structure with summary and track_health."""
    flacs = sorted(album_fixture.rglob("*.flac"))

    settings = Settings()
    metadata_by_path = {f: read_metadata(f) for f in flacs}
    report = build_album_health_report(album_fixture, flacs, metadata_by_path, settings)

    assert isinstance(report, AlbumHealthReport)
    assert report.tracks_checked == len(flacs)
    assert report.lrc_files_checked == len(flacs)  # 1 LRC per FLAC
    assert isinstance(report.summary, dict)
    assert "errors" in report.summary
    assert "warnings" in report.summary
    assert "info" in report.summary
    assert len(report.track_health) == len(flacs)

    # to_dict produces valid JSON-serializable structure
    d = report.to_dict()
    assert d["album_path"] == str(album_fixture)
    assert d["tracks_checked"] == len(flacs)
    assert "summary" in d
    assert "issues" in d
    assert "track_health" in d


# ── batch health report aggregation ────────────────────────────

def test_batch_summary_collects_health_reports(fixtures_dir: Path):
    """BatchWorkflow collects per-album health reports in BatchSummary."""
    from auto_tagger.config import Settings
    from auto_tagger.workflows.batch import BatchWorkflow

    settings = Settings(data_dir=fixtures_dir)
    summary = BatchWorkflow(settings).run(fixtures_dir, dry_run=True)

    assert summary.processed >= 2  # album + compilation + formats (which are skipped)
    assert len(summary.health_reports) >= 2
    for report_dict in summary.health_reports:
        assert "album_path" in report_dict
        assert "summary" in report_dict
        assert "issues" in report_dict
        assert isinstance(report_dict["summary"]["errors"], int)
        assert isinstance(report_dict["summary"]["warnings"], int)


# ── fix path: cover art ────────────────────────────────────────

def test_fix_missing_cover_art(edge_case_fixtures: Path, tmp_path: Path):
    """Adding a cover image clears the cover_art.missing_local warning."""
    album = edge_case_fixtures / "missing_cover" / "Artist" / "No Cover"
    flacs = sorted(album.rglob("*.flac"))

    settings = Settings()
    metadata_by_path = {f: read_metadata(f) for f in flacs}

    # Before: missing cover
    report_before = build_album_health_report(album, flacs, metadata_by_path, settings)
    assert any(i.category == "cover_art" and i.code == "missing_local"
               for i in report_before.issues)

    # Fix: add a cover image to the album dir
    # Use the same minimal JPEG pattern as the fixture factory
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
    (album / "cover.jpg").write_bytes(jpeg_data)

    # After: no cover art warning
    report_after = build_album_health_report(album, flacs, metadata_by_path, settings)
    assert not any(i.category == "cover_art" and i.code == "missing_local"
                   for i in report_after.issues)
