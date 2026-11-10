"""Compare existing paired baseline with corrected retention; never grants authority."""
import gzip
import hashlib
import importlib.util
import json
import lzma
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[4]
CASE = '036e0a00e45b'
PROFILES = ['folder_filename', 'assisted_without_ids', 'tagged_recovery']
CHANGED = {'src-tauri/src/state/providers.rs', 'src-tauri/src/commands/auto_tag.rs',
           'src-tauri/src/commands/album_search.rs'}


def load(path):
    path = Path(path)
    if not path.exists():
        path = next(p for p in [Path(str(path) + '.gz'), Path(str(path) + '.xz')] if p.exists())
    opener = gzip.open if path.suffix == '.gz' else lzma.open if path.suffix == '.xz' else open
    with opener(path, 'rt') as stream:
        return json.load(stream)


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def normalize(value, sources, observations, path=''):
    if isinstance(value, dict):
        result = {}
        for key, item in value.items():
            if key in ['discogsArtistNameVariants', 'discogs_artist_name_variants']:
                assert value.get('source') == 'discogs', 'cross-provider attachment'
                assert item, 'empty evidence must be omitted'
                for pair in item:
                    release = pair['releaseId']
                    raw = sources[release]
                    artist = raw['artists'][pair['artistIndex']]
                    assert pair == dict(releaseId=str(raw['id']), artistIndex=pair['artistIndex'],
                                        artistId=str(artist['id']), canonicalName=artist['name'],
                                        creditedName=artist['anv']), 'wrong raw source/primary binding'
                    assert value.get('discogs_release_id') == release or (
                        value.get('verification') == 'album_content' and value.get('discogs_release_id') is None
                    ), 'wrong candidate release binding'
                observations.append(dict(path=path, wireKey=key, pairs=item))
            else:
                result[key] = normalize(item, sources, observations, path + '/' + key)
        return result
    if isinstance(value, list):
        return [normalize(v, sources, observations, path + '/' + str(i)) for i, v in enumerate(value)]
    if isinstance(value, str):
        return re.sub(r'/private/tmp/soundrobe-auto-tag-eval-[0-9a-f-]{36}/', '$TEMP/', value)
    return value


def main(before, after, output):
    before, after = Path(before), Path(after)
    plans = [load(root / 'execution-plan.json') for root in [before, after]]
    assert plans[0]['jobs'] == plans[1]['jobs']
    assert load(plans[0]['providers']) == load(plans[1]['providers'])
    assert load(plans[0]['packetSelection']) == load(plans[1]['packetSelection'])
    differences = {p for p in plans[0]['locks'] if plans[0]['locks'][p] != plans[1]['locks'][p]}
    assert differences == CHANGED, differences
    for i, plan in enumerate(plans):
        assert plan['assistance'] == 'disabled' and plan['modelRequestsMaximum'] == 0
        assert digest(plan['binary']) == plan['binarySha256']
        for p, expected in plan['locks'].items():
            if not (i == 0 and p in CHANGED):
                assert digest(ROOT / p) == expected, p
    assert plans[0]['binarySha256'] != plans[1]['binarySha256']
    sources = {}
    for route in load(plans[1]['providers'])['records']:
        if route['provider'] == 'discogs' and route['path'].startswith('/releases/'):
            body = load(Path(plans[1]['providers']).parent / route['bodyFile'])
            sources[str(body['id'])] = body
    spec = importlib.util.spec_from_file_location('exporter', ROOT / 'scripts/auto-tag-eval-replay/export-evidence.py')
    exporter = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(exporter)
    rows, changes = [], []
    for profile in PROFILES:
        dirs = [r / 'runs' / f'{profile}-{CASE}' for r in [before, after]]
        for directory in dirs:
            execution = load(directory / 'execution.json')
            assert execution['exitCode'] == 0 and not execution['signal']
            assert not execution['modelRequests'] and not execution['misses']
            assert load(directory / 'source-hashes-before.json') == load(directory / 'source-hashes-after.json')
            cold, warm = [load(directory / f'{phase}.jsonl') for phase in ['cold', 'warm']]
            assert not any(cold['cacheBefore'].values())
            assert warm['cacheBefore'] == cold['cacheAfter']
            assert cold['profileBaseline'] == warm['profileBaseline']
        assert load(dirs[0] / 'provider-inventory-final.json') == load(dirs[1] / 'provider-inventory-final.json')
        for phase in ['cold', 'warm']:
            for name in [f'{phase}.jsonl', f'requests-{phase}-{CASE}.json',
                         f'donors-{phase}-{CASE}.json', f'evidence-{phase}-{CASE}.json']:
                pair = [load(d / name) for d in dirs]
                if name.endswith('.jsonl'):
                    for record in pair:
                        assert record['assistanceMode'] == 'disabled' and record['providerMode'] == 'offline_fixtures'
                        assert record['payloadUnchanged'] and record['readback']
                        assert len(record['beforeTracks']) == len(record['readbackTracks']) == 11
                        record.pop('elapsedMs')
                if name.startswith('evidence-'):
                    hashes = [v.pop('compiledPolicySha256') for v in pair]
                    assert {k for k in hashes[0] if hashes[0][k] != hashes[1][k]} == {'auto_tag.rs'}
                    assert hashes[1]['auto_tag.rs'] == digest(ROOT / 'src-tauri/src/commands/auto_tag.rs')
                    for directory, snapshot in zip(dirs, pair):
                        exported = exporter.export_evidence(snapshot, directory / 'providers.json')
                        assert exported['decision'] == 'insufficient'
                        assert exported['acceptedCertificate'] is False and exported['writableDelta'] == []
                observations = [[], []]
                normalized = [normalize(value, sources, observations[i]) for i, value in enumerate(pair)]
                assert normalized[0] == normalized[1], (profile, phase, name, 'non-evidence changed')
                # This replay changes the wire spelling, not the underlying pairs or ownership.
                assert [{k:v for k,v in o.items() if k != 'wireKey'} for o in observations[0]] == [
                    {k:v for k,v in o.items() if k != 'wireKey'} for o in observations[1]], (profile, phase, name, 'unexpected evidence change')
                for left, right in zip(*observations):
                    assert left['wireKey'] == 'discogs_artist_name_variants' and right['wireKey'] == 'discogsArtistNameVariants'
                changes.append(dict(profile=profile, phase=phase, file=name, observations=len(observations[1]),
                                    allowedChange='outer vector wire casing only'))
            record = load(dirs[1] / f'{phase}.jsonl')
            rows.append(dict(profile=profile, phase=phase, written=record['native']['written'],
                             missingGenre=sum(not t.get('genre') for t in record['readbackTracks'])))
    result = dict(comparisonClean=True, overallSafetyClean=False, evaluationsPerArm=6,
                  repeatedObservationsPerArm=66, uniquePhysicalInputs=11, newModelRequests=0,
                  groundedAssistanceAccuracyCredit=0, rows=rows, evidenceChanges=changes,
                  excludedDifferences=['temporary UUID root', 'elapsedMs', 'compiled auto_tag.rs hash', 'outer observation wire spelling'])
    with Path(output).open('x') as stream:
        json.dump(result, stream, indent=2)
        stream.write('\n')
    print(json.dumps({k:v for k,v in result.items() if k != 'evidenceChanges'}, indent=2))


if __name__ == '__main__':
    main(*sys.argv[1:])
