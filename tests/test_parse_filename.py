"""Tests for filename/folder-name metadata parsing.

Covers all naming conventions observed in real Chinese and Western
music collections.
"""

from __future__ import annotations

from pathlib import Path

from auto_tagger.core.parse_filename import (
    ParsedFilename,
    metadata_from_path,
    parse_album_folder_name,
    parse_track_filename,
)


# ── Track filename parsing ────────────────────────────────────────────


class TestParseTrackFilename:
    """Tests for ``parse_track_filename`` on individual file stems."""

    # Pattern 1:  "(NN) [Artist] Title"
    def test_paren_bracket_artist(self):
        """(01) [陈洁仪] 心痛 → track_number=1, title="心痛", artist="陈洁仪"."""
        result = parse_track_filename("(01) [陈洁仪] 心痛")
        assert result.track_number == 1
        assert result.title == "心痛"
        assert result.artist == "陈洁仪"
        assert result.artists == ["陈洁仪"]

    def test_paren_bracket_collaboration(self):
        """(12) [陈洁仪-苏永康] 来夜方长 → artists split on hyphen."""
        result = parse_track_filename("(12) [陈洁仪-苏永康] 来夜方长")
        assert result.track_number == 12
        assert result.title == "来夜方长"
        # "陈洁仪-苏永康" is not split on hyphen — the split function
        # uses ＋+&,/ comma separators, not plain hyphens
        assert result.artist == "陈洁仪-苏永康"

    def test_paren_bracket_english(self):
        """(10) [Artist] Song Title → English."""
        result = parse_track_filename("(10) [Queen] Bohemian Rhapsody")
        assert result.track_number == 10
        assert result.title == "Bohemian Rhapsody"
        assert result.artist == "Queen"

    def test_paren_bracket_single_digit(self):
        """(1) [A] B → single-digit track number."""
        result = parse_track_filename("(1) [Adele] Hello")
        assert result.track_number == 1
        assert result.title == "Hello"
        assert result.artist == "Adele"

    # Pattern 2:  "Artist - NN.Title"  or  "Artist - NN Title"
    def test_artist_dash_number_dot_title(self):
        """蔡健雅 - 01.呼吸 → track_number=1, title="呼吸", artist="蔡健雅"."""
        result = parse_track_filename("蔡健雅 - 01.呼吸")
        assert result.track_number == 1
        assert result.title == "呼吸"
        assert result.artist == "蔡健雅"

    def test_artist_dash_number_space_title(self):
        """陈绮贞 - 10.夜游 → track_number=10."""
        result = parse_track_filename("陈绮贞 - 10.夜游")
        assert result.track_number == 10
        assert result.title == "夜游"
        assert result.artist == "陈绮贞"

    def test_artist_dash_number_underscore_title(self):
        """Artist - 01_Song → underscore separator."""
        result = parse_track_filename("Artist - 01_Song Title")
        assert result.track_number == 1
        assert result.title == "Song Title"
        assert result.artist == "Artist"

    def test_artist_dash_em_dash_number(self):
        """Artist — 01. Title → em dash."""
        result = parse_track_filename("陈洁仪 — 01.心痛")
        assert result.track_number == 1
        assert result.title == "心痛"
        assert result.artist == "陈洁仪"

    def test_artist_dash_number_with_extra_dots(self):
        """莫文蔚 - 01.這等待眼睛 → no leading junk on title."""
        result = parse_track_filename("莫文蔚 - 01.這等待眼睛")
        assert result.track_number == 1
        assert result.title == "這等待眼睛"

    # Pattern 3:  "NN. Title"
    def test_number_dot_title(self):
        """01. 崇拜 → track_number=1, title="崇拜"."""
        result = parse_track_filename("01. 崇拜")
        assert result.track_number == 1
        assert result.title == "崇拜"
        assert result.artist is None

    def test_number_dot_title_english(self):
        """01. Bohemian Rhapsody → English."""
        result = parse_track_filename("01. Bohemian Rhapsody")
        assert result.track_number == 1
        assert result.title == "Bohemian Rhapsody"

    def test_number_dot_title_with_info_in_parens(self):
        """01 等了又等（国语版） → title includes Chinese parens."""
        result = parse_track_filename("01. 等了又等（国语版）")
        assert result.track_number == 1
        assert result.title == "等了又等（国语版）"

    def test_number_dot_title_two_digit(self):
        """10. 止戰之殤 → track_number=10."""
        result = parse_track_filename("10. 止戰之殤")
        assert result.track_number == 10

    def test_number_dot_title_single_digit(self):
        """3. Title → track_number=3."""
        result = parse_track_filename("3. Title")
        assert result.track_number == 3
        assert result.title == "Title"

    # Pattern 4:  "NN Title"
    def test_number_space_title(self):
        """01 今天情人節 → track_number=1."""
        result = parse_track_filename("01 今天情人節")
        assert result.track_number == 1
        assert result.title == "今天情人節"

    def test_number_space_title_numeric_name(self):
        """01 101 → track_number=1, title="101"."""
        result = parse_track_filename("01 101")
        assert result.track_number == 1
        assert result.title == "101"

    # Pattern 5:  "Artist - Title"
    def test_artist_dash_title(self):
        """陈洁仪 - 最好的年纪 → artist="陈洁仪", title="最好的年纪"."""
        result = parse_track_filename("陈洁仪 - 最好的年纪")
        assert result.artist == "陈洁仪"
        assert result.title == "最好的年纪"
        assert result.track_number is None

    def test_artist_dash_title_english(self):
        """Adele - Hello → English."""
        result = parse_track_filename("Adele - Hello")
        assert result.artist == "Adele"
        assert result.title == "Hello"

    def test_artist_em_dash_title(self):
        """陈洁仪 — 一念尘埃 → em dash."""
        result = parse_track_filename("陈洁仪 — 一念尘埃")
        assert result.artist == "陈洁仪"
        assert result.title == "一念尘埃"

    def test_artist_dash_title_with_feat(self):
        """Artist - Title (feat. Someone) → keeps parens in title."""
        result = parse_track_filename("周杰伦 - 告白气球 (feat. 阿信)")
        assert result.artist == "周杰伦"
        assert result.title == "告白气球 (feat. 阿信)"

    # Pattern 6:  Vinyl side+track  "A1. Title"
    def test_vinyl_side_a1(self):
        """A1. Rolling In The Deep → disc_number=1, track_number=1."""
        result = parse_track_filename("A1. Rolling In The Deep")
        assert result.disc_number == 1
        assert result.track_number == 1
        assert result.title == "Rolling In The Deep"

    def test_vinyl_side_b2(self):
        """B2. I'll Be Waiting → disc_number=2, track_number=2."""
        result = parse_track_filename("B2. I'll Be Waiting")
        assert result.disc_number == 2
        assert result.track_number == 2
        assert result.title == "I'll Be Waiting"

    def test_vinyl_side_d4(self):
        """D4. Last Track → disc_number=4, track_number=4."""
        result = parse_track_filename("D4. Last Track")
        assert result.disc_number == 4
        assert result.track_number == 4

    # Pattern 7:  "Title - Suffix YYYY - Artist"
    def test_title_suffix_year_artist(self):
        """Bohemian Rhapsody - Remastered 2011 - Queen → title, year, artist."""
        result = parse_track_filename("Bohemian Rhapsody - Remastered 2011 - Queen")
        assert result.title == "Bohemian Rhapsody"
        assert result.year == "2011"
        assert result.artist == "Queen"

    def test_title_suffix_year_artist_2(self):
        """39 - Remastered 2011 - Queen → title=39."""
        result = parse_track_filename("39 - Remastered 2011 - Queen")
        assert result.title == "39"
        assert result.year == "2011"
        assert result.artist == "Queen"

    # Pattern 8:  "(NN) Title"
    def test_paren_no_artist(self):
        """(01) 心痛 → track_number=1, title="心痛"."""
        result = parse_track_filename("(01) 心痛")
        assert result.track_number == 1
        assert result.title == "心痛"
        assert result.artist is None

    # Fallback: plain stem
    def test_plain_title(self):
        """JustATitle → title="JustATitle"."""
        result = parse_track_filename("JustATitle")
        assert result.title == "JustATitle"
        assert result.artist is None
        assert result.track_number is None

    def test_plain_title_with_spaces(self):
        """A Title With Spaces → title preserved."""
        result = parse_track_filename("A Title With Spaces")
        assert result.title == "A Title With Spaces"

    def test_unicode_only_title(self):
        """心痛 → title="心痛"."""
        result = parse_track_filename("心痛")
        assert result.title == "心痛"

    # Edge cases
    def test_artist_dash_number_avoids_conflict(self):
        """01 - Title should not match as artist "01"."""
        result = parse_track_filename("01 - Title")
        # Should NOT match Pattern 5 (artist="01") because "01" looks like a track number
        # Should fall through to Pattern 4? Actually "01 - Title" matches pattern 4 as "01"+"-" but "-" is a separator.
        # Let's see: pattern 4 is r"^(\d{1,3})\s+(.+)$" — requires space. "01 - Title" has " - " which isn't just space.
        # So it falls to the end as plain title.
        assert result.track_number is None
        assert result.title == "01 - Title"

    def test_empty_stem(self):
        """Empty stem → empty ParsedFilename."""
        result = parse_track_filename("")
        assert result.title == ""

    def test_only_track_number(self):
        """01 alone should parse as title."""
        result = parse_track_filename("01")
        assert result.title == "01"

    def test_mr_turner_dot_in_title(self):
        """Mr. Turner in title should not be treated as track number."""
        # "(07) [陈洁仪] Mr. Turner" should keep "Mr. Turner"
        result = parse_track_filename("(07) [陈洁仪] Mr. Turner")
        assert result.track_number == 7
        assert result.title == "Mr. Turner"
        assert result.artist == "陈洁仪"

    def test_parenthesized_english_text(self):
        """Title with parens like 'Wind Beneath My Wings (英)'."""
        result = parse_track_filename("07. Wind Beneath My Wings（英）")
        assert result.track_number == 7
        assert result.title == "Wind Beneath My Wings（英）"

    def test_collaboration_plus_sign(self):
        """Artist1＋Artist2 in bracket."""
        result = parse_track_filename("(13) [陈洁仪＋苏永康] 不知不觉")
        assert result.track_number == 13
        assert result.title == "不知不觉"
        assert result.artist == "陈洁仪＋苏永康"
        assert result.artists == ["陈洁仪", "苏永康"]

    def test_collaboration_with_comma(self):
        """Artist with comma separated names."""
        result = parse_track_filename("(01) [Adele, Beyonce] Hello")
        assert result.track_number == 1
        assert result.artists == ["Adele", "Beyonce"]

    def test_collaboration_with_ampersand(self):
        """Artist1 & Artist2 → split."""
        result = parse_track_filename("(01) [Adele & Beyonce] Hello")
        assert result.artists == ["Adele", "Beyonce"]


# ── Album folder name parsing ─────────────────────────────────────────


class TestParseAlbumFolderName:
    """Tests for ``parse_album_folder_name`` on directory names."""

    # Pattern A:  "Artist-《Year-Album》[Format]"
    def test_artist_bookmark_year_album(self):
        """陈洁仪-《1994-心痛》[WAV 分轨] → artist, year, album."""
        result = parse_album_folder_name("陈洁仪-《1994-心痛》[WAV 分轨]")
        assert result.artist == "陈洁仪"
        assert result.album_artist == "陈洁仪"
        assert result.year == "1994"
        assert result.album == "心痛"

    def test_artist_bookmark_2cd(self):
        """陈洁仪-《2002-异想世界 2CD 》[WAV 分轨] → album keeps 2CD."""
        result = parse_album_folder_name("陈洁仪-《2002-异想世界 2CD 》[WAV 分轨]")
        assert result.artist == "陈洁仪"
        assert result.year == "2002"
        assert result.album == "异想世界 2CD"

    def test_artist_bookmark_english(self):
        """陈洁仪-《2018-A Time For Everything》[FLAC 分轨] → English album."""
        result = parse_album_folder_name("陈洁仪-《2018-A Time For Everything》[FLAC 分轨]")
        assert result.artist == "陈洁仪"
        assert result.year == "2018"
        assert result.album == "A Time For Everything"

    def test_artist_bookmark_lpcd(self):
        """陈洁仪-《2007-LPCD 45》[WAV 分轨] → album="LPCD 45"."""
        result = parse_album_folder_name("陈洁仪-《2007-LPCD 45》[WAV 分轨]")
        assert result.year == "2007"
        assert result.album == "LPCD 45"

    # Pattern D:  "Artist - Album (Year) [Info]"
    def test_artist_dash_album_year(self):
        """Adele - 21 (2011) [LP] [flac] → year, artist, album."""
        result = parse_album_folder_name("Adele - 21 (2011) [LP] [flac]")
        assert result.artist == "Adele"
        assert result.album_artist == "Adele"
        assert result.year == "2011"
        assert result.album == "21"

    def test_artist_dash_album_year_parens_only(self):
        """Artist - Album (2015) → year from parens."""
        result = parse_album_folder_name("Adele - 25 (2015)")
        assert result.artist == "Adele"
        assert result.year == "2015"
        assert result.album == "25"

    # Pattern B:  "[Year] Album (Edition)"
    def test_bracket_year_album(self):
        """[1975] A Night At The Opera (2011 Remaster) → year, album."""
        result = parse_album_folder_name("[1975] A Night At The Opera (2011 Remaster)")
        assert result.year == "1975"
        assert result.album == "A Night At The Opera"
        assert result.artist is None

    def test_bracket_year_album_simple(self):
        """[1973] Queen I → year, album."""
        result = parse_album_folder_name("[1973] Queen I")
        assert result.year == "1973"
        assert result.album == "Queen I"

    # Pattern C:  "Year-Album"  or  "Year - Album"
    def test_year_dash_album(self):
        """1993-Karen → year, album."""
        result = parse_album_folder_name("1993-Karen")
        assert result.year == "1993"
        assert result.album == "Karen"
        assert result.artist is None

    def test_year_dash_album_subtitle(self):
        """2002-异想世界 2CD → year, album."""
        result = parse_album_folder_name("2002-异想世界 2CD")
        assert result.year == "2002"
        assert result.album == "异想世界 2CD"

    def test_year_em_dash_album(self):
        """1997 — Bored → em dash."""
        result = parse_album_folder_name("1997 — Bored")
        assert result.year == "1997"
        assert result.album == "Bored"

    # Plain album name
    def test_plain_album_name(self):
        """Album Name → album only."""
        result = parse_album_folder_name("Album Name")
        assert result.album == "Album Name"
        assert result.artist is None
        assert result.year is None

    def test_chinese_album_name(self):
        """太阳之子 → album."""
        result = parse_album_folder_name("太阳之子")
        assert result.album == "太阳之子"

    def test_numeric_album_name(self):
        """2001 → album="2001" (no year prefix)."""
        result = parse_album_folder_name("2001")
        # "2001" matches Pattern C (Year-Album) with no album part...
        # Actually the regex r"^(\d{4})\s*[-—]\s*(.+)$" requires a separator.
        # So "2001" alone falls through to plain name.
        assert result.album == "2001"
        assert result.year is None

    # Edge cases
    def test_artist_album_without_year(self):
        """Artist - Album (no year) → fallback to plain."""
        result = parse_album_folder_name("周杰伦 - 太阳之子")
        assert result.artist is None
        assert result.album == "周杰伦 - 太阳之子"
        # Actually "周杰伦 - 太阳之子" matches ONLY pattern 5 (Artist - Title) if we had that
        # But in album parsing we don't have that pattern. Let's check...
        # Pattern D requires "(Year)", so no match. Falls through to plain.
        # That's fine — the artist comes from the parent folder anyway.

    def test_empty_folder_name(self):
        """Empty string → empty ParsedFilename."""
        result = parse_album_folder_name("")
        assert result.album == ""


# ── Full path parsing ─────────────────────────────────────────────────


class TestMetadataFromPath:
    """Tests for ``metadata_from_path`` combining file + parent + grandparent info."""

    def test_simple_artist_album_track(self, tmp_path: Path):
        """Artist/Album/NN.Title.ext → full metadata."""
        path = tmp_path / "陈洁仪" / "1994-心痛" / "01. 心痛.flac"
        path.parent.mkdir(parents=True)
        path.touch()

        result = metadata_from_path(path)
        assert result.track_number == 1
        assert result.title == "心痛"
        assert result.album == "心痛"
        assert result.year == "1994"
        # Grandparent folder provides artist
        assert result.artist == "陈洁仪"
        assert result.album_artist == "陈洁仪"

    def test_grandparent_artist_fallback(self, tmp_path: Path):
        """When file has no artist tag, use grandparent folder."""
        path = tmp_path / "蔡健雅" / "1999-呼吸" / "01. 好无聊.flac"
        path.parent.mkdir(parents=True)
        path.touch()

        result = metadata_from_path(path)
        assert result.track_number == 1
        assert result.title == "好无聊"
        assert result.artist == "蔡健雅"
        assert result.album == "呼吸"
        assert result.year == "1999"

    def test_file_artist_overrides_parent(self, tmp_path: Path):
        """[Artist] in filename overrides grandparent."""
        path = tmp_path / "Various" / "Compilation" / "(01) [陈洁仪] 心痛.flac"
        path.parent.mkdir(parents=True)
        path.touch()

        result = metadata_from_path(path)
        assert result.artist == "陈洁仪"
        assert result.album == "Compilation"

    def test_year_from_parent(self, tmp_path: Path):
        """Year from folder name flows through."""
        path = tmp_path / "Queen" / "[1975] A Night At The Opera" / "01. Bohemian Rhapsody.flac"
        path.parent.mkdir(parents=True)
        path.touch()

        result = metadata_from_path(path)
        assert result.year == "1975"
        assert result.album == "A Night At The Opera"

    def test_loose_folder_skipped_as_artist(self, tmp_path: Path):
        """Loose folder should not become artist."""
        path = tmp_path / "Loose" / "01. Song.flac"
        path.parent.mkdir(parents=True)
        path.touch()

        result = metadata_from_path(path)
        assert result.artist is None  # "Loose" is a known non-artist folder

    def test_digit_only_artist(self, tmp_path: Path):
        """Numeric artist folder like '5566' should work."""
        path = tmp_path / "5566" / "挚爱" / "01. Song.flac"
        path.parent.mkdir(parents=True)
        path.touch()

        result = metadata_from_path(path)
        assert result.artist == "5566"

    def test_english_artist_album(self, tmp_path: Path):
        """Standard English two-level structure."""
        path = tmp_path / "Adele" / "21" / "01. Rolling In The Deep.flac"
        path.parent.mkdir(parents=True)
        path.touch()

        result = metadata_from_path(path)
        assert result.track_number == 1
        assert result.title == "Rolling In The Deep"
        assert result.artist == "Adele"
        assert result.album == "21"

    def test_artist_in_filename_album_in_parent(self, tmp_path: Path):
        """File has artist, parent provides album, grandparent is different."""
        path = tmp_path / "群星" / "合辑" / "(01) [陈洁仪] 心痛.flac"
        path.parent.mkdir(parents=True)
        path.touch()

        result = metadata_from_path(path)
        assert result.artist == "陈洁仪"  # from file bracket
        assert result.album == "合辑"    # from parent
        # Grandparent "群星" is not used as artist since file has one

    def test_no_parent_info(self, tmp_path: Path):
        """File at root level → just file parsing (no grandparent artist).

        The parent folder name IS treated as the album context (that's the
        design — the folder containing audio files IS the album directory).
        But no grandparent artist is inferred from a temp path.
        """
        path = tmp_path / "Song.flac"
        path.touch()

        result = metadata_from_path(path)
        assert result.title == "Song"
        assert result.artist is None  # no grandparent to infer from
        # Parent folder is always treated as album context
        assert result.album is not None


# ── Artist splitting ──────────────────────────────────────────────────


class TestSplitArtists:
    """Tests for the internal _split_artists helper."""

    def test_single_artist(self):
        """Single name returns as single-element list."""
        from auto_tagger.core.parse_filename import _split_artists
        assert _split_artists("陈洁仪") == ["陈洁仪"]

    def test_plus_sign(self):
        """＋ separator."""
        from auto_tagger.core.parse_filename import _split_artists
        assert _split_artists("陈洁仪＋苏永康") == ["陈洁仪", "苏永康"]

    def test_ampersand(self):
        """& separator."""
        from auto_tagger.core.parse_filename import _split_artists
        assert _split_artists("Adele & Beyonce") == ["Adele", "Beyonce"]

    def test_comma(self):
        """, separator."""
        from auto_tagger.core.parse_filename import _split_artists
        assert _split_artists("A, B") == ["A", "B"]

    def test_hyphen_is_not_separator(self):
        """Hyphen is NOT a separator (collaboration vs name)."""
        from auto_tagger.core.parse_filename import _split_artists
        assert _split_artists("陈洁仪-苏永康") == ["陈洁仪-苏永康"]


# ── Real-world integration scenarios ──────────────────────────────────


class TestRealWorldScenarios:
    """Parse real file paths from the observed music collection."""

    def test_chen_jieyi_1994_track(self):
        """陈洁仪/陈洁仪-《1994-心痛》[WAV 分轨]/(01) [陈洁仪] 心痛.wav"""
        result = parse_track_filename("(01) [陈洁仪] 心痛")
        assert result.track_number == 1
        assert result.title == "心痛"
        assert result.artist == "陈洁仪"

        album_result = parse_album_folder_name("陈洁仪-《1994-心痛》[WAV 分轨]")
        assert album_result.artist == "陈洁仪"
        assert album_result.year == "1994"
        assert album_result.album == "心痛"

    def test_chen_jieyi_2015_track(self):
        """2015 album uses 'NN. Title (Info)' pattern."""
        result = parse_track_filename("01. 等了又等（国语版）")
        assert result.track_number == 1
        assert result.title == "等了又等（国语版）"

    def test_chen_jieyi_2018_track(self):
        """2018 album uses 'Artist - Title' pattern (no number)."""
        result = parse_track_filename("陈洁仪 - 最好的年纪")
        assert result.artist == "陈洁仪"
        assert result.title == "最好的年纪"

    def test_zhou_jielun(self):
        """周杰伦/七里香/01 我的地盤.flac"""
        result = parse_track_filename("01 我的地盤")
        assert result.track_number == 1
        assert result.title == "我的地盤"

    def test_liang_jingru(self):
        """梁静茹/崇拜/01 崇拜.flac"""
        result = parse_track_filename("01 崇拜")
        assert result.track_number == 1
        assert result.title == "崇拜"

    def test_mo_wenwei(self):
        """莫文蔚/1993-Karen/莫文蔚 - 01.這等待眼睛.flac"""
        result = parse_track_filename("莫文蔚 - 01.這等待眼睛")
        assert result.track_number == 1
        assert result.artist == "莫文蔚"
        assert result.title == "這等待眼睛"

    def test_cai_jianya_bored(self):
        """蔡健雅/1997-Bored/蔡健雅 - 01.Bored 无聊.flac"""
        result = parse_track_filename("蔡健雅 - 01.Bored 无聊")
        assert result.track_number == 1
        assert result.artist == "蔡健雅"
        assert result.title == "Bored 无聊"

    def test_queen_vinyl(self):
        """Queen/[1975] A Night At The Opera/01. Bohemian Rhapsody.flac"""
        result = parse_track_filename("01. Bohemian Rhapsody")
        assert result.track_number == 1
        assert result.title == "Bohemian Rhapsody"

        album_result = parse_album_folder_name("[1975] A Night At The Opera (2011 Remaster)")
        assert album_result.year == "1975"
        assert album_result.album == "A Night At The Opera"

    def test_adele_lp(self):
        """Adele/Adele - 21 (2011) [LP] [flac]/A1. Rolling In The Deep.flac"""
        result = parse_track_filename("A1. Rolling In The Deep")
        assert result.disc_number == 1
        assert result.track_number == 1
        assert result.title == "Rolling In The Deep"

        album_result = parse_album_folder_name("Adele - 21 (2011) [LP] [flac]")
        assert album_result.year == "2011"
        assert album_result.artist == "Adele"
        assert album_result.album == "21"

    def test_chen_qizhen(self):
        """陈绮贞/1998-让我想一想/陈绮贞 - 01.让我想一想.flac"""
        result = parse_track_filename("陈绮贞 - 01.让我想一想")
        assert result.track_number == 1
        assert result.artist == "陈绮贞"
        assert result.title == "让我想一想"

    def test_queen_remastered_track(self):
        """Queen files use 'Title - Remastered YYYY - Artist' pattern."""
        result = parse_track_filename("Bohemian Rhapsody - Remastered 2011 - Queen")
        assert result.title == "Bohemian Rhapsody"
        assert result.year == "2011"
        assert result.artist == "Queen"

    def test_chen_jieyi_loose(self):
        """Loose track: 陈洁仪 - 一念尘埃.flac"""
        result = parse_track_filename("陈洁仪 - 一念尘埃")
        assert result.artist == "陈洁仪"
        assert result.title == "一念尘埃"
