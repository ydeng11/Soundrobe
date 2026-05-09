"""Tests for LLM candidate selection service."""

from pathlib import Path


class FakeLLMClient:
    """LLM client double returning configured JSON."""

    def __init__(self, data):
        from auto_tagger.llm.cost import TokenUsage

        self.data = data
        self.calls = []
        self.usage = TokenUsage(prompt_tokens=10, completion_tokens=5, total_tokens=15)

    def complete_json(self, messages, schema, model=None):
        from auto_tagger.llm.client import LLMResponse

        self.calls.append(messages)
        return LLMResponse(data=self.data, usage=self.usage, model=model or "test/model")


def test_selection_service_selects_candidate_from_llm_response():
    """Selection service returns the candidate selected by the LLM."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest, LookupSource
    from auto_tagger.llm.selection import CandidateSelectionService

    candidates = [
        AlbumCandidate(artist="A", album="Wrong", source=LookupSource.BEETS, distance=0.4),
        AlbumCandidate(artist="A", album="Right", source=LookupSource.BEETS, distance=0.2),
    ]
    service = CandidateSelectionService(
        FakeLLMClient({"selected_index": 1, "confidence": 0.88, "reason": "closest"}),
        Settings(llm_api_key="key"),
    )

    result = service.select_candidate(LookupRequest(path=Path("/music/A/B")), candidates)

    assert result.selected_candidate == candidates[1]
    assert result.confidence == 0.88
    assert result.cost_estimate is not None


def test_selection_service_skips_single_high_confidence_beets_candidate():
    """Obvious single Beets candidates do not spend LLM tokens."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest, LookupSource
    from auto_tagger.llm.selection import CandidateSelectionService

    candidate = AlbumCandidate(artist="A", album="Album", source=LookupSource.BEETS, distance=0.01)
    client = FakeLLMClient({"selected_index": 0, "confidence": 0.9, "reason": "unused"})

    result = CandidateSelectionService(client, Settings(llm_api_key="key")).select_candidate(
        LookupRequest(path=Path("/music/A/B")),
        [candidate],
    )

    assert result.selected_candidate == candidate
    assert client.calls == []


def test_selection_service_rejects_out_of_range_selection():
    """Out-of-range selected indexes produce no selected candidate."""
    from auto_tagger.config import Settings
    from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest
    from auto_tagger.llm.selection import CandidateSelectionService

    result = CandidateSelectionService(
        FakeLLMClient({"selected_index": 5, "confidence": 0.9, "reason": "bad"}),
        Settings(llm_api_key="key"),
    ).select_candidate(
        LookupRequest(path=Path("/music/A/B")),
        [AlbumCandidate(artist="A", album="Album")],
    )

    assert result.selected_candidate is None
    assert "out of range" in result.reason
