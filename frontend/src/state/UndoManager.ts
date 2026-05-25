/**
 * Undo stack for reverting track edits.
 * Ported from auto_tagger/ui/undo.py
 */

export interface TrackSnapshot {
  path: string;
  fields: Record<string, unknown>;
}

interface UndoOperation {
  description: string;
  timestamp: number;
  snapshots: TrackSnapshot[];
}

const MAX_STACK_DEPTH = 50;

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
    this.stack.push({
      description,
      timestamp: Date.now(),
      snapshots: [...snapshots],
    });
    if (this.stack.length > this.maxDepth) {
      this.stack.shift();
    }
  }

  pop(): UndoOperation | null {
    if (this.stack.length === 0) return null;
    return this.stack.pop()!;
  }

  clear(): void {
    this.stack = [];
  }

  get length(): number {
    return this.stack.length;
  }
}
