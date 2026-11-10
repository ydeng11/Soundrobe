"""Run with python3 -B scripts/auto-tag-eval-replay/test_safety_audit.py."""
import copy
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location('audit', pathlib.Path(__file__).with_name('audit-safety.py'))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class FieldAuditTests(unittest.TestCase):
    def setUp(self):
        self.job = dict(name='mixed-accepted', caseId='60ebec7ffadb', scenario='accepted', genres={'01.flac': 'User acoustic'})
        self.physical = [dict(relativePath='01.flac'), dict(relativePath='02.flac')]
        self.evidence = dict(titles=['First', 'Second (Live)'], artists=[['Lead', 'Guest'], ['Lead']])
        before = [dict(path='/tmp/01.flac', title='First', artists=['Lead', 'Guest'], genre='User acoustic'),
                  dict(path='/tmp/02.flac', title='Second (Live)', artists=['Lead'], genre=None)]
        after = copy.deepcopy(before)
        after[1]['genre'] = 'Pop'
        provenance = dict(model='scripted/genre-v1', value='Pop')
        self.record = dict(native=dict(written=2, authority='musicbrainz', outcome='needs_review', candidate=dict(assisted_genre=provenance)),
                           beforeTracks=before, readbackTracks=after, payloadUnchanged=True, readback=True,
                           providerMode='offline_fixtures', phase='cold', profile='folder_filename', cacheBefore={'cache': 0},
                           reviewPendingAfterRestart=True, persistedEvidence=dict(candidate=dict(assisted_genre=provenance)))

    def findings(self, record):
        return audit.audit_record(record, self.job, self.evidence, self.physical)[0]

    def test_retained_and_inferred_fields_have_different_review_obligations(self):
        failures, ledger = audit.audit_record(self.record, self.job, self.evidence, self.physical)
        self.assertEqual(failures, [])
        self.assertEqual([field['needsReview'] for field in ledger if field['field'] == 'genre'], [False, True])

    def test_genre_absence_controls_cannot_change_identity_or_credits(self):
        original = dict(id=17, title='Album (Live)', genres=['Rock'], styles=['Acoustic'],
                        results=[dict(title='Song', artist=['Lead', 'Guest'], genre=['Pop'], length=123.0)])
        controlled = dict(id=17, title='Album (Live)',
                          results=[dict(title='Song', artist=['Lead', 'Guest'], length=123)])
        self.assertTrue(audit.genre_absence_matches(original, controlled))
        for field, value in [('title', 'Other'), ('artist', ['Lead']), ('length', 123.000001), ('genre', ['Pop'])]:
            wrong = copy.deepcopy(controlled)
            wrong['results'][0][field] = value
            self.assertFalse(audit.genre_absence_matches(original, wrong), field)
        self.assertFalse(audit.genre_absence_matches(dict(id=1), dict(id=True)))
        del controlled['title']
        self.assertFalse(audit.genre_absence_matches(original, controlled))

    def test_provider_first_contract_rejects_ai_substitution_and_missing_receipts(self):
        job = dict(self.job, name='collaborator-disagreement', providerGenre='Electronic, Pop')
        record = copy.deepcopy(self.record)
        record['native'].update(outcome='applied', candidate=dict(verification='album_content'), diagnostics=[
            dict(stage='provider_field', field='genre', provider='discogs', releaseId='reviewed', value='Electronic, Pop')])
        record['persistedEvidence']['diagnostics'] = copy.deepcopy(record['native']['diagnostics'])
        record['native']['diagnostics'].append(dict(stage='pcm', seconds=179.01600907029479))
        record['persistedEvidence']['diagnostics'].append(dict(stage='pcm', seconds=179.0160090702948))
        record['readbackTracks'][1]['genre'] = 'Electronic, Pop'
        failures, ledger = audit.audit_record(record, job, self.evidence, self.physical)
        self.assertEqual(failures, [])
        self.assertEqual(next(row for row in ledger if row['file'] == '02.flac' and row['field'] == 'genre')['provenance'],
                         'discogs_proposal')
        failures, _ = audit.audit_record(self.record, job, self.evidence, self.physical)
        self.assertIn('field_authorization', [row['category'] for row in failures])
        record['persistedEvidence']['diagnostics'] = []
        failures, _ = audit.audit_record(record, job, self.evidence, self.physical)
        self.assertIn('review_highlight', [row['category'] for row in failures])
        record['persistedEvidence']['diagnostics'] = copy.deepcopy(record['native']['diagnostics'])
        record['persistedEvidence']['diagnostics'][0]['releaseId'] = 'changed'
        failures, _ = audit.audit_record(record, job, self.evidence, self.physical)
        self.assertIn('review_highlight', [row['category'] for row in failures])
        record['native']['diagnostics'] = []
        failures, _ = audit.audit_record(record, job, self.evidence, self.physical)
        self.assertIn('source_grounding', [row['category'] for row in failures])

    def test_field_donor_is_not_mislabeled_as_selected_edition(self):
        record = copy.deepcopy(self.record)
        record['native'].update(authority='discogs', outcome='applied', candidate={}, diagnostics=[
            dict(stage='provider_track_credits', localIndex=0, provider='musicbrainz', releaseId='full',
                 artists=['Lead', 'Guest']),
            dict(stage='album_content', fieldEvidence=[dict(stage='provider_field', field='genre',
                 provider='discogs', releaseId='genre-source', value='Pop')])])
        record['persistedEvidence']['diagnostics'] = copy.deepcopy(record['native']['diagnostics'])
        record['beforeTracks'][0]['artists'] = ['Lead']
        record['reviewPendingAfterRestart'] = False
        job = dict(self.job, name='provider-fields', caseId='other')
        failures, ledger = audit.audit_record(record, job, self.evidence, self.physical)
        self.assertEqual(failures, [])
        credit = next(row for row in ledger if row['file'] == '01.flac' and row['field'] == 'artists')
        self.assertEqual(credit['provenance'], 'musicbrainz_proposal')
        self.assertEqual(credit['sourceReceipt']['releaseId'], 'full')
        genre = next(row for row in ledger if row['file'] == '02.flac' and row['field'] == 'genre')
        self.assertEqual(genre['sourceReceipt']['releaseId'], 'genre-source')
        record['native']['diagnostics'][0]['artists'] = ['Lead', 'Wrong guest']
        failures, _ = audit.audit_record(record, job, self.evidence, self.physical)
        self.assertIn('source_grounding', [row['category'] for row in failures])

    def test_wrong_writes_cannot_hide_behind_native_success(self):
        for field, value, category in [('genre', 'Pop', 'field_authorization'),
                                       ('artists', ['Lead'], 'collaborator_credit')]:
            record = copy.deepcopy(self.record)
            record['readbackTracks'][0][field] = value
            self.assertIn(category, [item['category'] for item in self.findings(record)])
        record = copy.deepcopy(self.record)
        record['readbackTracks'][1]['title'] = 'Second'
        self.assertIn('identity_version', [item['category'] for item in self.findings(record)])

    def test_missing_persisted_provenance_and_payload_change_fail(self):
        for field, value, category in [('persistedEvidence', {}, 'review_highlight'),
                                       ('payloadUnchanged', False, 'write_readback')]:
            record = copy.deepcopy(self.record)
            record[field] = value
            self.assertIn(category, [item['category'] for item in self.findings(record)])

    def test_cancelled_inputs_still_get_a_per_field_audit(self):
        record = copy.deepcopy(self.record)
        record.update(native=None, cancellationRequested=True, reviewPendingAfterRestart=False,
                      error='Operation cancelled', readbackTracks=record['beforeTracks'], profileBaseline={'sha': 'same'}, after={'sha': 'same'})
        job = dict(self.job, noWrite=True, scenario='cancelled')
        failures, ledger = audit.audit_record(record, job, self.evidence, self.physical)
        self.assertEqual(failures, [])
        self.assertTrue(ledger, 'held/cancelled files must remain auditable, not disappear from the denominator')
        self.assertTrue(all(field['written'] is None for field in ledger))
        record['after'] = {'sha': 'changed'}
        failures, _ = audit.audit_record(record, job, self.evidence, self.physical)
        self.assertIn('field_authorization', [item['category'] for item in failures])

    def test_held_worker_state_does_not_require_a_duplicate_review_flag(self):
        record = copy.deepcopy(self.record)
        record.update(native=dict(outcome='needs_review', written=0), reviewPendingAfterRestart=False,
                      actionableReviewAfterRestart=True, readbackTracks=record['beforeTracks'],
                      profileBaseline={'sha': 'same'}, after={'sha': 'same'})
        failures, _ = audit.audit_record(record, dict(self.job, noWrite=True), self.evidence, self.physical)
        self.assertEqual(failures, [])


if __name__ == '__main__':
    unittest.main()
