// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";

import { createWebDesktopApi } from "../../src/shared/web-adapter";

describe("web-adapter command transport", () => {
  const fetchMock = vi.fn();
  let api: ReturnType<typeof createWebDesktopApi>;

  beforeEach(() => {
    fetchMock.mockReset();
    fetchMock.mockImplementation(() =>
      Promise.resolve(
        new Response(JSON.stringify({ ok: true }), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
      ),
    );
    api = createWebDesktopApi({ fetch: fetchMock });
  });

  it("posts appInfo to the typed command endpoint", async () => {
    await api.appInfo();

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/v1/commands/app%3Ainfo",
      expect.objectContaining({
        method: "POST",
        credentials: "same-origin",
        headers: { "content-type": "application/json" },
        body: "{}",
      }),
    );
  });

  it("preserves nested DesktopAPI payloads without changing their shape", async () => {
    const updates = [{ path: "/music/track.flac", fields: { title: "New title" } }];

    await api.writeTracks(updates);

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/v1/commands/tracks%3Abatch-write",
      expect.objectContaining({
        body: JSON.stringify({ updates }),
      }),
    );
  });

  it("maps every request-capable method to its canonical web command", async () => {
    const cases: Array<[string, () => Promise<unknown>, string]> = [
      ["appInfo", () => api.appInfo(), "app:info"],
      ["listLibraryRoots", () => api.listLibraryRoots(), "library:list-roots"],
      ["scanLibrary", () => api.scanLibrary("/lib"), "library:scan"],
      ["refreshAlbum", () => api.refreshAlbum("/album"), "album:refresh"],
      ["readAlbum", () => api.readAlbum("/album"), "album:read"],
      ["writeTrack", () => api.writeTrack("/track", {}), "track:write"],
      ["writeTracks", () => api.writeTracks([]), "tracks:batch-write"],
      ["readExtraTags", () => api.readExtraTags("/track"), "track:extra-tags:read"],
      ["writeExtraTags", () => api.writeExtraTags("/track", []), "track:extra-tags:write"],
      ["writeExtraTagsBatch", () => api.writeExtraTagsBatch([]), "tracks:batch-write-extra-tags"],
      ["renameTrack", () => api.renameTrack("/old", "/new"), "track:rename"],
      ["checkFileExists", () => api.checkFileExists("/track"), "file:exists"],
      ["probeWriteVolume", () => api.probeWriteVolume("/track"), "volume:probe-write"],
      ["probeWriteVolumeReal", () => api.probeWriteVolumeReal("/track", {}), "volume:probe-write-real"],
      ["deleteFiles", () => api.deleteFiles(["/track"]), "track:delete-files"],
      ["getCoverDataUrl", () => api.getCoverDataUrl("/album"), "cover:data-url"],
      ["setCover", () => api.setCover("/album"), "cover:set"],
      ["removeCover", () => api.removeCover("/album"), "cover:remove"],
      ["downloadCoverArt", () => api.downloadCoverArt("/album"), "cover:download"],
      ["downloadArtistArt", () => api.downloadArtistArt("/album"), "cover:download-artist-art"],
      ["listDirectory", () => api.listDirectory("/dir"), "directory:list"],
      ["readDirectory", () => api.readDirectory("/dir"), "directory:read"],
      ["fetchLyrics", () => api.fetchLyrics("title", "artist"), "lyrics:fetch"],
      ["getConfig", () => api.getConfig(), "config:get"],
      ["setConfig", () => api.setConfig("key", "value"), "config:set"],
      ["autoTagAlbum", () => api.autoTagAlbum("/album"), "album:auto-tag"],
      ["downloadAlbumLyrics", () => api.downloadAlbumLyrics("/album"), "album:download-lyrics"],
      ["getTaskProgress", () => api.getTaskProgress("task"), "task:progress"],
      ["cancelTask", () => api.cancelTask("task"), "task:cancel"],
      ["getDatasetStatus", () => api.getDatasetStatus(), "dataset:status"],
      ["runAudit", () => api.runAudit("/lib"), "audit:run"],
      ["runAuditOnTracks", () => api.runAuditOnTracks(["/track"]), "audit:run-specified"],
      ["runAuditOnAlbums", () => api.runAuditOnAlbums(["/album"]), "audit:run-specified"],
      ["runAlbumAudit", () => api.runAlbumAudit("/album"), "audit:run-album"],
      ["applyAuditFixes", () => api.applyAuditFixes([]), "audit:apply-fixes"],
      ["cancelAudit", () => api.cancelAudit(), "audit:cancel"],
      ["assistantSend", () => api.assistantSend({ message: "hi", apiKey: "key" }), "assistant:send"],
      ["assistantCancel", () => api.assistantCancel(), "assistant:cancel"],
      ["assistantClear", () => api.assistantClear(), "assistant:clear"],
      ["assistantApplyActions", () => api.assistantApplyActions("batch"), "assistant:apply-actions"],
      ["assistantCompleteTaskActions", () => api.assistantCompleteTaskActions("batch"), "assistant:complete-task-actions"],
      ["assistantRejectActions", () => api.assistantRejectActions("batch"), "assistant:reject-actions"],
      ["assistantGetBatches", () => api.assistantGetBatches(), "assistant:get-batches"],
      ["assistantInitRuntime", () => api.assistantInitRuntime(), "assistant:init-runtime"],
      ["assistantInitServices", () => api.assistantInitServices({ apiKey: "key" }), "assistant:init-services"],
      ["testLlmConnection", () => api.testLlmConnection("key", "model"), "test-llm-connection"],
      ["subscribeDebugLogs", () => api.subscribeDebugLogs(), "debug:subscribe"],
      ["setDebugMode", () => api.setDebugMode(true), "debug:set-mode"],
      ["searchReleases", () => api.searchReleases({ provider: "musicbrainz" }), "album:search-releases"],
      ["resolveRelease", () => api.resolveRelease("musicbrainz", "release"), "album:resolve-release"],
      ["previewReleaseMatch", () => api.previewReleaseMatch({
        albumPath: "/album",
        provider: "musicbrainz",
        release: { id: "release", title: "Album", artists: [], tracks: [] },
      }), "album:preview-release-match"],
      ["searchApplyCandidate", () => api.searchApplyCandidate(
        "/album",
        { artists: [], albumArtists: [], tracks: [] },
        [],
      ), "album:search-apply-candidate"],
      ["sortByAlbum", () => api.sortByAlbum("/dir"), "files:sort-by-album"],
      ["listSessions", () => api.listSessions(), "assistant:list-sessions"],
      ["getConversation", () => api.getConversation("session"), "assistant:get-conversation"],
      ["getSession", () => api.getSession("session"), "assistant:get-session"],
      ["getCurrentSession", () => api.getCurrentSession(), "assistant:current-session"],
    ];

    for (const [method, invoke, channel] of cases) {
      fetchMock.mockClear();
      await invoke();
      expect(fetchMock, method).toHaveBeenCalledWith(
        `/api/v1/commands/${encodeURIComponent(channel)}`,
        expect.any(Object),
      );
    }
  });

  it.each([
    ["onAutoTagEvent", "onAutoTagEvent"],
    ["onTrackWriteEvent", "onTrackWriteEvent"],
    ["onAuditEvent", "onAuditEvent"],
    ["onAssistantEvent", "onAssistantEvent"],
  ] as const)("fails loudly until the %s SSE channel is available", (method, action) => {
    const fn = (api as unknown as Record<string, (callback: () => void) => unknown>)[method];
    expect(() => fn(() => {})).toThrow(`${action} is unavailable in the web runtime`);
  });

  it("converts stable JSON HTTP errors into rejected Errors", async () => {
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ error: "authentication required" }), {
        status: 401,
        headers: { "content-type": "application/json" },
      }),
    );

    await expect(api.listLibraryRoots()).rejects.toThrow("authentication required");
  });

  it("keeps the complete DesktopAPI method surface available", () => {
    expect(Object.keys(api).sort()).toEqual([
      "appInfo",
      "applyAuditFixes",
      "assistantApplyActions",
      "assistantCancel",
      "assistantClear",
      "assistantCompleteTaskActions",
      "assistantGetBatches",
      "assistantInitRuntime",
      "assistantInitServices",
      "assistantRejectActions",
      "assistantSend",
      "autoTagAlbum",
      "cancelAudit",
      "cancelTask",
      "checkFileExists",
      "checkForUpdate",
      "deleteFiles",
      "downloadAlbumLyrics",
      "downloadArtistArt",
      "downloadCoverArt",
      "fetchLyrics",
      "getConfig",
      "getConversation",
      "getCoverDataUrl",
      "getCurrentSession",
      "getDatasetStatus",
      "getSession",
      "getTaskProgress",
      "installUpdate",
      "listDirectory",
      "listLibraryRoots",
      "listSessions",
      "onAssistantEvent",
      "onAuditEvent",
      "onAutoTagEvent",
      "onFocus",
      "onTrackWriteEvent",
      "openFolderDialog",
      "previewReleaseMatch",
      "probeWriteVolume",
      "probeWriteVolumeReal",
      "readAlbum",
      "readDirectory",
      "readExtraTags",
      "refreshAlbum",
      "removeCover",
      "renameTrack",
      "resolveRelease",
      "runAlbumAudit",
      "runAudit",
      "runAuditOnAlbums",
      "runAuditOnTracks",
      "scanLibrary",
      "searchApplyCandidate",
      "searchReleases",
      "setConfig",
      "setCover",
      "setDebugMode",
      "showTrackContextMenu",
      "sortByAlbum",
      "subscribeDebugLogs",
      "testLlmConnection",
      "writeExtraTags",
      "writeExtraTagsBatch",
      "writeTrack",
      "writeTracks",
    ].sort());
  });
});
