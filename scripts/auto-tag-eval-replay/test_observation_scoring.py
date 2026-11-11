"""Scripted comparator controls: zero model-quality credit."""
import copy
import importlib.util
import hashlib
import json
import pathlib
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('scorer', pathlib.Path(__file__).with_name('score-observations.py'))
scorer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scorer)


class ObservationScoring(unittest.TestCase):
    def setUp(self):
        self.contract = {'slots': [
            {'id': 'case/profile/cold/file/title', 'original': '', 'acceptedValues': ['Version (Live)']},
            {'id': 'case/profile/cold/file/artists', 'original': ['A', 'B'], 'acceptedValues': [['A', 'B']]},
            {'id': 'case/profile/cold/file/genre', 'original': '', 'acceptedValues': None},
        ]}
        self.rows = [{'id': slot['id'], 'readbackAvailable': True, 'readback': value,
                      'serviceFailure': False, 'decisionVisibility': 'unavailable'}
                     for slot, value in zip(self.contract['slots'], ['Version (Live)', ['A', 'B'], 'Pop'])]

    def test_value_agreement_does_not_infer_intent_or_genre_truth(self):
        result = scorer.score(self.contract, self.rows)
        self.assertEqual([r['outcome'] for r in result['fields']],
                         ['correct_catalog_fill', 'unchanged_unknown', 'unscorable'])
        self.assertEqual(result['requestedFieldSlots'], 3)
        self.assertEqual(result['predeclaredLabelableSlots'], 2)
        self.assertEqual(result['groundedAssistanceAccuracyCredit'], 0)
        self.assertEqual(result['writableDelta'], [])
        self.assertFalse(result['acceptedCertificate'])

    def test_missing_failed_observation_and_unsupported_extras_stay_distinct(self):
        for change, expected in [('', 'missing'), ('Version', 'wrong_catalog_change')]:
            rows = copy.deepcopy(self.rows)
            rows[0]['readback'] = change
            self.assertEqual(scorer.score(self.contract, rows)['fields'][0]['outcome'], expected)
        rows = copy.deepcopy(self.rows)
        rows[1]['readback'] = ['A', 'B', 'Unsupported']
        self.assertEqual(scorer.score(self.contract, rows)['fields'][1]['outcome'], 'wrong_catalog_change')
        rows[0].update(readbackAvailable=False, serviceFailure=True)
        result = scorer.score(self.contract, rows)
        self.assertEqual(result['fields'][0]['outcome'], 'observation_unknown')
        self.assertEqual(result['serviceFailureSlots'], 1)
        self.assertEqual(result['predeclaredLabelableSlots'], 2)

    def test_fixed_membership_rejects_drops_duplicates_and_unknown_decisions(self):
        for rows in [self.rows[:-1], self.rows + [self.rows[0]]]:
            with self.assertRaises(ValueError):
                scorer.score(self.contract, rows)
        rows = copy.deepcopy(self.rows)
        rows[0]['decisionVisibility'] = 'approved'
        with self.assertRaises(ValueError):
            scorer.score(self.contract, rows)

    def test_available_readback_must_be_explicit_even_when_empty(self):
        rows = copy.deepcopy(self.rows)
        del rows[0]['readback']
        with self.assertRaises(ValueError):
            scorer.score(self.contract, rows)
        rows[0]['readback'] = None
        self.assertEqual(scorer.score(self.contract, rows)['fields'][0]['outcome'], 'missing')

    def test_empty_unchanged_is_missing_without_inventing_a_hold(self):
        rows = copy.deepcopy(self.rows)
        rows[0]['readback'] = ''
        field = scorer.score(self.contract, rows)['fields'][0]
        self.assertEqual(field['outcome'], 'missing')
        self.assertEqual(field['valueTransition'], 'unchanged')
        self.assertEqual(field['decisionVisibility'], 'unavailable')


class NativeProjection(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.bindings = {}
        def save(key, value):
            raw = json.dumps(value, ensure_ascii=False).encode()
            path = key + '.json'
            (self.root / path).write_bytes(raw)
            self.bindings[key] = {'path': path, 'sha256': hashlib.sha256(raw).hexdigest()}
        self.save = save
        track = {'relativePath': 'disc/02.flac', 'originalRelativePath': 'disc/02.wav',
                 'originalSourceSha256': 'a' * 64, 'sourceSha256': 'b' * 64}
        save('corpus', {'cases': [{'caseId': 'case', 'sourceRelativeFolder': 'Artist/Album', 'tracks': [track]}]})
        save('inventory', {'cases': [{'caseId': 'case', 'sourceRelativeFolder': 'Artist/Album', 'tracks': [
            {'relativePath': 'disc/02.wav', 'sourceSha256': 'a' * 64}]}]})
        packet = {'sourceRoot': '.', 'sourceRegister': {'I': self.bindings['inventory']}, 'catalogLabels': [
            {'caseId': 'case', 'inventorySource': 'I', 'inventoryOccurrence': '/cases/0/tracks/0',
             'physicalRelativePath': 'disc/02.wav', 'capturedMediaSha256Claim': 'a' * 64}]}
        save('packet', packet)
        save('labels', {'parentPacketSha256': self.bindings['packet']['sha256'], 'occurrences': [
            {'caseId': 'case', 'inventorySource': 'I', 'inventoryOccurrence': '/cases/0/tracks/0',
             'acceptedArtistLists': [['Lead', 'Guest']], 'acceptedTitleValues': ['Song (Live)']}]})
        save('expectations', {'independent': 'not labels'})
        self.spec = {'version': 1, 'bindings': self.bindings, 'fields': ['title', 'artists', 'genre'],
                     'sourceRoot': 'dummy/media', 'caseIds': ['case'], 'profile': 'tagged_recovery',
                     'phases': ['cold', 'warm']}
        tracks = [{'path': 'disc/02.flac', 'title': '', 'artists': ['Lead'], 'genre': None}]
        hashes = {'disc/02.flac': {'fullSha256': 'c' * 64, 'payloadSha256': 'd' * 64, 'bytes': 12}}
        raw = json.dumps({'tracks': tracks, 'hashes': hashes}, separators=(',', ':'))
        self.rows = [{'caseId': 'case', 'profile': 'tagged_recovery', 'phase': phase,
                      'sourceRelativeFolder': 'Artist/Album', 'providerMode': 'offline_fixtures',
                      'outcome': 'applied', 'cancellationRequested': False, 'serviceFailureObserved': False,
                      'fieldObservations': {'inputJson': raw, 'inputSha256': hashlib.sha256(raw.encode()).hexdigest(),
                        'sourceRoot': 'dummy/media', 'requestJson': '{"path":"."}',
                        'readbackAvailable': True, 'readbackError': None,
                        'tracks': [{'path': 'disc/02.flac', 'title': 'Song (Live)', 'artists': ['Lead', 'Guest'], 'genre': 'Scripted'}]}}
                     for phase in ['cold', 'warm']]

    def freeze(self, rows=None):
        if not hasattr(scorer, 'freeze_native_contract'):
            self.fail('native projection and frozen source validation are missing')
        return scorer.freeze_native_contract(self.root, self.spec, self.rows if rows is None else rows)

    def test_native_projection_preserves_full_denominator_and_independent_labels(self):
        contract = self.freeze()
        result = scorer.project_native(self.root, contract, self.rows)
        self.assertEqual(result['requestedFieldSlots'], 6)
        self.assertEqual(result['predeclaredLabelableSlots'], 4)
        self.assertEqual(result['outcomes'], {'correct_catalog_fill': 2, 'correct_catalog_change': 2, 'unscorable': 2})
        self.assertEqual(result['groundedAssistanceAccuracyCredit'], 0)
        self.assertEqual(result['writableDelta'], [])

    def test_corruption_omission_duplicates_and_wrong_relative_paths_reject(self):
        contract = self.freeze()
        variants = [self.rows[:-1], self.rows + [self.rows[0]]]
        for key, value in [('inputSha256', 'e' * 64), ('sourceRoot', 'original/library'),
                           ('requestJson', '{"path":"changed"}')]:
            rows = copy.deepcopy(self.rows)
            rows[0]['fieldObservations'][key] = value
            variants.append(rows)
        for paths in [[], ['disc/02.flac', 'disc/02.flac'], ['other/02.flac'], ['../02.flac']]:
            rows = copy.deepcopy(self.rows)
            rows[0]['fieldObservations']['tracks'] = [dict(rows[0]['fieldObservations']['tracks'][0], path=p) for p in paths]
            variants.append(rows)
        rows = copy.deepcopy(self.rows)
        raw = json.loads(rows[0]['fieldObservations']['inputJson'])
        raw['tracks'][0]['artists'] = ['Changed baseline']
        rows[0]['fieldObservations']['inputJson'] = json.dumps(raw)
        rows[0]['fieldObservations']['inputSha256'] = hashlib.sha256(json.dumps(raw).encode()).hexdigest()
        variants.append(rows)
        for rows in variants:
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                scorer.project_native(self.root, contract, rows)

    def test_source_and_label_mismatch_reject_even_if_native_values_match(self):
        contract = self.freeze()
        self.save('labels', {'parentPacketSha256': 'wrong', 'occurrences': []})
        with self.assertRaises(ValueError):
            scorer.project_native(self.root, contract, self.rows)
        self.spec['bindings'] = self.bindings
        with self.assertRaises(ValueError):
            self.freeze()

    def test_freeze_rejects_unmapped_or_duplicated_reviewed_originals(self):
        for mutation in ['unmapped', 'duplicate']:
            with self.subTest(mutation=mutation):
                corpus = json.loads((self.root / 'corpus.json').read_text())
                original = corpus['cases'][0]['tracks'][0]
                if mutation == 'unmapped':
                    original['originalRelativePath'] = 'other.wav'
                else:
                    corpus['cases'][0]['tracks'].append(dict(original, relativePath='disc/copy.flac'))
                self.save('corpus', corpus)
                rows = copy.deepcopy(self.rows)
                if mutation == 'duplicate':
                    for row in rows:
                        before = json.loads(row['fieldObservations']['inputJson'])
                        before['tracks'].append(dict(before['tracks'][0], path='disc/copy.flac'))
                        before['hashes']['disc/copy.flac'] = before['hashes']['disc/02.flac']
                        raw = json.dumps(before)
                        row['fieldObservations'].update(inputJson=raw, inputSha256=hashlib.sha256(raw.encode()).hexdigest())
                with self.assertRaises(ValueError):
                    self.freeze(rows)
                # Restore only this disposable test fixture.
                self.save('corpus', {'cases': [{'caseId': 'case', 'sourceRelativeFolder': 'Artist/Album', 'tracks': [
                    {'relativePath': 'disc/02.flac', 'originalRelativePath': 'disc/02.wav',
                     'originalSourceSha256': 'a' * 64, 'sourceSha256': 'b' * 64}]}]})

    def test_freeze_rejects_malformed_hash_records(self):
        for malformed in [None, {}, {'fullSha256': 'bad', 'payloadSha256': 'd' * 64, 'bytes': True}]:
            rows = copy.deepcopy(self.rows)
            for row in rows:
                before = json.loads(row['fieldObservations']['inputJson'])
                before['hashes']['disc/02.flac'] = malformed
                raw = json.dumps(before)
                row['fieldObservations'].update(inputJson=raw, inputSha256=hashlib.sha256(raw.encode()).hexdigest())
            with self.subTest(malformed=malformed), self.assertRaises(ValueError):
                self.freeze(rows)

    def test_unavailable_readback_retains_failed_cancelled_and_held_slots(self):
        contract = self.freeze()
        for outcome, cancelled in [('failed', False), ('needs_review', False), ('cancelled', True)]:
            rows = copy.deepcopy(self.rows)
            rows[0].update(outcome=outcome, cancellationRequested=cancelled)
            rows[0]['fieldObservations'].update(readbackAvailable=False, readbackError='reader unavailable', tracks=None)
            result = scorer.project_native(self.root, contract, rows)
            self.assertEqual(result['requestedFieldSlots'], 6)
            self.assertEqual(result['predeclaredLabelableSlots'], 4)
            self.assertEqual(result['outcomes']['observation_unknown'], 3)
            rows[0]['fieldObservations']['readbackAvailable'] = True
            with self.assertRaises(ValueError):
                scorer.project_native(self.root, contract, rows)

    def test_provider_or_model_service_failure_is_separate_from_usable_album_outcome(self):
        contract = self.freeze()
        for outcome in ['applied', 'needs_review', 'admission_denied']:
            rows = copy.deepcopy(self.rows)
            rows[0].update(outcome=outcome, serviceFailureObserved=True)
            result = scorer.project_native(self.root, contract, rows)
            self.assertEqual(result['requestedFieldSlots'], 6)
            self.assertEqual(result['serviceFailureSlots'], 3)
            self.assertEqual(result['observedFieldSlots'], 6)

    def test_manifest_bound_projection_rejects_changed_origin_resources_and_invocation(self):
        self.spec['bindings']['input'] = self.bindings['corpus']
        self.spec['transport'] = {'provider': 'openai', 'model': 'scripted/genre-v1',
            'endpoint': 'http://127.0.0.1:12345/model', 'limits': {'attempts': 2, 'repairs': 0}}
        contract = self.freeze()
        def write(name, value):
            raw = json.dumps(value).encode()
            (self.root / name).write_bytes(raw)
            return {'path': name, 'sha256': hashlib.sha256(raw).hexdigest()}
        manifest = {'mode': 'mock', **self.spec['transport'], 'jobs': contract['jobs'],
            'bindings': {**contract['spec']['bindings'], 'scoringContract': write('contract.json', contract)}}
        for mutation in [None, 'origin', 'resources', 'invocation']:
            changed = copy.deepcopy(manifest)
            if mutation == 'origin':
                changed['endpoint'] = 'http://127.0.0.1:23456/model'
            if mutation == 'resources':
                changed['limits']['attempts'] = 3
            binding = write('manifest.json', changed)
            rows = copy.deepcopy(self.rows)
            for row in rows:
                row.update(boundedAssistance={'manifestSha256': binding['sha256']},
                           profileInputSha256=row['fieldObservations']['inputSha256'])
            if mutation == 'invocation':
                rows[0]['boundedAssistance']['manifestSha256'] = 'wrong'
            if mutation is None:
                self.assertEqual(scorer.project_native(self.root, contract, rows, binding)['requestedFieldSlots'], 6)
            else:
                with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                    scorer.project_native(self.root, contract, rows, binding)

    def test_reader_fallback_cannot_be_scored_as_available_metadata(self):
        contract = self.freeze()
        rows = copy.deepcopy(self.rows)
        rows[0]['fieldObservations']['tracks'][0].update(codec='unknown', title='02.flac', artists=[])
        with self.assertRaises(ValueError):
            scorer.project_native(self.root, contract, rows)


if __name__ == '__main__':
    unittest.main()
