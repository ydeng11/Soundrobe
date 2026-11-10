"""Independent field/review gate for run-assisted.cjs; no model-as-oracle scoring."""
import hashlib
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
snapshot = json.loads((root / 'snapshot.json').read_text())
findings = []
records = []


def require(ok, job, category, detail):
    if not ok:
        findings.append(dict(job=job, category=category, detail=detail))


for file, expected in snapshot['locks'].items():
    require(hashlib.sha256(pathlib.Path(file).read_bytes()).hexdigest() == expected,
            file, 'input_snapshot', 'source or input changed since run')
require(hashlib.sha256((root / 'native-evaluator').read_bytes()).hexdigest() == snapshot['binarySha256'],
        'binary', 'input_snapshot', 'executable changed')
for scenario in snapshot['scenarios']:
    for profile in snapshot['profiles']:
        job = f'{scenario}-{profile}'
        directory = root / job
        inventory = json.loads((directory / 'inventory.json').read_text())
        require(not inventory['misses'], job, 'provider_coverage', 'unexpected request')
        require(len(inventory['modelRequests']) == (0 if scenario == 'disabled' else 2),
                job, 'agent_request', 'missing or extra model requests')
        phases = []
        for phase in ['cold', 'warm']:
            file = directory / f'{phase}.jsonl'
            lines = file.read_text().splitlines() if file.exists() else []
            require(len(lines) == 1, job, 'evaluator', f'{phase} missing or duplicate')
            if len(lines) != 1:
                continue
            r = json.loads(lines[0])
            records.append(r)
            n = r.get('native') or {}
            require(r['payloadUnchanged'] and r['readback'], job, 'write_readback', phase)
            require(n.get('written') == 1 and n.get('authority') == 'musicbrainz',
                    job, 'provider_selection', f'{phase}: provider match lost')
            tracks = r.get('readbackTracks', [])
            require(len(tracks) == 1, job, 'mapping', 'physical file missing')
            if not tracks:
                continue
            track = tracks[0]
            require(track['path'] == r['beforeTracks'][0]['path'], job, 'mapping', 'file binding changed')
            # Frozen review accepts capitalization only, not arbitrary normalized/version titles.
            require(track['title'] in ['ocean eyes', 'Ocean Eyes']
                    and track['artists'] == ['Billie Eilish']
                    and track['musicbrainzTrackId'] == '5cbc1f32-3bfd-48e9-953c-5ec847f86370',
                    job, 'identity_credit_version', phase)
            accepted = scenario == 'accepted'
            require(track['genre'] == ('Pop' if accepted else None), job, 'field_authorization', phase)
            require(r['reviewPendingAfterRestart'] == accepted, job, 'review_highlight', phase)
            require(n.get('outcome') == ('needs_review' if accepted else 'applied'),
                    job, 'review_highlight', f'{phase}: incorrect outcome')
            candidate = n.get('candidate') or {}
            provenance = candidate.get('assisted_genre')
            require(bool(provenance) == accepted, job, 'review_highlight', f'{phase}: missing/extra field provenance')
            if accepted and provenance:
                require(provenance['model'] == 'scripted/genre-v1' and provenance['value'] == 'Pop',
                        job, 'source_grounding', 'scripted inference mislabeled')
                require(r['persistedEvidence']['candidate']['assisted_genre'] == provenance,
                        job, 'review_highlight', 'field provenance not persisted')
            phases.append({key: track[key] for key in ['title', 'artists', 'genre', 'musicbrainzTrackId']})
        require(len(phases) == 2 and phases[0] == phases[1], job, 'cache_isolation', 'cold/warm difference')
require(len(records) == snapshot['requestedPhysicalFiles'], 'denominator', 'evaluator', 'incomplete matrix')
summary = dict(clean=not findings, evaluations=len(records), requestedPhysicalFiles=snapshot['requestedPhysicalFiles'],
               groundedAssistedAccuracyCredit=0, findings=findings,
               limits='One development release, scripted genre inference; not live research or generalization evidence')
(root / 'audit.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary, indent=2))
sys.exit(0 if summary['clean'] else 1)
