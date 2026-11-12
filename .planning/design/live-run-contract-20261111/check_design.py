"""Read source/metadata only. Print a receipt; never execute an evaluator."""
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(path):
    return json.loads(path.read_text())


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT, text=True).strip()


def json_shape(value, depth=1):
    if isinstance(value, dict):
        children, width = list(value.values()), len(value)
        strings = [len(key.encode()) for key in value]
    elif isinstance(value, list):
        children, width, strings = value, len(value), []
    else:
        return depth, 1, 0, len(value.encode()) if isinstance(value, str) else 0
    shapes = [json_shape(child, depth + 1) for child in children]
    return (max([depth] + [s[0] for s in shapes]),
            1 + sum(s[1] for s in shapes), max([width] + [s[2] for s in shapes]),
            max([0] + strings + [s[3] for s in shapes]))


def check_resources(proposal):
    limits = proposal['limitsCandidate']
    for group in ['jsonParseLimits', 'modelJsonLimits', 'fixtureBodyAndStorageLimits',
                  'candidateAndReadbackLimits']:
        assert isinstance(limits[group], dict) and limits[group]
        assert all(type(v) is bool or type(v) is int and v > 0 for v in limits[group].values())
    assert limits['failurePublicationMs'] == 10000 and limits['failureReceiptBytes'] == 65536
    assert limits['inventoryDecodedBytesEach'] == 8388608
    assert not limits['jsonParseLimits']['duplicateKeysAllowed']
    assert not limits['modelJsonLimits']['genreControlCharactersAllowed']
    measured = load(HERE / 'resource-measurements.json')
    assert len(measured['files']) == 7
    for item in measured['files']:
        path = ROOT / item['path']
        assert path.suffix == '.json' and path.resolve().is_relative_to(ROOT)
        assert digest(path) == item['sha256'] and path.stat().st_size == item['bytes']
        assert list(json_shape(load(path))) == item['depthNodesCollectionStringUtf8']
    crowd_inventory = measured['files'][4]
    assert crowd_inventory['bytes'] > limits['inventoryDecodedBytesEach']
    return len(measured['files'])


def main():
    proposal = load(HERE / 'proposal.json')
    bindings = load(HERE / 'source-bindings.json')
    assert git('rev-parse', 'HEAD') == proposal['sourceHead'] == bindings['head']
    assert git('branch', '--show-current') == bindings['branch']
    for item in bindings['files']:
        path = ROOT / item['path']
        assert digest(path) == item['sha256'], path
        lines = path.read_text().splitlines()
        if 'lineCount' in item:
            assert len(lines) == item['lineCount'], path
        for anchor, locations in item.get('anchors', {}).items():
            assert [i + 1 for i, line in enumerate(lines) if anchor in line] == locations
    assert not proposal['executionAuthorized'] and not proposal['activationAvailable']
    assert proposal['currentAuthorizedLiveAttempts'] == 0
    assert all(value is None for value in proposal['approvals'].values())
    assert proposal['phases'] == ['cold']
    assert proposal['arms'] == ['disabled', 'assistance']
    sample = load(ROOT / proposal['sample'])
    labels = load(ROOT / proposal['userLabels'])
    assert len(sample['jobs']) == len(proposal['jobs']) == 3
    identities = set()
    catalog = 0
    for selected, job in zip(proposal['jobs'], sample['jobs']):
        assert (selected['caseId'], selected['profile'], selected['files']) == (
            job['caseId'], job['profile'], len(job['files']))
        for row in job['files']:
            key = (job['caseId'], row['path'])
            assert key not in identities
            identities.add(key)
            catalog += sum(value is not None for value in row['acceptedValues'].values())
    assert len(identities) == 36 and catalog == 4
    assert len(labels['occurrences']) == 11
    exact = ['Pop', 'Mandopop', 'Pop, Mandopop']
    for i, (label, row) in enumerate(zip(labels['occurrences'], sample['jobs'][0]['files'])):
        assert label['row'] == i + 1
        assert label['sampleSelector'] == f'/jobs/0/files/{i}'
        assert label['inventorySelector'] == row['inventorySelector']
        assert label['relativePath'] == row['path']
        assert label['capturedMediaSha256Claim'] == row['capturedMediaSha256Claim']
        assert label['title'] == row['profileVisible']['title']
        assert label['acceptedGenreValues'] == exact
    assert labels['occurrences'][0]['capturedDurationSeconds'] == 62.17
    assert labels['occurrences'][-1]['title'].endswith('[Acoustic Ver.]')
    assert labels['albumLevel']['acceptedGenreValues'] == exact
    n = len(identities)
    expected = dict(evaluations=len(sample['jobs']) * 2, uniquePhysicalOccurrences=n,
                    repeatedPhysicalObservations=n * 2, requestedFieldSlots=n * 2 * 3,
                    catalogLabelSlots=catalog * 2, userAcceptanceSlots=11 * 2,
                    labelableSlots=(catalog + 11) * 2,
                    unscorableSlots=n * 6 - (catalog + 11) * 2,
                    uniqueUserLabels=11, uniqueOccurrenceFieldLabels=catalog + 11)
    assert proposal['expectedMembership'] == expected
    assert set(proposal['executionOrderCandidate']) == {
        f'{arm}:{job["caseId"]}' for arm in proposal['arms'] for job in proposal['jobs']}
    assert len(proposal['executionOrderCandidate']) == expected['evaluations']
    assert sum(j['recommendedMaxAttempts'] for j in proposal['jobs']) == 1
    assert proposal['limitsCandidate']['totalModelAttempts'] == 1
    assert proposal['limitsCandidate']['modelRetries'] == proposal['limitsCandidate']['schemaRepairs'] == 0
    measurements = check_resources(proposal)
    seals = {}
    for name in ['crowd-lu-user-acceptance-v1-20261111/SHA256SUMS',
                 'scorable-assistance-contract-20261111/SHA256SUMS',
                 'offline-user-acceptance-integration-20261111/delivery-SHA256SUMS']:
        entries = (ROOT / '.planning/plans' / name).read_text().splitlines()
        for line in entries:
            expected_hash, path = line.split('  ', 1)
            assert digest(ROOT / path) == expected_hash, path
        seals[name] = len(entries)
    assert not git('diff', 'HEAD', '--name-only')
    print(json.dumps(dict(kind='metadata-only final design consistency checks; not enforcement proof',
                         sourceHead=proposal['sourceHead'], sourceHashesChecked=len(bindings['files']),
                         membership=expected, sealEntries=seals, resourceMeasurementsReproduced=measurements,
                         designBindings={str(p.relative_to(ROOT)): digest(p) for p in sorted(HERE.rglob('*'))
                                         if p != HERE / 'validation.json' and p.is_file()},
                         reviews='See review.md for independent findings, rechecks and author dispositions; no implementation proof',
                         trackedWorktreeAndIndexUnchanged=True,
                         noEvaluationOrMediaOrCredentialAccess=True), indent=2))


if __name__ == '__main__':
    main()
