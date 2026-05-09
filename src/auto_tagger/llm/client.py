"""OpenRouter HTTP client."""

from __future__ import annotations

import json
import time
from dataclasses import dataclass
from typing import Any

from pydantic import BaseModel, ValidationError

from auto_tagger.config import Settings
from auto_tagger.exceptions import ConfigError, TaggingError
from auto_tagger.llm.cost import TokenUsage

RETRYABLE_STATUS_CODES = {429, 500, 502, 503, 504}


@dataclass(frozen=True)
class LLMResponse:
    """Parsed LLM response."""

    data: dict[str, Any]
    usage: TokenUsage
    model: str


class OpenRouterClient:
    """Small OpenRouter chat-completions client."""

    def __init__(
        self,
        settings: Settings,
        http_client: Any | None = None,
        sleep_func: Any | None = None,
        max_retries: int = 2,
    ):
        self.settings = settings
        self.http_client = http_client
        self.sleep_func = sleep_func or time.sleep
        self.max_retries = max_retries

    def complete_json(
        self,
        messages: list[dict[str, str]],
        schema: type[BaseModel],
        *,
        model: str | None = None,
    ) -> LLMResponse:
        """Call OpenRouter and parse structured JSON content."""
        if not self.settings.llm_api_key:
            raise ConfigError("LLM API key is required")

        response = self._post_with_retries(messages, schema, model or self.settings.llm_model)
        payload = response.json()
        content = _first_message_content(payload)
        try:
            data = json.loads(content)
        except json.JSONDecodeError as exc:
            raise TaggingError(f"LLM returned malformed JSON: {exc}") from exc

        try:
            validated = schema.model_validate(data)
        except ValidationError as exc:
            raise TaggingError(f"LLM response validation failed: {exc}") from exc

        usage_payload = payload.get("usage", {})
        return LLMResponse(
            data=validated.model_dump(),
            usage=TokenUsage(
                prompt_tokens=int(usage_payload.get("prompt_tokens", 0) or 0),
                completion_tokens=int(usage_payload.get("completion_tokens", 0) or 0),
                total_tokens=int(usage_payload.get("total_tokens", 0) or 0),
            ),
            model=str(payload.get("model") or model or self.settings.llm_model),
        )

    def _post_with_retries(
        self,
        messages: list[dict[str, str]],
        schema: type[BaseModel],
        model: str,
    ) -> Any:
        last_response: Any = None
        for attempt in range(self.max_retries + 1):
            response = self._post(messages, schema, model)
            last_response = response
            if response.status_code < 400:
                return response
            if response.status_code not in RETRYABLE_STATUS_CODES or attempt >= self.max_retries:
                break
            self.sleep_func(0.25 * (attempt + 1))

        raise TaggingError(
            f"OpenRouter request failed with HTTP {last_response.status_code}: "
            f"{getattr(last_response, 'text', '')}"
        )

    def _post(
        self,
        messages: list[dict[str, str]],
        schema: type[BaseModel],
        model: str,
    ) -> Any:
        client = self.http_client or _default_http_client()
        return client.post(
            f"{self.settings.llm_endpoint.rstrip('/')}/chat/completions",
            headers={
                "Authorization": f"Bearer {self.settings.llm_api_key}",
                "Content-Type": "application/json",
            },
            json={
                "model": model,
                "messages": messages,
                "temperature": self.settings.llm_temperature,
                "max_tokens": self.settings.llm_max_tokens,
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": schema.__name__,
                        "schema": schema.model_json_schema(),
                    },
                },
            },
            timeout=30,
        )


def _default_http_client() -> Any:
    try:
        import httpx
    except ImportError as exc:
        raise ConfigError("httpx is required for OpenRouter support") from exc
    return httpx


def _first_message_content(payload: dict[str, Any]) -> str:
    choices = payload.get("choices") or []
    if not choices:
        raise TaggingError("OpenRouter response did not include choices")
    content = choices[0].get("message", {}).get("content")
    if not content:
        raise TaggingError("OpenRouter response did not include message content")
    return str(content)
