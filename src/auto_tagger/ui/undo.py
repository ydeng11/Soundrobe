"""Undo stack for reverting edits."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from time import time

from auto_tagger.core.metadata import TrackMetadata

MAX_STACK_DEPTH = 50


@dataclass
class TrackSnapshot:
    """Pre-operation state for one track."""

    path: Path
    metadata: TrackMetadata


@dataclass
class UndoOperation:
    """One undoable operation (batch auto-tag, single field edit, etc.)."""

    description: str
    timestamp: float
    snapshots: list[TrackSnapshot]


class UndoManager:
    """Stack of undoable operations."""

    def __init__(self, max_depth: int = MAX_STACK_DEPTH) -> None:
        self._stack: list[UndoOperation] = []
        self._max_depth = max_depth

    @property
    def can_undo(self) -> bool:
        return len(self._stack) > 0

    @property
    def current_description(self) -> str | None:
        if self._stack:
            return self._stack[-1].description
        return None

    def push(self, description: str, snapshots: list[TrackSnapshot]) -> None:
        """Record an undoable operation."""
        self._stack.append(
            UndoOperation(
                description=description,
                timestamp=time(),
                snapshots=list(snapshots),
            )
        )
        if len(self._stack) > self._max_depth:
            self._stack.pop(0)

    def pop(self) -> UndoOperation | None:
        """Pop and return the most recent operation, or None if stack empty."""
        if not self._stack:
            return None
        return self._stack.pop()

    def clear(self) -> None:
        self._stack.clear()

    def __len__(self) -> int:
        return len(self._stack)
