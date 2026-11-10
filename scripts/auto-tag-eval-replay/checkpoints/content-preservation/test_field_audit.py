import copy
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location('fields', pathlib.Path(__file__).with_name('field-audit.py'))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class FieldContracts(unittest.TestCase):
    def setUp(self):
        self.reference = dict(artist='Lead', album='Album', aliases=[], mapping=[
            dict(providerTitle='First', localTitle='First'),
            dict(providerTitle='Second (Live)', localTitle='Second (Live)')], durations=[101, 222])
        self.source = dict(id=17, title='Album', artists=[dict(name='Lead')], genres=['Pop'], styles=['Acoustic'], tracklist=[
            dict(title='First', duration='1:41', artists=[dict(name='Lead')]),
            dict(title='Second (Live)', duration='3:42', artists=[dict(name='Lead'), dict(name='Guest')])])
        self.receipt = dict(stage='provider_field', field='genre', provider='discogs', releaseId='17', value='Pop, Acoustic')
        self.record = dict(assistanceMode='disabled', outcome='applied', payloadUnchanged=True, readback=True,
                           beforeTracks=[dict(path='/tmp/01.flac', genre='User acoustic'), dict(path='/tmp/02.flac', genre=None)],
                           readbackTracks=[dict(path='/tmp/01.flac', genre='User acoustic', discogsReleaseId='17'),
                                           dict(path='/tmp/02.flac', genre='Pop, Acoustic', discogsReleaseId='17')],
                           profileBaseline={'01.flac': {'fullSha256': 'before'}}, after={'01.flac': {'fullSha256': 'after'}},
                           native=dict(written=2, outcome='applied', candidate=dict(source='discogs', genre='Pop, Acoustic', discogs_release_id='17'),
                                       diagnostics=[self.receipt]), reviewPendingAfterRestart=False,
                           persistedEvidence=dict(candidate=dict(source='discogs', genre='Pop, Acoustic', discogs_release_id='17'), diagnostics=[self.receipt]))

    def test_source_certificate_does_not_accept_similar_wrong_releases(self):
        self.assertTrue(audit.source_supported(self.reference, 'discogs', self.source))
        for field, value in [('title', 'Unrelated Album'), ('artists', [dict(name='Other')]),
                             ('tracklist', [dict(title='First', duration='1:41'), dict(title='Second (Remix)', duration='3:42')]),
                             ('tracklist', [dict(title='First', duration='1:41'), dict(title='Second (Live)', duration='4:20')]),
                             ('tracklist', [dict(title='First', duration='1:41'), dict(title='Second (Live)', duration='')])]:
            wrong = copy.deepcopy(self.source)
            wrong[field] = value
            self.assertFalse(audit.source_supported(self.reference, 'discogs', wrong), (field, value))

    def test_provider_fill_and_local_choice_have_distinct_authority(self):
        self.assertEqual(audit.audit_record(self.record, {('discogs', '17'): self.source})[0], [])
        wrong = copy.deepcopy(self.record)
        wrong['readbackTracks'][0]['genre'] = 'Pop, Acoustic'
        self.assertIn('local_genre', audit.audit_record(wrong, {('discogs', '17'): self.source})[0])
        wrong = copy.deepcopy(self.record)
        wrong['readbackTracks'][1]['genre'] = 'Rock'
        self.assertIn('provider_genre', audit.audit_record(wrong, {('discogs', '17'): self.source})[0])

    def test_receipts_and_review_survive_restart_without_inventing_identity(self):
        wrong = copy.deepcopy(self.record)
        wrong['persistedEvidence']['diagnostics'] = []
        self.assertIn('persisted_receipt', audit.audit_record(wrong, {('discogs', '17'): self.source})[0])
        wrong = copy.deepcopy(self.record)
        wrong['native']['candidate']['verification'] = 'album_content'
        wrong['persistedEvidence']['candidate']['verification'] = 'album_content'
        wrong['readbackTracks'][1]['discogsReleaseId'] = '17'
        codes = audit.audit_record(wrong, {('discogs', '17'): self.source})[0]
        self.assertIn('content_ids', codes)
        self.assertIn('review_state', codes)

    def test_a_hold_is_not_authorization_to_change_tags_or_bytes(self):
        held = copy.deepcopy(self.record)
        held['native']['written'] = 0
        codes = audit.audit_record(held, {})[0]
        self.assertIn('held_bytes', codes)
        self.assertIn('held_tags', codes)


if __name__ == '__main__':
    unittest.main()
