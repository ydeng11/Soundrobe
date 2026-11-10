"""Compare remaining supported-write construction fields without changing oracles."""
import hashlib
import importlib.util
import json
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]
load = lambda p: json.loads(p.read_text())
spec = importlib.util.spec_from_file_location('frozen_audit', HERE / 'audit.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)
prior = ROOT / load(HERE / 'freeze.json')['prior']
spec = importlib.util.spec_from_file_location('previous_runtime_contract', prior / 'compare-v2.py')
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


def compare(current, before, job):
    errors = []
    for key in ['selectedTrackEvidence', 'progressEvents']:
        if audit.stable(current[key]) != audit.stable(before[key]):
            errors.append(key)
    for container in ['native', 'persistedEvidence']:
        actual, old = current[container], before[container]
        # These optional values are checked by the primary provenance audit.
        if set(actual) - {'aiStatus', 'reasonCode'} != set(old) - {'aiStatus', 'reasonCode'}:
            errors.append(container + '_keys')
        for field, value in [('aiStatus', 'genre_inferred'), ('reasonCode', 'ai_genre_fill')]:
            expected = value if job['expected']=='inferred_unverified' else old.get(field)
            if actual.get(field) != expected:
                errors.append(container + '_' + field)
        if actual.get('keptTags') != old.get('keptTags'):
            errors.append(container + '_kept_tags')
        diagnostics = list(actual['diagnostics'])
        if job['scenario'] == 'failed':
            expected = dict(stage='genre', reason='AI provider returned HTTP 400')
            if diagnostics.count(expected) != 1:
                errors.append(container + '_failure_diagnostic')
            diagnostics = [d for d in diagnostics if d != expected]
        configs = [d['settings'] for d in diagnostics if d.get('stage') == 'configuration']
        old_configs = [d['settings'] for d in old['diagnostics'] if d.get('stage') == 'configuration']
        if len(configs) != 1 or len(old_configs) != 1:
            errors.append(container + '_configuration_count')
        else:
            # diagnostic_configuration hashes the sorted JSON before adding fingerprint.
            for settings in [configs[0], old_configs[0]]:
                encoded = json.dumps({k:v for k,v in settings.items() if k!='fingerprint'},
                                     sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()
                if hashlib.sha256(encoded).hexdigest() != settings['fingerprint']:
                    errors.append(container + '_configuration_fingerprint')
            ignored = {'fingerprint', 'llmConfigured', 'llmModel', 'llmProvider'}
            if {k:v for k,v in configs[0].items() if k not in ignored} != {k:v for k,v in old_configs[0].items() if k not in ignored}:
                errors.append(container + '_provider_configuration')
            if job['scenario'] == 'disabled':
                valid = configs == old_configs
            else:
                valid = (configs[0]['llmConfigured'] is True and configs[0]['llmModel'] == 'scripted/genre-v1'
                         and configs[0]['llmProvider'] == 'openai')
            if not valid:
                errors.append(container + '_model_configuration')
        without_config = lambda ds: [d for d in ds if d.get('stage') != 'configuration']
        differences = runtime.base.differences(
            runtime.base.stable({container: dict(diagnostics=without_config(old['diagnostics']))}),
            runtime.base.stable({container: dict(diagnostics=without_config(diagnostics))}))
        if any(not runtime.runtime(row, before, current) for row in differences):
            errors.append(container + '_diagnostics')
    review = dict(kind='needs_review', message='AI-assisted tags require review', data=dict(reasonCode='ai_genre_fill'))
    # The source's early album-content return persists review but emits no generic review event.
    generic = current['native']['candidate'].get('verification') != 'album_content'
    if current['reportEvents'].count(review) != int(job['expected']=='inferred_unverified' and generic):
        errors.append('inferred_review_event')
    provider_events = lambda r: [e for e in r['reportEvents']
                                 if (e.get('data') or {}).get('source') != 'llm' and e != review]
    if audit.stable(provider_events(current)) != audit.stable(provider_events(before)):
        errors.append('provider_report_events')
    return errors


if __name__ == '__main__':
    frozen = load(HERE / 'freeze.json')
    rows = []
    for job in frozen['jobs']:
        if job['expected'] in ['cancel_no_write', 'no_write_hold']:
            continue
        for phase in ['cold', 'warm']:
            current = load(HERE / 'runs' / job['name'] / f'{phase}.jsonl')
            before = load(ROOT / job['baseline'] / f'{phase}.jsonl')
            rows.append(dict(name=job['name'], phase=phase, findings=compare(current, before, job)))
    result = dict(evaluations=len(rows), findings=[r for r in rows if r['findings']], rows=rows,
                  configurationBinding='Recomputed source-defined fingerprints; only exact configured scripted model/provider/key-presence changes permitted',
                  scope='Supported-write construction only; primary audit separately requires exact prewrite bytes/tags and no authority for holds/cancellation',
                  toolSha256=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest())
    with (HERE / 'construction-diff.json').open('x') as stream:
        json.dump(result, stream, indent=2)
    print(json.dumps({k:v for k,v in result.items() if k!='rows'}, indent=2))
    raise SystemExit(bool(result['findings']))
