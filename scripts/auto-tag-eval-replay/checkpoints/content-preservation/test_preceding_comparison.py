"""Receipt comparison remains strict when both revisions already carry receipts."""
import copy
import importlib.util
import pathlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('comparison', HERE / 'compare-preceding.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)
PRIOR = HERE.parent / 'reviewed-51-preservation-20261110'


class ExistingReceiptContract(unittest.TestCase):
    def records(self):
        record = audit.base.load(PRIOR / 'assisted_without_ids/8452b6f2b3ea/cold.jsonl')
        return record, copy.deepcopy(record)

    def test_identical_existing_receipts_do_not_become_false_changes(self):
        before, after = self.records()
        changes = audit.compare_record(before, after)
        self.assertFalse(any(row['classification'] == 'unexplained' for row in changes))

    def test_later_overwrite_cannot_hide_behind_an_existing_receipt(self):
        before, after = self.records()
        receipt = next(r for d in after['native']['diagnostics'] for r in d.get('fieldEvidence', [])
                       if r.get('stage') == 'local_track_preservation')
        after['readbackTracks'][receipt['localIndex']]['artist'] = 'Wrong guest'
        changes = audit.compare_record(before, after)
        self.assertTrue(any(row['classification'] == 'unexplained' and '/readbackTracks/' in row['path'] for row in changes))

    def test_receipt_mutation_remains_visible(self):
        before, after = self.records()
        receipt = next(r for d in after['native']['diagnostics'] for r in d.get('fieldEvidence', [])
                       if r.get('stage') == 'local_track_preservation')
        receipt['filename'] = 'wrong file'
        self.assertTrue(any(row['classification'] == 'unexplained' and '/fieldEvidence' in row['path']
                            for row in audit.compare_record(before, after)))


if __name__ == '__main__':
    unittest.main()
