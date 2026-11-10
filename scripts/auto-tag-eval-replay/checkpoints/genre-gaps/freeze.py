"""Freeze source-grounded routing and exact offline scripts before execution."""
import collections
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]
PRIOR = HERE.parent / 'reviewed-51-content-preservation-20261110'
load = lambda p: json.loads(p.read_text())
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
save = lambda n, x: (HERE / n).open('x').write(json.dumps(x, indent=2, ensure_ascii=False))
previous = load(PRIOR / 'preflight-locks.json')
for name, expected in previous['sourceLocks'].items():
    assert sha(ROOT / name) == expected, name
selection = load(PRIOR / 'selection.json')
assert sha(pathlib.Path(selection['binary'])) == selection['binaryHash']
shutil.copyfile(selection['binary'], HERE / 'native-evaluator')
shutil.copyfile(PRIOR / 'source.tgz', HERE / 'source.tgz')
gaps = load(PRIOR / 'genre-absence-diagnostics.json')
cases = {c['caseId']: c for c in selection['cases']}
jobs, diagnoses = [], []


def make(case_id, profile, scenario, kind='gap', genres=None):
    case = cases[case_id]
    directory = PRIOR / profile / case_id
    record = load(directory / 'cold.jsonl')
    candidate = record['native'].get('candidate') or {}
    payload = dict(artist=candidate.get('artist'), album=candidate.get('album'),
                   tracks=[t['title'] for t in candidate.get('tracks', []) if t.get('title') is not None])
    response = dict(genre='Offline test genre', confidence=0.8)
    status = 200
    schema = 'GenreFillResponse'
    if scenario == 'rejected':
        response['confidence'] = 0.2
    if scenario == 'invalid':
        response = dict(genre='', confidence='not-a-number')
    if scenario == 'failed':
        status = 400
    if kind == 'hold':
        schema = 'TagCorrectionResponse'
        snapshot = load(directory / f'evidence-cold-{case_id}.json')
        request = snapshot['request']
        folder = pathlib.PurePath(request['path'])
        payload = dict(folder_name=folder.name, parent_name=folder.parent.name,
                       full_path='$EVAL_MEDIA/' + request['path'].split('/media/cold/', 1)[1],
                       parsed_hints={k: request.get(k + '_hint') for k in ['artist', 'album', 'year']},
                       current_tracks=[dict(index=i, **{k: t.get(k) for k in
                           ['title', 'filename', 'artist', 'artists', 'track_number', 'genre']})
                           for i, t in enumerate(request['tracks'])])
        if case_id == 'f7756354af37':
            response = load(ROOT / 'scripts/auto-tag-eval-replay/safety-model-inputs.json')['response']
        else:
            response = dict(artist='Billie Eilish', artists=['Billie Eilish'], album='Bellyache',
                            albumArtist='Billie Eilish', albumArtists=['Billie Eilish'], year='2017',
                            genre='Offline test genre', confidence=0.99,
                            tracks=[dict(index=0, title='Bellyache', artist='Billie Eilish',
                                         artists=['Billie Eilish'], trackNumber=1, trackTotal=1,
                                         discNumber=1, discTotal=1)])
    jobs.append(dict(case=case, profile=profile, scenario=scenario, kind=kind,
                     name=f'{kind}-{scenario}-{profile}-{case_id}', genres=genres or {},
                     expectedModelRequests=0 if scenario == 'disabled' or kind in ['provider', 'all_local'] else 2,
                     script=dict(maxRequests=2, schemaName=schema, payload=payload, response=response, status=status),
                     expected='no_write_hold' if kind == 'hold' else 'cancel_no_write' if scenario == 'cancelled'
                              else 'inferred_unverified' if scenario == 'accepted' and kind not in ['provider', 'all_local']
                              else 'provider_or_retained_incomplete',
                     baseline=str(directory.relative_to(ROOT))))


for gap in gaps:
    cid, profile = gap['caseId'], gap['profile']
    traces = load(PRIOR / profile / cid / f'donors-cold-{cid}.json')
    considered = [c for t in traces if t['field'] == 'genre' for c in t['candidates'] if c['candidate'].get('genre')]
    assert not any(c['genreEligible'] for c in considered), (cid, profile, 'unexpected eligible genre')
    diagnoses.append(dict(caseId=cid, profile=profile, missingFiles=gap['missingFilesPerPhase'],
                          category='all_considered_genre_donors_ineligible' if considered else 'no_considered_genre_bearing_candidate',
                          rejections=[dict(source=c['candidate']['source'], album=c['candidate']['album'],
                                           strictError=c['strictError'], genreEligible=c['genreEligible'],
                                           versionConflicts=[v for v in c['versions'] if not(v['trackCompatible'] and v['recordingCompatible'])])
                                      for c in considered]))
    for scenario in ['accepted', 'rejected', 'invalid', 'failed', 'disabled', 'cancelled']:
        make(cid, profile, scenario)
for cid, profile in [('4060f1de6662', 'assisted_without_ids'), ('7aca3617ff1a', 'folder_filename')]:
    make(cid, profile, 'accepted', 'provider')
cid, profile = 'e8a8a8d6b27d', 'assisted_without_ids'
before = load(PRIOR / profile / cid / 'cold.jsonl')['beforeTracks']
local = {pathlib.Path(t['path']).name: 'Explicit local folk' for t in before}
make(cid, profile, 'accepted', 'mixed_local', dict(list(local.items())[:1]))
make(cid, profile, 'accepted', 'all_local', local)
for cid in ['0e7c8a578cea', 'f7756354af37']:
    for profile in selection['profiles']:
        for scenario in ['disabled', 'accepted']:
            make(cid, profile, scenario, 'hold')
assert len(gaps) == 34 and len(jobs) == 220
git = lambda *args: subprocess.check_output(['git', *args], cwd=ROOT).decode()
locks = dict(selection['locks'])
for name in ['evidence-manifest.json', 'evidence-manifest.sha256', 'preflight-locks.json', 'selection.json']:
    locks[str((PRIOR / name).relative_to(ROOT))] = sha(PRIOR / name)
save('freeze.json', dict(sourceLocks=previous['sourceLocks'], locks=locks,
                         binarySha256=selection['binaryHash'], sourceArchiveSha256=sha(HERE / 'source.tgz'),
                         branch=git('branch', '--show-current').strip(), head=git('rev-parse', 'HEAD').strip(),
                         status=git('status', '--short'), stagedSha256=hashlib.sha256(subprocess.check_output(['git','diff','--cached','--binary'])).hexdigest(),
                         prior=str(PRIOR.relative_to(ROOT)), jobs=jobs, gapDiagnoses=diagnoses,
                         expectedEvaluations=440, gapEvaluations=408, excludedGapJobs=[],
                         assistanceAccuracyCredit=0, providerCaptures='unaltered authentic frozen captures',
                         limits='Independent certificates do not decide production donor eligibility; native rejections are not universal semantic judgments'))
print('Frozen', len(jobs), 'jobs;', len(gaps), 'gap jobs;', collections.Counter(r['category'] for r in diagnoses))
