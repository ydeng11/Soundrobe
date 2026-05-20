"""Tests for format-specific tag mapping."""

import struct
from pathlib import Path

import pytest
from mutagen.id3 import ID3, TALB, TIT2, TPE1, TPE2, TRCK, TXXX
from mutagen.mp4 import MP4FreeForm, MP4Tags

from auto_tagger.core.audio import AudioFormat
from auto_tagger.core.formats import read_tags, strip_wav_list_chunks, write_tags
from auto_tagger.core.metadata import ReplayGainTags, TrackMetadata
from auto_tagger.exceptions import TaggingError


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


# ── WAV LIST-chunk helpers ────────────────────────────────────────────────


def _make_wav_bytes(
    fmt_extra: bytes = b"",
    data_payload: bytes = b"\x00" * 64,
    list_payload: bytes | None = b"INFO" + struct.pack("<4sI", b"IART", 4) + b"Foo\x00",
    id3_payload: bytes | None = None,
    trailing: bytes = b"",
) -> bytearray:
    """Build a minimal valid WAV binary with optional LIST and id3 chunks.

    The resulting bytearray has a valid RIFF/WAVE header, ``fmt `` chunk,
    ``data`` chunk, plus the optional extras.  Every chunk's declared size
    matches the actual data so the file is structurally sound.
    """
    # fmt chunk: PCM 16-bit mono at 44100 Hz
    fmt_data = struct.pack("<HHIIHH", 1, 1, 44100, 88200, 2, 16) + fmt_extra
    fmt_chunk = struct.pack("<4sI", b"fmt ", len(fmt_data)) + fmt_data

    # data chunk
    data_chunk = struct.pack("<4sI", b"data", len(data_payload)) + data_payload

    chunks = bytearray()
    chunks.extend(fmt_chunk)
    chunks.extend(data_chunk)

    if list_payload is not None:
        list_chunk = struct.pack("<4sI", b"LIST", len(list_payload)) + list_payload
        if len(list_payload) % 2:
            list_chunk += b"\x00"  # RIFF padding
        chunks.extend(list_chunk)

    if id3_payload is not None:
        id3_chunk = struct.pack("<4sI", b"id3 ", len(id3_payload)) + id3_payload
        if len(id3_payload) % 2:
            id3_chunk += b"\x00"
        chunks.extend(id3_chunk)

    riff_size = len(chunks)
    header = struct.pack("<4sI4s", b"RIFF", riff_size, b"WAVE")
    result = bytearray(header)
    result.extend(chunks)
    result.extend(trailing)
    return result


def _make_wav_at(path: Path, **kwargs) -> Path:
    """Write a synthetic WAV from _make_wav_bytes to *path* and return it."""
    path.write_bytes(_make_wav_bytes(**kwargs))
    return path


# ── strip_wav_list_chunks tests ───────────────────────────────────────────


class TestStripWavListChunks:
    """Tests for strip_wav_list_chunks()."""

    def test_strips_list_chunk(self, tmp_path: Path) -> None:
        """LIST chunk is removed from the file."""
        # Build a WAV with LIST, id3, and a short IART inside LIST.
        list_payload = (
            b"INFO"
            + struct.pack("<4sI", b"IART", 4)
            + b"Foo\x00"
            + struct.pack("<4sI", b"IPRD", 6)
            + b"Album\x00"
        )
        wav = _make_wav_at(tmp_path / "test.wav", list_payload=list_payload)

        orig_size = wav.stat().st_size
        stripped = strip_wav_list_chunks(wav)

        assert stripped is True
        modified = wav.read_bytes()
        assert b"LIST" not in modified
        assert modified[:4] == b"RIFF"
        assert modified[8:12] == b"WAVE"
        assert wav.stat().st_size < orig_size  # smaller after removal

    def test_no_list_returns_false(self, tmp_path: Path) -> None:
        """Returns False when there is no LIST chunk to remove."""
        wav = _make_wav_at(tmp_path / "test.wav", list_payload=None)
        assert strip_wav_list_chunks(wav) is False
        # File should remain valid
        data = wav.read_bytes()
        assert data[0:4] == b"RIFF"
        assert data[8:12] == b"WAVE"

    def test_preserves_id3_chunk(self, tmp_path: Path) -> None:
        """After stripping, the id3 chunk is still present and readable by mutagen."""
        # Build a WAV with both LIST and a valid id3 payload positioned AFTER
        # the LIST chunk (not overlapping).  We construct the id3 chunk data
        # ourselves so it is properly formed.
        id3_payload = (
            b"ID3\x04\x00\x00\x00\x00\x00\x16"  # ID3v2.4 header, synchsafe-size=22
            b"TPE1\x00\x00\x00\x0c\x00\x00"  # frame header (size=12)
            b"\x03Test Artist"  # encoding byte + value
        )
        wav = _make_wav_at(
            tmp_path / "test.wav",
            list_payload=b"INFO" + struct.pack("<4sI", b"IART", 4) + b"Old\x00",
            id3_payload=id3_payload,
        )

        # Sanity check: both chunks present before stripping
        data = wav.read_bytes()
        assert b"LIST" in data
        assert b"id3 " in data

        strip_wav_list_chunks(wav)

        # id3 chunk still present
        data = wav.read_bytes()
        assert b"LIST" not in data
        idx = data.find(b"id3 ")
        assert idx != -1, "id3 chunk missing from cleaned file"
        declared_size = struct.unpack("<I", data[idx + 4 : idx + 8])[0]
        assert declared_size > 0

        # RIFF header still valid
        assert data[:4] == b"RIFF"
        assert data[8:12] == b"WAVE"

        # Mutagen can still parse the file
        from mutagen import File

        mf = File(str(wav))
        assert mf is not None
        assert mf.tags is not None
        assert str(mf.tags.get("TPE1")) == "Test Artist"

    def test_trailing_garbage_truncated(self, tmp_path: Path) -> None:
        """Trailing bytes after the last valid chunk are stripped."""
        wav = _make_wav_at(
            tmp_path / "test.wav",
            list_payload=None,
            trailing=b"GARBAGE" * 100,
        )
        orig = wav.stat().st_size

        stripped = strip_wav_list_chunks(wav)
        # No LIST to strip, but garbage is also truncated
        # (the chunk-finder sees "GARB" with an implausible size)
        assert stripped is False  # no LIST removed
        assert wav.stat().st_size < orig  # garbage truncated

    def test_invalid_file_raises_error(self, tmp_path: Path) -> None:
        """Non-WAV files raise TaggingError."""
        f = tmp_path / "not_a_wav.bin"
        f.write_bytes(b"NOTRIFF\x00\x00\x00")
        with pytest.raises(TaggingError, match="Not a valid WAV file"):
            strip_wav_list_chunks(f)

    def test_empty_file_raises_error(self, tmp_path: Path) -> None:
        """Empty or tiny files raise TaggingError."""
        f = tmp_path / "empty.wav"
        f.write_bytes(b"")
        with pytest.raises(TaggingError, match="Not a valid WAV file"):
            strip_wav_list_chunks(f)

    def test_write_tags_strips_list_before_save(self, tmp_path: Path) -> None:
        """write_tags() strips LIST chunks before the caller's save().

        When the full write_metadata() path is used on a WAV file that has
        LIST/INFO metadata, the resulting file should have no LIST chunks
        after save().
        """
        from mutagen import File
        from auto_tagger.core.audio import AudioFormat
        from auto_tagger.core.metadata import TrackMetadata

        # Build a WAV with a LIST chunk
        list_payload = (
            b"INFO"
            + struct.pack("<4sI", b"IART", 5)
            + b"Old\x00x"
            + struct.pack("<4sI", b"IPRD", 6)
            + b"Album\x00"
        )
        wav = _make_wav_at(
            tmp_path / "test.wav",
            list_payload=list_payload,
            id3_payload=None,
        )
        assert b"LIST" in wav.read_bytes()

        # Simulate write_metadata():  load via mutagen, write tags, save.
        # write_tags() should strip LIST before returning.
        mf = File(str(wav))
        assert mf is not None

        metadata = TrackMetadata(
            title="Test Song",
            artist="Test Artist",
            album="Test Album",
            track_number=1,
            track_total=10,
        )

        write_tags(AudioFormat.WAV, mf, metadata)

        # At this point the file on disk should already have LIST stripped
        # (because write_tags calls strip_wav_list_chunks before returning).
        data = wav.read_bytes()
        assert b"LIST" not in data, "LIST should be stripped before save"

        mf.save()

        # After save, file should still have no LIST chunk
        data = wav.read_bytes()
        assert b"LIST" not in data, "LIST should not reappear after save"
        assert data[:4] == b"RIFF"
        assert data[8:12] == b"WAVE"

        # Mutagen should still read the saved file correctly
        mf2 = File(str(wav))
        assert mf2 is not None
        assert mf2.tags is not None
        assert str(mf2.tags["TPE1"]) == "Test Artist"
        assert str(mf2.tags["TALB"]) == "Test Album"
