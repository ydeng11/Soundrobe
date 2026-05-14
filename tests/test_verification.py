"""Tests for album name verification."""

from auto_tagger.integrations.candidates import (
    AlbumCandidate,
    LookupSource,
    verify_album_name,
)


def test_verify_match_same_name():
    """Identical names after normalization produce 'match'."""
    candidate = AlbumCandidate(album="2001", source=LookupSource.DISCOGS)
    assert verify_album_name("2001", candidate) == "match"


def test_verify_match_different_case():
    """Case differences are normalized to 'match'."""
    candidate = AlbumCandidate(album="The Album", source=LookupSource.BEETS)
    assert verify_album_name("the album", candidate) == "match"


def test_verify_match_ignores_punctuation():
    """Punctuation differences are normalized away."""
    candidate = AlbumCandidate(album="Album!", source=LookupSource.BEETS)
    assert verify_album_name("Album", candidate) == "match"
    assert verify_album_name("Album!", candidate) == "match"


def test_verify_match_chinese():
    """Chinese characters match when identical."""
    candidate = AlbumCandidate(album="挚爱", source=LookupSource.DATASET)
    assert verify_album_name("挚爱", candidate) == "match"


def test_verify_close_contains():
    """When one name contains the other, it's 'close'."""
    candidate = AlbumCandidate(album="2001 (Instrumental)", source=LookupSource.DISCOGS)
    assert verify_album_name("2001", candidate) == "close"
    assert verify_album_name("2001 (Instrumental)", candidate) == "match"


def test_verify_mismatch_different():
    """Completely different names produce 'mismatch'."""
    candidate = AlbumCandidate(album="2001", source=LookupSource.DISCOGS)
    assert verify_album_name("The Chronic", candidate) == "mismatch"


def test_verify_none_hint_or_album():
    """None hint or None album defaults to 'match' (can't verify)."""
    candidate = AlbumCandidate(album=None, source=LookupSource.FOLDER)
    assert verify_album_name("Something", candidate) == "match"
    assert verify_album_name(None, AlbumCandidate(album="Something")) == "match"


def test_verify_digit_album_name():
    """Digit-only album names are handled correctly."""
    # Matching digit name
    candidate = AlbumCandidate(album="2001", source=LookupSource.DISCOGS)
    assert verify_album_name("2001", candidate) == "match"

    # Close digit name (one contains the other)
    candidate2 = AlbumCandidate(album="2112", source=LookupSource.DISCOGS)
    assert verify_album_name("2001", candidate2) == "mismatch"


def test_verify_5566_album():
    """5566 album verification scenarios."""
    candidate = AlbumCandidate(album="挚爱", source=LookupSource.DISCOGS)
    assert verify_album_name("挚爱", candidate) == "match"

    # Folder name with date prefix cleaned by clean_folder_name
    assert verify_album_name("挚爱", candidate) == "match"

    # Close match
    candidate2 = AlbumCandidate(album="挚爱 (First Album)", source=LookupSource.DISCOGS)
    assert verify_album_name("挚爱", candidate2) == "close"
