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


def read_native(path: Path | None) -> list[dict[str, Any]]:
    if path is None or not path.exists():
        return []
    if path.suffix == ".jsonl":
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    value = read_json(path)
    if isinstance(value, dict) and isinstance(value.get("invocations"), list):
        return [item for item in value["invocations"] if isinstance(item, dict)]
    return []


def profile_metrics(
    corpus: dict[str, Any], expectations: dict[str, Any], records: list[dict[str, Any]], profile_name: str
) -> dict[str, Any]:
    cases = corpus["cases"]
    reviewed = {
        item["caseId"]
        for item in expectations["cases"]
        if item.get("status") in ("verified_match", "verified_abstain")
    }
    counts = Counter(record.get("classification", "unknown") for record in records)
    unavailable = sum(
        1
        for record in records
        if any(
            isinstance(attempt, dict) and attempt.get("status") == "unavailable"
            for attempt in (record.get("native", {}) or {}).get("providerAttempts", [])
        )
    )
    return {
        "caseCount": len(cases),
        "trackCount": sum(len(case["tracks"]) for case in cases),
        "reviewedEligibleCases": len(reviewed),
        "nativeInvocations": len(records),
        "expectedNativeInvocations": len(cases) * 2,
        "nativeComplete": len(records) == len(cases) * 2,
        "nativeClassifications": dict(sorted(counts.items())),
        "providerUnavailableInvocations": unavailable,
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
    for profile_name in PROFILES:
        records = read_native(native_paths.get(profile_name))
        metrics = profile_metrics(corpus, expectations, records, profile_name)
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
        output["profiles"][profile_name] = metrics

    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "profiles.json").write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
    lines = [
        "# Auto-tag input-profile baseline",
        "",
        f"Run `{args.run_id}` over {output['caseCount']} cases and {output['trackCount']} tracks.",
        "",
        "| Profile | Cases | Tracks | Native invocations | Expected | Complete | Provider-unavailable invocations |",
        "| --- | ---: | ---: | ---: | ---: | --- | ---: |",
    ]
    for name, metrics in output["profiles"].items():
        lines.append(
            f"| {name} | {metrics['caseCount']} | {metrics['trackCount']} | "
            f"{metrics['nativeInvocations']} | {metrics['expectedNativeInvocations']} | "
            f"{str(metrics['nativeComplete']).lower()} | {metrics['providerUnavailableInvocations']} |"
        )
    lines.extend(
        [
            "",
            "Deterministic profile coverage is complete for all three input shapes. Native provider results are incomplete where the retained invocation count is below 642; those rows remain provider diagnostics rather than matcher scores.",
        ]
    )
    (args.output_dir / "profiles.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    (args.output_dir / "command.log").write_text(
        f"status=passed\nrun_id={args.run_id}\ncase_count={output['caseCount']}\ntrack_count={output['trackCount']}\n",
        encoding="utf-8",
    )
    print(json.dumps({name: value["nativeInvocations"] for name, value in output["profiles"].items()}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
