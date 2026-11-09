"""Independent readback-title audit; never imports the prototype's normalizer."""
import collections
import hashlib
import json
from pathlib import Path
import re
import sys
import unicodedata

ROOT = Path(__file__).resolve().parent
REPO = Path.cwd()
REVIEW = REPO / '.amp/in/artifacts/eval-dummies-offline-20261107/review/expectations.json'


def key(value):
    # Cosmetic punctuation/case only. Accents and lexical qualifiers remain.
    return ''.join(c for c in unicodedata.normalize('NFC', value).casefold() if c.isalnum())


def title_outcome(actual, mapping, artist, case_id):
    if not actual or not actual.strip():
        return {'status': 'missing', 'prefix': False}
    prefix = actual.casefold().startswith((artist + ' - ').casefold())
    value = actual[len(artist) + 3:] if prefix else actual
    accepted = {key(mapping['providerTitle']), key(mapping['localTitle'])}
    # This specific transliteration was independently identified in the audit.
    if case_id == '8452b6f2b3ea' and key(mapping['providerTitle']) == key('Déjà Vu'):
        accepted.add(key('Deja Vu'))
    if case_id == 'c0e96540c785' and key(mapping['providerTitle']) == key('Oíche Chiúin (Chorale)'):
        accepted.add(key('Oíche Chiuin (chorale)'))
    if key(value) in accepted:
        return {'status': 'correct', 'prefix': prefix}
    for title in (mapping['providerTitle'], mapping['localTitle']):
        bare = re.sub(r'\s*\((?:skit|instrumental)\)\s*$', '', title, flags=re.I)
        if bare != title and key(value) == key(bare):
            return {'status': 'incomplete_qualifier', 'prefix': prefix}
    return {'status': 'conflict', 'prefix': prefix}


def load(path):
    return json.loads(Path(path).read_text())


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def score_tracks(tracks, truth, case):
    assert len(tracks) == len(truth['mapping']) == len(case['tracks'])
    mapping = {m['localTrack']: m for m in truth['mapping']}
    details = []
    for index, (track, source) in enumerate(zip(tracks, case['tracks']), 1):
        assert Path(track['path']).name == Path(source['relativePath']).name, (case['caseId'], index)
        result = title_outcome(track['title'], mapping[index], case['artist'], case['caseId'])
        expected_position = [int(n) for n in mapping[index]['providerTrack'].split('-')]
        position_match = track['trackNumber'] == expected_position[-1]
        if len(expected_position) == 2:
            position_match = position_match and track['discNumber'] == expected_position[0]
        details.append({'localTrack': index, 'relativePath': source['relativePath'],
                        'actualTitle': track['title'], 'expectedTitle': mapping[index]['providerTitle'],
                        **result, 'positionMatchesReviewed': position_match})
    counts = collections.Counter(d['status'] for d in details)
    shared_fields = ['album', 'albumArtist', 'albumArtists', 'artist', 'artists', 'year', 'genre', 'trackTotal']
    varying = [field for field in shared_fields if len({json.dumps(t[field], sort_keys=True) for t in tracks}) != 1]
    artist_correct = all(key(t['artist'] or '') == key(case['artist']) for t in tracks)
    populated = all(t['album'] and t['albumArtist'] and t['artist'] and t['trackNumber'] for t in tracks)
    unique_positions = len({(t['discNumber'] or 1, t['trackNumber']) for t in tracks}) == len(tracks)
    clean_titles = counts['correct'] == len(tracks) and not any(d['prefix'] for d in details)
    return {'tracks': len(tracks), 'titleCounts': dict(counts),
            'artistPrefixes': sum(d['prefix'] for d in details),
            'allContentTitlesCorrect': counts['correct'] == len(tracks),
            'allCleanTitlesCorrect': clean_titles, 'albumArtistPolicyCorrect': artist_correct,
            'varyingSharedFields': varying, 'coreFieldsPopulated': bool(populated),
            'uniquePositions': unique_positions,
            'reviewedPositionsCorrect': sum(d['positionMatchesReviewed'] for d in details),
            'basicReady': bool(clean_titles and artist_correct and populated and unique_positions and not varying),
            'sharedValues': {f: tracks[0][f] for f in shared_fields}, 'details': details}


def run():
    selection = load(ROOT.parent / 'reviewed-20-recording-recovery-20261107/selection.json')
    expected = {c['caseId']: c for c in load(REVIEW)['cases']}
    corpus = {c['caseId']: c for c in load(REPO / '.amp/in/eval-dummies/original/corpus.json')['cases']}
    rows = []
    for case_id in selection['caseIds']:
        for phase in ['cold', 'warm']:
            record = json.loads((ROOT / 'best-aligned/cases' / case_id / f'{phase}.jsonl').read_text().strip().splitlines()[-1])
            experiment = record['albumContentComparison']
            assert experiment['sameProfileBaseline'] and experiment['readback'] and experiment['payloadUnchanged']
            assert record['readback'] and record['payloadUnchanged'] and record['error'] is None
            assert expected[case_id]['status'] == 'verified_match'
            baseline = score_tracks(experiment['baselineAfter'], expected[case_id], corpus[case_id])
            proposal = score_tracks(experiment['proposalAfter'], expected[case_id], corpus[case_id])
            # Additive composition, not another executed native policy. Keep confirmed editions.
            use_proposal = record['outcome'] != 'applied' and experiment['status'] == 'basic_tags_written'
            hybrid = proposal if use_proposal else baseline
            rows.append({'caseId': case_id, 'phase': phase, 'folder': record['sourceRelativeFolder'],
                         'existingOutcome': record['outcome'], 'existingWritten': record['native']['written'],
                         'proposalStatus': experiment['status'], 'proposalReason': experiment['reason'],
                         'proposalWritten': experiment['written'], 'hybridUsesProposal': use_proposal,
                         'baseline': baseline, 'proposal': proposal, 'hybrid': hybrid})
    totals = {}
    for phase in ['cold', 'warm']:
        phase_rows = [r for r in rows if r['phase'] == phase]
        totals[phase] = {}
        for arm in ['baseline', 'proposal', 'hybrid']:
            counts = collections.Counter()
            for row in phase_rows:
                counts.update(row[arm]['titleCounts'])
            totals[phase][arm] = {'titleCounts': dict(counts), 'tracks': sum(r[arm]['tracks'] for r in phase_rows),
                                  **{field: sum(bool(r[arm][field]) for r in phase_rows)
                                     for field in ['allContentTitlesCorrect', 'allCleanTitlesCorrect', 'basicReady']},
                                  'artistPrefixes': sum(r[arm]['artistPrefixes'] for r in phase_rows),
                                  'reviewedPositionsCorrect': sum(r[arm]['reviewedPositionsCorrect'] for r in phase_rows)}
    output = {'rubric': 'independent-reviewed-title-v1', 'scorerSha256': digest(__file__),
              'reviewSha256': digest(REVIEW), 'caseCount': len(selection['caseIds']), 'totals': totals, 'rows': rows}
    (ROOT / 'comparison-score-best-aligned.json').write_text(json.dumps(output, indent=2, ensure_ascii=False) + '\n')
    print(json.dumps(totals, indent=2))


def self_test():
    def classify(actual, expected, local=None, artist='Enya', case='test'):
        return title_outcome(actual, {'providerTitle': expected, 'localTitle': local or expected}, artist, case)
    assert classify('Ebudæ', 'Ebudæ', 'Ebudae')['status'] == 'correct'
    assert classify('Ebudae', 'Ebudæ', 'Ebudae')['status'] == 'correct'
    assert classify('Doolin‐Dalton', 'Doolin-Dalton')['status'] == 'correct'
    assert classify('Enya - Song', 'Song') == {'status': 'correct', 'prefix': True}
    assert classify('Other - Song', 'Song')['status'] == 'conflict'
    assert classify('Song (Live)', 'Song (Remix)')['status'] == 'conflict'
    assert classify('To Go Beyond (II)', 'To Go Beyond (I)')['status'] == 'conflict'
    assert classify('Dr. West', 'Dr. West (Skit)', artist='Eminem')['status'] == 'incomplete_qualifier'
    assert classify('Doolin-Dalton', 'Doolin-Dalton (instrumental)')['status'] == 'incomplete_qualifier'
    assert classify('Deja Vu', 'Déjà Vu', case='8452b6f2b3ea')['status'] == 'correct'
    assert classify('Oíche Chiuin (chorale)', 'Oíche Chiúin (Chorale)', case='c0e96540c785')['status'] == 'correct'
    assert classify('Smaoinim', 'Smaointe...')['status'] == 'conflict'
    assert classify(None, 'Song')['status'] == 'missing'
    assert classify('Exile', 'Exile = エクサイル（流浪）', 'Exile', case='e8a8a8d6b27d')['status'] == 'correct'
    assert classify('Song', 'Song = Different song')['status'] == 'conflict'
    print('15 independent title-oracle contracts passed')


if __name__ == '__main__':
    self_test()
    if '--score' in sys.argv:
        run()
