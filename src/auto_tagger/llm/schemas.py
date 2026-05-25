"""Pydantic schemas for structured LLM responses."""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel, ConfigDict, Field, model_validator


class CandidateSelectionResponse(BaseModel):
    """Structured response for candidate selection."""

    model_config = ConfigDict(extra="forbid")

    selected_index: int | None = None
    confidence: float = Field(ge=0.0, le=1.0)
    reason: str

    def validate_candidate_count(self, candidate_count: int) -> None:
        """Validate selected index against number of candidates."""
        if self.selected_index is None:
            return
        if self.selected_index < 0 or self.selected_index >= candidate_count:
            raise ValueError(f"selected_index out of range for {candidate_count} candidates")


class GeneratedTrackTags(BaseModel):
    """Generated tags for one track."""

    model_config = ConfigDict(extra="forbid")

    title: str
    artist: str | None = None
    artists: list[str] = Field(default_factory=list)
    album: str | None = None
    album_artist: str | None = None
    track_number: int | None = Field(default=None, gt=0)
    disc_number: int | None = Field(default=None, gt=0)


class GenreEnrichmentResponse(BaseModel):
    """Structured response for genre enrichment via LLM.

    Returns a Discogs-style genre string (e.g. 'Electronic, Ambient, Modern Classical')
    or None when uncertain.
    """

    model_config = ConfigDict(extra="forbid")

    genre: str | None = None


class FolderExtractionResponse(BaseModel):
    """Structured response for folder name metadata extraction."""

    model_config = ConfigDict(extra="forbid")

    artist: str | None = None
    album: str | None = None
    year: str | None = None
    disc: str | None = None


class FallbackTagResponse(BaseModel):
    """Structured response for fallback tag generation."""

    model_config = ConfigDict(extra="forbid")

    artist: str
    artists: list[str] = Field(default_factory=list)
    album: str
    album_artist: str
    album_artists: list[str] = Field(default_factory=list)
    year: str | None = None
    genre: str | None = None
    tracks: list[GeneratedTrackTags]
    confidence: float = Field(ge=0.0, le=1.0)
    reason: str

    @model_validator(mode="before")
    @classmethod
    def reject_musicbrainz_ids(cls, data: Any) -> Any:
        """Reject generated MusicBrainz fields."""
        if isinstance(data, dict):
            invented = [key for key in data if key.lower().startswith("musicbrainz")]
            if invented:
                raise ValueError("fallback output must not include MusicBrainz IDs")
        return data


class CorrectedTrack(BaseModel):
    """Complete corrected metadata for one track returned by LLM audit.

    Only populated for tracks with issues (warning/error). The code writes
    these values verbatim — no field mapping or transformation.
    """

    model_config = ConfigDict(extra="forbid")

    title: str | None = None
    artist: str | None = None
    artists: list[str] | None = None
    album: str | None = None
    album_artist: str | None = None
    year: str | None = None
    genre: str | None = None


class AuditTrackResult(BaseModel):
    """LLM audit result for a single track field."""

    model_config = ConfigDict(extra="forbid")

    index: int = Field(..., ge=0, description="Track index within the album")
    field: str = Field(..., description="Audited field name (artist, title, album, album_artist, artists, path)")
    status: str = Field(..., description="'correct' | 'warning' | 'error'")
    message: str = Field(..., description="Human-readable explanation")
    suggestion: str | None = Field(default=None, description="Suggested fix value (deprecated in favor of corrected)")
    corrected: CorrectedTrack | None = Field(default=None, description="Complete corrected metadata for problematic tracks")


class AuditResponse(BaseModel):
    """Structured response for LLM album audit."""

    model_config = ConfigDict(extra="forbid")

    tracks: list[AuditTrackResult] = Field(..., description="Per-track audit results")

