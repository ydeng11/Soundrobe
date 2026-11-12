"""Scripted comparator controls: zero model-quality credit."""
import copy
import importlib.util
import hashlib
import json
import pathlib
import subprocess
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

    def test_user_acceptance_is_exact_and_never_catalog_or_grounded_truth(self):
        self.contract['slots'][2].update(labelBasis='user_acceptance',
            acceptedValues=['Pop', 'Mandopop', 'Pop, Mandopop'])
        for value in ['Pop', 'Mandopop', 'Pop, Mandopop', 'pop', 'Pop ', 'Pop,Mandopop',
                      'Mandopop, Pop', ['Pop', 'Mandopop'], 'Rock', None]:
            self.rows[2]['readback'] = value
            result = scorer.score(self.contract, self.rows)
            genre = result['fields'][2]
            accepted = value in ['Pop', 'Mandopop', 'Pop, Mandopop']
            with self.subTest(value=value):
                self.assertIsNone(genre['catalogValueAgreement'])
                self.assertEqual(genre['userAcceptanceAgreement'], accepted)
                self.assertEqual(result['userAcceptance']['requestedFieldSlots'], 1)
                self.assertEqual(result['userAcceptance']['agreeingFieldSlots'], int(accepted))
                self.assertEqual(genre['outcome'], 'accepted_user_fill' if accepted else
                                 'missing' if value is None else 'nonaccepted_user_change')
                self.assertEqual(result['groundedAssistanceAccuracyCredit'], 0)
                self.assertFalse(result['acceptedCertificate'])
        self.rows[2].update(readback='Pop', serviceFailure=True)
        result = scorer.score(self.contract, self.rows)
        self.assertEqual(result['userAcceptance'], {'requestedFieldSlots': 1, 'observedFieldSlots': 1,
            'agreeingFieldSlots': 1, 'serviceFailureSlots': 1})
        self.contract['slots'][2]['original'] = 'Pop'
        self.assertEqual(scorer.score(self.contract, self.rows)['fields'][2]['outcome'], 'unchanged_unknown')


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

    def test_cold_arm_projection_retains_missing_execution_without_warm_substitution(self):
        self.spec.update(version=2, mode='mock', arm='assistance', phases=['cold'])
        self.rows = [dict(self.rows[0], arm='assistance')]
        events = [{'kind': 'patch', 'path': 'disc/02.flac', 'fields': {'title': 'Song (Live)'}}]
        digest = hashlib.sha256(json.dumps(events, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        plan = dict(mode='mock_only', preparing=True, stopped=False, events=events,
                    planSha256=digest, requests=0, matchedEvents=0, variant='provider_only')
        preparation = dict(self.rows[0], plan=plan)
        raw = json.dumps(preparation).encode()
        (self.root / 'preparation.json').write_bytes(raw)
        self.spec['bindings']['preparation'] = {'path': 'preparation.json', 'sha256': hashlib.sha256(raw).hexdigest()}
        self.rows[0]['pairedMock'] = dict(plan, preparing=False, matchedEvents=1, observedEvents=events)
        contract = self.freeze()
        self.assertEqual(contract['jobs'][0]['id'], 'assistance/case/tagged_recovery/cold')
        result = scorer.project_native(self.root, contract, [])
        self.assertEqual(result['requestedFieldSlots'], 3)
        self.assertEqual(result['observedFieldSlots'], 0)
        self.assertEqual(result['serviceFailureSlots'], 3)
        self.assertEqual(result['predeclaredLabelableSlots'], 2)
        self.assertEqual(result['outcomes'], {'observation_unknown': 3})
        self.assertEqual(result['groundedAssistanceAccuracyCredit'], 0)
        # A failed observation can still agree; failure never erases its slot.
        self.rows[0]['serviceFailureObserved'] = True
        observed = scorer.project_native(self.root, contract, self.rows)
        self.assertEqual(observed['serviceFailureSlots'], 3)
        self.assertEqual(observed['outcomes']['correct_catalog_fill'], 1)
        for evidence in [None, dict(self.rows[0]['pairedMock'], stopped=True),
                         dict(self.rows[0]['pairedMock'], matchedEvents=0),
                         dict(self.rows[0]['pairedMock'], events=[]),
                         {k: v for k, v in self.rows[0]['pairedMock'].items() if k != 'observedEvents'},
                         dict(self.rows[0]['pairedMock'], observedEvents=[]),
                         dict(self.rows[0]['pairedMock'], observedEvents=[{
                             'kind': 'patch', 'path': 'not-approved.flac', 'fields': {'title': 'Unapproved'}}])]:
            invalid = dict(self.rows[0], serviceFailureObserved=False)
            if evidence is None:
                invalid.pop('pairedMock')
            else:
                invalid['pairedMock'] = evidence
            unavailable = scorer.project_native(self.root, contract, [invalid])
            self.assertEqual(unavailable['observedFieldSlots'], 0)
            self.assertEqual(unavailable['serviceFailureSlots'], 3)
            self.assertEqual(unavailable['outcomes'], {'observation_unknown': 3})
        for changed in [dict(self.rows[0], arm='disabled'), dict(self.rows[0], phase='warm'),
                        dict(self.rows[0], caseId='another')]:
            with self.assertRaises(ValueError):
                scorer.project_native(self.root, contract, [changed])
        with self.assertRaises(ValueError):
            scorer.project_native(self.root, contract, self.rows * 2)
        for change in [dict(mode='live'), dict(arm='unknown'), dict(phases=['cold', 'warm'])]:
            with self.assertRaises(ValueError):
                scorer.freeze_native_contract(self.root, dict(self.spec, **change), self.rows)

    def test_observed_genre_delta_is_bound_to_request_and_other_fields_remain_exact(self):
        self.spec.update(version=2, mode='mock', arm='assistance', phases=['cold'])
        self.rows = [dict(self.rows[0], arm='assistance')]
        payload = '{"album":"Album","artist":"A","tracks":["Song (Live)"]}'
        request = {'kind': 'request', 'body': {'model': 'scripted/genre-v1',
                   'messages': [{'role': 'user', 'content': payload}]}}
        base = {'kind': 'candidate', 'candidate': {'source': 'musicbrainz', 'genre': None}}
        events = [request, base, {'kind': 'review', 'required': False}, base,
                  {'kind': 'patch', 'path': 'disc/02.flac', 'fields': {'title': 'Song (Live)'}}]
        plan = dict(mode='mock_only', preparing=True, stopped=False, events=events,
                    planSha256=hashlib.sha256(json.dumps(events, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
                    requests=1, matchedEvents=0, variant='eligible_request')
        raw = json.dumps(dict(self.rows[0], plan=plan)).encode()
        (self.root / 'preparation.json').write_bytes(raw)
        self.spec['bindings']['preparation'] = {'path': 'preparation.json', 'sha256': hashlib.sha256(raw).hexdigest()}
        observed = copy.deepcopy(events)
        observed[1]['candidate'].update(genre='Diagnostic', assisted_genre={
            'value': 'Diagnostic', 'model': 'scripted/genre-v1',
            'input_sha256': hashlib.sha256(payload.encode()).hexdigest()})
        observed[3] = copy.deepcopy(observed[1])
        observed[2]['required'] = True
        observed[4]['fields']['genre'] = 'Diagnostic'
        self.rows[0]['pairedMock'] = dict(plan, preparing=False, matchedEvents=len(events), observedEvents=observed)
        contract = self.freeze()
        self.assertEqual(scorer.project_native(self.root, contract, self.rows)['observedFieldSlots'], 3)
        # No response-derived fields (e.g. rejected/low-confidence response) is also valid.
        unchanged = copy.deepcopy(self.rows)
        unchanged[0]['pairedMock']['observedEvents'] = events
        self.assertEqual(scorer.project_native(self.root, contract, unchanged)['observedFieldSlots'], 3)
        controls = []
        for key, value in [('model', 'other'), ('input_sha256', '0' * 64), ('value', 'Other')]:
            changed = copy.deepcopy(observed)
            changed[1]['candidate']['assisted_genre'][key] = value
            controls.append(changed)
        changed = copy.deepcopy(observed)
        changed[4]['fields']['title'] = 'Song'
        controls.append(changed)
        changed = copy.deepcopy(observed)
        changed[4]['fields']['genre'] = 'Unrelated'
        controls.append(changed)
        changed = copy.deepcopy(observed)
        changed[4]['fields'].pop('genre')
        controls.append(changed)
        changed = copy.deepcopy(observed)
        changed[3] = copy.deepcopy(base)
        controls.append(changed)
        changed = copy.deepcopy(observed)
        changed[2]['required'] = False
        controls.append(changed)
        controls.append(list(reversed(observed)))
        for actual in controls:
            with self.subTest(actual=actual):
                changed = copy.deepcopy(self.rows)
                changed[0]['pairedMock']['observedEvents'] = actual
                result = scorer.project_native(self.root, contract, changed)
                self.assertEqual(result['observedFieldSlots'], 0)
                self.assertEqual(result['serviceFailureSlots'], 3)

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


class FrozenUserProjection(unittest.TestCase):
    """Metadata-only integration; requires the locally retained frozen packet, not audio."""
    def setUp(self):
        self.root = pathlib.Path(__file__).resolve().parents[2]
        self.directory = '.planning/plans/crowd-lu-user-acceptance-v1-20261111'
        def bind(path):
            return {'path': path, 'sha256': hashlib.sha256((self.root / path).read_bytes()).hexdigest()}
        self.bind = bind
        inventory = 'test/fixtures/tauri/auto-tag-eval/artist-batch-extension/corpus.json'
        self.case = json.loads((self.root / inventory).read_text())['cases'][0]
        self.temp = tempfile.TemporaryDirectory(dir=self.root / '.amp/in')
        self.addCleanup(self.temp.cleanup)
        corpus_path = pathlib.Path(self.temp.name) / 'generated-corpus.json'
        corpus = {'cases': [dict(self.case, tracks=[dict(t, originalRelativePath=t['relativePath'],
            originalSourceSha256=t['sourceSha256'], sourceSha256='b' * 64) for t in self.case['tracks']])]}
        corpus_path.write_text(json.dumps(corpus))
        self.spec = {'version': 1, 'bindings': {
            'corpus': bind(str(corpus_path.relative_to(self.root))),
            'labels': bind('.planning/research/bounded-assistance-machine-labels-20261111.json'),
            'packet': bind('.planning/research/bounded-assistance-label-review-20261111.json'),
            'expectations': bind('test/fixtures/tauri/auto-tag-eval/reviewed-cjk-artist-2-20261108.json'),
            'userAcceptance': bind(self.directory + '/labels.json'),
            'userAcceptanceSeal': bind(self.directory + '/SHA256SUMS')},
            'fields': ['title', 'artists', 'genre'], 'sourceRoot': 'dummy/media',
            'caseIds': [self.case['caseId']], 'profile': 'tagged_recovery', 'phases': ['cold', 'warm']}
        before = [{'path': t['relativePath'], 'title': t['title'], 'artists': [t['artist']], 'genre': None}
                  for t in self.case['tracks']]
        raw = json.dumps({'tracks': before, 'hashes': {t['path']: {'fullSha256': 'c' * 64,
            'payloadSha256': 'd' * 64, 'bytes': 12} for t in before}})
        self.rows = [{'caseId': self.case['caseId'], 'profile': 'tagged_recovery', 'phase': phase,
            'sourceRelativeFolder': self.case['sourceRelativeFolder'], 'providerMode': 'offline_fixtures',
            'outcome': 'applied', 'serviceFailureObserved': False, 'cancellationRequested': False,
            'fieldObservations': {'inputJson': raw, 'inputSha256': hashlib.sha256(raw.encode()).hexdigest(),
                'requestJson': '{}', 'sourceRoot': 'dummy/media', 'readbackAvailable': True,
                'readbackError': None, 'tracks': copy.deepcopy(before)}} for phase in ['cold', 'warm']]

    def test_all_eleven_exact_alternatives_with_title_credit_and_version_controls(self):
        contract = scorer.freeze_native_contract(self.root, self.spec, self.rows)
        self.assertEqual(contract['labelCoverage']['uniqueUserAcceptanceLabels'], 11)
        for value in ['Pop', 'Mandopop', 'Pop, Mandopop', 'Mandopop, Pop', 'Pop,Mandopop', 'Rock']:
            rows = copy.deepcopy(self.rows)
            for row in rows:
                for track in row['fieldObservations']['tracks']:
                    track['genre'] = value
            result = scorer.project_native(self.root, contract, rows)
            expected = 22 if value in ['Pop', 'Mandopop', 'Pop, Mandopop'] else 0
            self.assertEqual(result['userAcceptance']['agreeingFieldSlots'], expected)
            self.assertEqual(result['requestedFieldSlots'], 66)
            self.assertEqual(result['userAcceptance']['requestedFieldSlots'], 22)
            self.assertEqual(result['groundedAssistanceAccuracyCredit'], 0)
            # Catalog qualifiers and ordered performer lists still have independent controls.
            rows[0]['fieldObservations']['tracks'][-1].update(title='英雄', artists=['Unsupported performer'])
            changed = scorer.project_native(self.root, contract, rows)
            self.assertEqual(changed['outcomes']['wrong_catalog_change'], 2)

    def test_user_labels_do_not_drop_unavailable_or_failed_observations(self):
        contract = scorer.freeze_native_contract(self.root, self.spec, self.rows)
        self.rows[0].update(outcome='admission_denied', serviceFailureObserved=True)
        self.rows[0]['fieldObservations'].update(readbackAvailable=False, readbackError='unavailable', tracks=None)
        result = scorer.project_native(self.root, contract, self.rows)
        self.assertEqual(result['requestedFieldSlots'], 66)
        self.assertEqual(result['userAcceptance'], {'requestedFieldSlots': 22, 'observedFieldSlots': 11,
            'agreeingFieldSlots': 0, 'serviceFailureSlots': 11})
        self.assertEqual(result['outcomes']['observation_unknown'], 33)

    def test_wrong_scope_missing_seal_and_rebound_label_bytes_reject(self):
        variants = []
        for key, value in [('profile', 'folder_filename'), ('caseIds', ['other'])]:
            spec = copy.deepcopy(self.spec)
            spec[key] = value
            variants.append(spec)
        spec = copy.deepcopy(self.spec)
        del spec['bindings']['userAcceptanceSeal']
        variants.append(spec)
        original = json.loads((self.root / self.directory / 'labels.json').read_text())
        mutations = [('occurrences', original['occurrences'][:-1]), ('labelBasis', 'acoustic_truth'),
                     ('albumLevel', {'userAnswer': 'yes', 'acceptedGenreValues': ['Rock']})]
        for key, value in [('relativePath', 'wrong.flac'), ('capturedMediaSha256Claim', '0' * 64),
                           ('inventorySelector', '/cases/0/tracks/1'), ('acceptedGenreValues', ['Pop', 'Rock'])]:
            occurrences = copy.deepcopy(original['occurrences'])
            occurrences[0][key] = value
            mutations.append(('occurrences', occurrences))
        mutations.append(('occurrences', original['occurrences'] + [original['occurrences'][0]]))
        for index, (key, value) in enumerate(mutations):
            changed = dict(original, **{key: value})
            path = pathlib.Path(self.temp.name) / f'{index}.json'
            path.write_text(json.dumps(changed))
            spec = copy.deepcopy(self.spec)
            spec['bindings']['userAcceptance'] = self.bind(str(path.relative_to(self.root)))
            variants.append(spec)
        # Missing keys/types also reject through the public native projector.
        contract = scorer.freeze_native_contract(self.root, self.spec, self.rows)
        for bad in [None, {}, {'path': 'missing', 'sha256': 'wrong'},
                    {'path': '../outside', 'sha256': 'wrong'}, ['not a binding']]:
            changed = copy.deepcopy(contract)
            changed['spec']['bindings']['userAcceptance'] = bad
            with self.subTest(binding=bad), self.assertRaises((ValueError, OSError)):
                scorer.project_native(self.root, changed, self.rows)
        for spec in variants:
            with self.subTest(spec=spec), self.assertRaises(ValueError):
                scorer.freeze_native_contract(self.root, spec, self.rows)


class ColdDriverSelection(unittest.TestCase):
    def test_explicit_cold_plan_has_six_arm_qualified_jobs_and_one_conditional_role(self):
        driver = str(pathlib.Path(__file__).with_name('run-bounded.cjs').resolve())
        jobs = json.loads(subprocess.check_output(['node', '-e',
            'console.log(JSON.stringify(require(process.argv[1]).pairedColdJobs()))', driver]))
        self.assertEqual([(j['arm'], j['caseId']) for j in jobs], [
            ('disabled', '036e0a00e45b'), ('disabled', '54e7923a3000'), ('disabled', 'f7756354af37'),
            ('assistance', '54e7923a3000'), ('assistance', 'f7756354af37'), ('assistance', '036e0a00e45b')])
        self.assertEqual(len({j['name'] for j in jobs}), 6)
        self.assertEqual([j['name'] for j in jobs if j['role']], ['assistance-crowd'])

    def test_failure_publication_keeps_validated_scores_and_unfinished_denominators(self):
        driver = str(pathlib.Path(__file__).with_name('run-bounded.cjs').resolve())
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            fields = ['title', 'artists', 'genre']
            evaluations = [dict(name=f'{arm}-case', arm=arm, caseId='case', profile='tagged_recovery',
                phase='cold', fields=fields, physicalPaths=['one.flac']) for arm in ['disabled', 'assistance']]
            (root / 'requested-membership.json').write_text(json.dumps(dict(evaluations=evaluations,
                requestedFieldSlots=6, requestedPhysicalOccurrences=2, predeclaredLabelableSlots=2)))
            ready = root / 'disabled-case'
            ready.mkdir()
            (ready / 'result.provisional.json').write_text(json.dumps(dict(name='disabled-case',
                caseId='case', arm='disabled', profile='tagged_recovery', score=dict(requestedFieldSlots=3,
                observedFieldSlots=3, serviceFailureSlots=0, fields=[dict(id=f'disabled/case/tagged_recovery/cold/one.flac/{f}') for f in fields]))))
            result = json.loads(subprocess.check_output(['node', '-e',
                'console.log(JSON.stringify(require(process.argv[1]).failureEnvelope(process.argv[2])))', driver, directory]))
            self.assertEqual(result['requestedFieldSlots'], 6)
            self.assertEqual(result['observedFieldSlots'], 3)
            self.assertEqual(result['serviceFailureSlots'], 3)
            self.assertEqual(result['evaluations'][1]['nativeObservation'], 'unavailable')


def load_tests(loader, tests, pattern):
    # Portable controls stay runnable without ignored local planning records.
    return unittest.TestSuite(loader.loadTestsFromTestCase(case)
                              for case in [ObservationScoring, NativeProjection, ColdDriverSelection])


if __name__ == '__main__':
    unittest.main()
