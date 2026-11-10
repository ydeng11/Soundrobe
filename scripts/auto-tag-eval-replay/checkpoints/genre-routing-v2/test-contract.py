"""Check the authorized routing expectation without invoking native code or HTTP."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
OLD = HERE.with_name('genre-ai-gaps-20261110')
NAME = 'hold-accepted-folder_filename-f7756354af37'
load = lambda path: json.loads(path.read_text())
spec = importlib.util.spec_from_file_location('corrected_audit', HERE / 'audit.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class RoutingContract(unittest.TestCase):
    def evidence(self, name=NAME, phase='cold'):
        job = copy.deepcopy(next(j for j in audit.frozen['jobs'] if j['name'] == name))
        directory = HERE / 'runs' / name
        record = load(directory / f'{phase}.jsonl')
        baseline = load(audit.ROOT / job['baseline'] / f'{phase}.jsonl')
        models = load(directory / f'requests-{phase}-{job["case"]["caseId"]}.json')['modelRequests']
        if phase == 'warm':
            cold = load(directory / f'requests-cold-{job["case"]["caseId"]}.json')['modelRequests']
            models = models[len(cold):]
        return record, job, baseline, models

    def test_incomplete_discovery_requires_no_fallback_in_both_phases(self):
        for phase in ['cold', 'warm']:
            with self.subTest(phase=phase):
                record, job, baseline, models = self.evidence(phase=phase)
                self.assertEqual(job['expectedModelRequests'], 0)
                self.assertEqual(models, [])
                self.assertEqual(audit.audit_record(record, job, baseline, models)[0], [])

    def test_unexpected_metadata_call_is_rejected(self):
        record, job, baseline, models = self.evidence()
        other = self.evidence('hold-accepted-folder_filename-0e7c8a578cea')[3]
        self.assertEqual(len(other), 1)
        errors, _ = audit.audit_record(record, job, baseline, other)
        self.assertIn('model_count', errors)

    def test_unintended_write_is_rejected(self):
        record, job, baseline, models = self.evidence()
        record['native']['written'] = 1
        self.assertIn('no_write_bytes_and_tags', audit.audit_record(record, job, baseline, models)[0])

    def test_unauthorized_fallback_candidate_is_rejected(self):
        record, job, baseline, models = self.evidence()
        record['native']['candidate'] = {'source': 'llm'}
        record['native']['authority'] = 'llm'
        self.assertIn('held_authority', audit.audit_record(record, job, baseline, models)[0])

    def test_complete_no_match_controls_still_require_metadata_assistance(self):
        names = [j['name'] for j in audit.frozen['jobs']
                 if j['kind'] == 'hold' and j['scenario'] == 'accepted' and j['name'] != NAME]
        self.assertEqual(len(names), 5)
        for name in names:
            for phase in ['cold', 'warm']:
                with self.subTest(name=name, phase=phase):
                    record, job, baseline, models = self.evidence(name, phase)
                    self.assertEqual(job['expectedModelRequests'], 2)
                    self.assertEqual([a['status'] for a in record['native']['providerAttempts']], ['no_match', 'no_match'])
                    self.assertEqual(len(models), 1)
                    self.assertEqual(audit.audit_record(record, job, baseline, models)[0], [])
                    self.assertIn('model_count', audit.audit_record(record, job, baseline, [])[0])

    def test_only_one_expectation_value_changes_and_script_is_untouched(self):
        old = load(OLD / 'freeze.json')
        expected = copy.deepcopy(old)
        job = next(j for j in expected['jobs'] if j['name'] == NAME)
        self.assertEqual(job['expectedModelRequests'], 2)
        job['expectedModelRequests'] = 0
        self.assertEqual(audit.frozen, expected)


if __name__ == '__main__':
    unittest.main()
