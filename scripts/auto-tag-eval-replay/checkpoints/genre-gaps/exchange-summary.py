"""Separate actual phase increments from cumulative warm inventories."""
import collections
import csv
import hashlib
import json
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
load = lambda p: json.loads(p.read_text())
sha = lambda data: hashlib.sha256(data).hexdigest()
frozen = load(HERE / 'freeze.json')
assert load(HERE / 'exchange-tool-lock.json')['sha256'] == sha(pathlib.Path(__file__).read_bytes())
rows, scripts = [], set()
totals, roles, statuses = collections.Counter(), collections.Counter(), collections.Counter()
for job in frozen['jobs']:
    directory = HERE / 'runs' / job['name']
    prior_requests, prior_models = [], []
    for phase in ['cold', 'warm']:
        exchange = load(directory / f'requests-{phase}-{job["case"]["caseId"]}.json')
        assert exchange['requests'][:len(prior_requests)] == prior_requests
        assert exchange['modelRequests'][:len(prior_models)] == prior_models
        providers = exchange['requests'][len(prior_requests):]
        models = exchange['modelRequests'][len(prior_models):]
        for model in models:
            name = model['body']['response_format']['json_schema']['name']
            assert name == job['script']['schemaName']
            if model['status'] == 400:
                assert model['response'] == dict(error='scripted_service_failure')
            roles[name] += 1
            statuses[str(model['status'])] += 1
        if models:
            scripts.add(job['name'])
        counts = collections.Counter(r['provider'] for r in providers)
        rows.append(dict(name=job['name'], caseId=job['case']['caseId'], profile=job['profile'], phase=phase,
                         scenario=job['scenario'], kind=job['kind'], providerRequests=len(providers),
                         musicbrainz=counts['musicbrainz'], discogs=counts['discogs'], modelRequests=len(models),
                         schemaName=job['script']['schemaName'] if models else None,
                         scriptSha256=sha(json.dumps(job['script'],sort_keys=True,ensure_ascii=False).encode())))
        totals.update(dict(providerRequests=len(providers), modelRequests=len(models)))
        prior_requests, prior_models = exchange['requests'], exchange['modelRequests']
    final = load(directory / 'provider-inventory-final.json')
    assert final['requests'] == prior_requests and final['modelRequests'] == prior_models
    assert len(prior_models) == job['expectedModelRequests'] and not final['misses']
with (HERE / 'phase-exchanges.csv').open('x') as stream:
    writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
    writer.writeheader()
    writer.writerows(rows)
result = dict(totals=dict(totals), roles=dict(roles), statuses=dict(statuses), evaluations=len(rows),
               exercisedScriptInstances=len(scripts), unusedScriptDeclarations=len(frozen['jobs'])-len(scripts),
               maxRequestsPerProcess=2, rows=rows, toolSha256=sha(pathlib.Path(__file__).read_bytes()),
               groundedAssistanceAccuracyCredit=0)
with (HERE / 'exchange-summary.json').open('x') as stream:
    json.dump(result, stream, indent=2)
print(json.dumps({k:v for k,v in result.items() if k!='rows'},indent=2),flush=True)
