"""Source-derived progress/provenance checks and exact coverage breakdown."""
import collections
import csv
import json
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]
load = lambda p: json.loads(p.read_text())
frozen = load(HERE / 'freeze.json')
verification = load(HERE / 'verification.json')
jobs = {j['name']:j for j in frozen['jobs']}
findings, rows = [], []
redactions = 0
for row in verification['cases']:
    job = jobs[row['name']]
    record = load(HERE / 'runs' / row['name'] / f'{row["phase"]}.jsonl')
    native = record.get('native') or {}
    persisted = record.get('persistedEvidence') or {}
    expected_ai = 'genre_inferred' if job['expected']=='inferred_unverified' else None
    errors = []
    if job['kind']!='hold':
        if native.get('aiStatus') != expected_ai or persisted.get('aiStatus') != expected_ai:
            errors.append('ai_status_provenance')
    if job['kind']=='gap' and job['scenario'] in ['rejected','invalid','failed']:
        warnings = [e for e in record['reportEvents'] if e['kind']=='warning' and (e.get('data') or {}).get('source')=='llm']
        if len(warnings)!=1 or not warnings[0]['message'].startswith('Genre remains missing:'):
            errors.append('missing_genre_warning')
        if job['scenario']=='failed':
            diagnostic = dict(stage='genre', reason='AI provider returned HTTP 400')
            if diagnostic not in native['diagnostics'] or diagnostic not in persisted['diagnostics']:
                errors.append('persistent_terminal_failure')
        elif warnings:
            expected = dict(source='llm', genre='Offline test genre' if job['scenario']=='rejected' else None,
                            confidence=0.2 if job['scenario']=='rejected' else None, threshold=0.6)
            if warnings[0]['data'] != expected:
                errors.append('rejection_contract')
    if job['kind'] in ['provider','all_local'] or job['scenario']=='disabled':
        if any((e.get('data') or {}).get('source')=='llm' for e in record['reportEvents']):
            errors.append('unnecessary_assistance_report')
    redactions += sum(value=='[redacted]' for value in record['after'].values())
    genres = collections.Counter(g['provenance'] for g in row['genres'])
    output = {k:row[k] for k in ['name','caseId','profile','phase','kind','scenario','outcome','reasonCode','written','modelRequests','nativeReviewPending']}
    output.update(files=len(row['genres']), genres=dict(genres), findings=errors,
                  contractFindings=row['findings'], unchangedFieldAuditFindings=row['legacyDisabledOnlyFieldFindings'])
    rows.append(output)
    findings.extend(dict(name=row['name'], phase=row['phase'], category=code) for code in errors)
groups = []
for key in sorted({(r['kind'],r['scenario'],r['profile'],r['phase']) for r in rows}):
    selected = [r for r in rows if (r['kind'],r['scenario'],r['profile'],r['phase'])==key]
    genres = collections.Counter()
    for r in selected:
        genres.update(r['genres'])
    groups.append(dict(zip(['kind','scenario','profile','phase'],key), evaluations=len(selected),
                       repeatedFiles=sum(r['files'] for r in selected), writes=sum(r['written'] for r in selected),
                       modelRequests=sum(r['modelRequests'] for r in selected), genres=dict(genres),
                       outcomes=dict(collections.Counter(r['outcome'] for r in selected))))
with (HERE / 'case-profile-phase.csv').open('x') as stream:
    writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
    writer.writeheader()
    for row in rows:
        writer.writerow({k:json.dumps(v,ensure_ascii=False) if isinstance(v,(dict,list)) else v for k,v in row.items()})
result = dict(evaluations=len(rows), sourceContractFindings=findings, groups=groups, rows=rows,
               overallOutcomes=dict(collections.Counter(r['outcome'] for r in rows)),
               totalWrittenFiles=sum(r['written'] for r in rows),
               unchangedFieldAuditTotals=dict(collections.Counter(c for r in rows for c in r['unchangedFieldAuditFindings'])),
               redactedIndependentPayloadExports=redactions, groundedAssistanceAccuracyCredit=0,
               scope='Repeated controlled offline requests; no new provider certificate or semantic ground truth',
               additionalLimits='Cleanup and duplicate/multi-disc boundaries use fresh existing native controls, not newly injected into this gap matrix. Component stub Keep is not packaged integration.')
with (HERE / 'supplement.json').open('x') as stream:
    json.dump(result, stream, indent=2, ensure_ascii=False)
print(json.dumps({k:v for k,v in result.items() if k not in ['rows','groups']}, indent=2), flush=True)
raise SystemExit(bool(findings))
