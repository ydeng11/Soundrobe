"""LLM token usage and cost estimation."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class TokenUsage:
    """Token counts reported by the LLM provider."""

    prompt_tokens: int = 0
    completion_tokens: int = 0
    total_tokens: int = 0


@dataclass(frozen=True)
class CostEstimate:
    """Estimated cost for one LLM call."""

    model: str
    usage: TokenUsage
    prompt_cost: float = 0.0
    completion_cost: float = 0.0

    @property
    def total_cost(self) -> float:
        """Return total estimated cost."""
        return self.prompt_cost + self.completion_cost


@dataclass(frozen=True)
class CostSummary:
    """Aggregated LLM usage/cost summary."""

    call_count: int
    usage: TokenUsage
    total_cost: float


def estimate_cost(
    usage: TokenUsage,
    model: str,
    prompt_rate: float,
    completion_rate: float,
) -> CostEstimate:
    """Estimate cost from usage and per-1K-token rates."""
    return CostEstimate(
        model=model,
        usage=usage,
        prompt_cost=(usage.prompt_tokens / 1000) * prompt_rate,
        completion_cost=(usage.completion_tokens / 1000) * completion_rate,
    )


def summarize_costs(estimates: list[CostEstimate]) -> CostSummary:
    """Aggregate cost estimates into a summary."""
    usage = TokenUsage(
        prompt_tokens=sum(estimate.usage.prompt_tokens for estimate in estimates),
        completion_tokens=sum(estimate.usage.completion_tokens for estimate in estimates),
        total_tokens=sum(estimate.usage.total_tokens for estimate in estimates),
    )
    return CostSummary(
        call_count=len(estimates),
        usage=usage,
        total_cost=sum(estimate.total_cost for estimate in estimates),
    )
