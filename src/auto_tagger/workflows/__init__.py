"""Reusable tagging workflows."""

from auto_tagger.workflows.album import AlbumWorkflow, AlbumWorkflowResult
from auto_tagger.workflows.batch import BatchSummary, BatchWorkflow, discover_album_paths
from auto_tagger.workflows.interactive import (
    Decision,
    PromptSession,
    ScriptedPromptSession,
    choose_album_action,
)

__all__ = [
    "AlbumWorkflow",
    "AlbumWorkflowResult",
    "BatchSummary",
    "BatchWorkflow",
    "Decision",
    "PromptSession",
    "ScriptedPromptSession",
    "choose_album_action",
    "discover_album_paths",
]
