"""Derive metadata-only sample receipts. Never open media or execute the app."""
import hashlib
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
FIXTURES = "test/fixtures/tauri/auto-tag-eval/"
ATTEMPT = ".amp/in/artifacts/bounded-entrypoint-attempt-7-20261111/"
REHEARSAL = ".amp/in/artifacts/five-album-offline-rehearsal-20261110-v2/runs/"
LABELS = ".planning/research/bounded-assistance-machine-labels-20261111.json"
PARENT = ".planning/research/bounded-assistance-label-review-20261111.json"
SELECTION = [
    ("036e0a00e45b", "tagged_recovery", "artist-batch-extension/corpus.json", 0,
     ATTEMPT + "genre/preparation-disabled/records.json", "GenreFillResponse"),
    ("54e7923a3000", "tagged_recovery", "corpus.json", 98,
     REHEARSAL + "tagged_recovery-54e7923a3000/results.json", None),
    ("f7756354af37", "folder_filename", "corpus.json", 313,
     ATTEMPT + "hold/preparation-disabled/records.json", None),
]
FIELDS = ["title", "artists", "genre"]
SOURCE_PATHS = [
    "src-tauri/src/commands/auto_tag.rs", "src-tauri/src/commands/auto_tag_eval.rs",
    "src-tauri/src/commands/track_matcher.rs", "src-tauri/src/commands/auto_tag_acoustid.rs",
    "src-tauri/src/commands/mutations.rs", "src-tauri/src/commands/auto_tag_review.rs",
    "src-tauri/src/state/providers.rs", "src-tauri/src/state/write_queue.rs",
    "src-tauri/src/infra/openrouter.rs", "src-tauri/src/infra/bounded_eval.rs",
    "scripts/auto-tag-eval-replay/score-observations.py",
    "scripts/auto-tag-eval-replay/run-bounded.cjs",
    "src-tauri/Cargo.toml", "src-tauri/Cargo.lock",
]


def load(path):
    return json.loads((ROOT / path).read_text())


def digest(path):
    return hashlib.sha256((ROOT / path).read_bytes()).hexdigest()


def derive():
    labels = load(LABELS)
    parent = load(PARENT)
    if digest(PARENT) != labels["parentPacketSha256"]:
        raise ValueError("immutable label parent differs")
    for item in parent["sourceRegister"].values():
        if digest(FIXTURES + item["path"]) != item["sha256"]:
            raise ValueError("raw catalog source differs from reviewed receipt")
    paths = SOURCE_PATHS + [LABELS, PARENT]
    paths += [FIXTURES + item["path"] for item in parent["sourceRegister"].values()]
    jobs = []
    for case_id, profile, corpus, index, baseline, role in SELECTION:
        paths += [FIXTURES + corpus, baseline]
        case = load(FIXTURES + corpus)["cases"][index]
        if case["caseId"] != case_id:
            raise ValueError("case selector changed")
        records = load(baseline)
        if isinstance(records, dict):
            records = records["invocations"]
        phases = {record["phase"]: record for record in records}
        if len(records) != 2 or set(phases) != {"cold", "warm"}:
            raise ValueError("historical phases incomplete")
        expected_paths = {t["relativePath"] for t in case["tracks"]}
        for record in records:
            if set(record["profileBaseline"]) != expected_paths or len(record["beforeTracks"]) != len(expected_paths):
                raise ValueError("historical prewrite denominator differs")
        files = []
        for ordinal, track in enumerate(case["tracks"]):
            relative = track["relativePath"]
            visible = {}
            historical_hashes = {}
            for phase, record in phases.items():
                if (record["caseId"], record["profile"]) != (case_id, profile):
                    raise ValueError("wrong historical input")
                matches = [t for t in record["beforeTracks"] if t["path"].endswith("/" + relative)]
                if len(matches) != 1:
                    raise ValueError("historical file membership mismatch")
                # Paths are kept as inventory-relative selectors, never copied absolute paths.
                visible[phase] = {k: v for k, v in matches[0].items() if k != "path"}
                historical_hashes[phase] = record["profileBaseline"][relative]
            if visible["cold"] != visible["warm"]:
                raise ValueError("historical prewrites differ across phases")
            accepted = {field: None for field in FIELDS}
            label = next((o for o in labels["occurrences"] if o["caseId"] == case_id
                          and o["inventoryOccurrence"].endswith(f"/tracks/{ordinal}")), None)
            if label:
                accepted.update(title=label["acceptedTitleValues"], artists=label["acceptedArtistLists"])
            files.append({
                "path": relative, "inventorySelector": f"/cases/{index}/tracks/{ordinal}",
                "capturedMediaSha256Claim": track["sourceSha256"],
                "profileVisible": visible["cold"], "historicalGeneratedBaseline": historical_hashes,
                "acceptedValues": accepted, "labelOccurrence": label,
                "genreTruth": "unscorable; qualified occurrence/version annotations absent",
                "genreCapture": "unrepresented in inventory; observed null in historical dummy prewrite only",
            })
        jobs.append({
            "caseId": case_id, "profile": profile, "folder": case["sourceRelativeFolder"],
            "inventory": FIXTURES + corpus, "inventorySelector": f"/cases/{index}",
            "historicalPrewrite": baseline, "allowedAssistanceRoleConditional": role,
            "files": files,
        })
    n = sum(len(job["files"]) for job in jobs)
    label_count = sum(v is not None for job in jobs for f in job["files"] for v in f["acceptedValues"].values())
    return {
        "version": 1, "sourceHead": "6cb93861c12628aff6d87332f1721e81ba761080",
        "executionAuthorized": False, "activationAvailable": False,
        "decision": "insufficient", "acceptedCertificate": False, "writableDelta": [],
        "exposure": "All cases exposed development diagnostics, zero untouched holdouts",
        "inputScope": "Metadata-only selection and historical dummy prewrite receipts; not future input admission or original-media missingness",
        "arms": ["disabled", "assistance"], "phases": ["cold", "warm"], "fields": FIELDS,
        "jobs": jobs,
        "denominators": {
            "uniquePhysicalPaths": n, "evaluations": len(jobs) * 4,
            "repeatedPhysicalObservations": n * 4, "fieldSlots": n * 4 * len(FIELDS),
            "uniqueCatalogLabels": label_count, "replicatedCatalogLabelSlots": label_count * 4,
            "genreTruthLabels": 0, "genreMissingOpportunityUnique": len(jobs[0]["files"]),
            "currentlyUnscorableSlots": (n * len(FIELDS) - label_count) * 4,
        },
        "bindings": {p: digest(p) for p in sorted(set(paths))},
    }


def validate(sample, contract):
    if sample != derive():
        raise ValueError("sample/source/label/prewrite/membership receipt differs; no new seal inferred")
    if (contract["activationAvailable"] is not False or contract["executionAuthorized"] is not False
            or contract["acceptedCertificate"] is not False or contract["writableDelta"] != []):
        raise ValueError("preparation cannot authorize activation or writes")
    if contract["arms"] != sample["arms"] or contract["phases"] != sample["phases"]:
        raise ValueError("phase or arm membership differs")
    limits = contract["proposedLimits"]
    attempts = sum(job["allowedAssistanceRoleConditional"] is not None for job in sample["jobs"]) * 2
    if (limits["modelWireAttempts"] != attempts or limits["httpRetries"] != 0
            or limits["schemaRepairs"] != 0 or limits["concurrency"] != 1
            or limits["reservedOutputTokens"] != attempts * contract["generation"]["maxTokens"]
            or limits["aggregateInputTokens"] != attempts * limits["maxInputTokensPerAttempt"]):
        raise ValueError("proposed request/token contract inconsistent")
    for key in ["identity", "trustRoot", "approvedManifestSha256", "issuedAt", "expiresAt"]:
        if contract["approval"][key] is not None:
            raise ValueError("preparation cannot manufacture approval")
    if contract["allowedRoles"] != ["GenreFillResponse"]:
        raise ValueError("unreviewed model role")
    cost = contract["costIllustration"]
    estimate = limits["aggregateInputTokens"] * cost["assumedMaxInputUsdPerMillion"]
    estimate += limits["reservedOutputTokens"] * cost["assumedMaxOutputUsdPerMillion"]
    if estimate != cost["twoAttemptTokenOnlyMicrousd"] or cost["approvedAllowance"] is not None:
        raise ValueError("cost illustration is inconsistent or approved")


if __name__ == "__main__":
    if sys.argv[1:] == ["--derive"]:
        print(json.dumps(derive(), ensure_ascii=False, indent=2))
    elif not sys.argv[1:]:
        validate(json.loads((HERE / "sample.json").read_text()), json.loads((HERE / "contract.json").read_text()))
        print("Packet consistency passed; no tagging, model, media or approval validation performed")
    else:
        raise SystemExit("usage: packet.py [--derive]")
