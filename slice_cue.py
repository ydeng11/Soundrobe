#!/usr/bin/env python3
"""
WAV + CUE Sheet Slicer

Walks a library of CD rips, finds .cue + .wav pairs, parses the CUE sheet,
slices the monolithic WAV into individual track files (WAV), and writes
metadata from the CUE sheet into each output file.

Usage:
    python3 slice_cue.py /path/to/library                  # process everything
    python3 slice_cue.py /path/to/album --dry-run          # preview only
    python3 slice_cue.py /path/to/album --output ./out      # custom output root
    python3 slice_cue.py /path/to/album --format flac       # output as FLAC
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

# ---------------------------------------------------------------------------
# Attempt to import mutagen (optional; needed only when not in dry-run mode)
# ---------------------------------------------------------------------------
HAS_MUTAGEN = False
try:
    from mutagen.wave import WAVE
    from mutagen.id3 import ID3, TIT2, TPE1, TALB, TCON, TDRC, TRCK, COMM, error as ID3Error
    HAS_MUTAGEN = True
except ImportError:
    pass


# ===================================================================
#  Utilities
# ===================================================================

def frames_to_seconds(mm: int, ss: int, ff: int) -> float:
    """Convert CUE mm:ss:ff to seconds (1 frame = 1/75 s at CDDA)."""
    return mm * 60.0 + ss + ff / 75.0


def parse_timestamp(ts: str) -> tuple[int, int, int]:
    """Parse 'MM:SS:FF' into (mm, ss, ff)."""
    parts = ts.strip().split(":")
    if len(parts) != 3:
        raise ValueError(f"Invalid timestamp format: {ts!r}")
    return int(parts[0]), int(parts[1]), int(parts[2])


def sanitize_filename(name: str) -> str:
    """Remove or replace characters that are problematic in filenames."""
    name = name.replace("/", "／").replace("\\", "＼")
    name = name.replace(":", "：").replace("*", "＊")
    name = name.replace("?", "？").replace('"', "＂")
    name = name.replace("<", "＜").replace(">", "＞")
    name = name.replace("|", "｜")
    name = re.sub(r"\s+", " ", name).strip()
    return name


# ===================================================================
#  CUE Encoding detection
# ===================================================================

def decode_cue(raw: bytes) -> str:
    """Try to decode CUE bytes as GBK first, fallback to common encodings."""
    for enc in ("gbk", "gb2312", "gb18030", "utf-8", "latin-1", "big5"):
        try:
            text = raw.decode(enc)
            # If latin-1 works but has too many replacement chars, skip
            if enc == "latin-1" and text.count("�") > 5:
                continue
            return text
        except (UnicodeDecodeError, LookupError):
            continue
    return raw.decode("latin-1")


# ===================================================================
#  CUE Parser
# ===================================================================

class CueTrack:
    """Represents a single TRACK entry in a CUE sheet."""

    def __init__(self):
        self.number: int = 0
        self.title: str = ""
        self.performer: str = ""
        self.index01: float = 0.0   # seconds; the actual start position
        self.index00: float | None = None  # pregap start, if present

    def __repr__(self) -> str:
        return (
            f"<CueTrack #{self.number} {self.title!r} "
            f"artist={self.performer!r} start={self.index01:.3f}s>"
        )


class CueSheet:
    """Represents a parsed CUE sheet."""

    def __init__(self):
        self.album_artist: str = ""
        self.album_title: str = ""
        self.genre: str = ""
        self.date: str = ""
        self.file_ref: str = ""       # filename referenced in FILE directive
        self.tracks: list[CueTrack] = []

    def __repr__(self) -> str:
        return (
            f"<CueSheet album={self.album_title!r} artist={self.album_artist!r} "
            f"tracks={len(self.tracks)}>"
        )


def parse_cue(cue_text: str) -> CueSheet:
    """Parse a CUE sheet string into a CueSheet object."""
    sheet = CueSheet()
    current_track: CueTrack | None = None

    # State tracking — track-level vs album-level TITLE/PERFORMER
    in_track_block = False

    # Patterns
    re_performer = re.compile(r'^\s*PERFORMER\s+"(.+)"\s*$')
    re_title = re.compile(r'^\s*TITLE\s+"(.+)"\s*$')
    re_file = re.compile(r'^\s*FILE\s+"(.+)"\s+WAVE\s*$')
    re_track = re.compile(r'^\s*TRACK\s+(\d+)\s+AUDIO\s*$')
    re_index = re.compile(r'^\s*INDEX\s+(\d+)\s+(\d+:\d+:\d+)\s*$')

    for line in cue_text.splitlines():
        orig = line
        line = line.strip()

        # Skip empty lines
        if not line:
            continue

        # --- REM lines ---
        if line.startswith("REM "):
            rest = line[4:].strip()
            if rest.upper().startswith("GENRE "):
                sheet.genre = rest[6:].strip().strip('"')
            elif rest.upper().startswith("DATE "):
                sheet.date = rest[5:].strip().strip('"')
            continue

        # --- Track boundary ---
        m = re_track.match(line)
        if m:
            # Finalize previous track
            if current_track is not None:
                sheet.tracks.append(current_track)
            current_track = CueTrack()
            current_track.number = int(m.group(1))
            in_track_block = True
            continue

        # --- Track-level or Album-level fields ---
        m = re_performer.match(line)
        if m:
            if current_track is not None:
                current_track.performer = m.group(1)
            else:
                sheet.album_artist = m.group(1)
            continue

        m = re_title.match(line)
        if m:
            if current_track is not None:
                current_track.title = m.group(1)
            else:
                sheet.album_title = m.group(1)
            continue

        m = re_file.match(line)
        if m:
            sheet.file_ref = m.group(1)
            # FILE directive often resets track context
            continue

        # --- INDEX lines ---
        m = re_index.match(line)
        if m and current_track is not None:
            idx_num = int(m.group(1))
            mm, ss, ff = parse_timestamp(m.group(2))
            seconds = frames_to_seconds(mm, ss, ff)
            if idx_num == 1:
                current_track.index01 = seconds
            elif idx_num == 0:
                current_track.index00 = seconds
            continue

        # FLAGS, ISRC, etc. — silently skip
    # Finalize last track
    if current_track is not None:
        sheet.tracks.append(current_track)

    return sheet


# ===================================================================
#  Scanner: find .cue + .wav pairs in a directory tree
# ===================================================================

def find_cue_wav_pairs(root_dir: Path) -> list[tuple[Path, str]]:
    """
    Walk the directory tree and return a list of (cue_path, wav_path_string)
    where each CUE file has a matching WAV file.

    Matching logic:
    1. Find the WAV filename referenced inside the CUE's FILE directive.
    2. Look for that relative filename in the same directory as the CUE.
    3. If not found, fall back to finding any .wav in the same directory
       with the same stem as the .cue file.
    4. Last resort: any .wav in the same directory.
    """
    pairs: list[tuple[Path, str]] = []
    cue_files = sorted(root_dir.rglob("*.cue"))

    for cue_path in cue_files:
        cue_dir = cue_path.parent

        # Read and decode CUE
        try:
            raw = cue_path.read_bytes()
            cue_text = decode_cue(raw)
        except Exception as exc:
            print(f"  [WARN] Cannot read {cue_path}: {exc}", file=sys.stderr)
            continue

        # Parse minimally to get file_ref
        sheet = parse_cue(cue_text)
        wav_path: Path | None = None

        # Strategy 1: Try the FILE reference from CUE
        if sheet.file_ref:
            candidate = cue_dir / sheet.file_ref
            if candidate.is_file():
                wav_path = candidate

        # Strategy 2: Same stem as CUE
        if wav_path is None:
            stem = cue_path.stem
            candidates = list(cue_dir.glob(f"{stem}.wav"))
            if candidates:
                wav_path = candidates[0]

        # Strategy 3: Any WAV in the same directory
        if wav_path is None:
            candidates = list(cue_dir.glob("*.wav"))
            if candidates:
                wav_path = candidates[0]

        if wav_path is None:
            print(f"  [WARN] No matching WAV for {cue_path}", file=sys.stderr)
            continue

        pairs.append((cue_path, str(wav_path)))

    return pairs


# ===================================================================
#  WAV slicing via ffmpeg
# ===================================================================

def slice_track(
    wav_path: str,
    output_path: str,
    start_sec: float,
    duration_sec: float,
    output_format: str = "wav",
    dry_run: bool = False,
) -> bool:
    """
    Slice a segment from the source WAV using ffmpeg.

    For WAV: uses -c copy for lossless stream copy.
    For FLAC: uses -c:a flac for lossless encoding.
    Returns True on success.
    """
    if duration_sec <= 0:
        print(f"    [SKIP] Track with zero/negative duration ({duration_sec:.3f}s)")
        return False

    if output_format == "flac":
        codec = ["-c:a", "flac"]
    else:
        codec = ["-c", "copy"]

    cmd = [
        "ffmpeg",
        "-y",
        "-i", wav_path,
        "-ss", format_time_ffmpeg(start_sec),
        "-t", format_time_ffmpeg(duration_sec),
    ] + codec + [
        "-avoid_negative_ts", "make_zero",
        output_path,
    ]

    if dry_run:
        print(f"    [DRY-RUN] ffmpeg -i {Path(wav_path).name} ... -> {Path(output_path).name}")
        return True

    print(f"    Slicing {Path(output_path).name} ...", end=" ", flush=True)
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print("FAILED")
        print(f"      stderr: {result.stderr[:500]}", file=sys.stderr)
        return False
    print("OK")
    return True


def format_time_ffmpeg(seconds: float) -> str:
    """Format seconds as HH:MM:SS.mmm for ffmpeg."""
    hours = int(seconds // 3600)
    minutes = int((seconds % 3600) // 60)
    secs = seconds % 60
    return f"{hours:02d}:{minutes:02d}:{secs:06.3f}"


# ===================================================================
#  Metadata writing via mutagen
# ===================================================================

def write_flac_metadata(
    flac_path: str,
    track: CueTrack,
    sheet: CueSheet,
    total_tracks: int,
    dry_run: bool = False,
) -> bool:
    """
    Write CUE-derived metadata into a FLAC file using mutagen.
    Uses FLAC's native Vorbis comments.
    """
    if not HAS_MUTAGEN:
        if not dry_run:
            print(
                "    [WARN] mutagen not available -- skipping metadata",
                file=sys.stderr,
            )
        return False

    try:
        from mutagen.flac import FLAC
    except ImportError:
        print("    [WARN] mutagen.flac not available -- skipping metadata", file=sys.stderr)
        return False

    if dry_run:
        artist = track.performer if track.performer else sheet.album_artist
        print(
            f"    [DRY-RUN] FLAC metadata: title={track.title!r} "
            f"artist={artist!r} album={sheet.album_title!r} "
            f"track={track.number}/{total_tracks}"
        )
        return True

    try:
        audio = FLAC(flac_path)
    except Exception as exc:
        print(f"    [WARN] Cannot open {flac_path} for metadata: {exc}", file=sys.stderr)
        return False

    # Map CUE fields to Vorbis comments
    if track.title:
        audio["title"] = track.title
    artist = track.performer if track.performer else sheet.album_artist
    if artist:
        audio["artist"] = artist
    if sheet.album_title:
        audio["album"] = sheet.album_title
    if sheet.album_artist:
        audio["albumartist"] = sheet.album_artist
    if sheet.genre:
        audio["genre"] = sheet.genre
    if sheet.date:
        audio["date"] = sheet.date
    track_str = f"{track.number:02d}"
    if total_tracks:
        track_str += f"/{total_tracks:02d}"
    audio["tracknumber"] = track_str

    try:
        audio.save()
    except Exception as exc:
        print(f"    [WARN] Failed to save metadata to {flac_path}: {exc}", file=sys.stderr)
        return False

    return True


def write_metadata(
    wav_path: str,
    track: CueTrack,
    sheet: CueSheet,
    total_tracks: int,
    dry_run: bool = False,
) -> bool:
    """
    Write CUE-derived metadata into a WAV file using mutagen.
    Uses ID3v2 tags embedded in the WAV.
    """
    if not HAS_MUTAGEN:
        if not dry_run:
            print(
                "    [WARN] mutagen not available -- skipping metadata",
                file=sys.stderr,
            )
        return False

    if dry_run:
        artist = track.performer if track.performer else sheet.album_artist
        print(
            f"    [DRY-RUN] Metadata: title={track.title!r} "
            f"artist={artist!r} album={sheet.album_title!r} "
            f"track={track.number}/{total_tracks}"
        )
        return True

    from mutagen.wave import _WaveID3

    try:
        audio = WAVE(wav_path)
    except Exception as exc:
        print(f"    [WARN] Cannot open {wav_path} for metadata: {exc}", file=sys.stderr)
        return False

    # Get or create proper _WaveID3 tags (not bare ID3)
    tags = audio.tags
    if tags is None:
        audio.add_tags()
        tags = audio.tags

    if tags is None:
        print(f"    [WARN] Failed to create ID3 tags for {wav_path}", file=sys.stderr)
        return False

    # Map CUE fields to ID3 tags
    if track.title:
        tags.add(TIT2(encoding=3, text=track.title))
    artist = track.performer if track.performer else sheet.album_artist
    if artist:
        tags.add(TPE1(encoding=3, text=artist))
    if sheet.album_title:
        tags.add(TALB(encoding=3, text=sheet.album_title))
    if sheet.genre:
        tags.add(TCON(encoding=3, text=sheet.genre))
    if sheet.date:
        tags.add(TDRC(encoding=3, text=sheet.date))
    track_str = f"{track.number:02d}"
    if total_tracks:
        track_str += f"/{total_tracks:02d}"
    tags.add(TRCK(encoding=3, text=track_str))

    try:
        audio.save()
    except Exception as exc:
        print(f"    [WARN] Failed to save metadata to {wav_path}: {exc}", file=sys.stderr)
        return False

    return True


# ===================================================================
#  Per-album processor
# ===================================================================

def process_album(
    cue_path: Path,
    wav_path: str,
    input_root: Path,
    output_root: str | None,
    output_format: str,
    dry_run: bool = False,
) -> bool:
    """
    Process a single CUE+WAV pair: parse, slice, tag.
    Returns True if all tracks succeeded.
    """
    # Read & decode CUE
    try:
        raw = cue_path.read_bytes()
        cue_text = decode_cue(raw)
    except Exception as exc:
        print(f"  [ERROR] Cannot read CUE {cue_path}: {exc}", file=sys.stderr)
        return False

    # Parse
    sheet = parse_cue(cue_text)
    if not sheet.tracks:
        print(f"  [WARN] No tracks found in {cue_path.name}", file=sys.stderr)
        return False

    # Determine output directory relative to the input root
    if output_root:
        try:
            rel = cue_path.parent.relative_to(input_root)
        except ValueError:
            # Fallback: use parent dir name only
            rel = cue_path.parent.name
        out_dir = Path(output_root) / rel
    else:
        out_dir = cue_path.parent / "sliced"

    out_dir.mkdir(parents=True, exist_ok=True)

    # Use album artist as default performer if track performer is empty
    for t in sheet.tracks:
        if not t.performer:
            t.performer = sheet.album_artist

    total_tracks = len(sheet.tracks)
    success_count = 0

    print(f"\n{'='*60}")
    print(f"Album: {sheet.album_title}")
    print(f"Artist: {sheet.album_artist}")
    print(f"Year: {sheet.date}  Genre: {sheet.genre}")
    print(f"Tracks: {total_tracks}")
    print(f"Output: {out_dir}")
    print(f"{'='*60}")

    for i, track in enumerate(sheet.tracks):
        # Determine start and end times
        start = track.index01
        if i + 1 < len(sheet.tracks):
            end = sheet.tracks[i + 1].index01
        else:
            end = get_wav_duration(wav_path)

        duration = end - start

        # Build output filename
        safe_title = sanitize_filename(track.title)
        out_name = f"{track.number:02d}. {safe_title}.{output_format}"
        out_path = out_dir / out_name

        # Slice
        ok = slice_track(wav_path, str(out_path), start, duration, output_format, dry_run)
        if not ok:
            continue

        # Write metadata (WAV uses ID3; FLAC uses Vorbis comments)
        if output_format == "wav":
            write_metadata(str(out_path), track, sheet, total_tracks, dry_run)
        elif output_format == "flac":
            write_flac_metadata(str(out_path), track, sheet, total_tracks, dry_run)

        success_count += 1

    print(f"  Result: {success_count}/{total_tracks} tracks processed")
    return success_count > 0


# ===================================================================
#  Get WAV duration via ffprobe
# ===================================================================

def get_wav_duration(wav_path: str) -> float:
    """Use ffprobe to get the total duration of a WAV file in seconds."""
    try:
        result = subprocess.run(
            [
                "ffprobe",
                "-v", "quiet",
                "-print_format", "json",
                "-show_format",
                wav_path,
            ],
            capture_output=True,
            text=True,
        )
        data = json.loads(result.stdout)
        return float(data["format"]["duration"])
    except Exception as exc:
        print(f"  [WARN] Could not determine duration of {wav_path}: {exc}", file=sys.stderr)
        return 0.0


# ===================================================================
#  Main entry point
# ===================================================================

def main():
    parser = argparse.ArgumentParser(
        description="Slice WAV files using CUE sheets and insert metadata."
    )
    parser.add_argument(
        "input_dir",
        help="Root directory of the music library (or a single album directory)",
    )
    parser.add_argument(
        "--output", "-o",
        default=None,
        help="Output root directory (default: <album>/sliced/)",
    )
    parser.add_argument(
        "--format", "-f",
        choices=["wav", "flac"],
        default="wav",
        help="Output format (default: wav).",
    )
    parser.add_argument(
        "--dry-run", "-n",
        action="store_true",
        help="Preview what would be done without actually slicing",
    )

    args = parser.parse_args()

    # Validate input
    input_dir = Path(args.input_dir)
    if not input_dir.is_dir():
        print(f"Error: {input_dir} is not a valid directory", file=sys.stderr)
        sys.exit(1)

    # Check dependencies
    has_ffmpeg = subprocess.run(["which", "ffmpeg"], capture_output=True).returncode == 0
    if not has_ffmpeg:
        print("Error: ffmpeg not found in PATH.", file=sys.stderr)
        sys.exit(1)

    if not args.dry_run and not HAS_MUTAGEN and args.format == "wav":
        print(
            "Warning: mutagen Python library not found. Metadata will NOT be written.\n"
            "  Install via:  pip3 install --user mutagen",
            file=sys.stderr,
        )

    # Find all CUE+WAV pairs
    print(f"Scanning {input_dir} for CUE/WAV pairs...")
    pairs = find_cue_wav_pairs(input_dir)
    print(f"Found {len(pairs)} CUE/WAV pair(s).")

    if not pairs:
        print("Nothing to process.")
        sys.exit(0)

    if args.dry_run:
        print("\n=== DRY RUN -- no files will be modified ===\n")

    # Process each pair
    processed = 0
    failed = 0
    for cue_path, wav_path in pairs:
        try:
            ok = process_album(
                cue_path, wav_path,
                input_dir,
                args.output, args.format,
                args.dry_run,
            )
            if ok:
                processed += 1
            else:
                failed += 1
        except Exception as exc:
            print(f"  [ERROR] Processing {cue_path}: {exc}", file=sys.stderr)
            failed += 1

    # Summary
    print(f"\n{'='*60}")
    print(f"Done: {processed} album(s) processed, {failed} failed")
    if args.dry_run:
        print("(dry-run -- no files were modified)")
    print(f"{'='*60}")


if __name__ == "__main__":
    main()
