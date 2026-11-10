import copy
import importlib.util
import pathlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('comparison_v2', HERE / 'compare-v2.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class FullMatrixComparisonContract(unittest.TestCase):
    def setUp(self):
        relative = pathlib.Path('folder_filename/4713ee42bece/cold.jsonl')
        self.before = audit.base.load(audit.base.PRIOR / relative)
        frozen = HERE.parent / 'reviewed-51-preservation-20261110'
        self.after = audit.base.load(frozen / relative)
        self.clock = next(row for row in audit.base.differences(audit.base.stable(self.before), audit.base.stable(self.after))
                          if row['path'].startswith('/native/') and row['path'].endswith('/observation/source/observedAt'))

    def test_unchanged_preexisting_pcm_representation_does_not_disable_source_clock_check(self):
        self.assertNotEqual(self.before['native']['diagnostics'], self.before['persistedEvidence']['diagnostics'])
        self.assertTrue(audit.runtime(self.clock, self.before, self.after))

    def test_changed_persisted_pcm_is_still_an_unexplained_difference(self):
        self.after['persistedEvidence']['diagnostics'][1]['diagnostics']['observations'][1]['observation']['pcm']['seconds'] += 0.5
        changes = audit.compare_record(self.before, self.after)
        self.assertTrue(any(row['classification'] == 'unexplained' and row['path'].endswith('/pcm/seconds') for row in changes))

    def test_mismatching_persisted_source_cannot_exempt_its_clock(self):
        source_path = self.clock['path'].removesuffix('/observedAt').replace('/native/', '/persistedEvidence/', 1)
        audit.base.value(self.after, source_path)['expectedSha256'] = '0' * 64
        self.assertFalse(audit.runtime(self.clock, self.before, self.after))

    def test_changed_pcm_type_is_visible(self):
        self.after['persistedEvidence']['diagnostics'][1]['diagnostics']['observations'][1]['observation']['pcm']['seconds'] = '211.04'
        self.assertTrue(any(row['classification'] == 'unexplained' and row['path'].endswith('/pcm/seconds')
                            for row in audit.compare_record(self.before, self.after)))


if __name__ == '__main__':
    unittest.main()
