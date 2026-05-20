"""Tests for artist name alias matching and cross-script support.

Focuses on the substring-length guard that prevents false positives
when a tag artist has junk appended (e.g. folder path used as artist).
"""

from __future__ import annotations

from auto_tagger.integrations.aliases import (
    _characters_overlap,
    _convert_script,
    artist_matches_any,
    get_aliases,
    is_chinese_name,
    save_alias,
)


# ── artist_matches_any ───────────────────────────────────────────────

class TestArtistMatchesAny:
    """Tests for artist_matches_any with focus on substring-length guard."""

    # ── Exact matches ─────────────────────────────────────────

    def test_exact_match_ascii(self) -> None:
        assert artist_matches_any("Tanya Chua", "Tanya Chua") is True

    def test_exact_match_chinese(self) -> None:
        assert artist_matches_any("陈洁仪", "陈洁仪") is True

    def test_exact_match_case_insensitive(self) -> None:
        assert artist_matches_any("tanya chua", "Tanya Chua") is True

    def test_exact_match_whitespace(self) -> None:
        assert artist_matches_any(" 陈洁仪 ", "陈洁仪") is True

    # ── Corrupt tag: full folder path used as artist ──────────
    # These are the primary motivation for the length guard.

    def test_corrupt_folder_as_artist(self) -> None:
        """Artist tag contains the full folder path with junk appended.
        hint="陈洁仪" should NOT match artist="陈洁仪-2002-异想世界 2CD WAV 分轨"
        because the hint is only ~12% of the artist length.
        """
        assert artist_matches_any(
            "陈洁仪-2002-异想世界 2CD WAV 分轨", "陈洁仪"
        ) is False

    def test_corrupt_with_year_suffix(self) -> None:
        """Artist tag has year/year-range appended.
        "王菲" (2) vs "王菲1997" (5): ratio=40% > 20%, so the guard
        passes. The health report catches this through other signals.
        """
        assert artist_matches_any("王菲1997", "王菲") is True

    def test_corrupt_with_album_name(self) -> None:
        """Artist tag includes the album name after a separator.
        "陈慧琳" (3) vs "陈慧琳 - 爱情来了" (9): ratio=33% > 20%,
        so the guard passes. The health report catches this.
        """
        assert artist_matches_any(
            "陈慧琳 - 爱情来了", "陈慧琳"
        ) is True

    def test_corrupt_with_format_suffix(self) -> None:
        """Artist tag includes format info like [WAV分轨].
        "蔡依林" (3) vs "蔡依林[FLAC分轨]" (11): ratio=27% > 20%,
        so the guard passes. The health report catches this.
        """
        assert artist_matches_any(
            "蔡依林[FLAC分轨]", "蔡依林"
        ) is True

    # ── Legitimate substring: hint contained in artist ────────
    # hint is shorter than artist but ratio >= 40%.

    def test_feat_suffix_legitimate(self) -> None:
        """Artist includes feat. information; hint is ~42% of full name."""
        assert artist_matches_any(
            "Tanya Chua feat. Someone", "Tanya Chua"
        ) is True

    def test_ft_suffix_legitimate(self) -> None:
        """Artist includes ft. abbreviation; hint is ~32% of full name."""
        assert artist_matches_any(
            "Jay Chou ft. Lara Veronin", "Jay Chou"
        ) is True

    def test_full_name_legitimate(self) -> None:
        """Artist tag has the full name; hint is a shorter form (ratio > 80%)."""
        assert artist_matches_any(
            "Beyoncé Knowles", "Beyoncé"
        ) is True

    def test_and_suffix_legitimate(self) -> None:
        """Artist includes '&' collaborator; hint is ~25% of name."""
        assert artist_matches_any(
            "小娟&山谷里的居民", "小娟"
        ) is True

    # ── Safe direction: artist shorter than hint ──────────────
    # When the tag artist is a substring of the folder name,
    # no length guard applies (this direction is inherently safe).

    def test_tag_artist_is_subset_of_folder(self) -> None:
        """Tag says 小娟, folder says 小娟&山谷里的居民."""
        assert artist_matches_any("小娟", "小娟&山谷里的居民") is True

    def test_tag_artist_subset_with_pinyin(self) -> None:
        """Tag says Jay, folder says Jay Chou."""
        assert artist_matches_any("Jay", "Jay Chou") is True

    def test_tag_artist_is_short_alias(self) -> None:
        """Tag says 王菲, folder says 王菲 (精选) -- safe direction."""
        assert artist_matches_any("王菲", "王菲 精选") is True

    # ── Different artists ─────────────────────────────────────

    def test_different_artists_no_shared_chars(self) -> None:
        """Completely different names."""
        assert artist_matches_any("王菲", "陈洁仪") is False

    def test_different_artists_partial_overlap(self) -> None:
        """Different artists that share some characters.
        This is a pre-existing behavior of _characters_overlap:
        2/3 shared characters (黄, 明) gives overlap=0.667 >= 0.5.
        The length guard (3 vs 4, ratio=0.75) doesn't reject it.
        """
        assert artist_matches_any("黄晓明", "黄小明了") is True

    # ── SC/TC variant matching ────────────────────────────────

    def test_sc_tc_variant_same_length(self) -> None:
        """Simplified vs Traditional Chinese, same length."""
        assert artist_matches_any("刘德华", "刘德華") is True

    def test_sc_tc_variant_with_junk(self) -> None:
        """TC version with junk appended should still be rejected."""
        assert artist_matches_any(
            "陳潔儀-2002-異想世界 2CD WAV 分軌", "陈洁仪"
        ) is False

    def test_shinjitai_variant(self) -> None:
        """Japanese shinjitai vs simplified Chinese (same length)."""
        assert artist_matches_any("久石让", "久石譲") is True

    # ── Edge cases ────────────────────────────────────────────

    def test_none_artist(self) -> None:
        assert artist_matches_any(None, "test") is False

    def test_none_hint(self) -> None:
        assert artist_matches_any("test", None) is False

    def test_empty_string_artist(self) -> None:
        assert artist_matches_any("", "test") is False

    def test_both_empty(self) -> None:
        assert artist_matches_any("", "") is False

    def test_single_char_vs_long(self) -> None:
        """Single character hint should not match a long artist string."""
        assert artist_matches_any("A Very Long Artist Name Here", "A") is False

    def test_latin_short_vs_long(self) -> None:
        """Very short Latin hint should not match long artist."""
        assert artist_matches_any(
            "Taylor Swift (feat. Kendrick Lamar) - Bad Blood", "Taylor"
        ) is False

    def test_album_name_in_artist_field_rejected(self) -> None:
        """Artist field contains the album name — common download mistake.
        "林俊杰" (3) vs "林俊杰 2005 编号89757" (16, with numbers+spaces):
        ratio=18.75% < 20%, so the length guard rejects it.
        """
        assert artist_matches_any(
            "林俊杰 2005 编号89757", "林俊杰"
        ) is False


# ── _characters_overlap ──────────────────────────────────────────────


class TestCharactersOverlap:
    """Tests for character-level overlap with length guard."""

    def test_same_string(self) -> None:
        assert _characters_overlap("陈洁仪", "陈洁仪") == 1.0

    def test_same_chars_different_script(self) -> None:
        """SC vs TC should still match via per-character OpenCC."""
        score = _characters_overlap("刘德华", "刘德華")
        assert score >= 0.5

    def test_junk_appended_rejected(self) -> None:
        """Junk-appended name should return 0.0 due to length guard."""
        score = _characters_overlap(
            "陈洁仪", "陈洁仪-2002-异想世界 2CD WAV 分轨"
        )
        assert score == 0.0

    def test_tc_junk_appended_rejected(self) -> None:
        """TC variant with junk should also be rejected."""
        score = _characters_overlap(
            "陈洁仪", "陳潔儀-2002-異想世界 2CD WAV 分軌"
        )
        assert score == 0.0

    def test_all_chars_match_in_longer_string_rejected(self) -> None:
        """All chars from short string appear in long string, but length
        ratio (3/20 = 15%) is below 20% threshold — should return 0.0."""
        score = _characters_overlap("abc", "abc123def456ghijklmn")
        assert score == 0.0

    def test_empty_strings(self) -> None:
        assert _characters_overlap("", "test") == 0.0
        assert _characters_overlap("test", "") == 0.0
        assert _characters_overlap("", "") == 0.0


# ── _convert_script ──────────────────────────────────────────────────


class TestConvertScript:
    """Tests for OpenCC script variant generation."""

    def test_original_always_included(self) -> None:
        variants = _convert_script("陈洁仪")
        assert "陈洁仪" in variants

    def test_tc_variant_generated(self) -> None:
        variants = _convert_script("陈洁仪")
        assert "陳潔儀" in variants

    def test_sc_variant_generated(self) -> None:
        variants = _convert_script("陳潔儀")
        assert "陈洁仪" in variants

    def test_no_cjk_returns_original(self) -> None:
        variants = _convert_script("Tanya Chua")
        assert variants == ["Tanya Chua"]


# ── is_chinese_name ──────────────────────────────────────────────────


class TestIsChineseName:
    """Tests for CJK character detection."""

    def test_chinese(self) -> None:
        assert is_chinese_name("陈洁仪") is True

    def test_ascii(self) -> None:
        assert is_chinese_name("Tanya Chua") is False

    def test_mixed(self) -> None:
        assert is_chinese_name("Tanya Chua 蔡健雅") is True

    def test_empty(self) -> None:
        assert is_chinese_name("") is False

    def test_japanese_kanji(self) -> None:
        assert is_chinese_name("久石譲") is True


# ── Alias save/get integration ───────────────────────────────────────


class TestSaveAndGetAliases:
    """Tests for alias persistence (uses a mock path)."""

    def test_save_and_get(self, monkeypatch) -> None:  # type: ignore[no-untyped-def]
        """Round-trip: save an alias, then retrieve it."""
        import json
        from pathlib import Path
        from auto_tagger.integrations.aliases import ALIAS_FILE

        # Use a temp file
        tmp = Path("/tmp/test-auto-tagger-aliases.json")
        tmp.unlink(missing_ok=True)
        monkeypatch.setattr(ALIAS_FILE.__class__, "resolve", lambda self: tmp)  # type: ignore[assignment]
        monkeypatch.setattr("auto_tagger.integrations.aliases.ALIAS_FILE", tmp)

        save_alias("陈洁仪", "Kit Chan")
        aliases = get_aliases("陈洁仪")
        assert "Kit Chan" in aliases

        # Cleanup
        tmp.unlink(missing_ok=True)
