"""Deliberately corrupted in-memory evidence must not pass the independent audit."""
import copy
import importlib.util
import json
import pathlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('audit', HERE / 'audit.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)
load = lambda p: json.loads(p.read_text())


class NegativeControls(unittest.TestCase):
    def evidence(self, name, directory='pilot'):
        job = copy.deepcopy(next(j for j in audit.frozen['jobs'] if j['name'] == name))
        record = load(HERE / directory / name / 'cold.jsonl')
        baseline = load(audit.ROOT / job['baseline'] / 'cold.jsonl')
        models = load(HERE / directory / name / f'requests-cold-{job["case"]["caseId"]}.json')['modelRequests']
        self.assertEqual(audit.audit_record(record, job, baseline, models)[0], [])
        return record, job, baseline, models

    def accepted(self):
        return self.evidence('gap-accepted-assisted_without_ids-8452b6f2b3ea')

    def test_wrong_guest(self):
        record, job, baseline, models = self.accepted()
        record['readbackTracks'][0]['artists'] = ['Unapproved guest']
        self.assertIn('non_genre_readback', audit.audit_record(record, job, baseline, models)[0])

    def test_lost_persisted_uncertainty(self):
        record, job, baseline, models = self.accepted()
        record['persistedEvidence']['candidate'].pop('assisted_genre')
        record['reviewPendingAfterRestart'] = False
        errors, _ = audit.audit_record(record, job, baseline, models)
        self.assertIn('persisted_candidate', errors)
        self.assertIn('persisted_review', errors)

    def test_wrong_model_role(self):
        record, job, baseline, models = self.accepted()
        models[0]['body']['response_format']['json_schema']['name'] = 'TagCorrectionResponse'
        self.assertIn('model_role', audit.audit_record(record, job, baseline, models)[0])

    def test_overwritten_local_choice(self):
        record, job, baseline, models = self.evidence('mixed_local-accepted-assisted_without_ids-e8a8a8d6b27d', 'runs')
        self.assertEqual(record['beforeTracks'][0]['genre'], 'Explicit local folk')
        record['readbackTracks'][0]['genre'] = 'Offline test genre'
        self.assertIn('genre_readback', audit.audit_record(record, job, baseline, models)[0])


if __name__ == '__main__':
    unittest.main()
