# Plan: LLM Tag Correction Evaluation Test + Prompt Improvement

## Context

The folder `2009-100天` gets parsed as `albumHint="0天"` due to a greedy regex (`DATE_PREFIX_RE`) that matches "2009-10" from "2009-100天". We fixed the regex with a negative lookahead `(?!\d)` (already done). Now we need to:

1. **Evaluate LLM's ability to correct wrong album hints** — Does it catch cases the parser misses?
2. **Improve the LLM prompt** — Feed richer context so the LLM can make better decisions.
3. **Build a regression test suite** — Cover many edge cases of numeric folder names.

**Design answer**: Yes, feeding `full_path`, existing album/artist tags, filenames, parsed hints, and folder/parent names is better. The LLM should arbitrate when parser hints conflict with raw path/tags.

## Approach

### Step 1: Extend `buildTagCorrectionMessages` with optional context

In `frontend/electron/handlers/prompts.ts`, add an optional 7th parameter:

```ts
options?: {
  fullPath?: string;
  filenames?: string[];
  existingAlbumTags?: string[];
  existingArtistTags?: string[];
}
```

Add to the payload when present:
- `full_path` — the complete directory path
- `filenames` — list of audio filenames in the directory
- `existing_album_tags` — unique album tags from audio files
- `existing_artist_tags` — unique artist tags from audio files

Add new system rules:
```
13. parsed_hints may be WRONG — especially when the folder contains a year
    prefix followed by a number that looks like a month (e.g. "2009-100天"
    was misparsed as "0天"). Always derive the album from folder_name by
    stripping ONLY the 4-digit year+separator prefix. Never consume
    following album digits as part of the date.

14. When folder_name uses "Year - Artist - Album" or "Year - Artist - Album
    (format)" patterns, extract only the Album portion. Common separators:
    dash (-), space, comma (,), underscore (_). Common format suffixes:
    (Lossless), [FLAC], (24bit), (24bit-48Hz)(WAV), [24B/48H].
```

Update the caller in `auto-tag.ts` `resolveTagsViaLLM` to pass the new options.

### Step 2: Create `frontend/test/handlers/llm-tag-correction-eval.test.ts`

Opt-in real LLM eval test. Guarded by `RUN_LLM_EVAL=1` and `LLM_API_KEY`:

```bash
RUN_LLM_EVAL=1 LLM_API_KEY=... LLM_MODEL=... npx vitest run test/handlers/llm-tag-correction-eval.test.ts
```

**Test cases (20):**

**Group A — Numeric album names with year prefix (parser gets these wrong):**

| # | Folder name | Parsed album (wrong) | Expected album |
|---|-------------|---------------------|----------------|
| 1 | `2009-100天` | `0天` | `100天` |
| 2 | `2005-200首经典` | `0首经典` | `200首经典` |
| 3 | `2017-1001夜` | `1夜` | `1001夜` |
| 4 | `2005-10秒学会日语` | `0秒学会日语` | `10秒学会日语` |
| 5 | `2010-1st album` | `st album` | `1st album` |
| 6 | `2015-100` | empty | `100` |
| 7 | `2006-300` | `0` | `300` |

**Group B — Normal date prefixes (parser gets right, LLM should not break):**

| # | Folder name | Parsed album | Expected album |
|---|-------------|-------------|----------------|
| 8 | `2009-04 Something` | `Something` | `Something` |
| 9 | `2007-09-28 F.I.R飞儿乐团 爱‧歌姬(24bit-48Hz)(WAV)` | `F.I.R飞儿乐团 爱‧歌姬(24bit-48Hz)` | `爱‧歌姬` |
| 10 | `1992-跳不完.爱不完.唱不完` | `跳不完.爱不完.唱不完` | `跳不完.爱不完.唱不完` |
| 11 | `2020-1984` | `1984` | `1984` |
| 12 | `100天` (no year prefix) | `100天` | `100天` |

**Group C — Various separators and format suffixes:**

| # | Folder name | Expected album |
|---|-------------|----------------|
| 13 | `2009 - 林俊杰 - 100天` | `100天` |
| 14 | `2009 - 林俊杰 - 100天 (Lossless)` | `100天` |
| 15 | `2009, 林俊杰, 100天 [24bit]` | `100天` |
| 16 | `2009 林俊杰_100天 (24bit-48Hz)(WAV)` | `100天` |
| 17 | `2009-林俊杰-100天(经典)` | `100天` |
| 18 | `2009.04 林俊杰 - 100天[FLAC]` | `100天` |
| 19 | `2009林俊杰100天` | `100天` |
| 20 | `2009 - 林俊杰 - 100天 [24B/48H]` | `100天` |

### Step 3: Add deterministic unit tests in `prompts.test.ts`

Add tests that assert the prompt payload includes the new context fields (no LLM call needed):

```ts
it("includes full_path and filenames when provided", () => {
  const messages = buildTagCorrectionMessages(
    "2009-100天", "林俊杰", "林俊杰", "0天", "2009", [...],
    { fullPath: "/Volumes/downloads/林俊杰/2009-100天", filenames: ["林俊杰 - 01.X.flac", ...] }
  );
  const payload = JSON.parse(messages[1].content);
  expect(payload.full_path).toBe("/Volumes/downloads/林俊杰/2009-100天");
  expect(payload.filenames).toEqual([...]);
});

it("system prompt contains rule about year prefix misparsing", () => {
  const messages = buildTagCorrectionMessages("Album", null, null, null, null, []);
  expect(messages[0].content).toContain("parsed_hints may be WRONG");
});
```

## Files to modify

| File | Change |
|------|--------|
| `frontend/electron/handlers/prompts.ts` | Add optional `options` param to `buildTagCorrectionMessages`, add `full_path`/`filenames`/`existing_*_tags` to payload, add rules #13 and #14 |
| `frontend/electron/handlers/auto-tag.ts` | Update `resolveTagsViaLLM` to pass new options to `buildTagCorrectionMessages` |
| `frontend/test/handlers/prompts.test.ts` | Add unit tests for new payload fields and system rules |
| `frontend/test/handlers/llm-tag-correction-eval.test.ts` | **New** — opt-in LLM eval (20 test cases) |

## Reuse

- `buildTagCorrectionMessages` from `frontend/electron/handlers/prompts.ts`
- `OpenRouterClient` from `frontend/electron/handlers/openrouter.ts`
- Test patterns from `frontend/test/handlers/prompts.test.ts`

## Steps

- [ ] Create new branch `codex/llm-tag-correction-eval` from `main`
- [ ] Extend `buildTagCorrectionMessages` in `prompts.ts` with optional context param + rules #13/#14
- [ ] Update `resolveTagsViaLLM` in `auto-tag.ts` to pass the new options
- [ ] Add deterministic unit tests in `prompts.test.ts`
- [ ] Create `llm-tag-correction-eval.test.ts` with 20 test cases
- [ ] Run unit tests (`npx vitest run test/handlers/prompts.test.ts`)
- [ ] Run LLM eval (`RUN_LLM_EVAL=1 LLM_API_KEY=... npx vitest run test/handlers/llm-tag-correction-eval.test.ts`)
- [ ] Report pass rates and commit

## Verification

```bash
# Unit tests (no API key)
npx vitest run test/handlers/prompts.test.ts

# LLM eval (requires API key)
RUN_LLM_EVAL=1 LLM_API_KEY=... LLM_MODEL=... npx vitest run test/handlers/llm-tag-correction-eval.test.ts

# All tests
npx vitest run
```
