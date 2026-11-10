"""Independently check preservation receipts at both maintained locations."""
import json
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
load = lambda path: json.loads(path.read_text())


def receipts(diagnostics):
    return [r for item in diagnostics for r in [item, *item.get('fieldEvidence', [])]
            if r.get('stage') == 'local_track_preservation']


def audit_record(record):
    findings = []
    stored = receipts(record['persistedEvidence']['diagnostics'])
    for receipt in receipts(record['native']['diagnostics']):
        index = receipt.get('localIndex')
        if type(index) is not int or not 0 <= index < len(record['beforeTracks']):
            findings.append(dict(category='receipt_binding'))
            continue
        local, actual = record['beforeTracks'][index], record['readbackTracks'][index]
        if receipt.get('filename') != pathlib.Path(local['path']).stem:
            findings.append(dict(category='receipt_binding', index=index))
        fields = [field for field in receipt['fields'] if local[field] != actual[field]]
        if fields:
            findings.append(dict(category='receipt_prewrite_not_preserved', index=index,
                                 file=pathlib.Path(local['path']).name, fields=fields,
                                 prewrite={f: local[f] for f in fields}, readback={f: actual[f] for f in fields}))
        if receipt not in stored:
            findings.append(dict(category='receipt_not_persisted', index=index))
    return findings


if __name__ == '__main__':
    rows, findings, observed = [], [], 0
    selection = load(HERE / 'selection.json')
    for case in selection['cases']:
        for profile in selection['profiles']:
            for phase in ['cold', 'warm']:
                context = dict(caseId=case['caseId'], profile=profile, phase=phase)
                record = load(HERE / profile / case['caseId'] / f'{phase}.jsonl')
                observed += len(receipts(record['native']['diagnostics']))
                codes = audit_record(record)
                findings.extend(dict(**context, **code) for code in codes)
                rows.append(dict(**context, findings=codes))
    result = dict(evaluations=len(rows), nativeReceipts=observed, clean=not findings,
                  findings=findings, cases=rows,
                  limits='Checks receipt promises against actual raw prewrite values; no name parser, new authority or inferred user locks')
    with (HERE / 'receipt-verification.json').open('x') as stream:
        json.dump(result, stream, indent=2)
    print(json.dumps({k: v for k, v in result.items() if k != 'cases'}, indent=2))
    raise SystemExit(bool(findings))
