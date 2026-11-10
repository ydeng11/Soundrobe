"""Read-only source/input and unchanged-certificate audit of this local checkpoint."""
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[4]
BASE = '1c74c6c728f0a8ad87dc0a66de08dfd3fdabf5bd'
spec = importlib.util.spec_from_file_location('comparison', Path(__file__).with_name('compare.py'))
comparison = importlib.util.module_from_spec(spec)
spec.loader.exec_module(comparison)
load, digest = comparison.load, comparison.digest
EVIDENCE = ROOT / '.amp/in/artifacts/runtime-artist-diagnostic-20261110'


def main(output):
    subprocess.run(['git', 'merge-base', '--is-ancestor', BASE, 'HEAD'], cwd=ROOT, check=True)
    assert subprocess.check_output(['git', 'branch', '--show-current'], cwd=ROOT, text=True).strip() == 'artist-library-inventory'
    incoming = load(EVIDENCE / 'incoming-lock.json')
    changed = []
    for path, expected in incoming['files'].items():
        assert (ROOT / path).is_file(), path
        if digest(ROOT / path) != expected['sha256']:
            changed.append(path)
    assert set(changed) == comparison.CHANGED | {'docs/CHANGELOG.md'}, changed
    plan = load(EVIDENCE / 'final/replay/execution-plan.json')
    for path, expected in plan['locks'].items():
        assert digest(ROOT / path) == expected, path
    assert digest(plan['binary']) == plan['binarySha256']
    source = 'src-tauri/src/commands/auto_tag.rs'
    before = subprocess.check_output(['git', 'show', BASE + ':' + source], cwd=ROOT, text=True)
    after = (ROOT / source).read_text()
    pattern = r'(?m)^(?:pub(?:\([^)]*\))? )?(?:async )?fn (\w+)\('
    def section(text, name):
        starts = list(re.finditer(pattern, text))
        index = next(i for i, match in enumerate(starts) if match[1] == name)
        return text[starts[index].start():starts[index + 1].start() if index + 1 < len(starts) else len(text)]
    guards = ['build_lookup_request', 'fill_request_artist_identity', 'fill_candidate_artist_identity',
              'exact_artist_identity', 'strict_provider_evaluation', 'complete_provider_genre',
              'provider_candidate_credibility']
    for name in guards:
        assert section(before, name) == section(after, name), name
    spec = importlib.util.spec_from_file_location('fields', ROOT / 'scripts/auto-tag-eval-replay/checkpoints/content-preservation/field-audit.py')
    fields = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fields)
    certificate_root = ROOT / '.amp/in/artifacts/reviewed-51-content-preservation-20261110'
    certificate_file = certificate_root / 'source-certificates.json'
    certificate = load(certificate_file)[comparison.CASE]
    certified = {}
    for receipt in certificate['eligibleSources']:
        path = certificate_root / receipt['bodyFile']
        assert digest(path) == receipt['sha256']
        certified[(receipt['provider'], receipt['releaseId'])] = load(path)
    rows = []
    for arm in [ROOT / '.amp/in/artifacts/primary-artist-retention-corrections-20261110/replay', EVIDENCE / 'final/replay']:
        for profile in comparison.PROFILES:
            for phase in ['cold', 'warm']:
                path = arm / 'runs' / f'{profile}-{comparison.CASE}' / f'{phase}.jsonl'
                codes, _ = fields.audit_record(load(path), certified)
                codes = sorted(set(codes))
                assert codes == ([] if profile == 'tagged_recovery' else ['source_eligibility'])
                rows.append(dict(arm=str(arm.relative_to(ROOT)), profile=profile, phase=phase, findings=codes))
    result = dict(incomingFilesChecked=len(incoming['files']), authorizedChanges=sorted(changed),
                  unchangedGuardSections=guards, executionLocks=len(plan['locks']), binarySha256=plan['binarySha256'],
                  sourceCertificateSha256=digest(certificate_file), findingsPerArm=4, rows=rows,
                  overallSafetyClean=False, historical84FindingsReclassified=False,
                  limitation='Local sealed inputs required; source comparisons are not semantic/acoustic certification')
    with Path(output).open('x') as stream:
        json.dump(result, stream, indent=2)
        stream.write('\n')
    print('Verified incoming files, execution locks, guard sections and unchanged non-clean source findings')


if __name__ == '__main__':
    main(*sys.argv[1:])
