"""LLM fallback tag generation service."""

from __future__ import annotations

from dataclasses import dataclass, field

from auto_tagger.config import Settings
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.integrations.candidates import AlbumCandidate, LookupRequest, LookupSource
from auto_tagger.llm.cost import CostEstimate, estimate_cost
from auto_tagger.llm.prompts import build_fallback_messages
from auto_tagger.llm.schemas import FallbackTagResponse
from auto_tagger.llm.types import JsonLLMClient


@dataclass(frozen=True)
class FallbackGenerationResult:
    """Generated fallback metadata result."""

    tracks: list[TrackMetadata] = field(default_factory=list)
    confidence: float = 0.0
    reason: str = ""
    cost_estimate: CostEstimate | None = None


class FallbackTagGenerationService:
    """Generate conservative fallback tags from folder/current metadata hints."""

    def __init__(self, client: JsonLLMClient, settings: Settings):
        self.client = client
        self.settings = settings

    def generate_tags(
        self,
        request: LookupRequest,
        folder_candidate: AlbumCandidate,
        current_metadata: list[TrackMetadata],
    ) -> FallbackGenerationResult:
        """Generate fallback tags for folder-source candidates."""
        if folder_candidate.source is not LookupSource.FOLDER:
            return FallbackGenerationResult(
                reason="Fallback generation requires a folder-source candidate"
            )

        response = self.client.complete_json(
            build_fallback_messages(request, folder_candidate, current_metadata),
            FallbackTagResponse,
        )
        parsed = FallbackTagResponse.model_validate(response.data)
        tracks = [
            TrackMetadata(
                title=track.title,
                artist=track.artist or parsed.artist,
                artists=track.artists or parsed.artists,
                album=track.album or parsed.album,
                album_artist=track.album_artist or parsed.album_artist,
                album_artists=parsed.album_artists,
                track_number=track.track_number,
                disc_number=track.disc_number,
                year=parsed.year,
                genre=parsed.genre,
            )
            for track in parsed.tracks
        ]
        cost = estimate_cost(
            response.usage,
            response.model,
            self.settings.llm_cost_per_1k_prompt_tokens,
            self.settings.llm_cost_per_1k_completion_tokens,
        )
        return FallbackGenerationResult(tracks, parsed.confidence, parsed.reason, cost)
