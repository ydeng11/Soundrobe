#!/usr/bin/env python3
"""Review a frozen, representative auto-tag corpus subset offline.

The reviewer never discovers a release.  Each row names a checked-in provider
snapshot and the script proves that the local tracklist has a one-to-one,
duration-compatible mapping before a match can be promoted.  Rows without a
complete frozen payload remain explicitly unresolved.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import unicodedata
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


def title_keys(value: Any) -> set[str]:
    """Return conservative comparison keys for provider title variants."""
    if not isinstance(value, str):
        return set()
    variants = [value]
    # Discogs uses this form for bilingual Japanese releases.  The left side
    # is the catalog title; the right side is a translation and is not a
    # performance qualifier.
    if "=" in value:
        variants.append(value.split("=", 1)[0])
    result: set[str] = set()
    for variant in variants:
        text = unicodedata.normalize("NFKD", variant)
        text = text.replace("æ", "ae").replace("Æ", "AE")
        text = text.replace("œ", "oe").replace("Œ", "OE")
        text = "".join(char for char in text if not unicodedata.combining(char))
        result.add("".join(char.casefold() for char in text if char.isalnum()))
    return {key for key in result if key}


def duration_seconds(value: Any) -> float | None:
    if isinstance(value, (int, float)) and value > 0:
        # MusicBrainz lengths are milliseconds; Discogs values are seconds.
        return float(value) / 1000 if float(value) > 1000 else float(value)
    if not isinstance(value, str) or not value.strip():
        return None
    pieces = value.strip().split(":")
    try:
        result = 0.0
        for piece in pieces:
            result = result * 60 + float(piece)
        return result if result > 0 else None
    except ValueError:
        return None


def duration_compatible(local: Any, provider: Any) -> bool:
    left = duration_seconds(local)
    right = duration_seconds(provider)
    if left is None or right is None:
        return True
    return abs(left - right) <= max(5.0, left * 0.03)


def provider_tracks(payload: dict[str, Any], provider: str) -> list[dict[str, Any]]:
    tracks: list[dict[str, Any]] = []
    # Relapse's checked-in snapshots are the provider-normalized candidate
    # emitted by the native resolver.  They remain frozen evidence and use the
    # same fields as the raw payload adapters below.
    if isinstance(payload.get("tracks"), list):
        for index, item in enumerate(payload["tracks"], 1):
            title = item.get("title")
            if not isinstance(title, str) or not title.strip():
                continue
            tracks.append(
                {
                    "position": str(item.get("track_number") or index),
                    "title": title,
                    "duration": item.get("length"),
                    "artists": [str(value) for value in item.get("artists", []) if value],
                }
            )
        return tracks
    if provider == "discogs":
        for item in payload.get("tracklist", []):
            if item.get("type_") not in (None, "track"):
                continue
            title = item.get("title")
            if not isinstance(title, str) or not title.strip():
                continue
            tracks.append(
                {
                    "position": str(item.get("position") or ""),
                    "title": title,
                    "duration": item.get("duration"),
                    "artists": [str(a.get("name")) for a in item.get("artists", []) if a.get("name")],
                }
            )
        return tracks
    media = payload.get("media", [])
    qualify_medium = len(media) > 1
    for medium in media:
        medium_position = str(medium.get("position") or "")
        for item in medium.get("tracks", []):
            title = item.get("title")
            if not isinstance(title, str) or not title.strip():
                continue
            artists = []
            for credit in item.get("artist-credit", []):
                if isinstance(credit, dict) and isinstance(credit.get("artist"), dict):
                    name = credit["artist"].get("name")
                    if name:
                        artists.append(str(name))
            tracks.append(
                {
                    "position": (
                        f"{medium_position}-{item.get('position')}"
                        if qualify_medium and medium_position and item.get("position")
                        else str(item.get("position") or "")
                    ),
                    "title": title,
                    "duration": item.get("length"),
                    "artists": artists,
                }
            )
    return tracks


def artist_name(payload: dict[str, Any], provider: str) -> str | None:
    if isinstance(payload.get("artist"), str):
        return payload["artist"]
    if provider == "discogs":
        artists = payload.get("artists") or []
        return artists[0].get("name") if artists and isinstance(artists[0], dict) else None
    credits = payload.get("artist-credit") or []
    for credit in credits:
        if isinstance(credit, dict) and isinstance(credit.get("artist"), dict):
            return credit["artist"].get("name")
    return None


def compare_case(case: dict[str, Any], row: dict[str, Any], fixture_root: Path) -> dict[str, Any]:
    status = row.get("status")
    if status == "unresolved":
        reason = row.get("reason")
        if not isinstance(reason, str) or not reason.strip():
            raise ValueError(f"unresolved case {case['caseId']} needs a reason")
        return {
            "caseId": case["caseId"],
            "artist": case["artist"],
            "status": "unresolved",
            "reason": reason,
            "acceptableEditionIds": [],
            "hardNegativeIds": [str(value) for value in row.get("hardNegativeIds", [])],
            "mapping": [],
            "flags": ["unresolved"],
        }
    if status != "verified_match":
        raise ValueError(f"unsupported subset status for {case['caseId']}: {status!r}")
    provider = row.get("provider")
    snapshot = row.get("snapshot")
    release_id = row.get("releaseId")
    if provider not in {"discogs", "musicbrainz"} or not isinstance(snapshot, str) or not isinstance(release_id, str):
        raise ValueError(f"verified case {case['caseId']} has incomplete provider identity")
    path = (fixture_root / snapshot).resolve()
    try:
        path.relative_to(fixture_root.resolve())
    except ValueError as error:
        raise ValueError(f"snapshot escapes fixture root: {snapshot}") from error
    if not path.is_file():
        raise ValueError(f"missing provider snapshot for {case['caseId']}: {snapshot}")
    payload = read_json(path)
    tracks = provider_tracks(payload, provider)
    if not tracks:
        raise ValueError(f"provider snapshot has no tracks for {case['caseId']}")
    used: set[int] = set()
    exact_used: set[int] = set()
    exact_title_matches = 0
    mapping: list[dict[str, Any]] = []
    flags: set[str] = set()
    failures: list[dict[str, Any]] = []
    for ordinal, local in enumerate(case.get("tracks", []), 1):
        local_keys = title_keys(local.get("title"))
        exact_candidates = [
            index
            for index, remote in enumerate(tracks)
            if index not in exact_used and local.get("title") == remote["title"]
        ]
        if len(exact_candidates) == 1:
            exact_used.add(exact_candidates[0])
            exact_title_matches += 1
        candidates = [
            (index, remote)
            for index, remote in enumerate(tracks)
            if index not in used and local_keys.intersection(title_keys(remote["title"]))
        ]
        if len(candidates) != 1:
            failures.append(
                {
                    "localTrack": ordinal,
                    "title": local.get("title"),
                    "candidateCount": len(candidates),
                }
            )
            continue
        index, remote = candidates[0]
        used.add(index)
        exact = local.get("title") == remote["title"]
        if not exact:
            flags.add("normalized-title")
        if duration_seconds(remote.get("duration")) is None:
            flags.add("provider-duration-missing")
        elif not duration_compatible(local.get("duration"), remote.get("duration")):
            flags.add("duration-conflict")
            failures.append(
                {
                    "localTrack": ordinal,
                    "title": local.get("title"),
                    "providerPosition": remote["position"],
                    "reason": "duration-conflict",
                }
            )
            continue
        mapping.append(
            {
                "localTrack": ordinal,
                "providerTrack": remote["position"],
                "localTitle": local.get("title"),
                "providerTitle": remote["title"],
                "durationDeltaSeconds": round(
                    abs((duration_seconds(local.get("duration")) or 0) - (duration_seconds(remote.get("duration")) or 0)),
                    3,
                )
                if duration_seconds(remote.get("duration")) is not None
                else None,
            }
        )
    if failures or len(mapping) != len(case.get("tracks", [])):
        raise ValueError(
            f"verified case {case['caseId']} is not complete: "
            + json.dumps({"failures": failures, "mapped": len(mapping)}, ensure_ascii=False)
        )
    unmatched = [remote for index, remote in enumerate(tracks) if index not in used]
    policy = row.get("providerTrackPolicy")
    if unmatched:
        if not isinstance(policy, dict):
            raise ValueError(
                f"verified case {case['caseId']} leaves provider tracks unmatched: "
                + json.dumps([remote["position"] for remote in unmatched])
            )
        kind = policy.get("kind")
        extra_positions = {str(remote["position"]) for remote in unmatched}
        if kind == "selected_media":
            media_position = str(policy.get("mediaPosition") or "")
            if not media_position or any(
                str(item["providerTrack"]).split("-", 1)[0] != media_position
                for item in mapping
            ) or any(
                str(remote["position"]).split("-", 1)[0] == media_position
                for remote in unmatched
            ):
                raise ValueError(
                    f"verified case {case['caseId']} has an invalid selected-media policy"
                )
        elif kind == "allowed_extras":
            allowed = {str(value) for value in policy.get("providerTracks", [])}
            if extra_positions != allowed:
                raise ValueError(
                    f"verified case {case['caseId']} has unexpected provider extras: "
                    + json.dumps(sorted(extra_positions))
                )
        else:
            raise ValueError(f"verified case {case['caseId']} has an invalid provider track policy")
    elif policy:
        raise ValueError(f"verified case {case['caseId']} has an unnecessary provider track policy")
    local_artist = str(case.get("artist") or "")
    provider_artist = artist_name(payload, provider)
    if provider_artist and title_keys(provider_artist).isdisjoint(title_keys(local_artist)):
        flags.add("artist-conflict")
        raise ValueError(f"verified case {case['caseId']} has provider artist conflict: {provider_artist}")
    return {
        "caseId": case["caseId"],
        "artist": case["artist"],
        "status": "verified_match",
        "provider": provider,
        "releaseId": release_id,
        "snapshot": snapshot,
        "snapshotSha256": sha256_file(path),
        "acceptableEditionIds": [release_id],
        "hardNegativeIds": [str(value) for value in row.get("hardNegativeIds", [])],
        "mapping": mapping,
        "providerTrackPolicy": policy,
        "unmatchedProviderTracks": [remote["position"] for remote in unmatched],
        "flags": sorted(flags),
        "trackCount": len(mapping),
        "providerTrackCount": len(tracks),
        "baselineExactTitleMatches": exact_title_matches,
        "baselineExactComplete": exact_title_matches == len(case.get("tracks", [])),
        "improvementAttribution": "normalized-title" if exact_title_matches < len(mapping) else "none",
    }


def review(corpus: dict[str, Any], manifest: dict[str, Any], fixture_root: Path) -> dict[str, Any]:
    if manifest.get("schemaVersion") != 1:
        raise ValueError("subset manifest schemaVersion must be 1")
    cases = {case.get("caseId"): case for case in corpus.get("cases", [])}
    rows = manifest.get("cases")
    if not isinstance(rows, list) or not rows:
        raise ValueError("subset manifest has no cases")
    if len({row.get("caseId") for row in rows}) != len(rows):
        raise ValueError("subset manifest contains duplicate case IDs")
    reviewed = []
    for row in rows:
        case_id = row.get("caseId")
        if case_id not in cases:
            raise ValueError(f"subset case is absent from corpus: {case_id}")
        reviewed.append(compare_case(cases[case_id], row, fixture_root))
    counts = {status: sum(item["status"] == status for item in reviewed) for status in ["verified_match", "unresolved"]}
    matches = [item for item in reviewed if item["status"] == "verified_match"]
    return {
        "schemaVersion": 1,
        "corpusVersion": corpus.get("corpusVersion"),
        "manifestVersion": manifest.get("manifestVersion", 1),
        "caseCount": len(reviewed),
        "trackCount": sum(item.get("trackCount", len(cases[item["caseId"]].get("tracks", []))) for item in reviewed),
        "statusCounts": counts,
        "providerSnapshotCount": sum(item["status"] == "verified_match" for item in reviewed),
        "baseline": {
            "exactTitleMatches": sum(item.get("baselineExactTitleMatches", 0) for item in matches),
            "trackCount": sum(len(cases[item["caseId"]].get("tracks", [])) for item in matches),
            "completeCases": sum(item.get("baselineExactComplete", False) for item in matches),
        },
        "final": {
            "strongMatches": sum(item.get("trackCount", 0) for item in matches),
            "completeCases": len(matches),
        },
        "cases": reviewed,
    }


def expectation_ledger(result: dict[str, Any]) -> dict[str, Any]:
    """Export the reviewed subset in the native evaluator's expectation shape."""
    return {
        "schemaVersion": result["schemaVersion"],
        "corpusVersion": result["corpusVersion"],
        "cases": [
            {
                "caseId": item["caseId"],
                "status": item["status"],
                "acceptableEditionIds": item.get("acceptableEditionIds", []),
                "rejectedHardNegativeIds": item.get("hardNegativeIds", []),
                "matcherAttribution": item.get("improvementAttribution") == "normalized-title",
                "providerTrackCount": item.get("providerTrackCount"),
                "providerTrackPolicy": item.get("providerTrackPolicy"),
                "unmatchedProviderTracks": item.get("unmatchedProviderTracks", []),
                "mapping": item.get("mapping", []),
            }
            for item in result["cases"]
        ],
    }


def render(result: dict[str, Any]) -> str:
    lines = [
        "# Reviewed auto-tag representative subset",
        "",
        f"- Cases reviewed: {result['caseCount']}",
        f"- Tracks covered: {result['trackCount']}",
        f"- Verified matches: {result['statusCounts']['verified_match']}",
        f"- Explicitly unresolved: {result['statusCounts']['unresolved']}",
        f"- Frozen provider snapshots: {result['providerSnapshotCount']}",
        f"- Baseline exact-title matches: {result['baseline']['exactTitleMatches']}/{result['baseline']['trackCount']} tracks ({result['baseline']['completeCases']} complete cases)",
        f"- Final unique normalized-title matches: {result['final']['strongMatches']}/{result['baseline']['trackCount']} tracks ({result['final']['completeCases']} complete cases)",
        "",
        "A verified match has a complete one-to-one title mapping and compatible durations against a checked-in provider response. Unresolved rows are retained for representative coverage and do not contribute to precision or coverage.",
        "",
        "| Case | Artist | Status | Provider/release | Tracks | Flags |",
        "|---|---|---|---|---:|---|",
    ]
    for item in result["cases"]:
        identity = f"{item.get('provider', '')}/{item.get('releaseId', '')}" if item["status"] == "verified_match" else "—"
        lines.append(
            f"| {item['caseId']} | {item['artist']} | {item['status']} | {identity} | "
            f"{item.get('trackCount', '—')} | {', '.join(item.get('flags', [])) or '—'} |"
        )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--fixture-root", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    result = review(read_json(args.corpus), read_json(args.manifest), args.fixture_root)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "review.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    (args.output_dir / "expectations.json").write_text(
        json.dumps(expectation_ledger(result), indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    (args.output_dir / "review.md").write_text(render(result), encoding="utf-8")
    (args.output_dir / "command.log").write_text(
        f"status=passed\ncase_count={result['caseCount']}\ntrack_count={result['trackCount']}\n",
        encoding="utf-8",
    )
    print(json.dumps({k: result[k] for k in ["caseCount", "trackCount", "statusCounts", "providerSnapshotCount"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
