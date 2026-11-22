"""Independent fixed-denominator audit for run-paired.cjs; no network or media writes."""
import hashlib
import importlib.util
import json
import re
import sys
from pathlib import Path

spec = importlib.util.spec_from_file_location('titles', Path(__file__).with_name('score-best-aligned.py'))
titles = importlib.util.module_from_spec(spec)
spec.loader.exec_module(titles)
load = lambda file: json.loads(Path(file).read_text())
sha = lambda file: hashlib.sha256(Path(file).read_bytes()).hexdigest()


def audit_phase(context, records, case, truth, review, expected, require_no_write=False,
                source_fixture=None, inventory=None, require_assisted_write=False,
                require_no_assisted_obligation=False, expected_run_id=None,
                allow_legacy_scenario=False, withhold_cached_source=False):
    row = dict(**context, recovery='unavailable', readbackFailures=[], safetyFailures=[],
               missingFixtures=0, processFailure=False, skipped=False, unavailable=False,
               requiredNoWrite=require_no_write, providerPrecedenceControl=False,
               providerPrecedenceGenre=None, providerPrecedenceFailures=[],
               providerPrecedenceGenreFailures=[], providerPrecedenceSafetyFailures=[],
               providerPrecedenceUnavailable=[])
    if len(records) != 1:
        return row | dict(unavailable=True, reason='missing, duplicate or misbound native phase')
    record = records[0]
    if any(record.get(key) != value for key, value in context.items()
           if not (key == 'scenario' and allow_legacy_scenario and key not in record)):
        return row | dict(unavailable=True, reason='missing, duplicate or misbound native phase')
    if expected_run_id is not None and record.get('runId') != expected_run_id:
        return row | dict(unavailable=True, reason='native run ID does not match the runner-owned job')
    row.update(rawBeforeTracks=record.get('beforeTracks'),
               rawIndependentMetadataReadback=record.get('readbackTracks'),
               sourceMappingEvidence=record.get('genreSourceBindingEvidence'),
               reopenedFieldReviewObligations=record.get('fieldReviewObligationsAfterRestart'),
               jobScriptedModelPayloads=(inventory or {}).get('modelPayloads', []))
    native = record.get('native') or {}
    if withhold_cached_source:
        failures = cached_source_withholding_failures(record)
        row['safetyFailures'].extend(failures)
        row['unavailable'] = bool(failures)
        row['cachedSourceWithheld'] = not failures
    precedence_control, precedence_genre, precedence_genre_failures, precedence_safety_failures, precedence_unavailable = provider_precedence_check(
        record, context, case, source_fixture, inventory, require_no_assisted_obligation)
    precedence_failures = precedence_genre_failures + precedence_safety_failures + precedence_unavailable
    row.update(providerPrecedenceControl=precedence_control,
               providerPrecedenceGenre=precedence_genre,
               providerPrecedenceFailures=precedence_failures,
               providerPrecedenceGenreFailures=precedence_genre_failures,
               providerPrecedenceSafetyFailures=precedence_safety_failures,
               providerPrecedenceUnavailable=precedence_unavailable)
    row['readbackFailures'].extend(precedence_genre_failures)
    row['safetyFailures'].extend(precedence_safety_failures)
    if precedence_unavailable:
        row['unavailable'] = True
    mapping_failures = source_mapping_failures(record, case, truth, source_fixture, inventory)
    row['safetyFailures'].extend(mapping_failures)
    if mapping_failures:
        row['unavailable'] = True
    obligation_failures, review_persistence = field_obligation_observation(
        record, case, require_assisted_write=require_assisted_write)
    row['safetyFailures'].extend(obligation_failures)
    row['reviewPersistence'] = review_persistence
    if obligation_failures:
        row['unavailable'] = True
    row.update(outcome=record['outcome'], classification=record.get('classification'),
               written=native.get('written', 0), elapsedMs=record.get('elapsedMs'))
    if require_no_write and native.get('written'):
        row['safetyFailures'].append('required no-write control wrote metadata')
        row['recovery'] = 'wrong_match'
    expected_failure_control = context.get('scenario') == 'assistance_failure'
    if record.get('error') or (record.get('serviceFailureObserved') and not expected_failure_control) \
            or record['outcome'] not in ['applied', 'needs_review']:
        row.update(unavailable=True, reason=record.get('error') or 'incomplete provider/resolver work')
    if record.get('providerMode') != 'offline_fixtures' or record.get('mediaMode') != 'synthetic_flac':
        row['safetyFailures'].append('not offline synthetic replay')
    if not record.get('readback'):
        row['safetyFailures'].append('native write/readback mismatch')
    if not record.get('payloadUnchanged'):
        row['safetyFailures'].append('native payload mismatch')
    baseline, after = record.get('profileBaseline', {}), record.get('after', {})
    expected_files = [t['relativePath'] for t in case.get('tracks', [])]
    track_exports = {}
    for name in ['beforeTracks', 'readbackTracks']:
        exported = record.get(name, [])
        if not isinstance(exported, list) or not all(isinstance(t, dict) and isinstance(t.get('path'), str) for t in exported):
            exported = []
        track_exports[name] = exported
        if len(exported) != len(expected_files) or any(
                Path(track['path']).parts[-len(Path(file).parts):] != Path(file).parts
                for track, file in zip(exported, expected_files)):
            row['safetyFailures'].append(f'{name}: incomplete or misbound physical files')
            row['unavailable'] = True
            if name == 'readbackTracks':
                row['readbackFailures'].append('unavailable independent physical readback')
    for name in ['profileBaseline', 'after', 'copiedBeforeProfile']:
        if set(record.get(name, {})) != set(expected_files):
            row['safetyFailures'].append(f'{name}: incomplete or misbound physical hashes')
            row['unavailable'] = True
    if not baseline or baseline.keys() != after.keys():
        row['safetyFailures'].append('missing physical hash exports')
    for file, before in baseline.items():
        current = after.get(file)
        copied = record.get('copiedBeforeProfile', {}).get(file)
        if not all(isinstance(x, dict) and x.get('payloadSha256') for x in [before, current, copied]):
            row['safetyFailures'].append(f'{file}: unavailable independent payload hash')
        elif not before['payloadSha256'] == current['payloadSha256'] == copied['payloadSha256']:
            row['safetyFailures'].append(f'{file}: changed payload')
    tracks = track_exports['readbackTracks']
    changed_files = {file for file in expected_files
                     if isinstance(baseline.get(file), dict) and isinstance(after.get(file), dict)
                     and baseline[file].get('fullSha256') and after[file].get('fullSha256')
                     and baseline[file]['fullSha256'] != after[file]['fullSha256']}
    if expected_failure_control and not native.get('written'):
        row['safetyFailures'].append('scripted model failure control did not preserve provider writes')
        row['unavailable'] = True
        row['recovery'] = 'unavailable'
    if not native.get('written') and (after != baseline or tracks != record.get('beforeTracks')):
        row['safetyFailures'].append('unconfirmed write changed media or metadata')
        row['unavailable'] = True
    if not native.get('written') and not changed_files:
        if row['providerPrecedenceGenreFailures']:
            row['recovery'] = 'wrong_match'
        elif row['recovery'] != 'wrong_match':
            row['recovery'] = 'abstention' if not row['unavailable'] else 'unavailable'
        return row
    if changed_files and require_no_write and not native.get('written'):
        row['safetyFailures'].append('required no-write control changed media')
    mappings = {m['localTrack']: m for m in truth['mapping']}
    expected = expected_for_profile(expected, context.get('profile'))
    content_failures = []
    if native.get('written') and (len(tracks) != len(case['tracks']) or native['written'] != len(tracks)):
        row['readbackFailures'].append('incomplete physical-file write count')
        row['unavailable'] = True
    for index, track in enumerate(tracks[:len(case['tracks'])], 1):
        file = expected_files[index - 1]
        if (Path(track['path']).parts[-len(Path(file).parts):] != Path(file).parts
                or (not native.get('written') and file not in changed_files)):
            continue
        mapping = mappings[index]
        title = track_title_outcome(track.get('title'), mapping, case['artist'], context['caseId'],
                                    expected, track.get('musicbrainzTrackId'))
        if title['status'] != 'correct' or title['prefix']:
            content_failures.append(f'{index}: title {track.get("title")!r}')
        credits = expected['credits'][index - 1]
        if not performer_credits_match(track.get('artist'), track.get('artists', []), credits, expected):
            content_failures.append(f'{index}: ordered performer credits')
        if not album_title_matches(track.get('album'), expected):
            content_failures.append(f'{index}: album identity')
        album_artist_ok = album_credit_matches(track.get('albumArtist'), track.get('albumArtists'), expected) \
            if 'albumArtists' in expected else titles.key(track.get('albumArtist') or '') == titles.key(case['artist'])
        if not album_artist_ok:
            content_failures.append(f'{index}: album artist')
        if any(track.get(key) is not None and str(track[key]) in map(str, review.get('hardNegativeIds', []))
               for key in ['musicbrainzAlbumId', 'discogsReleaseId']):
            content_failures.append(f'{index}: reviewed hard-negative readback identity')
        original = record.get('beforeTracks', [])
        before = original[index - 1] if len(original) == len(case['tracks']) else {}
        positions = {name: before.get(name) or fallback for name, fallback in
                     [('trackNumber', index), ('trackTotal', len(case['tracks']))]}
        if any(track.get(k) != v for k, v in positions.items()):
            row['safetyFailures'].append(f'{index}: preserved/fallback numbering')
        if before and any(before.get(k) and track.get(k) != before[k] for k in ['discNumber', 'discTotal']):
            row['safetyFailures'].append(f'{index}: changed disc positions')
    candidate = native.get('candidate') or {}
    identity = candidate.get('musicbrainz_albumid') or candidate.get('discogs_release_id')
    row['selectedIdentity'] = identity
    row['edition'] = 'reviewed_representative' if str(identity) == str(review['releaseId']) else 'unresolved'
    row['reviewedLayoutMatches'] = len(tracks) == len(mappings) and all(
        track.get('trackNumber') == int(mapping['providerTrack'].split('-')[-1])
        and (len(mapping['providerTrack'].split('-')) == 1
             or (track.get('discNumber') or 1) == int(mapping['providerTrack'].split('-')[0]))
        for track, mapping in zip(tracks, truth['mapping']))
    if str(identity) in map(str, review.get('hardNegativeIds', [])):
        content_failures.append('reviewed hard-negative identity')
    if native.get('completionBasis') == 'album_content' and any(
            track.get(k) for track in tracks for k in ['musicbrainzAlbumId', 'musicbrainzTrackId', 'discogsReleaseId']):
        row['safetyFailures'].append('content-only output asserted edition/recording ID')
    row['readbackFailures'].extend(content_failures)
    row['recovery'] = ('wrong_match' if content_failures or row['providerPrecedenceGenreFailures']
                       or (require_no_write and (native.get('written') or changed_files))
                       else 'correct' if native.get('written') and not row['unavailable'] else 'unavailable')
    if expected_failure_control and not withhold_cached_source:
        requests = inventory.get('modelRequests', []) if isinstance(inventory, dict) else []
        if not requests or any(request.get('status') != 400 for request in requests) or not native.get('written'):
            row['safetyFailures'].append('scripted model failure did not preserve a provider metadata write')
            row['unavailable'] = True
            row['recovery'] = 'unavailable'
        else:
            row['assistanceFailureObserved'] = True
    return row


def cached_source_withholding_failures(record):
    before = record.get('beforeTracks')
    after = record.get('readbackTracks')
    obligations = record.get('fieldReviewObligationsAfterRestart')
    if not before or not isinstance(after, list) or len(before) != len(after) \
            or not isinstance(obligations, dict):
        return ['cached-source control lacks physical readback or reopened review evidence']
    paths = [track['path'] for track in before]
    candidate = (record.get('native') or {}).get('candidate') or {}
    if (candidate.get('assisted_genre') is not None or record.get('serviceFailureObserved')
            or set(obligations) != set(paths) or any(value != [] for value in obligations.values())
            or [(t['path'], t.get('genre')) for t in before] !=
               [(t['path'], t.get('genre')) for t in after]):
        return ['cached-source control did not withhold assistance for every physical file']
    return []


def provider_precedence_check(record, context, case, source_fixture, inventory,
                              require_no_assisted_obligation):
    if (not isinstance(source_fixture, dict)
            or context.get('caseId') != source_fixture.get('caseId')
            or not isinstance(source_fixture.get('providerPrecedenceReleaseResponse'), dict)
            or context.get('profile') == source_fixture.get('profile')
            or context.get('scenario')):
        return False, None, [], [], []
    genre_failures, safety_failures, unavailable = [], [], []
    body = source_fixture.get('providerPrecedenceReleaseResponse')
    release_id = str(source_fixture.get('releaseId', ''))
    genre_values = []
    if not isinstance(body, dict) or str(body.get('id')) != release_id:
        unavailable.append('locked provider-precedence release body is unavailable or misbound')
    else:
        for field in ['genres', 'styles']:
            values = body.get(field, [])
            if not isinstance(values, list):
                unavailable.append('locked provider-precedence genre/style value is malformed')
                continue
            for value in values:
                if not isinstance(value, str):
                    unavailable.append('locked provider-precedence genre/style value is malformed')
                    continue
                value = value.strip()
                if value and value not in genre_values:
                    genre_values.append(value)
    expected_genre = ', '.join(genre_values)
    route = source_fixture.get('route')
    expected_route = {'provider': 'discogs', 'path': f'/releases/{release_id}', 'query': {}}
    if route != expected_route or not expected_genre:
        unavailable.append('locked provider-precedence route or genre is unavailable')
    if not isinstance(inventory, dict):
        unavailable.append('provider-precedence loopback inventory is unavailable')
    else:
        expected_sha = hashlib.sha256(json.dumps(body, ensure_ascii=False,
            separators=(',', ':')).encode()).hexdigest() if isinstance(body, dict) else None
        routes = [item for item in inventory.get('records', [])
                  if item.get('provider') == 'discogs' and item.get('path') == expected_route['path']
                  and item.get('query') in ({}, [], '[]')]
        requests = [item for item in inventory.get('requests', [])
                    if item.get('provider') == 'discogs' and item.get('path') == expected_route['path']
                    and item.get('query') in ({}, [], '[]')]
        if len(routes) != 1 or routes[0].get('status') != 200 or routes[0].get('sha256') != expected_sha:
            unavailable.append('provider-precedence fixture response differs from the locked source body')
        if not requests or any(item.get('status') != 200 for item in requests):
            unavailable.append('provider-precedence release request did not return the locked response')
        if inventory.get('modelRequests') or inventory.get('modelPayloads') or any(
                item.get('method') == 'POST' and item.get('route') == '/model/chat/completions'
                for item in inventory.get('wireArrivals', [])):
            safety_failures.append('provider-precedence control made an unauthorized model request')
    candidate = ((record.get('native') or {}).get('candidate') or {})
    if candidate.get('assisted_genre') is not None:
        safety_failures.append('provider-precedence control produced an assisted genre proposal')
    before = record.get('beforeTracks')
    after = record.get('readbackTracks')
    if (not isinstance(before, list) or not isinstance(after, list)
            or len(before) != len(case.get('tracks', []))
            or len(after) != len(case.get('tracks', []))):
        unavailable.append('provider-precedence genre cannot be joined to every physical file')
    else:
        for index, (case_track, before_track, after_track) in enumerate(zip(case['tracks'], before, after), 1):
            relative_parts = Path(case_track['relativePath']).parts
            before_path = before_track.get('path') if isinstance(before_track, dict) else None
            after_path = after_track.get('path') if isinstance(after_track, dict) else None
            before_parts = Path(before_path).parts if isinstance(before_path, str) else ()
            if (not before_parts or before_path != after_path
                    or len(before_parts) < len(relative_parts)
                    or before_parts[-len(relative_parts):] != relative_parts):
                unavailable.append(f'{index}: provider-precedence physical file path is misbound')
                continue
            original = before_track.get('genre')
            expected_value = original if isinstance(original, str) and original.strip() else expected_genre
            if after_track.get('genre') != expected_value:
                genre_failures.append(f'{index}: provider-precedence genre readback differs from the locked value')
    if require_no_assisted_obligation:
        obligations = record.get('fieldReviewObligationsAfterRestart')
        if not isinstance(obligations, dict):
            unavailable.append('candidate provider-precedence control lacks reopened obligation export')
        else:
            expected_paths = {track.get('path') for track in before or [] if isinstance(track, dict)}
            if set(obligations) != expected_paths:
                unavailable.append('candidate provider-precedence obligation export does not match physical files')
            if any(not isinstance(values, list) for values in obligations.values()):
                unavailable.append('candidate provider-precedence obligation export has malformed rows')
            if any(item.get('field') == 'genre'
                   for values in obligations.values() if isinstance(values, list)
                   for item in values if isinstance(item, dict)):
                safety_failures.append('provider-precedence control created a genre review obligation')
    return True, expected_genre, genre_failures, safety_failures, unavailable


def scripted_model_failures(record, inventory, job, arm, source_fixture):
    if not isinstance(inventory, dict):
        return ['scripted model inventory unavailable']
    requests = inventory.get('modelRequests', [])
    payloads = inventory.get('modelPayloads', [])
    target = source_fixture and job.get('caseId') == source_fixture.get('caseId') \
        and job.get('profile') == source_fixture.get('profile')
    failure_control = job.get('scenario') == 'assistance_failure'
    if not target:
        return ['unexpected scripted model request outside declared source case'] if requests or payloads else []
    failures = []
    if not isinstance(requests, list) or not 1 <= len(requests) <= 2 or len(payloads) != len(requests):
        return ['source case model requests do not match the cold/warm allowance']
    expected_status = 400 if failure_control else 200
    cached_source_control = arm == 'candidate' and source_fixture.get('candidateWarmAssistance') == 'withhold_cached_source'
    if cached_source_control and len(requests) != 1:
        failures.append('cached-source job must make exactly one cold model request')
    if cached_source_control and record.get('phase') == 'warm':
        return failures
    if cached_source_control and failure_control and not record.get('serviceFailureObserved'):
        failures.append('cold failure control did not observe model failure')
    if any(item.get('status') != expected_status for item in requests):
        failures.append('scripted model status differs from declared scenario')
    expected_class = 'title_only_baseline' if arm == 'baseline' else 'synthetic_discogs_source'
    if any(item.get('payloadClass') != expected_class for item in requests):
        failures.append('scripted model payload class differs from implementation arm')
    expected_tracks = [item['title'] for item in source_fixture['successReleaseResponse']['tracklist']]
    for item in payloads:
        payload = item.get('payload')
        if arm == 'baseline':
            if (not isinstance(payload, dict) or set(payload) != {'artist', 'album', 'tracks'}
                    or payload.get('artist') != source_fixture['artist']
                    or payload.get('album') != source_fixture['album'] or payload.get('tracks') != expected_tracks):
                failures.append('baseline model payload differs from locked title-only contract')
        else:
            source = payload.get('sourceObservation') if isinstance(payload, dict) else None
            if (not isinstance(payload, dict) or set(payload) != {'artist', 'album', 'tracks', 'sourceObservation'}
                    or payload.get('artist') != source_fixture['artist']
                    or payload.get('album') != source_fixture['album'] or payload.get('tracks') != expected_tracks
                    or not isinstance(source, dict) or source.get('origin') != 'synthetic_loopback'
                    or source.get('provider') != 'discogs' or source.get('resourceId') != source_fixture['releaseId']
                    or source.get('responseSha256') != hashlib.sha256(json.dumps(
                        source_fixture['successReleaseResponse'], ensure_ascii=False, separators=(',', ':')).encode()).hexdigest()
                    or source.get('selector') != '/notes' or source.get('text') != source_fixture['sourceText']
                    or not re.fullmatch(r'[a-f0-9]{64}', source.get('trackMappingSha256', ''))):
                failures.append('candidate model payload differs from locked source-observation contract')
    assisted = (((record.get('native') or {}).get('candidate') or {}).get('assisted_genre') or {})
    if arm == 'candidate' and requests and not failure_control:
        if assisted.get('request_input') not in [item.get('payload') for item in payloads]:
            failures.append('candidate output does not retain the exact observed model payload')
    if not failure_control and inventory.get('misses'):
        failures.append('successful source scenario has unexpected fixture misses')
    return failures


def source_mapping_failures(record, case, truth, source_fixture, inventory):
    """Recompute a private runtime binding against locked notes and physical inputs."""
    native = record.get('native') or {}
    assisted = ((native.get('candidate') or {}).get('assisted_genre') or {})
    source = assisted.get('source_observation')
    exported = record.get('genreSourceBindingEvidence')
    if not source:
        return ['orphan source mapping export'] if exported else []
    failures = []
    if not isinstance(source_fixture, dict) or not isinstance(inventory, dict):
        return ['source mapping lacks locked fixture or loopback inventory']
    if not isinstance(exported, dict):
        return ['assisted source mapping export missing']
    try:
        body = source_fixture['successReleaseResponse']
        release_id = str(source_fixture['releaseId'])
        route = source_fixture.get('route')
        if (source_fixture.get('label') != 'synthetic_loopback_counterfactual'
                or route != {'provider': 'discogs', 'path': f'/releases/{release_id}', 'query': {}}):
            failures.append('source fixture route is not the declared synthetic release route')
        source_tracks = [track for track in body['tracklist'] if track.get('type_', 'track') == 'track']
        raw_body = json.dumps(body, ensure_ascii=False, separators=(',', ':')).encode()
        response_sha = hashlib.sha256(raw_body).hexdigest()
        canonical = lambda value: json.dumps(value, ensure_ascii=False, sort_keys=True,
                                              separators=(',', ':')).encode()
        digest = lambda value: hashlib.sha256(value).hexdigest()
        expected_keys = {'releaseId', 'requestTracks', 'mapping', 'mappingSha256'}
        if set(exported) != expected_keys:
            failures.append('source mapping export schema mismatch')
        binding = {key: exported[key] for key in ['releaseId', 'requestTracks', 'mapping']}
        mapping_sha = digest(canonical(binding))
        if exported.get('mappingSha256') != mapping_sha:
            failures.append('source mapping digest mismatch')
        if exported.get('releaseId') != release_id or source.get('resourceId') != release_id:
            failures.append('source mapping release identity mismatch')
        if source.get('provider') != 'discogs' or source.get('origin') != 'synthetic_loopback' \
                or source.get('selector') != '/notes':
            failures.append('source mapping route is not the locked synthetic Discogs notes route')
        if source.get('responseSha256') != response_sha or body.get('id') != int(release_id) \
                or source.get('text') != body.get('notes'):
            failures.append('source mapping source body or notes mismatch')
        if body.get('genres') or body.get('styles') or source.get('text') != source_fixture.get('sourceText'):
            failures.append('source mapping is not the declared absent-field notes scenario')
        if digest(str(body.get('notes', '')).encode()) != source.get('textSha256'):
            failures.append('source notes digest mismatch')
        source_observation = {key: source.get(key) for key in
                              ['provider', 'resourceId', 'responseSha256', 'selector',
                               'textSha256', 'trackMappingSha256']}
        if source.get('trackMappingSha256') != mapping_sha:
            failures.append('source observation does not bind exported mapping')
        if source.get('evidenceId') != digest(canonical(source_observation)):
            failures.append('source evidence digest mismatch')
        route = f'/releases/{release_id}'
        route_records = [item for item in inventory.get('records', [])
                         if item.get('provider') == 'discogs' and item.get('path') == route
                         and item.get('query') in ({}, [], '[]')]
        route_requests = [item for item in inventory.get('requests', [])
                          if item.get('provider') == 'discogs' and item.get('path') == route
                          and item.get('query') in ({}, [], '[]') and item.get('status') == 200]
        if len(route_records) != 1 or route_records[0].get('sha256') != response_sha or not route_requests:
            failures.append('loopback inventory does not contain the locked source route/body')
        request_json = json.loads(record.get('fieldObservations', {}).get('requestJson', 'null'))
        request_tracks = exported.get('requestTracks')
        if not isinstance(request_tracks, list) or request_json.get('tracks') != request_tracks:
            failures.append('source mapping does not match native preflight tracks')
        physical_files = [Path(track['relativePath']).stem for track in case.get('tracks', [])]
        if not isinstance(request_tracks, list) or [track.get('filename') for track in request_tracks] != physical_files:
            failures.append('source mapping files do not match the frozen physical file order')
        mapping = exported.get('mapping') or {}
        indices = mapping.get('remoteIndices')
        evidence = mapping.get('evidence')
        provider_tracks = mapping.get('providerTracks')
        if (not isinstance(indices, list) or not isinstance(evidence, list)
                or not isinstance(provider_tracks, list) or len(indices) != len(request_tracks or [])
                or len(evidence) != len(indices) or len(set(indices)) != len(indices)
                or any(type(index) is not int or index < 0 or index >= len(source_tracks)
                       or index >= len(provider_tracks) for index in indices)
                or any(not isinstance(item, str) or item.casefold() == 'position' for item in evidence)):
            failures.append('source mapping lacks complete unique title evidence')
        else:
            expected_mapping = {item['localTrack']: item for item in truth.get('mapping', [])}
            for local_index, remote_index in enumerate(indices, 1):
                expected = expected_mapping.get(local_index)
                source_title = source_tracks[remote_index].get('title')
                provider_title = provider_tracks[remote_index].get('title')
                local_title = request_tracks[local_index - 1].get('title')
                if (not expected or source_title != expected.get('providerTitle')
                        or provider_title != source_title or local_title != expected.get('localTitle')):
                    failures.append(f'source mapping disagrees with locked physical/source expectation at track {local_index}')
        if assisted.get('value') != source_fixture.get('genre'):
            failures.append('assisted proposal differs from the predeclared synthetic expectation')
    except (KeyError, TypeError, ValueError, IndexError) as error:
        failures.append(f'invalid source mapping evidence: {error}')
    return failures


def field_obligation_observation(record, case, require_assisted_write=False):
    candidate = ((record.get('native') or {}).get('candidate') or {})
    assisted = candidate.get('assisted_genre') or {}
    source = assisted.get('source_observation')
    if not source:
        if require_assisted_write:
            return ['required candidate genre fill lacks source-bound proposal evidence'], 'unavailable'
        return [], 'source_free_assistance' if assisted else 'not_applicable'
    exported = record.get('fieldReviewObligationsAfterRestart')
    if not isinstance(exported, dict):
        return ['source-bound write lacks reopened per-file obligation export'], 'unavailable'
    binding = record.get('genreSourceBindingEvidence')
    request_input = assisted.get('request_input')
    canonical = lambda value: json.dumps(value, ensure_ascii=False, sort_keys=True,
                                          separators=(',', ':')).encode()
    digest = lambda value: hashlib.sha256(value).hexdigest()
    source_fields = ['provider', 'resourceId', 'responseSha256', 'selector',
                     'textSha256', 'trackMappingSha256']
    proposal_fields = ['provider', 'resourceId', 'responseSha256', 'selector',
                       'textSha256', 'trackMappingSha256', 'evidenceId']
    source_identity = {key: source.get(key) for key in source_fields}
    source_id = digest(canonical(source_identity))
    if (source.get('origin') != 'synthetic_loopback' or source.get('provider') != 'discogs'
            or source.get('selector') != '/notes'
            or not re.fullmatch(r'[1-9][0-9]*', str(source.get('resourceId', '')))
            or not re.fullmatch(r'[a-f0-9]{64}', str(source.get('responseSha256', '')))
            or not re.fullmatch(r'[a-f0-9]{64}', str(source.get('trackMappingSha256', '')))
            or not isinstance(source.get('text'), str)
            or digest(source.get('text', '').encode()) != source.get('textSha256')
            or source.get('evidenceId') != source_id
            or not isinstance(binding, dict)
            or binding.get('releaseId') != source.get('resourceId')
            or binding.get('mappingSha256') != source.get('trackMappingSha256')
            or not isinstance(request_input, dict)
            or request_input.get('sourceObservation') != source
            or digest(canonical(request_input)) != assisted.get('input_sha256')):
        return ['source-bound obligation lacks matching release, evidence, or request provenance'], 'unavailable'
    proposal_source = {key: source[key] for key in proposal_fields}
    expected_proposal = digest(canonical({
        'rawValue': assisted.get('raw_value', ''),
        'appliedValue': assisted.get('value'),
        'requestSha256': assisted.get('input_sha256'),
        'source': proposal_source,
    }))
    if (not isinstance(assisted.get('value'), str) or not assisted.get('value').strip()
            or assisted.get('proposal_sha256') != expected_proposal):
        return ['assisted proposal digest does not bind its source and request'], 'unavailable'
    before = record.get('beforeTracks')
    after = record.get('readbackTracks')
    if not isinstance(before, list) or not isinstance(after, list) or len(before) != len(case.get('tracks', [])) \
            or len(after) != len(case.get('tracks', [])):
        return ['source-bound obligation cannot be joined to complete readback tracks'], 'unavailable'
    failures = []
    eligible_count = 0
    written_count = 0
    expected_revision = f"{record.get('phase')}-{record.get('caseId')}"
    expected_paths = []
    for track in before:
        path = track.get('path') if isinstance(track, dict) else None
        if isinstance(path, str):
            expected_paths.append(path)
    if len(expected_paths) != len(case['tracks']) or len(set(expected_paths)) != len(expected_paths) \
            or set(exported) != set(expected_paths):
        failures.append('reopened obligation paths do not exactly match physical readback files')
    for index, case_track in enumerate(case['tracks']):
        relative = case_track['relativePath']
        before_track, after_track = before[index], after[index]
        before_path, after_path = before_track.get('path'), after_track.get('path')
        relative_parts = Path(relative).parts
        before_parts = Path(before_path).parts if isinstance(before_path, str) else ()
        if (not before_parts or Path(before_path).is_absolute() is False
                or before_path != after_path
                or len(before_parts) < len(relative_parts)
                or before_parts[-len(relative_parts):] != relative_parts):
            failures.append(f'{relative}: field-obligation file order mismatch')
            continue
        obligations_for_path = exported.get(before_path)
        if not isinstance(obligations_for_path, list):
            failures.append(f'{relative}: missing or duplicate reopened per-file obligation row')
            continue
        was_empty = not isinstance(before_track.get('genre'), str) or not before_track['genre'].strip()
        was_written = was_empty and after_track.get('genre') == assisted.get('value')
        if not was_empty and after_track.get('genre') != before_track.get('genre'):
            failures.append(f'{relative}: assisted inference changed a pre-existing genre')
        eligible_count += int(was_empty)
        written_count += int(was_written)
        obligations = [item for item in obligations_for_path
                       if item.get('field') == 'genre' and item.get('revision') == expected_revision]
        if len(obligations) != int(was_written):
            failures.append(f'{relative}: reopened genre obligation does not match the assisted readback')
        elif was_written:
            obligation = obligations[0]
            provenance = obligation.get('provenance')
            expected_grounded = source.get('origin') == 'discogs'
            if (obligation.get('path') != before_path or obligation.get('before') != before_track.get('genre')
                    or obligation.get('after') != after_track.get('genre')
                    or obligation.get('state') != 'open'
                    or not isinstance(provenance, dict)
                    or provenance.get('kind') != 'assisted_inference'
                    or provenance.get('grounded') is not expected_grounded
                    or provenance.get('sourceMatch') != 'exact_phrase_in_source_text'
                    or provenance.get('sourceObservation') != source
                    or provenance.get('proposalSha256') != assisted.get('proposal_sha256')
                    or provenance.get('model') != assisted.get('model')
                    or provenance.get('rawValue') != assisted.get('raw_value', '')
                    or provenance.get('appliedValue') != assisted.get('value')
                    or provenance.get('requestInput') != request_input
                    or provenance.get('requestInputSha256') != assisted.get('input_sha256')
                    or obligation.get('evidence_ids') != [source.get('evidenceId')]):
                failures.append(f'{relative}: reopened obligation does not bind the exact before, source, proposal, and evidence')
        if require_assisted_write and was_empty and not was_written:
            failures.append(f'{relative}: required assisted genre is absent from independent readback')
    if require_assisted_write and not eligible_count:
        failures.append('required assisted-fill scenario had no eligible blank genre fields')
    if require_assisted_write and not written_count:
        failures.append('required assisted-fill scenario produced no independent genre write')
    return failures, 'open_per_file_obligations' if written_count and not failures else 'unavailable'


def summarize(before, after):
    assert [(r['caseId'], r['profile'], r['phase'], r.get('scenario')) for r in before] == [
        (r['caseId'], r['profile'], r['phase'], r.get('scenario')) for r in after], 'different requested membership'
    def counts(rows):
        return dict(correctRecoveries=sum(r['recovery'] == 'correct' and not r['unavailable'] for r in rows),
                    wrongMatches=sum(r['recovery'] == 'wrong_match' for r in rows),
                    abstentions=sum(r['recovery'] == 'abstention' for r in rows),
                    unavailable=sum(r['unavailable'] for r in rows),
                    missingFixtures=sum(bool(r['missingFixtures']) for r in rows),
                    processFailures=sum(r['processFailure'] for r in rows),
                    skipped=sum(r['skipped'] for r in rows),
                    readbackFailures=sum(bool(r['readbackFailures']) for r in rows),
                    safetyFailures=sum(bool(r['safetyFailures']) for r in rows),
                    requiredNoWriteControls=sum(bool(r.get('requiredNoWrite')) for r in rows),
                    passedNoWriteControls=sum(bool(r.get('requiredNoWrite')) and r['recovery'] == 'abstention'
                        and not r['unavailable'] and not r['safetyFailures'] and not r['readbackFailures'] for r in rows),
                    sourceBoundAssistanceWithOpenObligation=sum(r.get('reviewPersistence') == 'open_per_file_obligations'
                        and not r['unavailable'] for r in rows),
                    sourceFreeAssistanceObservations=sum(r.get('reviewPersistence') == 'source_free_assistance' for r in rows),
                    cachedSourceWithheld=sum(r.get('cachedSourceWithheld', False)
                        and not r['unavailable'] and not r['safetyFailures'] for r in rows),
                    assistanceFailureControls=sum(r.get('scenario') == 'assistance_failure' for r in rows),
                    passedAssistanceFailureControls=sum(bool(r.get('scenario') == 'assistance_failure'
                        and r.get('assistanceFailureObserved') and r.get('written', 0) > 0
                        and not r['unavailable'] and not r['safetyFailures'] and not r['readbackFailures'])
                        for r in rows),
                    providerPrecedenceControls=sum(r.get('providerPrecedenceControl', False) for r in rows),
                    passedProviderPrecedenceControls=sum(bool(r.get('providerPrecedenceControl')
                        and not r.get('providerPrecedenceFailures') and not r['unavailable']
                        and not r['safetyFailures']) for r in rows),
                    providerPrecedenceFailures=sum(bool(r.get('providerPrecedenceFailures')) for r in rows),
                    providerPrecedenceGenreFailures=sum(bool(r.get('providerPrecedenceGenreFailures')) for r in rows),
                    providerPrecedenceSafetyFailures=sum(bool(r.get('providerPrecedenceSafetyFailures')) for r in rows),
                    providerPrecedenceUnavailable=sum(bool(r.get('providerPrecedenceUnavailable')) for r in rows),
                    reviewedRepresentatives=sum(r.get('edition') == 'reviewed_representative' for r in rows),
                    unresolvedEditions=sum(r.get('edition') == 'unresolved' for r in rows),
                    reviewedLayouts=sum(bool(r.get('reviewedLayoutMatches')) for r in rows))
    complete = not any(r['unavailable'] for r in before + after)
    improvements = sum(a['recovery'] != 'correct' and b['recovery'] == 'correct'
                       and not a['unavailable'] and not b['unavailable'] for a, b in zip(before, after))
    regressions = sum((a['recovery'] == 'correct' and b['recovery'] != 'correct')
                      or (a['recovery'] != 'wrong_match' and b['recovery'] == 'wrong_match')
                      for a, b in zip(before, after))
    left, right = counts(before), counts(after)
    return dict(requestedEvaluationsPerImplementation=len(before), baseline=left, candidate=right,
                comparable=complete, improvements=improvements, regressions=regressions,
                clean=complete and regressions == 0 and not any(right[k] for k in
                      ['wrongMatches', 'safetyFailures', 'readbackFailures']),
                groundedModelQualityCredit=0)


def album_title_matches(actual, expected):
    documented = [expected['album'], *expected.get('albumAliases', [])]
    return any(titles.key(actual or '') == titles.key(title) for title in documented)


def performer_credits_match(display, performers, credits, expected):
    display = titles.key(display or '')
    actual = [titles.key(name) for name in performers]
    reviewed = [titles.key(name) for name in credits]
    if actual == reviewed and all(name in display for name in reviewed):
        return True
    spellings = {titles.key(name) for name in expected.get('principalArtistSpellings', [])}
    return (len(actual) == len(reviewed) == 1
            and {display, actual[0], reviewed[0]} <= spellings)


def track_title_outcome(actual, mapping, artist, case_id, expected, recording_id=None):
    result = titles.title_outcome(actual, mapping, artist, case_id)
    alias = expected.get('trackTitleAlias')
    if (alias and not result['prefix'] and case_id == alias['caseId']
            and recording_id == alias['recordingId']
            and mapping.get('localTrack') == alias['localTrack']
            and str(mapping.get('providerTrack')) == alias['providerTrack']
            and titles.key(mapping['providerTitle']) == titles.key(alias['reviewedTitle'])
            and titles.key(actual or '') == titles.key(alias['title'])):
        return {'status': 'correct', 'prefix': False}
    return result


def reviewed_alias_fields(review, truth, fixtures):
    case_id = review['caseId']
    spellings = {
        '204915b5a4ec': ('盧廣仲', '卢广仲'),
        '5da2dcff8d52': ('盧廣仲', '卢广仲'),
        '7634f48421db': ('盧廣仲', '卢广仲'),
        '90d6af52dacf': ('盧廣仲', '卢广仲'),
        '5fdcfd7d2149': ('草東沒有派對', '草东没有派对'),
    }.get(case_id)
    if spellings and spellings[0] in review.get('artistAliases', []):
        return {'principalArtistSpellings': list(spellings)}

    identity = (case_id, review['provider'], str(review['releaseId']), review['snapshot'])
    details = fixtures / 'provider-snapshots/discovery/details-2026-11-07'
    if identity == ('c0e96540c785', 'discogs', '2994652', 'provider-snapshots/enya/discogs-release-2994652.json'):
        alternate = details / 'musicbrainz-135228c9a6b562a8.json'
        if (sha(fixtures / review['snapshot']) != '9617f5efd35ce0e00329a19a7b0ae0d223e16f592e1e62a4779a9e673eb1f142'
                or sha(alternate) != '29b4374bc65a912a437722eb6c8df8e8e740dbe8b94b084b062e94da108eee16'):
            raise ValueError('And Winter Came oracle requires the two linked release snapshots')
        return {'albumAliases': [load(alternate)['title']]}

    if identity == ('872fee627bfd', 'musicbrainz', '8d9aa53f-8d47-45ab-829a-8352078a39d4',
                    'provider-snapshots/discovery/details-2026-11-07/musicbrainz-730231224291bcac.json'):
        alternate = details / 'musicbrainz-2c085592cd2c0cc6.json'
        reviewed = fixtures / review['snapshot']
        if (sha(reviewed) != '7c9e50cc1c082e7916ee882dc49cefe3605652b8ed2dfc553d1b3ff914a0fbf5'
                or sha(alternate) != 'b39c1c02c80d570e9287eb9c40f2fd1aeb56e402d53b50d0cb105ffc7b07e650'):
            raise ValueError('Shepherd Moons oracle requires the two recording snapshots')
        original, variant = (load(path)['media'][0]['tracks'][6] for path in (reviewed, alternate))
        recording = original['recording']['id']
        if recording != variant['recording']['id']:
            raise ValueError('Shepherd Moons track aliases require the same recording')
        if any(track['localTrack'] == 7 and str(track['providerTrack']) == '7'
               and titles.key(track['providerTitle']) == titles.key(original['title']) for track in truth['mapping']):
            return {'trackTitleAlias': dict(caseId=case_id, localTrack=7, providerTrack='7',
                                            reviewedTitle=original['title'], title=variant['title'], recordingId=recording)}
    return {}


def album_credit_matches(display, performers, expected):
    return (any(titles.key(display or '') == titles.key(name) for name in expected['albumArtistDisplays'])
            and [titles.key(name) for name in (performers or [])] == [
                titles.key(name) for name in expected['albumArtists']])


def expected_for_profile(expected, profile):
    if 'profileFields' not in expected:
        return expected
    return expected | expected['profileFields'][profile]


def relapse_reviewed_fields(snapshot, source, truth, fixtures):
    discogs = fixtures.parent / 'relapse-deluxe/release-36441795.json'
    musicbrainz = fixtures / 'provider-snapshots/discovery/details-2026-11-07/musicbrainz-22be98f32a7f4174.json'
    if (sha(snapshot) != 'c8ad11806c623231c430787599d051df2cd545d3f6a4ca7a067f4e8183e3e7c4'
            or sha(discogs) != '791a57e61770548bc7964c53fb7aa601702bf90528ea3bb2f7362c22a6f17c8f'
            or sha(musicbrainz) != 'cfa111b6bf92a5689262e13929fa8451443668b273306deaaaf017f3bdad6a62'):
        raise ValueError('Relapse oracle requires the three reviewed release snapshots')
    reviewed_tracks = {str(track['track_number']): track for track in source['tracks']}
    positions = [str(mapping['providerTrack']) for mapping in truth['mapping']]
    if len(positions) != len(reviewed_tracks) or set(positions) != set(reviewed_tracks):
        raise ValueError('Relapse oracle requires one mapping for each reviewed track')
    for index, mapping in enumerate(truth['mapping'], 1):
        track = reviewed_tracks[positions[index - 1]]
        if mapping['localTrack'] != index or titles.key(mapping['providerTitle']) != titles.key(track['title']):
            raise ValueError('Relapse oracle requires the reviewed track mapping')
    mb = load(musicbrainz)
    discogs = load(discogs)
    credits = [reviewed_tracks[position]['artists'] for position in positions]
    tracks = mb['media'][0]['tracks']
    credits[positions.index('13')] = [artist['name'] for artist in tracks[12]['artist-credit']]
    filename_credits = credits.copy()
    filename_credits[positions.index('18')] = [artist['name'] for artist in tracks[17]['artist-credit']]
    # MusicBrainz supplies the missing track 13 guest. For track 18, the
    # filename profile follows MusicBrainz and the tagged profiles retain
    # the documented Discogs order. Captured edition labels must survive.
    tagged = dict(album=discogs['title'], credits=credits)
    return tagged | {'profileFields': {
        'folder_filename': dict(album=mb['title'], credits=filename_credits),
        'assisted_without_ids': tagged,
        'tagged_recovery': tagged,
    }}


def reviewed_fields(review, truth, fixtures):
    snapshot = fixtures / review['snapshot']
    source = load(snapshot)
    if (review['caseId'], review['provider'], str(review['releaseId']), review['snapshot']) == (
            '8452b6f2b3ea', 'discogs', '36441795', 'provider-snapshots/relapse/discogs-release-36441795.json'):
        return relapse_reviewed_fields(snapshot, source, truth, fixtures)
    if (review['caseId'], review['provider'], str(review['releaseId']), review['snapshot']) == (
            'e8a8a8d6b27d', 'discogs', '544082', 'provider-snapshots/enya/discogs-release-544082.json'):
        alternate = fixtures / 'provider-snapshots/discovery/details-2026-11-07/musicbrainz-5eb14a0f3df79e2a.json'
        if (sha(snapshot) != '60801c2b6a948cb189370283633d3a5a0ec05ab7e164437637a4547a1e58aa40'
                or sha(alternate) != 'f2a315cc7d1a8dcbb02f505aedadf6512208e7c61c390e8902231ce638d95f3c'):
            raise ValueError('Exile oracle requires the two reviewed release snapshots')
        # Discogs presents artist 9807 twice, once with the Japanese alias.
        # The directly linked MusicBrainz entry supplies the short album title.
        return dict(album=source['title'], albumAliases=[load(alternate)['title']],
                    credits=[['Enya'] for _ in truth['mapping']])
    remote = []
    if review['provider'] == 'musicbrainz':
        for medium in source['media']:
            for track in medium.get('tracks', []):
                credits = track.get('artist-credit') or track.get('recording', {}).get('artist-credit') or source['artist-credit']
                names = [c.get('name') or c['artist']['name'] for c in credits]
                remote.append((track.get('title') or track['recording']['title'], names))
        album = source['title']
    else:
        clean = lambda name: re.sub(r' \(\d+\)$', '', name)
        for track in source['tracklist']:
            if track.get('type_', 'track') != 'track':
                continue
            credits = track.get('artists') or source['artists']
            names = [clean(c['name']) for c in credits]
            for guest in track.get('extraartists', []):
                if guest['role'].casefold() in ['featuring', 'guest [with]'] and clean(guest['name']) not in names:
                    names.append(clean(guest['name']))
            remote.append((track['title'], names))
        album = source['title']
    credits = []
    for mapping in truth['mapping']:
        options = {tuple(names) for title, names in remote if titles.key(title) == titles.key(mapping['providerTitle'])}
        if len(options) != 1:
            raise ValueError(f'{review["caseId"]}: ambiguous reviewed credits; not eligible for this checker')
        credits.append(list(options.pop()))
    expected = dict(album=album, credits=credits, **reviewed_alias_fields(review, truth, fixtures))
    if (review['caseId'] in {'3b95f81ac351', '4060f1de6662'}
            and review['provider'] == 'musicbrainz'
            and str(review['releaseId']) == '2ca2b5dd-1695-40e2-8947-9baf4b4c12b0'
            and review['snapshot'] == 'provider-snapshots/review-expansion-20261108/musicbrainz-372d24bff6c7bfdf.json'):
        alternate = fixtures / 'provider-snapshots/nine-case-discovery-20261108/discogs-59f094dc7673f1e7.json'
        if (sha(snapshot) != 'ae7334407e687f39e9f0b610eeafe2efb6e751a9087eb09e25f4eebbeaadb4f5'
                or sha(alternate) != '6fa33bb2d8dfbb48ac1d2ca4dfd33119ec171a5278af17093c6ef815049990b8'):
            raise ValueError('Lo Vas A Olvidar album credit requires both reviewed releases')
        release_credits = source['artist-credit']
        alternate_credits = load(alternate)['artists']
        expected['albumArtists'] = [item['name'] for item in release_credits]
        expected['albumArtistDisplays'] = [
            ''.join(item['name'] + item.get('joinphrase', '') for item in release_credits),
            ' & '.join(re.sub(r' \(\d+\)$', '', item['name']) for item in alternate_credits),
        ]
    return expected


def audit(root):
    snapshot = load(root / 'comparison-inputs.json')
    rows = {}
    failures = list(snapshot['setupFailures'])
    for file, digest in snapshot.get('locks', {}).items():
        if not Path(file).is_file() or sha(file) != digest:
            failures.append(f'changed/missing input: {file}')
    for arm in ['baseline', 'candidate']:
        result = []
        for job in snapshot['jobs']:
            directory = root / arm / job['profile'] / job['caseId']
            if job.get('scenario'):
                directory = directory / 'supplemental' / job['scenario']
            evidence_failures = []
            def export(name):
                try:
                    value = load(directory / name)
                    if not isinstance(value, dict) or not value:
                        raise ValueError('expected nonempty object')
                    if name == 'inventory.json':
                        if (not isinstance(value.get('misses'), list) or not isinstance(value.get('modelRequests'), list)
                                or any(not isinstance(m, dict) or type(m.get('count', 1)) is not int or m.get('count', 1) < 0
                                       for m in value['misses'])):
                            raise ValueError('invalid miss counts or model request array')
                    elif name == 'execution.json':
                        if ('exitCode' not in value or (value['exitCode'] is not None and type(value['exitCode']) is not int)
                                or 'signal' not in value or (value['signal'] is not None and not isinstance(value['signal'], str))
                                or ('skipped' in value and type(value['skipped']) is not bool)):
                            raise ValueError('invalid exit code, signal or skipped status')
                    else:
                        files = value.get(job['caseId'])
                        if (set(value) != {job['caseId']} or not isinstance(files, dict)
                                or set(files) != {t['relativePath'] for t in job['case']['tracks']}
                                or any(not isinstance(h, dict) or type(h.get('bytes')) is not int or h['bytes'] < 0
                                       or not isinstance(h.get('fullSha256'), str)
                                       or not re.fullmatch(r'[a-f0-9]{64}', h['fullSha256']) for h in files.values())):
                            raise ValueError('invalid source file membership, size or SHA-256')
                    return value
                except (OSError, ValueError) as error:
                    evidence_failures.append(f'{name}: unavailable export ({error})')
                    return {}
            execution = export('execution.json')
            expected_run_id = None
            runner_context_failures = []
            if job.get('scenario'):
                scenario = job['scenario']
                expected_run_id = f"{arm}-{job['profile']}-{job['caseId']}-{scenario}"
                expected_directory = Path(arm) / job['profile'] / job['caseId'] / 'supplemental' / scenario
                if job.get('supplemental') is not True:
                    runner_context_failures.append('supplemental scenario is not declared as a runner-owned job')
                fixture = snapshot.get('syntheticGenreSource')
                if (scenario != 'assistance_failure' or not isinstance(fixture, dict)
                        or job['caseId'] != fixture.get('caseId')
                        or job['profile'] != fixture.get('profile')):
                    runner_context_failures.append('supplemental scenario does not match the locked source job')
                if directory.relative_to(root).as_posix() != expected_directory.as_posix():
                    runner_context_failures.append('supplemental artifact directory differs from the runner-owned job')
                if (execution.get('runId') != expected_run_id
                        or execution.get('artifactDirectory') != expected_directory.as_posix()
                        or execution.get('scenario') != scenario):
                    runner_context_failures.append('execution context does not bind the supplemental run and directory')
            inventory = export('inventory.json')
            source_before = export('source-hashes-before.json')
            source_after = export('source-hashes-after.json')
            phases = {}
            for phase in ['cold', 'warm']:
                file = directory / f'{phase}.jsonl'
                try:
                    phases[phase] = [json.loads(line) for line in file.read_text().splitlines() if line.strip()] if file.exists() else []
                except (ValueError, OSError):
                    phases[phase] = []
                context = dict(caseId=job['caseId'], profile=job['profile'], phase=phase)
                if job.get('scenario'):
                    context['scenario'] = job['scenario']
                try:
                    record = phases[phase][0] if len(phases[phase]) == 1 else {}
                    allow_legacy_scenario = bool(
                        arm == 'baseline' and job.get('supplemental') is True
                        and job.get('scenario') and not runner_context_failures
                        and 'scenario' not in record)
                    source_fixture = snapshot.get('syntheticGenreSource') or {}
                    assisted_candidate = (arm == 'candidate'
                        and job.get('caseId') == source_fixture.get('caseId')
                        and job.get('profile') == source_fixture.get('profile'))
                    withhold_cached_source = (assisted_candidate and phase == 'warm'
                        and source_fixture.get('candidateWarmAssistance') == 'withhold_cached_source')
                    require_assisted_write = (assisted_candidate and not job.get('scenario')
                        and not withhold_cached_source)
                    row = audit_phase(context, phases[phase], job['case'], job['truth'], job['review'], job['expected'],
                                      require_no_write=job.get('requireNoWrite', False),
                                      source_fixture=snapshot.get('syntheticGenreSource'), inventory=inventory,
                                      require_assisted_write=require_assisted_write,
                                      require_no_assisted_obligation=arm == 'candidate',
                                      expected_run_id=expected_run_id,
                                      allow_legacy_scenario=allow_legacy_scenario,
                                      withhold_cached_source=withhold_cached_source)
                except (KeyError, TypeError, ValueError, IndexError) as error:
                    row = audit_phase(context, [], {}, {}, {}, {}, job.get('requireNoWrite', False)) | dict(
                        reason=f'invalid phase export: {error}')
                row['processFailure'] = (bool(execution) and (execution.get('exitCode') != 0 or execution.get('signal') is not None)
                                         or any(f.startswith('execution.json:') for f in evidence_failures))
                row['skipped'] = not execution or execution.get('skipped', False)
                model_failures = scripted_model_failures(
                    phases[phase][0], inventory, job, arm, snapshot.get('syntheticGenreSource')) \
                    if len(phases[phase]) == 1 else ['missing or duplicate model-observation phase']
                row['safetyFailures'].extend(model_failures)
                row['missingFixtures'] = sum(m.get('count', 1) for m in inventory.get('misses', []))
                row['safetyFailures'].extend(evidence_failures)
                row['safetyFailures'].extend(runner_context_failures)
                if evidence_failures or model_failures or row['processFailure'] or row['skipped'] or row['missingFixtures'] or not inventory:
                    row['unavailable'] = True
                    if row['recovery'] != 'wrong_match':
                        row['recovery'] = 'unavailable'
                if runner_context_failures:
                    row['unavailable'] = True
                    if row['recovery'] != 'wrong_match':
                        row['recovery'] = 'unavailable'
                result.append(row)
            if all(len(phases[p]) == 1 for p in phases):
                cold, warm = [phases[p][0] for p in ['cold', 'warm']]
                if (any(cold.get('cacheBefore', {}).values()) or warm.get('cacheBefore') != cold.get('cacheAfter')
                        or cold.get('profileBaseline') != warm.get('profileBaseline')):
                    for row in result[-2:]:
                        row['safetyFailures'].append('cold/warm cache or initial media isolation')
                # An accepted fresh source can fill genre and open review on cold only.
                # The warm-phase oracle separately verifies unchanged genre and zero obligations.
                expected_genre_delta = (not job.get('scenario') and result[-1].get('cachedSourceWithheld')
                    and result[-2].get('reviewPersistence') == 'open_per_file_obligations'
                    and cold.get('outcome') == 'needs_review' and warm.get('outcome') == 'applied')
                def portable(record):
                    tracks = record.get('readbackTracks')
                    if not isinstance(tracks, list) or not all(isinstance(t, dict) for t in tracks):
                        return None
                    return [{k: v for k, v in t.items() if k not in ['path', 'sizeBytes', 'bitrate']
                             and not (expected_genre_delta and k == 'genre')} for t in tracks]
                if ((not expected_genre_delta and cold.get('outcome') != warm.get('outcome'))
                        or portable(cold) != portable(warm)):
                    for row in result[-2:]:
                        row['safetyFailures'].append('cold/warm output drift')
            if execution and (not source_before or not source_after or source_before != source_after):
                for row in result[-2:]:
                    row['safetyFailures'].append('source-media verification missing or changed')
        rows[arm] = result
    report = summarize(rows['baseline'], rows['candidate'])
    report['primaryEvaluationsPerImplementation'] = sum(not job.get('supplemental') for job in snapshot['jobs']) * 2
    report['supplementalAssistanceFailureEvaluationsPerImplementation'] = sum(
        job.get('scenario') == 'assistance_failure' for job in snapshot['jobs']) * 2
    report.update(setupFailures=failures, implementations=snapshot['implementations'],
                  budget=snapshot['budget'], inputLockCount=len(snapshot.get('locks', {})), rows=rows,
                  scope=('Reviewed metadata content, not acoustic or exact pressing proof; scripted synthetic source/model only'
                         if snapshot['budget'].get('assistance') == 'scripted'
                         else 'Reviewed metadata content, not acoustic or exact pressing proof; models disabled'))
    report['reviewPersistence'] = {
        'baselineSourceReviewExport': bool(snapshot.get('implementations', {}).get('baseline', {}).get('pairedReviewExport')),
        'candidateSourceReviewExport': bool(snapshot.get('implementations', {}).get('candidate', {}).get('pairedReviewExport')),
        'groundedModelQualityCredit': 0,
        'interpretation': 'synthetic source-bound workflow evidence; no real-world source truth or model quality claim',
    }
    if failures:
        report.update(clean=False, comparable=False, improvements=0)
    # No credit for an initial-input mismatch between implementations.
    for job in snapshot['jobs']:
        for phase in ['cold', 'warm']:
            files = [root / arm / job['profile'] / job['caseId'] / f'{phase}.jsonl' for arm in rows]
            files = [root / implementation / job['profile'] / job['caseId'] / 'supplemental' / job['scenario'] / f'{phase}.jsonl'
                     if job.get('scenario') else root / implementation / job['profile'] / job['caseId'] / f'{phase}.jsonl'
                     for implementation in rows]
            if all(f.exists() for f in files):
                try:
                    records = [json.loads(f.read_text()) for f in files]
                    if records[0]['profileBaseline'] != records[1]['profileBaseline']:
                        raise ValueError('different profile input bytes')
                except (ValueError, KeyError):
                    failures.append(f'{job["caseId"]}/{job["profile"]}/{phase}: input mismatch or duplicate record')
                    report.update(clean=False, comparable=False, improvements=0)
    (root / 'comparison.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    primary_cases = sum(not job.get('supplemental') for job in snapshot['jobs']) // 3
    lines = ['# Offline paired comparison', '', f'Comparable: {report["comparable"]}; clean candidate gate: {report["clean"]}',
             f'Requested: {primary_cases} cases × 3 profiles × 2 phases = {report["primaryEvaluationsPerImplementation"]} primary slots; '
             f'{report["supplementalAssistanceFailureEvaluationsPerImplementation"]} separately declared supplemental slots; '
             f'{report["requestedEvaluationsPerImplementation"]} total per implementation.',
             f'Improvements: {report["improvements"]}; regressions: {report["regressions"]}.', '',
             '| Outcome / check | Baseline | Candidate |', '|---|---:|---:|']
    lines += [f'| {key} | {report["baseline"][key]} | {report["candidate"][key]} |' for key in report['baseline']]
    lines += ['', 'Missing-fixture/process counts above are affected evaluation slots, not distinct routes/processes.', '',
              '## Implementations']
    lines += [f'- {arm}: `{value.get("gitHead", "unavailable")}`, binary `{value.get("binarySha256", "unavailable")}`; dirty source: {value.get("dirtySource", "unknown")}'
              for arm, value in snapshot['implementations'].items()]
    lines += ['', '## Per-profile recoveries', '', '| Profile | Baseline | Candidate |', '|---|---:|---:|']
    for profile in snapshot['profiles']:
        counts = [sum(r['recovery'] == 'correct' and not r['unavailable'] for r in rows[arm] if r['profile'] == profile) for arm in rows]
        lines.append(f'| {profile} | {counts[0]} | {counts[1]} |')
    lines += ['', '## Incomplete setup', *[f'- {failure}' for failure in failures], '', report['scope'],
              'Readback and write-safety failures are separate from content recovery. Abstentions remain in the requested denominator.']
    (root / 'comparison.md').write_text('\n'.join(lines) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k not in ['rows', 'implementations']}, indent=2))
    return 0 if report['clean'] else 1


if __name__ == '__main__':
    if len(sys.argv) == 4 and sys.argv[1] == 'fields':
        job = load(sys.argv[2])
        print(json.dumps(reviewed_fields(job['review'], job['truth'], Path(sys.argv[3]))))
    else:
        sys.exit(audit(Path(sys.argv[1])))
