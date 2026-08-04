#!/usr/bin/env bash
# ============================================================================
# toolbox.sh — unified audio / music-library toolbox.
#
# One entry point for the per-purpose scripts that used to live in scripts/:
# cue splitting, DSF conversion, ISO slicing, RAR extraction, and the FLAC
# doctor/QA pipeline (which delegates to the node tools in this folder).
#
# Usage:
#   ./toolbox.sh <command> [options] [args]
#   ./toolbox.sh -h | --help
#   ./toolbox.sh <command> -h        # command-specific help
#
# Commands:
#   cue-split            Split FLAC/WAV album images per CUE sheet into
#                        per-track FLACs; copies album images alongside
#                        (-r recursive, -a artist mode, --no-doctor)
#   dsf-to-flac          Convert DSF (DSD/SACD) files to FLAC with metadata
#   slice-iso            Slice audio ISO images (K2HD SACD UDF / raw CD)
#                        into FLAC tracks
#   unrar                Extract RAR archives (unar, 7z fallback)
#   doctor               Scan, diagnose, and fix FLAC metadata corruption
#                        (delegates to fix-flac-metadata.js; renders the HTML
#                        report automatically when one is saved)
#   corpus               Build a reproducible FLAC test corpus from a library
#                        (delegates to build-flac-test-corpus.js)
#   corruption-report    Render a doctor scan into an HTML corruption report
#                        (delegates to generate-corruption-report.js)
#   aggregate-checkpoint Aggregate checkpoint batches into one report JSON
#                        (delegates to aggregate-checkpoint.js)
#
# Requirements (per command): python3, ffmpeg, ffprobe, node, hdiutil, unar/7z.
# ============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

toolbox_usage() {
  cat <<EOF
Usage: $(basename "$0") <command> [options] [args]

Audio / library tools:
  cue-split <path>...        Split FLAC/WAV album images per CUE sheet into per-track FLACs
  dsf-to-flac <dir> [artist] Convert DSF (DSD/SACD) files to FLAC with metadata
  slice-iso [dir]            Slice audio ISO images (K2HD SACD UDF / raw CD) into FLAC tracks
  unrar <dir|--file F>       Extract RAR archives (unar, 7z fallback)

FLAC health / QA tools (node):
  doctor <dir> [opts]        Scan, diagnose, and fix FLAC metadata (renders HTML report if one is saved)
  corpus [opts]              Build a reproducible FLAC test corpus from a library
  corruption-report <report> [html]  Render a doctor scan into an HTML report
  aggregate-checkpoint <dir> [json]  Aggregate checkpoint batches into a report JSON

Run '$(basename "$0") <command> -h' for command-specific help.
EOF
  exit "${1:-0}"
}

# ── cue-split ────────────────────────────────────────────────────────────────
# Split single-file FLAC/WAV album images into per-track FLAC files using the
# album's .cue sheet, and copy the album's image files into the output folder.
# Combines: slice-flac-per-cue.sh (core) + cue2flac.sh (recursive/artist modes,
# post-slice doctor hookup).
#
#   toolbox.sh cue-split [OPTIONS] <path> [<path> ...]
#
#   -r, --recursive   Recurse into subdirectories to find .cue files
#   -a, --artist      Artist mode: output to <artist>-processed/<album>/...
#   -o, --output DIR  Output directory (single album; default: <album>-tracks sibling)
#   -f, --force       Re-slice even if the output folder already has FLACs
#   -n, --dry-run     Show what would be done without doing it
#       --no-doctor   Skip the post-slice FLAC doctor health check
#   -h, --help
cmd_cue_split() {
  local RECURSIVE=false ARTIST_MODE=false FORCE=false DRY_RUN=false DOCTOR=false
  local OUTPUT="" TARGETS=()

  while [[ $# -gt 0 ]]; do
    case "$1" in
      -r|--recursive) RECURSIVE=true; shift ;;
      -a|--artist)    ARTIST_MODE=true; shift ;;
      -o|--output)    OUTPUT="$2"; shift 2 ;;
      -f|--force)     FORCE=true; shift ;;
      -n|--dry-run)   DRY_RUN=true; shift ;;
      --doctor)       DOCTOR=true; shift ;;
      -h|--help)
        cat <<EOF
Usage: $(basename "$0") cue-split [OPTIONS] <path> [<path> ...]

Split single-file FLAC/WAV album images into per-track FLAC files using each
album's .cue sheet; copies the album's image files alongside the tracks.

Options:
  -r, --recursive   Recurse into subdirectories to find .cue files (default: only the given folder)
  -a, --artist      Artist mode: output to <artist>-processed/<album>/...
  -o, --output DIR  Output directory (single album; default: <album>-tracks sibling)
  -f, --force       Re-slice even if the output folder already has FLACs
  -n, --dry-run     Show what would be done without doing it
      --doctor      Run the FLAC doctor health check on each sliced output
  -h, --help        Show this help
EOF
        exit 0 ;;
      -*) echo "Error: unknown option: $1" >&2; exit 1 ;;
      *)  TARGETS+=("$1"); shift ;;
    esac
  done

  if [[ ${#TARGETS[@]} -eq 0 ]]; then
    echo "Error: no path given." >&2
    sed -n '/^# ── cue-split/,/^cmd_cue_split()/p' "$0" | sed 's/^# \{0,1\}//' | sed -n '/Usage:/,/^$/p'
    exit 1
  fi

  for cmd in python3 ffmpeg ffprobe; do
    if ! command -v "$cmd" &>/dev/null; then
      echo "Error: '$cmd' not found." >&2
      exit 1
    fi
  done

  python3 - "${TARGETS[@]}" "$OUTPUT" "$RECURSIVE" "$ARTIST_MODE" "$FORCE" "$DRY_RUN" "$DOCTOR" "$SCRIPT_DIR" <<'PYEOF'
import os, re, shutil, subprocess, sys

TARGETS = sys.argv[1:-7]
OUTPUT, RECURSIVE, ARTIST_MODE, FORCE, DRY_RUN, DOCTOR, SCRIPT_DIR = sys.argv[-7:]
RECURSIVE = RECURSIVE == "true"
ARTIST_MODE = ARTIST_MODE == "true"
FORCE = FORCE == "true"
DRY_RUN = DRY_RUN == "true"
DOCTOR = DOCTOR == "true"

IMAGE_EXTS = ('.jpg', '.jpeg', '.png', '.gif', '.bmp', '.webp', '.tif', '.tiff')
AUDIO_EXTS = ('.flac', '.wav')

def read_text(path):
    """Read a file, auto-detect encoding (UTF-8 / GBK / Latin-1), normalize EOLs."""
    with open(path, 'rb') as f:
        data = f.read()
    data = data.replace(b'\r\n', b'\n').replace(b'\r', b'\n')
    for enc in ('utf-8', 'gbk', 'latin-1'):
        try:
            return data.decode(enc)
        except UnicodeDecodeError:
            continue
    return data.decode('latin-1')

def msf_to_seconds(msf):
    """Convert MM:SS:FF (CD frames, 75 per second) to seconds."""
    parts = re.split(r'[:.,]', msf.strip())
    if len(parts) == 3:
        m, s, f = (float(p) for p in parts)
        return m * 60 + s + f / 75.0
    if len(parts) == 2:
        m, s = (float(p) for p in parts)
        return m * 60 + s
    return float(parts[0])

def sanitize(name):
    """Make a string safe for a filename (keep EAC's [*] bonus marker)."""
    name = re.sub(r'[/\\:?"<>|]', '', name)
    name = re.sub(r'\s+', ' ', name).strip().rstrip('. ')
    return name

def quoted_val(line):
    """Extract the value from 'KEY "value"' or 'KEY value' lines."""
    if '"' in line:
        return line.split('"', 1)[1].rsplit('"', 1)[0].strip()
    parts = line.split(None, 1)
    return parts[1].strip() if len(parts) > 1 else ''

def parse_cue(path):
    """Parse a cue sheet -> (album meta dict, list of track dicts)."""
    text = read_text(path)
    album = {'performer': '', 'title': '', 'genre': '', 'date': '', 'file': ''}
    tracks = []
    cur = None
    for line in text.split('\n'):
        line = line.strip()
        upper = line.upper()
        if upper.startswith('REM '):
            rest = line[4:].strip()
            key, _, val = rest.partition(' ')
            if key.upper() in ('GENRE', 'DATE'):
                album[key.lower()] = val.strip().strip('"')
        elif upper.startswith('PERFORMER'):
            val = quoted_val(line)
            if cur is None:
                album['performer'] = val
            else:
                cur['performer'] = val
        elif upper.startswith('TITLE'):
            val = quoted_val(line)
            if cur is None:
                album['title'] = val
            else:
                cur['title'] = val
        elif upper.startswith('FILE'):
            m = re.match(r'FILE\s+"([^"]+)"', line)
            if m and not album['file']:
                album['file'] = m.group(1)
        elif upper.startswith('TRACK '):
            m = re.match(r'TRACK\s+(\d+)', line)
            if m:
                cur = {'num': int(m.group(1)), 'title': '', 'performer': '', 'start': None}
                tracks.append(cur)
        elif upper.startswith('INDEX ') and cur is not None:
            m = re.match(r'INDEX\s+(\d+)\s+([\d:.,]+)', line)
            if m and int(m.group(1)) == 1 and cur['start'] is None:
                cur['start'] = msf_to_seconds(m.group(2))
    # EAC convention: the pregap before track 1 belongs to track 1 -> start at 0.
    # Only when track 1 actually has an INDEX 01; otherwise leave None so the
    # missing-index validation can catch it.
    if tracks and tracks[0]['start'] is not None:
        tracks[0]['start'] = 0.0
    return album, tracks

def duration_of(path):
    try:
        r = subprocess.run(['ffprobe', '-v', 'error', '-show_entries', 'format=duration',
                            '-of', 'csv=p=0', path], capture_output=True, text=True, timeout=30)
        return float(r.stdout.strip()) if r.stdout.strip() else 0.0
    except Exception:
        return 0.0

def copy_images(src_dir, out_dir):
    copied = 0
    for item in sorted(os.listdir(src_dir)):
        if item.lower().endswith(IMAGE_EXTS):
            src = os.path.join(src_dir, item)
            dst = os.path.join(out_dir, item)
            if os.path.isfile(src) and not os.path.exists(dst):
                shutil.copy2(src, dst)
                copied += 1
    return copied

def find_cues(target, max_depth):
    """Find .cue files under target, up to max_depth levels deep."""
    results = []
    for root, dirs, files in os.walk(target):
        depth = root[len(target):].count(os.sep)
        if depth >= max_depth:
            dirs[:] = []
            continue
        for f in sorted(files):
            if f.lower().endswith('.cue'):
                results.append(os.path.join(root, f))
    return sorted(results)

def run_doctor(out_dir):
    """Post-slice health check: run the FLAC doctor over the output folder."""
    doctor = os.path.join(SCRIPT_DIR, 'fix-flac-metadata.js')
    if not (os.path.isfile(doctor) and shutil.which('node')):
        return
    print()
    print("  🏥 Running FLAC doctor health check on output...")
    try:
        r = subprocess.run(['node', doctor, out_dir, '--doctor'],
                           capture_output=True, text=True, timeout=600)
        out = (r.stdout or '').strip()
        if out:
            print(out)
        if r.returncode != 0 and (r.stderr or '').strip():
            print((r.stderr or '').strip()[-800:])
    except Exception as e:
        print(f"  (doctor skipped: {e})")

def _group_cues_by_dir(cues):
    """Group a flat list of cue paths by their album directory, preserving order."""
    groups = {}
    for cue in cues:
        groups.setdefault(os.path.dirname(cue), []).append(cue)
    return groups


DISC_SUFFIX_RE = re.compile(
    r'(?i)(?:\s*[-–—]?\s*)?[\[(]?\s*(?:CD|DISC)\s*0*(\d+)\s*[\])]?\s*$'
)


def split_disc_title(title):
    """Return an album title and disc number only for a recognized suffix."""
    match = DISC_SUFFIX_RE.search(title)
    if not match:
        return title, None
    album_title = title[:match.start()].strip()
    if not album_title:
        return title, None
    return album_title, int(match.group(1))


def split_album(album_dir, artist_root, cue_path, disc_label='', disc_num=0,
                disc_total=0, album_tag=None):
    """Slice one cue's tracks into FLACs. Multi-disc folders pass a disc_label
    so each disc lands in its own subfolder with disc metadata."""
    album_dir = os.path.abspath(album_dir)
    album, tracks = parse_cue(cue_path)
    if not tracks:
        print(f"⚠  SKIP: no tracks parsed from {cue_path}")
        return False
    missing = [t['num'] for t in tracks if t['start'] is None]
    if missing:
        print(f"⚠  SKIP: {os.path.basename(cue_path)}: track(s) missing INDEX 01: {missing}")
        return False

    # Locate the audio source: FILE directive first, else the only audio file.
    src = None
    if album['file']:
        cand = os.path.join(album_dir, album['file'])
        if os.path.isfile(cand):
            src = cand
    if src is None:
        audios = [f for f in os.listdir(album_dir)
                  if f.lower().endswith(AUDIO_EXTS) and os.path.isfile(os.path.join(album_dir, f))]
        if len(audios) == 1:
            src = os.path.join(album_dir, audios[0])
    if src is None:
        print(f"⚠  SKIP: no matching audio file for {cue_path}")
        return False

    if OUTPUT:
        out_dir = OUTPUT
    elif ARTIST_MODE:
        rel = os.path.relpath(album_dir, artist_root)
        out_dir = os.path.join(artist_root.rstrip('/') + '-processed', rel)
    else:
        out_dir = album_dir.rstrip('/') + '-tracks'
    if disc_label:
        output_root = os.path.abspath(out_dir)
        out_dir = os.path.abspath(os.path.join(output_root, disc_label))
        if os.path.commonpath((output_root, out_dir)) != output_root:
            print(f"⚠  SKIP: unsafe disc output label: {disc_label}")
            return False

    album_tag = album['title'] if album_tag is None else album_tag

    n = len(tracks)
    if not FORCE and os.path.isdir(out_dir):
        existing = [f for f in os.listdir(out_dir) if f.lower().endswith('.flac')]
        if len(existing) >= n:
            print(f"⏭  SKIP: output already has {len(existing)} FLAC(s): {out_dir} (use -f to re-slice)")
            return True
        if existing:
            print(f"⚠  Incomplete output ({len(existing)}/{n} FLACs) in {out_dir} — re-slicing")

    print("━" * 60)
    print(f"Album:   {album['title'] or os.path.basename(album_dir)}"
          f"  ({album['performer']})")
    print(f"Source:  {os.path.basename(src)}")
    print(f"Tracks:  {n}")
    print(f"Output:  {out_dir}")
    print("━" * 60)

    if DRY_RUN:
        for i, t in enumerate(tracks):
            end = tracks[i + 1]['start'] if i + 1 < n else 'end'
            end_str = f'{end:.3f}' if end != 'end' else 'end'
            print(f"  [dry-run] {t['num']:02d}. {sanitize(t['title']) or 'Track'}.flac"
                  f"  [{t['start']:.3f} → {end_str}]")
        return True

    os.makedirs(out_dir, exist_ok=True)
    imgs = copy_images(album_dir, out_dir)

    ok = True
    for i, t in enumerate(tracks):
        start = t['start']
        end = tracks[i + 1]['start'] if i + 1 < n else None
        if end is not None and end <= start:
            print(f"  ✗ Track {t['num']:02d}: bad range [{start:.3f} → {end:.3f}], skipped")
            ok = False
            continue

        artist = t['performer'] or album['performer']
        title = sanitize(t['title'])
        fname = f"{t['num']:02d}. {title}.flac" if title else f"{t['num']:02d}.flac"
        out_file = os.path.join(out_dir, fname)

        cmd = ['ffmpeg', '-y', '-ss', f'{start:.6f}']
        if end is not None:
            cmd += ['-to', f'{end:.6f}']
        cmd += ['-i', src]
        cmd += ['-c:a', 'flac', '-compression_level', '8']
        cmd += ['-metadata', f'artist={artist}',
                '-metadata', f'album={album_tag}',
                '-metadata', f'album_artist={album["performer"]}',
                '-metadata', f'title={t["title"]}',
                '-metadata', f'track={t["num"]}/{n}']
        if disc_total > 0:
            cmd += ['-metadata', f'disc={disc_num}/{disc_total}']
        if album['genre']:
            cmd += ['-metadata', f'genre={album["genre"]}']
        if album['date']:
            cmd += ['-metadata', f'date={album["date"]}']
        cmd.append(out_file)

        end_str = f"{end:.3f}" if end is not None else "end"
        print(f"  Track {t['num']:02d}/{n} [{start:.3f} → {end_str}]: {t['title']} ... ", end='', flush=True)
        try:
            subprocess.run(cmd, capture_output=True, timeout=900, check=True)
            print(f"({duration_of(out_file):.0f}s)")
        except subprocess.CalledProcessError as e:
            print("FAILED")
            tail = e.stderr.decode('utf-8', errors='replace').strip().split('\n')[-3:]
            for line in tail:
                if line.strip():
                    print(f"      {line.strip()}")
            if os.path.exists(out_file):
                os.remove(out_file)
            ok = False

    if ok:
        flacs = len([f for f in os.listdir(out_dir) if f.lower().endswith('.flac')])
        print(f"✅  Done: {flacs} tracks + {imgs} image(s) → {out_dir}")
        if DOCTOR:
            run_doctor(out_dir)
    else:
        print(f"❌  Album finished with errors: {out_dir}")
    return ok

# ── main ──
failures = 0
max_depth = 5 if RECURSIVE else 1

target_info = []
for target in TARGETS:
    target = os.path.abspath(target)
    if not os.path.isdir(target):
        print(f"⚠  SKIP: not a directory: {target}")
        failures += 1
        continue
    target_info.append((target, find_cues(target, max_depth)))

total_cues = sum(len(cues) for _, cues in target_info)
if total_cues == 0:
    print("No .cue files found.")
    sys.exit(1)
album_dirs = {
    os.path.dirname(cue)
    for _, cues in target_info
    for cue in cues
}
if OUTPUT and len(album_dirs) > 1:
    print("Error: --output with multiple albums is ambiguous; "
          "use one album per -o or omit -o for per-album defaults")
    sys.exit(1)

for target, cues in target_info:
    if not cues:
        print(f"⚠  No .cue files found in '{target}'")
        failures += 1
        continue

    # Artist root for artist mode: TARGET itself if it holds album subfolders
    # with cues, otherwise the parent (single-album target).
    artist_root = target
    if ARTIST_MODE:
        has_albums = False
        for item in os.listdir(target):
            p = os.path.join(target, item)
            if os.path.isdir(p) and not item.endswith(('-tracks', '-processed')):
                if any(f.lower().endswith('.cue') for f in os.listdir(p)):
                    has_albums = True
                    break
        if not has_albums:
            artist_root = os.path.dirname(target)

    for album_dir, dir_cues in _group_cues_by_dir(cues).items():
        parsed_cues = []
        for cue in dir_cues:
            alb, _ = parse_cue(cue)
            album_tag, disc_num = split_disc_title(alb['title'])
            parsed_cues.append((cue, alb['title'], album_tag, disc_num))

        expected_disc_nums = set(range(1, len(parsed_cues) + 1))
        parsed_disc_nums = {item[3] for item in parsed_cues}
        use_parsed_order = (
            len(parsed_cues) > 1
            and None not in parsed_disc_nums
            and parsed_disc_nums == expected_disc_nums
        )
        if use_parsed_order:
            parsed_cues.sort(key=lambda item: item[3])

        for i, (cue, original_title, parsed_title, _) in enumerate(parsed_cues, start=1):
            is_multi_disc = len(parsed_cues) > 1
            album_tag = parsed_title if use_parsed_order else original_title
            if not split_album(album_dir, artist_root, cue,
                               f'CD{i}' if is_multi_disc else '',
                               i if is_multi_disc else 0,
                               len(parsed_cues) if is_multi_disc else 0,
                               album_tag):
                failures += 1
        print()

print("═" * 60)
if failures:
    print(f"Finished with {failures} failed album(s).")
    sys.exit(1)
print("Done.")
print("═" * 60)
PYEOF
}

# ── dsf-to-flac ──────────────────────────────────────────────────────────────
# Convert DSF (DSD/SACD) files to FLAC with metadata.
#   toolbox.sh dsf-to-flac <source_dir> [artist]
#   toolbox.sh dsf-to-flac /Volumes/downloads/李克勤/李克勤\ -\ 我著10号\ SACD 李克勤
# Env overrides: TARGET_RATE (88200), LOWPASS_FREQ (30000), BITS_PER_SAMPLE (24)
cmd_dsf_to_flac() {
  if [[ $# -lt 1 || "$1" == "-h" || "$1" == "--help" ]]; then
    cat <<EOF
Usage: $(basename "$0") dsf-to-flac <source_dir> [artist]

Convert DSF (DSD/SACD) files to FLAC with metadata. Reads track titles from a
track-listing txt (GBK-aware) if present; copies album images to the output.

Example:
  $(basename "$0") dsf-to-flac /Volumes/downloads/李克勤/李克勤\ -\ 我著10号\ SACD

Env overrides: TARGET_RATE (88200), LOWPASS_FREQ (30000), BITS_PER_SAMPLE (24)
EOF
    [[ $# -ge 1 && ( "$1" == "-h" || "$1" == "--help" ) ]] && exit 0
    exit 1
  fi

  local SOURCE_DIR="$1" ARTIST="${2:-$(basename "$(dirname "$1")")}"
  local OUTPUT_BASE="${HOME}/Music/${ARTIST}"
  local ALBUM_NAME="$(basename "${SOURCE_DIR}")"
  local ALBUM_OUTPUT="${OUTPUT_BASE}/${ALBUM_NAME}"
  local LOG_FILE="${SCRIPT_DIR}/dsf-to-flac.log"

  local TARGET_RATE="${TARGET_RATE:-88200}"
  local LOWPASS_FREQ="${LOWPASS_FREQ:-30000}"
  local BITS_PER_SAMPLE="${BITS_PER_SAMPLE:-24}"

  mkdir -p "${ALBUM_OUTPUT}"

  log() { echo "$@" | tee -a "${LOG_FILE}"; }

  # Parse txt file for track names (GBK encoded)
  local -a TRACK_NAMES=()
  parse_txt_file() {
    local txt_file
    txt_file=$(find "${SOURCE_DIR}" -maxdepth 1 -name "*.txt" ! -name "a_*" ! -name "必读*" | head -1)
    if [ -z "${txt_file}" ] || [ ! -f "${txt_file}" ]; then
      log "  No track listing txt found"
      return
    fi

    local encoding
    encoding=$(file -b --mime-encoding "${txt_file}")

    local in_tracks=0 line title
    while IFS= read -r line; do
      line="${line%$'\r'}"
      if [[ "${line}" =~ ^([0-9]+)[.\ ]+(.+) ]]; then
        in_tracks=1
        local tn=$((10#${BASH_REMATCH[1]}))
        title="${BASH_REMATCH[2]}"
        title=$(echo "${title}" | tr -d '\r' | sed 's/[\/:<>"|?*]//g' | sed 's/  */ /g' | sed 's/^ //;s/ $//')
        [ -n "${title}" ] && TRACK_NAMES[${tn}]="${title}"
      elif [[ "${line}" =~ 曲目 ]]; then
        in_tracks=1
      fi
    done < <(
        if [[ "${encoding}" == *iso-8859* ]] || [[ "${encoding}" == *unknown* ]]; then
          iconv -f GBK -t UTF-8 "${txt_file}" 2>/dev/null
        else
          cat "${txt_file}"
        fi
        echo ""
    )

    log "  Loaded ${#TRACK_NAMES[@]} track names from $(basename "${txt_file}")"
  }

  parse_txt_file

  shopt -s nullglob
  local -a DSF_FILES=("${SOURCE_DIR}"/*.dsf "${SOURCE_DIR}"/*.DSF)
  local total_tracks=${#DSF_FILES[@]}
  shopt -u nullglob

  log "============================================================"
  log "DSF to FLAC started at $(date)"
  log "Source: ${SOURCE_DIR}"
  log "Output: ${ALBUM_OUTPUT}"
  log "Tracks: ${total_tracks}  Target rate: ${TARGET_RATE} Hz"
  log "============================================================"

  local errors=0
  local dsf basename_dsf tn tn_num title clean_name title_from_filename out_file track_meta
  for dsf in "${DSF_FILES[@]}"; do
    basename_dsf="$(basename "${dsf}")"

    tn=""
    tn_num=0
    title=""

    if [[ "${basename_dsf}" =~ ^([0-9]+) ]]; then
      tn="${BASH_REMATCH[1]}"
      tn_num=$((10#${tn}))
    fi

    if [ -n "${tn}" ] && [ -n "${TRACK_NAMES[${tn_num}]+x}" ]; then
      title="${TRACK_NAMES[${tn_num}]}"
    else
      clean_name="${basename_dsf%.*}"
      title_from_filename="${clean_name% - *}"
      [ "${title_from_filename}" = "${clean_name}" ] && title_from_filename="${clean_name}"

      local check_idx
      local matched=0
      for check_idx in "${!TRACK_NAMES[@]}"; do
        if [ "${TRACK_NAMES[${check_idx}]}" = "${title_from_filename}" ]; then
          tn="${check_idx}"
          tn_num=$((10#${tn}))
          title="${title_from_filename}"
          matched=1
          break
        fi
      done

      if [ "${matched}" -eq 0 ]; then
        title="${title_from_filename}"
      fi
    fi

    title=$(echo "${title}" | tr -d '\r' | sed 's/[\/:<>"|?*]//g' | sed 's/  */ /g' | sed 's/^ //;s/ $//')
    [ -z "${title}" ] && title="Track ${tn}"

    if [ -n "${tn}" ]; then
      out_file="${ALBUM_OUTPUT}/$(printf '%02d' "${tn_num}") ${title}.flac"
      track_meta="${tn_num}/${total_tracks}"
    else
      out_file="${ALBUM_OUTPUT}/${title}.flac"
      track_meta=""
    fi
    [ -f "${out_file}" ] && { log "  Skipping (exists): ${title}"; continue; }

    log "  Track ${tn_num}/${total_tracks}: ${title}..."

    local -a meta_args=(-metadata "artist=${ARTIST}" -metadata "album=${ALBUM_NAME}" -metadata "title=${title}")
    [ -n "${track_meta}" ] && meta_args+=(-metadata "track=${track_meta}")

    if ffmpeg -y -i "${dsf}" \
        -map 0:a:0 -vn -sn \
        -af "lowpass=f=${LOWPASS_FREQ},aresample=osr=${TARGET_RATE}" \
        -c:a flac -compression_level 8 \
        -sample_fmt s32 -bits_per_raw_sample "${BITS_PER_SAMPLE}" \
        "${meta_args[@]}" \
        "${out_file}" 2>>"${LOG_FILE}"; then
      log "    OK"
    else
      log "    ERROR (ffmpeg failed)"
      errors=$((errors + 1))
    fi
  done

  # Copy images
  local image_count=0
  while IFS= read -r -d '' img; do
    cp -p "${img}" "${ALBUM_OUTPUT}/"
    log "  Copied: $(basename "${img}")"
    image_count=$((image_count + 1))
  done < <(find "${SOURCE_DIR}" -maxdepth 1 -type f \( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' -o -iname '*.bmp' -o -iname '*.webp' -o -iname '*.tiff' -o -iname '*.tif' \) -print0)
  [ "${image_count}" -eq 0 ] && log "  No images found"

  log ""
  log "============================================================"
  log "Complete at $(date)"
  log "Album: ${ALBUM_NAME}  Tracks: ${total_tracks}  Errors: ${errors}"
  log "Output: ${ALBUM_OUTPUT}"
  log "============================================================"
}

# ── slice-iso ────────────────────────────────────────────────────────────────
# Slice audio ISO images into properly-named FLAC tracks.
#   toolbox.sh slice-iso [source_dir] [--artist NAME] [--output DIR]
# Defaults (env-overridable): SOURCE_DIR=/Volumes/downloads/邓丽君,
# ARTIST=Teresa Teng, OUTPUT_BASE=~/Music/<artist>
# Supported: K2HD SACD (mountable UDF ISO with 2C_AUDIO/TRACK*.2CH, 24-bit
# 96kHz sector-packed PCM) and raw CD audio (16-bit 44100Hz PCM, equal splits).
cmd_slice_iso() {
  local SOURCE_DIR="${SOURCE_DIR:-/Volumes/downloads/邓丽君}"
  local ARTIST="${ARTIST:-Teresa Teng}"
  local OUTPUT_BASE="${OUTPUT_BASE:-}"
  local LOG_FILE="${SCRIPT_DIR}/slice-isos.log"

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --artist)  ARTIST="$2"; shift 2 ;;
      --output)  OUTPUT_BASE="$2"; shift 2 ;;
      -h|--help)
        cat <<EOF
Usage: $(basename "$0") slice-iso [source_dir] [--artist NAME] [--output DIR]

Slice audio ISO images into properly-named FLAC tracks. Supports K2HD SACD
(mountable UDF ISO with 2C_AUDIO/TRACK*.2CH, 24-bit 96kHz sector-packed PCM)
and raw CD audio (16-bit 44100Hz PCM, equal splits). Reads track titles from
专辑曲目.txt when present; copies album images to the output.

Defaults (env-overridable): SOURCE_DIR=/Volumes/downloads/邓丽君,
ARTIST=Teresa Teng, OUTPUT_BASE=~/Music/<artist>
EOF
        exit 0 ;;
      -*) echo "Error: unknown option: $1" >&2; exit 1 ;;
      *)  SOURCE_DIR="$1"; shift ;;
    esac
  done

  # Default output base must reflect --artist; compute after parsing.
  OUTPUT_BASE="${OUTPUT_BASE:-${HOME}/Music/${ARTIST}}"

  mkdir -p "${OUTPUT_BASE}"

  log() { echo "$@" | tee -a "${LOG_FILE}"; }

  local TOTAL_ALBUMS=0 TOTAL_TRACKS=0 TOTAL_ERRORS=0
  inc_albums() { TOTAL_ALBUMS=$((TOTAL_ALBUMS + 1)); }
  inc_tracks() { TOTAL_TRACKS=$((TOTAL_TRACKS + 1)); }
  inc_errors() { TOTAL_ERRORS=$((TOTAL_ERRORS + 1)); }

  log "============================================================"
  log "Slice ISOs started at $(date)"
  log "Source: ${SOURCE_DIR}"
  log "Output: ${OUTPUT_BASE}"
  log "============================================================"

  local -a TITLES=()
  parse_titles() {
    local file="$1" line tn title
    TITLES=()
    if [ ! -f "${file}" ]; then return 0; fi
    while IFS= read -r line; do
        line=$(echo "${line}" | sed 's/^\xEF\xBB\xBF//' | sed 's/^[[:space:]]*//' | sed 's/[[:space:]]*$//')
        [ -z "${line}" ] && continue
        if [[ "${line}" =~ ^([0-9]+)[.、\)][[:space:]]*(.+)$ ]]; then
            local tn=$((10#${BASH_REMATCH[1]}))
            local title="${BASH_REMATCH[2]}"
            title=$(echo "${title}" | sed 's/[（(][^）)]*[）)]//g' | sed 's/^[[:space:]]*//' | sed 's/[[:space:]]*$//')
            TITLES[${tn}]="${title}"
        fi
    done < "${file}"
  }

  copy_images() {
    local src="$1" dst="$2" count=0
    while IFS= read -r -d '' img; do
        cp -p "${img}" "${dst}/"
        log "  Copied: $(basename "${img}")"
        count=$((count + 1))
    done < <(find "${src}" -maxdepth 1 -type f \( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' -o -iname '*.bmp' -o -iname '*.webp' -o -iname '*.tiff' -o -iname '*.tif' \) -print0)
    [ "${count}" -eq 0 ] && log "  No images found in ${src}" || true
  }

  extract_k2hd() {
    # Track path passed via argv (never interpolated into source).
    python3 - "$1" <<'PYEOF'
import sys
with open(sys.argv[1], 'rb') as f:
    f.read(2048)
    while True:
        s = f.read(2048)
        if len(s) < 2048: break
        sys.stdout.buffer.write(s[32:])
PYEOF
  }

  process_k2hd() {
    local iso="$1" album_dir="$2" album_name="$3" output="$4" pre_audio_dir="${5:-}"
    local audio_dir track_count track_file title safe_title out_file

    # audio_dir always comes from the caller (discovered mount or 7z extract);
    # process_k2hd never mounts an ISO itself, so mounts can't leak or double.
    if [ -z "${pre_audio_dir}" ] || [ ! -d "${pre_audio_dir}" ]; then
        log "  ERROR: no audio dir for ${iso}"
        return 1
    fi
    audio_dir="${pre_audio_dir}"

    parse_titles "${album_dir}/专辑曲目.txt"

    track_count=0
    local f
    for f in "${audio_dir}"/TRACK*.2CH; do
        [ -f "$f" ] && track_count=$((track_count + 1))
    done
    log "  Tracks: ${track_count}"

    local tn=0
    for track_file in "${audio_dir}"/TRACK*.2CH; do
        [ -f "${track_file}" ] || continue
        tn=$((tn + 1))
        title="${TITLES[${tn}]:-Track ${tn}}"
        safe_title=$(echo "${title}" | sed 's/[\/:<>"|?*]//g' | sed 's/  */ /g' | sed 's/^ //;s/ $//')
        [ -z "${safe_title}" ] && safe_title="Track${tn}"
        out_file="${output}/$(printf '%02d' ${tn}) ${safe_title}.flac"

        log "  Track ${tn}: ${title}..."
        extract_k2hd "${track_file}" | ffmpeg -y -f s24le -ar 96000 -ac 2 \
            -i pipe:0 -compression_level 8 \
            -metadata "artist=${ARTIST}" \
            -metadata "album=${album_name}" \
            -metadata "track=${tn}/${track_count}" \
            -metadata "title=${title}" \
            "${out_file}" 2>>"${LOG_FILE}"

        if [ $? -eq 0 ] && [ -f "${out_file}" ]; then
            log "    OK"
            inc_tracks
        else
            log "    ERROR"
            inc_errors
        fi
    done
  }

  process_raw() {
    local iso="$1" album_dir="$2" album_name="$3" output="$4" track_list="$5"
    local num_tracks base tn title safe_title out_file duration seg_dur temp_f
    parse_titles "${track_list}"
    num_tracks=${#TITLES[@]}
    [ "${num_tracks}" -eq 0 ] && num_tracks=20
    log "  Tracks: ${num_tracks}"

    local wav_tmp="${output}/_temp.wav"
    log "  Converting ISO to WAV..."
    ffmpeg -y -f s16le -ar 44100 -ac 2 -i "${iso}" "${wav_tmp}" 2>>"${LOG_FILE}"

    duration=$(ffprobe -v error -show_entries format=duration \
        -of default=noprint_wrappers=1:nokey=1 "${wav_tmp}" 2>/dev/null)
    seg_dur=$(python3 -c "print(${duration} / ${num_tracks})")
    log "  Duration: ${duration%.*}s — ${seg_dur%.*}s per track"

    local temp_pattern="${output}/%02d_temp.wav"
    ffmpeg -y -i "${wav_tmp}" -f segment -segment_time "${seg_dur}" \
        -c copy -reset_timestamps 1 "${temp_pattern}" 2>>"${LOG_FILE}"

    rm -f "${wav_tmp}"

    for temp_f in "${output}"/*_temp.wav; do
        [ -f "${temp_f}" ] || continue
        base=$(basename "${temp_f}" _temp.wav)
        tn=$((10#${base} + 1))
        [ ${tn} -gt ${num_tracks} ] && { rm -f "${temp_f}"; continue; }
        title="${TITLES[${tn}]:-Track ${tn}}"
        safe_title=$(echo "${title}" | sed 's/[\/:<>"|?*]//g' | sed 's/  */ /g' | sed 's/^ //;s/ $//')
        [ -z "${safe_title}" ] && safe_title="Track${tn}"
        out_file="${output}/$(printf '%02d' ${tn}) ${safe_title}.flac"

        log "  Track ${tn}: ${title}..."
        ffmpeg -y -i "${temp_f}" -compression_level 8 \
            -metadata "artist=${ARTIST}" \
            -metadata "album=${album_name}" \
            -metadata "track=${tn}/${num_tracks}" \
            -metadata "title=${title}" \
            "${out_file}" 2>>"${LOG_FILE}"

        rm -f "${temp_f}"
        if [ -f "${out_file}" ]; then
            log "    OK"
            inc_tracks
        else
            log "    ERROR"
            inc_errors
        fi
    done
  }

  # ── main ──
  local iso_file album_dir album_name album_output mount_point audio_dir _7z_tmp extract_dir
  local -a MOUNTED=()
  # Guaranteed cleanup of any attached ISO mounts (covers early exits/errors).
  detach_mounts() {
    local m
    for m in "${MOUNTED[@]:-}"; do
      hdiutil detach "${m}" 2>/dev/null || true
    done
  }
  trap detach_mounts EXIT

  while IFS= read -r -d '' iso_file; do
    album_dir="$(dirname "${iso_file}")"
    album_name="$(basename "${album_dir}")"
    album_output="${OUTPUT_BASE}/${album_name}"

    log ""
    log "--- ${album_name} ---"
    mkdir -p "${album_output}"

    # Single mount per ISO: the discovered audio dir is handed to process_k2hd,
    # which never re-mounts; detach here as soon as processing is done.
    mount_point=$(hdiutil attach -readonly "${iso_file}" 2>/dev/null | tail -1 | awk '{print $NF}' || true)
    if [ -n "${mount_point}" ] && [ -d "${mount_point}" ]; then
        MOUNTED+=("${mount_point}")
        audio_dir="${mount_point}/2C_AUDIO"
        [ ! -d "${audio_dir}" ] && audio_dir=$(find "${mount_point}" -type d -name "2C_AUDIO" 2>/dev/null | head -1)
        if [ -n "${audio_dir}" ]; then
            process_k2hd "${iso_file}" "${album_dir}" "${album_name}" "${album_output}" "${audio_dir}"
            hdiutil detach "${mount_point}" 2>/dev/null || true
            copy_images "${album_dir}" "${album_output}"
            inc_albums
            continue
        fi
        hdiutil detach "${mount_point}" 2>/dev/null || true
    fi

    log "  Checking for 2C_AUDIO/TRACK in ISO..."
    _7z_tmp=$(mktemp)
    if 7z l "${iso_file}" > "${_7z_tmp}" 2>&1 && grep -q '2C_AUDIO/TRACK' "${_7z_tmp}"; then
        rm -f "${_7z_tmp}"
        log "  Found 2C_AUDIO/TRACK - extracting with 7z"
        extract_dir=$(mktemp -d)
        log "  Extracting TRACK files with 7z..."
        7z x -o"${extract_dir}" "${iso_file}" '2C_AUDIO/TRACK*.2CH' >/dev/null 2>&1
        audio_dir="${extract_dir}/2C_AUDIO"
        if [ -d "${audio_dir}" ] && ls "${audio_dir}"/TRACK*.2CH >/dev/null 2>&1; then
            process_k2hd "${iso_file}" "${album_dir}" "${album_name}" "${album_output}" "${audio_dir}"
            rm -rf "${extract_dir}"
            copy_images "${album_dir}" "${album_output}"
            inc_albums
            continue
        fi
        rm -rf "${extract_dir}"
    else
        rm -f "${_7z_tmp}"
        log "  2C_AUDIO/TRACK not found - trying raw CD audio"
    fi

    if [ -f "${album_dir}/专辑曲目.txt" ]; then
        process_raw "${iso_file}" "${album_dir}" "${album_name}" "${album_output}" "${album_dir}/专辑曲目.txt"
        copy_images "${album_dir}" "${album_output}"
        inc_albums
    else
        log "  SKIP: no track list and not a standard K2HD ISO"
    fi
  done < <(find "${SOURCE_DIR}" -maxdepth 2 -name "*.iso" -print0 | sort -zV)

  log ""
  log "============================================================"
  log "Complete at $(date)"
  log "Albums: ${TOTAL_ALBUMS}  Tracks: ${TOTAL_TRACKS}  Errors: ${TOTAL_ERRORS}"
  log "Output: ${OUTPUT_BASE}"
  log "============================================================"
}

# ── unrar ────────────────────────────────────────────────────────────────────
# Extract all .rar files in a directory (or a specific file). unar primary,
# 7z fallback (handles RAR5, encrypted archives).
#   toolbox.sh unrar [options] [directory | --file RARFILE]
#   -f, --file F       Extract a single .rar file
#   -p, --password P   Password for encrypted RAR archives
#   -r, --recursive    Search recursively for .rar files
#   -o, --output-dir D Output directory (default: same dir as each .rar)
cmd_unrar() {
  local PASSWORD="" RECURSIVE=false OUTPUT_DIR="" DIR="" SINGLE_FILE=""

  while [[ $# -gt 0 ]]; do
    case "$1" in
      -f|--file)        shift; SINGLE_FILE="$1" ;;
      -p|--password)    shift; PASSWORD="$1" ;;
      -r|--recursive)   RECURSIVE=true ;;
      -o|--output-dir)  shift; OUTPUT_DIR="$1" ;;
      -h|--help)
        cat <<EOF
Usage: $(basename "$0") unrar [options] [directory | --file RARFILE]

Extract all .rar files in a directory (or a specific file). Uses unar
(primary) or 7z (fallback); handles RAR5 and encrypted archives.

Options:
  -f, --file F       Extract a single .rar file
  -p, --password P   Password for encrypted RAR archives
  -r, --recursive    Search recursively for .rar files
  -o, --output-dir D Output directory (default: same dir as each .rar)
  -h, --help         Show this help
EOF
        exit 0 ;;
      -*) echo "Error: unknown option $1" >&2; exit 1 ;;
      *)
        if [[ -z "$DIR" ]]; then DIR="$1"
        else echo "Error: unexpected argument $1" >&2; exit 1; fi
        ;;
    esac
    shift
  done

  local -a RAR_FILES=()
  if [[ -n "$SINGLE_FILE" ]]; then
    if [[ ! -f "$SINGLE_FILE" ]]; then
      echo "Error: file not found: $SINGLE_FILE" >&2
      exit 1
    fi
    RAR_FILES=("$SINGLE_FILE")
  else
    DIR="${DIR:-.}"
    if [[ ! -d "$DIR" ]]; then
      echo "Error: directory not found: $DIR" >&2
      exit 1
    fi
    if [[ "$RECURSIVE" == true ]]; then
      while IFS= read -r -d '' f; do RAR_FILES+=("$f"); done < <(find "$DIR" -type f -iname '*.rar' -print0 | sort -z)
    else
      while IFS= read -r -d '' f; do RAR_FILES+=("$f"); done < <(find "$DIR" -maxdepth 1 -type f -iname '*.rar' -print0 | sort -z)
    fi
    if [[ ${#RAR_FILES[@]} -eq 0 ]]; then
      echo "No .rar files found in $DIR" >&2
      exit 0
    fi
  fi

  if [[ -n "$SINGLE_FILE" ]]; then
    echo "Extracting: $SINGLE_FILE"
  else
    echo "Found ${#RAR_FILES[@]} .rar file(s) in $DIR"
  fi
  echo ""

  local EXTRACT_OK=0 EXTRACT_FAIL=0
  local rar BASENAME RAR_DIR TARGET_DIR EXTRACT_OUTPUT
  for rar in "${RAR_FILES[@]}"; do
    BASENAME="$(basename "$rar")"
    RAR_DIR="$(dirname "$rar")"

    if [[ -n "$OUTPUT_DIR" ]]; then
      TARGET_DIR="$OUTPUT_DIR"
    else
      TARGET_DIR="$RAR_DIR"
    fi
    mkdir -p "$TARGET_DIR"

    echo "━━━ Extracting: $BASENAME"

    EXTRACT_OUTPUT="$(mktemp)"

    local -a UNAR_CMD=(unar -q -f -o "$TARGET_DIR")
    if [[ -n "$PASSWORD" ]]; then
      UNAR_CMD+=(-p "$PASSWORD")
    fi
    UNAR_CMD+=("$rar")

    if "${UNAR_CMD[@]}" > "$EXTRACT_OUTPUT" 2>&1; then
      echo "  ✓ Done → $TARGET_DIR"
      rm -f "$EXTRACT_OUTPUT"
      EXTRACT_OK=$((EXTRACT_OK + 1))
      continue
    fi

    local -a SEVEN_CMD=(7z x -y)
    if [[ -n "$PASSWORD" ]]; then
      SEVEN_CMD+=("-p$PASSWORD")
    fi
    SEVEN_CMD+=("-o$TARGET_DIR" "$rar")

    if "${SEVEN_CMD[@]}" > "$EXTRACT_OUTPUT" 2>&1; then
      echo "  ✓ Done (via 7z) → $TARGET_DIR"
      rm -f "$EXTRACT_OUTPUT"
      EXTRACT_OK=$((EXTRACT_OK + 1))
    else
      grep -i 'error\|wrong\|unsupported\|cannot\|fail\|wrong password' "$EXTRACT_OUTPUT" 2>/dev/null \
        | head -10 | sed 's/^/  /'
      rm -f "$EXTRACT_OUTPUT"
      echo "  ✗ Failed"
      EXTRACT_FAIL=$((EXTRACT_FAIL + 1))
    fi
  done

  echo ""
  echo "━━━ Summary: ${EXTRACT_OK} succeeded, ${EXTRACT_FAIL} failed"

  if [[ "$EXTRACT_FAIL" -gt 0 ]]; then
    exit 1
  fi
}

# ── node tool delegates ──────────────────────────────────────────────────────

cmd_doctor() {
  # Run the doctor; when it saves a report, render the HTML corruption report
  # next to it (best-effort, non-fatal). The report path comes from the
  # doctor's own 'Report saved: <path>' line, so no arg parsing is duplicated.
  local tmp status report html
  tmp="$(mktemp)"
  set +e
  node "$SCRIPT_DIR/fix-flac-metadata.js" "$@" 2>&1 | tee "$tmp"
  status=${PIPESTATUS[0]}
  set -e
  report="$(sed -n 's/^Report saved: //p' "$tmp" | tail -1)"
  rm -f "$tmp"
  if [ -n "${report}" ] && [ -f "${report}" ]; then
    html="${report%.*}.html"
    if node "$SCRIPT_DIR/generate-corruption-report.js" "${report}" "${html}" >/dev/null 2>&1; then
      echo "HTML report: ${html}"
    else
      echo "⚠ could not render HTML report" >&2
    fi
  fi
  return "${status}"
}

cmd_corpus() {
  node "$SCRIPT_DIR/build-flac-test-corpus.js" "$@"
}

cmd_corruption_report() {
  node "$SCRIPT_DIR/generate-corruption-report.js" "$@"
}

cmd_aggregate_checkpoint() {
  node "$SCRIPT_DIR/aggregate-checkpoint.js" "$@"
}

# ── dispatch ─────────────────────────────────────────────────────────────────

COMMAND="${1:-}"
if [[ -z "$COMMAND" || "$COMMAND" == "-h" || "$COMMAND" == "--help" ]]; then
  toolbox_usage 0
fi
shift

case "$COMMAND" in
  cue-split)           cmd_cue_split "$@" ;;
  dsf-to-flac)         cmd_dsf_to_flac "$@" ;;
  slice-iso)           cmd_slice_iso "$@" ;;
  unrar)               cmd_unrar "$@" ;;
  doctor)              cmd_doctor "$@" ;;
  corpus)              cmd_corpus "$@" ;;
  corruption-report)   cmd_corruption_report "$@" ;;
  aggregate-checkpoint) cmd_aggregate_checkpoint "$@" ;;
  *)                   echo "Error: unknown command: $COMMAND" >&2; toolbox_usage 1 ;;
esac
