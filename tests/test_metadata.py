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


def test_normalized_splits_album_artists_too():
    """normalized() splits album_artists with separators."""
    metadata = TrackMetadata(
        title="Song",
        album_artist="Alice / Bob",
    )
    normalized = metadata.normalized()
    assert normalized.album_artists == ["Alice", "Bob"]


def test_metadata_to_display_rows_skips_empty_fields():
    """Display rows include populated fields and omit empty values."""
    metadata = TrackMetadata(title="Song", album="Album", track_number=1, track_total=12)

    assert metadata.to_display_rows() == [
        ["title", "Song"],
        ["album", "Album"],
        ["track", "1/12"],
    ]
