import type { E2eManifest } from "./fixtures";

const manifest = JSON.parse(
  process.env.SOUNDROBE_E2E_MANIFEST ?? "null",
) as E2eManifest | null;

if (!manifest) {
  throw new Error("SOUNDROBE_E2E_MANIFEST is required");
}

describe("Live OpenRouter assistant", () => {
  it("completes a native assistant turn and persists the API call", async () => {
    const result = await browser.execute(async () => {
      const event = await window.api.assistantSend({
        message: "Say hello in one short sentence.",
        apiKey: "configured-in-native-process",
        selectedTrackPaths: [],
        tracks: [],
        albums: [],
        autonomous: false,
      });
      const current = await window.api.getCurrentSession();
      const session = current
        ? await window.api.getSession(current.sessionId)
        : null;
      return { event, session };
    });

    if (result.event.type !== "message") {
      throw new Error(`assistant returned ${JSON.stringify(result.event)}`);
    }
    expect(result.event.message.trim().length).toBeGreaterThan(0);
    expect(result.session?.apiCallCount).toBeGreaterThanOrEqual(1);
  });

  it("repairs and verifies malformed plural Artists values at the end of a copied scope", async () => {
    const result = await browser.execute(async (albumPath) => {
      const before = await window.api.readAlbum(albumPath);
      const event = await window.api.assistantSend({
        message:
          "fix the malformed plural “Artists” tags from selected tracks by splitting joined values",
        apiKey: "configured-in-native-process",
        selectedTrackPaths: before.tracks.map((track) => track.path),
        tracks: before.tracks,
        albums: [],
        autonomous: false,
      });
      const batches = await window.api.assistantGetBatches();
      const batchId =
        event.data && typeof event.data === "object" && "actionBatchId" in event.data
          ? String(event.data.actionBatchId)
          : null;
      const batch = batches.find((candidate) => candidate.id === batchId) ?? null;
      const apply = batchId
        ? await window.api.assistantApplyActions(batchId)
        : { success: false, error: "missing preview batch" };
      const pendingAfterApply = await window.api.assistantGetBatches();
      const after = await window.api.readAlbum(albumPath);
      return {
        event,
        batch,
        apply,
        batchStillPending: pendingAfterApply.some(
          (candidate) => candidate.id === batchId,
        ),
        before: before.tracks.slice(-2),
        after: after.tracks.slice(-2),
      };
    }, manifest.assistantRepairAlbum);

    if (result.event.type !== "action_batch_created") {
      throw new Error(`assistant returned ${JSON.stringify(result.event)}`);
    }
    expect(result.batch?.actions).toEqual([
      expect.objectContaining({
        trackPath: manifest.assistantRepairTracks[44],
        field: "artists",
        oldValue: "Artist A & Collaborator 45",
        newValue: "Artist A; Collaborator 45",
      }),
      expect.objectContaining({
        trackPath: manifest.assistantRepairTracks[45],
        field: "artists",
        oldValue: "Artist A & Collaborator 46",
        newValue: "Artist A; Collaborator 46",
      }),
    ]);
    expect(result.batch?.actions.some((action) => action.field === "artist")).toBe(false);
    expect(result.batch?.completionContract).toEqual(
      expect.objectContaining({
        scopePaths: manifest.assistantRepairTracks,
        expectedActionPaths: manifest.assistantRepairTracks.slice(-2),
        postcondition: "splitArtistsNormalized",
      }),
    );
    expect(result.batch?.completionContract?.scopeSnapshot).toHaveLength(46);
    expect(result.apply.success).toBe(true);
    expect(result.batchStillPending).toBe(false);
    expect(result.apply.verification).toEqual(
      expect.objectContaining({
        status: "verified",
        scopeCount: 46,
        expectedActionCount: 2,
        verifiedActionCount: 2,
        failures: [],
      }),
    );
    expect(result.before.map((track) => track.artists)).toEqual([
      ["Artist A & Collaborator 45"],
      ["Artist A & Collaborator 46"],
    ]);
    expect(result.after.map((track) => track.artists)).toEqual([
      ["Artist A", "Collaborator 45"],
      ["Artist A", "Collaborator 46"],
    ]);
    expect(result.after.map((track) => track.artist)).toEqual([
      "Artist A & Collaborator 45",
      "Artist A & Collaborator 46",
    ]);
  });
});
