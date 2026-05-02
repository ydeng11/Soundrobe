# Music Tagging Features Research for Navidrome/Open Sonic Compatibility

## Overview

This document summarizes research on music tagging standards, Navidrome requirements, and best practices for an auto-tagger CLI tool.

---

## 1. Metadata Format Standards

### 1.1 ID3v2 (MP3, AIFF, WAV)

#### ID3v2.4 vs ID3v2.3
- **ID3v2.4** (recommended): Supports true multi-valued tags, UTF-8 encoding
- **ID3v2.3** (legacy): Limited multi-value support; use separators like `" / "` or `"; "`

#### Key ID3v2 Frames
| Frame ID | Field | Description |
|----------|-------|-------------|
| TIT2 | Title | Song/title name |
| TPE1 | Artist | Lead performer(s)/soloist(s) |
| TPE2 | Album Artist | Band/orchestra/accompaniment (used as Album Artist) |
| TALB | Album | Album/movie/show title |
| TRCK | Track Number | Position in set (e.g., "5" or "5/12") |
| TPOS | Disc Number | Part of a set (e.g., "1/2") |
| TYER/TDRC | Year/Date | Recording year or timestamp |
| TCON | Genre | Content type |
| TCMP | Compilation | Part of compilation flag (value: "1") |
| TBPM | BPM | Beats per minute |
| TKEY | Key | Initial musical key |
| TCOM | Composer | Composer name |
| TEXT | Lyricist | Lyricist/text writer |
| TPE3 | Conductor | Conductor name |
| TPE4 | Remixer | Interpreted/remixed by |
| TSST | Disc Subtitle | Set subtitle |
| TSOA | Album Sort | Album sort order |
| TSOP | Artist Sort | Performer sort order |
| TSOT | Title Sort | Title sort order |
| APIC | Picture | Attached picture (cover art) |
| USLT | Lyrics | Unsynchronised lyrics/text transcription |
| COMM | Comment | Comments |
| TCOP | Copyright | Copyright message |
| TPUB | Publisher | Publisher name |
| TSRC | ISRC | International Standard Recording Code |
| UFID | Unique ID | Unique file identifier (MusicBrainz uses this) |
| TXXX | Custom | User-defined text information frame |

#### Multi-Valued Tags in ID3v2.4
- Multiple values stored as null-separated list
- Use `TXXX:ARTISTS` and `TXXX:ALBUMARTISTS` for explicit multi-artist support
- Navidrome prefers these over separator-based single tags

#### ReplayGain in ID3v2
Uses TXXX frames:
```
TXXX:REPLAYGAIN_TRACK_GAIN = -6.84 dB
TXXX:REPLAYGAIN_TRACK_PEAK = 0.987654
TXXX:REPLAYGAIN_ALBUM_GAIN = -6.12 dB
TXXX:REPLAYGAIN_ALBUM_PEAK = 0.965432
```

Legacy formats (RVA2, RGAD) exist but TXXX is preferred.

---

### 1.2 Vorbis Comments (FLAC, Ogg Vorbis, Opus)

#### Format
- Simple `FIELD=value` pairs
- Case-insensitive field names
- Multiple values: Add same field multiple times
- UTF-8 encoding by default

#### Standard Fields
| Field | Description |
|-------|-------------|
| TITLE | Track/work name |
| VERSION | Version/differentiation |
| ALBUM | Collection name |
| TRACKNUMBER | Track number |
| ARTIST | Main artist |
| ARTISTS | Multi-valued artist list (preferred) |
| PERFORMER | Performer(s) |
| ALBUMARTIST | Album artist |
| ALBUMARTISTS | Multi-valued album artist list |
| DISCNUMBER | Disc number |
| TOTALTRACKS | Total tracks in album |
| TOTALDISCS | Total discs |
| DATE | Recording date |
| YEAR | Recording year |
| ORIGINALDATE | Original release date |
| RELEASEDATE | Release date |
| GENRE | Genre |
| MOOD | Mood |
| COMMENT | Comment/description |
| COPYRIGHT | Copyright |
| LICENSE | License info |
| ORGANIZATION | Record label |
| ISRC | ISRC code |
| COMPILATION | Compilation flag (value: "1") |
| LYRICS | Lyrics text |
| COMPOSER | Composer |
| LYRICIST | Lyricist |
| CONDUCTOR | Conductor |
| BPM | Beats per minute |

#### ReplayGain in Vorbis
```
REPLAYGAIN_TRACK_GAIN=-6.84 dB
REPLAYGAIN_TRACK_PEAK=0.987654
REPLAYGAIN_ALBUM_GAIN=-6.12 dB
REPLAYGAIN_ALBUM_PEAK=0.965432
```

#### MusicBrainz Tags
```
MUSICBRAINZ_ARTISTID=<uuid>
MUSICBRAINZ_ALBUMARTISTID=<uuid>
MUSICBRAINZ_ALBUMID=<uuid>
MUSICBRAINZ_TRACKID=<uuid>
MUSICBRAINZ_RELEASETRACKID=<uuid>
MUSICBRAINZ_RELEASEGROUPID=<uuid>
MUSICBRAINZ_DISCID=<uuid>
MUSICBRAINZ_WORKID=<uuid>
```

---

### 1.3 MP4/M4A (AAC, ALAC)

#### Format
Uses MP4 atoms/boxes for metadata

#### Key MP4 Tags
| Atom | Field | Description |
|------|-------|-------------|
| ©nam | Title | Song name |
| ©art | Artist | Artist |
| ©alb | Album | Album |
| aART | Album Artist | Album artist |
| trkn | Track Number | Track/disc numbers |
| ©gen | Genre | Genre |
| ©day | Year | Year/date |
| cpil | Compilation | Compilation flag |
| tmpo | BPM | Beats per minute |
| ©cmt | Comment | Comment |
| ©wrt | Composer | Writer/composer |
| disk | Disc Number | Disc number |
| ----:com.apple.itunes:ARTISTS | Artists | Multi-valued artists |
| ----:com.apple.itunes:ALBUMARTISTS | Album Artists | Multi-valued album artists |

#### ReplayGain in MP4
```
----:com.apple.itunes:REPLAYGAIN_TRACK_GAIN
----:com.apple.itunes:REPLAYGAIN_TRACK_PEAK
----:com.apple.itunes:REPLAYGAIN_ALBUM_GAIN
----:com.apple.itunes:REPLAYGAIN_ALBUM_PEAK
```

---

### 1.4 APEv2 (APE, WavPack, Musepack)

#### Format
- Key/value pairs similar to Vorbis
- Supports UTF-8 and binary values
- Allows multiple values per key

#### ReplayGain in APEv2
Same keys/format as Vorbis comments.

---

### 1.5 R128 Gain (EBU R128 Standard)

Modern loudness standard used by some formats:
```
R128_TRACK_GAIN=<integer>
R128_ALBUM_GAIN=<integer>
```
Values are in Q7.24 format (e.g., -256 = -1 dB)

---

## 2. Navidrome Compatibility Requirements

### 2.1 Essential Metadata Fields

**Required for proper library organization:**
- **Title** - Song name
- **Artist** - Performing artist(s)
- **Album** - Album name
- **Album Artist** - Primary album artist (critical for album grouping)
- **Track Number** - Position on album

**Strongly recommended:**
- **Disc Number** - For multi-disc albums
- **Genre** - For filtering/browsing
- **Year/Date** - Release or recording date
- **Compilation flag** - For "Various Artists" albums

### 2.2 Multi-Artist Tagging Conventions

#### Preferred Method: Multi-Valued Tags
```yaml
# FLAC/Vorbis example
ARTIST=Alice feat. Bob  # Display name
ARTISTS=Alice           # Individual artists
ARTISTS=Bob             # (multiple ARTISTS fields)

ALBUMARTIST=Alice       # Display name
ALBUMARTISTS=Alice      # Individual album artists
```

#### Fallback: Separator-Based
If multi-valued tags unavailable, use separators:
- `" / "` (recommended)
- `" feat. "`
- `" ft. "`
- `"; "`

**Warning:** Separators can conflict with artist names like:
- AC/DC
- Earth, Wind & Fire

#### Navidrome Artist Splitting Rules
From mappings.yaml:
```yaml
artists:
  split: [" / ", " feat. ", " feat ", " ft. ", " ft ", "; "]
```

### 2.3 Album Grouping Rules

Navidrome groups tracks into albums based on:
1. **Album Artist** + **Album Name** (primary grouping key)
2. All tracks with same Album Artist + Album belong to same album
3. Compilation flag overrides Album Artist for Various Artists albums

**Common issues:**
- Missing Album Artist → albums may split
- Inconsistent Album Artist spelling → duplicate albums
- Inconsistent Album name spelling → duplicate albums

### 2.4 Various Artists/Compilation Handling

**Best practice:**
```yaml
ALBUMARTIST=Various Artists
COMPILATION=1
```

Both should be set for compilation albums.

### 2.5 Multi-Disc Albums

**Required fields:**
```yaml
ALBUM=Greatest Hits
DISCNUMBER=1
DISCTOTAL=2
ALBUMARTIST=Artist Name
```

All discs must have identical Album + Album Artist.

Optional disc subtitle:
```yaml
DISCSUBTITLE=The Early Years
```

---

## 3. Lyrics Handling

### 3.1 Embedded Lyrics

#### ID3v2 (USLT Frame)
```
USLT frame structure:
- Text encoding
- Language (3 chars, e.g., "eng")
- Content descriptor
- Lyrics text
```

Can have multiple USLT frames (different languages).

#### Vorbis Comments
```
LYRICS=<text>
LYRICS:eng=<English lyrics>
```

#### MP4
```
----:com.apple.itunes:LYRICS
```

### 3.2 External LRC Files

#### Simple LRC Format
```lrc
[ar:Artist Name]
[al:Album Name]
[ti:Song Title]
[length:3:45]

[00:12.00]First line of lyrics
[00:17.20]Second line of lyrics
[00:21.10]Third line of lyrics
```

#### Enhanced LRC Format (A2)
Word-level timing:
```lrc
[00:12.00]<00:12.04>When<00:12.16>the<00:12.82>truth
```

#### LRC ID Tags
| Tag | Purpose |
|-----|---------|
| [ar:] | Artist |
| [al:] | Album |
| [ti:] | Title |
| [au:] | Author |
| [lr:] | Lyricist |
| [length:] | Song length (mm:ss) |
| [by:] | LRC file author |
| [offset:] | Time offset in ms |
| [re:] | Player/editor name |

#### LRC Naming Convention
- Same basename as audio file
- Example: `song.flac` → `song.lrc`

---

## 4. Cover Art Requirements

### 4.1 Embedded Cover Art

#### ID3v2 (APIC Frame)
```
APIC structure:
- Text encoding
- MIME type (image/jpeg, image/png)
- Picture type (03 = Cover front)
- Description
- Picture data
```

Picture types:
| Type | Meaning |
|------|---------|
| 00 | Other |
| 01 | 32x32 file icon (PNG) |
| 02 | Other file icon |
| 03 | Cover (front) |
| 04 | Cover (back) |
| 05 | Leaflet page |
| 06 | Media (CD label) |
| 07 | Lead artist |
| 08 | Artist/performer |
| 09 | Conductor |
| 0A | Band/orchestra |
| 0B | Composer |
| 0C | Lyricist |
| 0D | Recording location |
| 0E-0F | During recording/performance |
| 12 | Illustration |
| 13 | Band/artist logotype |
| 14 | Publisher logotype |

#### Vorbis/FLAC
```
METADATA_BLOCK_PICTURE (FLAC)
- Base64 encoded picture structure
```

#### MP4
```
covr atom - Contains image data
```

### 4.2 External Cover Art Files

Navidrome resolution order (default):
```yaml
CoverArtPriority: cover.*, folder.*, front.*, embedded, external
```

Common filenames:
- `cover.jpg`, `cover.png`
- `folder.jpg`, `folder.png`
- `front.jpg`, `front.png`
- `artwork.jpg`

### 4.3 Disc-Specific Artwork

Navidrome resolution order (default):
```yaml
DiscArtPriority: disc*.*, cd*.*, cover.*, folder.*, front.*, discsubtitle, embedded
```

Examples:
```
disc1.jpg, disc2.jpg
cd1.png, cd2.png
```

### 4.4 Recommended Image Specs
- **Size**: 500x500 to 1000x1000 pixels (optimal)
- **Format**: JPEG or PNG (PNG for transparency)
- **Maximum**: 3000x3000 (larger makes UI sluggish)
- **Embedded**: Recommended for portability
- **External**: Recommended for efficiency

---

## 5. Supported Audio Formats

### Navidrome Supported Formats
| Format | Container | Tagging System |
|--------|-----------|----------------|
| MP3 | MPEG | ID3v2 |
| FLAC | FLAC | Vorbis Comments |
| Ogg Vorbis | Ogg | Vorbis Comments |
| Opus | Ogg | Vorbis Comments |
| M4A/AAC | MP4 | MP4 atoms |
| M4A/ALAC | MP4 | MP4 atoms |
| WMA | ASF | ASF tags |
| APE | APE | APEv2 |
| WV (WavPack) | WavPack | APEv2 |
| MPC (Musepack) | MPC | APEv2 |
| WAV | RIFF | ID3v2 (in RIFF chunk) or BWF |
| AIFF | AIFF | ID3v2 (in ID3 IFF chunk) |
| TTA | True Audio | ID3v2 or APEv2 |
| Ogg FLAC | Ogg | Vorbis Comments |
| Ogg Speex | Ogg | Vorbis Comments |

---

## 6. Music Library Conventions

### 6.1 Standard Folder Structure

**Recommended patterns:**
```
Music/
├── Artist Name/
│   └── Album Name (Year)/
│       ├── 01 - Track Title.flac
│       ├── 02 - Track Title.flac
│       └── cover.jpg
```

**Multi-disc structure:**
```
Music/
└── Artist Name/
    └── Album Name (Year)/
        ├── Disc 1/
        │   ├── disc1.jpg
        │   ├── 01 - Track.flac
        │   └── 02 - Track.flac
        ├── Disc 2/
        │   ├── disc2.jpg
        │   ├── 01 - Track.flac
        │   └── 02 - Track.flac
        └── cover.jpg
```

**Compilation structure:**
```
Music/
└── Various Artists/
    └── Compilation Name (Year)/
        ├── 01 - Artist - Track.flac
        └── cover.jpg
```

### 6.2 File Naming Conventions

**Common patterns:**
- `TrackNumber - Title.ext` (e.g., `01 - Song.flac`)
- `TrackNumber - Artist - Title.ext`
- `Artist - Album - TrackNumber - Title.ext`

**Track number format:**
- Zero-padded: `01`, `02`, etc.
- With total: `01/12`

**Avoid:**
- Special characters in filenames
- Very long filenames (260 char limit on Windows)

### 6.3 Duplicate Detection Methods

**Detection strategies:**
1. **MusicBrainz IDs**: Most reliable
   - `MUSICBRAINZ_TRACKID` (recording ID)
   - `MUSICBRAINZ_RELEASETRACKID` (release-specific)
   
2. **Acoustic fingerprinting**:
   - MusicIP PUID
   - AcoustID
   
3. **Metadata matching**:
   - Same Artist + Title + Album + Duration
   
4. **File-based**:
   - Same file size + duration
   - Similar filename

### 6.4 Various Artists Album Handling

**Key requirements:**
- All tracks: Same Album Artist = "Various Artists"
- All tracks: Same Album name
- All tracks: Compilation = 1
- Individual Artist per track in Artist field

---

## 7. Quality Checks and Validation

### 7.1 Corrupt Audio Detection

#### ffprobe Validation
```bash
ffprobe -v error -show_format -show_streams file.flac
```

Key indicators:
- `format.format_name` valid
- `streams[].codec_type=audio` present
- No error messages in output

#### Duration Validation
```bash
ffprobe -v error -show_entries format=duration -of default=noprint_wrappers=1:nokey=1 file.flac
```

Compare with:
- Embedded `TLEN` tag (ID3v2)
- `LENGTH` field (Vorbis)
- File metadata duration

### 7.2 Common Encoding Issues

**Problem indicators:**
- Missing or truncated headers
- Incorrect sample rate
- Missing codec info
- Corrupted frame data
- Wrong bitrate values
- Glitch/pop at boundaries

**Detection methods:**
1. ffprobe error output
2. Playback attempt with ffmpeg
3. File signature validation
4. Header structure validation

### 7.3 Audio Validation Tools

| Tool | Purpose |
|------|---------|
| ffprobe | Format/codec validation |
| ffmpeg | Playback test |
| soxi (SoX) | Audio file info |
| flac --test | FLAC specific validation |
| lame --decode | MP3 validation |
| mediainfo | Comprehensive metadata |

#### Example Validation Commands

**FLAC validation:**
```bash
flac --test file.flac
```

**General audio validation:**
```bash
ffprobe -v warning -show_format -show_streams file.flac 2>&1 | grep -i error
```

**Duration check:**
```bash
duration=$(ffprobe -v error -show_entries format=duration -of csv=p=0 file.flac)
if [ -z "$duration" ] || [ "$duration" == "N/A" ]; then
  echo "Invalid duration"
fi
```

### 7.4 Tag Validation

**Required fields check:**
```python
required = ['title', 'artist', 'album', 'albumartist', 'tracknumber']
for field in required:
    if not file.tags.get(field):
        print(f"Missing: {field}")
```

**Consistency check:**
- Same Album Artist across all album tracks
- Same Album name across all album tracks
- Valid track numbers (1 to total)
- Valid disc numbers (1 to total)

---

## 8. ReplayGain Implementation

### 8.1 ReplayGain Specification

#### Reference Level
- Standard: 89 dB SPL (equivalent to -14 dBFS pink noise)
- Measured using: Equal loudness filter + RMS + 95th percentile

#### Required Metadata
```
REPLAYGAIN_TRACK_GAIN = -a.bb dB  (e.g., -6.84 dB)
REPLAYGAIN_TRACK_PEAK = c.dddddd  (e.g., 0.987654)
REPLAYGAIN_ALBUM_GAIN = -a.bb dB  (e.g., -6.12 dB)
REPLAYGAIN_ALBUM_PEAK = c.dddddd  (e.g., 0.965432)
```

#### Gain Format
- Decibels with 2 decimal precision
- Negative = attenuation (loud files)
- Positive = amplification (quiet files)
- Suffixed with " dB"

#### Peak Format
- Floating point (1.0 = full scale)
- 6 decimal precision
- May exceed 1.0 for coded audio (MP3, etc.)

### 8.2 ReplayGain Tools

| Tool | Description |
|------|-------------|
| rsgain | Modern ReplayGain scanner |
| loudgain | EBU R128 based scanner |
| metaflac | FLAC native tool |
| mp3gain | MP3 specific (modifies audio) |
| aacgain | AAC specific |
| bs1770gain | EBU R128 compliant |

### 8.3 Modern Alternatives (EBU R128)

**R128 tags:**
```
R128_TRACK_GAIN = -256  (Q7.24 format, -1 dB)
R128_ALBUM_GAIN = -512  (Q7.24 format, -2 dB)
```

**Advantages:**
- Industry standard (broadcasting)
- More accurate loudness measurement
- Integrated, short-term, momentary measurements

---

## 9. Python Libraries for Tagging

### 9.1 Mutagen (Recommended)

**Capabilities:**
- Supports: ID3v2, Vorbis, MP4, APEv2, ASF, FLAC, Ogg, etc.
- Read and write
- Multi-valued tags
- No dependencies outside stdlib
- GPL v2+ license

**Example usage:**
```python
from mutagen.flac import FLAC
from mutagen.id3 import ID3, TIT2, TPE1

# FLAC
audio = FLAC("file.flac")
audio["title"] = "Song Title"
audio["artist"] = ["Artist 1", "Artist 2"]
audio.save()

# MP3
audio = ID3("file.mp3")
audio.add(TIT2(encoding=3, text="Song Title"))
audio.add(TPE1(encoding=3, text=["Artist 1", "Artist 2"]))
audio.save()
```

### 9.2 Other Libraries

| Library | Notes |
|---------|-------|
| tinytag | Read-only, lightweight |
| music-tag | Wrapper around mutagen |
| taglib | C++ library with Python bindings |
| eyed3 | ID3 specific |

---

## 10. Navidrome Tag Mappings Summary

Full mappings from: https://github.com/navidrome/navidrome/blob/master/resources/mappings.yaml

### Main Tags (Direct Handling)
| Navidrome Field | Tag Aliases |
|-----------------|-------------|
| title | TIT2, title, ©nam |
| artist | TPE1, artist, ©art |
| artists | TXXX:ARTISTS, artists |
| albumartist | TPE2, albumartist, aart |
| albumartists | TXXX:ALBUMARTISTS, albumartists |
| album | TALB, album, ©alb |
| genre | TCON, genre, ©gen |
| mood | TMOO, mood |
| compilation | TCMP, compilation, cpil |
| track | TRCK, tracknumber, trkn |
| disc | TPOS, discnumber, disk |
| discsubtitle | TSST, discsubtitle |
| bpm | TBPM, bpm, tmpo |
| lyrics | USLT, lyrics |
| comment | COMM, comment, ©cmt |
| originaldate | TDOR, originaldate, TORY, originalyear |
| recordingdate | TDRC, date |
| releasedate | TDRL, releasedate, ©day, year |
| composer | TCOM, composer, ©wrt |
| lyricist | TEXT, lyricist |
| conductor | TPE3, conductor |
| remixer | TPE4, remixer |
| replaygain_track_gain | TXXX:REPLAYGAIN_TRACK_GAIN |
| replaygain_track_peak | TXXX:REPLAYGAIN_TRACK_PEAK |
| replaygain_album_gain | TXXX:REPLAYGAIN_ALBUM_GAIN |
| replaygain_album_peak | TXXX:REPLAYGAIN_ALBUM_PEAK |

### Additional Tags (Smart Playlists)
| Field | Tag Aliases |
|-------|-------------|
| grouping | GRP1, grouping, ©grp |
| key | TKEY, key |
| isrc | TSRC, isrc |
| language | TLAN, language |
| license | WCOP, license |
| media | TMED, media |
| recordlabel | TPUB, label, publisher |
| releasecountry | releasecountry |
| releasestatus | releasestatus |
| releasetype | releasetype, musicbrainz_albumtype |
| script | script |
| subtitle | TIT3, subtitle |
| website | WOAR, website |
| work | TIT1, work, ©wrk |

### MusicBrainz ID Fields
| Field | Tag Aliases |
|-------|-------------|
| musicbrainz_artistid | UFID:http://musicbrainz.org |
| musicbrainz_albumartistid | TXXX:musicbrainz album artist id |
| musicbrainz_albumid | TXXX:musicbrainz album id |
| musicbrainz_trackid | TXXX:musicbrainz release track id |
| musicbrainz_recordingid | UFID or TXXX:musicbrainz track id |

---

## 11. Implementation Recommendations

### 11.1 Recommended Python Libraries
- **mutagen** - Primary tagging library (read/write all formats)
- **ffprobe/ffmpeg** - Audio validation and duration checks
- **pydub** or **soundfile** - Audio analysis if needed

### 11.2 Key Features to Implement
1. **Tag reading/writing** across all formats
2. **Multi-valued tag support** (artists, genres)
3. **Validation checks**:
   - Required fields present
   - Album consistency
   - Audio file validity
4. **ReplayGain calculation** (using rsgain or internal)
5. **Cover art handling** (embedded + external)
6. **LRC file handling** (read/write)
7. **Duplicate detection** (MusicBrainz IDs + metadata)
8. **Batch operations** (album-level tagging)

### 11.3 Validation Checklist
- Audio file decodable (ffprobe check)
- Duration > 0 and matches tags
- Required tags: title, artist, album, albumartist, tracknumber
- Album consistency: all tracks share albumartist + album
- Track numbers within valid range
- Disc numbers within valid range (for multi-disc)
- Embedded cover art valid (if present)
- ReplayGain values valid format (if present)
- MusicBrainz IDs valid UUIDs (if present)

---

## References

- ReplayGain Specification: https://wiki.hydrogenaudio.org/index.php?title=Original_ReplayGain_specification
- ID3v2.4 Frames: https://id3.org/id3v2.4.0-frames
- Vorbis Comments: https://xiph.org/vorbis/doc/v-comment.html
- LRC Format: https://en.wikipedia.org/wiki/LRC_(file_format)
- Navidrome Tagging Guidelines: https://www.navidrome.org/docs/usage/library/tagging/
- Navidrome Artwork: https://www.navidrome.org/docs/usage/library/artwork/
- Navidrome Mappings: https://github.com/navidrome/navidrome/blob/master/resources/mappings.yaml
- Mutagen Docs: https://mutagen.readthedocs.io/