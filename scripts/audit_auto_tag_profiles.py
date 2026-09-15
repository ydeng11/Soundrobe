#!/usr/bin/env python3
"""Report all auto-tag input profiles without contacting providers.

The corpus shape and deterministic input transformations are always scored for
all cases. Optional native JSON/JSONL results are summarized separately so a
provider outage cannot be mistaken for a matcher result.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any


PROFILES = ("folder_filename", "assisted_without_ids", "tagged_recovery")


def read_json(path: Path) -> Any:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_native(path: Path | None) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    if path is None or not path.exists():
        return [], []
    if path.suffix == ".jsonl":
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()], []
    value = read_json(path)
    if isinstance(value, dict) and isinstance(value.get("invocations"), list):
        return (
            [item for item in value["invocations"] if isinstance(item, dict)],
            [item for item in value.get("folderResults", []) if isinstance(item, dict)],
        )
    return [], []


def identity(value: Any) -> str | None:
    if isinstance(value, str) and value:
        return value
    if isinstance(value, dict):
        for key in ("discogsReleaseId", "discogs_release_id", "musicbrainzAlbumId", "musicbrainz_album_id"):
            if value.get(key):
                return str(value[key])
    return None


def classify_case(
    expectation: dict[str, Any],
    folder: dict[str, Any] | None,
    records: list[dict[str, Any]],
) -> str:
    if folder is not None:
        final = folder.get("classification")
        selected = identity(folder.get("selectedIdentity"))
    else:
        phases = {record.get("phase"): record for record in records if record.get("phase") in {"cold", "warm"}}
        if set(phases) != {"cold", "warm"}:
            return "incomplete"
        cold, warm = phases["cold"], phases["warm"]
        cold_id, warm_id = identity(cold.get("selectedIdentity")), identity(warm.get("selectedIdentity"))
        if cold.get("classification") == "failed_verification" or warm.get("classification") == "failed_verification":
            return "failed_verification"
        if cold_id and warm_id and cold_id != warm_id:
            return "failed_verification"
        final = warm.get("classification") or cold.get("classification")
        selected = warm_id or cold_id
    acceptable = {str(value) for value in expectation.get("acceptableEditionIds", [])}
    hard_negative = {str(value) for value in expectation.get("rejectedHardNegativeIds", [])}
    if selected in hard_negative or final == "wrong_match":
        return "wrong_match"
    if expectation.get("status") == "verified_match" and selected in acceptable and final == "confirmed_success":
        return "correct"
    if expectation.get("status") == "verified_abstain" and final == "safe_abstention":
        return "correct"
    if final in {"failed_verification", "incomplete", "safe_abstention", "unresolved"}:
        return str(final)
    return "unresolved"


def profile_metrics(
    corpus: dict[str, Any],
    expectations: dict[str, Any],
    records: list[dict[str, Any]],
    folders: list[dict[str, Any]],
    profile_name: str,
) -> dict[str, Any]:
    cases = corpus["cases"]
    expectation_by_id = {item["caseId"]: item for item in expectations["cases"]}
    records_by_case: dict[str, list[dict[str, Any]]] = {}
    for record in records:
        if isinstance(record.get("caseId"), str):
            records_by_case.setdefault(record["caseId"], []).append(record)
    folder_by_case = {folder["caseId"]: folder for folder in folders if isinstance(folder.get("caseId"), str)}
    counts = Counter(record.get("classification", "unknown") for record in records)
    case_ids = {case["caseId"] for case in cases}
    phase_counts = Counter(
        (record.get("caseId"), record.get("phase"))
        for record in records
        if isinstance(record.get("caseId"), str) and record.get("phase") in {"cold", "warm"}
    )
    native_complete = (
        len(records) == len(cases) * 2
        and {case_id for case_id, _ in phase_counts} == case_ids
        and all(phase_counts[(case_id, phase)] == 1 for case_id in case_ids for phase in ("cold", "warm"))
    )
    unavailable = sum(
        1
        for record in records
        if any(
            isinstance(attempt, dict) and attempt.get("status") == "unavailable"
            for attempt in (record.get("native", {}) or {}).get("providerAttempts", [])
        )
    )
    outcomes = Counter(
        classify_case(
            expectation_by_id.get(case["caseId"], {}),
            folder_by_case.get(case["caseId"]),
            records_by_case.get(case["caseId"], []),
        )
        for case in cases
    )
    eligible = sum(
        expectation_by_id.get(case["caseId"], {}).get("status") in ("verified_match", "verified_abstain")
        for case in cases
    )
    correct = outcomes["correct"]
    decisive = correct + outcomes["wrong_match"]
    safe_abstentions = sum(
        1
        for case in cases
        if (
            folder_by_case.get(case["caseId"], {}).get("classification") == "safe_abstention"
            or any(
                record.get("classification") == "safe_abstention"
                for record in records_by_case.get(case["caseId"], [])
            )
        )
    )
    readback_failures = sum(
        any(record.get("readback") is False for record in records_by_case.get(case["caseId"], []))
        for case in cases
    )
    payload_failures = sum(
        any(record.get("payloadUnchanged") is False for record in records_by_case.get(case["caseId"], []))
        for case in cases
    )
    mapping_failures = sum(
        any(record.get("classification") == "failed_verification" for record in records_by_case.get(case["caseId"], []))
        for case in cases
    )
    return {
        "caseCount": len(cases),
        "trackCount": sum(len(case["tracks"]) for case in cases),
        "reviewedEligibleCases": eligible,
        "nativeInvocations": len(records),
        "expectedNativeInvocations": len(cases) * 2,
        "nativeComplete": native_complete,
        "nativeClassifications": dict(sorted(counts.items())),
        "providerUnavailableInvocations": unavailable,
        "correctMatches": correct,
        "wrongMatches": outcomes["wrong_match"],
        "safeAbstentions": safe_abstentions,
        "coverage": correct / eligible if eligible else None,
        "precision": correct / decisive if decisive else None,
        "incompleteCases": outcomes["incomplete"],
        "unresolvedCases": outcomes["unresolved"],
        "failedVerificationCases": outcomes["failed_verification"],
        "readbackFailures": readback_failures,
        "payloadFailures": payload_failures,
        "mappingFailures": mapping_failures,
        "deterministicInput": {
            "providerIds": corpus["profiles"][profile_name]["strips"],
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--expectations", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--run-id", default="profile-baseline")
    parser.add_argument(
        "--native",
        action="append",
        default=[],
        metavar="PROFILE=PATH",
        help="optional retained native results or JSONL for one profile",
    )
    args = parser.parse_args()
    corpus = read_json(args.corpus)
    expectations = read_json(args.expectations)
    native_paths: dict[str, Path] = {}
    for value in args.native:
        profile, separator, path = value.partition("=")
        if separator != "=" or profile not in PROFILES or not path:
            raise SystemExit(f"--native must be PROFILE=PATH for {', '.join(PROFILES)}")
        native_paths[profile] = Path(path)

    output: dict[str, Any] = {
        "schemaVersion": 1,
        "runId": args.run_id,
        "corpusVersion": corpus["corpusVersion"],
        "caseCount": len(corpus["cases"]),
        "trackCount": sum(len(case["tracks"]) for case in corpus["cases"]),
        "inputSha256": {
            "corpus": sha256_file(args.corpus),
            "expectations": sha256_file(args.expectations),
        },
        "profiles": {},
        "notes": [
            "Deterministic profile counts cover every corpus case and track.",
            "Native provider classifications are reported only for retained records.",
            "Unreviewed expectations remain outside matcher precision and coverage.",
        ],
    }
    native_incomplete = False
    for profile_name in PROFILES:
        records, folders = read_native(native_paths.get(profile_name))
        metrics = profile_metrics(corpus, expectations, records, folders, profile_name)
        metrics["deterministicInput"] = {
            "strips": corpus["profiles"][profile_name]["strips"],
            "purpose": corpus["profiles"][profile_name]["purpose"],
        }
        if profile_name in native_paths:
            metrics["nativeSource"] = str(native_paths[profile_name])
            if native_paths[profile_name].exists():
                metrics["nativeSourceSha256"] = sha256_file(native_paths[profile_name])
            else:
                metrics["nativeSourceMissing"] = True
            native_incomplete = native_incomplete or not metrics["nativeComplete"]
        output["profiles"][profile_name] = metrics

    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "profiles.json").write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
    lines = [
        "# Auto-tag input-profile baseline",
        "",
        f"Run `{args.run_id}` over {output['caseCount']} cases and {output['trackCount']} tracks.",
        "",
        "| Profile | Cases | Tracks | Native invocations | Expected | Complete | Correct | Wrong | Safe abstentions | Coverage | Incomplete | Provider-unavailable | Mapping failures | Readback failures | Payload failures |",
        "| --- | ---: | ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for name, metrics in output["profiles"].items():
        lines.append(
            f"| {name} | {metrics['caseCount']} | {metrics['trackCount']} | "
            f"{metrics['nativeInvocations']} | {metrics['expectedNativeInvocations']} | "
            f"{str(metrics['nativeComplete']).lower()} | {metrics['correctMatches']} | "
            f"{metrics['wrongMatches']} | {metrics['safeAbstentions']} | "
            f"{metrics['coverage'] if metrics['coverage'] is not None else 'n/a'} | "
            f"{metrics['incompleteCases']} | {metrics['providerUnavailableInvocations']} | "
            f"{metrics['mappingFailures']} | {metrics['readbackFailures']} | {metrics['payloadFailures']} |"
        )
    lines.extend(
        [
            "",
            "Deterministic input coverage is complete for all three shapes. Native outcome metrics are scored only for reviewed expectations; incomplete provider work remains diagnostic.",
        ]
    )
    (args.output_dir / "profiles.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    (args.output_dir / "command.log").write_text(
        f"status={'incomplete' if native_incomplete else 'passed'}\nrun_id={args.run_id}\ncase_count={output['caseCount']}\ntrack_count={output['trackCount']}\n",
        encoding="utf-8",
    )
    print(json.dumps({name: value["nativeInvocations"] for name, value in output["profiles"].items()}))
    return 0 if not native_incomplete else 1


if __name__ == "__main__":
    raise SystemExit(main())
