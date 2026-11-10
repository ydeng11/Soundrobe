"""Versioned comparator correction; preserve original comparator and red evidence."""
import importlib.util
import pathlib
import re

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('original_comparison', HERE / 'compare.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)


def runtime(row, before, after):
    match = re.fullmatch(r'(/(?:native|persistedEvidence)/diagnostics/\d+/diagnostics/observations/\d+/observation/source)/(?:observedAt|heldFileIdentity/(?:inode|ctimeSeconds|ctimeNanoseconds|mtimeSeconds|mtimeNanoseconds))', row['path'])
    if not match:
        return False
    source_path = match[1]
    diagnostic_path = source_path.split('/diagnostics/observations/')[0]
    batch_path = source_path.removesuffix('/observation/source')
    for record in [before, after]:
        if base.value(record, diagnostic_path)['stage'] != 'exact_audio_groups':
            return False
        batch = base.value(record, batch_path)
        observation = batch['observation']
        if (batch['sameInodeAlias'] is not False or observation['status'] != 'complete'
                or observation['error'] is not None or observation['source']['finalizationError'] is not None):
            return False
        source = observation['source']
        if source['observedAt'] < max(source['heldFileIdentity']['ctimeSeconds'], source['heldFileIdentity']['mtimeSeconds']):
            return False
        native_path = source_path.replace('/persistedEvidence/', '/native/', 1)
        persisted_path = source_path.replace('/native/', '/persistedEvidence/', 1)
        # Bind the exact allocation observation, not unrelated PCM floats.
        # Every PCM/provenance leaf still participates in the outer strict diff.
        if base.differences(base.value(record, native_path), base.value(record, persisted_path)):
            return False
    a, b = base.value(before, source_path), base.value(after, source_path)
    return (a['path'] != b['path'] and base.stable(a['path']) == base.stable(b['path'])
            and base.stable(a['path']).startswith('$DISPOSABLE_ROOT/')
            and a['stable'] is True and b['stable'] is True
            and len({a['expectedSha256'], a['beforeSha256'], a['afterSha256'],
                     b['expectedSha256'], b['beforeSha256'], b['afterSha256']}) == 1
            and re.fullmatch('[a-f0-9]{64}', a['expectedSha256']) is not None
            and a['bytes'] == b['bytes']
            and a['heldFileIdentity']['device'] == b['heldFileIdentity']['device']
            and all(type(row[k]) is int and row[k] >= 0 for k in ['before', 'after']))


def compare_record(before, after):
    base.runtime = runtime
    return base.compare_record(before, after)


if __name__ == '__main__':
    base.runtime = runtime
    raise SystemExit(base.main())
