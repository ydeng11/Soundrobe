import importlib.util
import pathlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('receipt_audit', HERE / 'receipt-audit.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class ReceiptContract(unittest.TestCase):
    def record(self, case):
        # Keep the demonstrated historical failure as a fixed negative input.
        frozen = HERE.parent / 'reviewed-51-preservation-20261110'
        return audit.load(frozen / 'assisted_without_ids' / case / 'cold.jsonl')

    def test_later_replacement_violates_nested_preservation_receipt(self):
        findings = audit.audit_record(self.record('4060f1de6662'))
        self.assertEqual([f['category'] for f in findings], ['receipt_prewrite_not_preserved'])
        self.assertEqual(findings[0]['fields'], ['artist', 'artists'])

    def test_valid_nested_preservation_remains_valid(self):
        self.assertEqual(audit.audit_record(self.record('8452b6f2b3ea')), [])

    def test_missing_persisted_receipt_is_detected(self):
        record = self.record('8452b6f2b3ea')
        record['persistedEvidence']['diagnostics'][2]['fieldEvidence'] = []
        self.assertEqual([f['category'] for f in audit.audit_record(record)], ['receipt_not_persisted'])


if __name__ == '__main__':
    unittest.main()
