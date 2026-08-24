/**
 * Fetch-backed adapter for the renderer-neutral `DesktopAPI` contract.
 *
 * The browser keeps the same method names and payloads as the Tauri adapter;
 * only the transport changes from `invoke` to authenticated JSON POSTs. Native
 * dialogs, menus, and self-updates are intentionally explicit web-runtime
 * errors because those affordances belong to the browser or the container.
 */

import type { DesktopAPI } from "./desktop-api";

export interface WebDesktopApiOptions {
  fetch?: typeof globalThis.fetch;
  baseUrl?: string;
  eventSource?: (url: string) => WebEventSource;
}

export interface WebAuthOptions {
  fetch?: typeof globalThis.fetch;
  baseUrl?: string;
}

export interface WebSession {
  authenticated: boolean;
}

export interface WebEventSource {
  addEventListener: (type: string, listener: (event: WebEventMessage) => void) => void;
  close: () => void;
}

interface WebEventMessage {
  data: string;
  lastEventId?: string;
}

const WEB_EVENT_CHANNELS = [
  "auto-tag:event",
  "tracks:write-event",
  "audit:event",
  "assistant:event",
  "debug:log",
  "soundrobe:replay-gap",
] as const;

function toError(reason: unknown): Error {
  if (reason instanceof Error) return reason;
  if (typeof reason === "string" && reason.length > 0) return new Error(reason);
  if (reason && typeof reason === "object" && "message" in reason) {
    return new Error(String((reason as { message: unknown }).message || reason));
  }
  return new Error(String(reason ?? "Web command failed"));
}

function unsupportedWebAction(action: string): never {
  throw new Error(`${action} is unavailable in the web runtime`);
}

class WebEventBus {
  private source: WebEventSource | null = null;
  private lastEventId: string | null = null;
  private readonly listeners = new Map<string, Set<(payload: unknown) => void>>();

  constructor(
    private readonly url: string,
    private readonly createSource: (url: string) => WebEventSource,
  ) {}

  subscribe<T>(channel: string, callback: (payload: T) => void): () => void {
    let channelListeners = this.listeners.get(channel);
    if (!channelListeners) {
      channelListeners = new Set();
      this.listeners.set(channel, channelListeners);
    }
    channelListeners.add(callback as (payload: unknown) => void);
    this.ensureSource();

    return () => {
      channelListeners?.delete(callback as (payload: unknown) => void);
      if (channelListeners?.size === 0) {
        this.listeners.delete(channel);
      }
      if (this.listeners.size === 0) {
        this.source?.close();
        this.source = null;
      }
    };
  }

  private ensureSource(): void {
    if (this.source) return;
    const channels = encodeURIComponent(WEB_EVENT_CHANNELS.join(","));
    const cursor = this.lastEventId
      ? `&after=${encodeURIComponent(this.lastEventId)}`
      : "";
    this.source = this.createSource(
      `${this.url}/api/v1/events?channels=${channels}${cursor}`,
    );
    for (const channel of WEB_EVENT_CHANNELS) {
      this.source.addEventListener(channel, (event) => {
        this.lastEventId = event.lastEventId ?? this.lastEventId;
        let payload: unknown;
        try {
          payload = JSON.parse(event.data) as unknown;
        } catch (reason) {
          console.error(`[soundrobe] invalid SSE payload for "${channel}":`, reason);
          return;
        }
        for (const listener of this.listeners.get(channel) ?? []) {
          listener(payload);
        }
      });
    }
    this.source.addEventListener("soundrobe:replay-gap", (event) => {
      this.lastEventId = event.lastEventId ?? this.lastEventId;
      console.error("[soundrobe] SSE replay gap; refresh is required:", event.data);
      const source = this.source;
      this.source = null;
      source?.close();
    });
  }
}

function commandUrl(baseUrl: string, channel: string): string {
  return `${baseUrl}/api/v1/commands/${encodeURIComponent(channel)}`;
}

function authUrl(baseUrl: string, action: "login" | "logout" | "session"): string {
  return `${baseUrl}/api/v1/auth/${action}`;
}

async function requestJson<T>(
  fetchImpl: typeof globalThis.fetch,
  url: string,
  init: RequestInit,
): Promise<T> {
  let response: Response;
  try {
    response = await fetchImpl(url, init);
  } catch (reason) {
    throw toError(reason);
  }

  const text = await response.text();
  let body: unknown = undefined;
  if (text.length > 0) {
    try {
      body = JSON.parse(text) as unknown;
    } catch {
      body = text;
    }
  }
  if (!response.ok) {
    const message =
      body && typeof body === "object" && "error" in body
        ? String((body as { error: unknown }).error)
        : `Web request failed (${response.status})`;
    throw new Error(message);
  }
  return body as T;
}

async function requestCommand<T>(
  fetchImpl: typeof globalThis.fetch,
  baseUrl: string,
  channel: string,
  payload: Record<string, unknown> = {},
): Promise<T> {
  return requestJson<T>(fetchImpl, commandUrl(baseUrl, channel), {
    method: "POST",
    credentials: "same-origin",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(payload),
  });
}

function webAuthOptions(options: WebAuthOptions): {
  fetchImpl: typeof globalThis.fetch;
  baseUrl: string;
} {
  return {
    fetchImpl: options.fetch ?? globalThis.fetch.bind(globalThis),
    baseUrl: (options.baseUrl ?? "").replace(/\/$/, ""),
  };
}

export function getWebSession(options: WebAuthOptions = {}): Promise<WebSession> {
  const { fetchImpl, baseUrl } = webAuthOptions(options);
  return requestJson<WebSession>(fetchImpl, authUrl(baseUrl, "session"), {
    method: "GET",
    credentials: "same-origin",
  });
}

export function loginWebSession(
  password: string,
  options: WebAuthOptions = {},
): Promise<WebSession> {
  const { fetchImpl, baseUrl } = webAuthOptions(options);
  return requestJson<WebSession>(fetchImpl, authUrl(baseUrl, "login"), {
    method: "POST",
    credentials: "same-origin",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ password }),
  });
}

export function logoutWebSession(options: WebAuthOptions = {}): Promise<WebSession> {
  const { fetchImpl, baseUrl } = webAuthOptions(options);
  return requestJson<WebSession>(fetchImpl, authUrl(baseUrl, "logout"), {
    method: "POST",
    credentials: "same-origin",
  });
}

/** Build the `DesktopAPI` facade backed by the headless HTTP service. */
export function createWebDesktopApi(options: WebDesktopApiOptions = {}): DesktopAPI {
  const fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
  const baseUrl = (options.baseUrl ?? "").replace(/\/$/, "");
  const createSource =
    options.eventSource ??
    ((url: string) => {
      if (typeof globalThis.EventSource !== "function") {
        throw new Error("EventSource is unavailable in the web runtime");
      }
      return new globalThis.EventSource(url) as unknown as WebEventSource;
    });
  const eventBus = new WebEventBus(baseUrl, createSource);
  let debugDisposer: (() => void) | null = null;
  const command = <T>(channel: string, payload?: Record<string, unknown>) =>
    requestCommand<T>(fetchImpl, baseUrl, channel, payload);

  return {
    // App
    appInfo: () => command("app:info"),
    checkForUpdate: async () => null,
    installUpdate: async () => unsupportedWebAction("installUpdate"),

    // Library
    listLibraryRoots: () => command("library:list-roots"),
    scanLibrary: (dirPath) => command("library:scan", { dirPath }),
    refreshAlbum: (albumPath) => command("album:refresh", { albumPath }),

    // Browser replacements for native dialogs are supplied by the React UI.
    openFolderDialog: async () => unsupportedWebAction("openFolderDialog"),

    // Tracks
    readAlbum: (albumPath) => command("album:read", { albumPath }),
    writeTrack: (path, fields) => command("track:write", { path, fields }),
    writeTracks: (updates) => command("tracks:batch-write", { updates }),
    readExtraTags: (trackPath) => command("track:extra-tags:read", { trackPath }),
    writeExtraTags: (trackPath, tags) =>
      command("track:extra-tags:write", { trackPath, tags }),
    writeExtraTagsBatch: (updates) =>
      command("tracks:batch-write-extra-tags", { updates }),
    renameTrack: (oldPath, newPath) => command("track:rename", { oldPath, newPath }),
    checkFileExists: (filePath) => command("file:exists", { filePath }),
    probeWriteVolume: (filePath) => command("volume:probe-write", { path: filePath }),
    probeWriteVolumeReal: (filePath, patch) =>
      command("volume:probe-write-real", { path: filePath, patchJson: patch }),
    showTrackContextMenu: async () => unsupportedWebAction("showTrackContextMenu"),
    deleteFiles: (filePaths) => command("track:delete-files", { filePaths }),

    // Cover
    getCoverDataUrl: (albumPath, preferredTrackPath) =>
      command("cover:data-url", { albumPath, preferredTrackPath }),
    setCover: (albumPath) => command("cover:set", { albumPath }),
    removeCover: (albumPath) => command("cover:remove", { albumPath }),
    downloadCoverArt: (albumPath) => command("cover:download", { albumPath }),
    downloadArtistArt: (albumPath) =>
      command("cover:download-artist-art", { albumPath }),

    // Directory browser
    listDirectory: (dirPath) => command("directory:list", { dirPath }),
    readDirectory: (dirPath) => command("directory:read", { dirPath }),

    // Lyrics
    fetchLyrics: (trackName, artistName, albumName, duration) =>
      command("lyrics:fetch", { trackName, artistName, albumName, duration }),

    // Config
    getConfig: () => command("config:get"),
    setConfig: (key, value) => command("config:set", { key, value }),

    // Auto-tag
    autoTagAlbum: (albumPath) => command("album:auto-tag", { albumPath }),
    downloadAlbumLyrics: (albumPath) =>
      command("album:download-lyrics", { albumPath }),
    onAutoTagEvent: (callback) => eventBus.subscribe("auto-tag:event", callback),
    onTrackWriteEvent: (callback) =>
      eventBus.subscribe("tracks:write-event", callback),
    getTaskProgress: (taskId) => command("task:progress", { taskId }),
    cancelTask: (taskId) => command("task:cancel", { taskId }),
    getDatasetStatus: () => command("dataset:status"),

    // Audit
    runAudit: (libraryPath) => command("audit:run", { libraryPath }),
    runAuditOnTracks: (trackPaths) =>
      command("audit:run-specified", { trackPaths }),
    runAuditOnAlbums: (albumPaths) =>
      command("audit:run-specified", { albumPaths }),
    runAlbumAudit: (albumPath) => command("audit:run-album", { albumPath }),
    applyAuditFixes: (albumResults) =>
      command("audit:apply-fixes", { albumResults }),
    onAuditEvent: (callback) => eventBus.subscribe("audit:event", callback),
    cancelAudit: () => command("audit:cancel"),

    // Assistant
    assistantSend: (input) => command("assistant:send", { input }),
    assistantCancel: () => command("assistant:cancel"),
    assistantClear: () => command("assistant:clear"),
    assistantApplyActions: (actionBatchId) =>
      command("assistant:apply-actions", { actionBatchId }),
    assistantCompleteTaskActions: (actionBatchId, error) =>
      command("assistant:complete-task-actions", {
        actionBatchId,
        error: error ?? null,
      }),
    assistantRejectActions: (actionBatchId) =>
      command("assistant:reject-actions", { actionBatchId }),
    assistantGetBatches: () => command("assistant:get-batches"),
    assistantInitRuntime: () => command("assistant:init-runtime"),
    assistantInitServices: (config) => command("assistant:init-services", { config }),
    testLlmConnection: (apiKey, model, provider, baseUrl) =>
      command("test-llm-connection", { apiKey, model, provider, baseUrl }),
    onAssistantEvent: (callback) => eventBus.subscribe("assistant:event", callback),

    // Debug logs use the shared SSE connection; focus is a browser no-op.
    subscribeDebugLogs: async () => {
      if (debugDisposer) return;
      debugDisposer = eventBus.subscribe("debug:log", (entry) => {
        console.debug("[soundrobe]", entry);
      });
    },
    setDebugMode: (enabled) => command("debug:set-mode", { enabled }),
    onFocus: async () => {},

    // Manual search
    searchReleases: (request) => command("album:search-releases", { request }),
    resolveRelease: (provider, releaseId, kind) =>
      command("album:resolve-release", {
        request: { provider, releaseId, kind },
      }),
    previewReleaseMatch: (request) =>
      command("album:preview-release-match", { request }),
    searchApplyCandidate: (albumPath, candidate, selectedTrackIndices) =>
      command("album:search-apply-candidate", {
        request: { albumPath, candidate, selectedTrackIndices },
      }),

    // Organizer
    sortByAlbum: (sourceDir, options) =>
      command("files:sort-by-album", { sourceDir, options }),

    // Conversation logs
    listSessions: (limit) => command("assistant:list-sessions", { limit }),
    getConversation: (sessionUuidOrNumber) =>
      command("assistant:get-conversation", { sessionUuidOrNumber }),
    getSession: (sessionUuidOrNumber) =>
      command("assistant:get-session", { sessionUuidOrNumber }),
    getCurrentSession: () => command("assistant:current-session"),
  };
}
