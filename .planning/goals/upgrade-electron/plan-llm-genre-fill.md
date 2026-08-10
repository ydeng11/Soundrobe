# Plan: Option D — Conditional LLM Genre Fill

## Context
The current auto-tag pipeline has two LLM calls: tag enhancement (step 2) and candidate selection (step 8). When step 8 fails (HTTP 400 from OpenRouter provider), the pipeline falls through to `folderFallback`, which previously chose the folder candidate over richer MusicBrainz candidates. The real fix is to remove the fragile LLM selection step entirely and replace it with a conditional genre-only LLM call.

## Approach
Move to **Option D**: sequential source lookups (Dataset → Cache → MusicBrainz → Discogs) → merge → conditional LLM genre fill → apply. No LLM selection step.

### Rationale
- `mergeCandidateFields` already produces the best merged candidate from all sources
- Discogs is the only source with genre (besides LLM), so it's queried for its unique value
- LLM genre fill is only called when genre is still missing after Discogs (minimal cost, acceptable degradation if it fails)
- Removes the most fragile step (LLM selection) entirely

## Files to modify
- `frontend/electron/handlers/prompts.ts` — add `buildGenreFillMessages`
- `frontend/electron/handlers/auto-tag.ts` — restructure pipeline, remove LLM selection, add conditional genre fill

## Steps
- [ ] Add `buildGenreFillMessages(artist, album, trackTitles)` to prompts.ts
- [ ] In `processAlbum`:
  - Remove `selectCandidate` call (step 8)
  - Use `mergedCandidates[0]` directly as the candidate
  - Add conditional LLM genre fill when `candidate.genre` is null
  - Add new `fillGenreViaLLM` method
- [ ] Remove `selectCandidate` method (dead code)
- [ ] Remove `folderFallback` method (dead code)
- [ ] Run typecheck + tests
- [ ] Commit on `feature/llm-genre-fill` branch
