/**
 * ArtworkResolverService — ordered artwork provider resolution.
 *
 * Standalone service for downloading album covers and artist images.
 * Each resolved result is normalized to JPEG, max 1000×1000, quality 90.
 * Providers are tried in order; the first successful result is returned.
 */

import sharp from "sharp";

// ── Types ───────────────────────────────────────────────────────────

export type ArtworkKind = "album-cover" | "artist-image";

export type ArtworkSource =
  | "local"
  | "cover-art-archive"
  | "discogs"
  | "theaudiodb"
  | "wikimedia"
  | "google";

export interface ArtworkResult {
  kind: ArtworkKind;
  source: ArtworkSource;
  bytes: Buffer;
  mime: string;
  url?: string;
}

export interface ArtworkContext {
  kind: ArtworkKind;
  albumPath: string;
  artistName: string | null;
  albumName: string | null;
  musicbrainzAlbumId: string | null;
}

export interface ArtworkProvider {
  name: ArtworkSource;
  /** When truthy, the provider requires credentials set via setCredentials(). */
  needsCredentials?: boolean;
  /** Attempt to find artwork. Return null to skip to the next provider. */
  find(ctx: ArtworkContext, creds: ArtworkCredentials): Promise<ArtworkResult | null>;
}

export interface ArtworkCredentials {
  googleApiKey?: string | null;
  googleSearchEngineId?: string | null;
  theAudioDbApiKey?: string | null;
  discogsToken?: string | null;
}

// ── Default provider order per kind ────────────────────────────
// album-cover: local → cover-art-archive → discogs → theaudiodb → google
// artist-image: local → wikimedia → google

function defaultAlbumCoverProviders(): ArtworkProvider[] {
  return [
    { name: "local", needsCredentials: false, find: findLocal },
    { name: "cover-art-archive", needsCredentials: false, find: findCoverArtArchive },
    { name: "discogs", needsCredentials: false, find: findDiscogs },
    { name: "theaudiodb", needsCredentials: true, find: findTheAudioDb },
    { name: "google", needsCredentials: true, find: findGoogle },
  ];
}

function defaultArtistImageProviders(): ArtworkProvider[] {
  return [
    { name: "local", needsCredentials: false, find: findLocal },
    { name: "wikimedia", needsCredentials: false, find: findWikimedia },
    { name: "google", needsCredentials: true, find: findGoogle },
  ];
}

// ── Service ─────────────────────────────────────────────────────────

export class ArtworkResolverService {
  private providers: ArtworkProvider[];
  private credentials: ArtworkCredentials = {};

  constructor() {
    this.providers = defaultAlbumCoverProviders();
  }

  /** Override the provider list (for testing or customization). */
  setProviders(providers: ArtworkProvider[]): void {
    this.providers = providers;
  }

  /** Get current provider names in order. */
  getProviderNames(): ArtworkSource[] {
    return this.providers.map((p) => p.name);
  }

  /** Set credentials for credentialed providers. */
  setCredentials(creds: ArtworkCredentials): void {
    this.credentials = { ...this.credentials, ...creds };
  }

  /** Resolve artwork by trying providers in order. */
  async resolve(ctx: ArtworkContext): Promise<ArtworkResult | null> {
    for (const provider of this.providers) {
      // Skip if provider doesn't apply to this artwork kind
      if (provider.name === "cover-art-archive" && ctx.kind !== "album-cover") continue;
      if (provider.name === "cover-art-archive" && !ctx.musicbrainzAlbumId) continue;
      if (provider.name === "wikimedia" && ctx.kind !== "artist-image") continue;

      // Skip credentialed providers when credentials are missing
      if (provider.needsCredentials && !this.hasCredentials(provider.name)) continue;

      try {
        const result = await provider.find(ctx, this.credentials);

        if (!result) continue;
        if (!result.bytes || result.bytes.length === 0) continue;

        // Normalize and validate before accepting
        const normalized = await this.normalizeImage(result.bytes);
        if (!normalized) continue;

        return {
          ...result,
          bytes: normalized,
        };
      } catch {
        // Provider error — try the next one
        continue;
      }
    }

    return null;
  }

  /** Normalize image bytes to JPEG, max 1000×1000, quality 90. Returns null on failure. */
  async normalizeImage(bytes: Buffer): Promise<Buffer | null> {
    if (!bytes || bytes.length === 0) return null;
    try {
      const normalized = await sharp(bytes)
        .resize(1000, 1000, { fit: "inside", withoutEnlargement: true })
        .jpeg({ quality: 90 })
        .toBuffer();
      return normalized;
    } catch {
      return null;
    }
  }

  /** Build an ArtworkContext from resolved metadata. */
  buildContext(
    kind: ArtworkKind,
    albumPath: string,
    artistName: string | null,
    albumName: string | null,
    musicbrainzAlbumId: string | null,
  ): ArtworkContext {
    return { kind, albumPath, artistName, albumName, musicbrainzAlbumId };
  }

  // ── Credential check ───────────────────────────────────────────

  private hasCredentials(name: ArtworkSource): boolean {
    switch (name) {
      case "google":
        return !!(this.credentials.googleApiKey && this.credentials.googleSearchEngineId);
      case "theaudiodb":
        return !!this.credentials.theAudioDbApiKey;
      default:
        return true;
    }
  }
}

// ── Provider implementations ────────────────────────────────────────

/**
 * Check for existing local artwork.
 * For album covers, looks for cover.jpg etc. in the album directory.
 * For artist images, looks for artist.jpg in the parent directory.
 */
async function findLocal(ctx: ArtworkContext, _creds: ArtworkCredentials): Promise<ArtworkResult | null> {
  const fs = await import("fs");
  const path = await import("path");

  if (ctx.kind === "album-cover") {
    // Check common cover filenames in the album directory
    const names = ["cover", "Cover", "COVER", "front", "Front", "FRONT", "folder", "Folder", "FOLDER", "albumart", "AlbumArt"];
    const exts = [".jpg", ".jpeg", ".png"];
    for (const name of names) {
      for (const ext of exts) {
        const candidate = path.join(ctx.albumPath, `${name}${ext}`);
        if (fs.existsSync(candidate)) {
          const bytes = fs.readFileSync(candidate);
          const mime = ext === ".png" ? "image/png" : "image/jpeg";
          return { kind: "album-cover", source: "local", bytes, mime, url: candidate };
        }
      }
    }
    return null;
  }

  // artist-image — check parent folder for artist.jpg
  const parentDir = path.dirname(ctx.albumPath);
  const artistJpg = path.join(parentDir, "artist.jpg");
  const artistPng = path.join(parentDir, "artist.png");
  if (fs.existsSync(artistJpg)) {
    const bytes = fs.readFileSync(artistJpg);
    return { kind: "artist-image", source: "local", bytes, mime: "image/jpeg", url: artistJpg };
  }
  if (fs.existsSync(artistPng)) {
    const bytes = fs.readFileSync(artistPng);
    return { kind: "artist-image", source: "local", bytes, mime: "image/png", url: artistPng };
  }
  return null;
}

/**
 * Cover Art Archive — album cover from MusicBrainz release ID.
 * Only applies for album-cover when a musicbrainzAlbumId is provided.
 * The caller (resolve()) already filters by kind and MBID, so this
 * provider always has a valid MBID when called.
 */
async function findCoverArtArchive(
  ctx: ArtworkContext,
  _creds: ArtworkCredentials,
): Promise<ArtworkResult | null> {
  const mbid = ctx.musicbrainzAlbumId;
  if (!mbid) return null;

  try {
    const response = await fetch(
      `https://coverartarchive.org/release/${encodeURIComponent(mbid)}`,
      { signal: AbortSignal.timeout(10_000) },
    );
    if (!response.ok) return null;

    const data = (await response.json()) as {
      images?: Array<{ image: string; types?: string[] }>;
    };
    if (!data.images || data.images.length === 0) return null;

    // Prefer "Front" type image
    const front = data.images.find((img) => img.types?.includes("Front"));
    const imageUrl = front?.image ?? data.images[0].image;
    if (!imageUrl) return null;

    const imgResponse = await fetch(imageUrl, { signal: AbortSignal.timeout(10_000) });
    if (!imgResponse.ok) return null;

    const contentType = imgResponse.headers.get("content-type") ?? "image/jpeg";
    const bytes = Buffer.from(await imgResponse.arrayBuffer());

    return { kind: "album-cover", source: "cover-art-archive", bytes, mime: contentType, url: imageUrl };
  } catch {
    return null;
  }
}

/**
 * Discogs — search for album/artist and return the first image.
 * Uses the existing Discogs token from credentials.
 */
async function findDiscogs(
  ctx: ArtworkContext,
  creds: ArtworkCredentials,
): Promise<ArtworkResult | null> {
  const artist = ctx.artistName;
  const album = ctx.albumName;
  if (!artist || !album) return null;

  const DISCOGS_BASE = "https://api.discogs.com";
  const token = creds.discogsToken;

  try {
    // Search releases (preferred over masters)
    const searchUrl = `${DISCOGS_BASE}/database/search?type=release&q=${encodeURIComponent(`${artist} ${album}`)}`;
    const headers: Record<string, string> = {};
    if (token) headers["Authorization"] = `Discogs token=${token}`;

    const searchRes = await fetch(searchUrl, { headers, signal: AbortSignal.timeout(10_000) });
    if (!searchRes.ok) return null;

    const searchData = (await searchRes.json()) as {
      results?: Array<{ cover_image?: string }>;
    };
    if (!searchData.results || searchData.results.length === 0) return null;

    const coverUrl = searchData.results[0].cover_image;
    if (!coverUrl) return null;

    const imgRes = await fetch(coverUrl, { signal: AbortSignal.timeout(10_000) });
    if (!imgRes.ok) return null;

    const bytes = Buffer.from(await imgRes.arrayBuffer());
    const mime = imgRes.headers.get("content-type") ?? "image/jpeg";

    return { kind: ctx.kind, source: "discogs", bytes, mime, url: coverUrl };
  } catch {
    return null;
  }
}

/**
 * TheAudioDB — album art from theaudiodb.com.
 * Credential: theaudiodb_api_key (required).
 */
async function findTheAudioDb(
  ctx: ArtworkContext,
  creds: ArtworkCredentials,
): Promise<ArtworkResult | null> {
  const apiKey = creds.theAudioDbApiKey;
  const album = ctx.albumName;
  if (!apiKey || !album) return null;

  try {
    const url = `https://theaudiodb.com/api/v1/json/${encodeURIComponent(apiKey)}/searchalbum.php?s=${encodeURIComponent(ctx.artistName ?? "")}&a=${encodeURIComponent(album)}`;
    const res = await fetch(url, { signal: AbortSignal.timeout(10_000) });
    if (!res.ok) return null;

    const data = (await res.json()) as {
      album?: Array<{ strAlbumThumb?: string }>;
    };
    if (!data.album || data.album.length === 0) return null;

    const thumbUrl = data.album[0].strAlbumThumb;
    if (!thumbUrl) return null;

    // TheAudioDB thumbs are often small; try replacing "preview" with larger
    const largeUrl = thumbUrl.replace("/preview/", "/");
    const imgRes = await fetch(largeUrl, { signal: AbortSignal.timeout(10_000) });
    if (!imgRes.ok) return null;

    const bytes = Buffer.from(await imgRes.arrayBuffer());
    const mime = imgRes.headers.get("content-type") ?? "image/jpeg";

    return { kind: ctx.kind, source: "theaudiodb", bytes, mime, url: largeUrl };
  } catch {
    return null;
  }
}

/**
 * Wikimedia/Wikidata — artist image search.
 * Only applies for artist-image. Uses Wikidata API to find the artist,
 * then retrieves the Commons image.
 */
async function findWikimedia(
  ctx: ArtworkContext,
  _creds: ArtworkCredentials,
): Promise<ArtworkResult | null> {
  const artist = ctx.artistName;
  if (!artist) return null;

  try {
    // 1. Search Wikidata for the artist
    const wikiDataUrl = `https://www.wikidata.org/w/api.php?action=wbsearchentities&search=${encodeURIComponent(artist)}&language=en&limit=5&format=json`;
    const wdRes = await fetch(wikiDataUrl, { signal: AbortSignal.timeout(10_000) });
    if (!wdRes.ok) return null;

    const wdData = (await wdRes.json()) as {
      search?: Array<{ id: string; label?: string }>;
    };
    if (!wdData.search || wdData.search.length === 0) return null;

    // 2. Check for P18 (image) property on the first result
    const entityId = wdData.search[0].id;
    const entityUrl = `https://www.wikidata.org/wiki/Special:EntityData/${entityId}.json`;
    const entityRes = await fetch(entityUrl, { signal: AbortSignal.timeout(10_000) });
    if (!entityRes.ok) return null;

    const entityData = (await entityRes.json()) as Record<string, unknown>;
    const entity = (entityData.entities as Record<string, unknown>)?.[entityId] as Record<string, unknown> | undefined;
    if (!entity) return null;

    const claims = entity.claims as Record<string, unknown> | undefined;
    if (!claims) return null;

    const p18 = claims.P18 as Array<Record<string, unknown>> | undefined;
    if (!p18 || p18.length === 0) return null;

    const filename = ((p18[0].mainsnak as any)?.datavalue?.value as string | undefined);
    if (!filename) return null;

    // 3. Fetch the image via Wikimedia Commons
    const commonsUrl = `https://commons.wikimedia.org/wiki/Special:FilePath/${encodeURIComponent(filename.replace(/ /g, "_"))}`;
    const imgRes = await fetch(commonsUrl, { signal: AbortSignal.timeout(15_000) });
    if (!imgRes.ok) return null;

    const bytes = Buffer.from(await imgRes.arrayBuffer());
    const mime = imgRes.headers.get("content-type") ?? "image/jpeg";

    return { kind: "artist-image", source: "wikimedia", bytes, mime, url: commonsUrl };
  } catch {
    return null;
  }
}

/**
 * Google Custom Search — final fallback.
 * Requires googleApiKey and googleSearchEngineId credentials.
 * Searches for "{artist} {album} cover" for album covers,
 * or "{artist} artist photo" for artist images.
 */
async function findGoogle(
  ctx: ArtworkContext,
  creds: ArtworkCredentials,
): Promise<ArtworkResult | null> {
  const apiKey = creds.googleApiKey;
  const cx = creds.googleSearchEngineId;
  if (!apiKey || !cx) return null;

  try {
    const query =
      ctx.kind === "album-cover"
        ? `${ctx.artistName ?? ""} ${ctx.albumName ?? ""} album cover`
        : `${ctx.artistName ?? ""} artist photo`;

    const url = `https://www.googleapis.com/customsearch/v1?key=${encodeURIComponent(apiKey)}&cx=${encodeURIComponent(cx)}&searchType=image&q=${encodeURIComponent(query)}&num=1`;
    const res = await fetch(url, { signal: AbortSignal.timeout(10_000) });
    if (!res.ok) return null;

    const data = (await res.json()) as {
      items?: Array<{ link?: string; mime?: string }>;
    };
    if (!data.items || data.items.length === 0) return null;

    const imageUrl = data.items[0].link;
    if (!imageUrl) return null;

    const imgRes = await fetch(imageUrl, { signal: AbortSignal.timeout(10_000) });
    if (!imgRes.ok) return null;

    const bytes = Buffer.from(await imgRes.arrayBuffer());
    const mime = imgRes.headers.get("content-type") ?? data.items[0].mime ?? "image/jpeg";

    return { kind: ctx.kind, source: "google", bytes, mime, url: imageUrl };
  } catch {
    return null;
  }
}
