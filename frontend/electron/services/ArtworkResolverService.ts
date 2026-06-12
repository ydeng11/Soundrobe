/**
 * ArtworkResolverService — ordered artwork provider resolution.
 *
 * Standalone service for downloading album covers and artist images.
 * Each resolved result is normalized to JPEG, max 1000×1000, quality 90.
 * Providers are tried in order; the first successful result is returned.
 */

import sharp from "sharp";
import logger from "../handlers/debug";

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
    { name: "discogs", needsCredentials: false, find: findDiscogs },
    { name: "wikimedia", needsCredentials: false, find: findWikimedia },
    { name: "google", needsCredentials: true, find: findGoogle },
  ];
}

// ── Service ─────────────────────────────────────────────────────────

export class ArtworkResolverService {
  private albumProviders: ArtworkProvider[];
  private artistProviders: ArtworkProvider[];
  private providers: ArtworkProvider[];
  private credentials: ArtworkCredentials = {};

  constructor() {
    this.albumProviders = defaultAlbumCoverProviders();
    this.artistProviders = defaultArtistImageProviders();
    this.providers = this.albumProviders;
  }

  /** Override the provider list (for testing or customization). Sets all kind-specific lists. */
  setProviders(providers: ArtworkProvider[]): void {
    this.providers = providers;
    this.albumProviders = providers;
    this.artistProviders = providers;
  }

  /** Get current provider names in order. Defaults to album-cover providers. */
  getProviderNames(): ArtworkSource[] {
    return this.providers.map((p) => p.name);
  }

  /** Set credentials for credentialed providers. */
  setCredentials(creds: ArtworkCredentials): void {
    this.credentials = { ...this.credentials, ...creds };
  }

  /** Resolve artwork by trying providers in order. */
  async resolve(ctx: ArtworkContext): Promise<ArtworkResult | null> {
    const providers = ctx.kind === "album-cover" ? this.albumProviders : this.artistProviders;

    if (ctx.kind === "artist-image") {
      logger.info("cover", `Resolve: kind=${ctx.kind} artist="${ctx.artistName ?? ""}" providers=[${providers.map(p => p.name).join(",")}]`);
    } else {
      logger.info("cover", `Resolve: kind=${ctx.kind} artist="${ctx.artistName ?? ""}" album="${ctx.albumName ?? ""}" mbid=${ctx.musicbrainzAlbumId ?? "null"} providers=[${providers.map(p => p.name).join(",")}]`);
    }

    for (const provider of providers) {
      const skipReason = this.skipReasonForProvider(provider, ctx);
      if (skipReason) {
        logger.debug("cover", `Resolve: skip provider=${provider.name} reason=${skipReason}`);
        continue;
      }

      logger.debug("cover", `Resolve: trying provider=${provider.name}`);

      try {
        const result = await provider.find(ctx, this.credentials);

        if (!result) {
          logger.debug("cover", `Resolve: provider=${provider.name} returned null`);
          continue;
        }
        if (!result.bytes || result.bytes.length === 0) {
          logger.debug("cover", `Resolve: provider=${provider.name} returned empty bytes`);
          continue;
        }

        // Normalize and validate before accepting
        const normalized = await this.normalizeImage(result.bytes);
        if (!normalized) {
          logger.warn("cover", `Resolve: provider=${provider.name} normalization failed, bytes=${result.bytes.length}`);
          continue;
        }

        logger.info("cover", `Resolve: SUCCESS provider=${result.source} url=${result.url ?? "(none)"} bytes=${normalized.length} kind=${result.kind}`);

        return {
          ...result,
          bytes: normalized,
        };
      } catch (err) {
        logger.warn("cover", `Resolve: provider=${provider.name} threw`, err);
        // Provider error — try the next one
        continue;
      }
    }

    if (ctx.kind === "artist-image") {
      logger.info("cover", `Resolve: ALL PROVIDERS FAILED for kind=${ctx.kind} artist="${ctx.artistName ?? ""}"`);
    } else {
      logger.info("cover", `Resolve: ALL PROVIDERS FAILED for kind=${ctx.kind} artist="${ctx.artistName ?? ""}" album="${ctx.albumName ?? ""}"`);
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

  /** Determine why a provider should be skipped, or null if it should run. */
  private skipReasonForProvider(provider: ArtworkProvider, ctx: ArtworkContext): string | null {
    if (provider.name === "cover-art-archive") {
      if (ctx.kind !== "album-cover") return "wrong kind";
      if (!ctx.musicbrainzAlbumId) return "no mbid";
    }
    // Safety nets for custom provider lists (setProviders):
    // These guards fire only when a test provides a non-kind-specific provider list.
    if (provider.name === "wikimedia" && ctx.kind !== "artist-image") return "wrong kind";
    if (provider.name === "theaudiodb" && ctx.kind !== "album-cover") return "wrong kind";
    if (provider.needsCredentials && !this.hasCredentials(provider.name)) return "missing credentials";
    return null;
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
          logger.debug("cover", `findLocal: found ${candidate} (${bytes.length} bytes)`);
          return { kind: "album-cover", source: "local", bytes, mime, url: candidate };
        }
      }
    }
    logger.debug("cover", `findLocal: no local cover file found in ${ctx.albumPath}`);
    return null;
  }

  // artist-image — check parent folder for artist.jpg
  const parentDir = path.dirname(ctx.albumPath);
  const artistJpg = path.join(parentDir, "artist.jpg");
  const artistPng = path.join(parentDir, "artist.png");
  if (fs.existsSync(artistJpg)) {
    const bytes = fs.readFileSync(artistJpg);
    logger.debug("cover", `findLocal: found artist image ${artistJpg} (${bytes.length} bytes)`);
    return { kind: "artist-image", source: "local", bytes, mime: "image/jpeg", url: artistJpg };
  }
  if (fs.existsSync(artistPng)) {
    const bytes = fs.readFileSync(artistPng);
    logger.debug("cover", `findLocal: found artist image ${artistPng} (${bytes.length} bytes)`);
    return { kind: "artist-image", source: "local", bytes, mime: "image/png", url: artistPng };
  }
  logger.debug("cover", `findLocal: no local artist image in ${parentDir}`);
  return null;
}

/**
 * Cover Art Archive — album cover from MusicBrainz release ID.
 * Only applies for album-cover when a musicbrainzAlbumId is provided.
 */
async function findCoverArtArchive(
  ctx: ArtworkContext,
  _creds: ArtworkCredentials,
): Promise<ArtworkResult | null> {
  const mbid = ctx.musicbrainzAlbumId;
  if (!mbid) return null;

  try {
    const caaUrl = `https://coverartarchive.org/release/${encodeURIComponent(mbid)}`;
    logger.debug("cover", `findCoverArtArchive: fetching ${caaUrl}`);

    const response = await fetch(caaUrl, { signal: AbortSignal.timeout(10_000) });
    if (!response.ok) {
      logger.debug("cover", `findCoverArtArchive: HTTP ${response.status}`);
      return null;
    }

    const data = (await response.json()) as {
      images?: Array<{ image: string; types?: string[] }>;
    };
    if (!data.images || data.images.length === 0) {
      logger.debug("cover", "findCoverArtArchive: no images in response");
      return null;
    }

    // Prefer "Front" type image
    const front = data.images.find((img) => img.types?.includes("Front"));
    const imageUrl = front?.image ?? data.images[0].image;
    if (!imageUrl) return null;

    logger.debug("cover", `findCoverArtArchive: selected image ${imageUrl} (${data.images.length} total images, front=${!!front})`);

    const imgResponse = await fetch(imageUrl, { signal: AbortSignal.timeout(10_000) });
    if (!imgResponse.ok) {
      logger.debug("cover", `findCoverArtArchive: image fetch HTTP ${imgResponse.status}`);
      return null;
    }

    const contentType = imgResponse.headers.get("content-type") ?? "image/jpeg";
    const bytes = Buffer.from(await imgResponse.arrayBuffer());
    logger.debug("cover", `findCoverArtArchive: downloaded ${bytes.length} bytes, type=${contentType}`);

    return { kind: "album-cover", source: "cover-art-archive", bytes, mime: contentType, url: imageUrl };
  } catch (err) {
    logger.warn("cover", "findCoverArtArchive: threw", err);
    return null;
  }
}

/**
 * Discogs — search for album/artist and validate candidates.
 * Uses the existing Discogs token from credentials.
 *
 * Instead of blindly accepting results[0].cover_image, this scans multiple
 * candidates (per_page=10), parses each Discogs title as "artist - album",
 * validates that the artist and album actually match the requested ones
 * (with normalization for Chinese variants, punctuation, etc.), and only
 * downloads the cover from the first valid match. If no valid match exists,
 * returns null so the next provider can run.
 */
/** Helper: download an image from a URL, return bytes + mime. */
async function downloadImage(
  url: string,
): Promise<{ bytes: Buffer; mime: string } | null> {
  const res = await fetch(url, { signal: AbortSignal.timeout(10_000) });
  if (!res.ok) return null;
  const bytes = Buffer.from(await res.arrayBuffer());
  const mime = res.headers.get("content-type") ?? "image/jpeg";
  return { bytes, mime };
}

// ── Discogs via DiscogsService ──────────────────────────────────────

/**
 * Discogs artwork lookup (artist-image + album-cover) via DiscogsService.
 */
async function findDiscogs(
  ctx: ArtworkContext,
  creds: ArtworkCredentials,
): Promise<ArtworkResult | null> {
  const artist = ctx.artistName;
  if (!artist) {
    logger.debug("cover", "findDiscogs: skip — no artist");
    return null;
  }

  try {
    if (ctx.kind === "artist-image") {
      return await findDiscogsArtistImage(artist, creds);
    }

    const album = ctx.albumName;
    if (!album) {
      logger.debug("cover", "findDiscogs: skip — no album name");
      return null;
    }

    return await findDiscogsAlbumCover(artist, album, creds);
  } catch (err) {
    logger.warn("cover", "findDiscogs: threw", err);
    return null;
  }
}

/**
 * Discogs artist image search.
 * Uses DiscogsService.searchArtists() + getArtistDetail() to find
 * artist images, with fallback from search cover_image to dedicated
 * artist endpoint for images.
 */
async function findDiscogsArtistImage(
  artist: string,
  creds: ArtworkCredentials,
): Promise<ArtworkResult | null> {
  const token = creds.discogsToken;
  if (!token) {
    logger.debug("cover", "findDiscogsArtistImage: skip — no Discogs token");
    return null;
  }

  const service = new DiscogsService({ token });

  // Use searchArtists for two-tier search (precise + generic)
  const aliasResult = await service.searchArtists(artist);
  if (!aliasResult) {
    logger.debug("cover", `findDiscogsArtistImage: no artist results for "${artist}"`);
    return null;
  }

  logger.debug("cover", `findDiscogsArtistImage: found artist id=${aliasResult.artistId} title="${aliasResult.title}"`);

  // Try the dedicated /artists/{id} endpoint for images
  const detail = await service.getArtistDetail(aliasResult.artistId);
  if (detail && detail.images.length > 0) {
    const image = detail.images.find((img) => img.type === "primary") ?? detail.images[0];
    logger.info("cover", `findDiscogsArtistImage: ACCEPTED id=${aliasResult.artistId} url=${image.uri}`);
    const img = await service.fetchImage(image.uri);
    if (img) {
      return { kind: "artist-image", source: "discogs", bytes: img.bytes, mime: img.mime, url: image.uri };
    }
  }

  logger.info("cover", `findDiscogsArtistImage: no image for artist="${artist}" (id=${aliasResult.artistId})`);
  return null;
}

/**
 * Discogs album cover search.
 * Uses DiscogsService.searchReleases() and validates artist + album match.
 */
async function findDiscogsAlbumCover(
  artist: string,
  album: string,
  creds: ArtworkCredentials,
): Promise<ArtworkResult | null> {
  const token = creds.discogsToken;
  if (!token) {
    logger.debug("cover", "findDiscogsAlbumCover: skip — no Discogs token");
    return null;
  }

  const service = new DiscogsService({ token });
  const searchResults = await service.searchReleases(artist, album, "release", 10);

  if (searchResults.length === 0) {
    logger.debug("cover", `findDiscogsAlbumCover: no results for artist="${artist}" album="${album}"`);
    return null;
  }

  for (let i = 0; i < searchResults.length; i++) {
    const candidate = searchResults[i];
    const title = candidate.title ?? "";

    // Parse title format: "Artist - Album" (Discogs format)
    const candidateArtist = title.includes(" - ") ? title.split(" - ")[0].trim() : "";
    const candidateAlbum = title.includes(" - ") ? title.split(" - ")[1].trim() : title;

    // Validate artist and album match
    const artistOk = candidateArtist ? await artistMatchesQuery(candidateArtist, artist) : false;
    if (!artistOk) {
      logger.debug("cover", `findDiscogsAlbumCover: candidate[${i}] REJECT artist — title="${title}"`);
      continue;
    }

    const albumOk = candidateAlbum ? await artistMatchesQuery(candidateAlbum, album) : false;
    if (!albumOk) {
      logger.debug("cover", `findDiscogsAlbumCover: candidate[${i}] REJECT album — title="${title}"`);
      continue;
    }

    if (!candidate.cover_image) {
      logger.debug("cover", `findDiscogsAlbumCover: candidate[${i}] ACCEPT artist+album but no cover_image — title="${title}"`);
      continue;
    }

    logger.info("cover", `findDiscogsAlbumCover: candidate[${i}] ACCEPTED title="${title}" id=${candidate.id ?? "?"} url=${candidate.cover_image}`);
    const img = await downloadImage(candidate.cover_image);
    if (img) {
      return { kind: "album-cover", source: "discogs", bytes: img.bytes, mime: img.mime, url: candidate.cover_image };
    }
  }



