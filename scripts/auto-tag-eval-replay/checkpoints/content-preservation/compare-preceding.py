"""Compare against the preservation matrix, retaining identical old receipts."""
import copy
import importlib.util
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('comparison_v3', HERE / 'compare-v3.py')
v3 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v3)
base = v3.base


def compare_record(before, after):
    aligned = copy.deepcopy(before)
    for area in ['native', 'persistedEvidence']:
        valid = []
        for diagnostic in after[area]['diagnostics']:
            for receipt in [diagnostic, *diagnostic.get('fieldEvidence', [])]:
                index = receipt.get('localIndex')
                local = (after['beforeTracks'][index] if type(index) is int
                         and 0 <= index < len(after['beforeTracks']) else None)
                expected = dict(stage='local_track_preservation', localIndex=index,
                                filename=pathlib.Path(local['path']).stem if local else None,
                                fields=['artist', 'artists'],
                                scope='existing collaborator presentation; no new performer authority')
                if (local and receipt == expected and after['profile'] != 'folder_filename'
                        and all(after['readbackTracks'][index][f] == local[f] for f in ['artist', 'artists'])):
                    valid.append(receipt)
        # V3 removes validated after-receipts. Remove only byte-equivalent
        # before-receipts too, so an unchanged receipt is not a false deletion.
        retained = []
        for diagnostic in aligned[area]['diagnostics']:
            if diagnostic in valid:
                continue
            if 'fieldEvidence' in diagnostic:
                diagnostic['fieldEvidence'] = [r for r in diagnostic['fieldEvidence'] if r not in valid]
            retained.append(diagnostic)
        aligned[area]['diagnostics'] = retained
    return v3.compare_record(aligned, after)


if __name__ == '__main__':
    target = HERE / 'preceding-comparison.json'
    assert not target.exists()
    base.PRIOR = HERE.parent / 'reviewed-51-preservation-20261110'
    base.compare_record = compare_record
    result = base.main()
    (HERE / 'comparison.json').rename(target)
    sys.exit(result)
