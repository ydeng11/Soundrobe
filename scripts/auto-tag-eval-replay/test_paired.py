"""Contract tests for fixed-denominator paired native reports."""
import importlib.util
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
            {'misses': [{'count': -1}], 'modelRequests': []}, {'misses': [], 'modelRequests': None}]]
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
