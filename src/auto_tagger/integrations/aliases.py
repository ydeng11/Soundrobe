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
    alias_stripped = alias.strip()
    alias_cf = alias_stripped.casefold()
    if hint_key == alias_cf:
        return  # same name, not an alias

    aliases = load_aliases()
    existing = aliases.setdefault(hint_key, [])
    # Dedup by casefolded value, but store the original casing
    seen_cf = {a.strip().casefold() for a in existing}
    if alias_cf not in seen_cf:
        existing.append(alias_stripped)
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

    Returns 0.0 when one string is significantly longer than the other
    (ratio < 0.5), since character-level presence is not meaningful when
    comparing e.g. a 3-char name against a 24-char junk-appended string.
    """
    if not name_a or not name_b:
        return 0.0

    shorter, longer = (name_a, name_b) if len(name_a) <= len(name_b) else (name_b, name_a)

    # Reject when the shorter string accounts for less than 20% of the
    # longer string's length. This prevents false positives from
    # junk-appended names (e.g. "陈洁仪" vs
    # "陈洁仪-2002-异想世界 2CD WAV 分轨" at 12.5%), while still
    # allowing legitimate matches (e.g. "小娟" vs
    # "小娟&山谷里的居民" at 22.2%).
    if len(shorter) <= len(longer) * 0.2:
        return 0.0

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

    return matches / len(shorter)


def is_chinese_name(name: str) -> bool:
    """Return True if *name* contains any CJK Unified Ideograph.

    Covers the main CJK block (U+4E00–U+9FFF), which is sufficient
    to detect Chinese, Japanese shinjitai, and Korean Hanja names.
    """
    if not name:
        return False
    for ch in name:
        if "\u4e00" <= ch <= "\u9fff":
            return True
    return False


def get_all_name_variants(name: str) -> list[str]:
    """Return all variant forms of *name* for querying external services.

    Priority order:
      1. Learned aliases that are Latin-script (English names, Pinyin)
      2. Traditional Chinese (from `_convert_script`)
      3. Simplified Chinese (from `_convert_script`)
      4. Other script variants (HK, TW from `_convert_script`)
      5. The original name
      6. All remaining learned aliases (non-Latin)

    Deduplicated, ordered by expected usefulness to external services
    (Discogs, MusicBrainz, etc.) which primarily use Latin scripts.
    """
    seen: set[str] = set()
    result: list[str] = []

    def _add(v: str) -> None:
        key = v.casefold().strip()
        if key and key not in seen:
            seen.add(key)
            # Keep original casing for display/query, but use
            # the stripped version for dedup
            result.append(v.strip())

    # 1. Learned aliases that are Latin-script, sorted by length descending.
    #    Longer aliases are more specific (e.g. "joe hisaishi" vs "joe hisaish"),
    #    so they should be tried first to avoid wasting API calls on misspellings.
    latin_aliases = [a for a in get_aliases(name) if a.isascii()]
    # Sort by:
    #   1. Title-cased (starts with uppercase) first — more likely canonical
    #   2. Length descending (longer = more specific = more likely correct)
    #   3. Highest character sum (heuristic: more complete spellings like
    #      "hisaishi" > "hisaichi")
    latin_aliases.sort(key=lambda a: (
        0 if a and a[0].isupper() else 1,
        -len(a),
        -sum(ord(c) for c in a),
    ))
    for alias in latin_aliases:
        _add(alias)

    # 2-4. Script variants (TC, SC, others)
    for variant in _convert_script(name):
        if variant != name:
            _add(variant)

    # 5. Original name
    _add(name)

    # 6. Remaining learned aliases (non-Latin)
    for alias in get_aliases(name):
        if not alias.isascii():
            _add(alias)

    return result


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

    # Substring match: hint in artist (hint shorter)
    # Guard against junk-appended names by requiring the shorter string
    # to account for at least 20% of the longer string's length.
    # This prevents false positives like hint="陈洁仪" matching
    # artist="陈洁仪-2002-异想世界 2CD WAV 分轨" (12.5%), while still
    # allowing legitimate matches like "小娟" vs
    # "小娟&山谷里的居民" (22.2%) or "Jay Chou" vs
    # "Jay Chou ft. Lara Veronin" (32%).
    if norm_hint in norm_artist:
        if len(norm_hint) > len(norm_artist) * 0.2:
            return True
    elif norm_artist in norm_hint:
        # artist in hint: tag is a subset of folder name — usually
        # legitimate (e.g. tag="小娟" in folder "小娟&山谷里的居民")
        return True

    # Full string variant match (simplified <-> traditional Chinese)
    hint_variants = set(_convert_script(norm_hint))
    artist_variants = set(_convert_script(norm_artist))
    if hint_variants & artist_variants:
        return True
    for hv in hint_variants:
        for av in artist_variants:
            if hv in av:
                if len(hv) > len(av) * 0.2:
                    return True
            elif av in hv:
                return True

    # Character-level overlap (handles Japanese shinjitai vs simplified Chinese)
    # e.g. 久石让 vs 久石譲
    overlap = _characters_overlap(norm_hint, norm_artist)
    if overlap >= 0.5:
        return True

    # Alias match (with SC/TC variants)
    for alias in get_aliases(hint):
        if alias in norm_artist:
            if len(alias) > len(norm_artist) * 0.2:
                return True
        elif norm_artist in alias:
            return True
        alias_variants = set(_convert_script(alias))
        if alias_variants & artist_variants:
            return True
        for av in alias_variants:
            for av2 in artist_variants:
                if av in av2:
                    if len(av) > len(av2) * 0.2:
                        return True
                elif av2 in av:
                    return True
        # Character-level overlap for alias too
        if _characters_overlap(alias, norm_artist) >= 0.5:
            return True

    return False
