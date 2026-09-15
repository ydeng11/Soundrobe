#!/usr/bin/env python3
"""Audit the reviewed expectation ledger against every corpus case.

This is intentionally offline.  It does not infer releases from equal counts or
titles; cases without a provider-backed ledger entry remain explicitly
unscored until an independent reviewer records a complete mapping or a safe
abstention.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path, PurePosixPath
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


def validate_relative_path(value: Any) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError("sourceRelativeFolder must be a non-empty string")
    path = PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts:
        raise ValueError(f"sourceRelativeFolder escapes corpus root: {value}")
    return value


def audit(corpus: dict[str, Any], expectations: dict[str, Any]) -> dict[str, Any]:
    if corpus.get("schemaVersion") != expectations.get("schemaVersion"):
        raise ValueError("corpus and expectations schema versions differ")
    if corpus.get("corpusVersion") != expectations.get("corpusVersion"):
        raise ValueError("corpus and expectations versions differ")
    corpus_cases = corpus.get("cases", [])
    expectation_cases = expectations.get("cases", [])
    corpus_by_id = {case.get("caseId"): case for case in corpus_cases}
    expectation_by_id = {case.get("caseId"): case for case in expectation_cases}
    if len(corpus_by_id) != len(corpus_cases):
        raise ValueError("corpus contains duplicate or missing case IDs")
    if len(expectation_by_id) != len(expectation_cases):
        raise ValueError("expectations contain duplicate or missing case IDs")
    missing = sorted(set(corpus_by_id) - set(expectation_by_id))
    extra = sorted(set(expectation_by_id) - set(corpus_by_id))
    if missing or extra:
        raise ValueError(f"expectation coverage mismatch: missing={missing}, extra={extra}")

    reviewed_cases = []
    status_counts = Counter()
    artist_counts: dict[str, Counter[str]] = {}
    for case_id in sorted(corpus_by_id):
        corpus_case = corpus_by_id[case_id]
        expectation = expectation_by_id[case_id]
        folder = validate_relative_path(corpus_case.get("sourceRelativeFolder"))
        if expectation.get("artist") != corpus_case.get("artist"):
            raise ValueError(f"expectation artist mismatch for {case_id}")
        if expectation.get("releaseGroupId") != corpus_case.get("releaseGroupId"):
            raise ValueError(f"expectation release group mismatch for {case_id}")
        status = expectation.get("status")
        track_count = len(corpus_case.get("tracks", []))
        acceptable = [str(value) for value in expectation.get("acceptableEditionIds", [])]
        hard_negatives = [str(value) for value in expectation.get("rejectedHardNegativeIds", [])]
        mapping = expectation.get("mapping", [])
        rationale = expectation.get("rationale")
        provenance = expectation.get("provenance")
        if status == "verified_match":
            if not acceptable:
                raise ValueError(f"verified match {case_id} has no acceptable edition")
            if len(mapping) != track_count:
                raise ValueError(
                    f"verified match {case_id} mapping has {len(mapping)} rows for {track_count} tracks"
                )
            if any(
                not isinstance(item, dict)
                or not isinstance(item.get("localTrack"), int)
                or not isinstance(item.get("providerTrack"), int)
                for item in mapping
            ):
                raise ValueError(f"verified match {case_id} contains an incomplete mapping row")
            local_tracks = [item["localTrack"] for item in mapping]
            if len(set(local_tracks)) != track_count or set(local_tracks) != set(range(1, track_count + 1)):
                raise ValueError(f"verified match {case_id} mapping does not cover local tracks exactly once")
            scored = True
            blocking_reason = None
        elif status == "verified_abstain":
            if acceptable or mapping:
                raise ValueError(f"verified abstention {case_id} contains match evidence")
            scored = True
            blocking_reason = None
        elif status == "unverified":
            if acceptable or hard_negatives or mapping:
                raise ValueError(f"unverified case {case_id} contains unreviewed release evidence")
            scored = False
            blocking_reason = "provider-backed edition content and mapping review pending"
        else:
            raise ValueError(f"unsupported expectation status for {case_id}: {status!r}")
        if not isinstance(rationale, str) or not rationale.strip():
            raise ValueError(f"expectation {case_id} has no rationale")
        if not isinstance(provenance, str) or not provenance.strip():
            raise ValueError(f"expectation {case_id} has no provenance")
        record = {
            "caseId": case_id,
            "artist": corpus_case.get("artist"),
            "sourceRelativeFolder": folder,
            "trackCount": track_count,
            "status": status,
            "scored": scored,
            "acceptableEditionIds": acceptable,
            "rejectedHardNegativeIds": hard_negatives,
            "mappingCount": len(mapping),
            "rationale": rationale,
            "provenance": provenance,
            "blockingReason": blocking_reason,
        }
        reviewed_cases.append(record)
        status_counts[status] += 1
        artist_counts.setdefault(str(corpus_case.get("artist")), Counter())[status] += 1

    return {
        "schemaVersion": 1,
        "corpusVersion": corpus.get("corpusVersion"),
        "sourceRoot": corpus.get("sourceRoot"),
        "caseCount": len(reviewed_cases),
        "trackCount": sum(item["trackCount"] for item in reviewed_cases),
        "statusCounts": dict(sorted(status_counts.items())),
        "scoredCaseCount": sum(item["scored"] for item in reviewed_cases),
        "unscoredCaseCount": sum(not item["scored"] for item in reviewed_cases),
        "artistStatusCounts": {
            artist: dict(sorted(counts.items()))
            for artist, counts in sorted(artist_counts.items())
        },
        "cases": reviewed_cases,
    }


def render_report(result: dict[str, Any], run_id: str) -> str:
    lines = [
        "# Auto-tag ground-truth audit",
        "",
        f"Run `{run_id}` against corpus `{result['corpusVersion']}` with network disabled.",
        "",
        f"Every corpus case is represented exactly once in the reviewed ledger ({result['caseCount']} cases, {result['trackCount']} tracks).",
        "",
        "## Status",
        "",
        f"- Verified matches: {result['statusCounts'].get('verified_match', 0)}",
        f"- Verified abstentions: {result['statusCounts'].get('verified_abstain', 0)}",
        f"- Explicitly unscored: {result['unscoredCaseCount']}",
        "",
        "A case enters scored metrics only with a complete provider-backed mapping or a justified verified abstention. Unscored cases retain their rationale and remain outside precision and coverage.",
        "",
        "## By artist",
        "",
        "| Artist | Verified match | Verified abstain | Unscored |",
        "|---|---:|---:|---:|",
    ]
    for artist, counts in result["artistStatusCounts"].items():
        lines.append(
            f"| {artist} | {counts.get('verified_match', 0)} | {counts.get('verified_abstain', 0)} | {counts.get('unverified', 0)} |"
        )
    lines.extend(
        [
            "",
            "## Current blocker",
            "",
            "Only the Relapse With Bonus case has a reviewed acceptable edition, hard negatives, and complete mapping. The remaining cases are explicitly unscored pending independent provider content and edition review; equal track counts, title similarity, or a successful runner exit do not promote them.",
            "",
        ]
    )
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--expectations", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--run-id", default="ground-truth-audit")
    parser.add_argument("--expected-case-count", type=int)
    parser.add_argument("--expected-track-count", type=int)
    args = parser.parse_args()
    corpus = read_json(args.corpus)
    expectations = read_json(args.expectations)
    result = audit(corpus, expectations)
    if args.expected_case_count is not None and result["caseCount"] != args.expected_case_count:
        raise ValueError(
            f"expected {args.expected_case_count} cases, found {result['caseCount']}"
        )
    if args.expected_track_count is not None and result["trackCount"] != args.expected_track_count:
        raise ValueError(
            f"expected {args.expected_track_count} tracks, found {result['trackCount']}"
        )
    result["runId"] = args.run_id
    result["inputSha256"] = {
        "corpus": sha256_file(args.corpus),
        "expectations": sha256_file(args.expectations),
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "ground-truth.json").write_text(
        json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    (args.output_dir / "ground-truth.md").write_text(
        render_report(result, args.run_id), encoding="utf-8"
    )
    (args.output_dir / "command.log").write_text(
        f"status=passed\nrun_id={args.run_id}\ncase_count={result['caseCount']}\n"
        "network=disabled\n",
        encoding="utf-8",
    )
    print(json.dumps({key: result[key] for key in ("statusCounts", "scoredCaseCount", "unscoredCaseCount")}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
