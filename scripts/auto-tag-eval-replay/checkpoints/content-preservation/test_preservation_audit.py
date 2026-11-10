import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location('preservation', pathlib.Path(__file__).with_name('preservation-audit.py'))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class PreservationContract(unittest.TestCase):
    def test_captured_positions_are_file_bound_not_filename_order(self):
        before = [dict(path='/tmp/21 My Darling.flac', trackNumber=22, trackTotal=23, discNumber=2, discTotal=3),
                  dict(path='/tmp/22 Careful.flac', trackNumber=21, trackTotal=23, discNumber=2, discTotal=3)]
        self.assertEqual(audit.numbering_findings(before, before), [])
        changed = [dict(before[0], trackNumber=21), dict(before[1], trackNumber=22)]
        self.assertEqual(len(audit.numbering_findings(before, changed)), 2)
        changed = [dict(before[0], trackTotal=2, discTotal=1), before[1]]
        self.assertEqual(len(audit.numbering_findings(before, changed)), 2)

    def test_missing_values_keep_fallback_without_importing_richer_tags(self):
        before = [dict(path='/tmp/21 My Darling.flac', trackNumber=None, trackTotal=None, discNumber=None, discTotal=None)]
        self.assertEqual(audit.numbering_findings(before, [dict(before[0], trackNumber=1, trackTotal=1, discNumber=1)]), [])
        self.assertTrue(audit.numbering_findings(before, [dict(before[0], trackNumber=22, trackTotal=23)]))

    def test_numeric_types_are_not_coerced(self):
        before = [dict(path='/tmp/01.flac', trackNumber=1, trackTotal=1, discNumber=1, discTotal=1)]
        for value in [True, 1.0, '1']:
            self.assertTrue(audit.numbering_findings(before, [dict(before[0], trackNumber=value)]), value)

    def test_same_and_subset_performers_keep_existing_presentation(self):
        before = dict(artist='GUEST feat. Lead', artists=['GUEST', 'Lead'])
        self.assertEqual(audit.credit_findings(before, before), [])
        self.assertIn('collaborator_presentation', audit.credit_findings(before, dict(artist='Lead & GUEST', artists=['Lead', 'GUEST'])))
        self.assertIn('collaborator_presentation', audit.credit_findings(before, dict(artist='Lead', artists=['Lead'])))

    def test_missing_and_fuller_credits_are_not_newly_certified_or_locked(self):
        self.assertEqual(audit.credit_findings(dict(artist=None, artists=[]), dict(artist='Lead', artists=['Lead'])), [])
        self.assertEqual(audit.credit_findings(dict(artist='Lead', artists=['Lead']), dict(artist='Lead & Guest', artists=['Lead', 'Guest'])), [])
        self.assertIn('incompatible_guests', audit.credit_findings(dict(artist='Lead & A', artists=['Lead', 'A']),
                                                               dict(artist='Lead & B', artists=['Lead', 'B'])))
        self.assertEqual(audit.credit_findings(dict(artist='Lead & ???', artists=['Lead', '???']), dict(artist='Lead', artists=['Lead'])), [])


if __name__ == '__main__':
    unittest.main()
