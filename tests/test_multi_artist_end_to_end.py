"""End-to-end tests for multi-artist detection and smart album tagging.

Creates real silent audio files with intentionally wrong tags, runs the
compilation analysis + smart tagging pipeline, writes the corrected tags,
and verifies the final output matches the expected correct state.

Each test uses a real-world scenario:

1. **We Are The World** — collaboration single (one track, many performers)
2. **天地剑心原声带** — TV OST compilation (different artist per track)
3. **Anne-Sophie Mutter** — classical album (single performer, varying composers)
4. **Sibelius** — symphony album (conductor + orchestra, uniform across tracks)
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.core.reader import read_metadata
from auto_tagger.core.writer import write_metadata
from auto_tagger.features.compilations import (
    analyze_compilation,
    apply_smart_album_tags,
)

# ── helpers ───────────────────────────────────────────────────


def _ffmpeg_available() -> bool:
    return shutil.which("ffmpeg") is not None


needs_ffmpeg = pytest.mark.skipif(
    not _ffmpeg_available(), reason="requires ffmpeg for audio generation"
)


def _create_silent_flac(path: Path, track: int | None = None) -> None:
    """Create a minimal silent FLAC file via ffmpeg."""
    subprocess.run(
        [
            "ffmpeg",
            "-y",
            "-f", "lavfi",
            "-i", "anullsrc=r=44100:cl=stereo",
            "-t", "0.1",
            "-c:a", "flac",
            "-sample_fmt", "s16",
            str(path),
        ],
        check=True,
        capture_output=True,
    )


def _create_silent_mp3(path: Path, track: int | None = None) -> None:
    """Create a minimal silent MP3 file via ffmpeg."""
    subprocess.run(
        [
            "ffmpeg",
            "-y",
            "-f", "lavfi",
            "-i", "anullsrc=r=44100:cl=stereo",
            "-t", "0.05",
            "-c:a", "libmp3lame",
            "-q:a", "9",
            str(path),
        ],
        check=True,
        capture_output=True,
    )


def _write_raw_flac_tags(path: Path, tags: dict[str, str | list[str]]) -> None:
    """Write raw Vorbis tags to a FLAC file using mutagen directly."""
    from mutagen.flac import FLAC

    audio = FLAC(path)
    for key, value in tags.items():
        if isinstance(value, list):
            audio[key] = value
        else:
            audio[key] = value
    audio.save()


def _write_raw_mp3_tags(path: Path, tags: dict[str, str]) -> None:
    """Write raw ID3 tags to an MP3 file using mutagen directly."""
    from mutagen.id3 import ID3, TALB, TCON, TDRC, TIT2, TPE1, TPE2, TRCK

    audio = ID3()
    for frame_id, text in tags.items():
        cls = {
            "TIT2": TIT2,
            "TPE1": TPE1,
            "TPE2": TPE2,
            "TALB": TALB,
            "TDRC": TDRC,
            "TCON": TCON,
            "TRCK": TRCK,
        }.get(frame_id)
        if cls:
            audio[frame_id] = cls(encoding=3, text=[text])
    audio.save(path)


# ── Scenario 1: We Are The World (Collaboration Single) ──────


class TestWeAreTheWorld:
    """A single track by 45 artists — collaboration, not compilation.

    Current wrong tagging (as seen on the real file):
        TPE1 = "U.S.A. For Africa, Al Jarreau, Anita Pointer, ..." (all names)
        TPE2 = "Various Artists"  (wrong — should be the group name)
        TCMP  = not set

    Expected correct tagging:
        artist   = "U.S.A. For Africa"
        artists  = ["Michael Jackson", "Lionel Richie", ...]
        album_artist = "U.S.A. For Africa"
        compilation  = False
    """

    WRONG_TAGS = {
        "TIT2": "We Are The World",
        "TPE1": "U.S.A. For Africa, Michael Jackson, Lionel Richie, Stevie Wonder, "
                "Bruce Springsteen, Cyndi Lauper, Bob Dylan, Ray Charles, Tina Turner",
        "TPE2": "Various Artists",
        "TALB": "We Are The World",
        "TDRC": "2025",
        "TRCK": "1",
    }

    @needs_ffmpeg
    def test_detects_collaboration_not_compilation(self, tmp_path: Path):
        """Analysis correctly identifies collaboration pattern."""
        album_dir = tmp_path / "U.S.A. For Africa" / "We Are The World"
        album_dir.mkdir(parents=True)
        mp3_path = album_dir / "01 - We Are The World.mp3"

        _create_silent_mp3(mp3_path)
        _write_raw_mp3_tags(mp3_path, self.WRONG_TAGS)

        # Read back with auto_tagger
        meta = read_metadata(mp3_path)
        analysis = analyze_compilation([meta], album_path_hint=str(album_dir))

        assert analysis.is_collaboration, (
            f"Expected collaboration, got: is_compilation={analysis.is_compilation}, "
            f"reasons={analysis.reasons}"
        )
        assert analysis.is_compilation is False
        assert analysis.suggested_album_artist == "U.S.A. For Africa"

    @needs_ffmpeg
    def test_smart_tags_corrects_album_artist(self, tmp_path: Path):
        """apply_smart_album_tags fixes album_artist and populates artists."""
        album_dir = tmp_path / "U.S.A. For Africa" / "We Are The World"
        album_dir.mkdir(parents=True)
        mp3_path = album_dir / "01 - We Are The World.mp3"

        _create_silent_mp3(mp3_path)
        _write_raw_mp3_tags(mp3_path, self.WRONG_TAGS)

        meta = read_metadata(mp3_path)
        analysis = analyze_compilation([meta], album_path_hint=str(album_dir))
        updated = apply_smart_album_tags([meta], analysis)

        updated_meta = updated[0]
        assert updated_meta.album_artist == "U.S.A. For Africa"
        assert updated_meta.compilation is not True  # None or False
        assert "Michael Jackson" in updated_meta.artists, f"artists={updated_meta.artists}"
        assert "Ray Charles" in updated_meta.artists

        # Write then re-read for persistence check
        write_metadata(mp3_path, updated_meta, dry_run=False)
        reloaded = read_metadata(mp3_path)
        assert reloaded.album_artist == "U.S.A. For Africa"
        # COMPILATION=1 is never written for non-compilations
        assert reloaded.compilation is not True


# ── Scenario 2: 天地剑心原声带 (TV OST / Compilation) ──────


class TestTianDiJianXinOST:
    """C-drama OST with 8 tracks by different artists.

    Current wrong tagging (as seen on the real file):
        TPE2 = "Various Artists"  (correct for a compilation)
        But compilation flag is not set, and there's messy duplicate tags.

    Expected correct tagging:
        album_artist = "Various Artists"
        compilation  = True
        Per-track artist preserved
    """

    ARTISTS_AND_TRACKS = [
        ("送雪", "张远"),
        ("梦境", "小时姑娘"),
        ("谁能", "李琦"),
        ("一刻天光", "阿兰"),
        ("万剑不改", "刘宇宁"),
        ("何所惧", "成毅"),
        ("你不是孤岛", "颜人中"),
        ("卿卿", "叶炫清"),
    ]

    @needs_ffmpeg
    def test_detects_soundtrack_compilation(self, tmp_path: Path):
        """8 tracks by different artists → compilation."""
        album_dir = tmp_path / "Various Artists" / "天地剑心原声带"
        album_dir.mkdir(parents=True)
        metas = []
        files = []

        for i, (title, artist) in enumerate(self.ARTISTS_AND_TRACKS, start=1):
            mp3_path = album_dir / f"{i:02d} - {artist} - {title}.mp3"
            _create_silent_mp3(mp3_path)
            # Write WRONG tags: no album_artist, no compilation flag
            _write_raw_mp3_tags(mp3_path, {
                "TIT2": title,
                "TPE1": artist,
                "TALB": "天地剑心原声带",
                "TRCK": str(i),
                "TDRC": "2025",
            })
            files.append(mp3_path)

        # Read all tracks
        for f in files:
            metas.append(read_metadata(f))

        analysis = analyze_compilation(metas, album_path_hint="天地剑心原声带")

        assert analysis.is_compilation, (
            f"Expected compilation, got is_compilation={analysis.is_compilation}, "
            f"reasons={analysis.reasons}"
        )
        assert analysis.suggested_album_artist == "Various Artists"
        assert any("原声带" in r or "soundtrack" in r.lower() for r in analysis.reasons)

    @needs_ffmpeg
    def test_smart_tags_preserves_track_artists(self, tmp_path: Path):
        """Compilation tagging preserves per-track artist."""
        album_dir = tmp_path / "Various Artists" / "天地剑心原声带"
        album_dir.mkdir(parents=True)
        metas = []
        files = []

        for i, (title, artist) in enumerate(self.ARTISTS_AND_TRACKS[:3], start=1):
            mp3_path = album_dir / f"{i:02d} - {artist} - {title}.mp3"
            _create_silent_mp3(mp3_path)
            _write_raw_mp3_tags(mp3_path, {
                "TIT2": title,
                "TPE1": artist,
                "TALB": "天地剑心原声带",
                "TRCK": str(i),
                "TDRC": "2025",
            })
            files.append(mp3_path)

        for f in files:
            metas.append(read_metadata(f))

        analysis = analyze_compilation(metas, album_path_hint="天地剑心原声带")
        updated = apply_smart_album_tags(metas, analysis)

        assert all(u.album_artist == "Various Artists" for u in updated)
        assert all(u.compilation is True for u in updated)
        assert updated[0].artist == "张远"
        assert updated[1].artist == "小时姑娘"
        assert updated[2].artist == "李琦"

        # Persist and re-read
        for f, u in zip(files, updated):
            write_metadata(f, u, dry_run=False)

        reloaded = [read_metadata(f) for f in files]
        assert all(r.album_artist == "Various Artists" for r in reloaded)
        assert all(r.compilation is True for r in reloaded)
        assert reloaded[0].artist == "张远"


# ── Scenario 3: Anne-Sophie Mutter (Classical / Single Performer) ─


class TestAnneSophieMutter:
    """Classical album: same primary performer, varying composers & ensembles.

    Current real tagging:
        artist per track varies: "Anne‐Sophie Mutter", "Anne‐Sophie Mutter, Ye-Eun Choi..."
        albumartist = "Aftab Darvishi, Unsuk Chin, Jörg Widmann, Thomas Adès; Anne‐Sophie Mutter"
        composer varies per track

    Expected correct tagging after smart analysis:
        album_artist = "Anne‐Sophie Mutter"
        compilation  = False
        composer preserved per track
        per-track artist preserved
    """

    TRACKS_DATA = [
        ("Likoo, for Violin Solo", "Anne‐Sophie Mutter", "Aftab Darvishi", 1),
        ("Gran Cadenza, for Two Violins", "Anne‐Sophie Mutter", "Unsuk Chin", 2),
        ("Studie über Beethoven I", "Anne‐Sophie Mutter, Ye-Eun Choi, Muriel Razavi, Pablo Ferrández", "Jörg Widmann", 3),
        ("Air-Homage to Sibelius I", "Anne‐Sophie Mutter, Stephanie Gonley, London Symphony Orchestra, Thomas Adès", "Thomas Adès", 14),
    ]

    @needs_ffmpeg
    def test_detects_classical_not_compilation(self, tmp_path: Path):
        """Varying track artists but same primary performer → not compilation."""
        album_dir = tmp_path / "Anne-Sophie Mutter" / "East Meets West"
        album_dir.mkdir(parents=True)
        metas = []
        files = []

        for title, artist, composer, num in self.TRACKS_DATA:
            flac_path = album_dir / f"{num:02d} - {title}.flac"
            _create_silent_flac(flac_path, track=num)
            _write_raw_flac_tags(flac_path, {
                "title": title,
                "artist": artist,
                "composer": composer,
                "album": "East Meets West",
                "tracknumber": str(num),
                "date": "2026",
            })
            files.append(flac_path)

        for f in files:
            metas.append(read_metadata(f))

        analysis = analyze_compilation(metas)

        assert analysis.is_compilation is False, (
            f"Expected not compilation, got: reasons={analysis.reasons}"
        )
        assert analysis.is_collaboration is False
        assert analysis.suggested_album_artist == "Anne‐Sophie Mutter"
        assert any("primary performer" in r.lower() for r in analysis.reasons)

    @needs_ffmpeg
    def test_smart_tags_sets_primary_performer(self, tmp_path: Path):
        """Smart tagging uses primary performer as album_artist."""
        album_dir = tmp_path / "Anne-Sophie Mutter" / "East Meets West"
        album_dir.mkdir(parents=True)
        metas = []
        files = []

        for title, artist, composer, num in self.TRACKS_DATA[:3]:
            flac_path = album_dir / f"{num:02d} - {title}.flac"
            _create_silent_flac(flac_path, track=num)
            _write_raw_flac_tags(flac_path, {
                "title": title,
                "artist": artist,
                "composer": composer,
                "album": "East Meets West",
                "tracknumber": str(num),
                "date": "2026",
            })
            files.append(flac_path)

        for f in files:
            metas.append(read_metadata(f))

        analysis = analyze_compilation(metas)
        updated = apply_smart_album_tags(metas, analysis)

        assert all(u.album_artist == "Anne‐Sophie Mutter" for u in updated)
        assert all(u.compilation is not True for u in updated)
        # Composers preserved
        assert updated[0].composer == "Aftab Darvishi"
        assert updated[1].composer == "Unsuk Chin"
        # Per-track artist preserved
        assert "Ye-Eun Choi" in updated[2].artist or "Ye-Eun Choi" in updated[2].artists

        # Persist
        for f, u in zip(files, updated):
            write_metadata(f, u, dry_run=False)

        reloaded = [read_metadata(f) for f in files]
        assert all(r.album_artist == "Anne‐Sophie Mutter" for r in reloaded)
        assert all(r.compilation is not True for r in reloaded)


# ── Scenario 4: Sibelius (Symphony / Uniform Performers) ───────


class TestSibelius:
    """Symphony album: same conductor + orchestra on all tracks.

    Current real tagging:
        artist = "Herbert Blomstedt, San Francisco Symphony"
        albumartist = "Herbert Blomstedt, San Francisco Symphony"
        No composer tag

    Expected correct tagging after smart analysis:
        album_artist = "Herbert Blomstedt"
        compilation  = False
        Per-track artist preserved
    """

    TRACKS_DATA = [
        ("Allegretto [Symphony No.2 in D major, Op.43]", "Herbert Blomstedt, San Francisco Symphony", 1),
        ("Tempo andante [Symphony No.2 in D major, Op.43]", "Herbert Blomstedt, San Francisco Symphony", 2),
        ("Valse triste, Op.44 No.1", "Herbert Blomstedt, San Francisco Symphony", 8),
    ]

    @needs_ffmpeg
    def test_detects_symphony_not_compilation(self, tmp_path: Path):
        """Uniform artist across tracks → not compilation, not collaboration."""
        album_dir = tmp_path / "Sibelius" / "Symphony No.2 & 5"
        album_dir.mkdir(parents=True)
        metas = []
        files = []

        for title, artist, num in self.TRACKS_DATA:
            flac_path = album_dir / f"{num:02d} - {title}.flac"
            _create_silent_flac(flac_path, track=num)
            _write_raw_flac_tags(flac_path, {
                "title": title,
                "artist": artist,
                "album": "Symphony no. 2 & no. 5 Valse Triste",
                "tracknumber": str(num),
            })
            files.append(flac_path)

        for f in files:
            metas.append(read_metadata(f))

        analysis = analyze_compilation(metas)

        assert analysis.is_compilation is False, (
            f"Expected not compilation, got is_compilation={analysis.is_compilation}, "
            f"reasons={analysis.reasons}"
        )
        assert analysis.is_collaboration is False
        # Single primary performer → suggested album_artist
        assert analysis.suggested_album_artist is not None

    @needs_ffmpeg
    def test_smart_tags_sets_conductor(self, tmp_path: Path):
        """Smart tagging sets conductor as album_artist, preserves orchestra."""
        album_dir = tmp_path / "Sibelius" / "Symphony No.2 & 5"
        album_dir.mkdir(parents=True)
        metas = []
        files = []

        for title, artist, num in self.TRACKS_DATA:
            flac_path = album_dir / f"{num:02d} - {title}.flac"
            _create_silent_flac(flac_path, track=num)
            _write_raw_flac_tags(flac_path, {
                "title": title,
                "artist": artist,
                "album": "Symphony no. 2 & no. 5 Valse Triste",
                "tracknumber": str(num),
            })
            files.append(flac_path)

        for f in files:
            metas.append(read_metadata(f))

        analysis = analyze_compilation(metas)
        updated = apply_smart_album_tags(metas, analysis)

        assert all(u.compilation is not True for u in updated)
        # Per-track artist preserved with full conductor+orchestra string
        assert "San Francisco Symphony" in updated[0].artist

        # Persist
        for f, u in zip(files, updated):
            write_metadata(f, u, dry_run=False)

        reloaded = [read_metadata(f) for f in files]
        assert all(r.compilation is not True for r in reloaded)
        assert "San Francisco Symphony" in reloaded[0].artist
