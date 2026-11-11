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

    def diagnostic_snapshot(self, supported=True):
        # Synthetic eleven-file observations, not catalog or file-history proof.
        tracks = [dict(path=f'/synthetic/{i:02}.flac', albumArtist='盧廣仲' if supported else None,
                       albumArtists=['盧廣仲'] if supported else [], discogsArtistId=None,
                       musicbrainzArtistId='separate-provider-id') for i in range(11)]
        claims = [dict(physicalPath=t['path'], albumArtist=t['albumArtist'], albumArtists=t['albumArtists'],
                       unscopedDiscogsArtistId=t['discogsArtistId'], separateMusicbrainzArtistId=t['musicbrainzArtistId']) for t in tracks]
        fields = [dict(path=t['path'], sha256=f'synthetic-{i}',
                       fields=[['ALBUMARTIST', '盧廣仲'], ['ALBUMARTISTS', '盧廣仲']] if supported else [])
                  for i, t in enumerate(tracks)]
        candidates = [dict(candidate=dict(source='musicbrainz'), strictError=None, versions=[])]
        for release in ['37231524', '37230261']:
            candidates.append(dict(candidate=dict(source='discogs', discogsReleaseReceipt=dict(
                requestPath=f'/releases/{release}', resourceId=release, responseSha256=f'synthetic-{release}')),
                strictError='artist conflict', versions=[]))
        traces = [dict(field='credits', candidates=candidates), dict(field='genre', candidates=candidates[:2])]
        rows = []
        for ti, trace in enumerate(traces):
            for ci, source in enumerate(trace['candidates']):
                if source['candidate']['source'] != 'discogs':
                    continue
                receipt = source['candidate']['discogsReleaseReceipt']
                identity = dict(outcome='insufficient', correspondence='supported' if supported else 'insufficient',
                    reason='explicit_decisions_unavailable' if supported else 'original_primary_scope_unavailable',
                    explicitDecisionVisibility='unavailable', originalPrimaryClaims=copy.deepcopy(claims),
                    originalPrimaryFields=copy.deepcopy(fields), origin=dict(receipt, selector='/artists/0'),
                    scope='candidate_single_primary_name_only_not_field_eligibility')
                rows.append(dict(traceIndex=ti, candidateIndex=ci, field=trace['field'], identity=identity,
                    nativeStrictError=source['strictError'], nativeMappedVersions=source['versions'],
                    scope='reported native route only; artist rejection does not execute or pass later predicates; no identity bypass'))
        return dict(nativeRouteTraces=traces, candidateIdentityDiagnostics=rows, baselineTracks=tracks,
                    baselineHashes={Path(t['path']).name: dict(fullSha256=fields[i]['sha256']) for i, t in enumerate(tracks)})

    def test_native_reasons_and_aggregate_support_cannot_be_hidden_as_diagnostics(self):
        snapshot = self.diagnostic_snapshot(supported=False)
        compare.check_diagnostics(copy.deepcopy(snapshot), 'folder_filename')
        for field in ['nativeStrictError', 'outcome']:
            changed = copy.deepcopy(snapshot)
            row = changed['candidateIdentityDiagnostics'][0]
            if field == 'nativeStrictError':
                row[field] = None
            else:
                row['identity'][field] = 'supported'
            with self.assertRaises(AssertionError):
                compare.check_diagnostics(changed, 'folder_filename')

    def test_complete_original_claims_are_bound_before_exclusion(self):
        for supported in [True, False]:
            for field, value in [('albumArtist', 'Other'), ('albumArtists', ['盧廣仲', 'Guest']),
                                 ('unscopedDiscogsArtistId', '2461941'), ('separateMusicbrainzArtistId', 'other-id')]:
                with self.subTest(supported=supported, field=field):
                    snapshot = self.diagnostic_snapshot(supported)
                    snapshot['candidateIdentityDiagnostics'][0]['identity']['originalPrimaryClaims'][0][field] = value
                    before = copy.deepcopy(snapshot)
                    with self.assertRaises(AssertionError):
                        compare.check_diagnostics(snapshot, 'assisted_without_ids' if supported else 'folder_filename')
                    self.assertEqual(snapshot, before, 'invalid evidence must not be discarded')

    def test_supported_primary_scope_rejects_joint_ids_and_duplicate_fields(self):
        for mutation in ['joint', 'unscoped_id', 'duplicate_singular', 'duplicate_plural', 'spaced_plural', 'missing_singular']:
            with self.subTest(mutation=mutation):
                snapshot = self.diagnostic_snapshot()
                identity = snapshot['candidateIdentityDiagnostics'][0]['identity']
                if mutation == 'joint':
                    snapshot['baselineTracks'][0]['albumArtists'] = ['盧廣仲', 'Guest']
                    for row in snapshot['candidateIdentityDiagnostics']:
                        row['identity']['originalPrimaryClaims'][0]['albumArtists'] = ['盧廣仲', 'Guest']
                elif mutation == 'unscoped_id':
                    snapshot['baselineTracks'][0]['discogsArtistId'] = '2461941'
                    for row in snapshot['candidateIdentityDiagnostics']:
                        row['identity']['originalPrimaryClaims'][0]['unscopedDiscogsArtistId'] = '2461941'
                else:
                    fields = identity['originalPrimaryFields'][0]['fields']
                    if mutation == 'missing_singular':
                        fields.pop(0)
                    else:
                        fields.append(['ALBUMARTIST' if mutation == 'duplicate_singular' else
                                       'ALBUM ARTISTS' if mutation == 'spaced_plural' else 'ALBUMARTISTS', '盧廣仲'])
                with self.assertRaises(AssertionError):
                    compare.check_diagnostics(snapshot, 'assisted_without_ids')

    def test_rows_have_exact_trace_coverage_and_attribution(self):
        for mutation in ['duplicate', 'omitted', 'field', 'scope', 'identity_scope', 'negative_index', 'boolean_index']:
            with self.subTest(mutation=mutation):
                snapshot = self.diagnostic_snapshot()
                rows = snapshot['candidateIdentityDiagnostics']
                if mutation == 'duplicate':
                    rows.append(copy.deepcopy(rows[0]))
                elif mutation == 'omitted':
                    rows.pop()
                elif mutation == 'identity_scope':
                    rows[0]['identity']['scope'] = 'field_eligibility'
                elif mutation.endswith('_index'):
                    rows[0]['candidateIndex'] = -2 if mutation == 'negative_index' else True
                else:
                    rows[0][mutation] = 'other'
                with self.assertRaises(AssertionError):
                    compare.check_diagnostics(snapshot, 'assisted_without_ids')

    def test_valid_diagnostics_exclude_only_the_validated_rows(self):
        snapshot = self.diagnostic_snapshot()
        expected = {k: copy.deepcopy(v) for k, v in snapshot.items() if k != 'candidateIdentityDiagnostics'}
        result = compare.check_diagnostics(snapshot, 'assisted_without_ids')
        self.assertEqual(snapshot, expected)
        self.assertEqual(result, dict(rows=3, supportedCorrespondenceReleases=['37230261', '37231524'],
                                      aggregate='insufficient', decisions='unavailable'))


if __name__ == '__main__':
    unittest.main()
