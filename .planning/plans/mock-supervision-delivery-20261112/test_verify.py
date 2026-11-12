"""Read-only controls for the supervision commit's receipt and byte checks."""
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
PREFIX = str(HERE.relative_to(ROOT)) + '/'
spec = importlib.util.spec_from_file_location('delivery', HERE / 'verify.py')
verifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verifier)


class DeliveryTests(unittest.TestCase):
    def read(self, path):
        if path.startswith('.amp/in/'):
            raise ValueError('untracked evidence requested by committed-only verification')
        return (ROOT / path).read_bytes()

    def altered(self, path, raw, rebind=False):
        seal = PREFIX + 'delivery-SHA256SUMS'
        manifest = self.read(seal)
        if rebind:
            manifest = manifest.replace(hashlib.sha256(self.read(path)).hexdigest().encode()
                + b'  ' + path.encode(), hashlib.sha256(raw).hexdigest().encode() + b'  ' + path.encode())
        return lambda name: raw if name == path else manifest if name == seal else self.read(name)

    def test_committed_only_reader_preserves_denominator(self):
        self.assertEqual(verifier.verify(self.read)['observedFieldSlots'], 216)

    def test_changed_runtime_bytes_reject(self):
        path = 'src-tauri/src/infra/mock_run_receiver.rs'
        with self.assertRaisesRegex(ValueError, 'hash mismatch'):
            verifier.verify(self.altered(path, self.read(path) + b'\n'))

    def test_rebound_staging_cannot_claim_completion(self):
        path = PREFIX + 'run-receiver-outcome.json'
        value = json.loads(self.read(path))
        value['state'] = 'provisional_complete'
        with self.assertRaisesRegex(ValueError, 'native completion'):
            verifier.verify(self.altered(path, json.dumps(value).encode(), rebind=True))

    def test_rebound_late_exit_cannot_claim_completion(self):
        path = PREFIX + 'run-receiver-outcome.json'
        value = json.loads(self.read(path))
        value['supervisorExitObservedAt'] = value['deadline'] + 1
        with self.assertRaisesRegex(ValueError, 'timely exit'):
            verifier.verify(self.altered(path, json.dumps(value).encode(), rebind=True))

    def test_rebound_parent_is_rejected(self):
        path = PREFIX + 'source-bindings.json'
        value = json.loads(self.read(path))
        value['parent'] = '0' * 40
        with self.assertRaisesRegex(ValueError, 'delivery base'):
            verifier.verify(self.altered(path, json.dumps(value).encode(), rebind=True))


if __name__ == '__main__':
    unittest.main()
