"""Export contract; metadata fixtures are not audio or independent local truth."""
import copy
import hashlib
import importlib.util
import json
import pathlib
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
spec = importlib.util.spec_from_file_location('exporter', HERE / 'export-evidence.py')
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)
FIXTURES = ROOT / 'test/fixtures/tauri'
DETAILS = FIXTURES / 'auto-tag-eval/provider-snapshots/discovery/details-2026-11-07'
M = DETAILS / 'musicbrainz-22be98f32a7f4174.json'


class ExportContract(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = pathlib.Path(self.temp.name)
        self.sources = []
        corpus = json.loads((FIXTURES / 'auto-tag-eval/corpus.json').read_text())
        case = next(c for c in corpus['cases'] if c['caseId'] == '8452b6f2b3ea')
        tracks, request, positions, hashes = [], [], [], {}
        for i, t in enumerate(case['tracks']):
            stem = pathlib.Path(t['filename']).stem
            path = '/synthetic-test/' + t['filename']
            tracks.append(dict(path=path, title=t['title'], artist=t['artist'], artists=[t['artist']],
                               album=t['album'], duration=t['duration'], trackNumber=t['trackNumber'],
                               discogsReleaseId=t['providerIds']['discogsReleaseId']))
            request.append(dict(title=t['title'], filename=stem, artist=t['artist'], artists=[t['artist']],
                                track_number=i+1, track_total=22, length=t['duration']))
            positions.append(dict(tagged=t['trackNumber'], filenameDerived=i+1,
                                  lookupPosition=i+1, fileCountDerivedTotal=22))
            hashes[t['filename']] = dict(fullSha256=t['sourceSha256'], payloadSha256=t['sourceSha256'])
        self.snapshot = dict(schemaVersion=1, purpose='non_authorizing_evidence_only',
            profile='tagged_recovery', caseId=case['caseId'], phase='cold',
            request=dict(path='/synthetic-test', artist_hint='Eminem', album_hint='Relapse (Deluxe)',
                         discogs_release_id=case['tracks'][0]['providerIds']['discogsReleaseId'], tracks=request),
            baselineTracks=tracks, baselineHashes=hashes, positions=positions, guards=[],
            attempt=dict(closed=True, searchComplete=False, closureReason='synthetic_test', requests=[],
                         misses=[], modelRequests=[]), writableDelta=[])
        self.add_source(M, 'musicbrainz')

    def add_source(self, file, provider, mutate=None):
        body = json.loads(file.read_text())
        if mutate:
            mutate(body)
            file = self.directory / f'synthetic-{len(self.sources)}.json'
            file.write_text(json.dumps(body))
        release = str(body['id'])
        receipt = dict(provider=provider, path=('/release/' if provider == 'musicbrainz' else '/releases/')+release,
                       query={}, status=200, bodyFile=str(file), sha256=hashlib.sha256(file.read_bytes()).hexdigest(),
                       source=dict(kind='synthetic_mutation' if mutate else 'genuine_frozen_capture'))
        self.sources.append(receipt)
        self.snapshot['attempt']['requests'].append(dict(provider=provider, path=receipt['path'], query=[], status=200))
        return receipt

    def export(self):
        manifest = self.directory / 'manifest.json'
        manifest.write_text(json.dumps(dict(schemaVersion=1, records=self.sources)))
        result = exporter.export_evidence(copy.deepcopy(self.snapshot), manifest)
        self.assertIn('purpose', result, 'the exporter must publish a non-authorizing observation')
        self.assertEqual(result['writableDelta'], [])
        self.assertFalse(result['acceptedCertificate'])
        self.assertEqual(result['purpose'], 'non_authorizing_evidence_only')
        return result

    def test_relapse_support_is_not_variant_or_write_authority(self):
        result = self.export()
        source = result['sources'][0]
        self.assertEqual(source['catalogSupport'], 'supported')
        self.assertEqual(source['decision'], 'insufficient')
        self.assertIn('qualifier_applicability_unresolved', source['reasons'])
        row = source['occurrences'][12]
        self.assertEqual(row['credits']['recordingNames'], ['Eminem', 'Dr. Dre'])
        self.assertEqual(row['qualifiers']['applicability'], 'unresolved')
        self.assertEqual(row['raw']['recording']['disambiguation'], 'explicit')
        self.assertEqual(row['durations']['releaseTrackSeconds'], 278)
        self.assertEqual(row['durations']['recordingSeconds'], 278.16)
        self.assertEqual(result['prewrite']['positions'][20]['tagged'], 22)
        self.assertEqual(result['prewrite']['positions'][20]['filenameDerived'], 21)
        self.assertEqual(result['identityClaims']['acoustic'], 'unverified')

    def test_genuine_negative_packages_do_not_gain_authority_from_a_guest(self):
        for file, provider, reason in [
            ('musicbrainz-0b5d8d713c375676.json', 'musicbrainz', 'unmapped_occurrence'),
            ('discogs-53635687a0526e2f.json', 'discogs', 'package_count_mismatch'),
            ('musicbrainz-be0597affae6eb4a.json', 'musicbrainz', 'package_count_mismatch'),
            ('discogs-2343588e2e1a8691.json', 'discogs', 'album_context_conflict')]:
            self.add_source(DETAILS / file, provider)
            source = self.export()['sources'][-1]
            self.assertEqual(source['decision'], 'rejected', file)
            self.assertIn(reason, source['reasons'])
            self.assertFalse(source['authorizationDonor'])
        relationship = self.export()['sources'][1]['occurrences'][17]['credits']
        self.assertEqual(relationship['agreement'], 'relationship_unresolved')
        self.assertEqual(set(relationship['trackNames']), set(relationship['recordingNames']))
        self.add_source(FIXTURES / 'auto-tag-eval/provider-snapshots/discovery/folder-identities-2026-11-07/discogs-0a91c13d9dab831c.json', 'discogs')
        self.assertEqual(self.export()['sources'][-1]['decision'], 'rejected')

    def test_selected_discogs_missing_principals_and_durations_are_not_inherited(self):
        self.add_source(FIXTURES / 'relapse-deluxe/release-36441795.json', 'discogs')
        source = self.export()['sources'][1]
        self.assertIsNone(source['occurrences'][17]['credits']['trackNames'])
        self.assertEqual(source['occurrences'][17]['credits']['directGuests'], ['50 Cent', 'Dr. Dre'])
        self.assertIn('missing_track_principals', source['occurrences'][17]['reasons'])
        self.assertIsNone(source['occurrences'][20]['durations']['releaseTrackSeconds'])
        self.assertIn('duration_evidence_insufficient', source['reasons'])

    def test_synthetic_qualifier_contradiction_and_non_nested_guests_hold(self):
        self.snapshot['request']['tracks'][12]['title'] += ' (Clean)'
        self.snapshot['baselineTracks'][12]['title'] += ' (Clean)'
        result = self.export()['sources'][0]
        self.assertEqual(result['decision'], 'rejected')
        self.assertIn('qualifier_contradiction', result['reasons'])
        self.snapshot['request']['tracks'][12]['title'] = "Old Time's Sake"
        self.snapshot['baselineTracks'][12]['title'] = "Old Time's Sake"
        self.snapshot['baselineTracks'][12]['artists'] = ['Eminem', '50 Cent']
        self.snapshot['request']['tracks'][12]['artists'] = ['Eminem', '50 Cent']
        result = self.export()['sources'][0]
        self.assertIn('non_nested_performer_assertions', result['reasons'])
        self.assertEqual(result['occurrences'][12]['credits']['baselineNames'], ['Eminem', '50 Cent'])

    def test_synthetic_missing_duplicate_and_boundary_duration_evidence(self):
        original = copy.deepcopy(self.snapshot)
        self.snapshot['request']['tracks'][0]['length'] = 0
        self.snapshot['baselineTracks'][0]['duration'] = 0
        self.assertIn('duration_evidence_insufficient', self.export()['sources'][0]['reasons'])
        self.snapshot = copy.deepcopy(original)
        self.snapshot['request']['tracks'][1]['title'] = 'Missing occurrence'
        self.snapshot['baselineTracks'][1]['title'] = 'Missing occurrence'
        self.assertIn('unmapped_occurrence', self.export()['sources'][0]['reasons'])
        self.snapshot = copy.deepcopy(original)
        for key, baseline_key in [('title', 'title'), ('length', 'duration')]:
            self.snapshot['request']['tracks'][1][key] = self.snapshot['request']['tracks'][0][key]
            self.snapshot['baselineTracks'][1][baseline_key] = self.snapshot['baselineTracks'][0][baseline_key]
        self.assertIn('duplicate_source_mapping', self.export()['sources'][0]['reasons'])
        for delta, expected in [(5, True), (5.001, False)]:
            self.assertEqual(exporter.duration_fact(100, 100+delta)['withinBound'], expected)

    def test_synthetic_incomplete_attempt_and_hash_or_identity_mismatch(self):
        self.snapshot['attempt']['pendingRelevant'] = ['release-detail-not-finished']
        self.assertIn('bounded_relevant_sources_incomplete', self.export()['reasons'])
        self.snapshot['attempt']['pendingRelevant'] = []
        self.sources[0]['sha256'] = '0'*64
        with self.assertRaisesRegex(ValueError, 'hash'):
            self.export()
        self.sources[0]['sha256'] = hashlib.sha256(M.read_bytes()).hexdigest()
        self.sources[0]['path'] = '/release/wrong-id'
        self.snapshot['attempt']['requests'][0]['path'] = '/release/wrong-id'
        with self.assertRaisesRegex(ValueError, 'identity'):
            self.export()

    def test_synthetic_profile_leakage_and_unreceived_sources(self):
        self.snapshot['profile'] = 'folder_filename'
        with self.assertRaisesRegex(ValueError, 'profile'):
            self.export()
        self.snapshot['profile'] = 'assisted_without_ids'
        with self.assertRaisesRegex(ValueError, 'profile'):
            self.export()
        self.snapshot['profile'] = 'tagged_recovery'
        self.snapshot['attempt']['requests'] = []
        self.assertEqual(self.export()['sources'], [])

    def test_synthetic_mismatched_prewrite_and_guard_receipts_are_rejected(self):
        self.snapshot['request']['tracks'][0]['length'] += 1
        with self.assertRaisesRegex(ValueError, 'prewrite'):
            self.export()
        self.snapshot['request']['tracks'][0]['length'] -= 1
        self.snapshot['profile'] = 'assisted_without_ids'
        for t in self.snapshot['baselineTracks']:
            t['discogsReleaseId'] = None
        self.snapshot['request']['discogs_release_id'] = 'stripped-identity'
        with self.assertRaisesRegex(ValueError, 'profile'):
            self.export()
        self.snapshot['request']['discogs_release_id'] = None
        self.snapshot['profile'] = 'tagged_recovery'
        self.snapshot['guards'] = [dict(provider='musicbrainz', path=self.sources[0]['path'],
                                       sha256='0'*64, pairs=[])]
        with self.assertRaisesRegex(ValueError, 'guard.*receipt'):
            self.export()

    def test_synthetic_superset_donors_cannot_be_unioned(self):
        def wrong_guest(body):
            for level in [body['media'][0]['tracks'][12], body['media'][0]['tracks'][12]['recording']]:
                level['artist-credit'][1]['name'] = '50 Cent'
                level['artist-credit'][1]['artist']['name'] = '50 Cent'
                level['artist-credit'][1]['artist']['id'] = 'synthetic-50-cent'
            body['id'] = 'synthetic-other-source'
        self.add_source(M, 'musicbrainz', wrong_guest)
        result = self.export()
        conflict = next(c for c in result['fieldConflicts'] if c['localIndex'] == 12)
        self.assertEqual(conflict['disposition'], 'held_non_nested_assertions')
        self.assertEqual(len(conflict['assertions']), 2)

    def test_desperado_filename_only_missing_qualifier_stays_unresolved(self):
        q = exporter.qualifiers(dict(title='Doolin-Dalton', filename='07. Doolin-Dalton'),
                                dict(title='Doolin-Dalton (Instrumental)'))
        self.assertEqual(q['applicability'], 'unresolved')
        self.assertEqual(exporter.qualifiers(dict(title='Song (Live)'),
                         dict(title='Song (Studio)'))['applicability'], 'contradicted')

    def test_synthetic_recording_duration_and_ambiguous_mapping_cannot_be_hidden(self):
        def recording_disagreement(body):
            body['id'] = 'synthetic-recording-duration'
            body['media'][0]['tracks'][12]['recording']['length'] = 300000
        self.add_source(M, 'musicbrainz', recording_disagreement)
        result = self.export()['sources'][-1]
        self.assertIn('duration_evidence_insufficient', result['reasons'])
        self.assertEqual(result['occurrences'][12]['durations']['releaseTrackSeconds'], 278)
        self.assertEqual(result['occurrences'][12]['durations']['recordingSeconds'], 300)
        def ambiguous(body):
            body['id'] = 'synthetic-ambiguous'
            body['media'][0]['tracks'][1] = copy.deepcopy(body['media'][0]['tracks'][0])
        self.add_source(M, 'musicbrainz', ambiguous)
        self.assertIn('ambiguous_mapping', self.export()['sources'][-1]['reasons'])

    def test_synthetic_variant_detail_and_internal_contradiction_are_not_equivalence(self):
        for left, raw, outcome in [
                ('Song', dict(title='Song', recording=dict(disambiguation='explicit')), 'unresolved'),
                ('Song (Clean)', dict(title='Song (Clean)', recording=dict(disambiguation='explicit')), 'contradicted'),
                ('Song (Live 1991)', dict(title='Song (Live 2001)'), 'unresolved'),
                ('Song (Remix A)', dict(title='Song (Remix B)'), 'contradicted')]:
            with self.subTest(left=left, raw=raw):
                self.assertEqual(exporter.qualifiers(dict(title=left), raw)['applicability'], outcome)

    def test_synthetic_review_controls_do_not_misreport_credit_or_context_agreement(self):
        guest_only = exporter.credits(dict(extraartists=[dict(name='Dr. Dre', role='Featuring')]),
                                      'discogs', dict(artists=['Eminem']))
        self.assertFalse(guest_only['nonNested'])
        self.assertIsNone(guest_only['completeNames'])
        def credit(name, identity, join=''):
            return dict(name=name, artist=dict(id=identity, name=name), joinphrase=join)
        track = [credit('Lead', 'A', ' feat. '), credit('Guest', 'B')]
        for recording in [[credit('Guest', 'B', ' feat. '), credit('Lead', 'A')],
                          [credit('Lead', 'B', ' feat. '), credit('Guest', 'A')]]:
            row = exporter.credits({'artist-credit':track, 'recording':{'artist-credit':recording}},
                                   'musicbrainz', dict(artists=['Lead']))
            self.assertEqual(row['agreement'], 'conflict')
        local = dict(title='Song (Explicit)')
        raw = dict(title='Song', recording=dict(disambiguation='explicit'))
        q = exporter.qualifiers(local, raw, dict(localAlbum='Album (Explicit)', sourceDisambiguation='deluxe; clean'))
        self.assertNotEqual(q['applicability'], 'matching_claims')
        self.assertIn('release_context_variant_unresolved', q['reasons'])

    def test_synthetic_review_controls_bind_diagnostics_and_filename_positions(self):
        self.snapshot['positions'][0]['filenameDerived'] = 999
        with self.assertRaisesRegex(ValueError, 'prewrite'):
            self.export()
        self.snapshot['positions'][0]['filenameDerived'] = 1
        self.snapshot['guards'] = [dict(provider='musicbrainz', path=self.sources[0]['path'],
            sha256=self.sources[0]['sha256'], query=[['unreceived','query']], pairs=[dict(
                localIndex=12, sourceSelector='/media/0/tracks/12', calls=[dict(
                    function='unrelated_function', args=['not-the-prewrite-input'], result=True)])])]
        with self.assertRaisesRegex(ValueError, 'guard'):
            self.export()
        self.snapshot['guards'][0]['query'] = []
        with self.assertRaisesRegex(ValueError, 'guard'):
            self.export()

    def test_synthetic_derived_totals_and_recovery_ids_cannot_be_rewritten(self):
        self.snapshot['request']['tracks'][0]['track_total'] = 999
        with self.assertRaisesRegex(ValueError, 'prewrite'):
            self.export()
        self.snapshot['request']['tracks'][0]['track_total'] = 22
        self.snapshot['request']['discogs_release_id'] = 'invented-id'
        with self.assertRaisesRegex(ValueError, 'prewrite'):
            self.export()

    def test_receipt_query_pairs_match_the_real_imported_manifest_format(self):
        self.sources[0]['query'] = [['inc', 'recordings+artist-credits+labels+url-rels'], ['fmt', 'json']]
        self.snapshot['attempt']['requests'][0]['query'] = list(reversed(self.sources[0]['query']))
        self.assertEqual(self.export()['sources'][0]['catalogSupport'], 'supported')

    def test_synthetic_discogs_full_guest_assertion_participates_in_conflicts(self):
        def guests(body):
            body['id'] = 9999999
            body['tracklist'][12]['artists'] = [dict(name='Eminem', id=1)]
            body['tracklist'][12]['extraartists'] = [dict(name='50 Cent', id=2, role='Featuring')]
        self.add_source(FIXTURES / 'relapse-deluxe/release-36441795.json', 'discogs', guests)
        result = self.export()
        self.assertTrue(any(c['localIndex'] == 12 for c in result['fieldConflicts']))

    def test_synthetic_unmapped_is_insufficient_and_bad_recording_length_is_not_eligible(self):
        self.snapshot['request']['tracks'][0]['title'] = 'Unknown missing occurrence'
        self.snapshot['baselineTracks'][0]['title'] = 'Unknown missing occurrence'
        self.assertEqual(self.export()['sources'][0]['decision'], 'insufficient')
        def duration(body):
            body['id'] = 'synthetic-conflict-duration'
            body['media'][0]['tracks'][1]['recording']['length'] = 999999
        self.add_source(M, 'musicbrainz', duration)
        self.snapshot['request']['tracks'][1]['title'] += ' (Explicit)'
        self.snapshot['baselineTracks'][1]['title'] += ' (Explicit)'
        row = self.export()['sources'][-1]['occurrences'][1]
        self.assertEqual(row['conflictScope'], 'unresolved_relevant')


if __name__ == '__main__':
    unittest.main()
