# Review: Simplification Improvements

## Changed: 1 file

### `src/auto_tagger/integrations/fallback.py`

**Removed `_tag_or_parsed()` helper function** (4 lines) and inlined its 4 call sites.

The helper was a trivial ternary chain:
```python
def _tag_or_parsed(meta_value, parsed_value, fallback=None):
    return meta_value if meta_value else (parsed_value if parsed_value else fallback)
```

Each call site already had the necessary context (`m`, `p`, `index`, `audio_path.stem`), so the indirection added no clarity — it just forced the reader to look up the helper to understand what `_tag_or_parsed(m.title if m else None, p.title, audio_path.stem)` does. Inlined expressions are shorter and self-documenting:

- `m.title if (m and m.title) else (p.title or audio_path.stem)` — reads left to right: "use metadata title, else parsed title, else filename stem"
- `m.artist if (m and m.artist) else p.artist` — "use metadata artist, else parsed artist"
- `m.track_number if (m and m.track_number is not None) else (p.track_number or index)` — same pattern with explicit None check for nullable int
- `m.disc_number if (m and m.disc_number is not None) else p.disc_number` — same for disc

## Reviewed: 27 files — no changes needed

The remaining files in scope were documentation, config, or well-structured code with no meaningful simplification opportunities:

- `.env.example`, `.gitignore`, `.rulesify/registry.toml`, `AGENTS.md`, `Justfile`, `PLAN.md`, `config.example.yaml`, `pyproject.toml` — config/doc files, no code to simplify
- `cli.py` — clean Click CLI; repeated `settings.yolo = True` pattern is intentional per-command override logic
- `commands/tag.py`, `commands/batch.py` — command implementations; `_write_health_reports` duplication is between private helpers in sibling modules, not worth consolidating
- `commands/artist.py` — clean, well-structured
- `config/settings.py` — clean Pydantic model
- `core/metadata.py` — clean dataclass
- `integrations/lookup.py` — complex but well-organized; not over-simplifiable without behavior changes
- `llm/prompts.py`, `llm/schemas.py` — clean prompt builders and schemas
- `utils/logging.py` — clean logging setup
- `workflows/album.py`, `workflows/artist.py`, `workflows/batch.py` — well-structured workflow classes
- All test files — clean tests

## Test results

- 50/50 tests pass in the directly affected/module test suites
- 1 pre-existing failure in `test_lookup_deterministic_parse_ambiguous_dot_convention` (unrelated to changes — tests dot-convention parsing in `lookup.py`, which was not modified)
