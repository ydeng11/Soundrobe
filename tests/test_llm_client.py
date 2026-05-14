"""Tests for OpenRouter client request and response handling."""

import json

import pytest


class FakeResponse:
    """Small HTTP response double."""

    def __init__(self, status_code=200, payload=None, text=""):
        self.status_code = status_code
        self._payload = payload or {}
        self.text = text

    def json(self):
        return self._payload


class FakeHttpClient:
    """Small HTTP client double."""

    def __init__(self, responses):
        self.responses = list(responses)
        self.requests = []

    def post(self, url, headers=None, json=None, timeout=None):
        self.requests.append({"url": url, "headers": headers, "json": json, "timeout": timeout})
        return self.responses.pop(0)


def test_openrouter_client_posts_structured_chat_completion():
    """Client sends OpenRouter chat completion request and parses JSON content."""
    from auto_tagger.config import Settings
    from auto_tagger.llm.client import OpenRouterClient
    from auto_tagger.llm.schemas import CandidateSelectionResponse

    http = FakeHttpClient(
        [
            FakeResponse(
                payload={
                    "model": "test/model",
                    "choices": [
                        {
                            "message": {
                                "content": json.dumps(
                                    {"selected_index": 0, "confidence": 0.9, "reason": "best"}
                                )
                            }
                        }
                    ],
                    "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
                }
            )
        ]
    )

    response = OpenRouterClient(
        Settings(llm_api_key="key", llm_model="test/model"),
        http_client=http,
    ).complete_json(
        [{"role": "user", "content": "pick"}],
        CandidateSelectionResponse,
    )

    assert response.data["selected_index"] == 0
    assert response.usage.total_tokens == 15
    request = http.requests[0]
    assert request["url"] == "https://openrouter.ai/api/v1/chat/completions"
    assert request["headers"]["Authorization"] == "Bearer key"
    assert request["json"]["response_format"]["type"] == "json_schema"


def test_openrouter_client_requires_api_key():
    """Client fails before network calls when no API key exists."""
    from auto_tagger.config import Settings
    from auto_tagger.exceptions import ConfigError
    from auto_tagger.llm.client import OpenRouterClient
    from auto_tagger.llm.schemas import CandidateSelectionResponse

    with pytest.raises(ConfigError, match="LLM API key"):
        OpenRouterClient(Settings(llm_api_key=None)).complete_json([], CandidateSelectionResponse)


def test_openrouter_client_retries_retryable_status():
    """Retryable status codes are retried before succeeding."""
    from auto_tagger.config import Settings
    from auto_tagger.llm.client import OpenRouterClient
    from auto_tagger.llm.schemas import CandidateSelectionResponse

    http = FakeHttpClient(
        [
            FakeResponse(status_code=429, text="slow down"),
            FakeResponse(
                payload={
                    "choices": [
                        {
                            "message": {
                                "content": json.dumps(
                                    {
                                        "selected_index": None,
                                        "confidence": 0.1,
                                        "reason": "none",
                                    }
                                )
                            }
                        }
                    ],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3},
                }
            ),
        ]
    )

    response = OpenRouterClient(
        Settings(llm_api_key="key"),
        http_client=http,
        sleep_func=lambda _seconds: None,
    ).complete_json([], CandidateSelectionResponse)

    assert response.data["selected_index"] is None
    assert len(http.requests) == 2


def test_openrouter_client_rejects_malformed_json():
    """Malformed model content is surfaced as a tagging error."""
    from auto_tagger.config import Settings
    from auto_tagger.exceptions import TaggingError
    from auto_tagger.llm.client import OpenRouterClient
    from auto_tagger.llm.schemas import CandidateSelectionResponse

    http = FakeHttpClient(
        [FakeResponse(payload={"choices": [{"message": {"content": "not json"}}]})]
    )

    with pytest.raises(TaggingError, match="malformed JSON"):
        OpenRouterClient(Settings(llm_api_key="key"), http_client=http).complete_json(
            [],
            CandidateSelectionResponse,
        )
