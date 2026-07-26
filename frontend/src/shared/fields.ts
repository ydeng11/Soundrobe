/**
 * Parse a user-entered disc value (e.g. "1" or "1/2") into the individual
 * `discNumber` / `discTotal` fields expected by `TrackPatch`.
 *
 * Returns an object with only the fields that could be parsed from `value`.
 * An empty string or unparseable input produces an empty object.
 *
 * Both `handleSaveMetadata` and `handleBatchSave` use this to prevent the
 * two code paths from diverging.
 */
export function parseDiscField(
  value: string,
): { discNumber?: number; discTotal?: number } {
  const parts = value.split("/");
  const fields: { discNumber?: number; discTotal?: number } = {};
  if (parts[0]) {
    const n = parseInt(parts[0], 10) || undefined;
    if (n != null) fields.discNumber = n;
  }
  if (parts[1]) {
    const t = parseInt(parts[1], 10) || undefined;
    if (t != null) fields.discTotal = t;
  }
  return fields;
}
