"""Reproduce this metadata-only label packet; never open audio or execute tagging."""
import hashlib
import json
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
OLD = ROOT / ".planning/plans/crowd-lu-annotation-sheet-20261111"
SAMPLE = ROOT / ".planning/plans/scorable-assistance-contract-20261111/sample.json"
VALUES = ["Pop", "Mandopop", "Pop, Mandopop"]
ARTIFACT = "https://ampcode.com/artifacts/crowd-lu-genre-annotations-8054842d-554b-48e3-94e1-766e1a36dbe2"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def derive():
    prior = json.loads((OLD / "row-bindings.json").read_text())
    sources = [ROOT / s["path"] for s in prior["sources"]]
    for path, binding in zip(sources, prior["sources"]):
        if digest(path) != binding["sha256"]:
            raise ValueError("historical source binding changed")
    if digest(OLD / "sheet.md") != prior["sheetSha256"]:
        raise ValueError("initial sheet changed")
    inventory = json.loads(sources[0].read_text())["cases"][0]
    provider = json.loads(sources[1].read_text())["media"][0]["tracks"]
    sample = json.loads(SAMPLE.read_text())["jobs"][0]
    snapshot = HERE / "annotations-rev63.md"
    text = snapshot.read_text()
    rule = text.split("## Confirmed acceptance rule\n", 1)[1].split("## How to fill it", 1)[0]
    if re.findall(r"^- `([^`]+)`$", rule, re.M) != VALUES:
        raise ValueError("confirmed exact alternatives differ")
    if '**Yes / No / Cannot judge:** *Yes*' not in text or '`"Pop" OR "Mandopop" OR "Pop, Mandopop"`' not in text:
        raise ValueError("album-level confirmation differs")
    lines = [line for line in text.splitlines() if re.match(r"^\| \d{2} · ", line)]
    if not (len(lines) == len(prior["rows"]) == len(inventory["tracks"])
            == len(sample["files"]) == len(provider) == 11):
        raise ValueError("eleven-row membership required")
    if len({t["relativePath"] for t in inventory["tracks"]}) != 11:
        raise ValueError("duplicate physical path")
    if inventory["caseId"] != sample["caseId"] or inventory["caseId"] != "036e0a00e45b":
        raise ValueError("case differs")
    if inventory["sourceRelativeFolder"] != sample["folder"] or sample["folder"] != prior["sourceRelativeFolder"]:
        raise ValueError("folder differs")
    rows = []
    for i, (bound, track, selected, catalog, line) in enumerate(zip(
            prior["rows"], inventory["tracks"], sample["files"], provider, lines)):
        if (bound["row"] != i + 1 or bound["inventorySelector"] != f"/cases/0/tracks/{i}"
                or bound["sampleSelector"] != f"/jobs/0/files/{i}"
                or bound["providerSelector"] != f"/media/0/tracks/{i}"
                or selected["inventorySelector"] != bound["inventorySelector"]
                or bound["relativePath"] != track["relativePath"] or bound["relativePath"] != selected["path"]
                or bound["title"] != track["title"] or bound["title"] != catalog["title"]
                or bound["title"] != selected["profileVisible"]["title"]
                or bound["capturedDurationSeconds"] != track["duration"]
                or bound["providerLengthMilliseconds"] != catalog["length"]
                or bound["capturedMediaSha256Claim"] != track["sourceSha256"]
                or bound["capturedMediaSha256Claim"] != selected["capturedMediaSha256Claim"]
                or bound["recordingMbidClaim"] != catalog["recording"]["id"]):
            raise ValueError("occurrence/source identity differs")
        title = track["title"].replace("[", r"\[").replace("]", r"\]")
        prefix = f'| {i+1:02d} · {title} · `{track["relativePath"]}` | {track["duration"]:.3f} s | '
        if not line.startswith(prefix):
            raise ValueError("annotation row/title/duration differs")
        cells = line[len(prefix):].split(" | ")
        rows.append({**bound, "acceptedGenreValues": VALUES.copy(),
                     "originalAnnotation": {"answer": cells[0], "statedBasis": cells[1],
                                            "notes": cells[2].removesuffix(" |")}})
    sources += [OLD / "row-bindings.json", OLD / "sheet.md", snapshot,
                ROOT / "src-tauri/src/commands/auto_tag.rs",
                ROOT / "scripts/auto-tag-eval-replay/score-observations.py"]
    return {
        "version": "crowd-lu-user-acceptance-v1", "labelBasis": "user_acceptance",
        "sourceArtifact": {"url": ARTIFACT, "revision": 63,
                           "snapshot": "annotations-rev63.md", "representation": "artifact read Markdown plus final newline"},
        "sourceHead": prior["sourceHead"], "caseId": inventory["caseId"],
        "profile": sample["profile"], "sourceRelativeFolder": sample["folder"],
        "field": "genre", "matching": "exact decoded string; no trimming, case folding, comma splitting, synonym or ordering normalization",
        "occurrences": rows,
        "albumLevel": {"userAnswer": "yes", "acceptedGenreValues": VALUES.copy(),
                       "scope": "all eleven occurrences, including Intro and full Acoustic Ver. title"},
        "exposure": "exposed development sample; zero untouched holdouts; not acoustic truth",
        "reviewerDatePlaceholder": "preserved unfilled; not inferred",
        "vocabularyPlaceholder": "preserved unfilled; explicit confirmation alone supplies these three alternatives",
        "executionAuthorized": False, "activationAvailable": False, "writableDelta": [],
        "promptSeparation": {
            "purpose": "scoring-only sidecar; never model input or expected-answer examples",
            "currentGenrePayloadSource": "src-tauri/src/commands/auto_tag.rs::fill_genre_if_missing",
            "currentPayloadKeys": ["artist", "album", "tracks"],
            "evidence": "source inspected and hashed; no runtime inputs changed, request constructed or dispatched",
            "futureRequirement": "independently audit exact serialized request/build/inputs; no sheet, notes, labels, taxonomy hints or scoring references may flow into model inputs",
            "enforcementClaim": "none added; future request admission and live boundary remain blocked",
        },
        "bindings": {str(path.relative_to(ROOT)): digest(path) for path in sorted(set(sources))},
    }


def validate(packet):
    if packet != derive():
        raise ValueError("packet differs from exact confirmed labels or bound occurrence sources")


if __name__ == "__main__":
    if sys.argv[1:] == ["--derive"]:
        print(json.dumps(derive(), ensure_ascii=False, indent=2))
    elif not sys.argv[1:]:
        validate(json.loads((HERE / "labels.json").read_text()))
        print("Exact user-acceptance label packet reproduced; no experiment or execution authorized")
    else:
        raise SystemExit("usage: freeze.py [--derive]")
