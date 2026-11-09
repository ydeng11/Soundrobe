#!/usr/bin/env python3
"""Freeze batch membership and input locks; never discover or promote new truth."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path

from audit_auto_tag_ground_truth import audit, read_json, sha256_file
from audit_auto_tag_reproducibility import audit_candidate_pools, safe_resolve, validate_digest
from review_auto_tag_subset import review


def build(args: argparse.Namespace) -> dict:
    root = args.fixture_root.resolve()
    inputs = {}

    def lock(path: Path) -> str:
        relative = path.resolve().relative_to(root).as_posix()
        inputs[relative] = sha256_file(path)
        return relative

    cases = {}
    corpora = {}
    answers = {}
    for path in sorted(args.corpus):
        corpus = read_json(path)
        ledger_path = path.with_name("expectations.json")
        ledger = read_json(ledger_path)
        audit(corpus, ledger)
        relative = lock(path)
        ledger_relative = lock(ledger_path)
        corpora[relative] = corpus
        if corpus.get("selectionManifest"):
            lock(safe_resolve(path.parent, corpus["selectionManifest"], "selection manifest", root))
        answers.update({row["caseId"]: row for row in ledger["cases"]})
        for case in corpus["cases"]:
            case_id = case["caseId"]
            if case_id in cases:
                raise ValueError(f"duplicate case across corpora: {case_id}")
            cases[case_id] = {
                "caseId": case_id,
                "corpus": relative,
                "artist": case["artist"],
                "sourceRelativeFolder": case["sourceRelativeFolder"],
                "releaseGroupId": case["releaseGroupId"],
                "releaseType": case["releaseType"],
                "trackCount": len(case["tracks"]),
                "status": answers[case_id]["status"],
                "expectationSource": ledger_relative,
                "difficultyLabels": case.get("difficultyLabels", []),
                "selectionRationale": case.get("selectionRationale"),
            }

    for manifest_path in args.reviewed_manifest:
        manifest = read_json(manifest_path)
        matching = [value for value in corpora.values() if value["corpusVersion"] == manifest["corpusVersion"]]
        if len(matching) != 1:
            raise ValueError("reviewed manifest must identify exactly one corpus version")
        result = review(matching[0], manifest, manifest_path.parent)
        source = lock(manifest_path)
        for row in result["cases"]:
            case = cases[row["caseId"]]
            if case["status"] != "unverified":
                previous = answers[row["caseId"]]
                previous_mapping = {(item["localTrack"], str(item["providerTrack"])) for item in previous["mapping"]}
                mapping = {(item["localTrack"], str(item["providerTrack"])) for item in row["mapping"]}
                if (previous["status"] != row["status"] or previous_mapping != mapping
                        or set(previous["acceptableEditionIds"]) != set(row["acceptableEditionIds"])
                        or set(previous["rejectedHardNegativeIds"]) != set(row["hardNegativeIds"])):
                    raise ValueError(f"conflicting reviewed answers need reconciliation: {row['caseId']}")
            case.update(status=row["status"], expectationSource=source)
            answers[row["caseId"]] = {**row, "rejectedHardNegativeIds": row["hardNegativeIds"]}
            if row.get("snapshot"):
                lock(manifest_path.parent / row["snapshot"])

    if args.candidate_pools:
        pools = audit_candidate_pools(args.candidate_pools)
        lock(args.candidate_pools)
        for pool in pools:
            lock(Path(pool["localFixture"]))
            for candidate in pool["candidates"]:
                lock(Path(candidate["response"]))
    if args.discovery:
        discovery = read_json(args.discovery)
        lock(args.discovery)
        for record in discovery["records"]:
            path = safe_resolve(args.discovery.parent, record["bodyFile"], "discovery body", root)
            validate_digest(path, record["sha256"], "discovery body")
            lock(path)

    batches = {}
    family_splits = {}
    for case in sorted(cases.values(), key=lambda item: item["caseId"]):
        compilation = case["releaseType"] == "compilation"
        key = ("compilation_album:" + case["releaseGroupId"]) if compilation else ("artist:" + case["artist"])
        split = "holdout" if int(hashlib.sha256(key.encode()).hexdigest(), 16) % 5 == 0 else "development"
        family = case["releaseGroupId"]
        if family in family_splits and family_splits[family] != split:
            raise ValueError(f"release family crosses splits: {family}")
        family_splits[family] = split
        batch = batches.setdefault(key, {
            "batchId": key,
            "kind": "compilation_album" if compilation else "artist_batch",
            "discovery": "album_only" if compilation else "shared_artist_catalog",
            "split": split,
            "caseIds": [],
            "trackCount": 0,
        })
        batch["caseIds"].append(case["caseId"])
        batch["trackCount"] += case["trackCount"]
        case.update(batchId=key, split=split, scored=case["status"] in {"verified_match", "verified_abstain"})

    counts = Counter(case["status"] for case in cases.values())
    scored = sum(case["scored"] for case in cases.values())
    return {
        "schemaVersion": 1,
        "splitPolicy": "SHA-256(batchId) modulo 5: zero is holdout; retrospective partition, not an unseen benchmark",
        "profiles": ["folder_filename", "assisted_without_ids", "tagged_recovery"],
        "runRequirements": {
            "sameHttpAttemptBudgetAcrossArms": True,
            "countRetriesAndFailures": True,
            "separateColdAndWarmCaches": True,
            "writeOnlyDisposableCopies": True,
            "syntheticAudioDoesNotTestRecognition": True,
        },
        "summary": {
            "cases": len(cases),
            "tracks": sum(case["trackCount"] for case in cases.values()),
            "scoredCases": scored,
            "unscoredCases": len(cases) - scored,
            "statusCounts": dict(sorted(counts.items())),
            "batches": len(batches),
            "compilationBatches": sum(batch["kind"] == "compilation_album" for batch in batches.values()),
        },
        "inputs": [{"path": path, "sha256": digest} for path, digest in sorted(inputs.items())],
        "batches": [batches[key] for key in sorted(batches)],
        "cases": list(sorted(cases.values(), key=lambda item: item["caseId"])),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture-root", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, action="append", required=True)
    parser.add_argument("--reviewed-manifest", type=Path, action="append", default=[])
    parser.add_argument("--candidate-pools", type=Path)
    parser.add_argument("--discovery", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--check", action="store_true", help="fail on changed locks or membership without rewriting")
    args = parser.parse_args()
    result = build(args)
    text = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.check:
        if args.output.read_text(encoding="utf-8") != text:
            raise ValueError("suite differs from frozen inputs or membership; review before regenerating")
    else:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text, encoding="utf-8")
    print(json.dumps(result["summary"], sort_keys=True))


if __name__ == "__main__":
    main()
