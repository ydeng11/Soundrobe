# Plan: WAV + CUE Slicer Script

## Context

A library of Chinese CD rips (陈小春 / 风火海 / etc.) lives at `/Volumes/downloads/陈小春`. Each album directory contains one or more WAV+CUE pairs. The goal is to create a script that:

1. Walks the library tree, finding every `.cue` + `.wav` pair
2. Parses the CUE sheet (GBK-encoded, with Chinese characters for titles/artists)
3. Slices the monolithic WAV into individual track WAV files
4. Inserts proper metadata (title, artist, album, genre, year, track number) into each output WAV

## Discovered Patterns

### Directory structure
- **Single-CD**: `.cue` + `.wav` directly in the album folder
  - e.g. `1994-风火海[香港首版][WAV]/风火海.-.[风火海](1994)[WAV].cue`
- **Multi-CD (2CD/3CD) — flat**: All `.cue` + `.wav` files in the album folder, named with CD1/CD2
  - e.g. `2008-绝对收藏 2CD[台湾首版][WAV]/陈小春.-.[绝对收藏 CD1](2008)[WAV].cue`
- **Multi-CD (2CD) — nested**: CD1/ and CD2/ subdirectories
  - e.g. `2000-精采完结篇全辑 2CD[台湾首版][WAV]/CD1/陈小春.-.[精采完结篇全辑 CD1](2000)[WAV].cue`

### CUE sheet encoding
- All CUE files are **GBK** encoded (confirmed via Python detection). Must decode as GBK to get correct Chinese characters.

### CUE format (key fields)
```
REM GENRE POP
REM DATE 1994
PERFORMER "风火海"          ← album artist
TITLE "风火海 [香港首版]"   ← album title
FILE "风火海.-.[风火海](1994)[WAV].wav" WAVE
  TRACK 01 AUDIO
    TITLE "热身"                      ← track title
    PERFORMER "风火海"                 ← track artist (can differ per track)
    FLAGS DCP                          ← optional, skip
    INDEX 01 00:00:00                  ← actual start position (mm:ss:ff)
  TRACK 02 AUDIO
    INDEX 00 00:57:00                  ← pregap start (silent lead-in)
    INDEX 01 00:58:00                  ← track start
```

Key handling rules:
- `INDEX 01` = the actual start of the track → use for slicing
- `INDEX 00` = pregap start (silent lead-in) → **ignore**; slice starts at INDEX 01
- `FLAGS DCP` appears on some tracks → skip it
- Time format is `mm:ss:ff` (minutes:seconds:frames) where 1 frame = 1/75 second at CDDA (44.1kHz)

### Available tools
- **Python 3.14** (stdlib only besides mutagen)
- **ffmpeg** (for slicing WAV by timestamp)
- **mutagen** (Python library, installed locally at `~/.local/lib/python3.14/site-packages/`) for writing WAV metadata
- No `sox`, no `shnsplit`, no `cuetools` — will rely on ffmpeg + mutagen

### Metadata mapping

| CUE field       | WAV metadata (INFO chunk via mutagen) |
|-----------------|---------------------------------------|
| REM GENRE       | `genre`                               |
| REM DATE        | `date`                                |
| PERFORMER (top) | `albumartist` or `artist` (if no track performer) |
| TITLE (top)     | `album`                                |
| TRACK TITLE     | `title`                                |
| TRACK PERFORMER | `artist` (overrides album artist)      |
| Track number    | `tracknumber`                          |

## Files to create

- `/Users/ihelio/code/cue-slicer/slice_cue.py` — main script

## Approach

1. **Scan**: Walk the library directory recursively, find every `.cue` file, verify a matching `.wav` file exists (same directory + same filename stem, or the filename referenced inside the CUE's `FILE` directive).
2. **Parse CUE**: Read raw bytes, decode as GBK (fallback: utf-8, latin-1). Extract:
   - REM GENRE, REM DATE
   - PERFORMER (album-level)
   - TITLE (album-level)
   - FILE name
   - For each TRACK: TRACK number, TITLE, PERFORMER, INDEX 01 position
3. **Slice**: For each track, use `ffmpeg -i input.wav -ss <start> -to <end> -c copy output.wav`
   - Start = INDEX 01 time of current track
   - End = INDEX 01 time of next track (or end of file for last track)
4. **Write metadata**: Use mutagen's `Wave` class to write INFO tags (title, artist, album, genre, date, tracknumber, tracktotal)
5. **Output naming**: `{tracknum:02d}. {title}.wav` placed in a `sliced/` subdirectory within each album folder (or user-specified output dir)

## Steps

- [ ] Step 1: Create the script skeleton with argument parsing (input directory, optional output directory, dry-run flag)
- [ ] Step 2: Implement CUE scanner — walk the tree, find `.cue` files, verify paired `.wav`
- [ ] Step 3: Implement CUE parser — read file, auto-detect encoding (prefer GBK), extract all metadata fields per track
- [ ] Step 4: Implement time conversion — parse `mm:ss:ff` → seconds (float), calculate track durations
- [ ] Step 5: Implement slicing — use `subprocess` + `ffmpeg` to cut WAV segments
- [ ] Step 6: Implement metadata writing — use `mutagen.wave.Wave` to set INFO tags on each sliced file
- [ ] Step 7: Add dry-run mode that prints what would be done without executing
- [ ] Step 8: Test on the real library at `/Volumes/downloads/陈小春` — run against a single album first, then full library

## Verification

1. Run `python3 slice_cue.py /Volumes/downloads/陈小春/1994-风火海[香港首版][WAV] --dry-run` — verify correct tracks detected
2. Run without dry-run on a single album — check output WAV files exist with correct durations and metadata
3. Use `ffprobe` or `mutagen-inspect` to verify metadata on output files
4. Run on the full library — check all albums processed without errors
