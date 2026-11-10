"""Derive one authorized expectation correction from bound source and discovery evidence."""
import copy
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
OLD = HERE.with_name('genre-ai-gaps-20261110')
ROOT = HERE.parents[3]
NAME = 'hold-accepted-folder_filename-f7756354af37'
load = lambda path: json.loads(path.read_text())


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def source_span(path, start, end, required):
    text = (ROOT / path).read_text()
    first = text.index(start)
    last = text.index(end, first) + len(end)
    excerpt = text[first:last]
    for value in required:
        assert value in excerpt, value
    return dict(path=path, sourceSha256=digest(ROOT / path), startSelector=start,
                endSelector=end, firstLine=text[:first].count('\n') + 1,
                lastLine=text[:last].count('\n') + 1, text=excerpt,
                excerptSha256=hashlib.sha256(excerpt.encode()).hexdigest())


if __name__ == '__main__':
    manifest = load(OLD / 'evidence-manifest.json')
    manifest_hash = digest(OLD / 'evidence-manifest.json')
    assert manifest_hash == (OLD / 'evidence-manifest.sha256').read_text().split()[0]
    assert manifest_hash == '86d69ab72c3c8e62806763f72ddd063d90eb5453bd1e54728e99b2b5334bdfd8'
    for name, row in manifest['files'].items():
        assert (OLD / name).stat().st_size == row['bytes'] and digest(OLD / name) == row['sha256'], name
    frozen = load(OLD / 'freeze.json')
    assert load(HERE / 'freeze.json') == frozen
    for locks in [frozen['sourceLocks'], frozen['locks']]:
        for name, expected in locks.items():
            path = Path(name)
            assert digest(path if path.is_absolute() else ROOT / path) == expected, name
    assert digest(OLD / 'source.tgz') == frozen['sourceArchiveSha256']
    assert digest(OLD / 'native-evaluator') == frozen['binarySha256']

    source = 'src-tauri/src/commands/auto_tag.rs'
    editions = 'src-tauri/src/commands/auto_tag_editions.rs'
    spans = [
        source_span(editions, 'const DETAIL_LIMIT: usize = 40;', 'const PAGE_LIMIT: u32 = 5;', ['40']),
        source_span(editions, '    pub async fn detail(', '        self.attempted.insert(id.to_string());',
                    ['self.attempted.len() >= DETAIL_LIMIT', 'Discogs discovery incomplete: release detail limit reached']),
        source_span(source, 'fn provider_error_status(', '\nfn provider_diagnostic(',
                    ['error.contains("incomplete") || error.contains("budget")', 'ProviderAttemptStatus::Incomplete']),
        source_span(source, 'fn provider_authority_decision(', '\nfn normalized_release_identity(',
                    ['attempt.status == ProviderAttemptStatus::Incomplete', 'ProviderAuthorityDecision::Incomplete', 'ProviderAuthorityDecision::Providerless']),
        source_span(source, 'fn providers_confirmed_no_match(', '\npub struct AutoTagServices',
                    ['matches!(provider_decision, ProviderAuthorityDecision::Providerless)', 'attempt.status == ProviderAttemptStatus::NoMatch']),
        source_span(source, '    let providers_confirmed_no_match =', '\n    let Some(mut candidate) = candidate else {',
                    ['if candidate.is_none() && providers_confirmed_no_match', 'resolve_tags_via_llm(&request, config, cancelled)']),
        source_span(source, '            } else if provider_attempts.iter().any(|attempt| {', '            } else if provider_attempts\n',
                    ['reason.contains("limit reached") || reason.contains("budget")', '"discovery_budget_limited"']),
    ]
    job = next(j for j in frozen['jobs'] if j['name'] == NAME)
    evidence = []
    for phase in ['cold', 'warm']:
        path = OLD / 'runs' / NAME / f'{phase}.jsonl'
        record = load(path)
        attempts = record['native']['providerAttempts']
        assert attempts == [dict(provider='musicbrainz', retryable=False, status='no_match'),
                            dict(provider='discogs', retryable=False, status='incomplete',
                                 diagnostic='Discogs discovery incomplete: release detail limit reached')]
        selection = next(d for d in record['native']['diagnostics'] if d.get('stage') == 'provider_selection')
        assert selection['credibleCounts'] == dict(discogs=0, musicbrainz=0)
        discovery = [d for d in record['native']['diagnostics'] if d.get('stage') == 'discogs_versions']
        assert max(d.get('detailRequests', 0) for d in discovery) == 40
        assert any(d.get('incomplete') is True and d['reason'] == attempts[1]['diagnostic'] for d in discovery)
        baseline = ROOT / job['baseline'] / f'{phase}.jsonl'
        assert load(baseline)['native']['providerAttempts'] == attempts
        evidence.append(dict(phase=phase, record=str(path.relative_to(ROOT)), recordSha256=digest(path),
                             baseline=str(baseline.relative_to(ROOT)), baselineSha256=digest(baseline),
                             attempts=attempts, credibleCounts=selection['credibleCounts'], detailLimit=40,
                             discovery=[d for d in discovery if d.get('incomplete')]))
    corrected = copy.deepcopy(frozen)
    index = next(i for i, j in enumerate(corrected['jobs']) if j['name'] == NAME)
    assert corrected['jobs'][index]['expectedModelRequests'] == 2
    corrected['jobs'][index]['expectedModelRequests'] = 0
    proof = dict(version=2, authorization=dict(
        sourceThread='https://ampcode.com/threads/T-01a1b234-730d-7557-9ba6-fc105c23f683',
        continuationThread='https://ampcode.com/threads/T-01a1be8d-794b-75ce-a0fb-4aca6f65d0db',
        request='Correct the Enya routing test expectation from source evidence, preserve the old results, and rerun the audit.'),
        originalManifestSha256=manifest_hash, originalFilesVerified=len(manifest['files']),
        originalFrozenSelectionSha256=digest(OLD / 'freeze.json'),
        sourceArchiveSha256=frozen['sourceArchiveSha256'], executableSha256=frozen['binarySha256'],
        sourceSpans=spans, discoveryEvidence=evidence,
        changes=[dict(selector=f'/jobs/{index}/expectedModelRequests', job=NAME, old=2, new=0,
                      unit='total model requests per cold/warm process; per-phase expectation 1 becomes 0')],
        derivation='No credible selected candidate plus an incomplete Discogs attempt yields Incomplete, not Providerless. The no-match gate is false, so metadata fallback is forbidden. Exhausting the detail budget leaves unexamined evidence, not a confirmed no-match. The registered metadata script must remain unused.',
        unchanged='All other jobs, scripts, budgets, profile inputs, outcomes, oracles, captures and authorization remain byte- or value-identical. No native process or network request is executed by this tool.')
    with (HERE / 'freeze.json').open('w') as stream:
        json.dump(corrected, stream, indent=2, ensure_ascii=False)
    proof['correctedFrozenSelectionSha256'] = digest(HERE / 'freeze.json')
    with (HERE / 'contract-correction.json').open('x') as stream:
        json.dump(proof, stream, indent=2, ensure_ascii=False)
    print(json.dumps({k:v for k,v in proof.items() if k not in ['sourceSpans','discoveryEvidence']}, indent=2))
