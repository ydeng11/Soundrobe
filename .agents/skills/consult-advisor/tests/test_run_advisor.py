#!/usr/bin/env python3
"""Regression tests for the isolated advisor runner."""

from __future__ import annotations

import importlib.util
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


RUNNER_PATH = Path(__file__).parents[1] / "scripts" / "run_advisor.py"
SPEC = importlib.util.spec_from_file_location("consult_advisor_runner", RUNNER_PATH)
assert SPEC and SPEC.loader
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)


class AdvisorIsolationTests(unittest.TestCase):
    def test_codex_command_disables_every_agent_tool_surface(self) -> None:
        command = RUNNER.build_command(
            backend="codex",
            executable="codex",
            model=None,
            effort=None,
            workspace=Path("/tmp/advisor-empty"),
        )

        self.assertIn("--ignore-rules", command)
        for feature in (
            "shell_tool",
            "unified_exec",
            "code_mode_host",
            "browser_use",
            "computer_use",
            "apps",
            "multi_agent",
        ):
            self.assertIn(["--disable", feature], [command[i : i + 2] for i in range(len(command) - 1)])
        self.assertIn(
            ["--config", 'shell_environment_policy.inherit="none"'],
            [command[i : i + 2] for i in range(len(command) - 1)],
        )

    def test_run_uses_an_empty_workspace_and_drops_unrelated_environment(self) -> None:
        captured: dict[str, object] = {}

        def fake_run(command: list[str], **kwargs: object) -> SimpleNamespace:
            captured["command"] = command
            captured.update(kwargs)
            return SimpleNamespace(returncode=0, stdout="On track\n", stderr="")

        with tempfile.TemporaryDirectory() as workspace, mock.patch.object(
            RUNNER.subprocess, "run", side_effect=fake_run
        ), mock.patch.dict(os.environ, {"ADVISOR_TEST_SECRET": "must-not-cross"}):
            advice = RUNNER.run_advisor(
                backend="codex",
                prompt="Review only this packet.",
                model=None,
                effort=None,
                workspace=Path(workspace),
                executable="codex",
            )

        self.assertEqual(advice, "On track\n")
        isolated_cwd = Path(captured["cwd"])
        self.assertNotEqual(isolated_cwd, Path(workspace))
        self.assertTrue(isolated_cwd.name.startswith("consult-advisor-"))
        command = captured["command"]
        self.assertEqual(command[command.index("--cd") + 1], str(isolated_cwd))
        child_env = captured["env"]
        self.assertNotIn("ADVISOR_TEST_SECRET", child_env)


if __name__ == "__main__":
    unittest.main()
