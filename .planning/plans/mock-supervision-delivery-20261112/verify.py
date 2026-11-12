"""Check delivery bytes and retained evidence without running native code or services."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
PREFIX = '.planning/plans/mock-supervision-delivery-20261112/'
BASE = '8287b6b0cb47365522ac13eab94e1c8595e90a06'
MANIFEST = PREFIX + 'delivery-SHA256SUMS'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def verify(read):
    digest = lambda path: hashlib.sha256(read(path)).hexdigest()
    load = lambda name: json.loads(read(PREFIX + name))
    entries = {}
    for line in read(MANIFEST).decode().splitlines():
        expected, path = line.split('  ', 1)
        require(not Path(path).is_absolute() and '..' not in Path(path).parts
                and path not in entries and not path.startswith('.amp/'), 'unsafe delivery path')
        require(digest(path) == expected, 'hash mismatch: ' + path)
        entries[path] = expected
    binding = load('source-bindings.json')
    require(binding['parent'] == BASE and binding['branch'] == 'artist-library-inventory', 'delivery base')
    for path, expected in binding['implementationFiles'].items():
        require(entries.get(path) == expected, 'reviewed implementation: ' + path)
    for line in read(PREFIX + 'source-100.SHA256SUMS').decode().splitlines():
        expected, path = line.split('  ', 1)
        if path == binding['executablePath']:
            require(expected == binding['executableSha256'], 'executable binding')
        else:
            require(digest(path) == expected, 'frozen source: ' + path)
    for name, count in [('run-source-and-tool-locks.json', 128),
                        ('run-receipt-bindings.json', 1662),
                        ('fault-source-and-tool-locks.json', 103)]:
        rows = load(name)
        require(len(rows) == count, 'binding count: ' + name)
        for row in rows:
            if row['path'] == binding['executablePath']:
                require(row['sha256'] == binding['executableSha256'], 'locked executable')
            elif not row['path'].startswith('.amp/in/'):
                require(digest(row['path']) == row['sha256'], 'source/tool binding: ' + row['path'])
    outcome = load('run-receiver-outcome.json')
    launcher = load('run-launcher-outcome.json')
    config = load('run-native-supervision.json')
    phase = load('run-execution-scope.json')
    require(outcome['state'] == launcher['state'] == 'complete'
            and outcome['authority'] == launcher['authority'] == 'native_witness_and_timely_supervisor_exit'
            and outcome['controlOnly'] is False and not config['control']
            and launcher['receiverExit'] == 0 and launcher['receiverSignal'] is None
            and launcher['actualReceiverExitObserved'] is True
            and outcome['liveAuthority'] is launcher['liveAuthority'] is False, 'native completion')
    require(outcome['supervisorExitObservedAt'] <= outcome['acceptedAt'] < outcome['deadline']
            == phase['deadline'], 'timely exit and acceptance')
    require(config['preparationMs'] == 300000 and config['executionMs'] == phase['executionMs'] == 600000
            and 0 < phase['deadline'] - 600000 - config['start'] < 300000, 'unchanged scopes')
    require(outcome['generation'] == launcher['generation'] == config['generation']
            and outcome['receiverPid'] == launcher['receiverPid'] == config['receiverPid'], 'generation and receiver')
    require(launcher['receiverOutcomeSha256'] == digest(PREFIX + 'run-receiver-outcome.json'), 'receiver receipt hash')
    retained = {row['path']: row for row in load('retained-bindings.json')['files']}
    raw = '.amp/in/artifacts/mock-supervision-20261112/run-114/'
    require(outcome['evidenceSha256'] == retained[raw + 'authority/provisional.json']['sha256']
            and outcome['resultSha256'] == retained[raw + 'results.json']['sha256'], 'staged receipt links')
    audit = load('run-audit.json')
    require(audit['native']['observedFieldSlots'] == audit['native']['requestedFieldSlots']
            == outcome['requestedFieldSlots'] == launcher['requestedFieldSlots'] == 216
            and audit['native']['userAcceptanceRequested'] == 22
            and audit['native']['userAcceptanceAgreeing'] == audit['groundedAssistanceAccuracyCredit'] == 0,
            'denominator and quality credit')
    require(audit['durable']['counts'] == dict(provider=472, inventory=24, model=1,
            reserved=497, arrived=497, completed=497)
            and audit['durable']['sessions'] == 12 and audit['durable']['consumptions'] == 1,
            'wire accounting')
    controls = load('audit-controls.json')
    require(len(controls) == 9 and controls[0]['exit'] == 0
            and all(row['exit'] != 0 for row in controls[1:])
            and len(load('fault-controls.json')['controls']) == 10, 'retained controls')
    for folder in ['crowd-lu-user-acceptance-v1-20261111', 'scorable-assistance-contract-20261111']:
        for line in read('.planning/plans/' + folder + '/SHA256SUMS').decode().splitlines():
            expected, path = line.split('  ', 1)
            require(digest(path) == expected, 'historical seal: ' + path)
    require(load('reused-gates.json')['freshDuringDelivery'] is False, 'reused gate classification')
    return dict(files=len(entries), requestedFieldSlots=216, observedFieldSlots=216,
                wireArrivals=497, groundedAiCredit=0, userAgreement=0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ref', choices=['index', 'HEAD'])
    parser.add_argument('--local', action='store_true', help='hash retained evidence and binary; never execute them')
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
        rows = json.loads(read(PREFIX + 'retained-bindings.json'))['files']
        rows += json.loads(read(PREFIX + 'run-receipt-bindings.json'))
        unique = {row['path']: row for row in rows}
        for path, row in unique.items():
            require(not Path(path).is_absolute() and '..' not in Path(path).parts, 'unsafe retained path')
            file = ROOT / path
            if 'bytes' in row:
                require(file.stat().st_size == row['bytes'], 'retained size: ' + path)
            value = hashlib.sha256()
            with file.open('rb') as source:
                for block in iter(lambda: source.read(1048576), b''):
                    value.update(block)
            require(value.hexdigest() == row['sha256'], 'retained hash: ' + path)
        result['retainedFilesChecked'] = len(unique)
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    main()
