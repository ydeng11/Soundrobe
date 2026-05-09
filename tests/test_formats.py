"""Tests for format-specific tag mapping."""

from mutagen.id3 import ID3, TALB, TIT2, TPE1, TPE2, TRCK, TXXX
from mutagen.mp4 import MP4FreeForm, MP4Tags

from auto_tagger.core.audio import AudioFormat
from auto_tagger.core.formats import read_tags, write_tags
from auto_tagger.core.metadata import ReplayGainTags, TrackMetadata


def test_mp3_tags_round_trip_normalized_metadata():
    """MP3 ID3 tags read and write normalized fields."""
    tags = ID3()
    tags.add(TIT2(encoding=3, text=["Song"]))
    tags.add(TPE1(encoding=3, text=["Alice feat. Bob"]))
    tags.add(TPE2(encoding=3, text=["Alice"]))
    tags.add(TALB(encoding=3, text=["Album"]))
    tags.add(TRCK(encoding=3, text=["2/10"]))
    tags.add(TXXX(encoding=3, desc="ARTISTS", text=["Alice", "Bob"]))
    tags.add(TXXX(encoding=3, desc="REPLAYGAIN_TRACK_GAIN", text=["-6.84 dB"]))

    metadata = read_tags(AudioFormat.MP3, tags)

    assert metadata.title == "Song"
    assert metadata.artists == ["Alice", "Bob"]
    assert metadata.track_number == 2
    assert metadata.track_total == 10
    assert metadata.replaygain.track_gain == "-6.84 dB"

    output = ID3()
    write_tags(AudioFormat.MP3, output, metadata)

    assert output.get("TIT2").text == ["Song"]
    assert output.get("TXXX:ARTISTS").text == ["Alice", "Bob"]
    assert output.get("TRCK").text == ["2/10"]


def test_mp4_tags_round_trip_normalized_metadata():
    """MP4 atoms and freeform fields read and write normalized fields."""
    tags = MP4Tags()
    tags["©nam"] = ["Song"]
    tags["©art"] = ["Alice feat. Bob"]
    tags["aART"] = ["Alice"]
    tags["©alb"] = ["Album"]
    tags["trkn"] = [(2, 10)]
    tags["----:com.apple.iTunes:ARTISTS"] = [MP4FreeForm(b"Alice"), MP4FreeForm(b"Bob")]
    tags["----:com.apple.iTunes:REPLAYGAIN_TRACK_GAIN"] = [MP4FreeForm(b"-6.84 dB")]

    metadata = read_tags(AudioFormat.M4A, tags)

    assert metadata.title == "Song"
    assert metadata.artists == ["Alice", "Bob"]
    assert metadata.track_number == 2
    assert metadata.track_total == 10
    assert metadata.replaygain.track_gain == "-6.84 dB"

    output = MP4Tags()
    write_tags(
        AudioFormat.M4A,
        output,
        TrackMetadata(
            title="Song",
            artist="Alice feat. Bob",
            artists=["Alice", "Bob"],
            track_number=2,
            track_total=10,
            replaygain=ReplayGainTags(track_gain="-6.84 dB"),
        ),
    )

    assert output["©nam"] == ["Song"]
    assert output["trkn"] == [(2, 10)]
    assert [bytes(value).decode("utf-8") for value in output["----:com.apple.iTunes:ARTISTS"]] == [
        "Alice",
        "Bob",
    ]
