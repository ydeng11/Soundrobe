"""Negative controls for the evidence-only comparator, not native semantics."""
import copy
import gzip
import importlib.util
import json
import lzma
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('compare', Path(__file__).with_name('compare.py'))
compare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compare)


class ComparisonControls(unittest.TestCase):
    def test_reads_plain_and_retained_compressed_json_without_ignored_helpers(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'snapshot.json'
            for opener, suffix in [(open, ''), (gzip.open, '.gz'), (lzma.open, '.xz')]:
                stored = Path(str(path) + suffix)
                with opener(stored, 'wt') as stream:
                    json.dump({'value': 'retained'}, stream)
                self.assertEqual(compare.load(path), {'value': 'retained'})
                stored.unlink()

    def setUp(self):
        self.sources = {'7': {'id': 7, 'artists': [{'id': 8, 'name': 'Canonical', 'anv': 'Credited'}]}}
        self.pair = dict(releaseId='7', artistIndex=0, artistId='8', canonicalName='Canonical', creditedName='Credited')
        self.before = dict(source='discogs', discogs_release_id='7', genre='Pop', genreEligible=False,
                           artist_aliases=[], track_number=3, writableDelta=[], acceptedCertificate=False,
                           discogs_artist_name_variants=[self.pair])
        self.after = dict(self.before)
        self.after['discogsArtistNameVariants'] = self.after.pop('discogs_artist_name_variants')

    def test_allows_only_wire_spelling(self):
        a, b = [], []
        self.assertEqual(compare.normalize(self.before, self.sources, a), compare.normalize(self.after, self.sources, b))
        self.assertEqual(a[0]['pairs'], b[0]['pairs'])
        self.assertNotEqual(a[0]['wireKey'], b[0]['wireKey'])

    def test_rejects_changed_source_binding(self):
        for key, value in [('artistId', '9'), ('releaseId', '9'), ('creditedName', 'Guessed'), ('artistIndex', 1)]:
            with self.subTest(key=key):
                changed = copy.deepcopy(self.after)
                changed['discogsArtistNameVariants'][0][key] = value
                with self.assertRaises((AssertionError, KeyError, IndexError)):
                    compare.normalize(changed, self.sources, [])
        for key, value in [('source', 'musicbrainz'), ('discogs_release_id', '9')]:
            with self.assertRaises(AssertionError):
                compare.normalize(dict(self.after, **{key:value}), self.sources, [])

    def test_keeps_behavior_and_authority_differences(self):
        for key, value in [('genre', 'Rock'), ('genreEligible', True), ('artist_aliases', ['Credited']),
                           ('track_number', 1), ('writableDelta', [{'genre':'Rock'}]), ('acceptedCertificate', True)]:
            with self.subTest(key=key):
                self.assertNotEqual(compare.normalize(self.before, self.sources, []),
                                    compare.normalize(dict(self.after, **{key:value}), self.sources, []))


if __name__ == '__main__':
    unittest.main()
