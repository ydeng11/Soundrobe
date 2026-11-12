"""Corrupt disposable copies only; historical and fresh native runs stay untouched."""
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import sys

root = pathlib.Path.cwd()
source, output = map(lambda p: pathlib.Path(p).resolve(), sys.argv[1:])
assert source.is_relative_to(root / '.amp/in/artifacts')
assert output.is_relative_to(root / '.amp/in/artifacts') and not output.exists()
output.mkdir()
auditor = root / 'scripts/auto-tag-eval-replay/audit-supervised.py'
load = lambda p: json.loads(p.read_text())
save = lambda p, v: p.write_text(json.dumps(v, ensure_ascii=False) + '\n')
digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
results = []
for mode in ['positive-copy', 'projection', 'published-score', 'late-exit', 'receiver-failure',
             'generation', 'denominator', 'staging-only', 'wire-accounting']:
    run = output / mode
    shutil.copytree(source, run)  # independent regular files, never hardlinks
    outcome = load(run / 'receiver-outcome.json')
    launcher = load(run / 'launcher-outcome.json')
    evidence = load(run / 'authority/provisional.json')
    published = load(run / 'results.json')
    if mode == 'projection':
        file = run / 'disabled-crowd/cold/records.json'
        rows = load(file)
        rows[0]['fieldObservations']['tracks'][0]['title'] = 'corrupt projection only'
        save(file, rows)
    elif mode == 'published-score':
        published['executions'][0]['score']['observedFieldSlots'] = 0
    elif mode == 'late-exit':
        outcome['supervisorExitObservedAt'] = outcome['deadline'] + 1
    elif mode == 'receiver-failure':
        launcher['receiverExit'] = 78
    elif mode == 'generation':
        outcome['generation'] = 'rebound-generation'
    elif mode == 'denominator':
        outcome['requestedFieldSlots'] = 215
    elif mode == 'staging-only':
        outcome['state'] = 'provisional_complete'
    elif mode == 'wire-accounting':
        evidence['counters']['model'] = 0
    if mode != 'positive-copy':
        # Bypass superficial file-hash mismatch so semantic checks must reject.
        save(run / 'results.json', published)
        evidence['resultSha256'] = outcome['resultSha256'] = digest(run / 'results.json')
        if mode == 'wire-accounting':
            save(run / 'authority-provisional.json', evidence)
        save(run / 'authority/provisional.json', evidence)
        outcome['evidenceSha256'] = digest(run / 'authority/provisional.json')
        save(run / 'receiver-outcome.json', outcome)
        launcher['receiverOutcomeSha256'] = digest(run / 'receiver-outcome.json')
        save(run / 'launcher-outcome.json', launcher)
    if mode == 'wire-accounting':
        assert load(run / 'authority-provisional.json') == load(run / 'authority/provisional.json'), 'wire control must preserve receipt equality'
    checked = subprocess.run([sys.executable, '-B', str(auditor), str(run)], text=True, capture_output=True)
    (output / f'{mode}.stdout').write_text(checked.stdout)
    (output / f'{mode}.stderr').write_text(checked.stderr)
    assert (checked.returncode == 0) == (mode == 'positive-copy'), (mode, checked.stderr)
    if mode == 'wire-accounting':
        sealed = root / '.planning/design/live-run-contract-20261111/durable-mock-evidence-20261112/audit-durable.py'
        # The maintained adapter inserts two status-adaptation lines above this assertion.
        line = next(i for i, text in enumerate(sealed.read_text().splitlines(), 1)
                    if text.startswith("assert receipt['counters'] == dict(")) + 2
        first = re.search(r'File "<string>", line (\d+), in <module>', checked.stderr)
        assert first and int(first[1]) == line, 'must reject counters against actual wire, not an earlier check'
    results.append(dict(control=mode, exit=checked.returncode, requestedFieldSlots=216,
                        evidenceCopyOnly=True, liveAuthority=False))
save(output / 'results.json', results)
print(json.dumps(results, indent=2))
