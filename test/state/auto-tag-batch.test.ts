import { describe, expect, it, vi } from "vitest";
import {
  runAutoTagBatch,
  type AutoTagBatchRunnerApi,
} from "../../src/state/auto-tag-batch";
import type { TaskProgress } from "../../src/shared/desktop-api";

function progress(
  taskId: string,
  status: TaskProgress["status"],
  result: unknown = null,
): TaskProgress {
  return {
    taskId,
    status,
    progress: status === "running" ? 4 : 9,
    total: 9,
    message: status === "completed" ? "Complete" : `Needs review — ${status}`,
    result,
  };
}

describe("runAutoTagBatch", () => {
  it("retries only provider-unavailable albums once and records recovered results", async () => {
    const albums = Array.from({ length: 12 }, (_, index) => `/music/Album ${index + 1}`);
    const attempts = new Map<string, number>();
    const taskProgress = new Map<string, TaskProgress>();
    const sleep = vi.fn().mockResolvedValue(undefined);
    const api: AutoTagBatchRunnerApi = {
      autoTagAlbum: vi.fn(async (albumPath) => {
        const attempt = (attempts.get(albumPath) ?? 0) + 1;
        attempts.set(albumPath, attempt);
        const taskId = `${albumPath}-${attempt}`;
        const unavailable =
          albumPath.endsWith("Album 3") || albumPath.endsWith("Album 9") || albumPath.endsWith("Album 12");
        const permanentNoMatch = albumPath.endsWith("Album 4");
        const status =
          unavailable && attempt === 1
            ? "needs_review"
            : unavailable && albumPath.endsWith("Album 12")
              ? "needs_review"
              : permanentNoMatch
                ? "needs_review"
                : "completed";
        taskProgress.set(
          taskId,
          progress(
            taskId,
            status,
            status === "needs_review"
              ? {
                  reasonCode: permanentNoMatch
                    ? "ai_validation_failed"
                    : "provider_unavailable",
                  providerAttempts: [
                    {
                      provider: "musicbrainz",
                      status: "unavailable",
                      retryCount: 1,
                      retryAfterSeconds: 0,
                    },
                  ],
                }
              : { outcome: "applied", written: 1 },
          ),
        );
        return taskId;
      }),
      onAutoTagEvent: vi.fn(() => () => undefined),
      getTaskProgress: vi.fn(async (taskId) => taskProgress.get(taskId) ?? null),
    };

    const summary = await runAutoTagBatch({
      albumPaths: albums,
      api,
      sleep,
    });

    expect(attempts.get("/music/Album 3")).toBe(2);
    expect(attempts.get("/music/Album 9")).toBe(2);
    expect(attempts.get("/music/Album 12")).toBe(2);
    expect(attempts.get("/music/Album 4")).toBe(1);
    expect(sleep).toHaveBeenCalledTimes(1);
    expect(sleep).toHaveBeenCalledWith(1_000);
    expect(summary.items.find((item) => item.albumPath.endsWith("Album 3"))?.status).toBe(
      "recovered",
    );
    expect(summary.items.find((item) => item.albumPath.endsWith("Album 9"))?.status).toBe(
      "recovered",
    );
    expect(summary.items.find((item) => item.albumPath.endsWith("Album 12"))?.status).toBe(
      "needs_review",
    );
    expect(
      summary.items
        .find((item) => item.albumPath.endsWith("Album 12"))
        ?.providerAttempts[0]?.retryCount,
    ).toBe(2);
    expect(summary.items.find((item) => item.albumPath.endsWith("Album 4"))?.attempts).toBe(1);
    expect(summary.items).toHaveLength(12);
  });

  it("continues past a failed album and stops starting work after cancellation", async () => {
    const albums = ["/music/First", "/music/Second", "/music/Third"];
    const taskProgress = new Map<string, TaskProgress>();
    const api: AutoTagBatchRunnerApi = {
      autoTagAlbum: vi.fn(async (albumPath) => {
        const taskId = `task-${albumPath}`;
        taskProgress.set(
          taskId,
          progress(taskId, albumPath.endsWith("First") ? "failed" : "cancelled"),
        );
        return taskId;
      }),
      onAutoTagEvent: vi.fn(() => () => undefined),
      getTaskProgress: vi.fn(async (taskId) => taskProgress.get(taskId) ?? null),
    };

    const summary = await runAutoTagBatch({
      albumPaths: albums,
      api,
      sleep: vi.fn().mockResolvedValue(undefined),
    });

    expect(api.autoTagAlbum).toHaveBeenCalledTimes(2);
    expect(summary.items.map((item) => item.status)).toEqual([
      "failed",
      "cancelled",
      "cancelled",
    ]);
  });

  it("does not start deferred retries after cancellation", async () => {
    let cancelled = false;
    const taskProgress = new Map<string, TaskProgress>();
    const api: AutoTagBatchRunnerApi = {
      autoTagAlbum: vi.fn(async (albumPath) => {
        const taskId = `task-${albumPath}`;
        taskProgress.set(
          taskId,
          progress(taskId, "needs_review", {
            reasonCode: "provider_unavailable",
          }),
        );
        return taskId;
      }),
      onAutoTagEvent: vi.fn(() => () => undefined),
      getTaskProgress: vi.fn(async (taskId) => taskProgress.get(taskId) ?? null),
    };
    const sleep = vi.fn(async () => {
      cancelled = true;
    });

    const summary = await runAutoTagBatch({
      albumPaths: ["/music/Unavailable"],
      api,
      sleep,
      isCancelled: () => cancelled,
    });

    expect(api.autoTagAlbum).toHaveBeenCalledOnce();
    expect(summary.items[0].status).toBe("needs_review");
  });
});
