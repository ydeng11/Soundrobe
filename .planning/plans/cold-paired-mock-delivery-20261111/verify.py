"""Verify delivery bytes and retained metadata without running native code."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
PREFIX = '.planning/plans/cold-paired-mock-delivery-20261111/'
MANIFEST = PREFIX + 'delivery-SHA256SUMS'
BASE = '4b0a9cfc41db8476721b2cc1fd453e121b85bc17'
PATCH = 'f7bb8f87230f9c701e7fb574c387f94d477ca63121c01b0b993162230df4cb96'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def verify(read):
    entries = {}
    for line in read(MANIFEST).decode().splitlines():
        expected, path = line.split('  ', 1)
        require(not Path(path).is_absolute() and '..' not in Path(path).parts,
                'unsafe manifest path')
        require(path not in entries, 'duplicate manifest path')
        require(hashlib.sha256(read(path)).hexdigest() == expected, 'hash mismatch: ' + path)
        entries[path] = expected
    load = lambda name: json.loads(read(PREFIX + name))
    binding = load('source-bindings.json')
    require(binding['head'] == BASE and binding['patchSha256'] == PATCH, 'reviewed patch binding')
    for path, expected in binding['files'].items():
        require(entries.get(path) == expected, 'reviewed source binding: ' + path)
    for lock in load('run-source-and-tool-locks.json'):
        if not lock['path'].startswith('.amp/in/'):
            require(hashlib.sha256(read(lock['path'])).hexdigest() == lock['sha256'],
                    'run source/tool binding: ' + lock['path'])
    for folder in ['crowd-lu-user-acceptance-v1-20261111', 'scorable-assistance-contract-20261111']:
        seal = '.planning/plans/' + folder + '/SHA256SUMS'
        for line in read(seal).decode().splitlines():
            expected, path = line.split('  ', 1)
            require(hashlib.sha256(read(path)).hexdigest() == expected, 'historical seal: ' + path)
    membership = load('requested-membership.json')
    jobs = membership['evaluations']
    names = ['disabled-crowd', 'disabled-billie', 'disabled-enya',
             'assistance-billie', 'assistance-enya', 'assistance-crowd']
    require([j['name'] for j in jobs] == names and all(j['phase'] == 'cold' for j in jobs),
            'cold job membership')
    identities = {(j['caseId'], p) for j in jobs for p in j['physicalPaths']}
    observations = sum(len(j['physicalPaths']) for j in jobs)
    slots = sum(len(j['physicalPaths']) * len(j['fields']) for j in jobs)
    require(len(identities) == 36 and observations == membership['requestedPhysicalOccurrences'] == 72
            and slots == membership['requestedFieldSlots'] == 216, 'physical membership denominator')
    require(sum(len(j['userLabelSelectors']) for j in jobs) == 22
            and sum(len(j['labelSelectors']) for j in jobs) == 8
            and membership['predeclaredLabelableSlots'] == 30, 'label membership denominator')
    selection = load('selection.json')
    require([j['name'] for j in selection['jobs']] == names and selection['phases'] == ['cold'],
            'selection membership')
    require(selection['orchestration']['inventoryBytes'] == 8388608, 'inventory cap changed')
    require(all(selection[k] == 0 for k in ['realProviderCalls', 'realModelCalls', 'originalMediaOperations']),
            'mock-only selection')
    require(load('selection-status.json') == {
        'executionStatus': 'complete', 'scoringStatus': 'insufficient',
        'resultsFile': '.amp/in/artifacts/cold-paired-development-20261111/run-05/results.json'},
        'execution and scoring status')
    audit = load('run-audit.json')
    require(audit['checks'] == 'passed' and audit['requestedFieldSlots'] == audit['observedFieldSlots'] == slots
            and audit['labelableSlots'] == 30 and audit['userAcceptanceRequested'] == 22
            and audit['userAcceptanceAgreeing'] == audit['groundedAssistanceAccuracyCredit'] == 0,
            'run audit counts or quality credit')
    inventories = load('inventory-summaries.json')
    require([i['name'] for i in inventories] == names, 'inventory membership')
    for row in inventories:
        require(row['records'] == 1533 and row['bytes'] <= 8388608
                and row['misses'] == 0 and not row['upstreamForwardingAvailable'], 'compact inventory accounting')
        require(row['providerRequests'] + row['modelRequests'] + row['inventoryArrivals'] == row['wireArrivals'],
                'arrival reconciliation')
    require(sum(i['providerRequests'] for i in inventories) == audit['providerRequests'] == 472
            and sum(i['modelRequests'] for i in inventories) == audit['modelRequests'] == 1
            and sum(i['inventoryArrivals'] for i in inventories) == 18
            and sum(i['wireArrivals'] for i in inventories) == audit['wireArrivals'] == 491,
            'total operation accounting')
    controls = load('projection-controls.json')
    require(len(set(controls['controls'])) == controls['controlsPassed'] == 70, 'projection controls')
    separation = load('label-separation.json')
    require(separation['sha256'] == binding['requestSha256'] and separation['bytes'] == 863
            and separation['matrixRequests'] == separation['separateLabelFreeControlRequests'] == 1
            and not separation['acousticAccuracyClaim'], 'serialized-request separation')
    return {'files': len(entries), 'requestedFieldSlots': slots, 'observedFieldSlots': 216,
            'userAgreement': 0, 'groundedAiCredit': 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ref', choices=['index', 'HEAD'])
    parser.add_argument('--local', action='store_true', help='also hash retained ignored metadata and executable bytes')
    args = parser.parse_args()
    git = lambda *words: subprocess.check_output(['git', *words], cwd=ROOT)
    def read(path):
        if args.ref:
            return git('show', (':' if args.ref == 'index' else 'HEAD:') + path)
        return (ROOT / path).read_bytes()
    result = verify(read)
    bindings = json.loads(read(PREFIX + 'source-bindings.json'))
    diff_args = ['diff', '--binary'] + (['--cached'] if args.ref == 'index' else [])
    diff_args += [BASE] + (['HEAD'] if args.ref == 'HEAD' else [])
    patch = git(*diff_args, '--', *sorted(bindings['files']))
    require(hashlib.sha256(patch).hexdigest() == PATCH, 'reviewed patch bytes changed')
    if args.ref:
        expected = {line.split('  ', 1)[1] for line in read(MANIFEST).decode().splitlines()} | {MANIFEST}
        if args.ref == 'index':
            require(git('rev-parse', 'HEAD').decode().strip() == BASE, 'unexpected staging base')
            actual = set(git('diff', '--cached', '--name-only').decode().splitlines())
        else:
            require(git('rev-parse', 'HEAD^').decode().strip() == BASE, 'unexpected commit parent')
            actual = set(git('diff-tree', '--no-commit-id', '--name-only', '-r', 'HEAD').decode().splitlines())
        require(actual == expected, 'Git delivery scope differs from manifest')
        result['gitScope'] = args.ref
    if args.local:
        retained = json.loads(read(PREFIX + 'retained-bindings.json'))['files']
        for item in retained:
            path = ROOT / item['path']
            require(item['path'].startswith('.amp/in/') and path.suffix.lower() not in ['.flac', '.wav', '.mp3', '.dsf'],
                    'unexpected retained input')
            require(path.stat().st_size == item['bytes'], 'local byte length: ' + item['path'])
            digest = hashlib.sha256()
            with path.open('rb') as source:
                for block in iter(lambda: source.read(1048576), b''):
                    digest.update(block)
            require(digest.hexdigest() == item['sha256'], 'local digest: ' + item['path'])
        result['retainedDependencies'] = len(retained)
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    main()
