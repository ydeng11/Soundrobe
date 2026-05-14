"""Tests for compilation album detection and tagging."""

from auto_tagger.core.audio import AudioFormat
from auto_tagger.core.formats import read_tags, write_tags
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.features.compilations import (
    analyze_compilation,
    apply_compilation_tags,
    apply_smart_album_tags,
)


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


# ── new tests for multi-artist patterns ────────────────────────


def test_collaboration_single_not_compilation():
    """A single track by many artists is a collaboration, not compilation."""
    tracks = [
        TrackMetadata(
            title="We Are The World",
            artist="U.S.A. For Africa, Michael Jackson, Lionel Richie, Stevie Wonder",
            album="We Are The World",
            track_number=1,
        ),
    ]

    analysis = analyze_compilation(tracks)

    assert analysis.is_compilation is False
    assert analysis.is_collaboration is True
    assert "collaboration" in " ".join(analysis.reasons).lower()


def test_smart_tags_on_collaboration():
    """apply_smart_album_tags sets group name, populates artists list."""
    tracks = [
        TrackMetadata(
            title="We Are The World",
            artist="U.S.A. For Africa, Michael Jackson, Lionel Richie, Stevie Wonder",
            album="We Are The World",
            track_number=1,
        ),
    ]
    analysis = analyze_compilation(tracks)
    updated = apply_smart_album_tags(tracks, analysis)

    assert updated[0].album_artist == "U.S.A. For Africa"
    assert updated[0].compilation is False
    assert "Michael Jackson" in updated[0].artists
    assert "Lionel Richie" in updated[0].artists
    assert "Stevie Wonder" in updated[0].artists


def test_classical_album_not_compilation():
    """Classical album with varying composers but same performer is not compilation."""
    tracks = [
        TrackMetadata(
            title="Likoo, for Violin Solo",
            artist="Anne‐Sophie Mutter",
            album="East Meets West",
            track_number=1,
            composer="Aftab Darvishi",
        ),
        TrackMetadata(
            title="Studie über Beethoven I",
            artist="Anne‐Sophie Mutter, Ye-Eun Choi, Muriel Razavi, Pablo Ferrández",
            album="East Meets West",
            track_number=3,
            composer="Jörg Widmann",
        ),
        TrackMetadata(
            title="Air-Homage to Sibelius I",
            artist="Anne‐Sophie Mutter, Stephanie Gonley, London Symphony Orchestra, Thomas Adès",
            album="East Meets West",
            track_number=14,
            composer="Thomas Adès",
        ),
    ]

    analysis = analyze_compilation(tracks)

    assert analysis.is_compilation is False
    assert analysis.is_collaboration is False
    assert analysis.suggested_album_artist == "Anne‐Sophie Mutter"
    assert any("primary performer" in r.lower() for r in analysis.reasons)


def test_smart_tags_on_classical_album():
    """apply_smart_album_tags sets primary performer as album_artist."""
    tracks = [
        TrackMetadata(
            title="Likoo, for Violin Solo",
            artist="Anne‐Sophie Mutter",
            album="East Meets West",
            track_number=1,
            composer="Aftab Darvishi",
        ),
        TrackMetadata(
            title="Air-Homage to Sibelius",
            artist="Anne‐Sophie Mutter, Thomas Adès, LSO",
            album="East Meets West",
            track_number=14,
            composer="Thomas Adès",
        ),
    ]
    analysis = analyze_compilation(tracks)
    updated = apply_smart_album_tags(tracks, analysis)

    assert updated[0].album_artist == "Anne‐Sophie Mutter"
    assert updated[1].album_artist == "Anne‐Sophie Mutter"
    assert all(t.compilation is False for t in updated)
    # Per-track artist is preserved
    assert "Thomas Adès" in updated[1].artists or "Thomas Adès" in updated[1].artist


def test_classical_album_uniform_performers_not_compilation():
    """Symphony album with same performers on all tracks."""
    tracks = [
        TrackMetadata(
            title="Allegretto [Symphony No.2 in D major, Op.43]",
            artist="Herbert Blomstedt, San Francisco Symphony",
            album="Symphony no. 2 & no. 5 Valse Triste",
            track_number=1,
        ),
        TrackMetadata(
            title="Tempo andante [Symphony No.2 in D major, Op.43]",
            artist="Herbert Blomstedt, San Francisco Symphony",
            album="Symphony no. 2 & no. 5 Valse Triste",
            track_number=2,
        ),
        TrackMetadata(
            title="Valse triste, Op.44 No.1",
            artist="Herbert Blomstedt, San Francisco Symphony",
            album="Symphony no. 2 & no. 5 Valse Triste",
            track_number=8,
        ),
    ]

    analysis = analyze_compilation(tracks)

    assert analysis.is_compilation is False
    # Same primary performer on all tracks → single-primary signal
    assert analysis.suggested_album_artist == "Herbert Blomstedt"


def test_smart_tags_preserves_symphony_album_artist():
    """Symphony album keeps conductor/orchestra as album artist."""
    tracks = [
        TrackMetadata(
            title="Allegretto",
            artist="Herbert Blomstedt, San Francisco Symphony",
            album="Symphony no. 2 & 5",
            track_number=1,
        ),
        TrackMetadata(
            title="Valse triste",
            artist="Herbert Blomstedt, San Francisco Symphony",
            album="Symphony no. 2 & 5",
            track_number=8,
        ),
    ]
    analysis = analyze_compilation(tracks)
    updated = apply_smart_album_tags(tracks, analysis)

    assert updated[0].album_artist == "Herbert Blomstedt"
    assert all(t.compilation is False for t in updated)
    # Per-track artist preserved — not split since it's conductor+orchestra (2 parts)
    assert "Herbert Blomstedt, San Francisco Symphony" == updated[0].artist


def test_soundtrack_compilation_detected():
    """TV OST with different artists per track is a compilation."""
    tracks = [
        TrackMetadata(title="送雪", artist="张远", album="天地剑心原声带", track_number=1),
        TrackMetadata(title="梦境", artist="小时姑娘", album="天地剑心原声带", track_number=2),
        TrackMetadata(title="谁能", artist="李琦", album="天地剑心原声带", track_number=3),
        TrackMetadata(title="一刻天光", artist="阿兰", album="天地剑心原声带", track_number=4),
        TrackMetadata(title="万剑不改", artist="刘宇宁", album="天地剑心原声带", track_number=5),
        TrackMetadata(title="何所惧", artist="成毅", album="天地剑心原声带", track_number=6),
        TrackMetadata(title="你不是孤岛", artist="颜人中", album="天地剑心原声带", track_number=7),
        TrackMetadata(title="卿卿", artist="叶炫清", album="天地剑心原声带", track_number=8),
    ]

    analysis = analyze_compilation(tracks, album_path_hint="天地剑心原声带")

    assert analysis.is_compilation is True
    assert analysis.suggested_album_artist == "Various Artists"


def test_smart_tags_preserves_soundtrack_compilation():
    """Soundtrack compilation gets Various Artists treatment."""
    tracks = [
        TrackMetadata(title="送雪", artist="张远", album="天地剑心原声带", track_number=1),
        TrackMetadata(title="梦境", artist="小时姑娘", album="天地剑心原声带", track_number=2),
        TrackMetadata(title="谁能", artist="李琦", album="天地剑心原声带", track_number=3),
    ]
    analysis = analyze_compilation(tracks, album_path_hint="天地剑心原声带")
    updated = apply_smart_album_tags(tracks, analysis)

    assert all(t.album_artist == "Various Artists" for t in updated)
    assert all(t.compilation is True for t in updated)
    assert updated[0].artist == "张远"
    assert updated[1].artist == "小时姑娘"


def test_composer_round_trip_vorbis():
    """Composer tag reads and writes correctly for Vorbis."""
    tags = {"TITLE": ["Song"], "ARTIST": ["A"], "COMPOSER": ["Beethoven"]}
    metadata = read_tags(AudioFormat.FLAC, tags)
    output: dict[str, list[str]] = {}
    write_tags(AudioFormat.FLAC, output, metadata)

    assert metadata.composer == "Beethoven"
    assert output["COMPOSER"] == ["Beethoven"]


def test_composer_round_trip_mp3():
    """Composer tag reads and writes correctly for MP3."""
    from mutagen.id3 import ID3, TCOM, TIT2, TPE1

    tags = ID3()
    tags["TIT2"] = TIT2(encoding=3, text=["Song"])
    tags["TPE1"] = TPE1(encoding=3, text=["A"])
    tags["TCOM"] = TCOM(encoding=3, text=["Beethoven"])

    metadata = read_tags(AudioFormat.MP3, tags)
    output = type(tags)()
    write_tags(AudioFormat.MP3, output, metadata)

    assert metadata.composer == "Beethoven"
    assert str(output.get("TCOM")) == "Beethoven"
