#!/usr/bin/env python3
"""Run a non-persistent, read-only Codex or Claude advisor session."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Sequence


BACKENDS = ("codex", "claude")
EFFORTS_BY_BACKEND = {
    "codex": {"minimal", "low", "medium", "high", "xhigh", "max", "ultra"},
    "claude": {"low", "medium", "high", "max"},
}
BACKEND_LABELS = {"codex": "Codex", "claude": "Claude"}
CONFIG_VERSION = 1
UNSET = object()
CODEX_DISABLED_FEATURES = (
    "shell_tool",
    "unified_exec",
    "code_mode_host",
    "browser_use",
    "computer_use",
    "apps",
    "multi_agent",
)
CHILD_ENV_KEYS = {
    "APPDATA",
    "ANTHROPIC_API_KEY",
    "CODEX_HOME",
    "COMSPEC",
    "HOME",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "LANG",
    "LC_ALL",
    "LOCALAPPDATA",
    "NO_PROXY",
    "OPENAI_API_KEY",
    "PATH",
    "PATHEXT",
    "SSL_CERT_DIR",
    "SSL_CERT_FILE",
    "SYSTEMROOT",
    "TMP",
    "TMPDIR",
    "TEMP",
    "USERPROFILE",
}


class AdvisorError(RuntimeError):
    """Raised when an advisor session cannot return usable advice."""


def default_config() -> dict[str, Any]:
    return {
        "version": CONFIG_VERSION,
        "default_backend": "codex",
        "backends": {
            "codex": {"model": None, "effort": None},
            "claude": {"model": None, "effort": None},
        },
    }


def default_config_path() -> Path:
    codex_home = os.environ.get("CODEX_HOME")
    root = Path(codex_home).expanduser() if codex_home else Path.home() / ".codex"
    return root / "consult-advisor.json"


def validate_effort(backend: str, effort: str | None) -> None:
    if not effort:
        return
    supported_efforts = EFFORTS_BY_BACKEND[backend]
    if effort not in supported_efforts:
        supported = ", ".join(sorted(supported_efforts))
        raise ValueError(
            f"Unsupported {BACKEND_LABELS[backend]} effort {effort!r}; choose: {supported}"
        )


def validate_config(raw: Any) -> dict[str, Any]:
    try:
        if not isinstance(raw, dict) or raw.get("version") != CONFIG_VERSION:
            raise ValueError(f"version must be {CONFIG_VERSION}")
        default_backend = raw["default_backend"]
        backends = raw["backends"]
        if default_backend not in BACKENDS:
            raise ValueError("default_backend must be codex or claude")
        if not isinstance(backends, dict):
            raise ValueError("backends must be an object")

        normalized = default_config()
        normalized["default_backend"] = default_backend
        for backend in BACKENDS:
            values = backends.get(backend, {})
            if not isinstance(values, dict):
                raise ValueError(f"backends.{backend} must be an object")
            model = values.get("model")
            effort = values.get("effort")
            if model is not None and (not isinstance(model, str) or not model.strip()):
                raise ValueError(f"backends.{backend}.model must be a non-empty string or null")
            if effort is not None and not isinstance(effort, str):
                raise ValueError(f"backends.{backend}.effort must be a string or null")
            validate_effort(backend, effort)
            normalized["backends"][backend] = {"model": model, "effort": effort}
        return normalized
    except (KeyError, TypeError, ValueError) as error:
        raise AdvisorError(f"Invalid advisor config: {error}") from error


def load_config(path: Path) -> dict[str, Any]:
    if not path.exists():
        return default_config()
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise AdvisorError(f"Invalid advisor config: {error}") from error
    return validate_config(raw)


def save_defaults(
    path: Path,
    *,
    backend: str,
    model: str | None | object = UNSET,
    effort: str | None | object = UNSET,
) -> dict[str, Any]:
    if backend not in BACKENDS:
        raise ValueError(f"Unsupported backend {backend!r}; choose: codex, claude")
    config = load_config(path)
    config["default_backend"] = backend
    if model is not UNSET:
        if model is not None and (not isinstance(model, str) or not model.strip()):
            raise ValueError("Model must be a non-empty string")
        config["backends"][backend]["model"] = model
    if effort is not UNSET:
        if effort is not None and not isinstance(effort, str):
            raise ValueError("Effort must be a string")
        validate_effort(backend, effort)
        config["backends"][backend]["effort"] = effort

    write_config(path, config)
    return config


def write_config(path: Path, config: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        mode="w",
        encoding="utf-8",
        dir=path.parent,
        prefix=f".{path.name}.",
        delete=False,
    ) as handle:
        temporary_path = Path(handle.name)
        json.dump(config, handle, indent=2)
        handle.write("\n")
    try:
        temporary_path.replace(path)
    finally:
        temporary_path.unlink(missing_ok=True)


def reset_defaults(path: Path) -> dict[str, Any]:
    config = default_config()
    write_config(path, config)
    return config


def print_config(path: Path, config: dict[str, Any]) -> None:
    print(json.dumps({"config_file": str(path), **config}, indent=2))


def resolve_settings(
    config: dict[str, Any],
    *,
    backend: str | None,
    model: str | None,
    effort: str | None,
) -> tuple[str, str | None, str | None]:
    selected_backend = backend or config["default_backend"]
    saved = config["backends"][selected_backend]
    selected_model = model if model is not None else saved["model"]
    selected_effort = effort if effort is not None else saved["effort"]
    validate_effort(selected_backend, selected_effort)
    return selected_backend, selected_model, selected_effort


def build_command(
    *,
    backend: str,
    executable: str,
    model: str | None,
    effort: str | None,
    workspace: Path,
) -> list[str]:
    """Build a shell-free command for the selected read-only backend."""
    if backend == "codex":
        validate_effort(backend, effort)
        command = [executable]
        for feature in CODEX_DISABLED_FEATURES:
            command.extend(["--disable", feature])
        command.extend([
            "--ask-for-approval",
            "never",
            "exec",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "--config",
            'shell_environment_policy.inherit="none"',
            "--cd",
            str(workspace),
        ])
        if model:
            command.extend(["--model", model])
        if effort:
            command.extend(["--config", f'model_reasoning_effort="{effort}"'])
        command.append("-")
        return command

    if backend == "claude":
        validate_effort(backend, effort)
        command = [
            executable,
            "--print",
            "--no-session-persistence",
            "--strict-mcp-config",
            "--no-chrome",
            "--setting-sources",
            "",
            "--permission-mode",
            "plan",
            "--tools",
            "",
        ]
        if model:
            command.extend(["--model", model])
        if effort:
            command.extend(["--effort", effort])
        return command

    raise ValueError(f"Unsupported backend {backend!r}; choose: codex, claude")


def child_environment() -> dict[str, str]:
    """Keep only runtime, proxy, certificate, and backend-auth variables."""
    return {key: value for key, value in os.environ.items() if key in CHILD_ENV_KEYS}


def run_advisor(
    *,
    backend: str,
    prompt: str,
    model: str | None,
    effort: str | None,
    workspace: Path,
    executable: str | None = None,
    timeout_seconds: int = 900,
) -> str:
    """Run the advisor and return its final text output."""
    if not prompt.strip():
        raise ValueError("Advisor prompt must not be empty")
    if not workspace.is_dir():
        raise ValueError(f"Workspace is not a directory: {workspace}")

    selected_executable = executable or shutil.which(backend)
    if not selected_executable:
        raise AdvisorError(f"{backend} executable was not found on PATH")

    try:
        with tempfile.TemporaryDirectory(prefix="consult-advisor-") as isolated_dir:
            isolated_workspace = Path(isolated_dir)
            command = build_command(
                backend=backend,
                executable=selected_executable,
                model=model,
                effort=effort,
                workspace=isolated_workspace,
            )
            result = subprocess.run(
                command,
                cwd=isolated_workspace,
                env=child_environment(),
                input=prompt,
                capture_output=True,
                text=True,
                timeout=timeout_seconds,
            )
    except subprocess.TimeoutExpired as error:
        raise AdvisorError(
            f"{backend} advisor exceeded the {timeout_seconds}-second timeout"
        ) from error
    except OSError as error:
        raise AdvisorError(f"{backend} advisor could not start: {error}") from error

    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "no error output"
        raise AdvisorError(f"{backend} advisor failed ({result.returncode}): {detail}")
    if not result.stdout.strip():
        raise AdvisorError(f"{backend} advisor returned no advice")
    return result.stdout


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", choices=("codex", "claude"))
    parser.add_argument("--model", help="Backend model name; invocation overrides saved default")
    parser.add_argument("--effort", help="Reasoning effort; invocation overrides saved default")
    parser.add_argument("--workspace", type=Path, default=Path.cwd())
    parser.add_argument("--executable", help="Override the backend executable path")
    parser.add_argument("--timeout-seconds", type=int, default=900)
    parser.add_argument("--config-file", type=Path, help="Override the advisor defaults path")
    parser.add_argument(
        "--save-defaults",
        action="store_true",
        help="Save the supplied backend and optional model/effort, then exit",
    )
    parser.add_argument(
        "--show-config",
        action="store_true",
        help="Print resolved saved defaults, then exit",
    )
    parser.add_argument(
        "--reset-defaults",
        action="store_true",
        help="Replace saved settings with portable defaults, then exit",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the command configuration without launching an advisor",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    workspace = args.workspace.expanduser().resolve()
    config_path = (args.config_file or default_config_path()).expanduser().resolve()

    try:
        config_actions = sum((args.save_defaults, args.show_config, args.reset_defaults))
        if config_actions > 1:
            raise ValueError(
                "Choose only one of --save-defaults, --show-config, or --reset-defaults"
            )
        if args.save_defaults:
            if not args.backend:
                raise ValueError("--save-defaults requires --backend")
            config = save_defaults(
                config_path,
                backend=args.backend,
                model=args.model if args.model is not None else UNSET,
                effort=args.effort if args.effort is not None else UNSET,
            )
            print_config(config_path, config)
            return 0

        if args.reset_defaults:
            config = reset_defaults(config_path)
            print_config(config_path, config)
            return 0

        config = load_config(config_path)
        if args.show_config:
            print_config(config_path, config)
            return 0

        backend, model, effort = resolve_settings(
            config,
            backend=args.backend,
            model=args.model,
            effort=args.effort,
        )
        if args.dry_run:
            executable = args.executable or shutil.which(backend) or backend
            command = build_command(
                backend=backend,
                executable=executable,
                model=model,
                effort=effort,
                workspace=workspace,
            )
            print(
                json.dumps(
                    {
                        "backend": backend,
                        "model": model,
                        "effort": effort,
                        "command": command,
                        "cwd": str(workspace),
                    },
                    indent=2,
                )
            )
            return 0

        prompt = sys.stdin.read()
        advice = run_advisor(
            backend=backend,
            prompt=prompt,
            model=model,
            effort=effort,
            workspace=workspace,
            executable=args.executable,
            timeout_seconds=args.timeout_seconds,
        )
        print(advice, end="" if advice.endswith("\n") else "\n")
        return 0
    except (AdvisorError, OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
