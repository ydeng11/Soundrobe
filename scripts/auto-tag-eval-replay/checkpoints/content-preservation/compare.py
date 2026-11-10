"""Compare the complete common evidence contract, retaining every difference."""
import ast
import collections
import copy
import json
import pathlib
import re
import sys

HERE = pathlib.Path(__file__).resolve().parent
PRIOR = HERE.parent / 'reviewed-51-provider-first-20261109'
load = lambda path: json.loads(path.read_text())
helper_path = HERE.parent / 'performer-evidence-export-20261109/compare.py'
tree = ast.parse(helper_path.read_text())
helpers = {}
exec(compile(ast.Module(body=[n for n in tree.body if isinstance(n, (ast.Import, ast.ImportFrom, ast.FunctionDef))],
                        type_ignores=[]), str(helper_path), 'exec'), helpers)
stable, differences, runtime = (helpers[n] for n in ['stable', 'differences', 'disposable_runtime_fact'])
FIELDS = dict(artist='artist', artists='artists', track_number='trackNumber', track_total='trackTotal',
              disc_number='discNumber', disc_total='discTotal')


def value(record, path):
    for part in path.strip('/').split('/'):
        record = record[int(part)] if isinstance(record, list) else record[part]
    return record


def preserved(local, field, observed, profile):
    return (profile != 'folder_filename' and type(observed) is type(local.get(field)) and observed == local.get(field)
            and (field in ['artist', 'artists'] and bool(local.get('artist')) and bool(local.get('artists'))
                 or type(local.get(field)) is int and local[field] > 0))


def compare_record(before, after):
    original = after['beforeTracks']
    assert stable(before['beforeTracks']) == stable(original), 'actual prewrite inputs changed'
    assert len(original) == len(after['readbackTracks']) == len(before['readbackTracks'])
    aligned = copy.deepcopy(after)
    preserved_fields, receipts = {}, []
    for index, (old, new, local) in enumerate(zip(before['readbackTracks'], after['readbackTracks'], original)):
        assert stable(old['path']) == stable(new['path']) == stable(local['path'])
        preserved_fields[index] = {field for field in FIELDS.values() if old[field] != new[field]
                                  and preserved(local, field, new[field], after['profile'])}
    for area in ['native', 'persistedEvidence']:
        retained = []
        for receipt in aligned[area]['diagnostics']:
            index = receipt.get('localIndex')
            expected = dict(stage='local_track_preservation', localIndex=index,
                            filename=pathlib.Path(original[index]['path']).stem if type(index) is int and 0 <= index < len(original) else None,
                            fields=['artist', 'artists'], scope='existing collaborator presentation; no new performer authority')
            if (receipt == expected and after['profile'] != 'folder_filename'
                    and all(after['readbackTracks'][index][f] == original[index][f] for f in ['artist', 'artists'])):
                receipts.append(dict(path=f'/{area}/diagnostics', classification='prewrite_bound_preservation_receipt', after=receipt))
            else:
                retained.append(receipt)
        aligned[area]['diagnostics'] = retained
    a, b = stable(before), stable(aligned)
    rows = receipts
    for row in differences(a, b):
        path, reason = row['path'], None
        if runtime(row, before, aligned):
            reason = 'bounded_disposable_runtime_fact'
        elif path == '/runId' and row['before'] == f'{PRIOR.name}-{after["profile"]}-{after["caseId"]}' and row['after'] == f'{HERE.name}-{after["profile"]}-{after["caseId"]}':
            reason = 'new_run_identity'
        elif re.fullmatch(r'/(?:native|persistedEvidence)/diagnostics/\d+/positionPolicy', path) and row.get('before') == 'local_file_order' and row.get('after') == 'captured_tags_else_local_file_order':
            reason = 'position_policy_label'
        match = re.fullmatch(r'/readbackTracks/(\d+)/(artist|artists|trackNumber|trackTotal|discNumber|discTotal)(?:/\d+)?', path)
        if match and match[2] in preserved_fields[int(match[1])]:
            reason = 'captured_field_readback'
        match = re.fullmatch(r'/(?:native|persistedEvidence)/candidate/tracks/(\d+)/(artist|artists|track_number|track_total|disc_number|disc_total)(?:/\d+)?', path)
        if match:
            index, field = int(match[1]), FIELDS[match[2]]
            base = path.rsplit('/', 1)[0] if path.rsplit('/', 1)[1].isdigit() else path
            if preserved(original[index], field, value(b, base), after['profile']):
                reason = 'captured_candidate_field'
        match = re.fullmatch(r'/after/(.+)/fullSha256', path)
        if match:
            affected = {pathlib.Path(original[i]['path']).name for i, fields in preserved_fields.items() if fields}
            if match[1] in affected and before['after'][match[1]]['payloadSha256'] == after['after'][match[1]]['payloadSha256']:
                reason = 'tag_hash_consequence_enumerated_reader_fields_only'
        if (path == '/classification' and any(preserved_fields.values())
                and before['outcome'] == after['outcome'] == 'applied'
                and before['native']['written'] == after['native']['written']):
            reason = 'legacy_classification_consequence_not_safety_authority'
        rows.append(dict(**row, classification=reason or 'unexplained'))
    return rows


def main():
    selection, old_selection = load(HERE / 'selection.json'), load(PRIOR / 'selection.json')
    assert selection['profiles'] == old_selection['profiles']
    for new, old in zip(selection['cases'], old_selection['cases']):
        assert {k: v for k, v in new.items() if k != 'expectations'} == {k: v for k, v in old.items() if k != 'expectations'}
        assert pathlib.Path(new['expectations']).read_bytes() == pathlib.Path(old['expectations']).read_bytes()
    assert (HERE / 'providers/manifest.json').read_bytes() == (PRIOR / 'providers/manifest.json').read_bytes()
    all_changes, rows, request_changes, totals = [], [], [], collections.Counter()
    partial = '--partial' in sys.argv
    for case in selection['cases']:
        for profile in selection['profiles']:
            relative = pathlib.Path(profile) / case['caseId']
            old, new = PRIOR / relative, HERE / relative
            if partial and not (new / 'execution.json').exists():
                continue
            x, y = load(old / 'provider-inventory-final.json'), load(new / 'provider-inventory-final.json')
            if x != y:
                request_changes.append(dict(caseId=case['caseId'], profile=profile, before=x, after=y))
            totals['providerRequestsBefore'] += len(x['requests'])
            totals['providerRequestsAfter'] += len(y['requests'])
            for phase in ['cold', 'warm']:
                before, after = load(old / f'{phase}.jsonl'), load(new / f'{phase}.jsonl')
                changes = compare_record(before, after)
                context = dict(caseId=case['caseId'], profile=profile, phase=phase)
                all_changes.extend(dict(**context, **row) for row in changes)
                rows.append(dict(**context, beforeOutcome=before['outcome'], afterOutcome=after['outcome'],
                                 beforeReason=(before.get('native') or {}).get('reasonCode'), afterReason=(after.get('native') or {}).get('reasonCode'),
                                 differences=len(changes), unexplained=sum(r['classification'] == 'unexplained' for r in changes)))
    result = dict(evaluations=len(rows), partial=partial, totals=dict(totals),
                  classes=dict(collections.Counter(r['classification'] for r in all_changes)),
                  requestChanges=request_changes, changes=all_changes, cases=rows,
                  normalization='disposable UUID path prefix and root elapsedMs only; key/type-preserving comparison; run IDs and allocation clocks retained with bounded classifications',
                  limits='No new write authority; classification changes are legacy-oracle consequences, not correctness. Full-file hash consequences do not independently cover unexported tags.')
    with (HERE / ('comparison-partial.json' if partial else 'comparison.json')).open('x') as stream:
        json.dump(result, stream, indent=2)
    print(json.dumps({k: v for k, v in result.items() if k not in ['changes', 'cases', 'requestChanges']}, indent=2))
    print('Changed ordered request jobs:', len(request_changes), flush=True)
    return bool(result['classes'].get('unexplained') or request_changes)


if __name__ == '__main__':
    sys.exit(main())
