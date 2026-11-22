"""Contract tests for fixed-denominator paired native reports."""
import importlib.util
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


spec = importlib.util.spec_from_file_location('paired', Path(__file__).with_name('audit-paired.py'))
paired = importlib.util.module_from_spec(spec)
if spec.loader and Path(spec.origin).exists():
    spec.loader.exec_module(paired)


class PairedReportTests(unittest.TestCase):
    def test_relapse_profile_oracle_retains_captured_title_and_supported_billing(self):
        fixtures = Path(__file__).resolve().parents[2] / 'test/fixtures/tauri/auto-tag-eval'
        review = next(row for row in paired.load(fixtures / 'reviewed-subset.json')['cases']
                      if row['caseId'] == '8452b6f2b3ea')
        discogs = paired.load(fixtures / review['snapshot'])
        truth = {'mapping': [dict(localTrack=i, providerTrack=str(track['track_number']),
                                  providerTitle=track['title']) for i, track in enumerate(discogs['tracks'], 1)]}
        # The reviewed files reverse the two bonus tracks relative to Discogs.
        for local, provider in ((21, 22), (22, 21)):
            track = discogs['tracks'][provider - 1]
            truth['mapping'][local - 1] = dict(localTrack=local, providerTrack=str(provider),
                                              providerTitle=track['title'])
        expected = paired.reviewed_fields(review, truth, fixtures)
        plain = paired.expected_for_profile(expected, 'folder_filename')
        self.assertEqual('Relapse', plain['album'])
        self.assertEqual(['Eminem', 'Dr. Dre'], plain['credits'][12])
        self.assertEqual(['Eminem', 'Dr. Dre', '50 Cent'], plain['credits'][17])
        for profile in ('assisted_without_ids', 'tagged_recovery'):
            tagged = paired.expected_for_profile(expected, profile)
            self.assertEqual('Relapse (Deluxe)', tagged['album'])
            self.assertFalse(paired.album_title_matches('Relapse', tagged))
            self.assertEqual(['Eminem', 'Dr. Dre'], tagged['credits'][12])
            self.assertEqual(['Eminem', '50 Cent', 'Dr. Dre'], tagged['credits'][17])
            self.assertFalse(paired.performer_credits_match('Eminem', ['Eminem'], tagged['credits'][12], tagged))
            self.assertFalse(paired.performer_credits_match('Eminem & 50 Cent & Dr. Dre',
                             ['Eminem & 50 Cent & Dr. Dre'], tagged['credits'][17], tagged))
        with self.assertRaises(ValueError):
            mismapped = list(truth['mapping'])
            mismapped[17] = dict(mismapped[17], providerTrack='13')
            paired.reviewed_fields(review, {'mapping': mismapped}, fixtures)

    def test_confirmed_solo_spellings_do_not_accept_duplicate_or_extra_credits(self):
        fixtures = Path(__file__).resolve().parents[2] / 'test/fixtures/tauri/auto-tag-eval'
        for case_id, manifest, traditional, simplified in [
                ('204915b5a4ec', 'reviewed-cjk-artist-2-20261108.json', '盧廣仲', '卢广仲'),
                ('5fdcfd7d2149', 'reviewed-cjk-library-20261108.json', '草東沒有派對', '草东没有派对')]:
            review = next(row for row in paired.load(fixtures / manifest)['cases'] if row['caseId'] == case_id)
            expected = paired.reviewed_alias_fields(review, {}, fixtures)
            self.assertTrue(paired.performer_credits_match(simplified, [simplified], [traditional], expected))
            self.assertFalse(paired.performer_credits_match('Other', [simplified], [traditional], expected))
            self.assertFalse(paired.performer_credits_match(simplified, [simplified, simplified], [traditional], expected))
            self.assertFalse(paired.performer_credits_match(simplified, [simplified], [traditional, 'Guest'], expected))
            self.assertEqual({}, paired.reviewed_alias_fields(dict(review, caseId='other'), {}, fixtures))
            self.assertEqual({}, paired.reviewed_alias_fields(dict(review, artistAliases=[]), {}, fixtures))

    def test_confirmed_enya_titles_are_bound_to_release_and_track(self):
        fixtures = Path(__file__).resolve().parents[2] / 'test/fixtures/tauri/auto-tag-eval'
        winter = next(row for row in paired.load(fixtures / 'reviewed-subset.json')['cases']
                      if row['caseId'] == 'c0e96540c785')
        expected = dict(album='And Winter Came... = 雪と氷の旋律',
                        **paired.reviewed_alias_fields(winter, {}, fixtures))
        self.assertTrue(paired.album_title_matches('And Winter Came…', expected))
        self.assertFalse(paired.album_title_matches('And Winter Came (Live)', expected))
        self.assertEqual({}, paired.reviewed_alias_fields(dict(winter, releaseId='other'), {}, fixtures))

        shepherd = next(row for row in paired.load(fixtures / 'reviewed-expansion-2-20261108.json')['cases']
                        if row['caseId'] == '872fee627bfd')
        track = dict(localTrack=7, providerTrack='7', providerTitle='Book of Days', localTitle='Book of Days')
        expected = paired.reviewed_alias_fields(shepherd, {'mapping': [track]}, fixtures)
        title = 'Book of Days (Gaelic only version)'
        recording = 'e5cc479b-a250-4967-b84c-5b66bb12f487'
        score = lambda value, mapping, recording_id=recording: paired.track_title_outcome(
            value, mapping, 'Enya', shepherd['caseId'], expected, recording_id)['status']
        self.assertEqual('correct', score(title, track))
        self.assertNotEqual('correct', score(title, track, None))
        for mapping in [dict(track, localTrack=8), dict(track, providerTrack='8'), dict(track, providerTitle='Other')]:
            self.assertNotEqual('correct', score(title, mapping))
        self.assertNotEqual('correct', score('Book of Days (Live)', track))
        self.assertEqual({}, paired.reviewed_alias_fields(dict(shepherd, caseId='other'), {'mapping': [track]}, fixtures))

    def test_lo_vas_a_olvidar_oracle_scores_album_display_and_ordered_performers(self):
        fixtures = Path(__file__).resolve().parents[2] / 'test/fixtures/tauri/auto-tag-eval'
        for case, manifest in [('3b95f81ac351', 'reviewed-expansion-2-20261108.json'),
                               ('4060f1de6662', 'reviewed-billie-20261108.json')]:
            review = next(row for row in paired.load(fixtures / manifest)['cases'] if row['caseId'] == case)
            source = paired.load(fixtures / review['snapshot'])
            mapping = {'mapping': [{'providerTitle': source['media'][0]['tracks'][0]['title']}]}
            expected = paired.reviewed_fields(review, mapping, fixtures)
            self.assertEqual(expected['albumArtists'], ['Billie Eilish', 'ROSALÍA'])
            self.assertTrue(paired.album_credit_matches('Billie Eilish & Rosalía',
                                                        ['Billie Eilish', 'Rosalía'], expected))
            self.assertTrue(paired.album_credit_matches('Billie Eilish | ROSALÍA',
                                                        ['Billie Eilish', 'ROSALÍA'], expected))
            self.assertFalse(paired.album_credit_matches('Billie Eilish & Rosalía',
                                                         ['Billie Eilish & Rosalía'], expected))
            self.assertFalse(paired.album_credit_matches('Billie Eilish',
                                                         ['Billie Eilish'], expected))
            self.assertFalse(paired.album_credit_matches('Billie Eilish & Rosalía',
                                                         ['Rosalía', 'Billie Eilish'], expected))
            self.assertNotIn('albumArtists', paired.reviewed_fields(dict(review, caseId='other'), mapping, fixtures))

    def test_exile_alternate_oracle_is_bound_to_the_reviewed_case(self):
        fixtures = Path(__file__).resolve().parents[2] / 'test/fixtures/tauri/auto-tag-eval'
        review = next(row for row in paired.load(fixtures / 'reviewed-subset.json')['cases']
                      if row['caseId'] == 'e8a8a8d6b27d')
        discogs = paired.load(fixtures / review['snapshot'])
        truth = {'mapping': [{'providerTitle': track['title']} for track in discogs['tracklist']]}
        expected = paired.reviewed_fields(review, truth, fixtures)
        self.assertEqual(expected['credits'], [['Enya']] * 4)
        self.assertEqual(expected['album'], discogs['title'])
        self.assertEqual(expected['albumAliases'], ['Exile'])
        self.assertTrue(paired.album_title_matches('Exile', expected))
        self.assertTrue(paired.album_title_matches(discogs['title'], expected))
        self.assertFalse(paired.album_title_matches('Exile (Live)', expected))
        other = paired.reviewed_fields(dict(review, caseId='unrelated'), truth, fixtures)
        self.assertEqual(other['credits'], [['Enya', 'Enya']] * 4)
        self.assertFalse(paired.album_title_matches('Exile', other))

    def test_paired_report_accepts_reviewed_album_alternate_but_rejects_other_titles(self):
        for observed, recoveries in [('Exile', 2), ('Exile (Live)', 0)]:
            with self.subTest(observed=observed), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                self.prepare_report(root, written=2)
                inputs = paired.load(root / 'comparison-inputs.json')
                expected = inputs['jobs'][0]['expected']
                expected.update(album='Exile = エクサイル', albumAliases=['Exile'])
                self.save(root / 'comparison-inputs.json', inputs)
                for arm in ['baseline', 'candidate']:
                    for phase in ['cold', 'warm']:
                        path = root / arm / 'folder_filename' / 'case' / f'{phase}.jsonl'
                        record = paired.load(path)
                        for track in record['readbackTracks']:
                            track['album'] = observed
                        self.save(path, record)
                report = self.report(root, expected_exit=0 if recoveries else 1)
                self.assertEqual(report['candidate']['correctRecoveries'], recoveries)

    def test_source_mapping_audit_recomputes_binding_and_joins_locked_source_to_files(self):
        canonical = lambda value: json.dumps(value, ensure_ascii=False, sort_keys=True,
                                             separators=(',', ':')).encode()
        digest = lambda value: hashlib.sha256(value).hexdigest()
        body = dict(id=42, notes='Genre: Dream pop', tracklist=[
            dict(position='1', title='Song'), dict(position='2', title='Other')])
        body_bytes = json.dumps(body, ensure_ascii=False, separators=(',', ':')).encode()
        body_sha = digest(body_bytes)
        fixture = dict(label='synthetic_loopback_counterfactual', releaseId='42',
                       route=dict(provider='discogs', path='/releases/42', query={}),
                       successReleaseResponse=body, sourceText=body['notes'], genre='Dream pop')
        route = dict(provider='discogs', path='/releases/42', query='[]', status=200, sha256=body_sha)
        inventory = dict(records=[route], requests=[dict(provider='discogs', path='/releases/42',
                                                           query=[], status=200)])
        request_tracks = [dict(title='Song', filename='01 Song')]
        mapping = dict(coverage=1, remoteIndices=[0], evidence=['TagTitle'], providerTracks=[
            dict(title='Song', track_number=1), dict(title='Other', track_number=2)])
        binding = dict(releaseId='42', requestTracks=request_tracks, mapping=mapping)
        mapping_sha = digest(canonical(binding))
        source_evidence = dict(provider='discogs', resourceId='42', responseSha256=body_sha,
                               selector='/notes', textSha256=digest(b'Genre: Dream pop'),
                               trackMappingSha256=mapping_sha)
        source_evidence['evidenceId'] = digest(canonical(source_evidence))
        record = dict(genreSourceBindingEvidence={**binding, 'mappingSha256': mapping_sha},
                      fieldObservations=dict(requestJson=json.dumps(dict(tracks=request_tracks))),
                      native=dict(candidate=dict(assisted_genre=dict(value='Dream pop', source_observation={
                          'origin': 'synthetic_loopback', **source_evidence, 'text': body['notes']}))))
        case = dict(tracks=[dict(relativePath='01 Song.flac')])
        truth = dict(mapping=[dict(localTrack=1, localTitle='Song', providerTitle='Song',
                                   providerTrack='1')])
        self.assertEqual(paired.source_mapping_failures(record, case, truth, fixture, inventory), [])

        missing = dict(record)
        missing.pop('genreSourceBindingEvidence')
        self.assertTrue(paired.source_mapping_failures(missing, case, truth, fixture, inventory))

        swapped = json.loads(json.dumps(record))
        swapped['genreSourceBindingEvidence']['requestTracks'][0]['filename'] = '02 Other'
        swapped['fieldObservations']['requestJson'] = json.dumps(dict(
            tracks=swapped['genreSourceBindingEvidence']['requestTracks']))
        self.assertTrue(paired.source_mapping_failures(swapped, case, truth, fixture, inventory))

        omitted = json.loads(json.dumps(record))
        omitted['genreSourceBindingEvidence']['requestTracks'] = []
        omitted['fieldObservations']['requestJson'] = json.dumps(dict(tracks=[]))
        self.assertTrue(paired.source_mapping_failures(omitted, case, truth, fixture, inventory))

        forged = json.loads(json.dumps(record))
        forged_map = forged['genreSourceBindingEvidence']
        forged_map['mapping']['remoteIndices'] = [1]
        forged_map['mappingSha256'] = digest(canonical({key: forged_map[key]
                                                         for key in ['releaseId', 'requestTracks', 'mapping']}))
        forged_source = forged['native']['candidate']['assisted_genre']['source_observation']
        forged_source['trackMappingSha256'] = forged_map['mappingSha256']
        forged_evidence = {key: forged_source[key] for key in ['provider', 'resourceId', 'responseSha256',
                                                               'selector', 'textSha256', 'trackMappingSha256']}
        forged_source['evidenceId'] = digest(canonical(forged_evidence))
        self.assertTrue(paired.source_mapping_failures(forged, case, truth, fixture, inventory))

        changed_source = json.loads(json.dumps(record))
        changed_fixture = json.loads(json.dumps(fixture))
        changed_fixture['successReleaseResponse']['notes'] = 'Genre: Noise pop'
        self.assertTrue(paired.source_mapping_failures(changed_source, case, truth, changed_fixture, inventory))

        changed_route = json.loads(json.dumps(fixture))
        changed_route['route']['path'] = '/masters/42'
        self.assertTrue(paired.source_mapping_failures(record, case, truth, changed_route, inventory))

    def test_report_keeps_expected_model_failure_separate_and_retains_provider_write(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.prepare_report(root)
            inputs = json.loads((root / 'comparison-inputs.json').read_text())
            fixture = dict(caseId='case', profile='folder_filename', artist='Artist', album='Album',
                           releaseId='42', sourceText='Genre: Dream pop', genre='Dream pop',
                           successReleaseResponse=dict(tracklist=[dict(title='First'), dict(title='Second')]))
            fixture['profile'] = 'assisted_without_ids'
            job = dict(inputs['jobs'][0], profile=fixture['profile'], scenario='assistance_failure',
                       supplemental=True, requireNoWrite=False)
            inputs['jobs'].append(job)
            inputs['syntheticGenreSource'] = fixture
            self.save(root / 'comparison-inputs.json', inputs)
            for arm in ['baseline', 'candidate']:
                primary = root / arm / 'folder_filename' / 'case'
                supplemental = root / arm / 'assisted_without_ids' / 'case' / 'supplemental' / 'assistance_failure'
                supplemental.mkdir(parents=True)
                for name in ['execution.json', 'source-hashes-before.json', 'source-hashes-after.json']:
                    (supplemental / name).write_bytes((primary / name).read_bytes())
                execution = json.loads((supplemental / 'execution.json').read_text())
                execution['runId'] = f'{arm}-assisted_without_ids-case-assistance_failure'
                execution['artifactDirectory'] = (
                    f'{arm}/assisted_without_ids/case/supplemental/assistance_failure')
                execution['scenario'] = 'assistance_failure'
                self.save(supplemental / 'execution.json', execution)
                records = []
                for phase in ['cold', 'warm']:
                    record = json.loads((primary / f'{phase}.jsonl').read_text())
                    record['profile'] = 'assisted_without_ids'
                    record['runId'] = f'{arm}-assisted_without_ids-case-assistance_failure'
                    if arm == 'candidate':
                        record['scenario'] = 'assistance_failure'
                    else:
                        record.pop('scenario', None)
                    record['native']['written'] = 2
                    record['outcome'] = 'applied'
                    records.append(json.dumps(record))
                (supplemental / 'cold.jsonl').write_text(records[0] + '\n')
                (supplemental / 'warm.jsonl').write_text(records[1] + '\n')
                payloads = []
                payload_class = 'title_only_baseline' if arm == 'baseline' else 'synthetic_discogs_source'
                for _ in range(2):
                    payload = dict(artist='Artist', album='Album', tracks=['First', 'Second'])
                    if arm == 'candidate':
                        body_sha = hashlib.sha256(json.dumps(fixture['successReleaseResponse'],
                            ensure_ascii=False, separators=(',', ':')).encode()).hexdigest()
                        payload['sourceObservation'] = dict(origin='synthetic_loopback', provider='discogs',
                            resourceId='42', responseSha256=body_sha, selector='/notes', text='Genre: Dream pop',
                            trackMappingSha256='b' * 64)
                    payloads.append(dict(payload=payload))
                self.save(supplemental / 'inventory.json', dict(misses=[],
                    modelRequests=[dict(status=400, payloadClass=payload_class)] * 2,
                    modelPayloads=payloads))
            report = self.report(root, expected_exit=0)
            self.assertEqual(report['requestedEvaluationsPerImplementation'], 4)
            self.assertEqual(report['baseline']['assistanceFailureControls'], 2)
            self.assertEqual(report['candidate']['assistanceFailureControls'], 2)
            self.assertEqual(report['baseline']['passedAssistanceFailureControls'], 2)
            self.assertEqual(report['candidate']['passedAssistanceFailureControls'], 2)
            for arm in ['baseline', 'candidate']:
                failed_rows = [row for row in report['rows'][arm] if row.get('scenario') == 'assistance_failure']
                self.assertEqual(len(failed_rows), 2)
                self.assertTrue(all(row['written'] == 2 and not row['unavailable'] for row in failed_rows))

            baseline_supplemental = root / 'baseline' / 'assisted_without_ids' / 'case' / 'supplemental' / 'assistance_failure'
            baseline_cold = baseline_supplemental / 'cold.jsonl'
            baseline_record = json.loads(baseline_cold.read_text())
            baseline_record['runId'] = 'baseline-assisted_without_ids-other-assistance_failure'
            self.save(baseline_cold, baseline_record)
            bad_run_id = self.report(root, expected_exit=1)
            baseline_row = next(row for row in bad_run_id['rows']['baseline']
                                if row.get('scenario') == 'assistance_failure' and row['phase'] == 'cold')
            self.assertTrue(baseline_row['unavailable'])
            self.assertEqual(baseline_row['reason'], 'native run ID does not match the runner-owned job')

            baseline_record['runId'] = 'baseline-assisted_without_ids-case-assistance_failure'
            self.save(baseline_cold, baseline_record)
            execution_file = baseline_supplemental / 'execution.json'
            execution = json.loads(execution_file.read_text())
            execution['artifactDirectory'] = 'baseline/other-case/supplemental/assistance_failure'
            self.save(execution_file, execution)
            bad_directory = self.report(root, expected_exit=1)
            baseline_rows = [row for row in bad_directory['rows']['baseline']
                             if row.get('scenario') == 'assistance_failure']
            self.assertTrue(all(row['unavailable'] for row in baseline_rows))
            self.assertTrue(all(any('execution context does not bind' in message for message in row['safetyFailures'])
                                for row in baseline_rows))

            execution['artifactDirectory'] = 'baseline/assisted_without_ids/case/supplemental/assistance_failure'
            self.save(execution_file, execution)
            candidate_cold = (root / 'candidate' / 'assisted_without_ids' / 'case' / 'supplemental'
                              / 'assistance_failure' / 'cold.jsonl')
            candidate_record = json.loads(candidate_cold.read_text())
            candidate_record.pop('scenario')
            self.save(candidate_cold, candidate_record)
            missing_candidate_scenario = self.report(root, expected_exit=1)
            candidate_row = next(row for row in missing_candidate_scenario['rows']['candidate']
                                 if row.get('scenario') == 'assistance_failure' and row['phase'] == 'cold')
            self.assertTrue(candidate_row['unavailable'])
            self.assertEqual(candidate_row['reason'], 'missing, duplicate or misbound native phase')

    def test_cached_source_withholding_checks_readback_and_obligations(self):
        path = '/tmp/warm/TV.flac'
        record = dict(beforeTracks=[dict(path=path, genre=None)],
                      readbackTracks=[dict(path=path, genre=None)],
                      fieldReviewObligationsAfterRestart={path: []}, native=dict(written=1))
        self.assertEqual(paired.cached_source_withholding_failures(record), [])
        for patch in [
            dict(readbackTracks=[dict(path=path, genre='Dream pop')]),
            dict(fieldReviewObligationsAfterRestart={path: [dict(field='genre')]}),
            dict(fieldReviewObligationsAfterRestart={'/unrelated/TV.flac': []}),
            *(dict(fieldReviewObligationsAfterRestart={path: item})
              for item in [None, {}, False, 0, '']),
            dict(native=dict(candidate=dict(assisted_genre={'value': 'Dream pop'}))),
            dict(serviceFailureObserved=True),
        ]:
            self.assertTrue(paired.cached_source_withholding_failures(record | patch))

    def test_required_candidate_fill_rejects_empty_readback_and_no_obligations(self):
        record = dict(phase='cold', caseId='case', native=dict(candidate=dict(assisted_genre=dict(
            value='Dream pop', source_observation={'evidenceId': 'b' * 64}))),
            beforeTracks=[dict(path='/tmp/01 Song.flac', genre=None)],
            readbackTracks=[dict(path='/tmp/01 Song.flac', genre=None)],
            fieldReviewObligationsAfterRestart={'/tmp/01 Song.flac': []})
        case = dict(tracks=[dict(relativePath='01 Song.flac')])
        failures, status = paired.field_obligation_observation(record, case, require_assisted_write=True)
        self.assertTrue(failures)
        self.assertEqual(status, 'unavailable')

    def test_reopened_obligation_requires_exact_file_before_and_source_proposal_binding(self):
        canonical = lambda value: json.dumps(value, ensure_ascii=False, sort_keys=True,
                                             separators=(',', ':')).encode()
        digest = lambda value: hashlib.sha256(value).hexdigest()
        text = 'Genre: Dream pop'
        source = dict(origin='synthetic_loopback', provider='discogs', resourceId='42',
                      responseSha256='a' * 64, selector='/notes', textSha256=digest(text.encode()),
                      trackMappingSha256='b' * 64, text=text)
        source['evidenceId'] = digest(canonical({key: source[key] for key in
            ['provider', 'resourceId', 'responseSha256', 'selector', 'textSha256', 'trackMappingSha256']}))
        request_input = dict(artist='Artist', album='Album', tracks=['Song'], sourceObservation=source)
        input_sha = digest(canonical(request_input))
        raw_value = applied_value = 'Dream pop'
        proposal = dict(rawValue=raw_value, appliedValue=applied_value, requestSha256=input_sha,
                        source={key: source[key] for key in
                            ['provider', 'resourceId', 'responseSha256', 'selector', 'textSha256',
                             'trackMappingSha256', 'evidenceId']})
        proposal_sha = digest(canonical(proposal))
        path = '/tmp/eval/case/01 Song.flac'
        assisted = dict(value=applied_value, raw_value=raw_value, model='scripted/genre-v1',
                        input_sha256=input_sha, request_input=request_input,
                        proposal_sha256=proposal_sha, source_observation=source)
        obligation = dict(path=path, field='genre', revision='cold-case', before=None, after=applied_value,
            provenance=dict(kind='assisted_inference', grounded=False, sourceMatch='exact_phrase_in_source_text',
                sourceObservation=source, proposalSha256=proposal_sha, model='scripted/genre-v1',
                rawValue=raw_value, appliedValue=applied_value, requestInput=request_input,
                requestInputSha256=input_sha), evidence_ids=[source['evidenceId']], state='open')
        record = dict(phase='cold', caseId='case', native=dict(candidate=dict(assisted_genre=assisted)),
            beforeTracks=[dict(path=path, genre=None)],
            readbackTracks=[dict(path=path, genre=applied_value)],
            genreSourceBindingEvidence=dict(releaseId='42', mappingSha256='b' * 64),
            fieldReviewObligationsAfterRestart={path: [obligation]})
        case = dict(tracks=[dict(relativePath='01 Song.flac')])
        self.assertEqual(paired.field_obligation_observation(record, case, require_assisted_write=True),
                         ([], 'open_per_file_obligations'))

        mutations = {
            'unrelated_path': lambda value: value['fieldReviewObligationsAfterRestart'].__setitem__(
                '/tmp/elsewhere/01 Song.flac', value['fieldReviewObligationsAfterRestart'].pop(path)),
            'different_readback_path': lambda value: value['readbackTracks'][0].update(
                path='/tmp/elsewhere/01 Song.flac'),
            'wrong_before_value': lambda value: value['fieldReviewObligationsAfterRestart'][path][0].update(
                before='Jazz'),
            'wrong_source_id': lambda value: value['native']['candidate']['assisted_genre']['source_observation'].update(
                resourceId='43'),
            'wrong_provenance': lambda value: value['fieldReviewObligationsAfterRestart'][path][0][
                'provenance']['sourceObservation'].update(resourceId='43'),
            'wrong_proposal_id': lambda value: value['fieldReviewObligationsAfterRestart'][path][0][
                'provenance'].update(proposalSha256='c' * 64),
            'wrong_evidence_id': lambda value: value['fieldReviewObligationsAfterRestart'][path][0].update(
                evidence_ids=['d' * 64]),
        }
        for name, mutate in mutations.items():
            with self.subTest(mutation=name):
                changed = json.loads(json.dumps(record))
                mutate(changed)
                failures, status = paired.field_obligation_observation(
                    changed, case, require_assisted_write=True)
                self.assertTrue(failures, name)
                self.assertEqual(status, 'unavailable', name)

    def test_provider_precedence_control_checks_every_genre_and_forbids_assistance(self):
        fixture = json.loads((Path(__file__).with_name('synthetic-genre-source.json')).read_text())
        body = fixture['providerPrecedenceReleaseResponse']
        genres = list(dict.fromkeys(value.strip() for value in body['genres'] + body['styles'] if value.strip()))
        expected_genre = ', '.join(genres)
        self.assertEqual(expected_genre, 'Pop, Acoustic, Alt-Pop')
        relative_files = ['01. Billie Eilish - TV.flac', '02. Billie Eilish - The 30th.flac']
        titles = ['TV', 'The 30th']
        case = dict(artist='Billie Eilish', tracks=[dict(relativePath=file) for file in relative_files])
        truth = dict(mapping=[dict(localTrack=i, localTitle=title, providerTitle=title,
                                   providerTrack=str(i)) for i, title in enumerate(titles, 1)])
        review = dict(releaseId=fixture['releaseId'], hardNegativeIds=[])
        expected = dict(album='Guitar Songs', credits=[['Billie Eilish'], ['Billie Eilish']])
        file_hashes = {file: dict(bytes=100, fullSha256='a' * 64, payloadSha256='p' * 64)
                       for file in relative_files}
        after_hashes = {file: dict(bytes=100, fullSha256='b' * 64, payloadSha256='p' * 64)
                        for file in relative_files}
        tracks = [dict(path=f'/tmp/eval/{file}', title=title, artist='Billie Eilish',
                       artists=['Billie Eilish'], album='Guitar Songs', albumArtist='Billie Eilish',
                       trackNumber=i, trackTotal=2, discNumber=1, genre=expected_genre)
                  for i, (file, title) in enumerate(zip(relative_files, titles), 1)]
        context = dict(caseId=fixture['caseId'], profile='tagged_recovery', phase='cold')
        record = dict(**context, outcome='applied', providerMode='offline_fixtures',
            mediaMode='synthetic_flac', readback=True, payloadUnchanged=True, written=2,
            beforeTracks=[dict(track, genre=None) for track in tracks], readbackTracks=tracks,
            profileBaseline=file_hashes, after=after_hashes, copiedBeforeProfile=file_hashes,
            native=dict(written=2, candidate=dict(discogs_release_id=fixture['releaseId'])),
            fieldReviewObligationsAfterRestart={track['path']: [] for track in tracks})
        inventory = dict(records=[dict(provider='discogs', path=fixture['route']['path'],
            query='[]', status=200, sha256=hashlib.sha256(json.dumps(body, ensure_ascii=False,
                separators=(',', ':')).encode()).hexdigest())],
            requests=[dict(provider='discogs', path=fixture['route']['path'], query=[], status=200)],
            modelRequests=[], modelPayloads=[], wireArrivals=[])

        def audit(mutated=record, observed=inventory):
            return paired.audit_phase(context, [mutated], case, truth, review, expected,
                source_fixture=fixture, inventory=observed, require_no_assisted_obligation=True)

        valid = audit()
        self.assertEqual(valid['providerPrecedenceGenre'], expected_genre)
        self.assertEqual(valid['providerPrecedenceFailures'], [])
        self.assertEqual(valid['providerPrecedenceGenreFailures'], [])
        self.assertEqual(valid['providerPrecedenceUnavailable'], [])
        self.assertTrue(valid['providerPrecedenceControl'])
        self.assertEqual(valid['recovery'], 'correct')
        self.assertFalse(valid['unavailable'])
        wrong_genre_row = None
        for index in range(len(tracks)):
            for value in ['WRONG PROVIDER GENRE', None]:
                changed = json.loads(json.dumps(record))
                changed['readbackTracks'][index]['genre'] = value
                row = audit(changed)
                self.assertTrue(row['providerPrecedenceFailures'], (index, value))
                self.assertTrue(row['providerPrecedenceGenreFailures'], (index, value))
                self.assertEqual(row['recovery'], 'wrong_match', (index, value))
                self.assertFalse(row['unavailable'], (index, value))
                if wrong_genre_row is None:
                    wrong_genre_row = row
        summary = paired.summarize([valid], [wrong_genre_row])
        self.assertEqual(summary['candidate']['wrongMatches'], 1)
        self.assertEqual(summary['candidate']['providerPrecedenceGenreFailures'], 1)
        self.assertEqual(summary['candidate']['unavailable'], 0)
        assisted = json.loads(json.dumps(record))
        assisted['native']['candidate']['assisted_genre'] = dict(value='Dream pop')
        assisted_row = audit(assisted)
        self.assertTrue(assisted_row['providerPrecedenceFailures'])
        self.assertTrue(assisted_row['providerPrecedenceSafetyFailures'])
        self.assertFalse(assisted_row['unavailable'])
        obligation = json.loads(json.dumps(record))
        obligation['fieldReviewObligationsAfterRestart'] = {
            track['path']: [dict(field='genre', revision='cold-case', state='open')]
            for track in tracks}
        obligation_row = audit(obligation)
        self.assertTrue(obligation_row['providerPrecedenceFailures'])
        self.assertTrue(obligation_row['providerPrecedenceSafetyFailures'])
        self.assertFalse(obligation_row['unavailable'])
        unauthorized_model = dict(inventory, modelRequests=[dict(status=501)],
                                  wireArrivals=[dict(method='POST', route='/model/chat/completions')])
        unauthorized_row = audit(observed=unauthorized_model)
        self.assertTrue(unauthorized_row['providerPrecedenceFailures'])
        self.assertTrue(unauthorized_row['providerPrecedenceSafetyFailures'])
        self.assertFalse(unauthorized_row['unavailable'])
        missing_readback = json.loads(json.dumps(record))
        missing_readback.pop('readbackTracks')
        unavailable_row = audit(missing_readback)
        self.assertTrue(unavailable_row['providerPrecedenceUnavailable'])
        self.assertFalse(unavailable_row['providerPrecedenceGenreFailures'])
        self.assertTrue(unavailable_row['unavailable'])

    def test_phase_report_retains_raw_readback_mapping_and_reopened_obligation(self):
        context = dict(caseId='case', profile='folder_filename', phase='cold')
        record = dict(**context, outcome='needs_review', providerMode='offline_fixtures',
                      mediaMode='synthetic_flac', readback=True, payloadUnchanged=True,
                      beforeTracks=[dict(path='/tmp/First.flac', genre=None)],
                      readbackTracks=[dict(path='/tmp/First.flac', genre='Dream pop')],
                      profileBaseline={}, after={}, copiedBeforeProfile={}, native=dict(written=0),
                      genreSourceBindingEvidence={'mappingSha256': 'a' * 64},
                      fieldReviewObligationsAfterRestart={'/tmp/First.flac': [dict(field='genre', state='open')]})
        row = paired.audit_phase(context, [record], {}, {}, {}, {})
        self.assertEqual(row['rawIndependentMetadataReadback'], record['readbackTracks'])
        self.assertEqual(row['sourceMappingEvidence'], record['genreSourceBindingEvidence'])
        self.assertEqual(row['reopenedFieldReviewObligations'], record['fieldReviewObligationsAfterRestart'])
        self.assertEqual(row['jobScriptedModelPayloads'], [])

    def prepare_report(self, root, written=0):
        tracks = [dict(path=f'/tmp/{name}.flac', title=name, artist='Artist', artists=['Artist'],
                       album='Album', albumArtist='Artist', trackNumber=i, trackTotal=2, discNumber=1)
                  for i, name in enumerate(['First', 'Second'], 1)]
        hashes = {Path(t['path']).name: dict(payloadSha256=t['title'], fullSha256=t['title']) for t in tracks}
        job = dict(caseId='case', profile='folder_filename', requireNoWrite=not written,
                   case=dict(artist='Artist', tracks=[dict(relativePath=Path(t['path']).name) for t in tracks]),
                   truth=dict(mapping=[dict(localTrack=i, localTitle=t['title'], providerTitle=t['title'],
                                           providerTrack=str(i)) for i, t in enumerate(tracks, 1)]),
                   review=dict(provider='musicbrainz', releaseId='reviewed'),
                   expected=dict(album='Album', credits=[['Artist'], ['Artist']]))
        self.save(root / 'comparison-inputs.json', dict(jobs=[job], profiles=['folder_filename'],
                  setupFailures=[], locks={}, implementations={}, budget={}))
        for arm in ['baseline', 'candidate']:
            directory = root / arm / 'folder_filename' / 'case'
            directory.mkdir(parents=True)
            self.save(directory / 'execution.json', dict(exitCode=0, signal=None))
            self.save(directory / 'inventory.json', dict(misses=[], modelRequests=[]))
            for file in ['source-hashes-before.json', 'source-hashes-after.json']:
                self.save(directory / file, {'case': {name: dict(bytes=100, fullSha256='a' * 64) for name in hashes}})
            for phase in ['cold', 'warm']:
                self.save(directory / f'{phase}.jsonl', dict(caseId='case', profile='folder_filename', phase=phase,
                          outcome='applied' if written else 'needs_review',
                          providerMode='offline_fixtures', mediaMode='synthetic_flac',
                          readback=True, payloadUnchanged=True, readbackTracks=tracks, beforeTracks=tracks,
                          profileBaseline=hashes, after=hashes, copiedBeforeProfile=hashes,
                          cacheBefore={}, cacheAfter={}, native=dict(written=written,
                              candidate=dict(musicbrainz_albumid='reviewed'))))

    def save(self, file, value):
        file.write_text(json.dumps(value))

    def report(self, root, expected_exit=1):
        result = subprocess.run([sys.executable, '-B', str(Path(spec.origin)), str(root)], capture_output=True, text=True)
        self.assertTrue((root / 'comparison.json').exists(), result.stdout + result.stderr)
        self.assertTrue((root / 'comparison.md').exists(), result.stdout + result.stderr)
        self.assertEqual(result.returncode, expected_exit, result.stdout + result.stderr)
        return json.loads((root / 'comparison.json').read_text())

    def test_no_write_requires_complete_bound_physical_exports(self):
        for mutation in ['omit', 'omit_tracks', 'omit_hashes', 'duplicate_tracks', 'substitute_tracks', 'substitute_hashes']:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                self.prepare_report(root)
                self.report(root, expected_exit=0)
                for file in (root / 'candidate').rglob('*.jsonl'):
                    record = json.loads(file.read_text())
                    for key in ['beforeTracks', 'readbackTracks']:
                        if mutation in ['omit', 'omit_tracks']:
                            record[key].pop()
                        elif mutation == 'duplicate_tracks':
                            record[key][1] = record[key][0]
                        elif mutation == 'substitute_tracks':
                            record[key][1]['path'] = '/tmp/Foreign.flac'
                    if mutation in ['omit', 'omit_hashes', 'substitute_hashes']:
                        for key in ['profileBaseline', 'after', 'copiedBeforeProfile']:
                            value = record[key].pop('Second.flac')
                            if mutation == 'substitute_hashes':
                                record[key]['Foreign.flac'] = value
                    self.save(file, record)
                report = self.report(root)
                self.assertEqual(report['requestedEvaluationsPerImplementation'], 2)
                self.assertEqual(report['candidate']['passedNoWriteControls'], 0)
                self.assertEqual(report['candidate']['safetyFailures'], 2)

    def test_malformed_ancillary_exports_keep_reports_and_denominator(self):
        for name in ['execution.json', 'inventory.json', 'source-hashes-before.json', 'source-hashes-after.json']:
            for content in ['{', '[]']:
                with self.subTest(file=name, content=content), tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    self.prepare_report(root)
                    (root / 'candidate' / 'folder_filename' / 'case' / name).write_text(content)
                    report = self.report(root)
                    self.assertEqual(report['requestedEvaluationsPerImplementation'], 2)
                    self.assertEqual(len(report['rows']['candidate']), 2)
                    self.assertEqual(report['candidate']['unavailable'], 2)
                    self.assertEqual(report['candidate']['passedNoWriteControls'], 0)
                    self.assertFalse(report['comparable'])

    def test_null_readback_retains_known_prohibited_writes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.prepare_report(root)
            directory = root / 'candidate' / 'folder_filename' / 'case'
            for file in directory.glob('*.jsonl'):
                record = json.loads(file.read_text())
                record.update(readbackTracks=None, readback=False, native=dict(written=2))
                self.save(file, record)
            self.save(directory / 'execution.json', dict(exitCode=1, signal=None))
            report = self.report(root)
            self.assertEqual(report['requestedEvaluationsPerImplementation'], 2)
            for key in ['wrongMatches', 'unavailable', 'processFailures', 'safetyFailures', 'readbackFailures']:
                self.assertEqual(report['candidate'][key], 2, key)
            self.assertEqual(report['candidate']['correctRecoveries'], 0)

    def test_readback_diagnostics_are_not_observed_wrong_content(self):
        for evidence, wrong_count in [('null', 0), ('partial_null', 0), ('count', 0), ('native_count', 0),
                                      ('binding', 0), ('count_wrong_title', 2), ('null_bad_candidate', 2)]:
            with self.subTest(evidence=evidence), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                self.prepare_report(root, written=2)
                inputs = json.loads((root / 'comparison-inputs.json').read_text())
                inputs['jobs'][0]['review']['hardNegativeIds'] = ['excluded-release']
                self.save(root / 'comparison-inputs.json', inputs)
                for file in (root / 'candidate').rglob('*.jsonl'):
                    record = json.loads(file.read_text())
                    record['readback'] = False
                    if evidence in ['null', 'partial_null', 'null_bad_candidate']:
                        record['readbackTracks'] = None
                    elif evidence == 'binding':
                        record['readbackTracks'][1].update(path='/tmp/Foreign.flac', title='Wrong file')
                    elif evidence == 'native_count':
                        record['native']['written'] = 1
                    else:
                        record['readbackTracks'].pop()
                        if evidence == 'count_wrong_title':
                            record['readbackTracks'][0]['title'] = 'Wrong performance'
                    if evidence == 'partial_null':
                        record.update(native=None, outcome='resolver_error', error='partial write failure')
                        record['after']['First.flac']['fullSha256'] = 'changed'
                    elif evidence == 'null_bad_candidate':
                        record['native']['candidate']['musicbrainz_albumid'] = 'excluded-release'
                    self.save(file, record)
                report = self.report(root)
                self.assertEqual(report['candidate']['wrongMatches'], wrong_count)
                self.assertEqual(report['candidate']['unavailable'], 2)
                self.assertEqual(report['candidate']['readbackFailures'], 2)
                self.assertEqual(report['candidate']['correctRecoveries'], 0)
                self.assertEqual(report['requestedEvaluationsPerImplementation'], 2)

    def test_partial_error_checks_readback_release_ids_only_on_written_files(self):
        for field, excluded in [('musicbrainzAlbumId', '6884af9c-40e3-4553-ad1b-1fff4db194b5'),
                                ('discogsReleaseId', '1234')]:
            for evidence, wrong_count in [('changed', 2), ('unchanged', 0), ('allowed', 0), ('native_success', 2)]:
                with self.subTest(field=field, evidence=evidence), tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    self.prepare_report(root, written=2)
                    inputs = json.loads((root / 'comparison-inputs.json').read_text())
                    inputs['jobs'][0]['review']['hardNegativeIds'] = [excluded]
                    self.save(root / 'comparison-inputs.json', inputs)
                    directory = root / 'candidate' / 'folder_filename' / 'case'
                    for file in directory.glob('*.jsonl'):
                        record = json.loads(file.read_text())
                        if evidence != 'native_success':
                            record.update(native=None, outcome='resolver_error', error='partial write failure')
                        record['after']['First.flac']['fullSha256'] = 'changed'
                        index = 1 if evidence == 'unchanged' else 0
                        record['readbackTracks'][index][field] = 'allowed-release' if evidence == 'allowed' else excluded
                        if evidence == 'unchanged':
                            record['beforeTracks'][index][field] = excluded
                        self.save(file, record)
                    if evidence != 'native_success':
                        self.save(directory / 'execution.json', dict(exitCode=1, signal=None))
                    report = self.report(root)
                    self.assertEqual(report['candidate']['wrongMatches'], wrong_count)
                    self.assertEqual(report['candidate']['readbackFailures'], wrong_count)
                    self.assertEqual(report['candidate']['unavailable'], 0 if evidence == 'native_success' else 2)
                    self.assertEqual(report['candidate']['processFailures'], 0 if evidence == 'native_success' else 2)
                    self.assertEqual(report['candidate']['correctRecoveries'], 0)

    def test_partial_error_grades_changed_files_not_unchanged_bad_input(self):
        for changed, wrong, prohibited in [(True, True, False), (False, True, False),
                                            (True, False, False), (True, False, True)]:
            with self.subTest(changed=changed, wrong=wrong, prohibited=prohibited), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                self.prepare_report(root, written=0 if prohibited else 2)
                directory = root / 'candidate' / 'folder_filename' / 'case'
                for file in directory.glob('*.jsonl'):
                    record = json.loads(file.read_text())
                    record.update(native=None, outcome='resolver_error', error='partial write failure')
                    # Both tracks start wrong; only the first may have been written.
                    for track in record['beforeTracks']:
                        track['title'] = 'Old bad input'
                    record['readbackTracks'][0]['title'] = 'Wrong performance' if wrong else 'First'
                    record['readbackTracks'][1]['title'] = 'Old bad input'
                    if changed:
                        record['after']['First.flac']['fullSha256'] = 'changed'
                    else:
                        record['beforeTracks'][0]['title'] = record['readbackTracks'][0]['title']
                    self.save(file, record)
                self.save(directory / 'execution.json', dict(exitCode=1, signal=None))
                report = self.report(root)
                self.assertEqual(report['candidate']['wrongMatches'], 2 if changed and (wrong or prohibited) else 0)
                self.assertEqual(report['candidate']['readbackFailures'], 2 if changed and wrong else 0)
                self.assertEqual(report['candidate']['unavailable'], 2)
                self.assertEqual(report['candidate']['correctRecoveries'], 0)

    def test_zero_write_claim_cannot_hide_changed_correct_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.prepare_report(root, written=2)
            directory = root / 'candidate' / 'folder_filename' / 'case'
            for file in directory.glob('*.jsonl'):
                record = json.loads(file.read_text())
                record['native']['written'] = 0
                record['after']['First.flac']['fullSha256'] = 'changed'
                self.save(file, record)
            report = self.report(root)
            self.assertEqual(report['candidate']['safetyFailures'], 2)
            self.assertEqual(report['candidate']['unavailable'], 2)
            self.assertEqual(report['candidate']['correctRecoveries'], 0)
            self.assertEqual(report['candidate']['wrongMatches'], 0)

    def test_nested_ancillary_validation_retains_unavailable_rows(self):
        invalid = [('inventory.json', value) for value in [
            {'misses': None, 'modelRequests': []}, {'misses': [None], 'modelRequests': []},
            {'misses': [{'count': '1'}], 'modelRequests': []},
            {'misses': [{'count': -1}], 'modelRequests': []},
            {'misses': [{'count': None}], 'modelRequests': []},
            {'misses': [{'count': True}], 'modelRequests': []}, {'misses': [], 'modelRequests': None}]]
        invalid += [('source-hashes-before.json', {'case': value}) for value in [None, {},
            {'First.flac': {'bytes': 1, 'fullSha256': 'bad'}}]]
        invalid += [('execution.json', {'exitCode': '0', 'signal': None})]
        invalid += [('execution.json', {'exitCode': 0, 'signal': None, 'skipped': value})
                    for value in [[], {}, None, 0, 1, 'false']]
        for name, value in invalid:
            with self.subTest(name=name, value=value), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                self.prepare_report(root)
                directory = root / 'candidate' / 'folder_filename' / 'case'
                self.save(directory / name, value)
                if name.startswith('source-hashes'):
                    self.save(directory / 'source-hashes-after.json', value)
                report = self.report(root)
                self.assertEqual(len(report['rows']['candidate']), 2)
                self.assertEqual(report['candidate']['unavailable'], 2)
                self.assertEqual(report['candidate']['passedNoWriteControls'], 0)
                self.assertFalse(report['comparable'])
                if name == 'execution.json':
                    self.assertEqual(report['candidate']['processFailures'], 2)

    def test_countless_mock_miss_marks_slots_unavailable_without_invalidating_inventory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.prepare_report(root)
            directory = root / 'candidate' / 'folder_filename' / 'case'
            route = dict(provider='musicbrainz', path='/release/example',
                         query=[['fmt', 'json']], status=501)
            self.save(directory / 'inventory.json', dict(misses=[route], modelRequests=[]))
            report = self.report(root)
            self.assertEqual(report['candidate']['missingFixtures'], 2)
            self.assertEqual(report['candidate']['unavailable'], 2)
            self.assertEqual(report['candidate']['passedNoWriteControls'], 0)
            for row in report['rows']['candidate']:
                self.assertEqual(row['missingFixtures'], 1)
                self.assertEqual(row['safetyFailures'], [])

    def test_boolean_skipped_status_keeps_its_declared_meaning(self):
        for skipped in [False, True]:
            with self.subTest(skipped=skipped), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                self.prepare_report(root)
                self.save(root / 'candidate' / 'folder_filename' / 'case' / 'execution.json',
                          dict(exitCode=0, signal=None, skipped=skipped))
                report = self.report(root, expected_exit=1 if skipped else 0)
                self.assertEqual(report['candidate']['skipped'], 2 if skipped else 0)
                self.assertEqual(report['candidate']['unavailable'], 2 if skipped else 0)
                self.assertEqual(report['candidate']['passedNoWriteControls'], 0 if skipped else 2)

    def test_process_or_fixture_failure_cannot_erase_observed_wrong_writes(self):
        for failure in ['process', 'fixture']:
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                self.prepare_report(root, written=2)
                directory = root / 'candidate' / 'folder_filename' / 'case'
                for file in directory.glob('*.jsonl'):
                    record = json.loads(file.read_text())
                    record['readbackTracks'][1]['title'] = 'Wrong performance'
                    self.save(file, record)
                if failure == 'process':
                    self.save(directory / 'execution.json', dict(exitCode=1, signal=None))
                else:
                    self.save(directory / 'inventory.json', dict(misses=[dict(count=1)], modelRequests=[]))
                report = self.report(root)
                self.assertEqual(report['candidate']['wrongMatches'], 2)
                self.assertEqual(report['candidate']['unavailable'], 2)
                self.assertEqual(report['candidate']['correctRecoveries'], 0)
                self.assertEqual(report['candidate']['readbackFailures'], 2)

    def row(self, outcome='abstention', **changes):
        result = dict(caseId='case', profile='folder_filename', phase='cold',
                    recovery=outcome, readbackFailures=[], safetyFailures=[],
                    missingFixtures=0, processFailure=False, skipped=False,
                    unavailable=False)
        return result | changes

    def test_recovery_is_separate_from_safety_and_regressions(self):
        before = [self.row(), {**self.row('correct'), 'phase': 'warm'}]
        after = [self.row('correct', safetyFailures=['payload']),
                 {**self.row('wrong_match'), 'phase': 'warm'}]
        result = paired.summarize(before, after)
        self.assertEqual(result['requestedEvaluationsPerImplementation'], 2)
        self.assertEqual(result['improvements'], 1)
        self.assertEqual(result['regressions'], 1)
        self.assertEqual(result['candidate']['wrongMatches'], 1)
        self.assertEqual(result['candidate']['safetyFailures'], 1)
        self.assertFalse(result['clean'])

    def test_incomplete_cannot_earn_improvement_credit(self):
        before = [self.row(unavailable=True, missingFixtures=1)]
        after = [self.row('correct')]
        result = paired.summarize(before, after)
        self.assertEqual(result['improvements'], 0)
        self.assertFalse(result['comparable'])
        self.assertEqual(result['baseline']['missingFixtures'], 1)

    def test_missing_and_duplicate_native_records_keep_requested_slot(self):
        context = dict(caseId='case', profile='folder_filename', phase='cold')
        for records in [[], [context, context]]:
            result = paired.audit_phase(context, records, {}, {}, {}, {})
            self.assertTrue(result['unavailable'])
            self.assertEqual(result['recovery'], 'unavailable')

    def test_wrong_content_is_not_rescued_by_native_success(self):
        context = dict(caseId='case', profile='folder_filename', phase='cold')
        case = dict(artist='Artist', tracks=[dict(relativePath='song.flac')])
        truth = dict(mapping=[dict(localTrack=1, localTitle='Song (Remix)', providerTitle='Song (Remix)', providerTrack='1')])
        review = dict(provider='musicbrainz', releaseId='reviewed', hardNegativeIds=['wrong'])
        track = dict(path='/tmp/song.flac', title='Song', artist='Artist', artists=['Artist'],
                     album='Album', albumArtist='Artist', trackNumber=1, trackTotal=1,
                     discNumber=1, musicbrainzAlbumId='wrong')
        record = dict(**context, outcome='applied', providerMode='offline_fixtures', mediaMode='synthetic_flac',
                      readback=True, payloadUnchanged=True, readbackTracks=[track], beforeTracks=[track],
                      cacheBefore={}, cacheAfter={}, profileBaseline={}, after={}, copiedBeforeProfile={},
                      native=dict(written=1, candidate=dict(source='musicbrainz', musicbrainz_albumid='wrong')))
        result = paired.audit_phase(context, [record], case, truth, review,
                                    dict(album='Album', credits=[['Artist']]))
        self.assertEqual(result['recovery'], 'wrong_match')
        self.assertTrue(result['readbackFailures'])

    def test_disc_fallback_does_not_authorize_overwriting_existing_disc(self):
        context = dict(caseId='case', profile='folder_filename', phase='cold')
        case = dict(artist='Artist', tracks=[dict(relativePath='song.flac')])
        truth = dict(mapping=[dict(localTrack=1, localTitle='Song', providerTitle='Song', providerTrack='1')])
        review = dict(provider='musicbrainz', releaseId='reviewed')
        track = dict(path='/tmp/song.flac', title='Song', artist='Artist', artists=['Artist'],
                     album='Album', albumArtist='Artist', trackNumber=1, trackTotal=1, discNumber=1)
        hashes = {'song.flac': dict(payloadSha256='payload', fullSha256='before')}
        record = dict(**context, outcome='applied', providerMode='offline_fixtures', mediaMode='synthetic_flac',
                      readback=True, payloadUnchanged=True, readbackTracks=[track],
                      beforeTracks=[{**track, 'discNumber': None}],
                      profileBaseline=hashes, after=hashes, copiedBeforeProfile=hashes,
                      native=dict(written=1, candidate=dict(source='musicbrainz', musicbrainz_albumid='reviewed')))
        expected = dict(album='Album', credits=[['Artist']])
        result = paired.audit_phase(context, [record], case, truth, review, expected)
        self.assertEqual(result['safetyFailures'], [])
        record['beforeTracks'][0]['discNumber'] = 2
        result = paired.audit_phase(context, [record], case, truth, review, expected)
        self.assertIn('1: changed disc positions', result['safetyFailures'])

    def test_unavailable_provider_is_not_an_abstention(self):
        context = dict(caseId='case', profile='folder_filename', phase='cold')
        record = dict(**context, outcome='needs_review', providerMode='offline_fixtures', mediaMode='synthetic_flac',
                      readback=True, payloadUnchanged=True, readbackTracks=[], beforeTracks=[],
                      profileBaseline={}, after={}, serviceFailureObserved=True)
        result = paired.audit_phase(context, [record], {}, {}, {}, {})
        self.assertTrue(result['unavailable'])
        self.assertEqual(result['recovery'], 'unavailable')

    def test_required_no_write_control_rejects_content_correct_writes_in_cli_report(self):
        case = dict(artist='Artist', tracks=[dict(relativePath='song.flac')])
        truth = dict(mapping=[dict(localTrack=1, localTitle='Song', providerTitle='Song', providerTrack='1')])
        review = dict(provider='musicbrainz', releaseId='reviewed')
        track = dict(path='/tmp/song.flac', title='Song', artist='Artist', artists=['Artist'],
                     album='Album', albumArtist='Artist', trackNumber=1, trackTotal=1, discNumber=1)
        hashes = {'song.flac': dict(payloadSha256='payload', fullSha256='unchanged')}
        job = dict(caseId='case', profile='folder_filename', case=case, truth=truth,
                   review=review, expected=dict(album='Album', credits=[['Artist']]), requireNoWrite=True)
        with tempfile.TemporaryDirectory(prefix='paired-no-write-') as temporary:
            root = Path(temporary)
            def save(file, value):
                file.write_text(json.dumps(value))
            save(root / 'comparison-inputs.json', dict(jobs=[job], profiles=['folder_filename'],
                 setupFailures=[], locks={}, implementations={}, budget={}))
            for arm in ['baseline', 'candidate']:
                directory = root / arm / 'folder_filename' / 'case'
                directory.mkdir(parents=True)
                save(directory / 'execution.json', dict(exitCode=0, signal=None))
                save(directory / 'inventory.json', dict(misses=[], modelRequests=[]))
                for file in ['source-hashes-before.json', 'source-hashes-after.json']:
                    save(directory / file, {'case': {'song.flac': dict(bytes=100, fullSha256='a' * 64)}})
                for phase in ['cold', 'warm']:
                    save(directory / f'{phase}.jsonl', dict(caseId='case', profile='folder_filename', phase=phase,
                         outcome='needs_review' if arm == 'baseline' else 'applied',
                         providerMode='offline_fixtures', mediaMode='synthetic_flac',
                         readback=True, payloadUnchanged=True, readbackTracks=[track], beforeTracks=[track],
                         profileBaseline=hashes, after=hashes, copiedBeforeProfile=hashes,
                         cacheBefore={}, cacheAfter={}, native=dict(written=0 if arm == 'baseline' else 1,
                             candidate=dict(musicbrainz_albumid='reviewed'))))
            result = subprocess.run([sys.executable, '-B', str(Path(spec.origin)), str(root)], capture_output=True, text=True)
            report = json.loads((root / 'comparison.json').read_text())
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertEqual(report['requestedEvaluationsPerImplementation'], 2)
            self.assertEqual(report['candidate']['correctRecoveries'], 0)
            self.assertEqual(report['candidate']['wrongMatches'], 2)
            self.assertEqual(report['candidate']['safetyFailures'], 2)
            self.assertEqual(report['candidate']['readbackFailures'], 0)
            self.assertEqual(report['baseline']['passedNoWriteControls'], 2)
            self.assertEqual(report['candidate']['passedNoWriteControls'], 0)
            self.assertEqual(report['improvements'], 0)


if __name__ == '__main__':
    unittest.main()
