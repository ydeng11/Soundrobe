#!/usr/bin/env python3
"""Audit frozen provider inputs and retained native replay reproducibility.

This gate is offline.  It verifies that every candidate-pool fixture exists at
the declared relative path and still has its locked digest, then reports the
production-reader equivalence gate and any cold/warm identity drift observed in
native results.  A provider recovery or cross-provider identity change remains
diagnostic and never becomes matcher credit.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


def read_json(path: Path) -> Any:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_resolve(base: Path, relative: Any, label: str, allowed_root: Path | None = None) -> Path:
    if not isinstance(relative, str) or not relative:
        raise ValueError(f"{label} must be a non-empty relative path")
    candidate = (base / relative).resolve()
    try:
        candidate.relative_to((allowed_root or base).resolve())
    except ValueError as error:
        raise ValueError(f"{label} escapes fixture root: {relative}") from error
    return candidate


def validate_digest(path: Path, expected: Any, label: str) -> str:
    if not isinstance(expected, str) or len(expected) != 64:
        raise ValueError(f"{label} is missing a SHA-256 lock")
    actual = sha256_file(path)
    if actual != expected:
        raise ValueError(f"{label} SHA-256 mismatch: expected {expected}, found {actual}")
    return actual


def audit_candidate_pools(path: Path) -> list[dict[str, Any]]:
    manifest = read_json(path)
    if manifest.get("schemaVersion") != 1:
        raise ValueError("candidate pool schemaVersion must be 1")
    pools = manifest.get("pools")
    if not isinstance(pools, list) or not pools:
        raise ValueError("candidate pool manifest has no pools")
    # The checked-in manifest lives beside sibling Relapse fixtures, so its
    # fixture root is the parent of `auto-tag-eval`.  Keep synthetic test
    # manifests rooted at their own directory.
    root = path.parent.parent if path.parent.name == "auto-tag-eval" else path.parent
    frozen: list[dict[str, Any]] = []
    for pool in pools:
        pool_id = pool.get("poolId")
        if not isinstance(pool_id, str) or not pool_id:
            raise ValueError("candidate pool has no poolId")
        local = safe_resolve(
            path.parent,
            pool.get("localFixture"),
            f"pool {pool_id} localFixture",
            root,
        )
        if not local.is_file():
            raise ValueError(f"pool {pool_id} local fixture is missing: {local}")
        local_hash = validate_digest(local, pool.get("localFixtureSha256"), f"pool {pool_id} localFixture")
        candidates = pool.get("candidates")
        if not isinstance(candidates, list) or not candidates:
            raise ValueError(f"pool {pool_id} has no candidates")
        pool_record = {
            "poolId": pool_id,
            "localFixture": str(local),
            "localFixtureSha256": local_hash,
            "candidates": [],
        }
        for candidate in candidates:
            provider = candidate.get("provider")
            release_id = candidate.get("releaseId")
            response = safe_resolve(
                path.parent,
                candidate.get("response"),
                f"pool {pool_id} response",
                root,
            )
            if not isinstance(provider, str) or not provider or not isinstance(release_id, str) or not release_id:
                raise ValueError(f"pool {pool_id} has an incomplete candidate identity")
            if not response.is_file():
                raise ValueError(f"pool {pool_id} response is missing: {response}")
            response_hash = validate_digest(
                response,
                candidate.get("responseSha256"),
                f"pool {pool_id} {provider}/{release_id}",
            )
            read_json(response)
            pool_record["candidates"].append(
                {
                    "provider": provider,
                    "releaseId": release_id,
                    "response": str(response),
                    "responseSha256": response_hash,
                    "expectation": candidate.get("expectation"),
                }
            )
        frozen.append(pool_record)
    return frozen


def identity(value: Any) -> str | None:
    if isinstance(value, str) and value:
        return value
    if isinstance(value, dict):
        for key in ("discogsReleaseId", "discogs_release_id", "musicbrainzAlbumId", "musicbrainz_album_id"):
            if value.get(key):
                return str(value[key])
    return None


def audit_equivalence(path: Path) -> dict[str, Any]:
    value = read_json(path)
    cases = value.get("cases")
    if not isinstance(cases, list) or not cases:
        raise ValueError("equivalence artifact has no cases")
    case_ids = [case.get("caseId") for case in cases if isinstance(case, dict)]
    if len(case_ids) != len(cases) or any(not isinstance(case_id, str) or not case_id for case_id in case_ids):
        raise ValueError("equivalence cases must have non-empty case IDs")
    if len(set(case_ids)) != len(case_ids):
        raise ValueError("equivalence artifact contains duplicate case IDs")
    comparable_fields = ("requestEqual", "evidenceEqual", "orderedMatchEqual", "remoteIndicesEqual")
    failures = []
    for case in cases:
        checks = [case[field] for field in comparable_fields if field in case]
        if not checks or any(value is not True for value in checks):
            failures.append(case.get("caseId"))
    passed = value.get("allLookupRequestsEquivalent") is True and not failures
    return {
        "caseCount": len(cases),
        "caseIds": case_ids,
        "allLookupRequestsEquivalent": passed,
        "failedCaseIds": failures,
        "mediaMode": value.get("mediaMode"),
        "source": value.get("source"),
    }


def audit_native_results(path: Path | None) -> dict[str, Any]:
    if path is None:
        return {
            "available": False,
            "complete": False,
            "invocationCount": 0,
            "folderCount": 0,
            "missingPhases": [],
            "duplicatePhases": [],
            "identityInconsistencies": [],
            "providerUnavailableCases": 0,
        }
    results = read_json(path)
    phases: dict[tuple[str, str], dict[str, Any]] = {}
    duplicate_phases: list[str] = []
    provider_unavailable: set[str] = set()
    invocations = results.get("invocations")
    if not isinstance(invocations, list):
        raise ValueError("native results have no invocations array")
    for record in invocations:
        case_id = record.get("caseId")
        phase = record.get("phase")
        if isinstance(case_id, str) and isinstance(phase, str):
            key = (case_id, phase)
            if key in phases:
                duplicate_phases.append(f"{case_id}:{phase}")
            phases[key] = record
        attempts = record.get("native", {}).get("providerAttempts", [])
        if isinstance(attempts, list) and any(
            isinstance(attempt, dict) and attempt.get("status") == "unavailable"
            for attempt in attempts
        ):
            provider_unavailable.add(case_id)
    drift = []
    for folder in results.get("folderResults", []):
        case_id = folder.get("caseId")
        cold = identity(folder.get("coldIdentity"))
        warm = identity(folder.get("warmIdentity"))
        if cold and warm and cold != warm:
            drift.append(
                {
                    "caseId": case_id,
                    "sourceRelativeFolder": folder.get("sourceRelativeFolder"),
                    "coldIdentity": cold,
                    "warmIdentity": warm,
                    "coldAuthority": phases.get((case_id, "cold"), {}).get("native", {}).get("authority"),
                    "warmAuthority": phases.get((case_id, "warm"), {}).get("native", {}).get("authority"),
                }
            )
    observed_case_ids = {
        case_id for case_id, _ in phases
    } | {
        folder.get("caseId")
        for folder in results.get("folderResults", [])
        if isinstance(folder, dict) and isinstance(folder.get("caseId"), str)
    }
    case_ids = sorted(observed_case_ids)
    missing_phases = [
        f"{case_id}:{phase}"
        for case_id in case_ids
        for phase in ("cold", "warm")
        if (case_id, phase) not in phases
    ]
    complete = bool(case_ids) and not missing_phases and not duplicate_phases
    return {
        "available": True,
        "complete": complete,
        "invocationCount": len(invocations),
        "folderCount": len(results.get("folderResults", [])),
        "missingPhases": missing_phases,
        "duplicatePhases": sorted(duplicate_phases),
        "identityInconsistencies": drift,
        "providerUnavailableCases": len(provider_unavailable),
        "reconciledAsFailedVerification": len(drift),
    }


def render_report(result: dict[str, Any], run_id: str) -> str:
    equiv = result["syntheticEquivalence"]
    native = result["nativeReplay"]
    lines = [
        "# Auto-tag reproducibility audit",
        "",
        f"Run `{run_id}` with network disabled.",
        "",
        "## Frozen provider inputs",
        "",
        f"- Candidate pools: {result['candidatePoolCount']}",
        f"- Frozen responses: {result['frozenResponseCount']}",
        f"- Candidate and fixture SHA-256 locks: {'passed' if result['candidatePoolHashesValid'] else 'failed'}",
        "",
        "## Synthetic-input equivalence",
        "",
        f"- Cases: {equiv['caseCount']}",
        f"- Lookup requests equivalent: {equiv['allLookupRequestsEquivalent']}",
        f"- Failed case IDs: {', '.join(equiv['failedCaseIds']) if equiv['failedCaseIds'] else 'none'}",
        "",
        "## Native cold/warm replay",
        "",
        f"- Invocations: {native['invocationCount']}",
        f"- Cold/warm phases complete: {native['complete']}",
        f"- Missing phases: {', '.join(native['missingPhases']) if native['missingPhases'] else 'none'}",
        f"- Duplicate phases: {', '.join(native['duplicatePhases']) if native['duplicatePhases'] else 'none'}",
        f"- Provider-unavailable cases: {native['providerUnavailableCases']}",
        f"- Cold/warm identity inconsistencies: {len(native['identityInconsistencies'])}",
        "",
        "Identity inconsistencies are reconciled as `failed_verification`; provider recovery and a different release are not matcher improvements.",
        "",
    ]
    for item in native["identityInconsistencies"]:
        lines.append(
            f"- {item['caseId']} ({item['sourceRelativeFolder']}): "
            f"{item['coldIdentity']} ({item['coldAuthority'] or 'unknown'}) -> "
            f"{item['warmIdentity']} ({item['warmAuthority'] or 'unknown'})"
        )
    if not native["identityInconsistencies"]:
        lines.append("- none")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate-pools", type=Path, required=True)
    parser.add_argument("--equivalence", type=Path, required=True)
    parser.add_argument("--native-results", type=Path)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--run-id", default="reproducibility-audit")
    args = parser.parse_args()
    frozen = audit_candidate_pools(args.candidate_pools)
    equivalence = audit_equivalence(args.equivalence)
    native = audit_native_results(args.native_results)
    result = {
        "schemaVersion": 1,
        "runId": args.run_id,
        "network": "disabled",
        "candidatePoolCount": len(frozen),
        "frozenResponseCount": sum(len(pool["candidates"]) for pool in frozen),
        "candidatePoolHashesValid": True,
        "candidatePools": frozen,
        "syntheticEquivalence": equivalence,
        "nativeReplay": native,
        "reproducible": (
            equivalence["allLookupRequestsEquivalent"]
            and native["available"]
            and native["complete"]
            and not native["identityInconsistencies"]
        ),
        "inputSha256": {
            "candidatePools": sha256_file(args.candidate_pools),
            "equivalence": sha256_file(args.equivalence),
            **({"nativeResults": sha256_file(args.native_results)} if args.native_results else {}),
        },
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "reproducibility.json").write_text(
        json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    (args.output_dir / "reproducibility.md").write_text(
        render_report(result, args.run_id), encoding="utf-8"
    )
    (args.output_dir / "command.log").write_text(
        f"status={'passed' if result['reproducible'] else 'failed'}\nrun_id={args.run_id}\nnetwork=disabled\n"
        f"reproducible={str(result['reproducible']).lower()}\n",
        encoding="utf-8",
    )
    print(json.dumps({key: result[key] for key in (
        "candidatePoolCount", "frozenResponseCount", "reproducible"
    )}))
    return 0 if result["reproducible"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
