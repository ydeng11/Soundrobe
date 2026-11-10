"""Export original metadata and hash-verified raw catalog fields, without native outputs."""
import hashlib
import json
import pathlib

ROOT = pathlib.Path.cwd()
HERE = pathlib.Path(__file__).resolve().parent
FIXTURES = ROOT / 'test/fixtures/tauri/auto-tag-eval'
MANIFEST = HERE.parent / 'reviewed-51-provider-first-20261109/providers/manifest.json'
load = lambda path: json.loads(pathlib.Path(path).read_text())
digest = lambda path: hashlib.file_digest(pathlib.Path(path).open('rb'), 'sha256').hexdigest()
corpus = load(FIXTURES / 'corpus.json')
manifest = load(MANIFEST)


def source(provider, release_id):
    route = ('/releases/' if provider == 'discogs' else '/release/') + release_id
    rows = [r for r in manifest['records'] if r['provider'] == provider
            and r['path'] == route and r.get('status', 200) == 200]
    assert len(rows) == 1, (provider, release_id, len(rows))
    row = rows[0]
    path = (MANIFEST.parent / row['bodyFile']).resolve()
    assert digest(path) == row['sha256'], path
    raw = load(path)
    if provider == 'discogs':
        fields = ['id', 'title', 'year', 'country', 'artists', 'labels', 'formats',
                  'genres', 'styles', 'notes', 'extraartists', 'tracklist']
    else:
        fields = ['id', 'title', 'disambiguation', 'date', 'country', 'barcode',
                  'packaging', 'artist-credit', 'label-info', 'genres', 'tags', 'media']
    return dict(provider=provider, releaseId=release_id, path=str(path.relative_to(ROOT)),
                sha256=row['sha256'], capture=row.get('source'),
                rawFields={key: raw[key] for key in fields if key in raw})


case_sources = {
    '9373204e076a': {
        'discogs': ['1151167', '1571575', '2336317', '2398669', '3512330',
                    '5390450', '5455029', '6425118', '6800188', '20260261'],
        'musicbrainz': ['3cd3587f-8eaf-427b-bd84-cb2f21bed994',
                        'e95722b4-c235-41f5-94db-941bcd24fc09'],
    },
    '8452b6f2b3ea': {
        'discogs': ['36441795', '16649340', '2058854', '38833854'],
        'musicbrainz': ['10e769d6-c110-4e1f-934e-2d12677d4fbb',
                        'c5706707-9bbd-42bc-ad1f-8f9d9450f217',
                        '88a856de-d5d1-4225-9612-8858b2f5353f'],
    },
}
cases = []
for case_id, providers in case_sources.items():
    case = next(c for c in corpus['cases'] if c['caseId'] == case_id)
    cases.append(dict(caseId=case_id, artist=case['artist'],
        sourceRelativeFolder=case['sourceRelativeFolder'],
        capturedMetadataProvenance=corpus['artistReview'][case['artist']],
        originalInventoryTracks=case['tracks'], originalProfiles=case['inputProfiles'],
        originalExpectation=case['expectation'],
        sources=[source(provider, release) for provider, releases in providers.items() for release in releases]))
print(json.dumps(dict(originalCorpusPath=str((FIXTURES / 'corpus.json').relative_to(ROOT)),
    originalCorpusSha256=digest(FIXTURES / 'corpus.json'), manifestSha256=digest(MANIFEST),
    cases=cases, nativeOutputsUsed=False, certificateIssued=False,
    originalAudioRead=False, providerRequests=0, modelRequests=0), indent=2, ensure_ascii=False))
