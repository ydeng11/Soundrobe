"""Interactive workflow prompt helpers."""

from __future__ import annotations

from enum import Enum
from typing import Protocol


class Decision(str, Enum):
    """User decision for an album preview."""

    ACCEPT = "accept"
    SKIP = "skip"
    EDIT = "edit"
    ABORT = "abort"


class PromptSession(Protocol):
    """Prompt abstraction for deterministic tests."""

    def ask(self, prompt: str) -> str:
        """Return a user response."""


class ScriptedPromptSession:
    """Prompt session backed by pre-seeded answers."""

    def __init__(self, answers: list[str]):
        self.answers = list(answers)

    def ask(self, prompt: str) -> str:
        """Return the next scripted answer or skip safely."""
        if not self.answers:
            return "skip"
        return self.answers.pop(0)


def choose_album_action(album_name: str, session: PromptSession) -> Decision:
    """Prompt for an album action and return a safe decision."""
    answer = session.ask(f"{album_name}: accept, skip, edit, or abort? ").strip().lower()
    try:
        return Decision(answer)
    except ValueError:
        return Decision.SKIP
