"""Keep byte equality strict wherever the observed genre does not change."""
import collections
import hashlib
import json
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]
load = lambda p: json.loads(p.read_text())
assert load(HERE / 'paired-hashes-tool-lock-v2.json')['sha256'] == hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()
frozen = load(HERE / 'freeze.json')
rows, errors = [], []
totals = collections.Counter()
for job in frozen['jobs']:
    for phase in ['cold','warm']:
        current = load(HERE / 'runs' / job['name'] / f'{phase}.jsonl')
        baseline = load(ROOT / job['baseline'] / f'{phase}.jsonl')
        no_write = job['expected'] in ['cancel_no_write','no_write_hold']
        for actual, old in zip(current['readbackTracks'], baseline['readbackTracks']):
            name = str(pathlib.Path(actual['path']).relative_to(pathlib.Path(current['beforeTracks'][0]['path']).parent))
            # The exported maps use album-relative physical file names.
            after = current['after'][name]
            previous = current['profileBaseline'][name] if no_write else baseline['after'][name]
            if after=='[redacted]' or previous=='[redacted]':
                verdict = 'redacted_independent_hash'
            elif no_write:
                verdict = 'no_write_bytes_equal' if after==previous else 'unexpected_no_write_change'
            elif actual.get('genre') == old.get('genre'):
                verdict = 'unchanged_output_bytes_equal' if after==previous else 'unexpected_unchanged_genre_bytes'
            else:
                allowed = job['expected']=='inferred_unverified' or pathlib.Path(actual['path']).name in job['genres']
                verdict = 'bounded_genre_hash_consequence' if allowed and after['payloadSha256']==previous['payloadSha256'] else 'unexpected_genre_hash_change'
            row = dict(name=job['name'], phase=phase, file=name, verdict=verdict)
            rows.append(row)
            totals[verdict]+=1
            if verdict.startswith('unexpected'):
                errors.append(row)
result = dict(evaluations=440, fileComparisons=len(rows), totals=dict(totals), findings=errors, rows=rows,
               limits='A changed full-file hash plus unchanged audio and expected readback does not independently prove every unexported tag; redactions remain unverified independently.')
with (HERE / 'paired-hashes.json').open('x') as stream:
    json.dump(result,stream,indent=2)
print(json.dumps({k:v for k,v in result.items() if k!='rows'},indent=2),flush=True)
raise SystemExit(bool(errors))
