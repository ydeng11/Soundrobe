"""Preparation checks only: no application, network, media, or model execution."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("packet", HERE / "packet.py")
packet = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packet)


class PacketTests(unittest.TestCase):
    def setUp(self):
        self.sample = json.loads((HERE / "sample.json").read_text())
        self.contract = json.loads((HERE / "contract.json").read_text())

    def reject(self, sample=None, contract=None):
        with self.assertRaises(ValueError):
            packet.validate(sample or self.sample, contract or self.contract)

    def test_frozen_packet(self):
        packet.validate(self.sample, self.contract)

    def test_missing_file_cannot_shrink_denominator(self):
        self.sample["jobs"][0]["files"].pop()
        self.reject()

    def test_duplicate_occurrence_is_not_new_evidence(self):
        self.sample["jobs"][0]["files"][1] = copy.deepcopy(self.sample["jobs"][0]["files"][0])
        self.reject()

    def test_scripted_genre_cannot_become_truth(self):
        self.sample["jobs"][0]["files"][0]["acceptedValues"]["genre"] = ["Pop"]
        self.reject()

    def test_acoustic_qualifier_cannot_be_dropped(self):
        self.sample["jobs"][0]["files"][-1]["profileVisible"]["title"] = "英雄 Heroes"
        self.reject()

    def test_capture_hash_cannot_be_rebound(self):
        self.sample["jobs"][0]["files"][0]["capturedMediaSha256Claim"] = "0" * 64
        self.reject()

    def test_live_mode_cannot_be_inferred(self):
        self.contract["activationAvailable"] = True
        self.reject()

    def test_more_attempts_need_a_new_contract(self):
        self.contract["proposedLimits"]["modelWireAttempts"] = 3
        self.reject()


if __name__ == "__main__":
    unittest.main()
