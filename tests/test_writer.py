"""Tests for metadata writers."""

from pathlib import Path
from types import SimpleNamespace


class FakeTags(dict):
    """Small mutable tag object that records save calls."""

    def __init__(self):
        super().__init__()
        self.save_calls = 0

    def save(self):
        self.save_calls += 1


def test_write_flac_like_tags(monkeypatch):
    """Writing normalized metadata stores Vorbis-style fields."""
    from auto_tagger.core.audio import AudioFormat
    from auto_tagger.core.metadata import ReplayGainTags, TrackMetadata
    from auto_tagger.core.writer import write_metadata

    tags = FakeTags()
    audio = SimpleNamespace(path=Path("track.flac"), format=AudioFormat.FLAC, mutagen_file=tags)
    monkeypatch.setattr("auto_tagger.core.writer.load_audio_file", lambda path: audio)

    write_metadata(
        Path("track.flac"),
        TrackMetadata(
            title="Track",
            artist="Alice feat. Bob",
            artists=["Alice", "Bob"],
            album="Album",
            album_artist="Alice",
            track_number=2,
            track_total=10,
            replaygain=ReplayGainTags(track_gain="-6.84 dB"),
        ),
    )

    assert tags["TITLE"] == ["Track"]
    assert tags["ARTISTS"] == ["Alice", "Bob"]
    assert tags["TRACKNUMBER"] == ["2"]
    assert tags["TOTALTRACKS"] == ["10"]
    assert tags["REPLAYGAIN_TRACK_GAIN"] == ["-6.84 dB"]
    assert tags.save_calls == 1


def test_write_dry_run_does_not_mutate_or_save(monkeypatch):
    """Dry-run writer returns normalized metadata without changing files."""
    from auto_tagger.core.audio import AudioFormat
    from auto_tagger.core.metadata import TrackMetadata
    from auto_tagger.core.writer import write_metadata

    tags = FakeTags()
    audio = SimpleNamespace(path=Path("track.flac"), format=AudioFormat.FLAC, mutagen_file=tags)
    monkeypatch.setattr("auto_tagger.core.writer.load_audio_file", lambda path: audio)

    result = write_metadata(Path("track.flac"), TrackMetadata(title="Track"), dry_run=True)

    assert result.title == "Track"
    assert tags == {}
    assert tags.save_calls == 0
