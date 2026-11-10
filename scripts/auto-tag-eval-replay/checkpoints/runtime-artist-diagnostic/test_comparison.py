"""Synthetic negative controls for the local receipt-only paired comparator."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('compare', Path(__file__).with_name('compare.py'))
compare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compare)


class Controls(unittest.TestCase):
    def setUp(self):
        self.body = {'id': 7, 'artists': [{'id': 8, 'name': 'Canonical', 'anv': 'Credited'}]}
        body = json.dumps(self.body)
        self.sources = {'7': (self.body, hashlib.sha256(body.encode()).hexdigest())}
        self.before = dict(source='discogs', discogs_release_id='7', genre='Pop', genreEligible=False,
                           artist_aliases=[], track_number=3, writableDelta=[], acceptedCertificate=False)
        self.receipt = dict(resourceId='7', resourceKind='release', requestMethod='GET',
            requestPath='/releases/7', requestUrl='http://127.0.0.1:123/discogs/releases/7',
            responseUrl='http://127.0.0.1:123/discogs/releases/7', responseStatus=200,
            responseSha256=self.sources['7'][1], responseBody=body)

    def test_verified_receipt_only_is_excluded(self):
        observed = []
        self.assertEqual(compare.without_receipts(dict(self.before, discogsReleaseReceipt=self.receipt), self.sources, observed), self.before)
        self.assertEqual(observed[0]['release'], '7')

    def test_mismatched_or_unscoped_origin_is_rejected(self):
        for key, value in [('responseBody', '{}'), ('responseSha256', 'bad'), ('resourceId', '8'),
                           ('resourceKind', 'master'), ('requestPath', '/masters/7'),
                           ('requestMethod', 'POST'), ('responseStatus', 201),
                           ('responseUrl', 'http://127.0.0.1:123/discogs/releases/8'),
                           ('requestUrl', 'http://user:secret@127.0.0.1:123/discogs/releases/7')]:
            with self.subTest(key=key):
                changed = dict(self.receipt, **{key: value})
                with self.assertRaises((AssertionError, KeyError)):
                    compare.without_receipts(dict(self.before, discogsReleaseReceipt=changed), self.sources, [])

    def test_no_authority_or_metadata_exclusions(self):
        for key, value in [('genre', 'Rock'), ('genreEligible', True), ('artist_aliases', ['Credited']),
                           ('track_number', 1), ('writableDelta', [{'genre': 'Rock'}]), ('acceptedCertificate', True)]:
            with self.subTest(key=key):
                changed = dict(self.before, discogsReleaseReceipt=self.receipt, **{key: value})
                self.assertNotEqual(compare.without_receipts(changed, self.sources, []), self.before)

    def test_native_reasons_and_aggregate_support_cannot_be_hidden_as_diagnostics(self):
        row = dict(traceIndex=0, candidateIndex=0, nativeStrictError='artist conflict', nativeMappedVersions=[],
                   identity=dict(outcome='insufficient', correspondence='insufficient', explicitDecisionVisibility='unavailable'))
        snapshot = dict(nativeRouteTraces=[dict(candidates=[dict(strictError='artist conflict', versions=[])])],
                        candidateIdentityDiagnostics=[row])
        compare.check_diagnostics(copy.deepcopy(snapshot), 'folder_filename')
        for changed in [dict(row, nativeStrictError=None), dict(row, identity=dict(row['identity'], outcome='supported'))]:
            with self.assertRaises(AssertionError):
                compare.check_diagnostics(dict(snapshot, candidateIdentityDiagnostics=[changed]), 'folder_filename')


if __name__ == '__main__':
    unittest.main()
