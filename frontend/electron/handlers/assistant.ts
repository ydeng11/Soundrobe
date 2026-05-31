/**
 * Assistant IPC Handler — wires up the assistant runtime, tool registry,
 * and shared services for the Electron main process.
 */

import { ipcMain, BrowserWindow } from "electron";
import path from "path";
import { LlmTaskRunner } from "../services/LlmTaskRunner";
import { AssistantRuntime, type AssistantActionBatch } from "../services/AssistantRuntime";
import type { AssistantEvent } from "../services/AssistantRuntime";
import { AssistantToolRegistry } from "../services/AssistantToolRegistry";
import type { AssistantToolDef } from "../services/AssistantToolRegistry";
import { LibraryService } from "../services/LibraryService";
import { TrackTagService } from "../services/TrackTagService";
import type { TagUpdateInstruction } from "../services/TrackTagService";
import { ExtraTagService } from "../services/ExtraTagService";
import type { ExtraTagPlanInput } from "../services/ExtraTagService";
import { SafeQueryService } from "../services/SafeQueryService";
import { SafeApiRequestService } from "../services/SafeApiRequestService";
import { FolderOrganizerService } from "../services/FolderOrganizerService";
import type { WriteFields, ExtraTagUpdate } from "./writer";
import type { AlbumInfo, TrackData } from "../preload";

// ── Shared service instances ─────────────────────────────────────

let libraryService: LibraryService | null = null;
let trackTagService: TrackTagService | null = null;
let extraTagService: ExtraTagService | null = null;
let safeQueryService: SafeQueryService | null = null;
let safeApiRequestService: SafeApiRequestService | null = null;
let folderOrganizerService: FolderOrganizerService | null = null;

// ── Stored config (real values, read from main-process config) ──

let storedApiKey = "";
let storedModel = "";

// ── Runtime state ────────────────────────────────────────────────

let currentRuntime: AssistantRuntime | null = null;
let currentRegistry: AssistantToolRegistry | null = null;

// ── Shared app state (set by the renderer on each assistant:send) ─

let currentAppState: {
  libraryPath: string | null;
  activeAlbumPath: string | null;
  selectedTrackPaths: string[];
  tracks: TrackData[];
  albums: AlbumInfo[];
  autonomous: boolean;
} = {
  libraryPath: null,
  activeAlbumPath: null,
  selectedTrackPaths: [],
  tracks: [],
  albums: [],
  autonomous: false,
};

type FolderPlanForBatch = ReturnType<FolderOrganizerService["planOrganizeFiles"]>;
type MetadataTargetScope = "selected" | "active_album" | "explicit_paths";
type TaskTargetScope = MetadataTargetScope | "library";
type LibraryTaskKind = "auto_tag" | "audit";

const STANDARD_TAG_FIELDS = [
  "title",
  "artist",
  "album",
  "albumArtist",
  "year",
  "trackNumber",
  "trackTotal",
  "discNumber",
  "discTotal",
  "genre",
  "composer",
  "comment",
  "lyrics",
  "compilation",
] as const;

type StandardTagField = typeof STANDARD_TAG_FIELDS[number];
const STANDARD_TAG_FIELD_SET = new Set<string>(STANDARD_TAG_FIELDS);

// ── Event subscribers ────────────────────────────────────────────

type AssistantEventSubscriber = (event: AssistantEvent) => void;
const subscribers = new Set<AssistantEventSubscriber>();

function subscribeEvents(subscriber: AssistantEventSubscriber): () => void {
  subscribers.add(subscriber);
  return () => subscribers.delete(subscriber);
}

function broadcastEvent(event: AssistantEvent): void {
  for (const sub of subscribers) {
    try {
      sub(event);
    } catch {
      // Ignore subscriber errors
    }
  }
  // Also forward to all BrowserWindows
  for (const win of BrowserWindow.getAllWindows()) {
    if (!win.isDestroyed()) {
      win.webContents.send("assistant:event", event);
    }
  }
}

// ── Update shared app state from renderer ────────────────────────

export function setAssistantAppState(state: typeof currentAppState): void {
  currentAppState = state;
  if (safeQueryService) {
    safeQueryService.setTracks(state.tracks);
  }
  if (libraryService) {
    libraryService.setLibraryPath(state.libraryPath);
  }
  if (folderOrganizerService) {
    folderOrganizerService.setLibraryRoot(state.libraryPath);
  }
}

// ── Update stored config from main-process config ────────────────

/**
 * Update the stored API key and model from the main-process config.
 * Call this from the init handler so the assistant always uses the
 * real API key (not the redacted one the renderer sees via getConfig()).
 */
export function setStoredConfig(config: { apiKey?: string; model?: string }): void {
  if (config.apiKey) storedApiKey = config.apiKey;
  if (config.model) storedModel = config.model;
}

// ── Initialize services (called once on app start) ───────────────

export function initializeAssistantServices(config: {
  apiKey: string;
  model?: string;
  discogsToken?: string | null;
  lyricsHost?: string | null;
  libraryPath?: string | null;
}): void {
  // Store the real API key (NOT redacted — this reads from main-process config)
  if (config.apiKey) storedApiKey = config.apiKey;
  if (config.model) storedModel = config.model;

  libraryService = new LibraryService();
  if (config.libraryPath) {
    libraryService.setLibraryPath(config.libraryPath);
  }
  trackTagService = new TrackTagService();
  extraTagService = new ExtraTagService();
  safeQueryService = new SafeQueryService();
  safeApiRequestService = new SafeApiRequestService();
  if (config.discogsToken) {
    safeApiRequestService.setDiscogsToken(config.discogsToken);
  }
  if (config.lyricsHost) {
    safeApiRequestService.setLyricsHost(config.lyricsHost);
  }
  folderOrganizerService = new FolderOrganizerService();
  if (config.libraryPath) {
    folderOrganizerService.setLibraryRoot(config.libraryPath);
  }
}

// ── Update service config (e.g. when settings change) ────────────

export function updateAssistantConfig(config: {
  discogsToken?: string | null;
  lyricsHost?: string | null;
}): void {
  if (config.discogsToken !== undefined && safeApiRequestService) {
    safeApiRequestService.setDiscogsToken(config.discogsToken);
  }
  if (config.lyricsHost !== undefined && safeApiRequestService) {
    safeApiRequestService.setLyricsHost(config.lyricsHost);
  }
}

// ── Build read-only tools ────────────────────────────────────────

function buildReadOnlyTools(): AssistantToolDef[] {
  if (!libraryService || !safeQueryService || !safeApiRequestService) {
    throw new Error("Assistant services not initialized");
  }

  return [
    {
      name: "library.summarize",
      description: "Summarize the current library, active album, and selection. Includes track counts, artist counts, genre counts, and tag completeness.",
      inputSchema: {
        type: "object",
        properties: {},
        required: [],
      },
      isReadOnly: true,
      executor: async () => {
        const ctx = libraryService!.buildAppContext({
          libraryPath: currentAppState.libraryPath,
          activeAlbumPath: currentAppState.activeAlbumPath,
          selectedTrackPaths: currentAppState.selectedTrackPaths,
          tracks: currentAppState.tracks,
          albums: currentAppState.albums,
          assistantAutonomous: currentAppState.autonomous,
        });

        const summary = libraryService!.summarizeLibrary(
          currentAppState.albums,
          currentAppState.tracks,
        );

        const lines: string[] = [
          `Library: ${ctx.libraryPath ?? "No library loaded"}`,
          `Albums: ${summary.albumCount}, Tracks: ${summary.trackCount}`,
          `Artists: ${summary.artistCount}, Genres: ${summary.genreCount}`,
          `Total size: ${(summary.totalSizeBytes / (1024 * 1024)).toFixed(1)} MB`,
          `Total duration: ${Math.round(summary.totalDurationSeconds / 60)} min`,
        ];

        if (summary.missingTitle > 0) lines.push(`Missing titles: ${summary.missingTitle}`);
        if (summary.missingArtist > 0) lines.push(`Missing artists: ${summary.missingArtist}`);
        if (summary.missingAlbum > 0) lines.push(`Missing albums: ${summary.missingAlbum}`);
        if (summary.missingYear > 0) lines.push(`Missing years: ${summary.missingYear}`);
        if (summary.missingGenre > 0) lines.push(`Missing genres: ${summary.missingGenre}`);
        if (summary.byCodec && Object.keys(summary.byCodec).length > 0) {
          lines.push(`Codecs: ${Object.entries(summary.byCodec).map(([c, n]) => `${c} (${n})`).join(", ")}`);
        }

        if (ctx.activeAlbumSummary) {
          lines.push(`\nActive album: ${ctx.activeAlbumSummary.name} (${ctx.activeAlbumSummary.trackCount} tracks, ${ctx.activeAlbumSummary.hasCover ? "has cover" : "no cover"})`);
        }

        if (ctx.selectedTrackSummaries.length > 0) {
          lines.push(`\nSelected ${ctx.selectedTrackSummaries.length} track(s):`);
          for (const t of ctx.selectedTrackSummaries.slice(0, 5)) {
            lines.push(`  - ${t.title ?? "?"} by ${t.artist ?? "?"} (${t.album ?? "?"})`);
          }
          if (ctx.selectedTrackSummaries.length > 5) {
            lines.push(`  ... and ${ctx.selectedTrackSummaries.length - 5} more`);
          }
        }

        return {
          ok: true,
          summary: lines.join("\n"),
          data: { context: ctx, summary },
        };
      },
    },
    {
      name: "tracks.search",
      description: "Search the current library tracks by title, artist, album, genre, year, codec, or missing fields. Use this to find specific tracks or groups of tracks.",
      inputSchema: {
        type: "object",
        properties: {
          title: { type: "string", description: "Filter by title (substring, case-insensitive)" },
          artist: { type: "string", description: "Filter by artist (substring, case-insensitive)" },
          album: { type: "string", description: "Filter by album (substring, case-insensitive)" },
          genre: { type: "string", description: "Filter by genre (substring, case-insensitive)" },
          year: { type: "string", description: "Filter by exact year" },
          codec: { type: "string", description: "Filter by codec (e.g. FLAC, MP3)" },
          missingTitle: { type: "boolean", description: "Find tracks missing title" },
          missingArtist: { type: "boolean", description: "Find tracks missing artist" },
          missingAlbum: { type: "boolean", description: "Find tracks missing album" },
          missingYear: { type: "boolean", description: "Find tracks missing year" },
          missingGenre: { type: "boolean", description: "Find tracks missing genre" },
          missingCover: { type: "boolean", description: "Find tracks missing cover art" },
          hasDuplicates: { type: "boolean", description: "Find tracks with duplicate title+artist+album" },
        },
        required: [],
      },
      isReadOnly: true,
      executor: async (args) => {
        const results = safeQueryService!.findTracks(args);
        const limited = results.slice(0, 20);

        let summary: string;
        if (results.length === 0) {
          summary = "No tracks match the query.";
        } else {
          summary = `Found ${results.length} track(s):\n`;
          for (const t of limited) {
            summary += `  - ${t.title ?? "?"} by ${t.artist ?? "?"} (${t.album ?? "?"}) [${t.codec}]\n`;
          }
          if (results.length > 20) {
            summary += `  ... and ${results.length - 20} more`;
          }
        }

        return { ok: true, summary, data: { total: results.length, tracks: limited } };
      },
    },
    {
      name: "tracks.inspect",
      description: "Inspect one or more tracks by path. Pass selectedTrackPaths to inspect the current selection, or provide explicit paths.",
      inputSchema: {
        type: "object",
        properties: {
          paths: {
            type: "array",
            items: { type: "string" },
            description: "Track paths to inspect. If empty, uses current selection.",
          },
        },
        required: [],
      },
      isReadOnly: true,
      executor: async (args) => {
        const paths = (args.paths as string[]) ?? currentAppState.selectedTrackPaths;
        if (paths.length === 0) {
          return { ok: true, summary: "No tracks specified and no current selection." };
        }

        const limited = paths.slice(0, 10);
        let summary = `Inspecting ${limited.length} track(s):\n`;

        for (const p of limited) {
          const track = currentAppState.tracks.find((t) => t.path === p);
          if (!track) {
            summary += `  - ${p}: not found in library\n`;
            continue;
          }
          summary += `  - "${track.title}" by ${track.artist ?? "?"}\n`;
          summary += `    Album: ${track.album ?? "?"} (${track.albumArtist ?? "?"})\n`;
          summary += `    Track ${track.trackNumber ?? "?"}/${track.trackTotal ?? "?"} | Year: ${track.year ?? "?"} | Genre: ${track.genre ?? "?"}\n`;
          summary += `    Codec: ${track.codec} | Duration: ${Math.round(track.duration)}s | Cover: ${track.hasCover ? "yes" : "no"}\n`;
        }

        if (paths.length > 10) {
          summary += `  (${paths.length - 10} more not shown)`;
        }

        return { ok: true, summary };
      },
    },
    {
      name: "albums.inspect",
      description: "Inspect an album by path. Provide the album path, or leave empty for the active album.",
      inputSchema: {
        type: "object",
        properties: {
          path: { type: "string", description: "Album directory path. If empty, uses active album." },
        },
        required: [],
      },
      isReadOnly: true,
      executor: async (args) => {
        const albumPath = (args.path as string) ?? currentAppState.activeAlbumPath;
        if (!albumPath) {
          return { ok: true, summary: "No album path specified and no active album." };
        }

        const albumInfo = currentAppState.albums.find((a) => a.path === albumPath);
        const albumTracks = currentAppState.tracks.filter((t) =>
          t.path.startsWith(albumPath),
        );

        let summary = `Album: ${albumInfo?.name ?? "?"}\n`;
        summary += `Artist hint: ${albumInfo?.artistHint ?? "?"}\n`;
        summary += `Tracks: ${albumTracks.length}\n`;

        if (albumTracks.length > 0) {
          summary += `\nTracks:\n`;
          for (const t of albumTracks) {
            summary += `  ${t.trackNumber != null ? `${t.trackNumber}.` : "-"} ${t.title ?? "?"} (${t.artist ?? "?"})\n`;
          }
        }

        return { ok: true, summary, data: { albumInfo, tracks: albumTracks } };
      },
    },
    {
      name: "query.metadata",
      description: "Run typed read-only queries against the library metadata. Get aggregate counts, find missing tags, or detect duplicates.",
      inputSchema: {
        type: "object",
        properties: {
          aggregate: { type: "boolean", description: "If true, return aggregate counts by album, artist, genre, year, codec, and tag completeness" },
          missingTags: { type: "string", description: "One of: title, artist, album, year, genre — get counts of tracks missing this tag" },
          duplicates: { type: "boolean", description: "If true, find tracks with duplicate title+artist+album" },
        },
        required: [],
      },
      isReadOnly: true,
      executor: async (args) => {
        if (args.aggregate) {
          const agg = safeQueryService!.aggregate();
          const summary = [
            `Total tracks: ${agg.totalTracks}`,
            `Albums: ${agg.totalAlbums}, Artists: ${agg.totalArtists}, Genres: ${agg.totalGenres}`,
            `\nTag completeness:`,
            `  Title: ${agg.tagCompleteness.title}%`,
            `  Artist: ${agg.tagCompleteness.artist}%`,
            `  Album: ${agg.tagCompleteness.album}%`,
            `  Year: ${agg.tagCompleteness.year}%`,
            `  Genre: ${agg.tagCompleteness.genre}%`,
            `\nTop artists: ${Object.entries(agg.byArtist).sort((a, b) => b[1] - a[1]).slice(0, 10).map(([a, n]) => `${a} (${n})`).join(", ")}`,
            `\nGenres: ${Object.entries(agg.byGenre).sort((a, b) => b[1] - a[1]).slice(0, 10).map(([g, n]) => `${g} (${n})`).join(", ")}`,
            `\nCodecs: ${Object.entries(agg.byCodec).map(([c, n]) => `${c} (${n})`).join(", ")}`,
          ].join("\n");
          return { ok: true, summary, data: agg };
        }

        if (args.missingTags) {
          const field = String(args.missingTags).toLowerCase();
          let results: TrackData[];
          switch (field) {
            case "title": results = safeQueryService!.findTracks({ missingTitle: true }); break;
            case "artist": results = safeQueryService!.findTracks({ missingArtist: true }); break;
            case "album": results = safeQueryService!.findTracks({ missingAlbum: true }); break;
            case "year": results = safeQueryService!.findTracks({ missingYear: true }); break;
            case "genre": results = safeQueryService!.findTracks({ missingGenre: true }); break;
            default: return { ok: true, summary: `Unknown missing-tag field: ${field}. Use: title, artist, album, year, genre` };
          }

          const limited = results.slice(0, 20);
          let summary = `Found ${results.length} track(s) missing ${field}:\n`;
          for (const t of limited) {
            summary += `  - ${t.title ?? "(no title)"} by ${t.artist ?? "?"} (${t.album ?? "?"})\n`;
          }
          if (results.length > 20) summary += `  ... and ${results.length - 20} more`;
          return { ok: true, summary, data: { total: results.length, tracks: limited } };
        }

        if (args.duplicates) {
          const results = safeQueryService!.findTracks({ hasDuplicates: true });
          let summary = results.length > 0
            ? `Found ${results.length} tracks with potential duplicates:\n`
            : "No duplicate tracks found.";
          return { ok: true, summary, data: { total: results.length, tracks: results.slice(0, 20) } };
        }

        return { ok: true, summary: "Specify at least one query option: aggregate, missingTags, or duplicates." };
      },
    },
    {
      name: "query.datasetStatus",
      description: "Check if the local dataset index is available and return status information.",
      inputSchema: {
        type: "object",
        properties: {},
        required: [],
      },
      isReadOnly: true,
      executor: async () => {
        return { ok: true, summary: "Dataset status query available but local dataset reader not yet integrated." };
      },
    },
    {
      name: "api.musicbrainzSearch",
      description: "Search MusicBrainz for a release by artist and album. Returns summarized results.",
      inputSchema: {
        type: "object",
        properties: {
          query: { type: "string", description: "Search query, e.g. 'artist:Radiohead album:OK Computer'" },
          limit: { type: "number", description: "Max results (default: 5)" },
        },
        required: ["query"],
      },
      isReadOnly: true,
      executor: async (args) => {
        const result = await safeApiRequestService!.execute({
          preset: "musicbrainzSearch",
          params: {
            query: String(args.query),
            limit: String(args.limit ?? 5),
          },
        });
        return result.ok
          ? { ok: true, summary: result.summary, data: result.data }
          : { ok: false, summary: result.summary, error: result.error };
      },
    },
    {
      name: "api.discogsSearch",
      description: "Search Discogs for a release by query. Requires Discogs token to be configured.",
      inputSchema: {
        type: "object",
        properties: {
          query: { type: "string", description: "Search query" },
          type: { type: "string", description: "Type: release, master, artist, label (default: release)" },
          limit: { type: "number", description: "Max results (default: 5)" },
        },
        required: ["query"],
      },
      isReadOnly: true,
      executor: async (args) => {
        const result = await safeApiRequestService!.execute({
          preset: "discogsSearch",
          params: {
            query: String(args.query),
            type: String(args.type ?? "release"),
            limit: String(args.limit ?? 5),
          },
        });
        return result.ok
          ? { ok: true, summary: result.summary, data: result.data }
          : { ok: false, summary: result.summary, error: result.error };
      },
    },
    {
      name: "api.lyricsSearch",
      description: "Search for lyrics of a specific track. Requires lyrics host to be configured.",
      inputSchema: {
        type: "object",
        properties: {
          artist: { type: "string", description: "Artist name" },
          title: { type: "string", description: "Track title" },
        },
        required: ["artist", "title"],
      },
      isReadOnly: true,
      executor: async (args) => {
        const result = await safeApiRequestService!.execute({
          preset: "lyricsSearch",
          params: {
            artist: String(args.artist),
            title: String(args.title),
          },
        });
        return result.ok
          ? { ok: true, summary: result.summary, data: result.data }
          : { ok: false, summary: result.summary, error: result.error };
      },
    },
  ];
}

// ── Build mutating tools (preview-first) ─────────────────────────

function standardTagSchema(): Record<StandardTagField, Record<string, unknown>> {
  return {
    title: { type: "string" },
    artist: { type: "string" },
    album: { type: "string" },
    albumArtist: { type: "string" },
    year: { type: "string" },
    trackNumber: { type: "number" },
    trackTotal: { type: "number" },
    discNumber: { type: "number" },
    discTotal: { type: "number" },
    genre: { type: "string" },
    composer: { type: "string" },
    comment: { type: "string" },
    lyrics: { type: "string" },
    compilation: { type: "boolean" },
  };
}

export function resolveTargetPathsForState(
  state: Pick<typeof currentAppState, "activeAlbumPath" | "selectedTrackPaths" | "tracks">,
  targetScope: TaskTargetScope,
  explicitPaths?: string[],
): { paths: string[]; description: string } {
  switch (targetScope) {
    case "selected":
      return {
        paths: state.selectedTrackPaths,
        description: `${state.selectedTrackPaths.length} selected track(s)`,
      };
    case "active_album": {
      const paths = state.tracks
        .filter((track) => state.activeAlbumPath && isInsideDirectory(track.path, state.activeAlbumPath))
        .map((track) => track.path);
      return {
        paths,
        description: `active album "${state.activeAlbumPath ?? "?"}" (${paths.length} track(s))`,
      };
    }
    case "library":
      return {
        paths: state.tracks.map((track) => track.path),
        description: `entire library (${state.tracks.length} track(s))`,
      };
    case "explicit_paths": {
      const requested = explicitPaths ?? [];
      const loadedPaths = new Set(state.tracks.map((track) => track.path));
      const paths = requested.filter((trackPath) => loadedPaths.has(trackPath));
      return {
        paths,
        description: `${paths.length} explicit track path(s)`,
      };
    }
  }
}

function resolveTargetPaths(
  targetScope: TaskTargetScope,
  explicitPaths?: string[],
): { paths: string[]; description: string } {
  return resolveTargetPathsForState(currentAppState, targetScope, explicitPaths);
}

function isInsideDirectory(filePath: string, directoryPath: string): boolean {
  const relative = path.relative(path.resolve(directoryPath), path.resolve(filePath));
  return relative !== "" && !relative.startsWith("..") && !path.isAbsolute(relative);
}

function buildStandardFields(
  updates: Record<string, unknown> | undefined,
  removes: string[],
): WriteFields {
  const fields: Record<string, unknown> = {};
  for (const [field, value] of Object.entries(updates ?? {})) {
    if (STANDARD_TAG_FIELD_SET.has(field)) {
      fields[field] = value;
    }
  }
  for (const field of removes) {
    if (STANDARD_TAG_FIELD_SET.has(field)) {
      fields[field] = null;
    }
  }
  return fields as WriteFields;
}

function metadataBatchSummary(
  standardActionCount: number,
  extraActionCount: number,
  affectedTrackCount: number,
): string {
  const parts: string[] = [];
  if (standardActionCount > 0) parts.push(`${standardActionCount} standard tag field(s)`);
  if (extraActionCount > 0) parts.push(`${extraActionCount} extra tag field(s)`);
  return parts.length > 0
    ? `Update ${parts.join(" and ")} across ${affectedTrackCount} track(s)`
    : "No metadata changes needed";
}

function metadataPreviewActions(input: {
  standardActions: NonNullable<Awaited<ReturnType<TrackTagService["planTagUpdates"]>>["actions"]>;
  extraActions: NonNullable<Awaited<ReturnType<ExtraTagService["planExtraTagUpdates"]>>["actions"]>;
}): AssistantActionBatch["actions"] {
  return [
    ...input.standardActions.map((action) => ({
      tagKind: "standard" as const,
      trackPath: action.trackPath,
      field: action.field,
      operation: "set",
      oldValue: action.oldValue,
      newValue: action.newValue,
    })),
    ...input.extraActions.map((action) => ({
      tagKind: "extra" as const,
      trackPath: action.trackPath,
      field: action.key,
      operation: action.operation,
      oldValue: action.oldValue,
      newValue: action.newValue,
    })),
  ];
}

function buildMutatingTools(): AssistantToolDef[] {
  return [
    {
      name: "edit_metadata",
      description: "Composite macro: plan standard tag edits and extra tag edits in one preview action batch.",
      inputSchema: {
        type: "object",
        properties: {
          target_scope: {
            type: "string",
            enum: ["selected", "active_album", "explicit_paths"],
            description: "Target tracks. Use explicit_paths with paths for specific tracks.",
          },
          paths: {
            type: "array",
            items: { type: "string" },
            description: "Track paths used when target_scope is explicit_paths.",
          },
          standard_updates: {
            type: "object",
            properties: standardTagSchema(),
            required: [],
            description: "Known standard tag fields to set.",
          },
          standard_removes: {
            type: "array",
            items: {
              type: "string",
              enum: [...STANDARD_TAG_FIELDS],
            },
            description: "Known standard tag fields to clear.",
          },
          extra_upserts: {
            type: "array",
            items: {
              type: "object",
              properties: {
                key: { type: "string" },
                value: { type: "string" },
              },
              required: ["key", "value"],
            },
            description: "Custom/extra tags to add or update.",
          },
          extra_removes: {
            type: "array",
            items: { type: "string" },
            description: "Custom/extra tag keys to remove.",
          },
        },
        required: ["target_scope"],
      },
      isReadOnly: false,
      riskLevel: "low",
      executor: async (args) => {
        if (!trackTagService || !extraTagService) {
          return { ok: false, summary: "Metadata services not initialized", error: "SERVICE_NOT_INITIALIZED" };
        }

        const targetScope = String(args.target_scope) as MetadataTargetScope;
        const { paths: targetPaths, description } = resolveTargetPaths(
          targetScope,
          args.paths as string[] | undefined,
        );

        if (targetPaths.length === 0) {
          return { ok: true, summary: `No tracks found for target_scope "${targetScope}".` };
        }

        const standardFields = buildStandardFields(
          args.standard_updates as Record<string, unknown> | undefined,
          (args.standard_removes as string[]) ?? [],
        );
        const extraUpserts = (args.extra_upserts as ExtraTagUpdate[]) ?? [];
        const extraRemoves = (args.extra_removes as string[]) ?? [];

        if (
          Object.keys(standardFields).length === 0 &&
          extraUpserts.length === 0 &&
          extraRemoves.length === 0
        ) {
          return { ok: true, summary: "No metadata changes specified." };
        }

        const standardPlan = Object.keys(standardFields).length > 0
          ? await trackTagService.planTagUpdates(
            targetPaths.map((trackPath) => ({ trackPath, fields: standardFields })),
          )
          : null;
        const extraPlan = extraUpserts.length > 0 || extraRemoves.length > 0
          ? await extraTagService.planExtraTagUpdates(
            targetPaths.map((trackPath) => ({
              trackPath,
              upserts: extraUpserts,
              removes: extraRemoves,
            })),
          )
          : null;

        const standardActions = standardPlan?.actions ?? [];
        const extraActions = extraPlan?.actions ?? [];
        const affectedTracks = new Set([
          ...standardActions.map((action) => action.trackPath),
          ...extraActions.map((action) => action.trackPath),
        ]);
        const summary = metadataBatchSummary(
          standardActions.length,
          extraActions.length,
          affectedTracks.size,
        );

        if (!currentRuntime) {
          return { ok: true, summary };
        }

        const batch = currentRuntime.createActionBatch({
          kind: "metadata-update",
          title: `Edit metadata for ${description}`,
          summary,
          riskLevel: "low",
          actions: metadataPreviewActions({ standardActions, extraActions }),
          reversible: true,
        });

        return {
          ok: true,
          summary: `Preview created (${batch.id}): ${summary}. Approve in the assistant panel to apply.`,
          pendingActionBatchId: batch.id,
          data: { batch, standardPlan, extraPlan },
        };
      },
    },
    {
      name: "organize_files",
      description: "Composite macro: scan direct child files in source_dir, group or filter them by extension, pattern, date_created, or size, then create a preview move batch under target_dir_name.",
      inputSchema: {
        type: "object",
        properties: {
          source_dir: {
            type: "string",
            description: "Directory to scan. Must be inside the current library root.",
          },
          criterion: {
            type: "string",
            enum: ["extension", "pattern", "date_created", "size"],
            description: "Grouping/filtering criterion.",
          },
          pattern_string: {
            type: "string",
            description: "Required for pattern glob matching; optional comma/space-separated extension filter for extension.",
          },
          target_dir_name: {
            type: "string",
            description: "Folder name to create inside source_dir for organized files.",
          },
        },
        required: ["source_dir", "criterion", "target_dir_name"],
      },
      isReadOnly: false,
      riskLevel: "medium",
      executor: async (args) => {
        if (!folderOrganizerService || !currentAppState.libraryPath) {
          return { ok: false, summary: "Folder organizer or library path not configured", error: "SERVICE_NOT_INITIALIZED" };
        }

        const criterion = String(args.criterion);
        if (criterion === "pattern" && !String(args.pattern_string ?? "").trim()) {
          return {
            ok: false,
            summary: "pattern_string is required when criterion is pattern",
            error: "MISSING_PATTERN_STRING",
          };
        }

        const plan = folderOrganizerService.planOrganizeFiles({
          sourceDir: String(args.source_dir),
          criterion: criterion as "extension" | "pattern" | "date_created" | "size",
          patternString: args.pattern_string === undefined ? undefined : String(args.pattern_string),
          targetDirName: String(args.target_dir_name),
        });

        if (!currentRuntime) {
          return { ok: true, summary: plan.summary };
        }

        const batch = currentRuntime.createActionBatch({
          kind: "folder-move",
          title: `Organize ${plan.moves.length} file(s) by ${criterion}`,
          summary: plan.summary,
          riskLevel: "medium",
          actions: folderPlanToActions(plan),
          reversible: plan.reversible,
        });

        return {
          ok: true,
          summary: `Preview created (${batch.id}): ${plan.summary}. Approve in the assistant panel to apply.`,
          pendingActionBatchId: batch.id,
          data: { batch, plan },
        };
      },
    },
    {
      name: "run_library_task",
      description: "Composite macro: plan an auto-tag or audit task for selected tracks, active album, entire library, or explicit track paths.",
      inputSchema: {
        type: "object",
        properties: {
          task: {
            type: "string",
            enum: ["auto_tag", "audit"],
            description: "Library task to run.",
          },
          target_scope: {
            type: "string",
            enum: ["selected", "active_album", "library", "explicit_paths"],
            description: "Task scope. Use explicit_paths with paths for specific tracks.",
          },
          paths: {
            type: "array",
            items: { type: "string" },
            description: "Track paths used when target_scope is explicit_paths.",
          },
        },
        required: ["task", "target_scope"],
      },
      isReadOnly: false,
      riskLevel: "medium",
      executor: async (args) => {
        const task = String(args.task) as LibraryTaskKind;
        const targetScope = String(args.target_scope) as TaskTargetScope;
        const { paths, description } = resolveTargetPaths(
          targetScope,
          args.paths as string[] | undefined,
        );

        if (paths.length === 0) {
          return { ok: true, summary: `No tracks found for target_scope "${targetScope}".` };
        }

        const isAutoTag = task === "auto_tag";
        const title = `${isAutoTag ? "Auto-tag" : "Audit"} ${description}`;
        const summary = isAutoTag
          ? `Run auto-tagging on ${description}. This may modify tags on ${paths.length} file(s).`
          : `Run audit on ${description} (${paths.length} track(s)).`;

        if (!currentRuntime) {
          return { ok: true, summary };
        }

        const batch = currentRuntime.createActionBatch({
          kind: isAutoTag ? "auto-tag-run" : "audit-run",
          title,
          summary,
          riskLevel: "medium",
          actions: paths.map((trackPath) => ({
            trackPath,
            description: `${title}: ${path.basename(trackPath)}`,
          })),
          reversible: isAutoTag,
        });

        return {
          ok: true,
          summary: `${title} preview created (${batch.id}).`,
          pendingActionBatchId: batch.id,
          data: { batch, task, targetScope, trackCount: paths.length, paths },
        };
      },
    },
  ];
}

function folderPlanToActions(plan: FolderPlanForBatch) {
  return [
    ...plan.moves.map((move) => ({
      sourcePath: move.sourcePath,
      destinationPath: move.destinationPath,
      description: "move",
    })),
    ...plan.noops.map((noop) => ({
      sourcePath: noop.sourcePath,
      destinationPath: noop.destinationPath,
      skipReason: noop.skipReason ?? "Already in place",
      description: "noop",
    })),
    ...plan.skipped.map((skipped) => ({
      sourcePath: skipped.sourcePath,
      skipReason: skipped.skipReason ?? "Skipped",
      description: "skip",
    })),
  ];
}

export function metadataBatchToStandardUpdates(
  batch: AssistantActionBatch,
): TagUpdateInstruction[] {
  const updatesByTrack = new Map<string, WriteFields>();

  for (const action of batch.actions) {
    if (action.tagKind !== "standard" || !action.trackPath || !action.field) {
      continue;
    }

    const fields = updatesByTrack.get(action.trackPath) ?? {};
    (fields as Record<string, unknown>)[action.field] = action.newValue ?? null;
    updatesByTrack.set(action.trackPath, fields);
  }

  return Array.from(updatesByTrack.entries()).map(([trackPath, fields]) => ({
    trackPath,
    fields,
  }));
}

export function metadataBatchToExtraInputs(
  batch: AssistantActionBatch,
): ExtraTagPlanInput[] {
  const updatesByTrack = new Map<string, { upserts: ExtraTagUpdate[]; removes: string[] }>();

  for (const action of batch.actions) {
    if (action.tagKind !== "extra" || !action.trackPath || !action.field) {
      continue;
    }

    const entry = updatesByTrack.get(action.trackPath) ?? { upserts: [], removes: [] };
    if (action.operation === "remove") {
      entry.removes.push(action.field);
    } else if (action.newValue != null) {
      entry.upserts.push({ key: action.field, value: action.newValue });
    }
    updatesByTrack.set(action.trackPath, entry);
  }

  return Array.from(updatesByTrack.entries()).map(([trackPath, operations]) => ({
    trackPath,
    upserts: operations.upserts,
    removes: operations.removes,
  }));
}

function legacyExtraTagBatchToInputs(batch: AssistantActionBatch): ExtraTagPlanInput[] {
  const updatesByTrack = new Map<string, { upserts: ExtraTagUpdate[]; removes: string[] }>();

  for (const action of batch.actions) {
    if (!action.trackPath || !action.field) continue;

    const entry = updatesByTrack.get(action.trackPath) ?? { upserts: [], removes: [] };
    if (action.operation === "remove") {
      entry.removes.push(action.field);
    } else if (action.operation === "upsert" && action.newValue != null) {
      entry.upserts.push({ key: action.field, value: action.newValue });
    }
    updatesByTrack.set(action.trackPath, entry);
  }

  return Array.from(updatesByTrack.entries()).map(([trackPath, operations]) => ({
    trackPath,
    upserts: operations.upserts,
    removes: operations.removes,
  }));
}

async function applyMetadataUpdateBatch(
  batch: AssistantActionBatch,
  actionBatchId: string,
) {
  if (!currentRuntime) throw new Error("Assistant runtime not initialized");
  if (!trackTagService) throw new Error("TrackTagService not initialized");
  if (!extraTagService) throw new Error("ExtraTagService not initialized");

  const standardUpdates = metadataBatchToStandardUpdates(batch);
  const extraInputs = metadataBatchToExtraInputs(batch);

  const undoSnapshots = standardUpdates.length > 0
    ? await trackTagService.buildUndoSnapshots(standardUpdates)
    : [];
  const extraUndoSnapshots = extraInputs.length > 0
    ? await extraTagService.buildUndoExtraTags(extraInputs)
    : [];
  const standardResults = standardUpdates.length > 0
    ? await trackTagService.applyTagUpdates(standardUpdates)
    : [];
  const extraResults = extraInputs.length > 0
    ? await extraTagService.applyExtraTagUpdates(extraInputs)
    : [];

  const failedStandard = standardResults.filter((result) => !result.success);
  const failedExtra = extraResults.filter((result) => !result.success);
  const failedCount = failedStandard.length + failedExtra.length;
  if (failedCount > 0) {
    currentRuntime.markBatchFailed(actionBatchId, `Failed to update ${failedCount} track(s)`);
    return {
      success: false,
      error: `Failed to update ${failedCount} track(s)`,
      results: { standard: failedStandard, extra: failedExtra },
      undoSnapshots,
      extraUndoSnapshots,
    };
  }

  currentRuntime.markBatchApplied(actionBatchId);
  return {
    success: true,
    results: { standard: standardResults, extra: extraResults },
    undoSnapshots,
    extraUndoSnapshots,
  };
}

async function applyLegacyExtraTagBatch(
  batch: AssistantActionBatch,
  actionBatchId: string,
) {
  if (!currentRuntime) throw new Error("Assistant runtime not initialized");
  if (!extraTagService) throw new Error("ExtraTagService not initialized");

  const inputs = legacyExtraTagBatchToInputs(batch);
  const extraUndoSnapshots = await extraTagService.buildUndoExtraTags(inputs);
  const results = await extraTagService.applyExtraTagUpdates(inputs);
  const failed = results.filter((result) => !result.success);
  if (failed.length > 0) {
    currentRuntime.markBatchFailed(actionBatchId, `Failed to update ${failed.length} track(s)`);
    return {
      success: false,
      error: `Failed to update ${failed.length} track(s)`,
      results: failed,
      extraUndoSnapshots,
    };
  }
  currentRuntime.markBatchApplied(actionBatchId);
  return { success: true, results, extraUndoSnapshots };
}

// ── Create or reuse runtime ──────────────────────────────────────

function getOrCreateRuntime(config: { apiKey: string; model?: string; autonomous?: boolean }): AssistantRuntime {
  if (currentRuntime) {
    currentRuntime.setAutonomous(config.autonomous ?? false);
    return currentRuntime;
  }

  const runner = new LlmTaskRunner({
    apiKey: config.apiKey,
    model: config.model,
  });

  currentRegistry = new AssistantToolRegistry();
  const readOnlyTools = buildReadOnlyTools();
  const mutatingTools = buildMutatingTools();
  currentRegistry.registerAll([...readOnlyTools, ...mutatingTools]);

  const runtime = new AssistantRuntime(runner, currentRegistry, config.autonomous ?? false);

  runtime.onEvent((event) => {
    broadcastEvent(event);
  });

  currentRuntime = runtime;
  return runtime;
}

// ── IPC Handler Registration ─────────────────────────────────────

export function registerAssistantHandlers(): void {
  // ── assistant:send ──────────────────────────────────────────────
  ipcMain.handle(
    "assistant:send",
    async (
      _event,
      input: {
        message: string;
        apiKey: string;
        model?: string;
        libraryPath?: string | null;
        activeAlbumPath?: string | null;
        selectedTrackPaths?: string[];
        tracks?: TrackData[];
        albums?: AlbumInfo[];
        autonomous?: boolean;
      },
    ) => {
      try {
        // Validate API key and model — use stored values from main-process config,
        // NOT the renderer's versions (the renderer only gets a redacted API key)
        if (!storedApiKey) {
          throw new Error(
            "LLM API key is not configured. Set it in Settings or via the " +
            "LLM_API_KEY environment variable.",
          );
        }
        if (!storedModel) {
          throw new Error(
            "LLM model is not configured. Set it in Settings or via the " +
            "LLM_MODEL environment variable.",
          );
        }

        // Update shared state
        currentAppState = {
          libraryPath: input.libraryPath ?? currentAppState.libraryPath,
          activeAlbumPath: input.activeAlbumPath ?? currentAppState.activeAlbumPath,
          selectedTrackPaths: input.selectedTrackPaths ?? currentAppState.selectedTrackPaths,
          tracks: input.tracks ?? currentAppState.tracks,
          albums: input.albums ?? currentAppState.albums,
          autonomous: input.autonomous ?? currentAppState.autonomous,
        };
        setAssistantAppState(currentAppState);

        // Get or create runtime using the stored (real) API key and model from config.
        // Ignore input.model — model must be configured via config.yaml or env var.
        const runtime = getOrCreateRuntime({
          apiKey: storedApiKey,
          model: storedModel,
          autonomous: currentAppState.autonomous,
        });

        // Send the user message
        const event = await runtime.send(input.message);
        return event;
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        const event: AssistantEvent = {
          sessionId: currentRuntime?.getSessionId() ?? "none",
          type: "error",
          message,
        };
        broadcastEvent(event);
        return event;
      }
    },
  );

  // ── assistant:cancel ───────────────────────────────────────────
  ipcMain.handle("assistant:cancel", async () => {
    if (currentRuntime) {
      currentRuntime.cancel();
    }
  });

  // ── assistant:apply-actions ────────────────────────────────────
  ipcMain.handle(
    "assistant:apply-actions",
    async (_event, actionBatchId: string) => {
      if (!currentRuntime) {
        return { success: false, error: "No active assistant session" };
      }

      const batch = currentRuntime.getActionBatch(actionBatchId);
      if (!batch) {
        return { success: false, error: `Action batch not found: ${actionBatchId}` };
      }

      if (batch.status !== "pending") {
        return { success: false, error: `Batch already ${batch.status}` };
      }

      try {
        // Apply based on batch kind
        switch (batch.kind) {
          case "metadata-update":
            return await applyMetadataUpdateBatch(batch, actionBatchId);

          case "tag-update": {
            if (!trackTagService) throw new Error("TrackTagService not initialized");
            const updates = batch.actions
              .filter((a) => a.trackPath && a.field)
              .map((a) => ({
                trackPath: a.trackPath!,
                fields: { [a.field!]: a.newValue ?? null } as any,
              }));
            // Capture undo snapshots BEFORE applying
            const undoSnapshots = await trackTagService.buildUndoSnapshots(updates);
            const results = await trackTagService.applyTagUpdates(updates);
            const failed = results.filter((r) => !r.success);
            if (failed.length > 0) {
              currentRuntime.markBatchFailed(actionBatchId, `Failed to update ${failed.length} track(s)`);
              return { success: false, error: `Failed to update ${failed.length} track(s)`, results: failed, undoSnapshots };
            }
            currentRuntime.markBatchApplied(actionBatchId);
            return { success: true, results, undoSnapshots };
          }

          case "extra-tag-update":
            return await applyLegacyExtraTagBatch(batch, actionBatchId);

          case "folder-move": {
            if (!folderOrganizerService) throw new Error("FolderOrganizerService not initialized");
            const moves = batch.actions
              .filter((a) => a.sourcePath && a.destinationPath && !a.skipReason)
              .map((a) => ({
                sourcePath: a.sourcePath!,
                destinationPath: a.destinationPath!,
              }));

            // Rebuild the plan from actions
            const plan = {
              kind: "folder-move" as const,
              summary: `Move ${moves.length} files`,
              moves: moves.map((m) => ({
                sourcePath: m.sourcePath,
                destinationPath: m.destinationPath,
              })),
              noops: [] as Array<{ sourcePath: string; destinationPath: string; skipReason?: string }>,
              skipped: [] as Array<{ sourcePath: string; destinationPath: string; skipReason?: string }>,
              affectedTracks: moves.length,
              reversible: true,
            };

            const { results } = await folderOrganizerService.applyMoves(plan);
            const failed = results.filter((r) => !r.success);
            if (failed.length > 0) {
              currentRuntime.markBatchFailed(actionBatchId, `Failed to move ${failed.length} file(s)`);
              return { success: false, error: `Failed to move ${failed.length} file(s)`, results: failed };
            }
            currentRuntime.markBatchApplied(actionBatchId);
            return { success: true, results, manifest: results.map((r) => ({ from: r.sourcePath, to: r.destinationPath })) };
          }

          case "auto-tag-run": {
            // Delegate to existing auto-tag flow — the renderer handles this
            currentRuntime.markBatchApplied(actionBatchId);
            return {
              success: true,
              message: "Auto-tag will be triggered by the renderer",
              task: "auto_tag",
              trackPaths: batch.actions
                .map((action) => action.trackPath)
                .filter((trackPath): trackPath is string => Boolean(trackPath)),
            };
          }

          case "audit-run": {
            // Delegate to existing audit flow
            currentRuntime.markBatchApplied(actionBatchId);
            return {
              success: true,
              message: "Audit will be triggered by the renderer",
              task: "audit",
              trackPaths: batch.actions
                .map((action) => action.trackPath)
                .filter((trackPath): trackPath is string => Boolean(trackPath)),
            };
          }

          default:
            currentRuntime.markBatchFailed(actionBatchId, `Unknown batch kind: ${batch.kind}`);
            return { success: false, error: `Unknown batch kind: ${batch.kind}` };
        }
      } catch (error) {
        const msg = error instanceof Error ? error.message : String(error);
        currentRuntime.markBatchFailed(actionBatchId, msg);
        return { success: false, error: msg };
      }
    },
  );

  // ── assistant:reject-actions ───────────────────────────────────
  ipcMain.handle("assistant:reject-actions", async (_event, actionBatchId: string) => {
    if (currentRuntime) {
      currentRuntime.markBatchRejected(actionBatchId);
    }
  });

  // ── assistant:get-batches ──────────────────────────────────────
  ipcMain.handle("assistant:get-batches", async () => {
    if (!currentRuntime) return [];
    return currentRuntime.getPendingBatches();
  });

  // ── assistant:init-services ────────────────────────────────────
  ipcMain.handle(
    "assistant:init-services",
    async (
      _event,
      config: {
        apiKey: string;
        model?: string;
        discogsToken?: string | null;
        lyricsHost?: string | null;
        libraryPath?: string | null;
      },
    ) => {
      initializeAssistantServices(config);
    },
  );
}
