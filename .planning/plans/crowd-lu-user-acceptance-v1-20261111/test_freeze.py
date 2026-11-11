"""Offline packet checks and synthetic value probes; no tagging or model execution."""
import copy
import importlib.util
import json
from pathlib import Path
import re
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


class FreezeTests(unittest.TestCase):
    def setUp(self):
        self.freeze = module("freeze", HERE / "freeze.py")
        self.packet = json.loads((HERE / "labels.json").read_text())

    def test_reproducible_packet(self):
        self.freeze.validate(self.packet)

    def test_all_three_exact_values_per_row_and_album(self):
        expected = ["Pop", "Mandopop", "Pop, Mandopop"]
        self.assertEqual(len(self.packet["occurrences"]), 11)
        for row in self.packet["occurrences"]:
            self.assertEqual(row["acceptedGenreValues"], expected)
        self.assertEqual(self.packet["albumLevel"]["acceptedGenreValues"], expected)

    def test_tampered_membership_or_labels_rejected(self):
        mutations = [
            lambda p: p["occurrences"].pop(),
            lambda p: p["occurrences"].append(copy.deepcopy(p["occurrences"][0])),
            lambda p: p["occurrences"].reverse(),
            lambda p: p["occurrences"][0].update(relativePath="other.flac"),
            lambda p: p["occurrences"][0].update(capturedMediaSha256Claim="0" * 64),
            lambda p: p["occurrences"][-1].update(title="英雄 Heroes"),
            lambda p: p["occurrences"][0]["acceptedGenreValues"].append("pop"),
            lambda p: p["albumLevel"]["acceptedGenreValues"].pop(),
            lambda p: p.update(labelBasis="acoustic_truth"),
            lambda p: p.update(executionAuthorized=True),
        ]
        for mutate in mutations:
            with self.subTest(mutation=mutate):
                changed = copy.deepcopy(self.packet)
                mutate(changed)
                with self.assertRaises(ValueError):
                    self.freeze.validate(changed)

    def test_existing_scorer_exact_acceptance_and_negative_controls(self):
        scorer = module("scorer", ROOT / "scripts/auto-tag-eval-replay/score-observations.py")
        accepted = ["Pop", "Mandopop", "Pop, Mandopop"]
        rejected = [None, "", "pop", "mandopop", "Pop,Mandopop", "Mandopop, Pop",
                    " Pop", "Pop ", "Pop, Mandopop ", "Pop / Mandopop", "Pop; Mandopop",
                    "Pop + Mandopop", "Pop, Rock", "Rock", "Unknown", "Pop\n"]
        for value in accepted + rejected:
            slots = [{"id": row["inventorySelector"], "original": None,
                      "acceptedValues": row["acceptedGenreValues"]}
                     for row in self.packet["occurrences"]]
            observations = [{"id": slot["id"], "readbackAvailable": True,
                             "readback": value, "serviceFailure": False,
                             "decisionVisibility": "unavailable"} for slot in slots]
            result = scorer.score({"slots": slots}, observations)
            with self.subTest(value=value):
                self.assertEqual(result["requestedFieldSlots"], 11)
                self.assertTrue(all(r["catalogValueAgreement"] == (value in accepted)
                                    for r in result["fields"]))
                self.assertEqual(result["groundedAssistanceAccuracyCredit"], 0)
                self.assertFalse(result["acceptedCertificate"])
                self.assertEqual(result["writableDelta"], [])
        # These are fabricated readbacks for an equality probe, never experiment results.

    def test_source_prompt_has_no_expected_answer_inputs(self):
        source = (ROOT / "src-tauri/src/commands/auto_tag.rs").read_text()
        function = source.split("async fn fill_genre_if_missing(", 1)[1]
        payload = function.split("let payload = serde_json::json!({", 1)[1].split("});", 1)[0]
        self.assertEqual(re.findall(r'^\s*"([^"]+)":', payload, re.M),
                         ["artist", "album", "tracks"])
        messages = function.split("let messages = vec![", 1)[1].split("let schema =", 1)[0]
        for forbidden in ["acceptedGenreValues", "annotations-rev63", "user-acceptance",
                          "Mandopop", '"Pop"']:
            self.assertNotIn(forbidden, payload + messages)
        self.assertIn("ChatMessage::user(payload.to_string())", messages)
        self.assertFalse(self.packet["executionAuthorized"])
        self.assertFalse(self.packet["activationAvailable"])


if __name__ == "__main__":
    unittest.main()
