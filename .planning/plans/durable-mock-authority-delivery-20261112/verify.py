"""Verify committed metadata and retained bindings without running native code."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
PREFIX = '.planning/plans/durable-mock-authority-delivery-20261112/'
EVIDENCE = '.planning/design/live-run-contract-20261111/durable-mock-evidence-20261112/'
MANIFEST = PREFIX + 'delivery-SHA256SUMS'
BASE = '5f8389e1afc966c3eb9bd7ba9b21684035cd6343'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def verify(read):
    digest = lambda path: hashlib.sha256(read(path)).hexdigest()
    load = lambda path: json.loads(read(path))
    entries = {}
    for line in read(MANIFEST).decode().splitlines():
        expected, path = line.split('  ', 1)
        require(not Path(path).is_absolute() and '..' not in Path(path).parts
                and path not in entries, 'unsafe or duplicate delivery path')
        require(digest(path) == expected, 'hash mismatch: ' + path)
        entries[path] = expected
    for line in read(EVIDENCE + 'SHA256SUMS').decode().splitlines():
        expected, path = line.split('  ', 1)
        require(digest(path) == expected, 'evidence seal: ' + path)
    binding = load(PREFIX + 'source-bindings.json')
    require(binding['parent'] == BASE and binding['branch'] == 'artist-library-inventory', 'delivery base')
    require(binding['implementationFiles'] == load(EVIDENCE + 'source-state.json')['files'], 'reviewed sources')
    require(digest(EVIDENCE + 'SHA256SUMS') == binding['evidenceSealSha256'], 'evidence seal binding')
    for path, expected in binding['implementationFiles'].items():
        require(entries.get(path) == expected, 'reviewed implementation binding: ' + path)
    build = load(PREFIX + 'build-receipt.json')
    require(build['executable']['sha256'] == binding['executableSha256']
            == load(EVIDENCE + 'process-controls.json')['executableSha256'], 'executable binding')
    for rows in [build['sourceBindings'], load(PREFIX + 'run-source-and-tool-locks.json'),
                 load(PREFIX + 'control-source-and-tool-locks.json')]:
        for row in rows:
            if row['path'] == build['executable']['path']:
                require(row['sha256'] == binding['executableSha256'], 'control executable binding')
            elif not row['path'].startswith('.amp/in/'):
                require(digest(row['path']) == row['sha256'], 'source/tool binding: ' + row['path'])
    run_locks = load(PREFIX + 'run-source-and-tool-locks.json')
    require(run_locks[0]['sha256'] == binding['executableSha256'], 'frozen executable binding')
    for folder in ['crowd-lu-user-acceptance-v1-20261111', 'scorable-assistance-contract-20261111']:
        for line in read('.planning/plans/' + folder + '/SHA256SUMS').decode().splitlines():
            expected, path = line.split('  ', 1)
            require(digest(path) == expected, 'historical seal: ' + path)
    membership = load(EVIDENCE + 'requested-membership.json')
    jobs = membership['evaluations']
    require([j['name'] for j in jobs] == ['disabled-crowd', 'disabled-billie', 'disabled-enya',
        'assistance-billie', 'assistance-enya', 'assistance-crowd'], 'cold membership')
    require(sum(len(j['physicalPaths']) for j in jobs) == membership['requestedPhysicalOccurrences'] == 72
            and sum(len(j['physicalPaths']) * len(j['fields']) for j in jobs)
            == membership['requestedFieldSlots'] == 216, 'physical denominator')
    audit = load(EVIDENCE + 'run-audit.json')
    require(audit['native']['observedFieldSlots'] == audit['native']['requestedFieldSlots'] == 216
            and audit['native']['userAcceptanceRequested'] == 22
            and audit['native']['userAcceptanceAgreeing'] == audit['groundedAssistanceAccuracyCredit'] == 0,
            'observations and quality credit')
    require(audit['durable']['counts'] == dict(provider=472, inventory=24, model=1,
            reserved=497, arrived=497, completed=497)
            and audit['durable']['actualWireArrivals'] == 497
            and audit['durable']['consumptions'] == 1, 'operation accounting')
    require(len(load(EVIDENCE + 'process-controls.json')['results']) == 20
            and load(EVIDENCE + 'projection-controls.json')['controlsPassed'] == 70
            and load(EVIDENCE + 'audit-controls.json')['controlsPassed'] == 8, 'retained controls')
    require(load(EVIDENCE + 'label-binding-status.json')['fiveHistoricalTests']['errors'] == 1
            and not load(PREFIX + 'reused-gates.json')['freshDuringDelivery'], 'historical and reused limits')
    return {'files': len(entries), 'requestedFieldSlots': 216, 'observedFieldSlots': 216,
            'wireArrivals': 497, 'userAgreement': 0, 'groundedAiCredit': 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ref', choices=['index', 'HEAD'])
    parser.add_argument('--local', action='store_true', help='hash retained metadata and binaries, never execute them')
    args = parser.parse_args()
    git = lambda *words: subprocess.check_output(['git', *words], cwd=ROOT)
    def read(path):
        if args.ref:
            return git('show', (':' if args.ref == 'index' else 'HEAD:') + path)
        return (ROOT / path).read_bytes()
    result = verify(read)
    binding = json.loads(read(PREFIX + 'source-bindings.json'))
    diff = ['diff', '--binary'] + (['--cached'] if args.ref == 'index' else [])
    diff += [BASE] + (['HEAD'] if args.ref == 'HEAD' else [])
    patch = git(*diff, '--', *sorted(binding['implementationFiles']))
    require(hashlib.sha256(patch).hexdigest() == binding['implementationPatchSha256'], 'reviewed patch bytes')
    if args.ref:
        expected = {line.split('  ', 1)[1] for line in read(MANIFEST).decode().splitlines()} | {MANIFEST}
        if args.ref == 'index':
            require(git('rev-parse', 'HEAD').decode().strip() == BASE, 'unexpected staging parent')
            actual = set(git('diff', '--cached', '--name-only').decode().splitlines())
        else:
            require(git('rev-parse', 'HEAD^').decode().strip() == BASE, 'unexpected commit parent')
            actual = set(git('diff-tree', '--no-commit-id', '--name-only', '-r', 'HEAD').decode().splitlines())
        require(actual == expected, 'Git scope differs from manifest')
        result['gitScope'] = args.ref
    if args.local:
        retained = json.loads(read(EVIDENCE + 'retained-bindings.json'))['files']
        for row in retained:
            path = ROOT / row['path']
            require(row['path'].startswith('.amp/in/artifacts/durable-mock-authority-20261112/')
                    and path.suffix.lower() in ['.log', '.json', '.jsonl', '.stderr', '.sqlite', '.gz', '']
                    and path.stat().st_size == row['bytes'], 'unexpected retained input: ' + row['path'])
            digest = hashlib.sha256()
            with path.open('rb') as source:
                for block in iter(lambda: source.read(1048576), b''):
                    digest.update(block)
            require(digest.hexdigest() == row['sha256'], 'retained digest: ' + row['path'])
        result['retainedDependencies'] = len(retained)
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    main()
