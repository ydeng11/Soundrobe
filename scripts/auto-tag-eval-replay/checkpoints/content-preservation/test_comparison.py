import copy
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location('comparison', pathlib.Path(__file__).with_name('compare.py'))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class ComparisonContract(unittest.TestCase):
    def test_preservation_requires_captured_types_and_non_stripped_profile(self):
        self.assertTrue(audit.preserved(dict(trackNumber=1), 'trackNumber', 1, 'tagged_recovery'))
        for value in [True, 1.0, '1']:
            self.assertFalse(audit.preserved(dict(trackNumber=1), 'trackNumber', value, 'tagged_recovery'))
        self.assertFalse(audit.preserved(dict(trackNumber=22), 'trackNumber', 22, 'folder_filename'))
        self.assertFalse(audit.preserved(dict(trackNumber=22), 'trackNumber', 21, 'tagged_recovery'))

    def test_key_and_type_changes_remain_visible(self):
        self.assertTrue(audit.differences({}, dict(x=None)))
        self.assertTrue(audit.differences(dict(x=1), dict(x=True)))
        self.assertTrue(audit.differences(dict(x=[]), {}))

    def test_unrelated_output_changes_are_not_preservation(self):
        track = dict(path='/synthetic/01.flac', artist='Lead', artists=['Lead'], trackNumber=1,
                     trackTotal=1, discNumber=1, discTotal=None)
        record = dict(beforeTracks=[copy.deepcopy(track)], readbackTracks=[copy.deepcopy(track)], profile='tagged_recovery',
                      native=dict(diagnostics=[], candidate=dict(tracks=[]), written=1),
                      persistedEvidence=dict(diagnostics=[]), outcome='applied')
        for field, value in [('genre', 'Unrelated'), ('musicbrainzAlbumId', 'invented'), ('trackNumber', 22)]:
            after = copy.deepcopy(record)
            after['readbackTracks'][0][field] = value
            self.assertTrue(any(r['classification'] == 'unexplained' for r in audit.compare_record(record, after)))


if __name__ == '__main__':
    unittest.main()
