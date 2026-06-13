/**
 * TagPrettifyService — prettify tag values by normalizing casing and separators.
 *
 * Converts underscore/hyphen-separated text ("you_are_so_famous") into
 * properly capitalized natural text ("You Are So Famous"), while preserving
 * CJK characters, dotted acronyms, and already-pretty text.
 *
 * This is a pure function service — no Electron APIs, testable in plain Node.
 */

const LEADING_TRACK_NUMBER_RE = /^\s*(?:disc\s*)?\d{1,3}(?:[._ -]+|\s+)/i;

/**
 * Check whether a word is a dotted acronym like "F.I.R." or "U.S.A.".
 * These should be preserved without title-casing.
 */
function isDottedAcronym(word: string): boolean {
  return /^([A-Za-z]\.)([A-Za-z]\.)*[A-Za-z]\.?$/.test(word);
}

/**
 * Title-case a single word, preserving leading/trailing non-alphanumeric
 * characters (like parentheses or periods) and dotted acronyms.
 *
 * "you" → "You"
 * "(feat." → "(Feat."
 * "F.I.R." → "F.I.R." (dotted acronym preserved)
 * "someone)" → "Someone)"
 */
function titleCaseWord(word: string): string {
  if (!word) return word;
  if (isDottedAcronym(word)) return word;

  // Extract leading non-alphanumeric characters
  const leadingMatch = word.match(/^([^a-zA-Z0-9]*)(.*)$/);
  if (!leadingMatch) return word;
  const leading = leadingMatch[1];
  let rest = leadingMatch[2];

  // Extract trailing non-alphanumeric characters from rest
  const trailingMatch = rest.match(/^(.*?)([^a-zA-Z0-9]*)$/);
  if (!trailingMatch) return word;
  const core = trailingMatch[1];
  const trailing = trailingMatch[2];

  if (!core) return word;

  // Title-case the core: uppercase first char, lowercase the rest
  return leading + core.charAt(0).toUpperCase() + core.slice(1).toLocaleLowerCase() + trailing;
}

/**
 * Prettify a single tag string.
 *
 * Handles:
 * - Leading track number stripping ("01_", "110-", "disc1-")
 * - Underscore and hyphen separator replacement
 * - Number/letter boundary splitting ("track2" → "track 2")
 * - Title-casing (preserving CJK, dotted acronyms, and punctuation)
 *
 * @param text - The raw tag string to prettify
 * @returns Prettified string, or empty string for null/undefined input
 */
export function prettifyTag(text: string): string {
  if (!text || typeof text !== "string") return "";

  let result = text.trim();

  // Strip leading track number (e.g., "01.", "05_", "110-", "disc1-")
  result = result.replace(LEADING_TRACK_NUMBER_RE, "");

  // Replace underscores and hyphens with spaces
  result = result.replace(/[_\-]+/g, " ");

  // Insert space between letter-digit boundaries ("track2" → "track 2")
  result = result.replace(/([a-zA-Z])(\d)/g, "$1 $2");
  result = result.replace(/(\d)([a-zA-Z])/g, "$1 $2");

  // Collapse multiple spaces
  result = result.replace(/\s+/g, " ");

  // Trim whitespace again after all replacements
  result = result.trim();

  // Split into words, title-case each (non-dotted-acronym) word
  const words = result.split(" ");
  const prettified = words.map((word) => titleCaseWord(word));

  return prettified.join(" ");
}

/**
 * Prettify multiple tag fields at once.
 *
 * Takes an object of field names → raw string values and returns a new
 * object with each value prettified. Null/undefined values are passed
 * through without modification.
 *
 * @param fields - Object mapping field names to raw tag string values
 * @returns New object with prettified values
 */
export function prettifyTags(
  fields: Record<string, string | null | undefined>,
): Record<string, string | null | undefined> {
  const result: Record<string, string | null | undefined> = {};

  for (const [key, value] of Object.entries(fields)) {
    if (value === null || value === undefined) {
      result[key] = value;
    } else {
      result[key] = prettifyTag(value);
    }
  }

  return result;
}
