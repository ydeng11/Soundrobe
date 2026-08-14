/**
 * Undo stack for reverting track edits.
 * Ported from soundrobe/ui/undo.py
 */

export interface TrackSnapshot {
  path: string;
  fields: Record<string, unknown>;
}

export interface UndoOperation {
  id: number;
  description: string;
  timestamp: number;
  snapshots: TrackSnapshot[];
  affectedFileCount: number;
}

export interface SnapshotRevertFailure {
  snapshot: TrackSnapshot;
  error: string;
}

const MAX_STACK_DEPTH = 20;
let nextOperationId = 1;

export class UndoManager {
  private stack: UndoOperation[] = [];
  private maxDepth: number;

  constructor(maxDepth: number = MAX_STACK_DEPTH) {
    this.maxDepth = maxDepth;
  }

  get canUndo(): boolean {
    return this.stack.length > 0;
  }

  get currentDescription(): string | null {
    if (this.stack.length > 0) {
      return this.stack[this.stack.length - 1].description;
    }
    return null;
  }

  push(description: string, snapshots: TrackSnapshot[]): void {
    if (snapshots.length === 0) return;
    this.stack.push({
      id: nextOperationId++,
      description,
      timestamp: Date.now(),
      snapshots: [...snapshots],
      affectedFileCount: new Set(snapshots.map((snapshot) => snapshot.path))
        .size,
    });
    if (this.stack.length > this.maxDepth) {
      this.stack.shift();
    }
  }

  /**
   * Pure alternative to push — returns a new UndoManager with the
   * operation already pushed. The original is not mutated.
   */
  cloneAndPush(description: string, snapshots: TrackSnapshot[]): UndoManager {
    const clone = this.clone();
    clone.push(description, snapshots);
    return clone;
  }

  clone(): UndoManager {
    const clone = new UndoManager(this.maxDepth);
    clone.stack = this.stack.map((operation) => ({
      ...operation,
      snapshots: operation.snapshots.map((snapshot) => ({
        ...snapshot,
        fields: { ...snapshot.fields },
      })),
    }));
    return clone;
  }

  get history(): UndoOperation[] {
    return this.stack
      .slice()
      .reverse()
      .map((operation) => ({
        ...operation,
        snapshots: operation.snapshots.map((snapshot) => ({
          ...snapshot,
          fields: { ...snapshot.fields },
        })),
      }));
  }

  replaceHistory(history: UndoOperation[]): UndoManager {
    const clone = new UndoManager(this.maxDepth);
    clone.stack = history
      .slice(0, this.maxDepth)
      .reverse()
      .map((operation) => ({
        ...operation,
        snapshots: operation.snapshots.map((snapshot) => ({
          ...snapshot,
          fields: { ...snapshot.fields },
        })),
        affectedFileCount: new Set(
          operation.snapshots.map((snapshot) => snapshot.path),
        ).size,
      }));
    return clone;
  }

  clear(): void {
    this.stack = [];
  }

  get length(): number {
    return this.stack.length;
  }
}

/**
 * Revert the selected command and every newer command. The callback returns
 * null after a complete snapshot revert, or the retryable remainder after a
 * partial/failed revert.
 */
export async function revertHistoryThrough(
  manager: UndoManager,
  targetId: number,
  revertSnapshot: (
    snapshot: TrackSnapshot,
  ) => Promise<SnapshotRevertFailure | null>,
): Promise<{
  manager: UndoManager;
  failures: Array<{ path: string; error: string }>;
}> {
  const history = manager.history;
  const targetIndex = history.findIndex((operation) => operation.id === targetId);
  if (targetIndex < 0) {
    return { manager, failures: [] };
  }

  let remainingHistory = history;
  for (const operation of history.slice(0, targetIndex + 1)) {
    const failedSnapshots: TrackSnapshot[] = [];
    const failures: Array<{ path: string; error: string }> = [];

    for (const snapshot of operation.snapshots) {
      const failure = await revertSnapshot(snapshot);
      if (failure) {
        failedSnapshots.push(failure.snapshot);
        failures.push({ path: snapshot.path, error: failure.error });
      }
    }

    remainingHistory = remainingHistory.filter(
      (candidate) => candidate.id !== operation.id,
    );
    if (failedSnapshots.length > 0) {
      remainingHistory.unshift({
        ...operation,
        snapshots: failedSnapshots,
        affectedFileCount: new Set(
          failedSnapshots.map((snapshot) => snapshot.path),
        ).size,
      });
      return {
        manager: manager.replaceHistory(remainingHistory),
        failures,
      };
    }
  }

  return {
    manager: manager.replaceHistory(remainingHistory),
    failures: [],
  };
}
