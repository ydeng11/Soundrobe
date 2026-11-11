"""Offline catalog-value observation scoring; never authorizes or certifies writes."""
import collections
import hashlib
import json
import pathlib
import sys


def checked_bytes(root, binding):
    relative = pathlib.PurePosixPath(binding['path'])
    if relative.is_absolute() or '..' in relative.parts:
        raise ValueError('binding must be checkout relative')
    path = (root / str(relative)).resolve()
    if not path.is_relative_to(root.resolve()):
        raise ValueError('binding escaped checkout')
    raw = path.read_bytes()
    if hashlib.sha256(raw).hexdigest() != binding['sha256']:
        raise ValueError('changed source binding')
    return raw


def checked_json(root, binding):
    return json.loads(checked_bytes(root, binding))


def pointer(value, selector):
    for key in selector.strip('/').split('/'):
        value = value[int(key)] if isinstance(value, list) else value[key.replace('~1', '/').replace('~0', '~')]
    return value


def indexed_tracks(tracks, expected):
    if not isinstance(tracks, list):
        raise ValueError('available reader tracks required')
    result = {}
    for track in tracks:
        if track.get('codec') == 'unknown':
            raise ValueError('unreadable-track fallback is not metadata readback')
        path = track['path']
        relative = pathlib.PurePosixPath(path)
        if relative.is_absolute() or '..' in relative.parts or str(relative) != path or path in result:
            raise ValueError('invalid or duplicate physical path')
        for field in ['title', 'genre', 'artists']:
            value = track[field]
            if field == 'artists':
                if not isinstance(value, list) or any(not isinstance(v, str) for v in value):
                    raise ValueError('invalid structured artist readback')
            elif value is not None and not isinstance(value, str):
                raise ValueError('invalid field readback')
        result[path] = track
    if set(result) != set(expected):
        raise ValueError('physical occurrence membership mismatch')
    return result


def freeze_native_contract(root, spec, records):
    """Freeze ALL requested slots from independent sources and prewrite observations only."""
    if spec['version'] != 1 or spec['fields'] != ['title', 'artists', 'genre'] or spec['phases'] != ['cold', 'warm']:
        raise ValueError('unsupported projection contract')
    bindings = spec['bindings']
    for binding in bindings.values():
        checked_bytes(root, binding)
    corpus = checked_json(root, bindings['corpus'])
    labels = checked_json(root, bindings['labels'])
    packet = checked_json(root, bindings['packet'])
    checked_json(root, bindings['expectations'])
    if labels['parentPacketSha256'] != bindings['packet']['sha256']:
        raise ValueError('machine labels have another source packet')
    sources = {key: checked_json(root, {'path': str(pathlib.PurePosixPath(packet['sourceRoot']) / value['path']),
                                      'sha256': value['sha256']})
               for key, value in packet['sourceRegister'].items()}
    expected_jobs = [f'{case}/{spec["profile"]}/{phase}' for phase in spec['phases'] for case in spec['caseIds']]
    jobs = {f'{r["caseId"]}/{r["profile"]}/{r["phase"]}': r for r in records}
    if len(set(expected_jobs)) != len(expected_jobs) or len(jobs) != len(records) or set(jobs) != set(expected_jobs):
        raise ValueError('requested job/phase membership mismatch')
    cases = {c['caseId']: c for c in corpus['cases']}
    if len(cases) != len(corpus['cases']):
        raise ValueError('duplicate corpus case')
    slots, frozen_jobs = [], []
    for job_id in expected_jobs:
        row = jobs[job_id]
        case = cases[row['caseId']]
        obs = row['fieldObservations']
        if row['providerMode'] != 'offline_fixtures' or obs['sourceRoot'] != spec['sourceRoot'] or row['sourceRelativeFolder'] != case['sourceRelativeFolder']:
            raise ValueError('source root, provider or case folder mismatch')
        raw = obs['inputJson']
        if hashlib.sha256(raw.encode()).hexdigest() != obs['inputSha256']:
            raise ValueError('corrupted native prewrite observation')
        before = json.loads(raw)
        paths = [t['relativePath'] for t in case['tracks']]
        if len(set(paths)) != len(paths) or set(before['hashes']) != set(paths):
            raise ValueError('prewrite hash membership mismatch')
        for hashes in before['hashes'].values():
            if not isinstance(hashes, dict) or any(
                not isinstance(hashes.get(key), str) or len(hashes[key]) != 64
                or any(c not in '0123456789abcdef' for c in hashes[key])
                for key in ['fullSha256', 'payloadSha256']
            ) or type(hashes.get('bytes')) is not int or hashes['bytes'] < 0:
                raise ValueError('malformed prewrite hash record')
        inventories = [c for source in sources.values() for c in source.get('cases', [])
                       if c['caseId'] == row['caseId'] and c['sourceRelativeFolder'] == case['sourceRelativeFolder']]
        if len(inventories) != 1:
            raise ValueError('physical case inventory is ambiguous or missing')
        originals = {t['relativePath']: t for t in inventories[0]['tracks']}
        mapped = [t['originalRelativePath'] for t in case['tracks']]
        if len(originals) != len(inventories[0]['tracks']) or len(set(mapped)) != len(mapped) or set(mapped) != set(originals):
            raise ValueError('generated-to-original mapping must be complete and one-to-one')
        for captured in case['tracks']:
            original = captured['originalRelativePath']
            if str(pathlib.PurePosixPath(original)) != original or pathlib.PurePosixPath(original).is_absolute() or '..' in pathlib.PurePosixPath(original).parts:
                raise ValueError('invalid original occurrence path')
            if originals[original]['sourceSha256'] != captured['originalSourceSha256']:
                raise ValueError('generated media does not bind captured identity')
        tracks = indexed_tracks(before['tracks'], paths)
        frozen_jobs.append({'id': job_id, 'inputSha256': obs['inputSha256'],
                            'requestSha256': hashlib.sha256(obs['requestJson'].encode()).hexdigest()})
        for captured in case['tracks']:
            relative = captured['relativePath']
            matches = []
            for label in labels['occurrences']:
                if label['caseId'] != row['caseId']:
                    continue
                originals = [v for v in packet['catalogLabels'] if all(v[k] == label[k] for k in ['caseId', 'inventorySource', 'inventoryOccurrence'])]
                if len(originals) != 1:
                    raise ValueError('label occurrence does not bind reviewed source')
                original = originals[0]
                inventory_case_selector = label['inventoryOccurrence'].split('/tracks/')[0]
                inventory = sources[label['inventorySource']]
                inventory_case = pointer(inventory, inventory_case_selector)
                inventory_track = pointer(inventory, label['inventoryOccurrence'])
                if inventory_case['caseId'] != row['caseId'] or inventory_case['sourceRelativeFolder'] != case['sourceRelativeFolder']:
                    raise ValueError('source label case/folder mismatch')
                if inventory_track['relativePath'] != original['physicalRelativePath'] or inventory_track['sourceSha256'] != original['capturedMediaSha256Claim']:
                    raise ValueError('source label physical identity mismatch')
                if captured['originalRelativePath'] == original['physicalRelativePath']:
                    if captured['originalSourceSha256'] != original['capturedMediaSha256Claim']:
                        raise ValueError('captured and generated media identities conflated')
                    matches.append(label)
            if len(matches) > 1:
                raise ValueError('duplicate source label')
            label = matches[0] if matches else None
            for field in spec['fields']:
                accepted = None if not label or field == 'genre' else label[
                    'acceptedTitleValues' if field == 'title' else 'acceptedArtistLists']
                slots.append({'id': f'{job_id}/{relative}/{field}', 'job': job_id, 'path': relative, 'field': field,
                              'original': tracks[relative][field], 'acceptedValues': accepted,
                              'generatedSourceSha256': captured['sourceSha256'],
                              'capturedMediaSha256Claim': captured['originalSourceSha256'],
                              'labelOccurrence': label['inventoryOccurrence'] if label else None})
    # Own the snapshot; subsequent caller mutation must not update frozen authority.
    return json.loads(json.dumps({'version': 1, 'spec': spec, 'jobs': frozen_jobs, 'slots': slots,
        'labelCoverage': {'uniqueOccurrenceFieldLabels': len({(s['labelOccurrence'], s['field'], s['job'].split('/')[0])
                            for s in slots if s['acceptedValues'] is not None}),
                          'replicatedLabelableSlots': sum(s['acceptedValues'] is not None for s in slots)}}))


def project_native(root, contract, records, manifest=None):
    """Validate native membership/baselines before making any scorer observations."""
    try:
        if freeze_native_contract(root, contract['spec'], records) != contract:
            raise ValueError('native baseline/request or frozen contract changed')
        if manifest is not None:
            raw_manifest = checked_bytes(root, manifest)
            manifest_digest = hashlib.sha256(raw_manifest).hexdigest()
            approved_manifest = json.loads(raw_manifest)
            for binding in approved_manifest['bindings'].values():
                checked_bytes(root, binding)
            approved = checked_json(root, approved_manifest['bindings']['scoringContract'])
            if approved != contract or approved_manifest['mode'] != 'mock':
                raise ValueError('manifest does not bind scoring contract')
            if {j['id']: j['inputSha256'] for j in approved_manifest['jobs']} != {j['id']: j['inputSha256'] for j in contract['jobs']}:
                raise ValueError('manifest job/profile inputs differ')
            for key, value in contract['spec']['transport'].items():
                if approved_manifest[key] != value:
                    raise ValueError('manifest transport/resource scope changed')
            for key, value in contract['spec']['bindings'].items():
                if approved_manifest['bindings'].get(key) != value:
                    raise ValueError('manifest source/profile/label binding differs')
            if approved_manifest['bindings']['input'] != contract['spec']['bindings']['corpus']:
                raise ValueError('manifest corpus differs')
            for row in records:
                if row['boundedAssistance']['manifestSha256'] != manifest_digest or row['profileInputSha256'] != row['fieldObservations']['inputSha256']:
                    raise ValueError('native invocation not bound to frozen manifest')
        by_job = {f'{r["caseId"]}/{r["profile"]}/{r["phase"]}': r for r in records}
        readbacks = {}
        for job in contract['jobs']:
            obs = by_job[job['id']]['fieldObservations']
            if type(obs['readbackAvailable']) is not bool:
                raise ValueError('readback availability must be explicit')
            expected = [s['path'] for s in contract['slots'] if s['job'] == job['id'] and s['field'] == 'title']
            if obs['readbackAvailable']:
                if obs['readbackError'] is not None:
                    raise ValueError('available readback conflicts with reader error')
                readbacks[job['id']] = indexed_tracks(obs['tracks'], expected)
            elif obs['tracks'] is not None or not obs['readbackError']:
                raise ValueError('unavailable readback requires a reader error and no invented values')
        observations = []
        for slot in contract['slots']:
            row = by_job[slot['job']]
            available = row['fieldObservations']['readbackAvailable']
            if type(row['serviceFailureObserved']) is not bool:
                raise ValueError('service failure observation must be explicit')
            observations.append({'id': slot['id'], 'readbackAvailable': available,
                'readback': readbacks[slot['job']][slot['path']][slot['field']] if available else None,
                'serviceFailure': row['serviceFailureObserved'] or row['outcome'] in ['failed', 'resolver_error', 'timeout', 'cancelled', 'timed_out', 'admission_denied'] or row['cancellationRequested'],
                'decisionVisibility': 'unavailable'})
        result = score(contract, observations)
        result['requestedJobs'] = len(contract['jobs'])
        result['requestedPhysicalOccurrences'] = sum(s['field'] == 'title' for s in contract['slots'])
        result['nativeOutcomes'] = dict(collections.Counter(r['outcome'] for r in records))
        return result
    except (KeyError, TypeError, IndexError) as error:
        raise ValueError('incomplete native/source projection input') from error


def score(contract, observations):
    slots = contract['slots']
    ids = [slot['id'] for slot in slots]
    observed_ids = [row['id'] for row in observations]
    if len(set(ids)) != len(ids) or len(set(observed_ids)) != len(observed_ids) or set(ids) != set(observed_ids):
        raise ValueError('requested membership mismatch')
    by_id = {row['id']: row for row in observations}
    fields = []
    for slot in slots:
        row = by_id[slot['id']]
        if row['decisionVisibility'] != 'unavailable':
            raise ValueError('explicit decision scoring is not implemented')
        if type(row['readbackAvailable']) is not bool or type(row['serviceFailure']) is not bool:
            raise ValueError('observation visibility must be explicit')
        available = row['readbackAvailable']
        if available and 'readback' not in row:
            raise ValueError('available readback requires an explicit value')
        alternatives = slot['acceptedValues']
        if alternatives is not None and (not isinstance(alternatives, list) or not alternatives):
            raise ValueError('incomplete accepted-value label')
        value = row.get('readback')
        before = slot['original']
        agreement = value in alternatives if available and alternatives is not None else None
        if not available:
            outcome = 'observation_unknown'
        elif alternatives is None:
            outcome = 'unscorable'
        elif value in (None, '', []):
            outcome = 'missing'
        elif value == before:
            outcome = 'unchanged_unknown'
        elif not agreement:
            outcome = 'wrong_catalog_change'
        elif before in (None, '', []):
            outcome = 'correct_catalog_fill'
        else:
            outcome = 'correct_catalog_change'
        fields.append({'id': slot['id'], 'outcome': outcome, 'catalogValueAgreement': agreement,
                       'valueTransition': ('unchanged' if value == before else 'changed') if available else 'unknown',
                       'decisionVisibility': 'unavailable', 'serviceFailure': row['serviceFailure']})
    return {'requestedFieldSlots': len(slots),
            'predeclaredLabelableSlots': sum(s['acceptedValues'] is not None for s in slots),
            'observedFieldSlots': sum(r['readbackAvailable'] for r in observations),
            'serviceFailureSlots': sum(r['serviceFailure'] for r in observations),
            'outcomes': dict(collections.Counter(f['outcome'] for f in fields)), 'fields': fields,
            'explicitDecisionSlots': 0, 'groundedAssistanceAccuracyCredit': 0,
            'outcome': 'insufficient', 'writableDelta': [], 'acceptedCertificate': False}


if __name__ == '__main__':
    if sys.argv[1] in ['freeze-native', 'project-native']:
        mode, source_path, *args = sys.argv[1:]
        root = pathlib.Path.cwd()
        if mode == 'freeze-native':
            records_path, output_path = args
            result = freeze_native_contract(root, json.loads(pathlib.Path(source_path).read_text()),
                                            json.loads(pathlib.Path(records_path).read_text()))
        else:
            expected, records_path, manifest_path, output_path = args
            raw = pathlib.Path(source_path).read_bytes()
            if hashlib.sha256(raw).hexdigest() != expected:
                raise ValueError('changed frozen projection contract')
            manifest_raw = pathlib.Path(manifest_path).read_bytes()
            manifest_binding = {'path': str(pathlib.Path(manifest_path).resolve().relative_to(root)),
                                'sha256': hashlib.sha256(manifest_raw).hexdigest()}
            result = project_native(root, json.loads(raw), json.loads(pathlib.Path(records_path).read_text()), manifest_binding)
        with pathlib.Path(output_path).open('x') as output:
            json.dump(result, output, ensure_ascii=False, indent=2)
            output.write('\n')
        sys.exit(0)
    # This digest is a caller's frozen binding, not an authenticated approval.
    contract_path, expected_sha256, observations_path, output_path = sys.argv[1:]
    raw = pathlib.Path(contract_path).read_bytes()
    if hashlib.sha256(raw).hexdigest() != expected_sha256:
        raise ValueError('changed scoring contract')
    result = score(json.loads(raw), json.loads(pathlib.Path(observations_path).read_text()))
    with pathlib.Path(output_path).open('x') as output:
        json.dump(result, output, ensure_ascii=False, indent=2)
        output.write('\n')
