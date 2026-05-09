"""Tests for LLM response schemas."""

import pytest
from pydantic import ValidationError


def test_candidate_selection_response_validates_candidate_range():
    """Selection response rejects out-of-range candidate indexes."""
    from auto_tagger.llm.schemas import CandidateSelectionResponse

    response = CandidateSelectionResponse(selected_index=1, confidence=0.8, reason="best")
    response.validate_candidate_count(2)

    with pytest.raises(ValueError, match="out of range"):
        response.validate_candidate_count(1)


def test_candidate_selection_response_allows_none_selection():
    """Selection can explicitly reject all candidates."""
    from auto_tagger.llm.schemas import CandidateSelectionResponse

    response = CandidateSelectionResponse(selected_index=None, confidence=0.2, reason="poor")

    response.validate_candidate_count(0)


def test_fallback_tag_response_rejects_musicbrainz_ids_and_bad_track_numbers():
    """Generated fallback tags cannot invent MusicBrainz IDs or invalid tracks."""
    from auto_tagger.llm.schemas import FallbackTagResponse

    with pytest.raises(ValidationError):
        FallbackTagResponse(
            artist="Artist",
            album="Album",
            album_artist="Artist",
            tracks=[{"title": "Song", "track_number": 0}],
            confidence=0.8,
            reason="parsed",
        )

    with pytest.raises(ValidationError):
        FallbackTagResponse(
            artist="Artist",
            album="Album",
            album_artist="Artist",
            musicbrainz_albumid="invented",
            tracks=[{"title": "Song", "track_number": 1}],
            confidence=0.8,
            reason="parsed",
        )
