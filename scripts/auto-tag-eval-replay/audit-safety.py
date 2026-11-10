"""Independent offline safety gate. Scripted outputs earn no accuracy credit."""
import hashlib
import json
import pathlib
import sys
import tarfile


def digest(data):
    return hashlib.sha256(data).hexdigest()


def genre_absence_matches(original, controlled):
    if isinstance(original, dict):
        keys = original.keys() - {'genre', 'genres', 'style', 'styles'}
        return (isinstance(controlled, dict) and controlled.keys() == keys
                and all(genre_absence_matches(original[key], controlled[key]) for key in keys))
    if isinstance(original, list):
        return (isinstance(controlled, list) and len(original) == len(controlled)
                and all(genre_absence_matches(a, b) for a, b in zip(original, controlled)))
    return original == controlled and isinstance(original, bool) == isinstance(controlled, bool)


def audit_record(record, job, evidence, physical):
    findings, ledger = [], []

    def require(ok, category, detail):
        if not ok:
            findings.append(dict(category=category, detail=detail))

    native = record.get('native') or {}
    candidate = native.get('candidate') or {}
    before, after = record.get('beforeTracks', []), record.get('readbackTracks', [])
    require(record.get('payloadUnchanged') and record.get('readback'), 'write_readback', 'payload or native readback failed')
    require(len(before) == len(after) == len(physical), 'physical_mapping', 'missing physical file')
    require([t['path'] for t in before] == [t['path'] for t in after], 'physical_mapping', 'file binding changed')
    require(record.get('providerMode') == 'offline_fixtures', 'network_safety', 'non-offline provider mode')
    require(record.get('cacheBefore', {}) != {} or record['phase'] != 'cold', 'cache_isolation', 'missing initial cache audit')
    cancelled = job['scenario'] == 'cancelled'
    require(bool(record.get('cancellationRequested')) == cancelled, 'cancellation', 'prewrite cancellation not exercised')
    if job.get('noWrite'):
        require(not native.get('written') and record.get('profileBaseline') == record.get('after'),
                'field_authorization', 'held/cancelled case changed file bytes')
        require(before == after, 'field_authorization', 'held/cancelled case changed tags')
        require(record.get('actionableReviewAfterRestart', False) == (not cancelled), 'review_highlight', 'held review state not persisted')
        if cancelled:
            require('cancel' in (record.get('error') or '').lower(), 'cancellation', 'wrong resolver failure')
        else:
            require(native.get('outcome') == 'needs_review', 'review_highlight', 'unsupported case not held')
        require(not candidate.get('assisted_genre'), 'source_grounding', 'unwritten genre mislabeled applied')
        for original, actual in zip(before, after):
            for field in sorted(set(original) | set(actual)):
                ledger.append(dict(file=pathlib.Path(actual['path']).name, field=field,
                                   original=original.get(field), readback=actual.get(field),
                                   written=actual.get(field) if native.get('written') else None,
                                   provenance='unsupported_proposal' if native.get('written') else 'retained_local',
                                   needsReview=not cancelled))
        return findings, ledger

    require(native.get('written') == len(physical) and native.get('authority') in ['musicbrainz', 'discogs'],
            'provider_selection', 'provider identity lost')
    should_infer = (job['name'] == 'mixed-accepted' or job['name'] == 'incomplete-duration'
                    or (job['name'] == 'collaborator-disagreement' and record['profile'] == 'folder_filename'
                        and not job.get('providerGenre')))
    provenance = candidate.get('assisted_genre')
    require(bool(provenance) == should_infer, 'source_grounding', 'missing/extra assisted field provenance')
    should_review = should_infer or candidate.get('verification') == 'album_content'
    require(record.get('reviewPendingAfterRestart') == should_review, 'review_highlight', 'persisted assisted/content review mismatch')
    require(native.get('outcome') == ('needs_review' if should_infer else 'applied'), 'review_highlight', 'wrong written review outcome')
    if provenance:
        require(provenance.get('model') == 'scripted/genre-v1' and provenance.get('value') == 'Pop',
                'source_grounding', 'scripted inference mislabeled')
        require((record.get('persistedEvidence') or {}).get('candidate', {}).get('assisted_genre') == provenance,
                'review_highlight', 'assisted field evidence lost on restart')
    diagnostics = native.get('diagnostics') or []
    receipts = [row for diagnostic in diagnostics
                for row in ([diagnostic] + (diagnostic.get('fieldEvidence') or []))]
    persisted_receipts = [row for diagnostic in (record.get('persistedEvidence') or {}).get('diagnostics', [])
                          for row in ([diagnostic] + (diagnostic.get('fieldEvidence') or []))]
    for receipt in receipts:
        if receipt.get('stage') in ['provider_field', 'provider_track_credits']:
            require(receipt in persisted_receipts, 'review_highlight', 'field receipt lost or changed on restart')
    genre_receipt = next((row for row in receipts if row.get('stage') == 'provider_field'
                          and row.get('field') == 'genre'), None)
    if job.get('providerGenre'):
        require(bool(genre_receipt), 'source_grounding', 'provider genre has no source receipt')
        source = evidence.get('genreSource')
        if source and genre_receipt:
            require(genre_receipt.get('provider') == source['provider']
                    and genre_receipt.get('releaseId') in source['releaseIds']
                    and genre_receipt.get('value') == source['value'],
                    'source_grounding', 'genre receipt is outside reviewed content evidence')
    for index, (original, actual, source) in enumerate(zip(before, after, physical)):
        filename = source['relativePath']
        require(pathlib.Path(actual['path']).name == filename, 'physical_mapping', filename)
        require((actual.get('title') or '').casefold() == evidence['titles'][index].casefold(),
                'identity_version', f'{filename}: reviewed title/version changed')
        require([name.casefold() for name in actual.get('artists', [])] == [name.casefold() for name in evidence['artists'][index]],
                'collaborator_credit', f'{filename}: missing or changed reviewed performers')
        original_genre = original.get('genre')
        expected_genre = (job.get('genres') or {}).get(filename)
        if expected_genre is not None:
            require(original_genre == expected_genre, 'input_snapshot', f'{filename}: missing explicit user decision')
            require(actual.get('genre') == expected_genre, 'field_authorization', f'{filename}: explicit genre decision overwritten')
        elif job.get('providerGenre'):
            require(actual.get('genre') == job['providerGenre'], 'field_authorization', f'{filename}: provider genre not used')
        elif should_infer:
            require(actual.get('genre') == 'Pop', 'field_authorization', f'{filename}: missing inferred fill')
        elif job['caseId'] == '60ebec7ffadb':
            require(actual.get('genre') == original_genre, 'field_authorization', f'{filename}: rejected inference wrote a genre')
        credit_receipt = next((row for row in receipts if row.get('stage') == 'provider_track_credits'
                               and row.get('localIndex') == index), None)
        if credit_receipt:
            require(actual.get('artists') == credit_receipt.get('artists'),
                    'source_grounding', f'{filename}: performer donor differs from readback')
        if genre_receipt and expected_genre is None:
            require(actual.get('genre') == genre_receipt.get('value'),
                    'source_grounding', f'{filename}: genre donor differs from readback')
        # Values are recorded by physical file. This is not a blanket claim that
        # every edition-dependent field is independently semantically verified.
        for field in sorted(set(original) | set(actual)):
            receipt = (credit_receipt if field in ['artist', 'artists']
                       else genre_receipt if field == 'genre' and expected_genre is None else None)
            ledger.append(dict(file=filename, field=field, original=original.get(field), readback=actual.get(field),
                               written=actual.get(field) if native.get('written') else None,
                               provenance=('local_user' if field == 'genre' and expected_genre is not None
                                           else 'scripted_unverified' if field == 'genre' and should_infer
                                           else 'retained_local' if original.get(field) == actual.get(field)
                                           else f"{receipt['provider']}_proposal" if receipt
                                           else f"{native.get('authority')}_proposal"),
                               sourceReceipt=receipt,
                               needsReview=field == 'genre' and should_infer and not (original_genre or '').strip()))
    return findings, ledger


def main(root):
    root = pathlib.Path(root)
    load = lambda file: json.loads(file.read_text())
    snapshot, selection = load(root / 'snapshot.json'), load(root / 'selection.json')
    findings, records, ledgers = [], [], []

    def require(ok, job, category, detail):
        if not ok:
            findings.append(dict(job=job, category=category, detail=detail))

    require(digest((root / 'selection.json').read_bytes()) == snapshot['selectionSha256'], 'selection', 'input_snapshot', 'oracle changed')
    require(digest((root / 'native-evaluator').read_bytes()) == snapshot['binarySha256'], 'binary', 'input_snapshot', 'binary changed')
    require(digest((root / 'source.tgz').read_bytes()) == snapshot['sourceArchiveSha256'], 'source', 'input_snapshot', 'archive changed')
    with tarfile.open(root / 'source.tgz') as archive:
        for file, expected in snapshot['sourceLocks'].items():
            require(digest(archive.extractfile(file).read()) == expected, file, 'input_snapshot', 'frozen source changed')
    if snapshot.get('harnessLocks'):
        require(digest((root / 'harness.tgz').read_bytes()) == snapshot['harnessArchiveSha256'],
                'harness', 'input_snapshot', 'harness archive changed')
        with tarfile.open(root / 'harness.tgz') as archive:
            for file, expected in snapshot['harnessLocks'].items():
                require(digest(archive.extractfile(file).read()) == expected, file, 'input_snapshot', 'frozen harness changed')
    for file, expected in snapshot['locks'].items():
        local = root / pathlib.Path(file).name
        source = local if file.startswith('scripts/auto-tag-eval-replay/safety-') and local.exists() else pathlib.Path(file)
        require(digest(source.read_bytes()) == expected, file, 'input_snapshot', 'input changed')
    provider_requests, model_requests = 0, 0
    for job in selection['jobs']:
        if snapshot.get('jobFilter') and job['name'] not in snapshot['jobFilter']:
            continue
        for profile in selection['profiles']:
            name = f"{job['name']}-{profile}"
            directory = root / name
            try:
                inventory, execution = load(directory / 'inventory.json'), load(directory / 'execution.json')
                manifest = load(directory / 'providers.json')
                require(execution['exitCode'] == 0 and not execution['signal'], name, 'evaluator', 'native process failed')
                require(not inventory['misses'], name, 'provider_coverage', 'unexpected request; not a semantic abstention')
                control = load(directory / 'job-lock.json')
                require(digest((directory / 'input.json').read_bytes()) == control['inputSha256']
                        and digest((directory / 'providers.json').read_bytes()) == control['manifestSha256'],
                        name, 'input_snapshot', 'job controls changed')
                if job.get('providerGenreAbsent'):
                    for row in manifest['records']:
                        if row['provider'] != 'discogs':
                            continue
                        source = row.get('source') or {}
                        if source.get('kind') == 'scripted_genre_absence':
                            raw = (directory / source['originalBodyFile']).read_bytes()
                            require(digest(raw) == source['originalSha256']
                                    and genre_absence_matches(json.loads(raw), row['body']),
                                    name, 'input_snapshot', 'genre absence changed other provider evidence')
                        else:
                            body = row.get('body') if 'body' in row else load(directory / row['bodyFile'])
                            require(genre_absence_matches(body, body), name, 'input_snapshot', 'provider genre not removed')
                provider_requests += len(inventory['requests'])
                model_requests += len(inventory['modelRequests'])
                model_expected = (job['name'] in ['mixed-accepted', 'mixed-rejected', 'mixed-failed', 'mixed-cancelled', 'incomplete-duration']
                                  or (job['name'] == 'collaborator-disagreement' and profile == 'folder_filename'
                                      and not job.get('providerGenre'))
                                  or job.get('wrongOnly')
                                  or (job['name'] == 'unproven-remaster' and profile != 'folder_filename'))
                require(len(inventory['modelRequests']) == (2 if model_expected else 0), name, 'agent_request', 'missing/extra inference requests')
                if job.get('wrongOnly'):
                    wrong_id = load(pathlib.Path('test/fixtures/tauri/auto-tag-eval') / selection['evidence'][job['caseId']]['wrongSnapshot'])['id']
                    require(any(request['path'] == f'/release/{wrong_id}' and request['status'] == 200 for request in inventory['requests']),
                            name, 'negative_coverage', 'incompatible concrete detail never evaluated')
                phases = []
                for phase in ['cold', 'warm']:
                    lines = (directory / f'{phase}.jsonl').read_text().splitlines()
                    require(len(lines) == 1, name, 'evaluator', f'{phase}: missing or duplicate evaluation')
                    if len(lines) != 1:
                        continue
                    record = json.loads(lines[0])
                    records.append(record)
                    case = next(case for case in snapshot['cases'] if case['caseId'] == job['caseId'])
                    evidence = dict(selection['evidence'][job['caseId']])
                    if job['caseId'] in selection.get('genreEvidence', {}):
                        evidence['genreSource'] = selection['genreEvidence'][job['caseId']]
                    failed, ledger = audit_record(record, job, evidence, case['tracks'])
                    findings.extend(dict(job=name, phase=phase, **item) for item in failed)
                    ledgers.append(dict(job=name, phase=phase, fields=ledger, proposed=(record.get('native') or {}).get('candidate'),
                                        evidence=selection['evidence'][job['caseId']], outcome=record['outcome']))
                    candidate = (record.get('native') or {}).get('candidate') or {}
                    if selection.get('oracleRevision') and (record.get('native') or {}).get('written'):
                        for field in ledger:
                            receipt = field.get('sourceReceipt')
                            if not receipt:
                                continue
                            endpoint = '/releases/' if receipt['provider'] == 'discogs' else '/release/'
                            require(any(row['provider'] == receipt['provider']
                                        and row['path'] == endpoint + receipt['releaseId'] and row['status'] == 200
                                        for row in inventory['requests']),
                                    name, 'source_grounding', f'{phase}: field source was not fetched')
                            source = next((row for row in manifest['records']
                                           if row['provider'] == receipt['provider']
                                           and row['path'] == endpoint + receipt['releaseId']), None)
                            require(bool(source and source.get('sha256') and source.get('bodyFile')
                                         and digest((directory / source['bodyFile']).read_bytes()) == source['sha256']),
                                    name, 'source_grounding', f'{phase}: field capture hash missing or changed')
                            if source:
                                field['sourceReceipt'] = dict(receipt, captureSha256=source.get('sha256'),
                                                              capture=source.get('source'))
                    if candidate.get('assisted_genre'):
                        request = inventory['modelRequests'][0 if phase == 'cold' else 1]['body']
                        payload = next(message['content'] for message in request['messages'] if message['role'] == 'user')
                        require(candidate['assisted_genre']['input_sha256'] == digest(payload.encode()), name, 'source_grounding', 'prompt hash mismatch')
                    if phase == 'cold':
                        require(all(value == 0 for value in record['cacheBefore'].values()), name, 'cache_isolation', 'cache not initially empty')
                    phases.append([{key: value for key, value in track.items() if key != 'path'} for track in record['readbackTracks']])
                require(len(phases) == 2 and phases[0] == phases[1], name, 'cache_isolation', 'cold/warm outputs differ')
            except (OSError, ValueError, KeyError, IndexError, StopIteration) as error:
                require(False, name, 'evaluator', str(error))
    evaluated_files = sum(len(record.get('beforeTracks', [])) for record in records)
    require(len(records) == snapshot['requestedEvaluations'] and evaluated_files == snapshot['requestedPhysicalFiles'],
            'denominator', 'evaluator', 'incomplete matrix')
    summary = dict(clean=not findings, oracleRevision=selection.get('oracleRevision', 'v1'),
                   checkerSha256=digest(pathlib.Path(__file__).read_bytes()),
                   evaluations=len(records), requestedEvaluations=snapshot['requestedEvaluations'],
                   physicalFileEvaluations=evaluated_files, uniquePhysicalFiles=snapshot['uniquePhysicalFiles'],
                   uniqueCases=len(snapshot['cases']), providerRequests=provider_requests, modelRequests=model_requests,
                   groundedAssistedAccuracyCredit=0, findings=findings,
                   limits='Inspected development cases and scripted negatives; not live model, acoustic or generalization evidence')
    (root / 'field-audit.json').write_text(json.dumps(ledgers, indent=2, ensure_ascii=False) + '\n')
    (root / 'audit.json').write_text(json.dumps(summary, indent=2, ensure_ascii=False) + '\n')
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    return 0 if summary['clean'] else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1]))
