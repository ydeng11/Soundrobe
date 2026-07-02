# Plan: Add Chinese Script Enforcement Flag

## Context

The auto-tagger writes metadata tags to audio files. Chinese artist/album/title text can appear in either Simplified Chinese (SC) or Traditional Chinese (TC), and the code does not normalize between them. The project already depends on `opencc` (used in `integrations/aliases.py`). The user wants a config flag that forces all Chinese text fields to be converted to a target script at write time. The source script is unknown — text may already be SC, TC, or mixed.

## Approach

Add a `chinese_script` config option (`"simplified"`, `"traditional"`, or `None`) with user-facing aliases `sc`/`tc`. Apply OpenCC conversion to text-like metadata fields after `normalized()` and before `write_tags(...)`, so dry-run returns also reflect the conversion.

**OpenCC config:** `t2s` for simplified, `s2t` for generic traditional (not region-specific TW/HK; a future flag could add that).

### 1. Add the config setting

**File: `src/auto_tagger/config/settings.py`**

Add field + validator:

```python
chinese_script: str | None = Field(
    default=None,
    description="Enforce Chinese script variant when writing tags: "
                "'simplified'/'sc' or 'traditional'/'tc'. Null disables.",
)

@field_validator("chinese_script")
@classmethod
def validate_chinese_script(cls, v: str | None) -> str | None:
    if v is not None:
        ALIASES = {"sc": "simplified", "simplified": "simplified",
                   "tc": "traditional", "traditional": "traditional"}
        v = v.strip().lower()
        if v not in ALIASES:
            raise ValueError(f"chinese_script must be one of {set(ALIASES)}, got {v!r}")
        return ALIASES[v]
    return v
```

Env var: `AUTO_TAG_CHINESE_SCRIPT` (via existing `env_prefix`).

### 2. Fix config loader validation bypass

**File: `src/auto_tagger/config/loader.py`**

`load_settings()` currently uses `model_copy(update=config_data)` which **skips validators**. Replace with `model_validate()` so config-file values are validated:

```python
def load_settings(config_file: Path | None = None, **cli_overrides: Any) -> Settings:
    config_data = load_config_file(config_file)
    env_settings = Settings()
    # Merge config data into env-derived defaults, re-validating
    merged_dict = env_settings.model_dump()
    merged_dict.update(config_data)
    merged_settings = Settings.model_validate(merged_dict)
    if cli_overrides:
        merged_settings = merged_settings.merge_with_cli_args(**cli_overrides)
    return merged_settings
```

### 3. Add Chinese conversion utility

**File: `src/auto_tagger/core/metadata.py`** (add near top-level helpers)

```python
def convert_chinese_script(text: str | None, target: str) -> str | None:
    """Convert a string to the target Chinese script variant.

    OpenCC passes non-CJK text through unchanged.
    Uses 't2s' for simplified, 's2t' for generic traditional.
    """
    if not text:
        return text
    try:
        import opencc
        cfg = "t2s" if target == "simplified" else "s2t"
        conv = opencc.OpenCC(cfg)
        return conv.convert(text)
    except Exception:
        return text
```

Add method on `TrackMetadata`:

```python
def with_chinese_script(self, target: str) -> "TrackMetadata":
    """Return a copy with text-like fields converted to the target script.

    Converts: title, artist, artists, album, album_artist, album_artists,
    genre, composer, lyrics, year.
    Does NOT convert: musicbrainz_* IDs, replaygain values, booleans,
    track/disc numbers.
    """
    if not target:
        return self
    c = lambda t: convert_chinese_script(t, target)
    return replace(
        self,
        title=c(self.title),
        artist=c(self.artist),
        artists=[c(a) for a in self.artists],
        album=c(self.album),
        album_artist=c(self.album_artist),
        album_artists=[c(a) for a in self.album_artists],
        year=c(self.year),
        genre=c(self.genre),
        composer=c(self.composer),
        lyrics=c(self.lyrics),
    )
```

### 4. Apply conversion at write time

**File: `src/auto_tagger/core/writer.py`**

```python
def write_metadata(
    path: Path,
    metadata: TrackMetadata,
    dry_run: bool = False,
    chinese_script: str | None = None,
) -> TrackMetadata:
    normalized = metadata.normalized()
    if chinese_script:
        normalized = normalized.with_chinese_script(chinese_script)
    try:
        audio_file = load_audio_file(path)
        if dry_run:
            return normalized
        write_tags(audio_file.format, audio_file.mutagen_file, normalized)
        audio_file.mutagen_file.save()
        return normalized
    except FileProcessingError:
        raise
    except Exception as exc:
        raise TaggingError(f"Could not write metadata to {path}: {exc}") from exc
```

Conversion happens **after** `normalized()` and **before** `write_tags(...)`. Dry-run returns converted metadata too.

### 5. Thread `chinese_script` through all callers

All `write_metadata()` call sites:

- **`workflows/album.py`** (10 calls) — all in `AlbumWorkflow` methods with `self.settings`. Add `chinese_script=self.settings.chinese_script` to each.
- **`commands/audit.py`** (1 call, line 264) — in `_apply_fixes()` helper called from `execute(settings, ...)`. Thread `settings.chinese_script` from `execute()` into `_apply_fixes()`.
- **`quality/replaygain.py`** (1 call, line 123) — in `apply_replaygain_tags()`. Add optional `chinese_script: str | None = None` param. This function is exported and called in tests; callers pass it from their settings context.

### 6. Add CLI flag

**File: `src/auto_tagger/cli.py`**

Add to `tag` and `batch` commands:

```python
@click.option(
    "--chinese-script",
    type=click.Choice(["simplified", "traditional", "sc", "tc"]),
    default=None,
    help="Enforce Simplified (sc) or Traditional (tc) Chinese in tag text fields",
)
```

Thread to `settings.chinese_script` via direct assignment (same pattern as existing `--yolo`).

### 7. Update example config

**File: `config.example.yaml`**

```yaml
# Enforce a specific Chinese script variant in tag text fields.
# Accepts: simplified/sc, traditional/tc, or omit to disable.
# chinese_script: simplified
```

### 8. Tests

**File: `tests/test_chinese_script.py`** (new)

- `convert_chinese_script()` SC→TC (e.g. "蔡健雅" → "蔡健雅" already TC; "音乐" → "音樂")
- `convert_chinese_script()` TC→SC (e.g. "音樂" → "音乐")
- `convert_chinese_script()` with non-CJK text (unchanged)
- `convert_chinese_script()` with `None` input (returns `None`)
- `TrackMetadata.with_chinese_script()` end-to-end on all text fields
- `write_metadata(..., chinese_script="simplified", dry_run=True)` returns converted metadata without saving
- `Settings(chinese_script="sc").chinese_script == "simplified"`
- `Settings(chinese_script="tc").chinese_script == "traditional"`
- Invalid config value (`chinese_script: foo`) raises `ValidationError` after loader fix
- `chinese_script=None` (default) produces no conversion

## Files to Modify

1. `src/auto_tagger/config/settings.py` — add `chinese_script` field + validator
2. `src/auto_tagger/config/loader.py` — fix validation bypass with `model_validate()`
3. `src/auto_tagger/core/metadata.py` — add `convert_chinese_script()` + `TrackMetadata.with_chinese_script()`
4. `src/auto_tagger/core/writer.py` — accept + apply `chinese_script` param
5. `src/auto_tagger/workflows/album.py` — thread `chinese_script` through all 10 `write_metadata()` calls
6. `src/auto_tagger/commands/audit.py` — thread `chinese_script` through 1 `write_metadata()` call
7. `src/auto_tagger/quality/replaygain.py` — add `chinese_script` param to `apply_replaygain_tags()`
8. `src/auto_tagger/cli.py` — add `--chinese-script` option to `tag` and `batch`
9. `config.example.yaml` — document the new setting
10. `tests/test_chinese_script.py` — new test file

## Verification

1. `uv run pytest tests/test_chinese_script.py -v` — unit tests for conversion + settings + dry-run
2. `uv run pytest tests/test_writer.py tests/test_metadata.py tests/test_formats.py -v` — existing tests still pass
3. `uv run pytest tests/test_replaygain.py -v` — replaygain tests still pass with new param
4. `uv run pytest tests/ -v` — full test suite green
5. Manual: `auto-tag tag /path/to/chinese-album --chinese-script sc --dry-run`
