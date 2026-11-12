"""Read-only controls for the durable mock commit packet."""
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
PREFIX = str(HERE.relative_to(ROOT)) + '/'
EVIDENCE = '.planning/design/live-run-contract-20261111/durable-mock-evidence-20261112/'
spec = importlib.util.spec_from_file_location('delivery', HERE / 'verify.py')
verifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verifier)


class DeliveryTests(unittest.TestCase):
    def read(self, path):
        return (ROOT / path).read_bytes()

    def altered(self, path, raw, rebind=False):
        seal = PREFIX + 'delivery-SHA256SUMS'
        manifest = self.read(seal)
        if rebind:
            manifest = manifest.replace(hashlib.sha256(self.read(path)).hexdigest().encode()
                + b'  ' + path.encode(), hashlib.sha256(raw).hexdigest().encode() + b'  ' + path.encode())
        return lambda name: raw if name == path else manifest if name == seal else self.read(name)

    def test_complete_packet_preserves_216_slots(self):
        self.assertEqual(verifier.verify(self.read)['requestedFieldSlots'], 216)

    def test_committed_reader_does_not_read_untracked_binary(self):
        def committed_read(path):
            if path.startswith(('src-tauri/target/', '.amp/in/')):
                raise ValueError('untracked binary requested from Git')
            return self.read(path)
        self.assertEqual(verifier.verify(committed_read)['wireArrivals'], 497)

    def test_changed_runtime_bytes_reject(self):
        path = 'src-tauri/src/infra/mock_run_authority.rs'
        with self.assertRaisesRegex(ValueError, 'hash mismatch'):
            verifier.verify(self.altered(path, self.read(path) + b'\n'))

    def test_rebound_missing_member_rejects_immutable_evidence_change(self):
        path = EVIDENCE + 'requested-membership.json'
        value = json.loads(self.read(path))
        value['evaluations'][0]['physicalPaths'].pop()
        with self.assertRaisesRegex(ValueError, 'evidence seal'):
            verifier.verify(self.altered(path, json.dumps(value).encode(), rebind=True))

    def test_rebound_build_cannot_claim_another_executable(self):
        path = PREFIX + 'build-receipt.json'
        value = json.loads(self.read(path))
        value['executable']['sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'executable binding'):
            verifier.verify(self.altered(path, json.dumps(value).encode(), rebind=True))


if __name__ == '__main__':
    unittest.main()
