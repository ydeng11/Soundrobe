"""Non-authorizing offline observations; never an input to tagging."""
import hashlib
import json
import math
import pathlib
import re
import sys
import unicodedata


def digest(data):
    return hashlib.sha256(data).hexdigest()


def query_pairs(value):
    pairs = (value or {}).items() if isinstance(value, dict) else value or []
    return tuple(sorted(tuple(pair) for pair in pairs))


def normal(value):
    value = unicodedata.normalize('NFKD', value or '').casefold()
    return ''.join(c for c in value if c.isalnum())


VARIANT = re.compile(r'\b(clean|explicit|instrumental|reprise|remix|mix|live|studio|edit|demo|acoustic|karaoke|mono|stereo|remaster(?:ed)?|version|skit)\b', re.I)


def title_key(value):
    # Only a correspondence search key. Removed text remains raw evidence and
    # is independently held by the qualifier check; this never proves a version.
    value = re.sub(r'[([]([^\])]+)[)\]]',
                   lambda m: '' if VARIANT.search(m[1]) else m[0], value or '')
    return normal(value)


def qualifiers(local, raw, context=None):
    def claims(values):
        return sorted({m[0].casefold() for value in values if value
                       for m in VARIANT.finditer(value)})
    left = claims([local.get('title'), local.get('filename')])
    recording = raw.get('recording') or {}
    right = claims([raw.get('title'), recording.get('title'), recording.get('disambiguation')])
    contrary = [('clean', 'explicit'), ('mono', 'stereo'), ('live', 'studio')]
    contradiction = any((a in left and b in right) or (b in left and a in right) for a, b in contrary)
    applicability = 'contradicted' if contradiction else 'matching_claims' if left == right and left else (
        'no_marked_variant_obstacle' if not left and not right else 'unresolved')
    # Non-lexical variant text is retained, not declared equivalent. Explicit
    # remaster dates or mix names need reviewed interpretation outside this unit.
    details = [v for v in [local.get('title'), local.get('filename'), raw.get('title'),
                           recording.get('title'), recording.get('disambiguation')] if v]
    def descriptions(values):
        return {normal(m[1]) for value in values if value
                for m in re.finditer(r'[([]([^\])]+)[)\]]', value) if VARIANT.search(m[1])}
    local_details = descriptions([local.get('title'), local.get('filename')])
    source_details = descriptions([raw.get('title'), recording.get('title')])
    if applicability != 'contradicted' and local_details and source_details and local_details != source_details:
        applicability = ('contradicted' if any(re.search(r'\b(remix|mix|remaster(?:ed)?)\b', v, re.I)
                                               for v in details) else 'unresolved')
    if applicability == 'matching_claims' and any(re.search(r'\b(remix|mix|version|remaster(?:ed)?)\b', v, re.I) for v in details):
        applicability = 'unresolved'
    context = context or {}
    lc = claims([context.get('localAlbum'), context.get('localFolder')])
    sc = claims([context.get('sourceAlbum'), context.get('sourceDisambiguation')])
    reasons = []
    if (lc or sc) and (lc != sc or set(lc) - set(left) or set(sc) - set(right)):
        reasons.append('release_context_variant_unresolved')
        if applicability != 'contradicted':
            applicability = 'unresolved'
    return dict(local=left, source=right, applicability=applicability,
                context=context, localContextClaims=lc, sourceContextClaims=sc, reasons=reasons,
                rawRecordingDisambiguation=recording.get('disambiguation'))


def duration_fact(local, remote):
    valid = lambda v: isinstance(v, (int, float)) and not isinstance(v, bool) and math.isfinite(v) and v > 0
    available = valid(local) and valid(remote)
    delta = abs(local-remote) if available else None
    bound = max(5, 0.03*local) if valid(local) else None
    return dict(localSeconds=local, sourceSeconds=remote, deltaSeconds=delta, boundSeconds=bound,
                withinBound=available and delta <= bound)


def occurrences(body, provider):
    if provider == 'musicbrainz':
        return [(f'/media/{m}/tracks/{i}', track) for m, medium in enumerate(body.get('media', []))
                for i, track in enumerate(medium.get('tracks', []))]
    return [(f'/tracklist/{i}', track) for i, track in enumerate(body.get('tracklist', []))]


def seconds(raw, provider):
    if provider == 'musicbrainz':
        value = raw.get('length')
        return value/1000 if isinstance(value, (int, float)) and not isinstance(value, bool) else None
    value = raw.get('duration')
    if not isinstance(value, str) or not re.fullmatch(r'\d+:\d{2}', value):
        return None
    minutes, second = map(int, value.split(':'))
    return minutes*60+second if second < 60 else None


def credits(raw, provider, baseline):
    track = raw.get('artist-credit' if provider == 'musicbrainz' else 'artists')
    recording = (raw.get('recording') or {}).get('artist-credit')
    def names(values):
        if not isinstance(values, list) or not values:
            return None
        result = [c.get('name') or (c.get('artist') or {}).get('name') for c in values]
        return result if all(result) else None
    def identities(values):
        return {((c.get('artist') or {}).get('id'), c.get('name') or (c.get('artist') or {}).get('name'))
                for c in values or []}
    def featuring(values):
        edges = set()
        for i, c in enumerate((values or [])[:-1]):
            if re.search(r'\b(feat\.?|featuring|ft\.?)\b', c.get('joinphrase', ''), re.I):
                before, after = values[:i+1], values[i+1:]
                edges |= {(a, b) for a in identities(before) for b in identities(after)}
        return edges
    tn, rn = names(track), names(recording)
    guests = [c['name'] for c in raw.get('extraartists', []) if c.get('name') and
              any(role.strip().casefold() == 'featuring' for role in c.get('role', '').split(','))]
    reason = None
    if not tn:
        agreement, reason = 'insufficient', 'missing_track_principals'
    elif provider == 'musicbrainz' and not rn:
        agreement, reason = 'insufficient', 'missing_recording_credits'
    elif provider == 'musicbrainz' and (set(tn) != set(rn) or identities(track) != identities(recording)):
        agreement, reason = 'conflict', 'track_recording_identity_conflict'
    elif provider == 'musicbrainz' and featuring(track) != featuring(recording):
        if featuring(track) and featuring(recording):
            agreement, reason = 'conflict', 'track_recording_relationship_conflict'
        else:
            agreement, reason = 'relationship_unresolved', 'track_recording_relationship_unresolved'
    else:
        agreement = 'direct_agreement'
    baseline_names = baseline.get('artists') or ([baseline['artist']] if baseline.get('artist') else [])
    donor = tn + guests if tn else None
    a, b = {normal(n) for n in baseline_names}, {normal(n) for n in donor or []}
    non_nested = bool(a and b and not (a <= b or b <= a))
    return dict(trackNames=tn, recordingNames=rn, directGuests=guests, completeNames=donor, baselineNames=baseline_names,
                agreement=agreement, reason=reason, nonNested=non_nested,
                rawTrackCredits=track, rawRecordingCredits=recording,
                comparison='observed_names_and_same-provider_ids_only; no alias or identity inference')


def validate_guards(guard, receipt, body, tracks):
    if query_pairs(guard.get('query')) != query_pairs(receipt.get('query')):
        raise ValueError('guard query does not bind receipt')
    raw_by_selector = dict(occurrences(body, receipt['provider']))
    seen = set()
    for pair in guard['pairs']:
        key = (pair['localIndex'], pair['sourceSelector'])
        if key in seen or not 0 <= key[0] < len(tracks) or key[1] not in raw_by_selector:
            raise ValueError('guard occurrence binding invalid or duplicated')
        seen.add(key)
        raw, local = raw_by_selector[key[1]], tracks[key[0]]
        rec = raw.get('recording') or {}
        remote = dict(title=raw.get('title') or rec.get('title'), match_titles=[], release_disambiguation=body.get('disambiguation'),
                      artist=None, artists=[], track_number=None, track_total=None, disc_number=None, disc_total=None,
                      media_type=None, musicbrainz_trackid=None, length=None, genre=None, filename=None)
        if rec.get('title') is not None and rec.get('title') != raw.get('title'):
            remote['match_titles'] = [rec['title']]
        local_title = local.get('title') if local.get('title') is not None else local.get('filename') or ''
        expected = [
            ('named_local_to_raw_source_diagnostic', 'track_versions_compatible', [local, local.get('filename') or '', remote]),
            ('named_local_to_raw_source_diagnostic', 'recording_versions_compatible', [local, rec.get('title'), rec.get('disambiguation')]),
            ('genre_route_diagnostic_not_execution', 'track_versions_compatible', [remote, '', local]),
            ('genre_route_diagnostic_not_execution', 'recording_versions_compatible', [remote, local_title, None])]
        calls = pair.get('calls', [])
        if len(calls) != 4 or pair.get('rawRecordingDisambiguation') != rec.get('disambiguation'):
            raise ValueError('guard diagnostics missing or stale')
        for i, (call, (context, function, args)) in enumerate(zip(calls, expected)):
            result = call.get('result')
            valid_result = result is None if i == 1 and rec.get('title') is None else isinstance(result, bool)
            if call.get('context') != context or call.get('function') != function or call.get('args') != args or not valid_result:
                raise ValueError('guard invocation does not bind prewrite/raw source inputs')
    if len(seen) != len(tracks)*len(raw_by_selector):
        raise ValueError('guard occurrence matrix incomplete')


def source_observation(snapshot, receipt, body):
    request, provider = snapshot['request'], receipt['provider']
    local, remote = request['tracks'], occurrences(body, provider)
    reasons, mapped, used = [], [], []
    album = request.get('album_hint')
    source_album = body.get('title')
    album_key = lambda title: normal(re.sub(r'\s*[([]deluxe(?: edition)?[)\]]', '', title or '', flags=re.I))
    context = album_key(album) == album_key(source_album) and bool(album_key(album))
    if not context:
        reasons.append('album_context_conflict' if album and source_album else 'album_context_unavailable')
    if album and 'deluxe' in album.casefold() and 'deluxe' not in ' '.join(
            [source_album or '', body.get('disambiguation') or '']).casefold():
        reasons.append('deluxe_context_unavailable')
    principal = (body.get('artist-credit') or body.get('artists') or [{}])[0]
    principal_name = principal.get('name') or (principal.get('artist') or {}).get('name')
    principal_matches = bool(principal_name) and normal(principal_name) == normal(request.get('artist_hint'))
    if not principal_matches:
        reasons.append('primary_artist_context_unresolved')
    if len(local) != len(remote):
        reasons.append('package_count_mismatch')
    guard_source = next((s for s in snapshot['guards'] if s['provider'] == provider
                         and s['path'] == receipt['path'] and s['sha256'] == receipt['sha256']
                         and query_pairs(s.get('query')) == query_pairs(receipt.get('query'))), None)
    for i, track in enumerate(local):
        alternatives = [(selector, raw) for selector, raw in remote
                        if title_key(track.get('title')) and title_key(track.get('title')) == title_key(raw.get('title'))]
        close = [(selector, raw) for selector, raw in alternatives
                 if duration_fact(track.get('length'), seconds(raw, provider))['withinBound']]
        candidates = close if close else alternatives
        row_reasons = []
        if len(candidates) != 1:
            row_reasons.append('unmapped_occurrence' if not candidates else 'ambiguous_mapping')
            mapped.append(dict(localIndex=i, physicalPath=snapshot['baselineTracks'][i]['path'],
                               alternatives=[s for s, _ in alternatives], sourceSelector=None, reasons=row_reasons))
            reasons.extend(row_reasons)
            continue
        selector, raw = candidates[0]
        used.append(selector)
        rt = seconds(raw, provider)
        rec = raw.get('recording') or {}
        rd = rec.get('length')
        rd = rd/1000 if isinstance(rd, (int, float)) and not isinstance(rd, bool) else None
        facts = dict(releaseTrackSeconds=rt, recordingSeconds=rd,
                     releaseTrack=duration_fact(track.get('length'), rt),
                     recording=duration_fact(track.get('length'), rd), units='seconds; MB input milliseconds')
        if not facts['releaseTrack']['withinBound'] or ('length' in rec and not facts['recording']['withinBound']):
            row_reasons.append('duration_evidence_insufficient')
        q = qualifiers(track, raw, dict(localAlbum=request.get('album_hint'), localFolder=request.get('folder_album_hint'),
                                       sourceAlbum=body.get('title'), sourceDisambiguation=body.get('disambiguation')))
        if q['applicability'] == 'contradicted':
            row_reasons.append('qualifier_contradiction')
        elif q['applicability'] == 'unresolved':
            row_reasons.append('qualifier_applicability_unresolved')
        credit = credits(raw, provider, snapshot['baselineTracks'][i])
        if credit['reason']:
            row_reasons.append(credit['reason'])
        if credit['nonNested']:
            row_reasons.append('non_nested_performer_assertions')
        guard = next((g for g in guard_source['pairs'] if g['localIndex'] == i and g['sourceSelector'] == selector), None) if guard_source else None
        if guard is None:
            row_reasons.append('guard_observation_unavailable')
        else:
            # Keep both conventions, not whichever happens to pass. These are
            # diagnostics, not proof this raw source reached a production route.
            if any(call['result'] is False for call in guard['calls']):
                row_reasons.append('unchanged_guard_failed')
            if any(call['result'] is None for call in guard['calls']):
                row_reasons.append('guard_input_unavailable')
        if rec.get('video') is True:
            row_reasons.append('video_occurrence')
        mapped.append(dict(localIndex=i, physicalPath=snapshot['baselineTracks'][i]['path'],
                           sourceSelector=selector, alternatives=[s for s, _ in alternatives], raw=raw,
                           durations=facts, qualifiers=q, credits=credit, guardObservation=guard,
                           conflictScope=('eligible_occurrence_observation' if context and principal_matches and
                               facts['releaseTrack']['withinBound'] and ('length' not in rec or facts['recording']['withinBound']) and q['applicability'] in
                               ['matching_claims', 'no_marked_variant_obstacle'] and credit['agreement'] == 'direct_agreement'
                               else 'unresolved_relevant' if context else 'excluded_unrelated_context'),
                           reasons=row_reasons))
        reasons.extend(row_reasons)
    if len(set(used)) != len(used):
        reasons.append('duplicate_source_mapping')
        for row in mapped:
            if used.count(row.get('sourceSelector')) > 1:
                row['conflictScope'] = 'unresolved_relevant'
    if len(set(used)) != len(remote):
        reasons.append('uncovered_source_occurrences')
    unmapped = [(selector, raw) for selector, raw in remote if selector not in used]
    mapped_keys = {title_key(raw.get('title')) for selector, raw in remote if selector in used}
    if any(title_key(raw.get('title')) in mapped_keys and VARIANT.search(raw.get('title') or '')
           for _, raw in unmapped):
        reasons.append('unmatched_versioned_duplicate_content')
    structural = {'package_count_mismatch', 'album_context_conflict', 'unmatched_versioned_duplicate_content',
                  'video_occurrence', 'qualifier_contradiction'}
    catalog_obstacles = structural | {'unmapped_occurrence', 'ambiguous_mapping', 'duplicate_source_mapping', 'uncovered_source_occurrences',
                                     'duration_evidence_insufficient', 'album_context_unavailable',
                                     'primary_artist_context_unresolved', 'deluxe_context_unavailable'}
    rejected = bool(set(reasons) & structural)
    return dict(receipt=receipt, rawAlbum={k: body[k] for k in
                ['id', 'title', 'disambiguation', 'artist-credit', 'artists', 'genres', 'styles', 'media'] if k in body},
                occurrences=mapped, localCount=len(local), sourceCount=len(remote),
                unmappedSourceOccurrences=[dict(selector=s, raw=r) for s, r in unmapped],
                catalogSupport='supported' if not set(reasons) & catalog_obstacles else 'not_established',
                authorizationDonorScope='complete_catalog_package' if not set(reasons) & catalog_obstacles else 'excluded_package',
                authorizationDonor=False, decision='rejected' if rejected else 'insufficient',
                reasons=sorted(set(reasons) | {'observational_only_no_authority'}))


def export_evidence(snapshot, manifest_path):
    if snapshot.get('purpose') != 'non_authorizing_evidence_only' or snapshot.get('writableDelta') != []:
        raise ValueError('invalid non-authorizing snapshot')
    tracks, baseline = snapshot['request']['tracks'], snapshot['baselineTracks']
    profile = snapshot['profile']
    if profile not in ['folder_filename', 'assisted_without_ids', 'tagged_recovery']:
        raise ValueError('unknown profile')
    if not tracks or len(tracks) != len(baseline) or len(tracks) != len(snapshot['positions']):
        raise ValueError('prewrite occurrence binding incomplete')
    paths = [t['path'] for t in baseline]
    if len(set(paths)) != len(paths) or len(snapshot['baselineHashes']) != len(paths):
        raise ValueError('prewrite physical identity incomplete')
    for track, tag, position in zip(tracks, baseline, snapshot['positions']):
        relative = pathlib.Path(tag['path']).relative_to(snapshot['request']['path']).as_posix()
        hashes = snapshot['baselineHashes'].get(relative, {})
        if not all(re.fullmatch(r'[a-f0-9]{64}', hashes.get(k, '')) for k in ['fullSha256', 'payloadSha256']):
            raise ValueError('prewrite physical hash binding incomplete')
        expected_title = tag.get('title') or re.sub(r'^\d{1,3}\s*[.\-_\s]+', '', pathlib.Path(relative).stem).strip() or None
        if (track.get('length') != tag.get('duration') or track.get('title') != expected_title
                or track.get('filename') != pathlib.Path(relative).stem
                or track.get('artist') != tag.get('artist')
                or track.get('artists', []) != (tag.get('artists') or ([tag['artist']] if tag.get('artist') else []))
                or position['tagged'] != tag.get('trackNumber')
                or position['filenameDerived'] != (int(m[1]) if (m := re.match(r'^(\d{1,3})\s*[.\-_\s]+', pathlib.Path(relative).stem)) else None)
                or position['lookupPosition'] != track.get('track_number')
                or track.get('track_total') != len(tracks)
                or track.get('disc_number') != tag.get('discNumber')
                or track.get('disc_total') != tag.get('discTotal')
                or track.get('musicbrainz_trackid') != tag.get('musicbrainzTrackId')
                or position['fileCountDerivedTotal'] != len(tracks)):
            raise ValueError('prewrite request does not bind the original baseline')
    ids = ['musicbrainzTrackId', 'musicbrainzAlbumId', 'musicbrainzArtistId', 'discogsArtistId', 'discogsReleaseId']
    if profile != 'tagged_recovery' and (any(snapshot['request'].get(k) for k in
            ['musicbrainz_album_id', 'musicbrainz_artist_id', 'discogs_artist_id', 'discogs_release_id'])
            or any(t.get('musicbrainz_trackid') for t in tracks)):
        raise ValueError('profile request contains stripped IDs')
    for t in baseline:
        if profile != 'tagged_recovery' and any(t.get(k) for k in ids):
            raise ValueError('profile contains stripped IDs')
        if profile == 'folder_filename' and any(t.get(k) for k in
                ['title', 'artist', 'artists', 'album', 'albumArtist', 'albumArtists', 'year', 'genre',
                 'trackNumber', 'trackTotal', 'discNumber', 'discTotal']):
            raise ValueError('profile contains stripped tags')
    for field, tag_field in [('musicbrainz_album_id', 'musicbrainzAlbumId'), ('musicbrainz_artist_id', 'musicbrainzArtistId'),
                             ('discogs_release_id', 'discogsReleaseId'), ('discogs_artist_id', 'discogsArtistId')]:
        expected = next((t[tag_field] for t in baseline if t.get(tag_field)), None)
        if snapshot['request'].get(field) != expected:
            raise ValueError('prewrite embedded identity binding mismatch')
    manifest_path = pathlib.Path(manifest_path)
    manifest_bytes = manifest_path.read_bytes()
    manifest = json.loads(manifest_bytes)
    if manifest.get('schemaVersion') != 1:
        raise ValueError('invalid receipt manifest')
    attempt = snapshot['attempt']
    requests = attempt['requests']
    received, sources = [], []
    guard_keys = [(g['provider'], g['path'], query_pairs(g.get('query'))) for g in snapshot['guards']]
    if len(set(guard_keys)) != len(guard_keys):
        raise ValueError('duplicated guard source receipt')
    receipt_keys = [(r['provider'], r['path'], query_pairs(r.get('query'))) for r in manifest['records']]
    if len(set(receipt_keys)) != len(receipt_keys):
        raise ValueError('ambiguous receipt identity')
    for request in requests:
        if request['status'] == 200 and (request['provider'], request['path'], query_pairs(request['query'])) not in receipt_keys:
            raise ValueError('successful request has no receipt')
    for guard in snapshot['guards']:
        matches = [r for r in manifest['records'] if r['provider'] == guard['provider'] and r['path'] == guard['path']
                   and r['sha256'] == guard['sha256'] and query_pairs(r.get('query')) == query_pairs(guard.get('query'))]
        if len(matches) != 1:
            raise ValueError('guard does not bind a source receipt')
        if not any(r['provider'] == guard['provider'] and r['path'] == guard['path']
                   and query_pairs(r['query']) == query_pairs(matches[0].get('query'))
                   and r['status'] == 200 for r in requests):
            raise ValueError('guard source was not received')
    for receipt in manifest['records']:
        query = query_pairs(receipt.get('query'))
        matches = [r for r in requests if r['provider'] == receipt['provider'] and r['path'] == receipt['path']
                   and query_pairs(r['query']) == query]
        if not matches:
            continue
        raw_bytes = (manifest_path.parent / receipt['bodyFile']).read_bytes()
        if digest(raw_bytes) != receipt['sha256']:
            raise ValueError('source hash mismatch')
        if any(r['status'] != receipt.get('status', 200) for r in matches):
            raise ValueError('request/receipt status mismatch')
        body = json.loads(raw_bytes)
        for guard in snapshot['guards']:
            if guard['provider'] == receipt['provider'] and guard['path'] == receipt['path'] and guard['sha256'] == receipt['sha256']:
                validate_guards(guard, receipt, body, tracks)
        received.append(dict(receipt=receipt, attempts=matches))
        detail = re.fullmatch(r'/release[s]?/([^/]+)', receipt['path'])
        if detail and receipt.get('status', 200) == 200 and any(r['status'] == 200 for r in matches):
            if str(body.get('id')) != detail[1]:
                raise ValueError('source identity mismatch')
            sources.append(source_observation(snapshot, receipt, body))
    reasons = ['observational_only_no_authority']
    if (not attempt.get('closed') or attempt.get('pendingRelevant') or attempt.get('misses')
            or any(r['status'] >= 400 for r in requests)):
        reasons.append('bounded_relevant_sources_incomplete')
    # This telemetry cannot see every scheduled/cancelled request. A completed
    # process is not proof of exhaustive discovery or a closed relevant-source set.
    if not attempt.get('searchComplete'):
        reasons.append('source_set_completeness_unestablished')
    conflicts = []
    for index in range(len(tracks)):
        assertions = [dict(source=s['receipt'], names=row['credits']['completeNames'], scope=row['conflictScope'])
                      for s in sources for row in s['occurrences'] if row['localIndex'] == index
                      and row.get('conflictScope') != 'excluded_unrelated_context'
                      and row.get('credits', {}).get('completeNames')]
        sets = [{normal(n) for n in a['names']} for a in assertions]
        if any(not (a <= b or b <= a) for i, a in enumerate(sets) for b in sets[i+1:]):
            conflicts.append(dict(localIndex=index, assertions=assertions, disposition='held_non_nested_assertions'))
    if conflicts:
        reasons.append('non_nested_source_assertions')
    return dict(schemaVersion=1, purpose='non_authorizing_evidence_only', writableDelta=[], acceptedCertificate=False,
                decision='insufficient', reasons=reasons,
                preservedAuthority='all existing field decisions, IDs, genre rules, certificates, and review obligations; this export supplies none',
                unavailableObservations=['exhaustive pending/cancelled relevant-source set', 'separate local decision/cleanup store'],
                prewrite={k: snapshot[k] for k in ['profile', 'caseId', 'phase', 'request', 'baselineTracks', 'baselineHashes', 'positions']},
                evidenceOrigins=dict(baselineTracks='actual_prewrite_reader', request='actual_prewrite_lookup',
                                     positions='native_tagged_filename_and_file_count_observations',
                                     rawSources='hash_verified_received_receipts_not_output'),
                binding=dict(snapshotSha256=digest(json.dumps(snapshot, sort_keys=True).encode()),
                             manifestSha256=digest(manifest_bytes),
                             policy='field-scoped-performer-certificate-20261109/evidence-only-v1',
                             compiledPolicies=snapshot.get('compiledPolicySha256'),
                             exporterSha256=digest(pathlib.Path(__file__).read_bytes())),
                identityClaims=dict(edition='not_established_by_this_export', acoustic='unverified',
                                    cleanExplicitAudio='unverified', mastering='unverified'),
                attempt=dict(observed=attempt, received=received,
                             decisionRevisionDigest=digest(json.dumps([snapshot, received], sort_keys=True).encode()),
                             invalidation='any changed prewrite input, policy, source, or new relevant receipt invalidates this observation'),
                sources=sources, fieldConflicts=conflicts, nativeRouteTraces=snapshot.get('nativeRouteTraces'),
                unresolvedRelevantAssertions=[dict(source=s['receipt'], localIndex=row['localIndex'],
                                                   reasons=row['reasons'], directGuests=row.get('credits', {}).get('directGuests'))
                                              for s in sources for row in s['occurrences']
                                              if row.get('conflictScope') == 'unresolved_relevant'],
                groundedAssistanceAccuracyCredit=0)


if __name__ == '__main__':
    if len(sys.argv) != 4:
        raise SystemExit('Usage: export-evidence.py NATIVE-EVIDENCE.json PROVIDER-MANIFEST.json NEW-OUTPUT.json')
    snapshot_path, manifest_path, output_path = map(pathlib.Path, sys.argv[1:])
    snapshot_bytes = snapshot_path.read_bytes()
    result = export_evidence(json.loads(snapshot_bytes), manifest_path)
    result['binding']['snapshotFileSha256'] = digest(snapshot_bytes)
    with output_path.open('x') as output:
        json.dump(result, output, indent=2, ensure_ascii=False, allow_nan=False)
        output.write('\n')
