"""Shared LLM service typing helpers."""

from __future__ import annotations

from typing import Protocol

from pydantic import BaseModel

from auto_tagger.llm.client import LLMResponse


class JsonLLMClient(Protocol):
    """Protocol for clients that return structured JSON LLM responses."""

    def complete_json(
        self,
        messages: list[dict[str, str]],
        schema: type[BaseModel],
        *,
        model: str | None = None,
    ) -> LLMResponse:
        """Return a parsed LLM JSON response."""
