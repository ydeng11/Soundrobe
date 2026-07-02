"""Tests for Chinese script enforcement flag."""

from __future__ import annotations

from pathlib import Path
from unittest.mock import MagicMock

import pytest

from auto_tagger.config.settings import Settings
from auto_tagger.core.metadata import TrackMetadata, ReplayGainTags, convert_chinese_script
from auto_tagger.core.writer import write_metadata


# ── convert_chinese_script ────────────────────────────────────────────────


class TestConvertChineseScript:
    def test_sc_to_tc(self):
        """Simplified Chinese should convert to Traditional Chinese."""
        result = convert_chinese_script("音乐", "traditional")
        assert result == "音樂"

    def test_tc_to_sc(self):
        """Traditional Chinese should convert to Simplified Chinese."""
        result = convert_chinese_script("音樂", "simplified")
        assert result == "音乐"

    def test_sc_to_tc_artist_name(self):
        result = convert_chinese_script("蔡健雅", "traditional")
        # 蔡健雅 is the same in SC/TC (these chars are not variant), but
        # the function should still return without error.
        assert result is not None

    def test_non_cjk_unchanged(self):
        """Non-CJK text should pass through unchanged."""
        result = convert_chinese_script("Radiohead OK Computer", "simplified")
        assert result == "Radiohead OK Computer"

    def test_none_input(self):
        """None input should return None."""
        assert convert_chinese_script(None, "simplified") is None
        assert convert_chinese_script(None, "traditional") is None

    def test_empty_string(self):
        """Empty string should return empty string."""
        assert convert_chinese_script("", "simplified") == ""
        assert convert_chinese_script("", "traditional") == ""

    def test_mixed_cjk_non_cjk(self):
        """Mixed CJK and non-CJK text should have only CJK converted."""
        result = convert_chinese_script("Jay Chou 专辑", "traditional")
        assert "專輯" in result
        assert "Jay Chou" in result


# ── TrackMetadata.with_chinese_script ──────────────────────────────────────


class TestWithChineseScript:
    def test_converts_text_fields(self):
        """All text-like fields should be converted."""
        meta = TrackMetadata(
            title="音乐之声",
            artist="张三",
            artists=["张三", "李四"],
            album="专辑名",
            album_artist="张三",
            album_artists=["张三"],
            genre="流行",
            composer="王五",
            year="2024",
        )
        tc = meta.with_chinese_script("traditional")
        assert tc.title == "音樂之聲"
        assert tc.artist == "張三"
        assert tc.artists == ["張三", "李四"]
        assert tc.album == "專輯名"
        assert tc.album_artist == "張三"
        assert tc.album_artists == ["張三"]
        assert tc.genre == "流行"  # 流行 is the same in SC/TC
        assert tc.composer == "王五"  # 王五 is the same

    def test_preserves_non_text_fields(self):
        """Non-text fields should be unchanged."""
        rg = ReplayGainTags(track_gain="-6.0 dB", track_peak="1.0")
        meta = TrackMetadata(
            title="测试",
            track_number=1,
            track_total=10,
            disc_number=2,
            disc_total=3,
            musicbrainz_trackid="abc-123",
            musicbrainz_albumid="def-456",
            musicbrainz_artistid="ghi-789",
            compilation=True,
            replaygain=rg,
        )
        tc = meta.with_chinese_script("traditional")
        assert tc.track_number == 1
        assert tc.track_total == 10
        assert tc.disc_number == 2
        assert tc.disc_total == 3
        assert tc.musicbrainz_trackid == "abc-123"
        assert tc.musicbrainz_albumid == "def-456"
        assert tc.musicbrainz_artistid == "ghi-789"
        assert tc.compilation is True
        assert tc.replaygain.track_gain == "-6.0 dB"
        assert tc.replaygain.track_peak == "1.0"

    def test_no_conversion_when_none(self):
        """When target is None, return self unchanged."""
        meta = TrackMetadata(title="测试")
        result = meta.with_chinese_script(None)
        assert result is meta

    def test_no_conversion_when_empty_string(self):
        """When target is empty string, return self unchanged."""
        meta = TrackMetadata(title="测试")
        result = meta.with_chinese_script("")
        assert result is meta

    def test_empty_artists_lists(self):
        """Empty artist lists should remain empty."""
        meta = TrackMetadata(artists=[], album_artists=[])
        tc = meta.with_chinese_script("traditional")
        assert tc.artists == []
        assert tc.album_artists == []


# ── Settings validation ───────────────────────────────────────────────────


class TestSettingsChineseScript:
    def test_simplified(self):
        s = Settings(chinese_script="simplified")
        assert s.chinese_script == "simplified"

    def test_traditional(self):
        s = Settings(chinese_script="traditional")
        assert s.chinese_script == "traditional"

    def test_sc_alias(self):
        """'sc' should be normalized to 'simplified'."""
        s = Settings(chinese_script="sc")
        assert s.chinese_script == "simplified"

    def test_tc_alias(self):
        """'tc' should be normalized to 'traditional'."""
        s = Settings(chinese_script="tc")
        assert s.chinese_script == "traditional"

    def test_case_insensitive(self):
        s = Settings(chinese_script="SC")
        assert s.chinese_script == "simplified"
        s = Settings(chinese_script="Traditional")
        assert s.chinese_script == "traditional"

    def test_default_is_none(self):
        s = Settings()
        assert s.chinese_script is None

    def test_invalid_value_raises(self):
        with pytest.raises(Exception):
            Settings(chinese_script="foo")

    def test_invalid_value_via_model_validate(self):
        """Invalid values should also fail via model_validate (loader path)."""
        with pytest.raises(Exception):
            Settings.model_validate({"chinese_script": "invalid"})


# ── write_metadata dry-run with chinese_script ─────────────────────────────


class FakeTags(dict):
    """Minimal mutable tag object for testing."""
    def __init__(self):
        super().__init__()
        self.save_calls = 0

    def save(self):
        self.save_calls += 1


class TestWriteMetadataChineseScriptDryRun:
    def _setup_monkeypatch(self, monkeypatch):
        """Monkeypatch load_audio_file to return a fake Vorbis-like tag object."""
        from types import SimpleNamespace
        from auto_tagger.core.audio import AudioFormat

        tags = FakeTags()
        audio = SimpleNamespace(path=Path("track.flac"), format=AudioFormat.FLAC, mutagen_file=tags)
        monkeypatch.setattr("auto_tagger.core.writer.load_audio_file", lambda path: audio)
        return tags

    def test_dry_run_returns_converted_metadata(self, monkeypatch):
        """dry_run=True with chinese_script should return converted metadata without writing."""
        tags = self._setup_monkeypatch(monkeypatch)
        meta = TrackMetadata(title="音乐测试", artist="张三")
        result = write_metadata(Path("track.flac"), meta, dry_run=True, chinese_script="traditional")
        assert result.title == "音樂測試"
        assert result.artist == "張三"
        # File should not have been modified (dry run)
        assert tags == {}
        assert tags.save_calls == 0

    def test_dry_run_without_chinese_script(self, monkeypatch):
        """dry_run=True without chinese_script should return normalized but unconverted metadata."""
        self._setup_monkeypatch(monkeypatch)
        meta = TrackMetadata(title="音乐测试", artist="张三")
        result = write_metadata(Path("track.flac"), meta, dry_run=True)
        # Should NOT be converted — just normalized
        assert result.title == "音乐测试"
        assert result.artist == "张三"

    def test_sc_enforcement_via_writer(self, monkeypatch):
        """write_metadata with chinese_script='simplified' should convert TC to SC."""
        self._setup_monkeypatch(monkeypatch)
        meta = TrackMetadata(title="音樂之聲", album="專輯")
        result = write_metadata(Path("track.flac"), meta, dry_run=True, chinese_script="simplified")
        assert result.title == "音乐之声"
        assert result.album == "专辑"
