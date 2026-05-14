"""Artist name alias management for cross-script matching.

Handles cases where the same artist uses different names in different scripts,
e.g. 蔡健雅 in Chinese and "Tanya Chua" in English.

Aliases are persisted to a JSON file and self-learned from LLM fallback results.
"""

from __future__ import annotations

import json
from pathlib import Path

ALIAS_FILE = Path.home() / ".auto-tagger" / "artist-aliases.json"


def load_aliases() -> dict[str, list[str]]:
    """Load all artist aliases from disk.

    Returns {casefolded_hint: [casefolded_alias, ...]}.
    """
    if not ALIAS_FILE.exists():
        return {}
    try:
        return json.loads(ALIAS_FILE.read_text())
    except (json.JSONDecodeError, OSError):
        return {}


def save_alias(hint: str, alias: str) -> None:
    """Persist an artist name alias for future lookups.

    Both names are stored casefolded. No-op if hint == alias or either is empty.
    """
    if not hint or not alias:
        return
    hint_key = hint.casefold().strip()
    alias_val = alias.casefold().strip()
    if hint_key == alias_val:
        return  # same name, not an alias

    aliases = load_aliases()
    existing = aliases.setdefault(hint_key, [])
    if alias_val not in existing:
        existing.append(alias_val)
    ALIAS_FILE.parent.mkdir(parents=True, exist_ok=True)
    ALIAS_FILE.write_text(json.dumps(aliases, ensure_ascii=False, indent=2))


def get_aliases(hint: str | None) -> list[str]:
    """Return known aliases for an artist name hint."""
    if not hint:
        return []
    return load_aliases().get(hint.casefold().strip(), [])


def _convert_script(name: str) -> list[str]:
    """Return simplified and traditional script variants of *name*.

    Tries OpenCC conversion if available. Falls back to just the original.
    """
    variants = [name]
    try:
        import opencc

        for cfg in ("s2t", "t2s", "s2tw", "tw2s", "s2hk", "hk2s"):
            try:
                conv = opencc.OpenCC(cfg)
                converted = conv.convert(name)
                if converted and converted != name and converted not in variants:
                    variants.append(converted)
            except Exception:
                continue
    except Exception:
        pass
    return variants


def _characters_overlap(name_a: str, name_b: str) -> float:
    """Return the fraction of characters in the shorter string that have a
    match in the longer string, considering OpenCC per-character variants.

    Used to match names like 久石让 vs 久石譲 where individual characters
    differ between simplified Chinese and Japanese shinjitai.
    """
    if not name_a or not name_b:
        return 0.0

    shorter, longer = (name_a, name_b) if len(name_a) <= len(name_b) else (name_b, name_a)

    # Precompute per-character variants for the longer string
    longer_chars: list[set[str]] = []
    for ch in longer:
        variants = {ch}
        for v in _convert_script(ch):
            variants.add(v)
        longer_chars.append(variants)

    matches = 0
    for ch in shorter:
        ch_variants = {ch}
        for v in _convert_script(ch):
            ch_variants.add(v)
        # Check if any variant of this char appears in any position of longer string
        found = any(bool(ch_variants & lc_set) for lc_set in longer_chars)
        if found:
            matches += 1

    return matches / len(shorter) if shorter else 0.0


def artist_matches_any(artist: str | None, hint: str | None) -> bool:
    """Check if *artist* matches *hint* directly or via a known alias.

    Used by _select_best_candidate to match candidates whose artist name
    differs from the user's hint (e.g. hint="蔡健雅", candidate artist="Tanya Chua").
    Handles simplified/traditional Chinese variant matching via OpenCC.
    """
    if not artist or not hint:
        return False
    norm_artist = artist.casefold().strip()
    norm_hint = hint.casefold().strip()

    # Direct match (substring in either direction)
    if norm_hint in norm_artist or norm_artist in norm_hint:
        return True

    # Full string variant match (simplified <-> traditional Chinese)
    hint_variants = set(_convert_script(norm_hint))
    artist_variants = set(_convert_script(norm_artist))
    if hint_variants & artist_variants:
        return True
    for hv in hint_variants:
        for av in artist_variants:
            if hv in av or av in hv:
                return True

    # Character-level overlap (handles Japanese shinjitai vs simplified Chinese)
    # e.g. 久石让 vs 久石譲
    overlap = _characters_overlap(norm_hint, norm_artist)
    if overlap >= 0.5:
        return True

    # Alias match (with SC/TC variants)
    for alias in get_aliases(hint):
        if alias in norm_artist or norm_artist in alias:
            return True
        alias_variants = set(_convert_script(alias))
        if alias_variants & artist_variants:
            return True
        for av in alias_variants:
            for av2 in artist_variants:
                if av in av2 or av2 in av:
                    return True
        # Character-level overlap for alias too
        if _characters_overlap(alias, norm_artist) >= 0.5:
            return True

    return False
