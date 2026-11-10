"""Extend receipt locations, but only for exact file-bound prewrite preservation."""
import copy
import importlib.util
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('comparison_v2', HERE / 'compare-v2.py')
v2 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v2)
base = v2.base
original = base.compare_record


def compare_record(before, after):
    aligned, receipts = copy.deepcopy(after), []
    for area in ['native', 'persistedEvidence']:
        for diagnostic_index, diagnostic in enumerate(aligned[area]['diagnostics']):
            if 'fieldEvidence' not in diagnostic:
                continue
            retained = []
            for index, receipt in enumerate(diagnostic['fieldEvidence']):
                local_index = receipt.get('localIndex')
                local = (after['beforeTracks'][local_index] if type(local_index) is int
                         and 0 <= local_index < len(after['beforeTracks']) else None)
                expected = dict(stage='local_track_preservation', localIndex=local_index,
                                filename=pathlib.Path(local['path']).stem if local else None,
                                fields=['artist', 'artists'], scope='existing collaborator presentation; no new performer authority')
                if (local and receipt == expected and after['profile'] != 'folder_filename'
                        and all(after['readbackTracks'][local_index][f] == local[f] for f in ['artist', 'artists'])):
                    receipts.append(dict(path=f'/{area}/diagnostics/{diagnostic_index}/fieldEvidence/{index}',
                                         classification='prewrite_bound_preservation_receipt', after=receipt))
                else:
                    retained.append(receipt)
            diagnostic['fieldEvidence'] = retained
    base.runtime = v2.runtime
    return receipts + original(before, aligned)


if __name__ == '__main__':
    base.compare_record = compare_record
    raise SystemExit(base.main())
