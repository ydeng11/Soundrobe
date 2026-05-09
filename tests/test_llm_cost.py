"""Tests for LLM token usage and cost estimates."""


def test_estimate_cost_from_token_usage():
    """Cost estimates split prompt and completion costs."""
    from auto_tagger.llm.cost import TokenUsage, estimate_cost

    estimate = estimate_cost(
        TokenUsage(prompt_tokens=1000, completion_tokens=500, total_tokens=1500),
        model="test/model",
        prompt_rate=0.001,
        completion_rate=0.002,
    )

    assert estimate.prompt_cost == 0.001
    assert estimate.completion_cost == 0.001
    assert estimate.total_cost == 0.002
    assert estimate.model == "test/model"


def test_cost_summary_aggregates_usage_and_cost():
    """Cost summaries aggregate token usage and estimated spend."""
    from auto_tagger.llm.cost import CostEstimate, TokenUsage, summarize_costs

    summary = summarize_costs(
        [
            CostEstimate(
                model="a",
                usage=TokenUsage(prompt_tokens=10, completion_tokens=5, total_tokens=15),
                prompt_cost=0.01,
                completion_cost=0.02,
            ),
            CostEstimate(
                model="a",
                usage=TokenUsage(prompt_tokens=20, completion_tokens=10, total_tokens=30),
                prompt_cost=0.03,
                completion_cost=0.04,
            ),
        ]
    )

    assert summary.call_count == 2
    assert summary.usage.total_tokens == 45
    assert summary.total_cost == 0.10
