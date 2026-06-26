/**
 * ArtworkResolverService — ordered artwork provider resolution.
 *
 * Standalone service for downloading album covers and artist images.
 * Each resolved result is normalized to JPEG, max 1000×1000, quality 90.
 * Providers are tried in order; the first successful result is returned.
 */

import sharp from "sharp";
import logger from "../handlers/debug";
import { DiscogsService } from "./DiscogsService";
import { findArtistIdentity } from "./ArtistIdentityResolver";

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
  discogsArtistId?: string | null;
  discogsReleaseId?: string | null;
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
    logger.info(
      "cover",
      `Resolve: kind=${ctx.kind} artist="${ctx.artistName ?? ""}" album="${ctx.albumName ?? ""}" ` +
      `mbid=${ctx.musicbrainzAlbumId ?? "null"} discogsArtistId=${ctx.discogsArtistId ?? "null"} ` +
      `discogsReleaseId=${ctx.discogsReleaseId ?? "null"} providers=[${this.providers.map(p => p.name).join(",")}]`,
    );

    for (const provider of this.providers) {
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

    logger.info("cover", `Resolve: ALL PROVIDERS FAILED for kind=${ctx.kind} artist="${ctx.artistName ?? ""}" album="${ctx.albumName ?? ""}"`);
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
    discogsArtistId?: string | null,
    discogsReleaseId?: string | null,
  ): ArtworkContext {
    return { kind, albumPath, artistName, albumName, musicbrainzAlbumId, discogsArtistId, discogsReleaseId };
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
    if (provider.name === "wikimedia" && ctx.kind !== "artist-image") return "wrong kind";
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
async function findDiscogs(
  ctx: ArtworkContext,
  creds: ArtworkCredentials,
): Promise<ArtworkResult | null> {
  const token = creds.discogsToken;
  if (!token) {
    logger.debug("cover", "findDiscogs: skip — no token");
    return null;
  }

  const service = new DiscogsService({ token });

  try {
    const artist = ctx.artistName;

    if (ctx.kind === "artist-image") {
      if (!artist) {
        logger.debug("cover", "findDiscogs (artist-image): skip — no artist");
        return null;
      }

      if (ctx.discogsArtistId) {
        logger.debug("cover", `findDiscogs (artist-image): trying direct artist ID=${ctx.discogsArtistId}`);
        const direct = await fetchDiscogsArtistImage(service, ctx.discogsArtistId, "direct");
        if (direct) return direct;
        logger.debug("cover", `findDiscogs (artist-image): direct artist ID=${ctx.discogsArtistId} had no image`);
      }

      const identity = await findArtistIdentity(artist, { discogsToken: token });

      if (!identity.discogsArtistId) {
        logger.debug("cover", `findDiscogs (artist-image): no Discogs ID for "${artist}"`);
        return null;
      }

      if (identity.discogsArtistId === ctx.discogsArtistId) {
        logger.info("cover", `findDiscogs (artist-image): fallback ID=${identity.discogsArtistId} already tried and had no image`);
        return null;
      }

      logger.info("cover", `findDiscogs (artist-image): fallback ID=${identity.discogsArtistId} (source=${identity.source})`);
      const fallback = await fetchDiscogsArtistImage(service, identity.discogsArtistId, identity.source);
      if (fallback) return fallback;

      logger.info("cover", `findDiscogs (artist-image): no image for "${artist}"`);
      return null;
    }

    // Album cover: priority flow — known IDs first, then generic search
    // Priority 1: Known Discogs release ID — fetch release directly
    if (ctx.discogsReleaseId) {
      logger.debug("cover", `findDiscogs: trying direct release ID: ${ctx.discogsReleaseId}`);
      const release = await service.getReleaseDetail(ctx.discogsReleaseId);
      if (release && release.images && release.images.length > 0) {
        const frontImage = release.images.find((i) => i.type === "primary") ?? release.images[0];
        logger.info("cover", `findDiscogs: DIRECT release=${ctx.discogsReleaseId} url=${frontImage.uri}`);
        const img = await service.fetchImage(frontImage.uri);
        if (img) {
          return { kind: "album-cover", source: "discogs", bytes: img.bytes, mime: img.mime, url: frontImage.uri };
        }
      }
      logger.debug("cover", `findDiscogs: direct release ID ${ctx.discogsReleaseId} had no downloadable image`);
    }

    const album = ctx.albumName;
    if (!artist) {
      logger.debug("cover", "findDiscogs: skip search — no artist");
      return null;
    }
    if (!album) {
      logger.debug("cover", "findDiscogs: skip search — no album name");
      return null;
    }

    // Priority 2: Known Discogs artist ID — fetch artist releases, match by album title
    if (ctx.discogsArtistId) {
      logger.debug("cover", `findDiscogs: trying artist ID: ${ctx.discogsArtistId}`);
      const release = await service.getArtistReleaseByTitle(ctx.discogsArtistId, album);
      if (release) {
        if (release.images && release.images.length > 0) {
          const frontImage = release.images.find((i) => i.type === "primary") ?? release.images[0];
          logger.info("cover", `findDiscogs: ARTIST id=${ctx.discogsArtistId} release=${release.id} url=${frontImage.uri}`);
          const img = await service.fetchImage(frontImage.uri);
          if (img) {
            return { kind: "album-cover", source: "discogs", bytes: img.bytes, mime: img.mime, url: frontImage.uri };
          }
        }
        // The release itself may not have images in the artist-releases list;
        // fetch release detail to get images
        if (release.id) {
          const detail = await service.getReleaseDetail(release.id);
          if (detail && detail.images && detail.images.length > 0) {
            const frontImage = detail.images.find((i) => i.type === "primary") ?? detail.images[0];
            logger.info("cover", `findDiscogs: ARTIST detail release=${detail.id} url=${frontImage.uri}`);
            const img = await service.fetchImage(frontImage.uri);
            if (img) {
              return { kind: "album-cover", source: "discogs", bytes: img.bytes, mime: img.mime, url: frontImage.uri };
            }
          }
        }
      }
    }

    // Priority 3: Generic search (fallback)
    const searchResults = await service.searchReleases(artist, album, "release", 10);
    if (searchResults.length === 0) {
      logger.debug("cover", `findDiscogs (album-cover): no results for "${artist} ${album}"`);
      return null;
    }

    for (let i = 0; i < searchResults.length; i++) {
      const candidate = searchResults[i];
      const title = candidate.title ?? "";

      const parsed = parseDiscogsTitle(title);
      if (!parsed) {
        logger.debug("cover", `findDiscogs: candidate[${i}] cannot parse title="${title}" — skip`);
        continue;
      }

      const artistOk = await artistMatchesQuery(parsed.artist, artist);
      if (!artistOk) {
        logger.debug("cover", `findDiscogs: candidate[${i}] REJECT artist — title="${title}"`);
        continue;
      }

      const albumOk = await albumMatchesQuery(parsed.album, album, artist);
      if (!albumOk) {
        logger.debug("cover", `findDiscogs: candidate[${i}] REJECT album — title="${title}"`);
        continue;
      }

      if (!candidate.cover_image) {
        logger.debug("cover", `findDiscogs: candidate[${i}] ACCEPT artist+album but no cover_image — title="${title}" id=${candidate.id ?? "?"}`);
        continue;
      }

      logger.info("cover", `findDiscogs: candidate[${i}] ACCEPTED title="${title}" id=${candidate.id ?? "?"} url=${candidate.cover_image}`);
      const img = await service.fetchImage(candidate.cover_image);
      if (img) {
        return { kind: "album-cover", source: "discogs", bytes: img.bytes, mime: img.mime, url: candidate.cover_image };
      }
    }

    logger.info("cover", `findDiscogs: no acceptable candidate among ${searchResults.length} results for q="${artist} ${album}"`);
    return null;
  } catch (err) {
    logger.warn("cover", "findDiscogs: threw", err);
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
  if (!apiKey || !album) {
    logger.debug("cover", `findTheAudioDb: skip — apiKey=${!!apiKey} album=${!!album}`);
    return null;
  }

  try {
    const url = `https://theaudiodb.com/api/v1/json/${encodeURIComponent(apiKey)}/searchalbum.php?s=${encodeURIComponent(ctx.artistName ?? "")}&a=${encodeURIComponent(album)}`;
    logger.debug("cover", `findTheAudioDb: searching artist="${ctx.artistName ?? ""}" album="${album}"`);

    const res = await fetch(url, { signal: AbortSignal.timeout(10_000) });
    if (!res.ok) {
      logger.debug("cover", `findTheAudioDb: HTTP ${res.status}`);
      return null;
    }

    const data = (await res.json()) as {
      album?: Array<{ strAlbumThumb?: string }>;
    };
    const albumCount = data.album?.length ?? 0;
    logger.debug("cover", `findTheAudioDb: returned ${albumCount} albums`);

    if (!data.album || data.album.length === 0) return null;

    const thumbUrl = data.album[0].strAlbumThumb;
    if (!thumbUrl) {
      logger.debug("cover", "findTheAudioDb: album result has no strAlbumThumb");
      return null;
    }

    // TheAudioDB thumbs are often small; try replacing "preview" with larger
    const largeUrl = thumbUrl.replace("/preview/", "/");
    logger.debug("cover", `findTheAudioDb: thumb=${largeUrl}`);

    const imgRes = await fetch(largeUrl, { signal: AbortSignal.timeout(10_000) });
    if (!imgRes.ok) {
      logger.debug("cover", `findTheAudioDb: image fetch HTTP ${imgRes.status}`);
      return null;
    }

    const bytes = Buffer.from(await imgRes.arrayBuffer());
    const mime = imgRes.headers.get("content-type") ?? "image/jpeg";
    logger.debug("cover", `findTheAudioDb: downloaded ${bytes.length} bytes, type=${mime}`);

    return { kind: ctx.kind, source: "theaudiodb", bytes, mime, url: largeUrl };
  } catch (err) {
    logger.warn("cover", "findTheAudioDb: threw", err);
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
  if (!artist) {
    logger.debug("cover", "findWikimedia: skip — no artist");
    return null;
  }

  try {
    // 1. Search Wikidata for the artist
    const wikiDataUrl = `https://www.wikidata.org/w/api.php?action=wbsearchentities&search=${encodeURIComponent(artist)}&language=en&limit=5&format=json`;
    logger.debug("cover", `findWikimedia: searching Wikidata for "${artist}"`);

    const wdRes = await fetch(wikiDataUrl, { signal: AbortSignal.timeout(10_000) });
    if (!wdRes.ok) {
      logger.debug("cover", `findWikimedia: Wikidata HTTP ${wdRes.status}`);
      return null;
    }

    const wdData = (await wdRes.json()) as {
      search?: Array<{ id: string; label?: string }>;
    };
    const searchCount = wdData.search?.length ?? 0;
    logger.debug("cover", `findWikimedia: Wikidata returned ${searchCount} entities`);

    if (!wdData.search || wdData.search.length === 0) return null;

    // 2. Check for P18 (image) property on the first result
    const entityId = wdData.search[0].id;
    logger.debug("cover", `findWikimedia: selected entity ${entityId} ("${wdData.search[0].label ?? "?"}")`);

    const entityUrl = `https://www.wikidata.org/wiki/Special:EntityData/${entityId}.json`;
    const entityRes = await fetch(entityUrl, { signal: AbortSignal.timeout(10_000) });
    if (!entityRes.ok) {
      logger.debug("cover", `findWikimedia: entity HTTP ${entityRes.status}`);
      return null;
    }

    const entityData = (await entityRes.json()) as Record<string, unknown>;
    const entity = (entityData.entities as Record<string, unknown>)?.[entityId] as Record<string, unknown> | undefined;
    if (!entity) {
      logger.debug("cover", "findWikimedia: entity not found in response");
      return null;
    }

    const claims = entity.claims as Record<string, unknown> | undefined;
    if (!claims) {
      logger.debug("cover", "findWikimedia: entity has no claims");
      return null;
    }

    const p18 = claims.P18 as Array<Record<string, unknown>> | undefined;
    if (!p18 || p18.length === 0) {
      logger.debug("cover", "findWikimedia: entity has no P18 (image) claim");
      return null;
    }

    const filename = ((p18[0].mainsnak as any)?.datavalue?.value as string | undefined);
    if (!filename) {
      logger.debug("cover", "findWikimedia: P18 claim has no value");
      return null;
    }

    // 3. Fetch the image via Wikimedia Commons
    const commonsUrl = `https://commons.wikimedia.org/wiki/Special:FilePath/${encodeURIComponent(filename.replace(/ /g, "_"))}`;
    logger.debug("cover", `findWikimedia: Commons filename="${filename}" url=${commonsUrl}`);

    const imgRes = await fetch(commonsUrl, { signal: AbortSignal.timeout(15_000) });
    if (!imgRes.ok) {
      logger.debug("cover", `findWikimedia: Commons HTTP ${imgRes.status}`);
      return null;
    }

    const bytes = Buffer.from(await imgRes.arrayBuffer());
    const mime = imgRes.headers.get("content-type") ?? "image/jpeg";
    logger.debug("cover", `findWikimedia: downloaded ${bytes.length} bytes, type=${mime}`);

    return { kind: "artist-image", source: "wikimedia", bytes, mime, url: commonsUrl };
  } catch (err) {
    logger.warn("cover", "findWikimedia: threw", err);
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
  if (!apiKey || !cx) {
    logger.debug("cover", "findGoogle: skip — missing apiKey or cx");
    return null;
  }

  try {
    const query =
      ctx.kind === "album-cover"
        ? `${ctx.artistName ?? ""} ${ctx.albumName ?? ""} album cover`
        : `${ctx.artistName ?? ""} artist photo`;

    const url = `https://www.googleapis.com/customsearch/v1?key=${encodeURIComponent(apiKey)}&cx=${encodeURIComponent(cx)}&searchType=image&q=${encodeURIComponent(query)}&num=1`;
    logger.debug("cover", `findGoogle: query="${query}"`);

    const res = await fetch(url, { signal: AbortSignal.timeout(10_000) });
    if (!res.ok) {
      logger.debug("cover", `findGoogle: HTTP ${res.status}`);
      return null;
    }

    const data = (await res.json()) as {
      items?: Array<{ link?: string; mime?: string }>;
    };
    const itemCount = data.items?.length ?? 0;
    logger.debug("cover", `findGoogle: returned ${itemCount} items`);

    if (!data.items || data.items.length === 0) return null;

    const imageUrl = data.items[0].link;
    if (!imageUrl) {
      logger.debug("cover", "findGoogle: first item has no link");
      return null;
    }

    logger.debug("cover", `findGoogle: selected image ${imageUrl}`);

    const imgRes = await fetch(imageUrl, { signal: AbortSignal.timeout(10_000) });
    if (!imgRes.ok) {
      logger.debug("cover", `findGoogle: image fetch HTTP ${imgRes.status}`);
      return null;
    }

    const bytes = Buffer.from(await imgRes.arrayBuffer());
    const mime = imgRes.headers.get("content-type") ?? data.items[0].mime ?? "image/jpeg";
    logger.debug("cover", `findGoogle: downloaded ${bytes.length} bytes, type=${mime}`);

    return { kind: ctx.kind, source: "google", bytes, mime, url: imageUrl };
  } catch (err) {
    logger.warn("cover", "findGoogle: threw", err);
    return null;
  }
}

// ── Discogs title parsing and candidate matching ────────────────────

async function fetchDiscogsArtistImage(
  service: DiscogsService,
  artistId: string,
  source: string,
): Promise<ArtworkResult | null> {
  const detail = await service.getArtistDetail(Number(artistId));
  if (!detail || detail.images.length === 0) return null;

  const image = detail.images.find((img) => img.type === "primary") ?? detail.images[0];
  logger.info("cover", `findDiscogs (artist-image): ACCEPTED id=${artistId} (source=${source})`);
  const img = await service.fetchImage(image.uri);
  if (!img) return null;

  return { kind: "artist-image", source: "discogs", bytes: img.bytes, mime: img.mime, url: image.uri };
}

const UNICODE_PUNCT_SYMBOL_RE = /[\p{P}\p{S}]+/gu;
const WHITESPACE_RE = /\s+/g;

/**
 * Parse a Discogs release title into artist and album parts.
 * Discogs titles follow the format "Artist Name - Album Title".
 * Returns null if the title can't be parsed.
 */
function parseDiscogsTitle(title: string): { artist: string; album: string } | null {
  if (!title) return null;
  const sepIndex = title.indexOf(" - ");
  if (sepIndex === -1) return null;
  return {
    artist: title.slice(0, sepIndex).trim(),
    album: title.slice(sepIndex + 3).trim(),
  };
}

/**
 * Normalize text for comparison: NFKC + lowercase + strip punctuation/symbols + collapse whitespace.
 */
function normalizeForMatch(text: string): string {
  return text
    .normalize("NFKC")
    .toLowerCase()
    .replace(UNICODE_PUNCT_SYMBOL_RE, " ")
    .replace(WHITESPACE_RE, " ")
    .trim();
}

// ── Lazy OpenCC loader for Simplified/Traditional Chinese conversion ─

let openCCLazyInstance: {
  s2t: (s: string) => string;
  t2s: (s: string) => string;
} | null | undefined = undefined;

async function getOpenCCLazy(): Promise<{
  s2t: (s: string) => string;
  t2s: (s: string) => string;
} | null> {
  if (openCCLazyInstance !== undefined) return openCCLazyInstance;
  try {
    const mod = await import("opencc-js");
    const s2t = mod.Converter({ from: "cn", to: "tw" });
    const t2s = mod.Converter({ from: "tw", to: "cn" });
    openCCLazyInstance = { s2t, t2s };
    return openCCLazyInstance;
  } catch {
    openCCLazyInstance = null;
    return null;
  }
}

/**
 * Generate normalized variants of a name, including Simplified/Traditional Chinese conversions.
 */
async function getNormalizedVariants(name: string): Promise<string[]> {
  const variants = new Set<string>();
  const nf = normalizeForMatch(name);
  variants.add(nf);

  const oc = await getOpenCCLazy();
  if (oc) {
    const s2tNorm = normalizeForMatch(oc.s2t(name));
    if (s2tNorm !== nf) variants.add(s2tNorm);
    const t2sNorm = normalizeForMatch(oc.t2s(name));
    if (t2sNorm !== nf && t2sNorm !== s2tNorm) variants.add(t2sNorm);
  }

  return [...variants];
}

/**
 * Clean a Discogs artist string and return parts for matching.
 * - Strips trailing `*` (Discogs label-artifact indicator)
 * - Splits on ` = ` to get alternative representations (e.g. "F.I.R. = 飛兒楽團")
 */
function cleanDiscogsArtistParts(artist: string): string[] {
  const cleaned = artist.replace(/\s*\*+$/, "").trim();
  if (!cleaned) return [];
  const parts = cleaned.split(/\s*=\s*/).map((p) => p.trim()).filter(Boolean);
  return parts.length > 0 ? parts : [cleaned];
}

/**
 * Check whether a Discogs parsed artist name matches the requested query artist.
 *
 * Rejects:
 * - "Various" and "Various Artists"
 * - Artists with no overlap after normalization
 *
 * Accepts:
 * - Exact match after NFKC + punct/symbol stripping
 * - Simplified/Traditional Chinese variants
 * - Matching any part of a " = " separated artist (e.g. "F.I.R." matches "F.I.R. = 飛兒楽團")
 * - Safe containment when one normalized form is a substring of another
 *   (handles cases like "F.I.R." contained in "F.I.R.飞儿乐团")
 */
async function artistMatchesQuery(discogsArtist: string, queryArtist: string): Promise<boolean> {
  // Reject "Various" outright (first word check covers "Various" and "Various Artists")
  const firstWord = discogsArtist.split(/\s+/)[0]?.toLowerCase() ?? "";
  if (firstWord === "various") return false;

  const parts = cleanDiscogsArtistParts(discogsArtist);
  const queryVariants = await getNormalizedVariants(queryArtist);

  for (const part of parts) {
    const partVariants = await getNormalizedVariants(part);

    // Exact / variant match
    if (partVariants.some((pv) => queryVariants.includes(pv))) return true;

    // Safe containment: one normalized variant contains the other
    // (min length 3 to avoid false positives on single characters)
    for (const pv of partVariants) {
      if (pv.length < 3) continue;
      for (const qv of queryVariants) {
        if (qv.length < 3) continue;
        if (pv.includes(qv) || qv.includes(pv)) return true;
      }
    }
  }

  return false;
}

/**
 * Check whether a Discogs parsed album name matches the requested query album.
 *
 * Accepts (in order of preference):
 * 1. Exact match after NFKC + punct/symbol normalization
 * 2. Simplified/Traditional Chinese variant match (e.g. "無限" ≈ "无限")
 * 3. Self-titled convention: if the Discogs title is "同名专辑" / "同名"
 *    and the query album is the same as the query artist (self-titled release),
 *    it's a match.
 */
async function albumMatchesQuery(discogsAlbum: string, queryAlbum: string, queryArtist?: string): Promise<boolean> {
  const queryNorm = normalizeForMatch(queryAlbum);

  // Check if queryNorm matches any normalized variant of the discogs album
  // (handles exact match and Simplified/Traditional Chinese variants)
  const albumVariants = await getNormalizedVariants(discogsAlbum);
  if (albumVariants.includes(queryNorm)) return true;

  // Self-titled convention: "同名专辑" (same-name album) on Discogs
  // matches when the query album name equals the query artist name.
  if (queryArtist) {
    const albumNorm = normalizeForMatch(discogsAlbum);
    if ((albumNorm === "同名专辑" || albumNorm === "同名") && queryNorm === normalizeForMatch(queryArtist)) return true;
  }

  return false;
}
