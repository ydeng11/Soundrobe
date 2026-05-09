"""LLM candidate selection service."""

from __future__ import annotations

from dataclasses import dataclass

from auto_tagger.config import Settings
from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest, LookupSource
from auto_tagger.llm.cost import CostEstimate, estimate_cost
from auto_tagger.llm.prompts import build_selection_messages
from auto_tagger.llm.schemas import CandidateSelectionResponse
from auto_tagger.llm.types import JsonLLMClient


@dataclass(frozen=True)
class SelectionResult:
    """Result of selecting a candidate."""

    selected_candidate: AlbumCandidate | None
    confidence: float
    reason: str
    cost_estimate: CostEstimate | None = None


class CandidateSelectionService:
    """Use LLM responses to select among lookup candidates."""

    def __init__(self, client: JsonLLMClient, settings: Settings):
        self.client = client
        self.settings = settings

    def select_candidate(
        self,
        request: LookupRequest,
        candidates: list[AlbumCandidate],
    ) -> SelectionResult:
        """Select the best candidate, or return no selection."""
        if not candidates:
            return SelectionResult(None, 0.0, "No candidates")

        if _is_single_high_confidence_beets_candidate(candidates):
            return SelectionResult(candidates[0], 1.0, "Single high-confidence Beets candidate")

        response = self.client.complete_json(
            build_selection_messages(request, candidates, self.settings.llm_max_candidates),
            CandidateSelectionResponse,
        )
        parsed = CandidateSelectionResponse.model_validate(response.data)
        try:
            parsed.validate_candidate_count(len(candidates))
        except ValueError as exc:
            return SelectionResult(None, parsed.confidence, str(exc))

        selected = candidates[parsed.selected_index] if parsed.selected_index is not None else None
        cost = estimate_cost(
            response.usage,
            response.model,
            self.settings.llm_cost_per_1k_prompt_tokens,
            self.settings.llm_cost_per_1k_completion_tokens,
        )
        return SelectionResult(selected, parsed.confidence, parsed.reason, cost)


def _is_single_high_confidence_beets_candidate(candidates: list[AlbumCandidate]) -> bool:
    if len(candidates) != 1:
        return False
    candidate = candidates[0]
    return (
        candidate.source is LookupSource.BEETS
        and candidate.distance is not None
        and candidate.distance <= 0.05
    )
