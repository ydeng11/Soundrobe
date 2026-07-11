export const PROVIDER_TAG_KEYS = [
  "MUSICBRAINZ_TRACKID",
  "MUSICBRAINZ_ALBUMID",
  "MUSICBRAINZ_ARTISTID",
  "DISCOGS_ARTIST_ID",
  "DISCOGS_RELEASE_ID",
] as const;

export type ProviderTagKey = (typeof PROVIDER_TAG_KEYS)[number];

const ID3_DESCRIPTIONS: Record<ProviderTagKey, string> = {
  MUSICBRAINZ_TRACKID: "MusicBrainz Track Id",
  MUSICBRAINZ_ALBUMID: "MusicBrainz Album Id",
  MUSICBRAINZ_ARTISTID: "MusicBrainz Artist Id",
  DISCOGS_ARTIST_ID: "Discogs Artist Id",
  DISCOGS_RELEASE_ID: "Discogs Release Id",
};

const NORMALIZED_KEYS = new Map<string, ProviderTagKey>(
  PROVIDER_TAG_KEYS.map((key) => [normalizeProviderTagKey(key), key]),
);

export function canonicalProviderTagKey(key: string): ProviderTagKey | null {
  return NORMALIZED_KEYS.get(normalizeProviderTagKey(key)) ?? null;
}

export function providerTagId3Description(key: string): string | null {
  const canonical = canonicalProviderTagKey(key);
  return canonical ? ID3_DESCRIPTIONS[canonical] : null;
}

function normalizeProviderTagKey(key: string): string {
  return key
    .replace(/^TXXX:/i, "")
    .replace(/[\s_-]/g, "")
    .toUpperCase()
    .replace(/^MUSICBRAINS/, "MUSICBRAINZ");
}
