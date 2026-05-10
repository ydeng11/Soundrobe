"""Tests for compilation album detection and tagging."""

from auto_tagger.core.audio import AudioFormat
from auto_tagger.core.formats import read_tags, write_tags
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.features.compilations import analyze_compilation, apply_compilation_tags


def test_analyze_compilation_detects_various_track_artists():
    """Albums with varied track artists are detected as compilations."""
    tracks = [
        TrackMetadata(title="A", artist="Artist One", album="Mix", track_number=1),
        TrackMetadata(title="B", artist="Artist Two", album="Mix", track_number=2),
        TrackMetadata(title="C", artist="Artist Three", album="Mix", track_number=3),
    ]

    analysis = analyze_compilation(tracks, album_path_hint="Various Artists/Mix")

    assert analysis.is_compilation is True
    assert analysis.confidence >= 0.8
    assert any("various" in reason.lower() for reason in analysis.reasons)


def test_apply_compilation_tags_sets_album_artist_and_flag():
    """Compilation transform preserves track artists and sets Navidrome tags."""
    tracks = [
        TrackMetadata(title="A", artist="Artist One", album="Mix", track_number=1),
        TrackMetadata(title="B", artist="Artist Two", album="Mix", track_number=2),
    ]

    updated = apply_compilation_tags(tracks)

    assert all(track.album_artist == "Various Artists" for track in updated)
    assert all(track.compilation is True for track in updated)
    assert updated[0].artist == "Artist One"


def test_compilation_tags_round_trip_vorbis():
    """Vorbis-style tags read and write compilation flag."""
    tags = {"TITLE": ["Song"], "ARTIST": ["A"], "COMPILATION": ["1"]}

    metadata = read_tags(AudioFormat.FLAC, tags)
    output: dict[str, list[str]] = {}
    write_tags(AudioFormat.FLAC, output, metadata)

    assert metadata.compilation is True
    assert output["COMPILATION"] == ["1"]
