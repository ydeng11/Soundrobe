import copy
import importlib.util
import pathlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('comparison_v3', HERE / 'compare-v3.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class NestedReceiptContract(unittest.TestCase):
    def records(self, case):
        relative = pathlib.Path('assisted_without_ids') / case / 'cold.jsonl'
        frozen = HERE.parent / 'reviewed-51-preservation-20261110'
        audit.base.HERE = frozen  # The frozen record's exact run ID is part of its contract.
        return audit.base.load(audit.base.PRIOR / relative), audit.base.load(frozen / relative)

    def test_exact_prewrite_bound_nested_receipt_is_classified(self):
        before, after = self.records('8452b6f2b3ea')
        changes = audit.compare_record(before, after)
        self.assertTrue(any(r['classification'] == 'prewrite_bound_preservation_receipt'
                            and '/fieldEvidence/' in r['path'] for r in changes))
        self.assertFalse(any(r['classification'] == 'unexplained' for r in changes))

    def test_later_album_artist_replacement_is_not_exempted(self):
        before, after = self.records('4060f1de6662')
        changes = audit.compare_record(before, after)
        self.assertTrue(any(r['classification'] == 'unexplained' and r['path'] == '/readbackTracks/0/artists' for r in changes))
        self.assertFalse(any(r['classification'] == 'prewrite_bound_preservation_receipt' for r in changes))

    def test_wrong_receipt_filename_stays_unexplained(self):
        before, after = self.records('8452b6f2b3ea')
        after['native']['diagnostics'][2]['fieldEvidence'][0]['filename'] = 'wrong file'
        self.assertTrue(any(r['classification'] == 'unexplained' and '/fieldEvidence' in r['path']
                            for r in audit.compare_record(before, after)))


if __name__ == '__main__':
    unittest.main()
