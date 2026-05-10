"""Tests for interactive album decisions."""

from auto_tagger.workflows.interactive import Decision, ScriptedPromptSession, choose_album_action


def test_scripted_prompt_accepts_album():
    """Scripted prompt responses support deterministic interactive tests."""
    decision = choose_album_action("Album", ScriptedPromptSession(["accept"]))

    assert decision == Decision.ACCEPT


def test_unknown_prompt_response_defaults_to_skip():
    """Unknown interactive responses fail safely as skip."""
    decision = choose_album_action("Album", ScriptedPromptSession(["???"]))

    assert decision == Decision.SKIP
