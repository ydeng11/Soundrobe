"""Tests for metadata readers."""

from pathlib import Path
from types import SimpleNamespace


def test_read_flac_like_tags(monkeypatch):
    """Vorbis-style tags read into normalized metadata."""
    from auto_tagger.core.audio import AudioFormat
    from auto_tagger.core.reader import read_metadata

    tags = {
        "TITLE": ["Track"],
        "ARTIST": ["Alice feat. Bob"],
        "ARTISTS": ["Alice", "Bob"],
        "ALBUM": ["Album"],
        "ALBUMARTIST": ["Alice"],
        "TRACKNUMBER": ["2"],
        "TOTALTRACKS": ["10"],
        "DISCNUMBER": ["1/2"],
        "DATE": ["2024"],
        "GENRE": ["Pop"],
        "MUSICBRAINZ_TRACKID": ["track-id"],
        "REPLAYGAIN_TRACK_GAIN": ["-6.84 dB"],
    }
    audio = SimpleNamespace(path=Path("track.flac"), format=AudioFormat.FLAC, mutagen_file=tags)
    monkeypatch.setattr("auto_tagger.core.reader.load_audio_file", lambda path: audio)

    metadata = read_metadata(Path("track.flac"))

    assert metadata.title == "Track"
    assert metadata.artists == ["Alice", "Bob"]
    assert metadata.track_number == 2
    assert metadata.track_total == 10
    assert metadata.disc_number == 1
    assert metadata.disc_total == 2
    assert metadata.musicbrainz_trackid == "track-id"
    assert metadata.replaygain.track_gain == "-6.84 dB"


def test_read_album_metadata_reads_all_discovered_files(monkeypatch):
    """Album reads preserve deterministic path ordering from discovery."""
    from auto_tagger.core.metadata import TrackMetadata
    from auto_tagger.core.reader import read_album_metadata

    paths = [Path("01.mp3"), Path("02.flac")]
    monkeypatch.setattr(
        "auto_tagger.core.reader.iter_audio_files",
        lambda path, recursive=False: paths,
    )
    monkeypatch.setattr(
        "auto_tagger.core.reader.read_metadata",
        lambda path: TrackMetadata(title=path.stem),
    )

    result = read_album_metadata(Path("album"))

    assert list(result) == paths
    assert [metadata.title for metadata in result.values()] == ["01", "02"]
