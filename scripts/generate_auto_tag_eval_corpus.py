#!/usr/bin/env python3
"""Freeze metadata-only auto-tag evaluation inputs from the curated library."""
from __future__ import annotations
import argparse, hashlib, json, os, re, sys
from pathlib import Path
from datetime import date
from mutagen import File
from mutagen.id3 import ID3

ROOT = Path()
ARTISTS = ['Ariana Grande','Billie Eilish','Eagles','Doja Cat','Ellie Goulding','Eminem','Enya']
PROVISIONAL_GOLD = ['Ariana Grande','Billie Eilish','Eagles','Doja Cat','Ellie Goulding']
EXTS = {'.flac','.mp3','.m4a','.wav','.ogg','.opus','.ape','.aiff','.mp4','.wma'}
OUT = Path('test/fixtures/tauri/auto-tag-eval/corpus.json')
EXPECTATIONS_OUT = Path('test/fixtures/tauri/auto-tag-eval/expectations.json')

def visible(p: Path, root: Path) -> bool:
    try: rel = p.relative_to(root)
    except ValueError: return False
    return not any(part.startswith('.') for part in rel.parts)

def text(value):
    if value is None: return None
    if isinstance(value, (list, tuple)):
        return str(value[0]) if value else None
    return str(value)

def first(tags, *keys):
    if not tags: return None
    for key in keys:
        value = tags.get(key)
        if value:
            return text(value)
    return None

def credits(tags, *keys):
    for key in keys:
        value = tags.get(key)
        if value:
            return [str(item) for item in value] if isinstance(value, (list, tuple)) else [str(value)]
    return []

def ints(value):
    if not value: return None
    m = re.match(r'\s*(\d+)(?:\s*/\s*(\d+))?', text(value) or '')
    return (int(m.group(1)), int(m.group(2)) if m.group(2) else None) if m else None

def track_metadata(path: Path, artist_root: Path, folder: Path):
    raw = path.read_bytes()
    h = hashlib.sha256(raw).hexdigest()
    audio = File(path, easy=True)
    tags = getattr(audio, 'tags', None) or {}
    native_audio = File(path)
    native_tags = getattr(native_audio, 'tags', None)
    user_text = {frame.desc.casefold(): frame.text for frame in native_tags.getall('TXXX')} if isinstance(native_tags, ID3) else {}
    artist_credits = credits(tags, 'artists', 'artist')
    album_credits = credits(tags, 'albumartists', 'albumartist', 'album artist')
    for key, target in [('artists', artist_credits), ('albumartists', album_credits)]:
        if user_text.get(key):
            target[:] = [part.strip() for value in user_text[key] for part in value.split(';') if part.strip()]
    track = ints(first(tags, 'tracknumber', 'trkn'))
    disc = ints(first(tags, 'discnumber', 'disknumber', 'disk'))
    title = first(tags, 'title')
    artists = first(tags, 'artist')
    album = first(tags, 'album')
    album_artist = first(tags, 'albumartist', 'album artist')
    year = first(tags, 'date', 'year', 'originaldate')
    if year: year = year[:4]
    ids = {
        'musicbrainzTrackId': first(tags, 'musicbrainz_trackid', 'musicbrainz_track_id'),
        'musicbrainzAlbumId': first(tags, 'musicbrainz_albumid', 'musicbrainz_album_id'),
        'musicbrainzArtistId': first(tags, 'musicbrainz_artistid', 'musicbrainz_artist_id'),
        'discogsArtistId': first(tags, 'discogs_artist_id', 'discogs_artistid'),
        'discogsReleaseId': first(tags, 'discogs_release_id', 'discogs_releaseid'),
    }
    for key, description in [('musicbrainzTrackId', 'MusicBrainz Track Id'),
                             ('musicbrainzAlbumId', 'MusicBrainz Album Id'),
                             ('musicbrainzArtistId', 'MusicBrainz Artist Id'),
                             ('discogsArtistId', 'Discogs Artist Id'),
                             ('discogsReleaseId', 'Discogs Release Id')]:
        value = first(user_text, description.casefold())
        if value: ids[key] = value
    ids = {k:v for k,v in ids.items() if v}
    duration = float(getattr(audio.info, 'length', 0.0) or 0.0) if audio else 0.0
    rel = path.relative_to(folder).as_posix()
    return {
        'relativePath': rel,
        'filename': path.name,
        'title': title,
        'artist': artists,
        'artists': artist_credits,
        'album': album,
        'albumArtist': album_artist,
        'albumArtists': album_credits,
        'year': year,
        'trackNumber': track[0] if track else None,
        'trackTotal': track[1] if track else None,
        'discNumber': disc[0] if disc else None,
        'discTotal': disc[1] if disc else None,
        'duration': round(duration, 3),
        'format': path.suffix.lower().lstrip('.'),
        'bytes': len(raw),
        'sourceSha256': h,
        'providerIds': ids,
    }

def release_type(name: str) -> str:
    lower = name.lower()
    if any(x in lower for x in ('box', 'disc set', '8cd', '11 cd', '6 releases')): return 'box_set'
    if any(x in lower for x in ('single', 'cds', 'cdm', 'promo', 'remix ep')): return 'single'
    if re.search(r'\bep\b', lower): return 'ep'
    return 'album'

def release_family(relative_folder: str) -> str:
    """Group physical disc folders while keeping unrelated editions apart."""
    parts = relative_folder.split('/')
    while len(parts) > 1 and re.match(r'(?i)^(?:cd|disc|disk|bonus\s+disc|bonus\s+cd)\s*\d*\b', parts[-1]):
        parts.pop()
    return '/'.join(parts)

def discovery_track(t):
    return {k: t.get(k) for k in ('relativePath','filename','duration','format','trackNumber','trackTotal','discNumber','discTotal')}

def assisted_track(t):
    out = dict(t)
    out['providerIds'] = {}
    return out

def recovery_track(t):
    return dict(t)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--source-root",
        type=Path,
        default=os.environ.get("SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT"),
        help="curated library root; may also be set with SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT",
    )
    parser.add_argument("--artist", action="append", help="artist container to inventory; repeatable")
    parser.add_argument("--compilation-container", action="append", default=[], help="container of independent compilation albums; repeatable")
    parser.add_argument("--selection", type=Path, help="JSON manifest of explicitly sampled album folders")
    parser.add_argument("--output-dir", type=Path, default=OUT.parent)
    parser.add_argument("--corpus-version", default="2026-09-12.inventory-1")
    args = parser.parse_args()
    if args.source_root is None:
        raise SystemExit("--source-root or SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT is required")
    global ROOT
    ROOT = args.source_root.expanduser().resolve()
    if not ROOT.is_dir():
        raise SystemExit(f"source root does not exist: {ROOT}")
    artists = args.artist if args.artist is not None else ([] if args.compilation_container else ARTISTS)
    containers = artists + args.compilation_container
    if len(set(containers)) != len(containers):
        raise SystemExit("artist and compilation containers must be unique")
    for container in containers:
        if Path(container).name != container or container in {".", ".."}:
            raise SystemExit(f"container must be one directory name: {container}")
        if (args.artist or args.compilation_container) and not (ROOT / container).is_dir():
            raise SystemExit(f"missing source container: {container}")
    selections = [
        {'folder': container, 'artist': 'Various Artists' if container in args.compilation_container else container,
         'releaseType': 'compilation' if container in args.compilation_container else None}
        for container in containers
    ]
    if args.selection:
        if args.artist or args.compilation_container:
            raise SystemExit("--selection cannot be combined with container options")
        selection = json.loads(args.selection.read_text(encoding='utf-8'))
        selections = selection.get('folders')
        if selection.get('schemaVersion') != 1 or not isinstance(selections, list) or not selections:
            raise SystemExit("selection needs schemaVersion 1 and nonempty folders")
        for row in selections:
            folder = Path(row['folder'])
            if folder.is_absolute() or '..' in folder.parts or not (ROOT / folder).resolve().is_relative_to(ROOT):
                raise SystemExit(f"selected folder escapes source root: {folder}")
            if not (ROOT / folder).is_dir():
                raise SystemExit(f"missing selected folder: {folder}")
            if not row.get('artist') or not row.get('reason') or not isinstance(row.get('labels'), list):
                raise SystemExit(f"selected folder needs artist, reason and labels: {folder}")
    cases=[]
    physical=[]
    for row in selections:
        artist = row['artist']
        ar = ROOT / row['folder']
        folders = sorted({p.parent for p in ar.rglob('*') if p.is_file() and p.suffix.lower() in EXTS and visible(p, ar)})
        if args.selection and not folders:
            raise SystemExit(f"selected folder contains no supported audio: {row['folder']}")
        for folder in folders:
            rel = folder.relative_to(ROOT).as_posix()
            if rel in physical:
                raise SystemExit(f"overlapping selected folder: {rel}")
            case_id = hashlib.sha1(rel.encode()).hexdigest()[:12]
            family = release_family(rel)
            group_id = hashlib.sha1(family.encode()).hexdigest()[:12]
            tracks=[]
            for path in sorted(folder.iterdir()):
                if path.is_file() and path.suffix.lower() in EXTS:
                    tracks.append(track_metadata(path, ar, folder))
            if not tracks: continue
            physical.append(rel)
            cases.append({
                'caseId': case_id,
                'artist': artist,
                'sourceRelativeFolder': rel,
                'physicalFolders': [rel],
                'releaseGroupId': f'{artist.lower().replace(" ","-")}:{group_id}',
                'editionFamily': family,
                'releaseType': row.get('releaseType') or release_type(folder.name),
                'difficulty': 'unverified',
                'difficultyLabels': row.get('labels', []),
                'selectionRationale': row.get('reason'),
                'tracks': tracks,
                'inputProfiles': {
                    'folder_filename': {'tracks': [discovery_track(t) for t in tracks], 'providerIds': 'stripped', 'tags': 'stripped'},
                    'assisted_without_ids': {'tracks': [assisted_track(t) for t in tracks], 'providerIds': 'stripped', 'tags': 'captured'},
                    'tagged_recovery': {'tracks': [recovery_track(t) for t in tracks], 'providerIds': 'preserved', 'tags': 'captured'},
                },
                'expectation': {
                    'status': 'unverified',
                    'provenance': 'inventory-only; provider tracklist and edition mapping review pending',
                    'reviewer': None,
                    'reviewedDate': None,
                    'rationale': 'No target release or hard negative is blessed from equal-count/title evidence.',
                    'acceptableEditionIds': [],
                    'rejectedHardNegativeIds': [],
                    'mapping': [],
                },
                'providerSnapshots': [],
                'providerSnapshotStatus': 'not_captured',
            })
    cases.sort(key=lambda x: x['caseId'])
    corpus={
      'schemaVersion': 1,
      'corpusVersion': args.corpus_version,
      'capturedDate': str(date.today()),
      'sourceRoot': '.',
      'artists': sorted({case['artist'] for case in cases}),
      'summary': {'physicalFolders': len(physical), 'tracks': sum(len(c['tracks']) for c in cases), 'cases': len(cases), 'verifiedMatchable': 0, 'verifiedAbstain': 0, 'unverified': len(cases), 'excluded': 0, 'provisionalGoldArtists': sum(artist in PROVISIONAL_GOLD for artist in {c['artist'] for c in cases}), 'diagnosticUnverifiedArtists': len({c['artist'] for c in cases} - set(PROVISIONAL_GOLD))},
      'artistReview': {
        artist: {
          'status': 'provisional_gold_unverified' if artist in PROVISIONAL_GOLD else 'diagnostic_unverified',
          'provenance': 'Captured assisted metadata; independent provider content and mapping validation required before scoring.' if artist in PROVISIONAL_GOLD else 'Diagnostic population retained outside scored metrics until reviewed.',
          'reviewer': None,
          'reviewedDate': None,
        }
        for artist in sorted({case['artist'] for case in cases})
      },
      'profiles': {
        'folder_filename': {'purpose':'clean discovery from folder and filename evidence', 'strips':['tags','providerIds']},
        'assisted_without_ids': {'purpose':'assisted-tag recovery without provider IDs', 'strips':['providerIds']},
        'tagged_recovery': {'purpose':'direct lookup and idempotence replay', 'strips':[]},
      },
      'providerSnapshots': [
        {'provider':'discogs','releaseId':'36441795','path':'provider-snapshots/relapse/discogs-release-36441795.json','capturedFrom':'existing Relapse regression fixture'},
        {'provider':'discogs','releaseId':'16649340','path':'provider-snapshots/relapse/discogs-release-16649340.json','capturedFrom':'existing Relapse hard-negative fixture'},
      ] if 'Eminem' in artists and not args.selection else [],
      'cases': cases,
      'physicalFolderIndex': physical,
      'reviewPolicy': {'compatibleCompleteEditionsAreCorrect': True, 'pressingMustBeProven': True, 'equalCountTitleIsInsufficient': True, 'unseenRelease':'oracle_review_required'},
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    if args.selection:
        corpus['selectionManifest'] = 'selection.json'
        (args.output_dir / 'selection.json').write_text(args.selection.read_text(encoding='utf-8'), encoding='utf-8')
    (args.output_dir / OUT.name).write_text(json.dumps(corpus, ensure_ascii=False, indent=2) + '\n')
    expectations = {
        'schemaVersion': 1,
        'corpusVersion': corpus['corpusVersion'],
        'cases': [
            {'caseId': case['caseId'], 'artist': case['artist'], 'releaseGroupId': case['releaseGroupId'], **case['expectation']}
            for case in cases
        ],
    }
    (args.output_dir / EXPECTATIONS_OUT.name).write_text(json.dumps(expectations, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(corpus['summary']))

if __name__ == '__main__': main()
