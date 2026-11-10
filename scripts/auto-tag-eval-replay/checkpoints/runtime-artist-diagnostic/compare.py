"""Strict local paired audit. Receipts and observations never authorize writes."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[4]
SPEC = importlib.util.spec_from_file_location('retention', ROOT / 'scripts/auto-tag-eval-replay/checkpoints/primary-artist-retention/compare.py')
retention = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(retention)
load, digest = retention.load, retention.digest
CASE, PROFILES = retention.CASE, retention.PROFILES
CHANGED = retention.CHANGED | {'src-tauri/src/commands/auto_tag_eval.rs',
    'src-tauri/src/commands/auto_tag_evidence.rs', 'src-tauri/src/commands/mutations.rs',
    'src/shared/desktop-api.ts'}


def without_receipts(value, sources, receipts, path=''):
    if isinstance(value, dict):
        result = {}
        for key, item in value.items():
            if key == 'discogsReleaseReceipt':
                assert value.get('source') == 'discogs'
                release = item['resourceId']
                body, expected_hash = sources[release]
                assert item['resourceKind'] == 'release' and item['requestMethod'] == 'GET'
                assert item['requestPath'] == f'/releases/{release}' and item['responseStatus'] == 200
                assert item['responseSha256'] == expected_hash == hashlib.sha256(item['responseBody'].encode()).hexdigest()
                assert json.loads(item['responseBody']) == body
                assert item['requestUrl'] == item['responseUrl']
                endpoint = urlsplit(item['requestUrl'])
                assert endpoint.hostname == '127.0.0.1' and endpoint.scheme == 'http'
                assert not endpoint.username and not endpoint.password and not endpoint.query and not endpoint.fragment
                assert endpoint.path == f'/discogs/releases/{release}'
                assert value.get('discogs_release_id') == release or (
                    value.get('verification') == 'album_content' and value.get('discogs_release_id') is None)
                receipts.append(dict(path=path, release=release, sha256=expected_hash))
            else:
                result[key] = without_receipts(item, sources, receipts, path + '/' + key)
        return result
    if isinstance(value, list):
        return [without_receipts(v, sources, receipts, path + '/' + str(i)) for i, v in enumerate(value)]
    return value


def check_diagnostics(snapshot, profile):
    diagnostics = snapshot.pop('candidateIdentityDiagnostics')
    assert diagnostics
    supported = []
    for row in diagnostics:
        source = snapshot['nativeRouteTraces'][row['traceIndex']]['candidates'][row['candidateIndex']]
        identity = row['identity']
        assert identity['outcome'] == 'insufficient'
        assert identity['explicitDecisionVisibility'] == 'unavailable'
        assert row['nativeStrictError'] == source['strictError']
        assert row['nativeMappedVersions'] == source['versions']
        if identity['correspondence'] == 'supported':
            assert profile != 'folder_filename'
            assert identity['reason'] == 'explicit_decisions_unavailable'
            assert len(identity['originalPrimaryClaims']) == len(identity['originalPrimaryFields']) == 11
            assert all(v['albumArtist'] == '盧廣仲' for v in identity['originalPrimaryClaims'])
            assert [v['physicalPath'] for v in identity['originalPrimaryClaims']] == [v['path'] for v in snapshot['baselineTracks']]
            for fields, track in zip(identity['originalPrimaryFields'], snapshot['baselineTracks']):
                assert fields['path'] == track['path'] and fields['fields']
                assert fields['sha256'] == snapshot['baselineHashes'][Path(track['path']).name]['fullSha256']
                assert all(key in ['ALBUMARTIST', 'ALBUMARTISTS'] and value == '盧廣仲' for key, value in fields['fields'])
            receipt = source['candidate']['discogsReleaseReceipt']
            assert identity['origin'] == dict(requestPath=receipt['requestPath'], resourceId=receipt['resourceId'],
                                              responseSha256=receipt['responseSha256'], selector='/artists/0')
            supported.append(identity['origin']['resourceId'])
    if profile == 'folder_filename':
        assert not supported
    else:
        assert {'37231524', '37230261'} <= set(supported)
    return dict(rows=len(diagnostics), supportedCorrespondenceReleases=sorted(set(supported)),
                aggregate='insufficient', decisions='unavailable')


def main(before, after, output):
    roots = [Path(before), Path(after)]
    plans = [load(root / 'execution-plan.json') for root in roots]
    assert plans[0]['jobs'] == plans[1]['jobs']
    assert load(plans[0]['providers']) == load(plans[1]['providers'])
    assert load(plans[0]['packetSelection']) == load(plans[1]['packetSelection'])
    differences = {p for p in plans[0]['locks'] if plans[0]['locks'][p] != plans[1]['locks'][p]}
    assert differences == CHANGED & plans[0]['locks'].keys(), differences
    for index, plan in enumerate(plans):
        assert plan['assistance'] == 'disabled' and plan['modelRequestsMaximum'] == 0
        assert digest(plan['binary']) == plan['binarySha256']
        for path, expected in plan['locks'].items():
            if not (index == 0 and path in CHANGED):
                assert digest(ROOT / path) == expected, path
    sources = {}
    for record in load(plans[1]['providers'])['records']:
        if record['provider'] == 'discogs' and record['path'].startswith('/releases/'):
            path = Path(plans[1]['providers']).parent / record['bodyFile']
            assert digest(path) == record['sha256']
            body = load(path)
            sources[str(body['id'])] = body, digest(path)
    spec = importlib.util.spec_from_file_location('exporter', ROOT / 'scripts/auto-tag-eval-replay/export-evidence.py')
    exporter = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(exporter)
    results = []
    for profile in PROFILES:
        directories = [root / 'runs' / f'{profile}-{CASE}' for root in roots]
        for directory in directories:
            execution = load(directory / 'execution.json')
            assert execution['exitCode'] == 0 and not execution['signal']
            assert not execution['modelRequests'] and not execution['misses']
            assert load(directory / 'source-hashes-before.json') == load(directory / 'source-hashes-after.json')
            cold, warm = [load(directory / f'{phase}.jsonl') for phase in ['cold', 'warm']]
            assert not any(cold['cacheBefore'].values())
            assert warm['cacheBefore'] == cold['cacheAfter']
            assert cold['profileBaseline'] == warm['profileBaseline']
        assert load(directories[0] / 'provider-inventory-final.json') == load(directories[1] / 'provider-inventory-final.json')
        for phase in ['cold', 'warm']:
            receipt_count, diagnostic = 0, None
            for filename in [f'{phase}.jsonl', f'requests-{phase}-{CASE}.json',
                             f'donors-{phase}-{CASE}.json', f'evidence-{phase}-{CASE}.json']:
                pair = [load(directory / filename) for directory in directories]
                if filename.endswith('.jsonl'):
                    for record in pair:
                        assert record['assistanceMode'] == 'disabled' and record['providerMode'] == 'offline_fixtures'
                        assert record['payloadUnchanged'] and record['readback']
                        assert len(record['beforeTracks']) == len(record['readbackTracks']) == 11
                        record.pop('elapsedMs')
                if filename.startswith('evidence-'):
                    hashes = [v.pop('compiledPolicySha256') for v in pair]
                    assert {k for k in hashes[0] if hashes[0][k] != hashes[1][k]} == {'auto_tag.rs', 'auto_tag_eval.rs', 'auto_tag_evidence.rs'}
                    for name, expected in hashes[1].items():
                        assert digest(ROOT / 'src-tauri/src/commands' / name) == expected
                    diagnostic = check_diagnostics(pair[1], profile)
                    for directory, snapshot in zip(directories, pair):
                        exported = exporter.export_evidence(snapshot, directory / 'providers.json')
                        assert exported['decision'] == 'insufficient'
                        assert not exported['acceptedCertificate'] and exported['writableDelta'] == []
                receipts, observations = [[], []], [[], []]
                stripped = [without_receipts(value, sources, receipts[i]) for i, value in enumerate(pair)]
                assert not receipts[0]
                normalized = [retention.normalize(value, {key: v[0] for key, v in sources.items()}, observations[i]) for i, value in enumerate(stripped)]
                assert normalized[0] == normalized[1], (profile, phase, filename, 'non-evidence difference')
                assert observations[0] == observations[1], (profile, phase, filename, 'retained variants changed')
                receipt_count += len(receipts[1])
            assert receipt_count
            record = load(directories[1] / f'{phase}.jsonl')
            results.append(dict(profile=profile, phase=phase, written=record['native']['written'],
                                receiptOccurrences=receipt_count, diagnostics=diagnostic))
    result = dict(comparisonClean=True, overallSafetyClean=False, evaluationsPerArm=6,
                  repeatedTrackObservationsPerArm=66, uniquePhysicalInputs=11, modelRequests=0,
                  acceptedCertificate=False, writableDelta=[], groundedAssistanceAccuracyCredit=0,
                  exclusions=['temporary UUID roots', 'elapsedMs', 'three declared compiled hashes',
                              'verified originating receipt field', 'validated diagnostic-only observations'], rows=results)
    with Path(output).open('x') as stream:
        json.dump(result, stream, indent=2)
        stream.write('\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main(*sys.argv[1:])
