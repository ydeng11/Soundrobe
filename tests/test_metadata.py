"""Tests for normalized metadata models."""

from auto_tagger.core.metadata import ReplayGainTags, TrackMetadata, format_position, parse_position


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
        artist="Alice feat. Bob",
        album_artist="Alice",
        replaygain=ReplayGainTags(track_gain="-6.84 dB"),
    )

    normalized = metadata.normalized()

    assert normalized.artists == ["Alice feat. Bob"]
    assert normalized.album_artists == ["Alice"]
    assert normalized.replaygain.track_gain == "-6.84 dB"


def test_metadata_to_display_rows_skips_empty_fields():
    """Display rows include populated fields and omit empty values."""
    metadata = TrackMetadata(title="Song", album="Album", track_number=1, track_total=12)

    assert metadata.to_display_rows() == [
        ["title", "Song"],
        ["album", "Album"],
        ["track", "1/12"],
    ]
