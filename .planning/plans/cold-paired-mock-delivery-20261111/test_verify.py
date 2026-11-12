"""Metadata-only controls for the delivery packet verifier."""
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest

DIRECTORY = Path(__file__).resolve().parent
ROOT = DIRECTORY.parents[2]
MODULE = importlib.util.spec_from_file_location('delivery_verify', DIRECTORY / 'verify.py')
verifier = importlib.util.module_from_spec(MODULE)
MODULE.loader.exec_module(verifier)


class DeliveryVerification(unittest.TestCase):
    def read(self, path):
        return (ROOT / path).read_bytes()

    def changed(self, path, raw, rebind=False):
        manifest_path = str((DIRECTORY / 'delivery-SHA256SUMS').relative_to(ROOT))
        manifest = self.read(manifest_path)
        if rebind:
            old = hashlib.sha256(self.read(path)).hexdigest().encode()
            manifest = manifest.replace(old + b'  ' + path.encode(),
                hashlib.sha256(raw).hexdigest().encode() + b'  ' + path.encode())
        return lambda name: raw if name == path else manifest if name == manifest_path else self.read(name)

    def test_current_packet_passes(self):
        self.assertEqual(verifier.verify(self.read)['requestedFieldSlots'], 216)

    def test_changed_runtime_source_is_rejected(self):
        path = 'scripts/auto-tag-eval-replay/score-observations.py'
        with self.assertRaisesRegex(ValueError, 'hash mismatch'):
            verifier.verify(self.changed(path, self.read(path) + b'\n# changed\n'))

    def test_rebound_incomplete_membership_is_rejected(self):
        path = str((DIRECTORY / 'requested-membership.json').relative_to(ROOT))
        data = json.loads(self.read(path))
        data['evaluations'][0]['physicalPaths'].pop()
        with self.assertRaisesRegex(ValueError, 'membership'):
            verifier.verify(self.changed(path, json.dumps(data).encode(), rebind=True))

    def test_rebound_expanded_inventory_cap_is_rejected(self):
        path = str((DIRECTORY / 'selection.json').relative_to(ROOT))
        data = json.loads(self.read(path))
        data['orchestration']['inventoryBytes'] += 1
        with self.assertRaisesRegex(ValueError, 'inventory cap'):
            verifier.verify(self.changed(path, json.dumps(data).encode(), rebind=True))


if __name__ == '__main__':
    unittest.main()
