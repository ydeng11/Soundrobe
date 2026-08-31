import type {
  AutoTagProviderAttempt,
  DesktopAPI,
  TaskProgress,
} from "../shared/desktop-api";

export type AutoTagBatchItemStatus =
  | "applied"
  | "recovered"
  | "needs_review"
  | "failed"
  | "cancelled";

export interface AutoTagBatchItem {
  albumPath: string;
  status: AutoTagBatchItemStatus;
  attempts: number;
  message: string;
  reasonCode: string | null;
  providerAttempts: AutoTagProviderAttempt[];
  readbackRequired: boolean;
}

export interface AutoTagBatchSummary {
  items: AutoTagBatchItem[];
}

export type AutoTagBatchRunnerApi = Pick<
  DesktopAPI,
  "autoTagAlbum" | "onAutoTagEvent" | "getTaskProgress"
>;

type Sleep = (milliseconds: number) => Promise<void>;

interface AutoTagBatchOptions {
  albumPaths: string[];
  api: AutoTagBatchRunnerApi;
  sleep?: Sleep;
  isCancelled?: () => boolean;
  onProgress?: (progress: {
    current: number;
    total: number;
    message: string;
  }) => void;
}

const POLL_INTERVAL_MS = 300;
const PROVIDER_RETRY_DELAY_MS = 1_000;
const MAX_PROVIDER_RETRY_DELAY_MS = 30_000;

function sleepFor(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function providerAttemptsFromResult(result: unknown): AutoTagProviderAttempt[] {
  if (!isRecord(result) || !Array.isArray(result.providerAttempts)) return [];
  return result.providerAttempts.filter((attempt): attempt is AutoTagProviderAttempt => {
    if (!isRecord(attempt)) return false;
    return (
      typeof attempt.provider === "string" &&
      (attempt.status === "matched" ||
        attempt.status === "no_match" ||
        attempt.status === "unavailable")
    );
  });
}

function resultReasonCode(result: unknown): string | null {
  if (!isRecord(result) || typeof result.reasonCode !== "string") return null;
  return result.reasonCode;
}

function isProviderUnavailable(item: AutoTagBatchItem): boolean {
  return item.status === "needs_review" && item.reasonCode === "provider_unavailable";
}

function itemFromProgress(
  albumPath: string,
  attempts: number,
  progress: TaskProgress,
): AutoTagBatchItem {
  if (progress.status === "completed") {
    return {
      albumPath,
      status: "applied",
      attempts,
      message: progress.message,
      reasonCode: null,
      providerAttempts: providerAttemptsFromResult(progress.result),
      readbackRequired: true,
    };
  }

  if (progress.status === "needs_review") {
    return {
      albumPath,
      status: "needs_review",
      attempts,
      message: progress.message,
      reasonCode: resultReasonCode(progress.result),
      providerAttempts: providerAttemptsFromResult(progress.result),
      readbackRequired: false,
    };
  }

  return {
    albumPath,
    status: progress.status === "cancelled" ? "cancelled" : "failed",
    attempts,
    message: progress.message,
    reasonCode: resultReasonCode(progress.result),
    providerAttempts: providerAttemptsFromResult(progress.result),
    readbackRequired: true,
  };
}

function failedItem(albumPath: string, attempts: number, message: string): AutoTagBatchItem {
  return {
    albumPath,
    status: "failed",
    attempts,
    message,
    reasonCode: null,
    providerAttempts: [],
    readbackRequired: attempts > 0,
  };
}

function cancelledItem(albumPath: string, attempts = 0): AutoTagBatchItem {
  return {
    albumPath,
    status: "cancelled",
    attempts,
    message: "Auto-tag cancelled",
    reasonCode: "cancelled",
    providerAttempts: [],
    readbackRequired: attempts > 0,
  };
}

function retryDelayMilliseconds(retryAfterSeconds: number): number {
  if (retryAfterSeconds <= 0) return PROVIDER_RETRY_DELAY_MS;
  return Math.min(
    MAX_PROVIDER_RETRY_DELAY_MS,
    Math.max(PROVIDER_RETRY_DELAY_MS, retryAfterSeconds * 1_000),
  );
}

function mergeProviderAttempts(
  previous: AutoTagProviderAttempt[],
  current: AutoTagProviderAttempt[],
): AutoTagProviderAttempt[] {
  const merged = new Map<string, AutoTagProviderAttempt>();
  for (const attempt of [...previous, ...current]) {
    const existing = merged.get(attempt.provider);
    if (!existing) {
      merged.set(attempt.provider, { ...attempt });
      continue;
    }
    merged.set(attempt.provider, {
      ...existing,
      ...attempt,
      diagnostic: attempt.diagnostic ?? existing.diagnostic,
      retryCount: (existing.retryCount ?? 0) + (attempt.retryCount ?? 0),
      retryAfterSeconds: Math.max(
        existing.retryAfterSeconds ?? 0,
        attempt.retryAfterSeconds ?? 0,
      ) || undefined,
    });
  }
  return Array.from(merged.values());
}

async function waitForDeferredRetry(
  milliseconds: number,
  sleep: Sleep,
  isCancelled?: () => boolean,
): Promise<boolean> {
  if (!isCancelled) {
    await sleep(milliseconds);
    return true;
  }
  for (let elapsed = 0; elapsed < milliseconds; elapsed += 100) {
    if (isCancelled()) return false;
    await sleep(Math.min(100, milliseconds - elapsed));
  }
  return !isCancelled();
}

async function runAlbumAttempt(
  albumPath: string,
  attempts: number,
  api: AutoTagBatchRunnerApi,
  sleep: Sleep,
  report: (message: string) => void,
): Promise<AutoTagBatchItem> {
  let taskId: string;
  try {
    taskId = await api.autoTagAlbum(albumPath);
  } catch (error) {
    return failedItem(
      albumPath,
      attempts - 1,
      error instanceof Error ? error.message : "Auto-tag could not start",
    );
  }

  const unsubscribe = api.onAutoTagEvent((event) => {
    if (event.taskId !== taskId) return;
    report(event.message);
  });

  try {
    while (true) {
      let progress: TaskProgress | null;
      try {
        progress = await api.getTaskProgress(taskId);
      } catch (error) {
        return failedItem(
          albumPath,
          attempts,
          error instanceof Error ? error.message : "Auto-tag progress failed",
        );
      }
      if (!progress) {
        return failedItem(
          albumPath,
          attempts,
          `Auto-tag task progress disappeared: ${taskId}`,
        );
      }

      report(progress.message);
      if (progress.status !== "running") {
        return itemFromProgress(albumPath, attempts, progress);
      }
      await sleep(POLL_INTERVAL_MS);
    }
  } finally {
    unsubscribe();
  }
}

export async function runAutoTagBatch({
  albumPaths,
  api,
  sleep = sleepFor,
  isCancelled,
  onProgress,
}: AutoTagBatchOptions): Promise<AutoTagBatchSummary> {
  const items = albumPaths.map((albumPath) => cancelledItem(albumPath));
  const retryPaths: string[] = [];
  let completedAttempts = 0;
  let cancelled = false;

  const report = (albumPath: string, message: string, total: number) => {
    onProgress?.({
      current: completedAttempts,
      total,
      message: `${albumPath}: ${message}`,
    });
  };

  for (let index = 0; index < albumPaths.length; index += 1) {
    if (cancelled || isCancelled?.()) {
      cancelled = true;
      break;
    }
    const albumPath = albumPaths[index];
    const item = await runAlbumAttempt(
      albumPath,
      1,
      api,
      sleep,
      (message) => report(albumPath, message, albumPaths.length),
    );
    items[index] = item;
    completedAttempts += 1;
    onProgress?.({
      current: completedAttempts,
      total: albumPaths.length,
      message: item.message,
    });

    if (item.status === "cancelled") {
      cancelled = true;
      break;
    }
    if (isProviderUnavailable(item)) retryPaths.push(albumPath);
  }

  if (cancelled) {
    for (let index = 0; index < items.length; index += 1) {
      if (items[index].attempts === 0) items[index] = cancelledItem(albumPaths[index]);
    }
    return { items };
  }

  if (retryPaths.length > 0) {
    const retryDelay = retryPaths.reduce(
      (maximum, albumPath) => {
        const item = items[albumPaths.indexOf(albumPath)];
        const retryAfterSeconds = item.providerAttempts.reduce(
          (attemptMaximum, attempt) =>
            Math.max(attemptMaximum, attempt.retryAfterSeconds ?? 0),
          0,
        );
        return Math.max(maximum, retryDelayMilliseconds(retryAfterSeconds));
      },
      PROVIDER_RETRY_DELAY_MS,
    );
    if (!(await waitForDeferredRetry(retryDelay, sleep, isCancelled))) {
      return { items };
    }
    for (const albumPath of retryPaths) {
      const index = albumPaths.indexOf(albumPath);
      if (isCancelled?.()) {
        cancelled = true;
        break;
      }
      const item = await runAlbumAttempt(
        albumPath,
        2,
        api,
        sleep,
        (message) => report(albumPath, message, albumPaths.length),
      );
      const previous = items[index];
      const retriedItem = {
        ...item,
        providerAttempts: mergeProviderAttempts(
          previous.providerAttempts,
          item.providerAttempts,
        ),
      };
      items[index] =
        retriedItem.status === "applied"
          ? { ...retriedItem, status: "recovered" }
          : retriedItem;
      completedAttempts += 1;
      onProgress?.({
        current: completedAttempts,
        total: albumPaths.length + retryPaths.length,
        message: items[index].message,
      });
      if (item.status === "cancelled") {
        cancelled = true;
        break;
      }
    }
  }

  if (cancelled) {
    const started = new Set(
      albumPaths.filter((albumPath, index) => items[index].attempts > 0),
    );
    for (let index = 0; index < items.length; index += 1) {
      if (!started.has(albumPaths[index])) items[index] = cancelledItem(albumPaths[index]);
    }
  }

  return { items };
}
