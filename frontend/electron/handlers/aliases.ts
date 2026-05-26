/**
 * Artist name alias management for cross-script matching.
 * Ported from Python auto_tagger.integrations.aliases.
 *
 * Handles cases where the same artist uses different names in different scripts,
 * e.g. 蔡健雅 in Chinese and "Tanya Chua" in English.
 *
 * Aliases are persisted to a JSON file and self-learned from LLM fallback results.
 */

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";

const DEFAULT_ALIAS_FILE = join(homedir(), ".auto-tagger", "artist-aliases.json");

/**
 * Override for testing. Set to a temp path before calling saveAlias/getAliases.
 */
export let aliasFilePath: string = DEFAULT_ALIAS_FILE;

export function setAliasFilePath(path: string): void {
  aliasFilePath = path;
}

function loadAliasesRaw(): Record<string, string[]> {
  try {
    return JSON.parse(readFileSync(aliasFilePath, "utf-8"));
  } catch {
    return {};
  }
}

/**
 * Save an alias pair to disk.
 * Both names are stored casefolded. No-op if hint === alias or either is empty.
 */
export function saveAlias(hint: string, alias: string): void {
  if (!hint || !alias) return;
  const hintKey = hint.toLowerCase().trim();
  const aliasStripped = alias.trim();
  const aliasCf = aliasStripped.toLowerCase();
  if (hintKey === aliasCf) return;

  const aliases = loadAliasesRaw();
  const existing = (aliases[hintKey] ??= []);
  const seenCf = new Set(existing.map((a) => a.trim().toLowerCase()));
  if (!seenCf.has(aliasCf)) {
    existing.push(aliasStripped);
  }
  try {
    mkdirSync(join(homedir(), ".auto-tagger"), { recursive: true });
  } catch {
    // already exists
  }
  writeFileSync(aliasFilePath, JSON.stringify(aliases, null, 2), "utf-8");
}

/**
 * Return known aliases for an artist name hint.
 */
export function getAliases(hint: string | null): string[] {
  if (!hint) return [];
  const raw = loadAliasesRaw();
  return raw[hint.toLowerCase().trim()] ?? [];
}

/**
 * Script conversion: simplified ↔ traditional Chinese.
 * Uses a simple character-level mapping table for common CJK pairs.
 * Falls back to just the original if opencc-js is not available.
 */

let openccJsInstance: any = null;

async function getOpenCC(): Promise<any> {
  if (openccJsInstance) return openccJsInstance;
  try {
    const mod = await import("opencc-js");
    // Create a converter: Simplified → Traditional
    openccJsInstance = mod.Converter({ from: "cn", to: "tw" });
    return openccJsInstance;
  } catch {
    return null;
  }
}

/**
 * Return simplified and traditional script variants of name.
 * Uses opencc-js if available. Falls back to just the original.
 */
export async function convertScript(name: string): Promise<string[]> {
  const variants: string[] = [name];
  try {
    const conv = await getOpenCC();
    if (conv) {
      const converted = conv(name);
      if (converted && converted !== name && !variants.includes(converted)) {
        variants.push(converted);
      }
    }
  } catch {
    // fallback to original only
  }
  return variants;
}

/** Check if a string contains any CJK Unified Ideograph (U+4E00–U+9FFF). */
export function isChineseName(name: string): boolean {
  if (!name) return false;
  for (let i = 0; i < name.length; i++) {
    const code = name.charCodeAt(i);
    if (code >= 0x4e00 && code <= 0x9fff) return true;
  }
  return false;
}

/** Character-level overlap for matching CJK names across scripts. */
function charactersOverlap(nameA: string, nameB: string, conv: any): number {
  if (!nameA || !nameB) return 0;

  const shorter = nameA.length <= nameB.length ? nameA : nameB;
  const longer = nameA.length <= nameB.length ? nameB : nameA;

  // Reject when the shorter string accounts for less than 20% of longer
  if (shorter.length <= longer.length * 0.2) return 0;

  // Build per-character variants for the longer string
  const longerChars: Set<string>[] = [];
  for (const ch of longer) {
    const variants = new Set([ch]);
    if (conv) {
      const converted = conv(ch);
      if (converted && converted !== ch) variants.add(converted);
    }
    longerChars.push(variants);
  }

  let matches = 0;
  for (const ch of shorter) {
    const chVariants = new Set([ch]);
    if (conv) {
      const converted = conv(ch);
      if (converted && converted !== ch) chVariants.add(converted);
    }
    const found = longerChars.some((lc) => {
      for (const v of chVariants) {
        if (lc.has(v)) return true;
      }
      return false;
    });
    if (found) matches++;
  }
  return matches / shorter.length;
}

/**
 * Check if artist name matches a hint directly or via a known alias.
 */
export async function artistMatchesAny(
  artist: string | null,
  hint: string | null,
): Promise<boolean> {
  if (!artist || !hint) return false;

  const normArtist = artist.toLowerCase().trim();
  const normHint = hint.toLowerCase().trim();

  // Substring match with length guard
  if (normHint.includes(normArtist)) return true;
  if (normArtist.includes(normHint) && normHint.length > normArtist.length * 0.2)
    return true;

  // SC/TC variant match
  const conv = await getOpenCC();
  if (conv) {
    const hintVariant = conv(normHint);
    const artistVariant = conv(normArtist);
    if (hintVariant === artistVariant) return true;
  }

  // Character-level overlap
  const convForChar = await getOpenCC();
  const overlap = charactersOverlap(normHint, normArtist, convForChar);
  if (overlap >= 0.5) return true;

  // Alias match
  for (const alias of getAliases(hint)) {
    const normAlias = alias.toLowerCase().trim();
    if (normArtist.includes(normAlias) || normAlias.includes(normArtist))
      return true;
    if (conv) {
      const aliasVariant = conv(normAlias);
      const artistVar = conv(normArtist);
      if (aliasVariant === artistVar) return true;
    }
  }

  return false;
}

/**
 * Return all variant forms of a name for querying external services.
 */
export async function getAllNameVariants(name: string): Promise<string[]> {
  const seen = new Set<string>();
  const result: string[] = [];

  function add(v: string): void {
    const key = v.trim().toLowerCase();
    if (key && !seen.has(key)) {
      seen.add(key);
      result.push(v.trim());
    }
  }

  // 1. Learned Latin-script aliases, sorted by specificity
  const latinAliases = getAliases(name)
    .filter((a) => /^[\x00-\x7F]+$/.test(a))
    .sort((a, b) => {
      const aUpper = a[0]?.toUpperCase() === a[0] ? 0 : 1;
      const bUpper = b[0]?.toUpperCase() === b[0] ? 0 : 1;
      if (aUpper !== bUpper) return aUpper - bUpper;
      return b.length - a.length;
    });
  for (const alias of latinAliases) add(alias);

  // 2. Script variants (SC/TC)
  const conv = await getOpenCC();
  if (conv) {
    const converted = conv(name);
    if (converted && converted !== name) add(converted);
  }

  // 3. Original
  add(name);

  // 4. Non-Latin aliases
  for (const alias of getAliases(name)) {
    if (!/^[\x00-\x7F]+$/.test(alias)) add(alias);
  }

  return result;
}
