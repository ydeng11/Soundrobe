# Phase 4 Execution Plan: LLM Integration

## Overview

**Goal**: Integrate OpenRouter for match selection and fallback tag generation

**Duration**: ~6-8 hours

**Dependencies**: Phase 3 complete

**Primary Requirements**:
- REQ-LM-001: OpenRouter Client
- REQ-LM-002: Match Selection
- REQ-LM-003: Fallback Tag Generation
- REQ-LM-004: Cost Optimization
- REQ-BT-002: Candidate Selection

**Success Criteria**:
- OpenRouter API calls work through a project-owned client using configured model/API key
- LLM can select the best candidate from Phase 3 Beets/folder candidates
- LLM can generate reasonable fallback metadata from folder and file hints when no Beets candidate exists
- LLM responses are schema-validated and rejected safely when malformed
- Token usage and estimated cost are tracked per album
- Unit tests cover client request/response parsing, prompt construction, response validation, selection, fallback generation, and cost reporting without live network calls

---

## Architecture Target

Phase 4 adds an `soundrobe.llm` layer that consumes Phase 3 lookup candidates and produces validated decisions. It should not write tags directly; later workflow code can decide whether and how to apply selected/generated metadata.

OpenRouter's official API docs currently describe a Chat Completions endpoint at `/api/v1/chat/completions`, with OpenAI-compatible request/response shape, `response_format` support, and `usage` token statistics. Use direct HTTP via `httpx` so the project is not coupled to a third-party SDK surface.

**New modules**:
```
src/soundrobe/llm/
  __init__.py
  client.py        # OpenRouter HTTP client and retry/error handling
  cost.py          # Token usage and estimated cost models
  prompts.py       # Compact prompt builders for selection/fallback
  schemas.py       # Pydantic response schemas and validation helpers
  selection.py     # Candidate selection service
  fallback.py      # Fallback tag generation service
```

**New tests**:
```
tests/test_llm_client.py
tests/test_llm_cost.py
tests/test_llm_prompts.py
tests/test_llm_schemas.py
tests/test_llm_selection.py
tests/test_llm_fallback.py
```

**Dependency update**:
- Add `httpx>=0.28.0` to runtime dependencies.

---

## Wave 4.1: OpenRouter Client and Cost Models

**Objective**: Build a small, testable OpenRouter client with structured-output request support and cost tracking.

### Task 4.1.1: Add HTTP dependency and config fields

**Action**: Update project dependencies and settings.

**Changes**:
- Add `httpx>=0.28.0` to `pyproject.toml`.
- Confirm/extend `Settings` fields:
  - `llm_api_key`
  - `llm_endpoint` defaulting to `https://openrouter.ai/api/v1`
  - `llm_model` defaulting to the configured low-cost model
  - `llm_fallback_model` for model failover
  - `llm_max_candidates` defaulting to 5
  - `llm_max_tokens` / `llm_temperature`
  - `llm_cost_per_1k_prompt_tokens`
  - `llm_cost_per_1k_completion_tokens`

**Verification**:
```bash
.venv/bin/python -m pip install -e ".[dev]"
.venv/bin/pytest tests/test_config.py tests/test_llm_cost.py
```

---

### Task 4.1.2: Implement cost and usage models

**Action**: Add `src/soundrobe/llm/cost.py`.

**Design**:
- `TokenUsage` dataclass:
  - `prompt_tokens`
  - `completion_tokens`
  - `total_tokens`
- `CostEstimate` dataclass:
  - `prompt_cost`
  - `completion_cost`
  - `total_cost`
  - `model`
- Helper `estimate_cost(usage, model, prompt_rate, completion_rate)`.
- Helper for aggregating multiple album calls into a cost summary.

**Verification**:
```bash
.venv/bin/pytest tests/test_llm_cost.py
```

---

### Task 4.1.3: Implement OpenRouter client

**Action**: Add `src/soundrobe/llm/client.py`.

**Design**:
- `OpenRouterClient(settings: Settings, http_client: optional injectable)`
- `complete_json(messages, schema, *, model=None) -> LLMResponse`
- POST to `{settings.llm_endpoint}/chat/completions`
- Headers:
  - `Authorization: Bearer <api key>`
  - `Content-Type: application/json`
- Request body:
  - `model`
  - `messages`
  - `temperature`
  - `max_tokens` or `max_completion_tokens`
  - `response_format` with JSON schema where supported
- Parse:
  - first `choices[].message.content`
  - `usage.prompt_tokens`, `usage.completion_tokens`, `usage.total_tokens`
  - model from response
- Raise `ConfigError` if API key is missing.
- Raise `TaggingError` for non-2xx responses, empty choices, malformed JSON, timeouts, and provider failures.
- Implement bounded retry/backoff for `429`, `500`, `502`, `503`, and `504`.

**Testing strategy**:
- Use fake/injected HTTP client objects; do not hit OpenRouter in unit tests.
- Test success, missing key, HTTP error, malformed JSON, empty choices, and retryable errors.

**Verification**:
```bash
.venv/bin/pytest tests/test_llm_client.py
```

---

## Wave 4.2: Structured Prompts and Response Schemas

**Objective**: Keep prompts compact, deterministic, and easy to validate.

### Task 4.2.1: Implement response schemas

**Action**: Add `src/soundrobe/llm/schemas.py`.

**Models**:
- `CandidateSelectionResponse`:
  - `selected_index: int | None`
  - `confidence: float`
  - `reason: str`
- `GeneratedTrackTags`:
  - `title`
  - `artist`
  - `artists`
  - `album`
  - `album_artist`
  - `track_number`
  - `disc_number`
- `FallbackTagResponse`:
  - `artist`
  - `artists`
  - `album`
  - `album_artist`
  - `album_artists`
  - `year`
  - `genre`
  - `tracks`
  - `confidence`
  - `reason`

**Validation rules**:
- Confidence must be `0.0 <= confidence <= 1.0`.
- `selected_index` may be `None` but must be within candidate range when present.
- Generated tags must not contain MusicBrainz IDs.
- Track numbers must be positive when present.
- Required final values: artist, album, track title for generated track tags.

**Verification**:
```bash
.venv/bin/pytest tests/test_llm_schemas.py
```

---

### Task 4.2.2: Implement match selection prompts

**Action**: Add prompt builders in `src/soundrobe/llm/prompts.py`.

**Design**:
- `build_selection_messages(request, candidates) -> list[dict]`
- Include:
  - artist/album hints
  - existing track count and compact track title list
  - top N candidate summaries from Phase 3
  - strict instruction to return only structured JSON
  - explicit option to return `selected_index: null` when all candidates are poor
- Exclude:
  - verbose raw Beets objects
  - full file paths unless needed
  - unrelated current metadata fields

**Verification**:
```bash
.venv/bin/pytest tests/test_llm_prompts.py
```

---

### Task 4.2.3: Implement fallback tag prompts

**Action**: Add `build_fallback_messages(request, folder_candidate, current_metadata)`.

**Design**:
- Include folder artist/album hints, sorted file/title hints, current tags from Phase 2, and known constraints.
- Instruct the model not to invent MusicBrainz IDs.
- Instruct the model to leave uncertain fields empty rather than fabricate them.
- Require JSON matching `FallbackTagResponse`.
- Keep prompts under a small token budget by summarizing repeated fields.

**Verification**:
```bash
.venv/bin/pytest tests/test_llm_prompts.py
```

---

## Wave 4.3: Match Selection Service

**Objective**: Use LLM only where it adds value: picking among uncertain candidates from Phase 3.

### Task 4.3.1: Implement selection service

**Action**: Add `src/soundrobe/llm/selection.py`.

**Design**:
- `CandidateSelectionService(client, settings)`
- `select_candidate(request, candidates) -> SelectionResult`
- Flow:
  1. Return immediately if candidates list is empty.
  2. Optionally skip LLM when there is a single high-confidence Beets candidate below a distance threshold.
  3. Build compact selection prompt for remaining cases.
  4. Call OpenRouter client with structured response schema.
  5. Validate selected index and confidence.
  6. Return selected candidate, response reason, usage, and cost estimate.
- Never write tags.
- Preserve `None` selection when all candidates are poor.

**Verification**:
```bash
.venv/bin/pytest tests/test_llm_selection.py
```

---

### Task 4.3.2: Add selection cache keying

**Action**: Extend or add cache support for LLM decisions.

**Design**:
- Cache selection decisions by lookup request hash + candidate IDs/sources + prompt version.
- Store validated project-owned JSON, not raw model text.
- Do not cache malformed responses or transient errors.
- Keep Beets cache and LLM decision cache logically separate even if both use SQLite.

**Verification**:
```bash
.venv/bin/pytest tests/test_llm_selection.py tests/test_cache.py
```

---

### Task 4.3.3: Integrate selection into dry-run preview

**Action**: Extend `auto-tag tag PATH --dry-run` preview.

**Design**:
- Display lookup candidates as Phase 3 already does.
- When API key is configured and multiple candidates exist, display selected candidate and confidence.
- When API key is missing, show lookup candidates and a concise "LLM selection unavailable" note.
- Do not make network calls when no key is configured.
- Do not write tags in Phase 4.

**Verification**:
```bash
.venv/bin/pytest tests/test_cli.py tests/test_llm_selection.py
```

---

## Wave 4.4: Fallback Tag Generation and Cost Reporting

**Objective**: Generate validated fallback tags only when Beets cannot provide useful candidates.

### Task 4.4.1: Implement fallback generation service

**Action**: Add `src/soundrobe/llm/fallback.py`.

**Design**:
- `FallbackTagGenerationService(client, settings)`
- `generate_tags(request, folder_candidate, current_metadata) -> FallbackGenerationResult`
- Only used for folder-source candidates or no-candidate cases.
- Convert validated response into `TrackMetadata` objects for later writing.
- Include confidence and reason.
- Reject output that invents MusicBrainz IDs, has impossible track numbers, or conflicts with obvious file counts.

**Verification**:
```bash
.venv/bin/pytest tests/test_llm_fallback.py
```

---

### Task 4.4.2: Implement cost summary reporting

**Action**: Add reporting helpers and CLI display hooks.

**Design**:
- Aggregate `TokenUsage` and `CostEstimate` across selection and fallback calls.
- Display:
  - model
  - prompt/completion/total tokens
  - estimated cost
  - count of LLM calls
- Keep rates configurable because OpenRouter model prices change.
- Ensure output format can later support JSON/plain/table.

**Verification**:
```bash
.venv/bin/pytest tests/test_llm_cost.py tests/test_cli.py
```

---

### Task 4.4.3: Add robust validation and error handling tests

**Action**: Add negative-path tests across LLM services.

**Cases**:
- malformed JSON
- selected index out of range
- low confidence with selection present
- fallback tags missing required track titles
- generated MusicBrainz IDs
- API timeout / `429` / model unavailable
- budget exceeded or estimated cost above configured threshold

**Verification**:
```bash
.venv/bin/pytest tests/test_llm_client.py tests/test_llm_schemas.py tests/test_llm_selection.py tests/test_llm_fallback.py
```

---

## Implementation Notes

- Prefer direct `httpx` calls to OpenRouter's Chat Completions endpoint over a dedicated SDK unless execution discovers a stable official Python SDK already installed and suitable.
- Unit tests must mock HTTP. Live OpenRouter calls should be manual smoke tests gated by an API key, not part of default test runs.
- Use structured JSON responses and Pydantic validation. If validation fails, return a recoverable error and do not write tags.
- Keep LLM services decision-only in Phase 4. Writing metadata remains an explicit workflow step for later phases/CLI completion.
- Do not implement cover art, lyrics, ReplayGain calculation, health reports, or batch processing here.
- Minimize prompt input: candidate summaries and track hints, not raw objects or entire metadata dumps.
- Store no API keys in cache, logs, test fixtures, or plan files.

## Risks and Mitigations

**Hallucinated metadata**: The model can fabricate artists, dates, IDs, and track names.
- Mitigation: schema validation, no MusicBrainz IDs in generated fallback output, confidence thresholds, and "leave unknown empty" prompt instructions.

**Malformed model output**: Even structured prompts can produce invalid JSON.
- Mitigation: strict parsing, validation errors, and optional one retry with a repair instruction.

**Cost creep**: Large libraries can multiply small per-album calls into real spend.
- Mitigation: skip LLM for obvious high-confidence Beets candidates, cache decisions, track usage, and display estimated cost.

**Model/provider instability**: Models can become unavailable or rate-limited.
- Mitigation: retry transient failures, support fallback model configuration, and degrade to lookup/folder preview when LLM is unavailable.

**Prompt bloat**: Too much candidate/current metadata increases cost and failure risk.
- Mitigation: compact prompt builders with tests that assert important fields are present and large raw objects are absent.

## Final Verification

Run:
```bash
.venv/bin/ruff check src tests
.venv/bin/mypy src
.venv/bin/pytest --cov=soundrobe
auto-tag tag <sample-album-path> --dry-run
```

Optional manual smoke test when an API key is available:
```bash
AUTO_TAG_LLM_API_KEY=... auto-tag tag <sample-album-path> --dry-run
```

Phase 4 is complete when all LLM requirements pass, default tests do not require network access, dry-run preview degrades cleanly without an API key, and Phase 5 is ready for planning.
