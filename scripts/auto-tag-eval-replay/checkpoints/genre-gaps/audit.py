"""Independent actual-value/provenance audit; old disabled-only audit stays intact."""
import collections
import hashlib
import importlib.util
import json
import pathlib
import re
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]
load = lambda p: json.loads(p.read_text())
frozen = load(HERE / 'freeze.json')
PRIOR = ROOT / frozen['prior']
spec = importlib.util.spec_from_file_location('old_fields', PRIOR / 'field-audit.py')
old_fields = importlib.util.module_from_spec(spec)
spec.loader.exec_module(old_fields)


def stable(value):
    if isinstance(value, dict):
        return {k: stable(v) for k, v in value.items()}
    if isinstance(value, list):
        return [stable(v) for v in value]
    if isinstance(value, str):
        return re.sub(r'/private/tmp/soundrobe-auto-tag-eval-[^/]+', '$DISPOSABLE', value)
    return value


def receipts(diagnostics):
    return [r for d in diagnostics for r in [d] + d.get('fieldEvidence', [])
            if r.get('stage') in ['provider_field', 'provider_track_credits', 'local_track_preservation']]


def audit_record(record, job, baseline, models):
    errors, ledger = [], []
    def require(ok, code):
        if not ok:
            errors.append(code)
    native, old = record.get('native') or {}, baseline.get('native') or {}
    candidate, previous = native.get('candidate') or {}, old.get('candidate') or {}
    before, after = record['beforeTracks'], record['readbackTracks']
    inferred = job['expected'] == 'inferred_unverified'
    cancelled = job['expected'] == 'cancel_no_write'
    held = job['kind'] == 'hold'
    no_write = cancelled or held
    require(record['providerMode'] == 'offline_fixtures', 'live_provider')
    require(record['assistanceMode'] == ('disabled' if job['scenario'] == 'disabled' else 'scripted'), 'assistance_mode')
    require(record['payloadUnchanged'] and record['readback'], 'native_payload_readback')
    require(len(before) == len(after) == job['case']['tracks'], 'file_count')
    require([t['path'] for t in before] == [t['path'] for t in after], 'physical_binding')
    require(bool(record['cancellationRequested']) == cancelled, 'cancellation')
    require(len(models) == job['expectedModelRequests'] // 2, 'model_count')
    for model in models:
        body = model['body']
        user = [m for m in body['messages'] if m['role'] == 'user']
        payload = json.loads(user[0]['content'])
        if job['script']['schemaName'] == 'TagCorrectionResponse':
            payload['full_path'] = re.sub(r'^/private/tmp/soundrobe-auto-tag-eval-[^/]+/media/(cold|warm)/', '$EVAL_MEDIA/', payload['full_path'])
        require(payload == job['script']['payload'], 'exact_model_payload')
        require(body['model'] == 'scripted/genre-v1' and body['response_format']['json_schema']['name'] == job['script']['schemaName'], 'model_role')
        encoded = json.dumps(body, separators=(',', ':'), ensure_ascii=False)
        if job['script']['schemaName'] == 'TagCorrectionResponse' and body.get('temperature') == 0:
            # Rust's metadata request emits 0.0; the mock's parsed JSON stores 0.
            # Reconstruct only this source-defined lexical difference, not inputs.
            encoded = encoded.replace('"temperature":0}', '"temperature":0.0}')
        require(hashlib.sha256(encoded.encode()).hexdigest() == model['sha256'], 'model_raw_binding')
        require(model['status'] == job['script']['status'], 'model_status')
        if model['status'] == 200:
            require(json.loads(model['response']['choices'][0]['message']['content']) == job['script']['response'], 'exact_model_response')
    require(bool(candidate.get('assisted_genre')) == inferred, 'assisted_provenance')
    if no_write:
        require(not native.get('written') and before == after and record['profileBaseline'] == record['after'], 'no_write_bytes_and_tags')
        require(not candidate and not native.get('authority'), 'held_authority')
        require(record['actionableReviewAfterRestart'] == (not cancelled), 'held_review')
        if cancelled:
            require('cancel' in (record.get('error') or '').lower(), 'cancel_error')
    else:
        require(native.get('written') == len(before), 'supported_write_count')
        for field in ['authority', 'completionBasis', 'coreReceipts', 'proofVersion', 'providerAttempts']:
            require(stable(native.get(field)) == stable(old.get(field)), 'selected_' + field)
        require(stable({k:v for k,v in candidate.items() if k not in ['genre','assisted_genre','tracks']})
                == stable({k:v for k,v in previous.items() if k not in ['genre','assisted_genre','tracks']}), 'candidate_non_genre')
        require(stable([{k:v for k,v in t.items() if k!='genre'} for t in candidate.get('tracks',[])])
                == stable([{k:v for k,v in t.items() if k!='genre'} for t in previous.get('tracks',[])]), 'candidate_track_non_genre')
        require(native['outcome'] == ('needs_review' if inferred else old['outcome']), 'outcome_contract')
        require(stable(candidate) == stable(record['persistedEvidence'].get('candidate')), 'persisted_candidate')
        review = previous.get('verification') in ['album_content', 'recording_only'] or inferred or baseline['reviewPendingAfterRestart']
        require(record['reviewPendingAfterRestart'] == review, 'persisted_review')
        require(stable(receipts(native.get('diagnostics', []))) == stable(receipts(old.get('diagnostics', []))), 'source_receipts')
        for receipt in receipts(native.get('diagnostics', [])):
            require(stable(receipt) in stable(receipts(record['persistedEvidence'].get('diagnostics', []))), 'persisted_receipt')
            if receipt['stage'] == 'local_track_preservation':
                index = receipt['localIndex']
                for field in receipt['fields']:
                    require(after[index].get(field) == before[index].get(field), 'preservation_receipt')
        if inferred:
            provenance = candidate['assisted_genre']
            compact = json.dumps(job['script']['payload'], sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()
            require(provenance == dict(model='scripted/genre-v1', value='Offline test genre', input_sha256=hashlib.sha256(compact).hexdigest()), 'model_provenance_binding')
            require(native['reasonCode'] == 'ai_genre_fill', 'inferred_review_reason')
    for index, (local, actual, old_local, old_actual) in enumerate(zip(before, after, baseline['beforeTracks'], baseline['readbackTracks'])):
        name = pathlib.Path(local['path']).name
        expected_local = job['genres'].get(name, old_local.get('genre'))
        require(local.get('genre') == expected_local, 'prewrite_local_choice')
        require(stable({k:v for k,v in local.items() if k!='genre'}) == stable({k:v for k,v in old_local.items() if k!='genre'}), 'prewrite_profile_boundary')
        target = local if no_write else old_actual
        require(stable({k:v for k,v in actual.items() if k!='genre'}) == stable({k:v for k,v in target.items() if k!='genre'}), 'non_genre_readback')
        expected = expected_local if expected_local else 'Offline test genre' if inferred else old_actual.get('genre')
        require(actual.get('genre') == expected, 'genre_readback')
        origin = 'retained_local' if expected_local else 'ai_inferred_unverified' if inferred else 'provider' if expected else 'missing'
        ledger.append(dict(index=index, file=name, before=local.get('genre'), after=actual.get('genre'), provenance=origin))
    for name, value in record['after'].items():
        baseline_hash = record['profileBaseline'][name]
        if value != '[redacted]' and baseline_hash != '[redacted]':
            require(value['payloadSha256'] == baseline_hash['payloadSha256'], 'independent_payload')
    return sorted(set(errors)), ledger


def main():
    pilot = '--pilot' in sys.argv
    base = HERE / ('pilot' if pilot else 'runs')
    rows, ledgers, failed = [], [], []
    unique = set()
    totals = collections.Counter()
    certificates = load(PRIOR / 'source-certificates.json')
    for job in frozen['jobs']:
        directory = base / job['name']
        if not (directory / 'execution.json').exists():
            assert pilot, job['name']
            continue
        inventory = load(directory / 'provider-inventory-final.json')
        old_requests = load(ROOT / job['baseline'] / 'provider-inventory-final.json')['requests']
        require_requests = inventory['requests'] == old_requests
        totals.update(dict(providerRequests=len(inventory['requests']), modelRequests=len(inventory['modelRequests']), misses=len(inventory['misses'])))
        cold_models = 0
        cold = None
        sources = {(s['provider'], s['releaseId']): load(PRIOR / s['bodyFile']) for s in certificates[job['case']['caseId']]['eligibleSources']}
        for phase in ['cold', 'warm']:
            record = load(directory / f'{phase}.jsonl')
            baseline = load(ROOT / job['baseline'] / f'{phase}.jsonl')
            exchange = load(directory / f'requests-{phase}-{job["case"]["caseId"]}.json')
            models = exchange['modelRequests'][cold_models:]
            if phase == 'cold':
                cold_models = len(exchange['modelRequests'])
                cold = record
            findings, ledger = audit_record(record, job, baseline, models)
            if not require_requests:
                findings.append('provider_exchange_change')
            if inventory['misses']:
                findings.append('fixture_miss')
            if phase == 'cold' and any(record['cacheBefore'].values()) or phase == 'warm' and (record['cacheBefore'] != cold['cacheAfter'] or record['profileBaseline'] != cold['profileBaseline']):
                findings.append('cache_isolation')
            old_codes, _ = old_fields.audit_record(record, sources)
            context = dict(name=job['name'], caseId=job['case']['caseId'], profile=job['profile'], phase=phase,
                           kind=job['kind'], scenario=job['scenario'])
            rows.append(dict(**context, findings=sorted(set(findings)), legacyDisabledOnlyFieldFindings=sorted(set(old_codes)),
                             written=(record['native'] or {}).get('written', 0), modelRequests=len(models),
                             outcome=record['outcome'], reasonCode=(record['native'] or {}).get('reasonCode'),
                             nativeReviewPending=record['reviewPendingAfterRestart'], genres=ledger))
            ledgers.extend(dict(**context, **r) for r in ledger)
            for t in record['beforeTracks']:
                unique.add((job['case']['caseId'], pathlib.Path(t['path']).name))
            totals.update(r['provenance'] for r in ledger)
            if findings:
                failed.append(dict(**context, findings=sorted(set(findings))))
    result = dict(evaluations=len(rows), uniquePhysicalInputs=len(unique), repeatedFileEvaluations=len(ledgers),
                  totals=dict(totals), findings=failed, clean=not failed, cases=rows,
                  historicalCertificationFindingsRetained=84, semanticAccuracyCredit=0,
                  legacyScope='Unchanged field auditor assumes assistance disabled; its assistance/genre findings remain reported, not silently removed',
                  limits='Silent dummy media, scripted responses, no acoustic truth or live-model quality; native persistence, not packaged acknowledgment')
    with (HERE / ('pilot-verification.json' if pilot else 'verification.json')).open('x') as stream:
        json.dump(result, stream, indent=2, ensure_ascii=False)
    print(json.dumps({k:v for k,v in result.items() if k!='cases'}, indent=2, ensure_ascii=False))
    return bool(failed)


if __name__ == '__main__':
    raise SystemExit(main())
