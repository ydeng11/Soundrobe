#!/usr/bin/env python3
"""Score reviewed auto-tag cases without contacting providers.

The native evaluator deliberately keeps most corpus expectations unverified.
This script makes that boundary explicit while scoring any reviewed cases that
have an acceptable-edition ledger.  It also reports provider recovery and
cold/warm identity drift separately from matcher attribution.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
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


def stable_partition(release_group_id: str) -> str:
    return "development" if hashlib.sha256(release_group_id.encode()).digest()[0] < 204 else "holdout"


def identity_id(value: Any) -> str | None:
    if isinstance(value, str) and value:
        return value
    if isinstance(value, dict):
        for key in ("discogsReleaseId", "discogs_release_id", "musicbrainzAlbumId", "musicbrainz_album_id"):
            candidate = value.get(key)
            if candidate:
                return str(candidate)
    return None


def evidence_contains(record: dict[str, Any], evidence: str) -> bool:
    values = record.get("selectedTrackEvidence")
    return isinstance(values, dict) and evidence in values.get("evidence", [])


def provider_has_unavailable(record: dict[str, Any]) -> bool:
    native = record.get("native")
    attempts = native.get("providerAttempts", []) if isinstance(native, dict) else []
    return any(isinstance(attempt, dict) and attempt.get("status") == "unavailable" for attempt in attempts)


def invocation_by_phase(results: dict[str, Any]) -> dict[str, dict[str, dict[str, Any]]]:
    phases: dict[str, dict[str, dict[str, Any]]] = {}
    for record in results.get("invocations", []):
        if not isinstance(record, dict):
            continue
        case_id = record.get("caseId")
        phase = record.get("phase")
        if isinstance(case_id, str) and isinstance(phase, str):
            phases.setdefault(case_id, {})[phase] = record
    return phases


def diagnostic_observations(results: dict[str, Any]) -> dict[str, Any]:
    phases = invocation_by_phase(results)
    provider_unavailable: set[str] = set()
    warm_recovery: set[str] = set()
    guarded_title_records = 0
    for case_id, records in phases.items():
        if any(provider_has_unavailable(record) for record in records.values()):
            provider_unavailable.add(case_id)
        cold = records.get("cold", {})
        warm = records.get("warm", {})
        if cold.get("classification") == "incomplete" and warm.get("classification") == "confirmed_success":
            warm_recovery.add(case_id)
        guarded_title_records += sum(
            evidence_contains(record, "GuardedTitle") for record in records.values()
        )
    identity_inconsistencies = []
    for folder in results.get("folderResults", []):
        if not isinstance(folder, dict):
            continue
        cold_id = identity_id(folder.get("coldIdentity"))
        warm_id = identity_id(folder.get("warmIdentity"))
        if cold_id and warm_id and cold_id != warm_id:
            case_id = folder.get("caseId")
            records = phases.get(case_id, {}) if isinstance(case_id, str) else {}
            identity_inconsistencies.append(
                {
                    "caseId": folder.get("caseId"),
                    "sourceRelativeFolder": folder.get("sourceRelativeFolder"),
                    "coldIdentity": cold_id,
                    "warmIdentity": warm_id,
                    "coldAuthority": records.get("cold", {}).get("native", {}).get("authority"),
                    "warmAuthority": records.get("warm", {}).get("native", {}).get("authority"),
                    "coldClassification": folder.get("coldClassification"),
                    "warmClassification": folder.get("warmClassification"),
                }
            )
    return {
        "providerUnavailableCases": len(provider_unavailable),
        "warmRecoveryCases": len(warm_recovery),
        "coldWarmIdentityInconsistencies": identity_inconsistencies,
        "guardedTitleInvocationRecords": guarded_title_records,
    }


def classify_case(expectation: dict[str, Any], folder: dict[str, Any] | None) -> str:
    if not folder:
        return "incomplete"
    status = expectation.get("status")
    selected = identity_id(folder.get("selectedIdentity"))
    acceptable = {str(value) for value in expectation.get("acceptableEditionIds", [])}
    hard_negative = {str(value) for value in expectation.get("rejectedHardNegativeIds", [])}
    final = folder.get("classification")
    if selected in hard_negative or final == "wrong_match":
        return "wrong_match"
    if status == "verified_match" and selected in acceptable and final == "confirmed_success":
        return "correct"
    if status == "verified_abstain" and final == "safe_abstention":
        return "correct"
    if final == "failed_verification":
        return "failed_verification"
    if final == "incomplete":
        return "incomplete"
    return "unresolved"


def score_results(corpus: dict[str, Any], expectations: dict[str, Any], results: dict[str, Any]) -> dict[str, Any]:
    expectation_by_id = {case["caseId"]: case for case in expectations.get("cases", [])}
    folder_by_id = {folder["caseId"]: folder for folder in results.get("folderResults", [])}
    invocations = invocation_by_phase(results)
    scored = Counter()
    partitions = {"development": Counter(), "holdout": Counter()}
    provider_unavailable: set[str] = set()
    warm_recovery: set[str] = set()
    identity_inconsistent: set[str] = set()
    matcher_attribution: set[str] = set()
    case_results = []

    for case_id, expectation in expectation_by_id.items():
        status = expectation.get("status")
        if status not in ("verified_match", "verified_abstain"):
            continue
        outcome = classify_case(expectation, folder_by_id.get(case_id))
        scored[outcome] += 1
        case = next((item for item in corpus.get("cases", []) if item.get("caseId") == case_id), None)
        partition = stable_partition(str(case.get("releaseGroupId", ""))) if case else "development"
        partitions[partition][outcome] += 1
        phases = invocations.get(case_id, {})
        case_results.append(
            {
                "caseId": case_id,
                "status": status,
                "outcome": outcome,
                "partition": partition,
                "selectedIdentity": identity_id((folder_by_id.get(case_id) or {}).get("selectedIdentity")),
                "coldIdentity": identity_id(phases.get("cold", {}).get("selectedIdentity")),
                "warmIdentity": identity_id(phases.get("warm", {}).get("selectedIdentity")),
            }
        )
        if any(provider_has_unavailable(record) for record in phases.values()):
            provider_unavailable.add(case_id)
        cold = phases.get("cold", {})
        warm = phases.get("warm", {})
        if cold.get("classification") == "incomplete" and warm.get("classification") == "confirmed_success":
            warm_recovery.add(case_id)
        cold_id = identity_id(cold.get("selectedIdentity"))
        warm_id = identity_id(warm.get("selectedIdentity"))
        if cold_id and warm_id and cold_id != warm_id:
            identity_inconsistent.add(case_id)
        if outcome == "correct" and any(evidence_contains(record, "GuardedTitle") for record in phases.values()):
            # Native attribution is only valid when the reviewed ledger also
            # records a prior failure on this case.
            if expectation.get("matcherAttribution") is True:
                matcher_attribution.add(case_id)

    eligible = sum(scored.values())
    decisive = scored["correct"] + scored["wrong_match"]
    return {
        "eligibleCases": eligible,
        "correct": scored["correct"],
        "wrongMatch": scored["wrong_match"],
        "unresolved": scored["unresolved"],
        "incomplete": scored["incomplete"],
        "failedVerification": scored["failed_verification"],
        "coverage": scored["correct"] / eligible if eligible else None,
        "precision": scored["correct"] / decisive if decisive else None,
        "partitionCounts": {name: dict(counts) for name, counts in partitions.items()},
        "providerUnavailableCases": len(provider_unavailable),
        "warmRecoveryCases": len(warm_recovery),
        "coldWarmIdentityInconsistencies": len(identity_inconsistent),
        "nativeMatcherAttributionCases": len(matcher_attribution),
        "cases": sorted(case_results, key=lambda item: item["caseId"]),
        "diagnostic": diagnostic_observations(results),
    }


def score_reviewed_truth(truth: dict[str, Any]) -> dict[str, Any]:
    baseline = truth.get("baseline", {})
    post_fix = truth.get("postFix", {})
    total = len(truth.get("mapping", []))

    def metrics(values: dict[str, Any]) -> dict[str, Any]:
        strong = int(values.get("strongTitleMatches", values.get("strongMatches", 0)))
        positional = int(values.get("positionOnlyMatches", 0))
        guarded = int(values.get("guardedTitleMatches", 0))
        return {
            "strongEvidenceMatches": strong,
            "positionOnlyMatches": positional,
            "guardedTitleMatches": guarded,
            "strongEvidenceCoverage": strong / total if total else None,
            "positionOnlyRate": positional / total if total else None,
        }

    before = metrics(baseline)
    after = metrics(post_fix)
    return {
        "case": truth.get("source"),
        "status": truth.get("status"),
        "acceptableEditionIds": truth.get("acceptableEditionIds", []),
        "rejectedHardNegativeIds": truth.get("rejectedHardNegativeIds", []),
        "trackCount": total,
        "baseline": before,
        "postFix": after,
        "delta": {
            "strongEvidenceMatches": after["strongEvidenceMatches"] - before["strongEvidenceMatches"],
            "positionOnlyMatches": after["positionOnlyMatches"] - before["positionOnlyMatches"],
            "strongEvidenceCoverage": (after["strongEvidenceCoverage"] - before["strongEvidenceCoverage"])
            if before["strongEvidenceCoverage"] is not None and after["strongEvidenceCoverage"] is not None
            else None,
        },
        "matcherAttribution": (
            truth.get("status") == "verified_match"
            and before["positionOnlyMatches"] > 0
            and after["guardedTitleMatches"] > 0
        ),
    }


def render_report(score: dict[str, Any], run_id: str) -> str:
    reviewed = score["reviewedTruth"]
    metrics = reviewed["postFix"]

    def display(value: Any) -> str:
        return "n/a" if value is None else str(value)

    lines = [
        "# Auto-tag scored evaluation",
        "",
        f"Run `{run_id}` using frozen native results and reviewed ledgers.",
        "",
        (
            "Only cases marked `verified_match` or `verified_abstain` are scored. "
            "All other corpus cases remain diagnostic."
        ),
        "",
        "## Scored metrics",
        "",
        f"- Eligible cases: {score['scored']['eligibleCases']}",
        f"- Correct: {score['scored']['correct']}",
        f"- Wrong match: {score['scored']['wrongMatch']}",
        f"- Unresolved: {score['scored']['unresolved']}",
        f"- Incomplete: {score['scored']['incomplete']}",
        f"- Failed verification: {score['scored']['failedVerification']}",
        f"- Coverage: {display(score['scored']['coverage'])}",
        f"- Precision among decisive cases: {display(score['scored']['precision'])}",
        "",
        "## Scored cases",
        "",
        *[
            f"- {item['caseId']} [{item['partition']}]: {item['outcome']}"
            + (f" ({item['selectedIdentity']})" if item["selectedIdentity"] else "")
            for item in score["scored"]["cases"]
        ],
        "- none" if not score["scored"]["cases"] else "",
        "",
        "## Recovery and attribution",
        "",
        f"- Provider-unavailable cases: {score['scored']['providerUnavailableCases']}",
        f"- Warm recovery cases: {score['scored']['warmRecoveryCases']}",
        f"- Cold/warm identity inconsistencies: {score['scored']['coldWarmIdentityInconsistencies']}",
        f"- Native GuardedTitle attributions: {score['scored']['nativeMatcherAttributionCases']}",
        "",
        "## Diagnostic replay observations",
        "",
        "- Provider-unavailable cases (all replayed cases): "
        f"{score['scored']['diagnostic']['providerUnavailableCases']}",
        f"- Warm recovery cases (all replayed cases): {score['scored']['diagnostic']['warmRecoveryCases']}",
        "- GuardedTitle invocation records (all replayed cases): "
        f"{score['scored']['diagnostic']['guardedTitleInvocationRecords']}",
        *[
            "- Cold/warm identity inconsistency: "
            f"{item['caseId']} ({item['sourceRelativeFolder']}): "
            f"{item['coldIdentity']} ({item['coldAuthority'] or 'unknown'}) → "
            f"{item['warmIdentity']} ({item['warmAuthority'] or 'unknown'})"
            for item in score['scored']['diagnostic']['coldWarmIdentityInconsistencies']
        ],
        "- Cold/warm identity inconsistencies: none"
        if not score['scored']['diagnostic']['coldWarmIdentityInconsistencies']
        else "",
        "",
        "## Reviewed Relapse regression",
        "",
        f"- Acceptable edition: {', '.join(reviewed['acceptableEditionIds'])}",
        f"- Hard negatives: {', '.join(reviewed['rejectedHardNegativeIds'])}",
        f"- Baseline strong evidence: {reviewed['baseline']['strongEvidenceMatches']}/{reviewed['trackCount']}",
        f"- Post-fix strong evidence: {metrics['strongEvidenceMatches']}/{reviewed['trackCount']}",
        f"- Post-fix GuardedTitle evidence: {metrics['guardedTitleMatches']}",
        f"- Reviewed matcher attribution: {reviewed['matcherAttribution']}",
        "",
        "Unreviewed corpus cases remain outside precision and coverage.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--expectations", type=Path, required=True)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--reviewed-truth", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--run-id", default="scored-evaluation")
    args = parser.parse_args()
    corpus = read_json(args.corpus)
    expectations = read_json(args.expectations)
    results = read_json(args.results)
    truth = read_json(args.reviewed_truth)
    score = {
        "schemaVersion": 1,
        "runId": args.run_id,
        "corpusVersion": corpus.get("corpusVersion"),
        "sourceResults": str(args.results),
        "inputSha256": {
            "corpus": sha256_file(args.corpus),
            "expectations": sha256_file(args.expectations),
            "results": sha256_file(args.results),
            "reviewedTruth": sha256_file(args.reviewed_truth),
        },
        "scored": score_results(corpus, expectations, results),
        "reviewedTruth": score_reviewed_truth(truth),
        "notes": [
            "Unverified expectations are excluded from scored precision and coverage.",
            "Provider recovery and cold/warm identity drift are reported separately from matcher attribution.",
        ],
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "score.json").write_text(
        json.dumps(score, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    (args.output_dir / "score.md").write_text(render_report(score, args.run_id), encoding="utf-8")
    (args.output_dir / "command.log").write_text(
        f"status=passed\nrun_id={args.run_id}\nscored_cases={score['scored']['eligibleCases']}\n"
        "providers=none\nnetwork=disabled\n",
        encoding="utf-8",
    )
    print(json.dumps(score["scored"], indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
