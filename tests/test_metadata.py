"""Tests for normalized metadata models."""

from auto_tagger.core.metadata import (
    ReplayGainTags,
    TrackMetadata,
    format_position,
    parse_position,
    split_artist_strings,
)


def test_parse_position_supports_value_and_total():
    """Track and disc numbers parse both simple and total forms."""
    assert parse_position("7") == (7, None)
    assert parse_position("7/12") == (7, 12)
    assert parse_position(["3/9"]) == (3, 9)
    assert parse_position(None) == (None, None)


def test_format_position_omits_missing_values():
    """Track and disc numbers serialize without inventing totals."""
    assert format_position(2, None) == "2"
    assert format_position(2, 10) == "2/10"
    assert format_position(None, 10) is None


def test_metadata_defaults_multi_values_from_display_fields():
    """Missing multi-value fields fall back to display artist fields."""
    metadata = TrackMetadata(
        title="Song",
        artist="Alice",
        album_artist="Alice",
        replaygain=ReplayGainTags(track_gain="-6.84 dB"),
    )

    normalized = metadata.normalized()

    assert normalized.artists == ["Alice"]
    assert normalized.album_artists == ["Alice"]
    assert normalized.replaygain.track_gain == "-6.84 dB"


# ── split_artist_strings ────────────────────────────────────────


def test_split_artist_strings_empty_list():
    """An empty list returns an empty list."""
    assert split_artist_strings([]) == []


def test_split_artist_strings_single_artist_unchanged():
    """A single artist name without separators is returned unchanged."""
    assert split_artist_strings(["Alice"]) == ["Alice"]


def test_split_artist_strings_dot_separator():
    """A bare dot between CJK characters is treated as a separator."""
    assert split_artist_strings(["\u9673\u6167\u73b3.\u9673\u5c0f\u6625"]) == [
        "\u9673\u6167\u73b3",
        "\u9673\u5c0f\u6625",
    ]


def test_split_artist_strings_dot_ascii_not_split():
    """A dot within ASCII text (e.g. "Mr. Bungle") is NOT split."""
    assert split_artist_strings(["Mr. Bungle"]) == ["Mr. Bungle"]


def test_split_artist_strings_feat_separator():
    """"feat." with spaces splits the string."""
    assert split_artist_strings(["Alice feat. Bob"]) == ["Alice", "Bob"]


def test_split_artist_strings_ft_separator():
    """"ft." with spaces splits the string."""
    assert split_artist_strings(["Alice ft. Bob"]) == ["Alice", "Bob"]


def test_split_artist_strings_featuring_separator():
    """"featuring" with spaces splits the string."""
    assert split_artist_strings(["Alice featuring Bob"]) == ["Alice", "Bob"]


def test_split_artist_strings_slash_separator():
    """" / " with spaces splits the string."""
    assert split_artist_strings(["Alice / Bob"]) == ["Alice", "Bob"]


def test_split_artist_strings_ampersand_separator():
    """" & " with spaces splits the string."""
    assert split_artist_strings(["Alice & Bob"]) == ["Alice", "Bob"]


def test_split_artist_strings_plus_separator():
    """"+" and "＋" (fullwidth) split the string."""
    assert split_artist_strings(["Alice+Bob"]) == ["Alice", "Bob"]
    assert split_artist_strings(["Alice\uff0bBob"]) == ["Alice", "Bob"]


def test_split_artist_strings_chinese_comma():
    """Chinese enumeration comma splits the string."""
    assert split_artist_strings(["Alice\u3001Bob"]) == ["Alice", "Bob"]


def test_split_artist_strings_middle_dot():
    """Middle dot variants split the string."""
    assert split_artist_strings(["Alice\u00b7Bob"]) == ["Alice", "Bob"]
    assert split_artist_strings(["Alice\u2027Bob"]) == ["Alice", "Bob"]


def test_split_artist_strings_multiple_entries():
    """A list with multiple entries splits each as needed."""
    assert split_artist_strings(["Alice / Bob", "Charlie"]) == [
        "Alice", "Bob", "Charlie",
    ]


def test_split_artist_strings_whitespace_stripped():
    """Leading/trailing whitespace is stripped from each part."""
    assert split_artist_strings(["  Alice feat. Bob  "]) == ["Alice", "Bob"]


def test_split_artist_strings_no_separator_returns_single():
    """A single artist string without any separator returns as-is."""
    assert split_artist_strings(["The Beatles"]) == ["The Beatles"]
    assert split_artist_strings(["Bj\u00f6rk"]) == ["Bj\u00f6rk"]
    assert split_artist_strings(["ABBA"]) == ["ABBA"]


def test_normalized_splits_artists_with_feat():
    """normalized() splits artist strings with known separators."""
    metadata = TrackMetadata(
        title="Song",
        artist="Alice feat. Bob",
        album_artist="Alice",
    )
    normalized = metadata.normalized()
    assert normalized.artists == ["Alice", "Bob"]
    assert normalized.album_artists == ["Alice"]


def test_normalized_splits_artists_list_value():
    """normalized() splits entries already in the artists list."""
    metadata = TrackMetadata(
        title="Song",
        artists=["\u9673\u6167\u73b3.\u9673\u5c0f\u6625"],
    )
    normalized = metadata.normalized()
    assert normalized.artists == ["\u9673\u6167\u73b3", "\u9673\u5c0f\u6625"]


def test_normalized_does_not_split_ascii_dots():
    """normalized() does NOT split ASCII dots like "Mr. Bungle"."""
    metadata = TrackMetadata(
        title="Retrovertigo",
        artist="Mr. Bungle",
    )
    normalized = metadata.normalized()
    assert normalized.artists == ["Mr. Bungle"]


def test_normalized_splits_chinese_slash_artist():
    """normalized() splits Chinese slash-separated collaborative artists."""
    metadata = TrackMetadata(
        title="古古惑惑(清清楚楚系我)",
        artist="陈小春/郑伊健",
        artists=[],
    )
    normalized = metadata.normalized()
    # "陈小春/郑伊健" has 2 singers → artists should have 2 entries
    assert len(normalized.artists) == 2
    assert normalized.artists == ["陈小春", "郑伊健"]
    assert normalized.artist == "陈小春/郑伊健"


def test_normalized_splits_ampersand_artist():
    """normalized() splits ' & '-separated collaborative artists, strips spaces."""
    metadata = TrackMetadata(
        title="Under Pressure",
        artist="Queen & David Bowie",
        artists=[],
    )
    normalized = metadata.normalized()
    # "Queen & David Bowie" has 2 singers → 2 ARTISTS entries
    assert len(normalized.artists) == 2
    assert normalized.artists == ["Queen", "David Bowie"]
    assert normalized.artist == "Queen & David Bowie"


def test_normalized_splits_cjk_ampersand_no_spaces():
    """normalized() splits & between CJK chars even without spaces."""
    metadata = TrackMetadata(
        title="一起飞",
        artist="郑伊健&陈小春&林晓峰",
        artists=[],
    )
    normalized = metadata.normalized()
    # "郑伊健&陈小春&林晓峰" has 3 singers → 3 ARTISTS entries
    assert len(normalized.artists) == 3
    assert normalized.artists == ["郑伊健", "陈小春", "林晓峰"]
    assert normalized.artist == "郑伊健&陈小春&林晓峰"
    # Non-CJK "&" without spaces is NOT split (e.g. R&B)
    assert split_artist_strings(["R&B"]) == ["R&B"]


def test_normalized_splits_album_artists_too():
    """normalized() splits album_artists with separators."""
    metadata = TrackMetadata(
        title="Song",
        album_artist="Alice / Bob",
    )
    normalized = metadata.normalized()
    assert normalized.album_artists == ["Alice", "Bob"]


def test_write_metadata_normalizes_collaborative_artist(tmp_path):
    """write_metadata() splits collaborative artists via normalized()."""
    audio = tmp_path / "01-古古惑惑.flac"
    audio.touch()

    meta = TrackMetadata(
        title="古古惑惑(清清楚楚系我)",
        artist="陈小春/郑伊健",
        artists=[],
        album="友情岁月",
        album_artist="陈小春",
    )

    from auto_tagger.core.writer import write_metadata
    from unittest.mock import patch

    with patch("auto_tagger.core.writer.load_audio_file") as mock_load, \
         patch("auto_tagger.core.writer.write_tags") as mock_write:
        mock_af = __import__("unittest").mock.MagicMock()
        mock_load.return_value = mock_af

        result = write_metadata(audio, meta, dry_run=False)

    # normalized() splits the artist before writing
    # "陈小春/郑伊健" = 2 singers → 2 ARTISTS entries
    assert len(result.artists) == 2
    assert result.artists == ["陈小春", "郑伊健"]
    assert result.artist == "陈小春/郑伊健"
    mock_write.assert_called_once()
    # write_tags receives the normalized metadata
    written_meta = mock_write.call_args[0][2]
    assert len(written_meta.artists) == 2
    assert written_meta.artists == ["陈小春", "郑伊健"]


def test_metadata_to_display_rows_skips_empty_fields():
    """Display rows include populated fields and omit empty values."""
    metadata = TrackMetadata(title="Song", album="Album", track_number=1, track_total=12)

    assert metadata.to_display_rows() == [
        ["title", "Song"],
        ["album", "Album"],
        ["track", "1/12"],
    ]
