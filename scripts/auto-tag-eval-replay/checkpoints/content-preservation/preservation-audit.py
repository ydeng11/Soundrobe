"""Independent prewrite-based preservation checks; no application matcher imports."""
import collections
import json
import pathlib
import sys
import unicodedata

HERE = pathlib.Path(__file__).resolve().parent
load = lambda path: json.loads(path.read_text())
key = lambda value: ''.join(c for c in unicodedata.normalize('NFC', value or '').casefold() if c.isalnum())
NUMBERS = ['trackNumber', 'trackTotal', 'discNumber', 'discTotal']


def numbering_findings(before, after):
    findings = []
    if len(before) != len(after):
        return [dict(category='physical_binding', detail='track count')]
    for index, (local, actual) in enumerate(zip(before, after)):
        if pathlib.Path(local['path']).name != pathlib.Path(actual['path']).name:
            findings.append(dict(category='physical_binding', index=index))
        for field in NUMBERS:
            captured = local.get(field)
            expected = captured if type(captured) is int and captured > 0 else (
                index + 1 if field == 'trackNumber' else len(before) if field == 'trackTotal' else None)
            if expected is not None and (type(actual.get(field)) is not int or actual[field] != expected):
                findings.append(dict(category='captured_numbering' if captured else 'missing_numbering_fallback',
                                     index=index, file=pathlib.Path(local['path']).name,
                                     field=field, captured=captured, expected=expected, actual=actual.get(field)))
    return findings


def credit_findings(local, actual):
    # Exact explicit entities only. Composite single-string lists and aliases
    # are not independently re-parsed or certified by this audit.
    original = {key(name) for name in local.get('artists', [])}
    completed = {key(name) for name in actual.get('artists', [])}
    if len(original) < 2 or '' in original:
        return []
    if completed <= original:
        return [] if all(local.get(f) == actual.get(f) for f in ['artist', 'artists']) else ['collaborator_presentation']
    return [] if original <= completed else ['incompatible_guests']


def main():
    selection = load(HERE / 'selection.json')
    findings, rows, numbers, receipts, raw_redactions = [], [], collections.Counter(), 0, 0
    for case in selection['cases']:
        for profile in selection['profiles']:
            directory = HERE / profile / case['caseId']
            for phase in ['cold', 'warm']:
                context = dict(caseId=case['caseId'], profile=profile, phase=phase)
                record = load(directory / f'{phase}.jsonl')
                snapshot = load(directory / f'evidence-{phase}-{case["caseId"]}.json')
                before, after = record['beforeTracks'], record['readbackTracks']
                codes = []
                if snapshot['baselineTracks'] != before:
                    codes.append(dict(category='prewrite_track_binding'))
                if len(snapshot['positions']) != len(before) or len(snapshot['request']['tracks']) != len(before):
                    codes.append(dict(category='prewrite_request_binding'))
                for index, (local, position, request) in enumerate(zip(before, snapshot['positions'], snapshot['request']['tracks'])):
                    if position['tagged'] != local['trackNumber'] or position['lookupPosition'] != request['track_number']:
                        codes.append(dict(category='position_origin_binding', index=index))
                    if request['filename'] != pathlib.Path(local['path']).stem:
                        codes.append(dict(category='request_physical_binding', index=index))
                    for field in NUMBERS:
                        numbers[field + ('Captured' if type(local.get(field)) is int and local[field] > 0 else 'Missing')] += 1
                    if profile == 'folder_filename' and any(local.get(f) for f in NUMBERS + ['musicbrainzAlbumId', 'musicbrainzTrackId', 'discogsReleaseId']):
                        codes.append(dict(category='stripped_profile_leakage', index=index))
                if not record['payloadUnchanged'] or not record['readback']:
                    codes.append(dict(category='native_integrity'))
                for name, hashes in snapshot['baselineHashes'].items():
                    visible = record['profileBaseline'][name]
                    if visible == '[redacted]':
                        raw_redactions += 1
                    elif hashes != visible:
                        codes.append(dict(category='prewrite_hash_binding', file=name))
                native = record.get('native') or {}
                if native.get('written'):
                    codes.extend(numbering_findings(before, after))
                    for index, (local, actual) in enumerate(zip(before, after)):
                        codes.extend(dict(category=code, index=index, file=pathlib.Path(local['path']).name)
                                     for code in credit_findings(local, actual))
                    for receipt in native['diagnostics']:
                        if receipt.get('stage') != 'local_track_preservation':
                            continue
                        receipts += 1
                        index = receipt['localIndex']
                        if any(before[index][field] != after[index][field] for field in receipt['fields']):
                            codes.append(dict(category='preservation_receipt_readback', index=index))
                        if receipt not in record['persistedEvidence']['diagnostics']:
                            codes.append(dict(category='preservation_receipt_not_persisted', index=index))
                elif before != after or record['after'] != record['profileBaseline']:
                    codes.append(dict(category='held_mutation'))
                findings.extend(dict(**context, **code) for code in codes)
                rows.append(dict(**context, written=native.get('written', 0),
                                 reasonCode=native.get('reasonCode'), findings=codes))
    result = dict(evaluations=len(rows), uniquePhysicalInputs=539, repeatedFileEvaluations=3234,
                  clean=not findings, findings=findings, totals=dict(collections.Counter(f['category'] for f in findings)),
                  numericObservations=dict(numbers), preservationReceipts=receipts,
                  redactedBaselineHashEntries=raw_redactions, cases=rows,
                  limits='Explicit multi-entry performer lists only; composite/alias consistency and incompatible-selected assertions use fresh native controls. No independent proof for every unexported tag. Matrix has no nonblank local genres or explicit cleanup inputs.')
    with (HERE / 'preservation-verification.json').open('x') as stream:
        json.dump(result, stream, indent=2)
    print(json.dumps({k: v for k, v in result.items() if k not in ['cases', 'findings']}, indent=2))
    return bool(findings)


if __name__ == '__main__':
    sys.exit(main())
