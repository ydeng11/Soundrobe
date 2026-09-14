#!/usr/bin/env python3
"""Plan and optionally run bounded, resumable native evidence collection.

The normal evaluation path is offline.  This helper only launches the existing
ignored native replay when ``--run`` is supplied, one small case batch at a
time.  It never edits the corpus or injects provider IDs into discovery input;
the native runner applies the selected profile and its existing retry, cache,
rate-limit, and per-folder budgets.  Completed, provider-backed batches stay
in the output directory while cases with unavailable providers remain queued
for a later invocation.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

PROFILES = {"folder_filename", "assisted_without_ids", "tagged_recovery"}
SCORED_STATUSES = {"verified_match", "verified_abstain"}


def read_json(path: Path) -> Any:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def sanitize_text(value: str) -> str:
    value = re.sub(r"https?://[^\s)]+", "<redacted-url>", value)
    value = re.sub(
        r"(?i)(authorization|token|api[_-]?key|secret|password)\s*[:=]\s*[^\s,]+",
        r"\1=<redacted>",
        value,
    )
    return value


def parse_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.is_file():
        return []
    records: list[dict[str, Any]] = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"invalid JSON in {path}:{line_number}: {error}") from error
        if isinstance(value, dict):
            records.append(value)
    return records


def latest_records(output_dir: Path) -> dict[tuple[str, str], dict[str, Any]]:
    """Return the newest retained cold/warm record for each case."""
    records: dict[tuple[str, str], dict[str, Any]] = {}
    for batch in sorted(output_dir.glob("batch-*")):
        if not batch.is_dir():
            continue
        for phase in ("cold", "warm"):
            for record in parse_jsonl(batch / f"{phase}.jsonl"):
                case_id = record.get("caseId")
                if isinstance(case_id, str) and case_id:
                    records[(case_id, phase)] = record
    return records


def provider_complete(record: dict[str, Any]) -> bool:
    if record.get("timedOut") is True or record.get("resolverError"):
        return False
    if record.get("outcome") in {"timeout", "resolver_error"}:
        return False
    native = record.get("native")
    attempts = native.get("providerAttempts", []) if isinstance(native, dict) else []
    return not any(
        isinstance(attempt, dict) and attempt.get("status") == "unavailable"
        for attempt in attempts
    )


def complete_case(case_id: str, records: dict[tuple[str, str], dict[str, Any]]) -> bool:
    phases = [records.get((case_id, phase)) for phase in ("cold", "warm")]
    return all(isinstance(record, dict) and provider_complete(record) for record in phases)


def validate_case_ids(
    corpus: dict[str, Any], expectations: dict[str, Any], requested: list[str] | None
) -> list[str]:
    cases = corpus.get("cases")
    if not isinstance(cases, list):
        raise ValueError("corpus has no cases array")
    by_id = {case.get("caseId"): case for case in cases if isinstance(case, dict)}
    if len(by_id) != len(cases):
        raise ValueError("corpus contains duplicate or missing case IDs")
    expectation_by_id = {
        case.get("caseId"): case
        for case in expectations.get("cases", [])
        if isinstance(case, dict)
    }
    if set(expectation_by_id) != set(by_id):
        raise ValueError("expectations do not cover the corpus exactly")
    if requested is None:
        return sorted(
            case_id
            for case_id, expectation in expectation_by_id.items()
            if expectation.get("status") not in SCORED_STATUSES
        )
    values = sorted({value.strip() for value in requested if value.strip()})
    unknown = sorted(set(values) - set(by_id))
    if unknown:
        raise ValueError(f"unknown case IDs: {', '.join(unknown)}")
    return values


def load_or_validate_state(
    path: Path,
    corpus_hash: str,
    expectations_hash: str,
    profile: str,
    case_ids: list[str],
) -> dict[str, Any]:
    if not path.is_file():
        return {
            "schemaVersion": 1,
            "corpusSha256": corpus_hash,
            "expectationsSha256": expectations_hash,
            "profile": profile,
            "caseIds": case_ids,
            "createdAt": datetime.now(timezone.utc).isoformat(),
            "batches": [],
        }
    state = read_json(path)
    if not isinstance(state, dict) or state.get("schemaVersion") != 1:
        raise ValueError("evidence collector state schemaVersion must be 1")
    for key, expected in (
        ("corpusSha256", corpus_hash),
        ("expectationsSha256", expectations_hash),
        ("profile", profile),
    ):
        if state.get(key) != expected:
            raise ValueError(f"collector state {key} does not match current inputs")
    if state.get("caseIds") != case_ids:
        raise ValueError("collector state case IDs do not match the requested queue")
    if not isinstance(state.get("batches", []), list):
        raise ValueError("collector state batches must be an array")
    return state


def batch_number(output_dir: Path) -> int:
    numbers = []
    for path in output_dir.glob("batch-*"):
        match = re.fullmatch(r"batch-(\d+)", path.name)
        if match:
            numbers.append(int(match.group(1)))
    return max(numbers, default=0) + 1


def make_batches(case_ids: list[str], batch_size: int) -> list[dict[str, Any]]:
    return [
        {"caseIds": case_ids[offset : offset + batch_size]}
        for offset in range(0, len(case_ids), batch_size)
    ]


def run_batch(
    repo_root: Path,
    corpus: Path,
    expectations: Path,
    output_dir: Path,
    profile: str,
    batch_id: str,
    case_ids: list[str],
    run_id: str,
) -> dict[str, Any]:
    artifact_dir = output_dir / batch_id
    artifact_dir.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(
        {
            "SOUNDROBE_AUTO_TAG_EVAL_CORPUS": str(corpus.resolve()),
            "SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS": str(expectations.resolve()),
            "SOUNDROBE_AUTO_TAG_EVAL_ARTIFACT_DIR": str(artifact_dir.resolve()),
            "SOUNDROBE_AUTO_TAG_EVAL_RUN_ID": run_id,
            "SOUNDROBE_AUTO_TAG_EVAL_PROFILE": profile,
            "SOUNDROBE_AUTO_TAG_EVAL_ARTISTS": "",
            "SOUNDROBE_AUTO_TAG_EVAL_CASES": ",".join(case_ids),
        }
    )
    command = [
        "cargo",
        "test",
        "--manifest-path",
        str(repo_root / "src-tauri/Cargo.toml"),
        "--lib",
        "live_auto_tag_eval",
        "--",
        "--ignored",
        "--nocapture",
    ]
    completed = subprocess.run(command, cwd=repo_root, env=env, capture_output=True, text=True, check=False)
    output = sanitize_text((completed.stdout or "") + (completed.stderr or ""))
    (artifact_dir / "collector-command.log").write_text(
        f"status={'passed' if completed.returncode == 0 else 'failed'}\n"
        f"exit_code={completed.returncode}\n"
        f"case_ids={','.join(case_ids)}\n"
        f"command={' '.join(command)}\n\n{output}",
        encoding="utf-8",
    )
    return {
        "batchId": batch_id,
        "caseIds": case_ids,
        "runId": run_id,
        "exitCode": completed.returncode,
        "artifactDir": str(artifact_dir),
        "status": "complete" if completed.returncode == 0 else "failed",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--expectations", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--profile", choices=sorted(PROFILES), default="folder_filename")
    parser.add_argument("--case-ids", help="comma-separated case IDs; defaults to unscored cases")
    parser.add_argument("--batch-size", type=int, default=4)
    parser.add_argument("--max-batches", type=int, default=1)
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--run", action="store_true", help="run the first missing batches through the native test")
    args = parser.parse_args()
    if args.batch_size < 1 or args.batch_size > 20:
        raise ValueError("batch-size must be between 1 and 20")
    if args.max_batches < 1:
        raise ValueError("max-batches must be positive")
    if not args.corpus.is_file() or not args.expectations.is_file():
        raise ValueError("corpus and expectations files must exist")
    corpus = read_json(args.corpus)
    expectations = read_json(args.expectations)
    if corpus.get("corpusVersion") != expectations.get("corpusVersion"):
        raise ValueError("corpus and expectations versions differ")
    requested = args.case_ids.split(",") if args.case_ids else None
    case_ids = validate_case_ids(corpus, expectations, requested)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    state_path = args.output_dir / "state.json"
    state = load_or_validate_state(
        state_path,
        sha256_file(args.corpus),
        sha256_file(args.expectations),
        args.profile,
        case_ids,
    )
    retained = latest_records(args.output_dir)
    complete = sorted(case_id for case_id in case_ids if complete_case(case_id, retained))
    pending = [case_id for case_id in case_ids if case_id not in set(complete)]
    plan = make_batches(pending, args.batch_size)
    planned_batches = []
    next_number = batch_number(args.output_dir)
    for batch in plan:
        planned_batches.append(
            {
                "batchId": f"batch-{next_number:03d}",
                "caseIds": batch["caseIds"],
                "status": "planned",
            }
        )
        next_number += 1
    state["lastPlannedAt"] = datetime.now(timezone.utc).isoformat()
    state["completeCaseIds"] = complete
    state["pendingCaseIds"] = pending
    state["batches"].extend(planned_batches)
    write_json(state_path, state)
    write_json(
        args.output_dir / "queue.json",
        {
            "schemaVersion": 1,
            "profile": args.profile,
            "caseCount": len(case_ids),
            "completeCaseCount": len(complete),
            "pendingCaseCount": len(pending),
            "batches": planned_batches,
        },
    )
    run_results = []
    if args.run:
        for batch in planned_batches[: args.max_batches]:
            run_id = f"{args.output_dir.name}-{batch['batchId']}"
            result = run_batch(
                args.repo_root.resolve(),
                args.corpus,
                args.expectations,
                args.output_dir,
                args.profile,
                batch["batchId"],
                batch["caseIds"],
                run_id,
            )
            run_results.append(result)
            batch.update(result)
        retained = latest_records(args.output_dir)
        complete = sorted(case_id for case_id in case_ids if complete_case(case_id, retained))
        state["completeCaseIds"] = complete
        state["pendingCaseIds"] = [case_id for case_id in case_ids if case_id not in set(complete)]
        write_json(state_path, state)
        write_json(args.output_dir / "queue.json", {
            "schemaVersion": 1,
            "profile": args.profile,
            "caseCount": len(case_ids),
            "completeCaseCount": len(complete),
            "pendingCaseCount": len(state["pendingCaseIds"]),
            "batches": planned_batches,
        })
    (args.output_dir / "command.log").write_text(
        f"network={'enabled' if args.run else 'disabled'}\n"
        f"profile={args.profile}\ncase_count={len(case_ids)}\n"
        f"complete_case_count={len(complete)}\npending_case_count={len(state.get('pendingCaseIds', pending))}\n"
        f"run_batches={len(run_results)}\n",
        encoding="utf-8",
    )
    print(json.dumps({
        "caseCount": len(case_ids),
        "completeCaseCount": len(complete),
        "pendingCaseCount": len(state.get("pendingCaseIds", pending)),
        "plannedBatchCount": len(planned_batches),
        "runBatchCount": len(run_results),
    }, sort_keys=True))
    return 0 if all(result["exitCode"] == 0 for result in run_results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
