#!/usr/bin/env bash
#
# cue2flac.sh — Split WAV/FLAC+CUE album images into per-track FLAC files.
#
# Usage:
#   ./cue2flac.sh /path/to/album          # single album folder
#   ./cue2flac.sh /path/to/artist-root    # recurse into subfolders
#   ./cue2flac.sh -r /path/to/artist-root # same as above
#
# Requirements: cuebreakpoints (cuetools), ffmpeg, python3
#
set -euo pipefail

# ── parse args ───────────────────────────────────────────────────────
RECURSIVE=false
DRY_RUN=false
ARTIST_MODE=false
OUTPUT_SUFFIX="-tracks"

usage() {
  cat <<EOF
Usage: $(basename "$0") [OPTIONS] <path>

Options:
  -r, --recursive   Recurse into subdirectories to find .cue files
  -n, --dry-run     Show what would be done without doing it
  -a, --artist      Artist mode: rename source folder to _old, output to artist/album/
  -h, --help        Show this help
EOF
  exit 0
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    -r|--recursive) RECURSIVE=true; shift ;;
    -n|--dry-run)   DRY_RUN=true; shift ;;
    -a|--artist)    ARTIST_MODE=true; shift ;;
    -h|--help)      usage ;;
    -*)             echo "Unknown option: $1" >&2; exit 1 ;;
    *)              TARGET="$1"; shift ;;
  esac
done

if [[ -z "${TARGET:-}" ]]; then
  echo "Error: no path given." >&2; usage
fi
if ! [[ -d "$TARGET" ]]; then
  echo "Error: '$TARGET' is not a directory." >&2; exit 1
fi

for cmd in cuebreakpoints ffmpeg ffprobe python3; do
  if ! command -v "$cmd" &>/dev/null; then
    echo "Error: '$cmd' not found." >&2; exit 1
  fi
done

# ── process each CUE via python helper ───────────────────────────────
MAX_DEPTH=5
if ! $RECURSIVE; then MAX_DEPTH=3; fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

python3 - "$TARGET" "$MAX_DEPTH" "$DRY_RUN" "$OUTPUT_SUFFIX" "$SCRIPT_DIR" "$ARTIST_MODE" <<'PYEOF'
import os, sys, subprocess, tempfile, shutil

TARGET = sys.argv[1]
MAX_DEPTH = int(sys.argv[2])
DRY_RUN = sys.argv[3] == "true"
OUTPUT_SUFFIX = sys.argv[4]
SCRIPT_DIR = sys.argv[5]
ARTIST_MODE = sys.argv[6] == "true"

def read_cue(path):
    """Read CUE file, auto-detect encoding, return UTF-8 string."""
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
    """Convert MM:SS.ff to seconds."""
    parts = msf.replace(',', '.').split(':')
    if len(parts) == 3:
        m, s, f = parts
        return float(m) * 60 + float(s) + float(f) / 100
    elif len(parts) == 2:
        m, s = parts
        return float(m) * 60 + float(s)
    return float(parts[0])

def find_cue_files(target, max_depth):
    """Find .cue files up to max_depth."""
    results = []
    for root, dirs, files in os.walk(target):
        depth = root[len(target):].count(os.sep)
        if depth >= max_depth:
            dirs[:] = []
            continue
        for f in files:
            if f.lower().endswith('.cue'):
                results.append(os.path.join(root, f))
    return sorted(results)

def find_source_file(cue_dir):
    """Find matching WAV or FLAC file in the same directory."""
    exts = ('.wav', '.flac')
    files = [f for f in os.listdir(cue_dir)
             if os.path.isfile(os.path.join(cue_dir, f))
             and f.lower().endswith(exts)]
    if len(files) == 1:
        return os.path.join(cue_dir, files[0])
    return None

def process_cue(cue_path):
    cue_dir = os.path.dirname(cue_path)
    cue_base = os.path.splitext(os.path.basename(cue_path))[0]

    # Read and parse CUE
    cue_text = read_cue(cue_path)

    # Find source file
    # First try FILE directive
    src_path = None
    for line in cue_text.split('\n'):
        line_stripped = line.strip()
        if line_stripped.upper().startswith('FILE'):
            # FILE "filename" TYPE
            parts = line_stripped.split('"', 2)
            if len(parts) >= 2:
                fname = parts[1]
                candidate = os.path.join(cue_dir, fname)
                if os.path.isfile(candidate):
                    src_path = candidate
                    break

    if not src_path:
        src_path = find_source_file(cue_dir)

    if not src_path:
        print(f"⚠  SKIP: No .wav/.flac file found in '{cue_dir}'")
        return

    # Output directory
    album_parent = os.path.dirname(cue_dir)
    album_name = os.path.basename(cue_dir)

    if ARTIST_MODE and TARGET_PROCESSED:
        # Artist mode: output to artist-processed/album_name/
        # Find the relative path from TARGET_ARTIST to cue_dir
        rel_path = os.path.relpath(cue_dir, TARGET_ARTIST)
        out_dir = os.path.join(TARGET_PROCESSED, rel_path)

        # Ensure output dir exists
        os.makedirs(out_dir, exist_ok=True)
    else:
        # Default: output at parent level with -tracks suffix
        out_dir = os.path.join(album_parent, cue_base + OUTPUT_SUFFIX)

    if os.path.isdir(out_dir):
        # Check for split FLACs (numbered files like 01. xxx.flac)
        flacs = [f for f in os.listdir(out_dir) if f.endswith('.flac') and f[:2].isdigit()]
        if flacs:
            print(f"⏭  SKIP: Output dir already has FLAC files: '{out_dir}'")
            return

    # Extract breakpoints
    try:
        result = subprocess.run(['cuebreakpoints', cue_path],
                              capture_output=True, text=True, timeout=30)
        breakpoints_text = result.stdout.strip()
    except Exception:
        breakpoints_text = ""

    if not breakpoints_text:
        print(f"⚠  SKIP: No track breakpoints in '{cue_path}'")
        return

    breakpoints = [bp.strip() for bp in breakpoints_text.split('\n') if bp.strip()]
    total_tracks = len(breakpoints) + 1

    # Parse metadata
    performer = ""
    album_title = ""
    track_titles = []
    title_count = 0
    for line in cue_text.split('\n'):
        ls = line.strip()
        ls_upper = ls.upper()
        if ls_upper.startswith('PERFORMER') and not performer:
            parts = ls.split('"', 2)
            if len(parts) >= 2:
                performer = parts[1]
        elif ls_upper.startswith('TITLE'):
            parts = ls.split('"', 2)
            if len(parts) >= 2:
                title_count += 1
                if title_count == 1:
                    album_title = parts[1]
                else:
                    track_titles.append(parts[1])

    # Build start times
    start_times = [0.0]
    for bp in breakpoints:
        start_times.append(msf_to_seconds(bp))

    # Print info
    src_name = os.path.basename(src_path)
    print("━" * 55)
    print(f"Album:   {cue_base}")
    print(f"Source:  {src_name}")
    print(f"Tracks:  {total_tracks}")
    print(f"Output:  {out_dir}")
    print("━" * 55)

    if DRY_RUN:
        print(f"  [dry-run] Would split '{src_path}' into {total_tracks} FLAC tracks")
        return

    os.makedirs(out_dir, exist_ok=True)

    # Copy image files from source directory to output
    image_exts = ('.jpg', '.jpeg', '.png', '.gif', '.bmp', '.webp')
    for item in os.listdir(cue_dir):
        if item.lower().endswith(image_exts):
            src_img = os.path.join(cue_dir, item)
            dst_img = os.path.join(out_dir, item)
            if os.path.isfile(src_img) and not os.path.exists(dst_img):
                shutil.copy2(src_img, dst_img)

    # Split each track
    for i in range(len(start_times)):
        start = start_times[i]
        end = start_times[i + 1] if i + 1 < len(start_times) else None
        track_num = i + 1
        track_pad = f"{track_num:02d}"

        # Get track title
        track_title = track_titles[i] if i < len(track_titles) else ""

        # Build output filename
        clean_title = track_title.lstrip('0123456789. -')
        if clean_title:
            out_file = os.path.join(out_dir, f"{track_pad}. {clean_title}.flac")
        else:
            out_file = os.path.join(out_dir, f"{track_pad}.flac")

        # Build ffmpeg command
        cmd = ['ffmpeg', '-y', '-i', src_path]
        if end is not None:
            cmd += ['-ss', str(start), '-to', str(end)]
        else:
            cmd += ['-ss', str(start)]
        cmd += ['-c:a', 'flac']

        if performer:
            cmd += ['-metadata', f'artist={performer}']
        if album_title:
            cmd += ['-metadata', f'album={album_title}']
        if track_title:
            cmd += ['-metadata', f'title={track_title}']
        cmd += ['-metadata', f'track={track_num}']
        cmd.append(out_file)

        end_str = f"{end}" if end is not None else "end"
        title_str = f" [{track_title}]" if track_title else ""
        print(f"  Track {track_pad}: {start} → {end_str}{title_str}... ", end='', flush=True)

        try:
            subprocess.run(cmd, capture_output=True, timeout=600, check=True)
            # Get duration
            try:
                dur_result = subprocess.run(
                    ['ffprobe', '-v', 'error', '-show_entries', 'format=duration',
                     '-of', 'csv=p=0', out_file],
                    capture_output=True, text=True, timeout=10)
                duration = float(dur_result.stdout.strip()) if dur_result.stdout.strip() else 0
                print(f"({duration:.0f}s)")
            except Exception:
                print("(done)")
        except subprocess.CalledProcessError as e:
            print("FAILED")
            if e.stderr:
                # Show last few lines of error
                err_lines = e.stderr.decode('utf-8', errors='replace').strip().split('\n')
                for line in err_lines[-3:]:
                    if line.strip():
                        print(f"    Error: {line.strip()}")
            shutil.rmtree(out_dir, ignore_errors=True)
            return

    # Count output files
    flac_count = len([f for f in os.listdir(out_dir) if f.endswith('.flac')])
    print(f"✅  Done: {flac_count} tracks → {out_dir}")

    # Health check
    doctor = os.path.join(SCRIPT_DIR, "..", "auto_tagger", "scripts", "fix-flac-metadata.js")
    if os.path.isfile(doctor) and os.access(doctor, os.X_OK):
        print()
        print("  🏥 Running health check...")
        try:
            result = subprocess.run(['node', doctor, out_dir, '--doctor'],
                                  capture_output=True, text=True, timeout=60)
            print(result.stdout)
        except Exception:
            pass

# ── main ─────────────────────────────────────────────────────────────
# In artist mode, determine if TARGET is artist folder or album folder
# and create -processed output folder
if ARTIST_MODE:
    # Check if TARGET contains album subfolders (artist folder) or is an album folder
    has_albums = False
    for item in os.listdir(TARGET):
        item_path = os.path.join(TARGET, item)
        # Skip -tracks subfolders and -processed folder
        if os.path.isdir(item_path) and not item.endswith('-tracks') and not item.endswith('-processed'):
            for f in os.listdir(item_path):
                if f.lower().endswith('.cue'):
                    has_albums = True
                    break
        if has_albums:
            break

    if has_albums:
        # TARGET is an artist folder (has album subfolders with CUE files)
        artist_path = TARGET
    else:
        # TARGET is an album folder, use parent as artist
        artist_path = os.path.dirname(TARGET)

    artist_name = os.path.basename(artist_path)
    PROCESSED_DIR = artist_path + "-processed"

    # Create -processed directory
    if not os.path.exists(PROCESSED_DIR):
        if not DRY_RUN:
            os.makedirs(PROCESSED_DIR, exist_ok=True)
            print(f"📁 Created: {artist_name}-processed")
        else:
            print(f"[dry-run] Would create: {artist_name}-processed")

    # Store for use in process_cue
    TARGET_ARTIST = artist_path
    TARGET_PROCESSED = PROCESSED_DIR
else:
    TARGET_ARTIST = None
    TARGET_PROCESSED = None

if ARTIST_MODE:
    # In artist mode, look for CUE files in original TARGET
    # (we don't rename, we create -processed folder)
    cue_files = find_cue_files(TARGET, MAX_DEPTH)
    if cue_files:
        print(f"Found CUE file(s).")
    else:
        print(f"No .cue files found.", file=sys.stderr)
        sys.exit(1)
else:
    cue_files = find_cue_files(TARGET, MAX_DEPTH)
    if not cue_files:
        print(f"No .cue files found in '{TARGET}'.", file=sys.stderr)
        sys.exit(1)
    print(f"Found CUE file(s).")
print()

for cue in cue_files:
    process_cue(cue)

print()
print("═" * 55)
print("Done.")
print("═" * 55)
PYEOF
