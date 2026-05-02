# Audio Tagging & Beets Pitfalls Research

## Executive Summary

This document consolidates research findings on common pitfalls, gotchas, and challenges in audio tagging systems, specifically focusing on beets library usage, audio encoding complexities, LLM integration challenges, and performance optimization.

---

## 1. Beets Pitfalls

### 1.1 Common Import Issues

#### **Problem: "Can't Find Match" Errors**
**Causes:**
- No autotagger plugins enabled (musicbrainz, chroma, discogs)
- Album not in MusicBrainz database
- Poor metadata quality in source files
- Multi-disc albums in separate directories

**Solutions:**
```yaml
# Ensure autotagger extensions are enabled
plugins: musicbrainz chroma discogs

# For multi-disc albums, use --flat flag or consolidate directories
import:
    group_albums: yes
```

**Warning Signs:**
- Empty metadata fields (title, artist, album)
- Generic filenames (Track01.mp3, Unknown Artist)
- Inconsistent track numbering

#### **Problem: Import Hangs or Appears Stuck**
**Root Cause:** Multithreaded importer backlog
- Beets processes prompts asynchronously
- Background tasks (copying, tagging, plugin tasks) continue after last decision
- Chromaprint/fingerprinting adds significant CPU overhead

**Mitigation:**
```yaml
threaded: no  # Disable for debugging
import:
    timid: yes  # Ask for confirmation on every match
```

#### **Problem: Unreadable Files**
**Causes:**
- Corrupted audio files
- Files with wrong extension (e.g., .mp3 but actually FLAC)
- DRM-protected files
- Incomplete downloads

**Detection:**
- Test with VLC or other media players
- Use `metaflac --list` for FLAC integrity check
- Run `ffprobe` for format detection

**Solution:**
```yaml
import:
    fix_ext_inplace: yes  # Auto-fix incorrect extensions
```

### 1.2 MusicBrainz Match Accuracy Problems

#### **Problem: Wrong Album Matched**
**Causes:**
- Similar album names (e.g., "Greatest Hits" compilations)
- Multiple versions of same album (remasters, deluxe editions)
- Various Artists compilations
- Regional releases with different track orders

**Configuration Solutions:**
```yaml
match:
    # Prefer specific media types and countries
    preferred:
        countries: ['US', 'GB|UK']
        media: ['CD', 'Digital Media|File']
        original_year: yes
    
    # Adjust distance weights for better matching
    distance_weights:
        artist: 3.0
        album: 3.0
        year: 1.0
        tracks: 2.0
        track_title: 3.0
        track_length: 2.0
        
    # Require certain fields for match acceptance
    required: year label catalognum country
    
    # Ignore problematic matches
    ignored: missing_tracks unmatched_tracks
```

#### **Problem: Missing/Unmatched Tracks**
**Detection:**
- Compare imported track count with MusicBrainz release
- Check track duration differences
- Verify disc/track numbering

**Mitigation Strategies:**
```yaml
match:
    max_rec:
        missing_tracks: medium
        unmatched_tracks: medium
        
    ignored_media: ['Data CD', 'DVD', 'DVD-Video', 'Blu-ray']
    ignore_data_tracks: yes
    ignore_video_tracks: yes
```

### 1.3 Performance Bottlenecks

#### **Problem: Slow Import Process**
**Causes:**
1. **Chromaprint Fingerprinting:**
   - CPU-intensive audio analysis
   - Memory overhead for decoding
   - Dependency on external tools (fpcalc)

2. **MusicBrainz API Rate Limiting:**
   - Max 1 request/second per IP
   - 50 requests/second for known User-Agents
   - 300 requests/second global cap

3. **Plugin Overhead:**
   - Lyrics fetching
   - Genre lookups (lastgenre)
   - Album art downloads

**Optimization Strategies:**
```yaml
# Reduce fingerprinting overhead
chroma:
    auto: no  # Disable auto-fingerprinting

# Configure threading appropriately
threaded: yes  # Default, but can disable for debugging

# Batch processing
import:
    incremental: yes  # Skip already-imported directories
    resume: yes  # Resume interrupted imports
```

**Batch Processing Best Practices:**
- Process in chunks of 100-500 albums
- Use incremental import mode
- Schedule imports at random intervals (avoid peak times)
- Implement local caching of MusicBrainz responses

### 1.4 Database Corruption Scenarios

#### **Problem: SQLite Database Issues**
**Causes:**
- Concurrent access without proper locking
- Disk space exhaustion during write
- Power failure during transaction
- File system corruption
- Large database size (>1GB)

**Warning Signs:**
- `database disk image is malformed` errors
- Missing albums/items after import
- Inconsistent queries (albums showing wrong items)
- Slow query performance

**Prevention:**
```bash
# Regular database integrity check
beet check

# Backup before major operations
cp ~/.config/beets/library.db ~/.config/beets/library.db.backup

# Use WAL mode for better concurrency
# (requires SQLite >= 3.7.0)
```

**Recovery:**
```bash
# Dump and restore
sqlite3 library.db .dump > backup.sql
sqlite3 new_library.db < backup.sql

# Or use SQLite recovery tools
sqlite3 library.db ".recover" > recovered.sql
```

### 1.5 Configuration Gotchas

#### **Problem: ID3v2.3 vs ID3v2.4 Compatibility**
**Issue:** Beets writes ID3v2.4 by default; some players only support v2.3

**Software with v2.3 Requirement:**
- Windows Media Player
- Windows Explorer
- id3lib/id3v2 tools
- Some car audio systems

**Solution:**
```yaml
id3v23: yes  # Use ID3v2.3 instead of v2.4
```

#### **Problem: Windows-Safe Filename Issues**
**Issue:** Default `replace` config sanitizes filenames for Windows

**Problematic Characters:**
- Trailing dots: "M.I.A." → "M.I.A_"
- Trailing spaces
- Path separators
- Control characters
- Reserved characters: `< > : " ? * |`

**Configuration:**
```yaml
replace:
    '[\\/]': _
    '^\.': _
    '[\x00-\x1f]': _
    '[<>:"\?\*\|]': _
    '\.$': _
    '\s+$': ''
    '^\s+': ''
    '^-': _
    
# Or disable Windows-safe names (risky for SMB/network filesystems)
# replace: {}
```

#### **Problem: Path Format Conflicts**
**Issue:** Albums with same name collide

**Solution - Unique Disambiguation:**
```yaml
paths:
    default: $albumartist/$album%aunique{}/$track $title
    
aunique:
    keys: albumartist album
    disambiguators: albumtype year label catalognum albumdisambig
    bracket: '[]'
```

#### **Problem: Per-Disc Numbering Confusion**
**Issue:** Track numbering behavior differs with `per_disc_numbering`

```yaml
per_disc_numbering: yes

# Then adjust path format to include disc number
paths:
    default: $albumartist/$album%aunique{}/$disc-$track $title
```

---

## 2. Audio Encoding Issues

### 2.1 Non-UTF8 Metadata Handling

#### **Problem: Encoding Corruption**
**Causes:**
- Legacy encodings (ISO-8859-1, Windows-1252)
- Mixed encoding within same tag
- Incorrect encoding markers
- Unicode conversion errors

**Common Scenarios:**
1. Latin-1 encoded tags read as UTF-8
2. CP1252 (Windows) misinterpreted as Latin-1
3. Null-byte termination issues
4. BOM (Byte Order Mark) problems

**Detection:**
```python
# Check for encoding issues
import chardet
raw_bytes = file.read()
encoding = chardet.detect(raw_bytes)
```

**Mitigation:**
- Use `mutagen` library (handles encoding automatically)
- Force UTF-8 encoding on write
- Normalize text with Unicode normalization (NFC/NFD)

### 2.2 ID3v2.3 vs ID3v2.4 Issues

#### **Key Differences:**

| Feature | ID3v2.3 | ID3v2.4 |
|---------|---------|---------|
| Text Encoding | ISO-8859-1, UTF-16 | UTF-8, UTF-16, UTF-16BE |
| Multiple Values | Not standard | Standard with separators |
| Timestamps | TYER, TDAT, TIME frames | Single TDRC frame |
| Frame Size | 4 bytes (not synchsafe) | 4 bytes (synchsafe) |
| Unsynchronisation | Tag-level only | Frame-level optional |

#### **Problem: Multiple Values in v2.3**
**Issue:** v2.3 spec states "after text termination, all following information should be ignored"

**Mutagen Approach:**
```python
# Join multiple values with separator
audio.save(v2_version=3, v23_sep='/')  # Use '/' as separator

# Or use null terminator (non-standard but works with some readers)
audio.save(v2_version=3, v23_sep=None)
```

#### **Problem: Timestamp Frames Migration**
**v2.3 → v2.4:**
- TYER (year) → absorbed into TDRC
- TDAT (date) → absorbed into TDRC
- TIME (time) → absorbed into TDRC
- TRDA (recording dates) → absorbed into TDRC

**v2.4 → v2.3:**
- TDRC splits into multiple frames
- Potential data loss (TDRC has more precision)

### 2.3 Multi-Value Tag Handling

#### **Format Differences:**

| Format | Multi-Value Support | Implementation |
|--------|---------------------|----------------|
| ID3v2.4 | Yes | Multiple text frames or separators |
| ID3v2.3 | Limited | '/' separator (non-standard) |
| Vorbis/FLAC | Yes | Multiple fields with same key |
| APEv2 | Yes | Multiple values with separator |
| MP4/M4A | Yes | Multiple atoms |

#### **Problem: Genre Lists**
**Issue:** Multiple genres represented differently per format

**ID3v2.4:**
- TCON frame can have multiple values
- Genre IDs (numeric) + text refinements

**Vorbis/FLAC:**
- Multiple GENRE fields allowed

**Beets Handling:**
```yaml
# Genre field can be list or single string
genres: ["Rock", "Alternative", "Indie"]

# lastgenre plugin handles normalization
plugins: lastgenre
lastgenre:
    source: lastfm
    fallback: Unknown
    separator: ', '
```

### 2.4 Codec-Specific Quirks

#### **MP3 Issues:**
1. **Sync-safe integer bugs:** Large tags (>128KB) can corrupt size fields
2. **Padding:** Old ID3v1 tag remnants can confuse parsers
3. **ReplayGain:** Non-standard TXXX frames for gain values
4. **Album Art:** APIC frame description uniqueness issues

#### **FLAC Issues:**
1. **Vorbis Comments:** No standardized field names (ARTIST vs artist)
2. **Picture Metablocks:** Multiple pictures need type/description uniqueness
3. **CUESHEET:** Embedded cue sheets can conflict with metadata

#### **MP4/M4A Issues:**
1. **Atom naming:** Free-form atoms (----) vs standard atoms
2. **Rating/Score:** Multiple incompatible rating systems
3. **Compilation flag:** 'cpil' atom vs album artist approach

#### **WAV Issues:**
1. **RIFF INFO:** Limited field set, no standardization
2. **BWF extension:** Broadcast Wave Format adds specialized fields
3. **ID3 in WAV:** Non-standard but sometimes used
4. **MP3 inside WAV:** WAVE_FORMAT_MPEGLAYER3 containers

**Beets WAV Handling:**
```yaml
import:
    remux_mp3_in_wav: yes  # Extract MP3 from WAV container
```

---

## 3. LLM Integration Challenges

### 3.1 Hallucination Risks

#### **Problem: Fabricated Metadata**
**Risks:**
1. **Invented Artist Names:** LLM may create plausible but non-existent artists
2. **False Album Titles:** Generate albums that don't exist in reality
3. **Incorrect Dates:** Fabricate release years, recording dates
4. **Genre Invention:** Create non-standard or fake genre classifications
5. **Track List Fabrication:** Generate fictional track names and orders

#### **Hallucination Patterns:**
- Confident assertions without factual basis
- Mixing real and fictional information
- Creating "plausible" but unverifiable details
- Temporal confusion (wrong decade, year)
- Geographic mixing (wrong country, region)

#### **Mitigation Strategies:**

**1. Validation Pipeline:**
```
Raw File → LLM Analysis → Validation → MusicBrainz Verification → Write Tags
           ↓                ↓                ↓
        Extract info    Check format    Cross-reference
        Generate tags   Validate fields  Confirm existence
```

**2. Validation Checks:**
- **Artist existence:** Search MusicBrainz for artist
- **Album verification:** Check album exists with track count match
- **Date validation:** Verify release date in MusicBrainz
- **Genre normalization:** Map to standard genre taxonomy
- **Duration verification:** Compare LLM-suggested length with actual

**3. Confidence Thresholds:**
```python
class LLMMetadataValidator:
    def validate_artist(self, artist: str) -> ValidationResult:
        mb_results = musicbrainz.search_artist(artist)
        if mb_results['count'] == 0:
            return ValidationResult(
                confidence=0.0,
                error="Artist not found in MusicBrainz",
                needs_review=True
            )
        
        # Check exact match vs fuzzy match
        exact_matches = [r for r in mb_results['artists'] 
                        if r['name'].lower() == artist.lower()]
        
        confidence = 1.0 if exact_matches else 0.5
        return ValidationResult(
            confidence=confidence,
            verified_artist=mb_results['artists'][0]['name'],
            mbid=mb_results['artists'][0]['id'],
            needs_review=confidence < 0.8
        )
```

**4. Structured Output Enforcement:**
```python
# Use JSON schema validation
metadata_schema = {
    "type": "object",
    "properties": {
        "artist": {"type": "string"},
        "album": {"type": "string"},
        "year": {"type": "integer", "minimum": 1900, "maximum": 2030},
        "track_number": {"type": "integer", "minimum": 1, "maximum": 99},
        "genre": {"type": "string"},
        "title": {"type": "string"}
    },
    "required": ["artist", "title"],
    "additionalProperties": False
}

# Validate LLM output against schema
def validate_llm_output(llm_response: dict) -> bool:
    try:
        validate(instance=llm_response, schema=metadata_schema)
        return True
    except ValidationError as e:
        log_error(f"Schema validation failed: {e.message}")
        return False
```

**5. Multi-Source Verification:**
```python
def verify_metadata(metadata: dict) -> VerificationReport:
    sources = []
    
    # Check MusicBrainz
    mb_match = search_musicbrainz(metadata)
    sources.append(SourceVerification(
        name="MusicBrainz",
        match_score=mb_match.distance,
        mbid=mb_match.mbid,
        verified=mb_match.confidence > 0.8
    ))
    
    # Check Discogs (optional)
    discogs_match = search_discogs(metadata)
    sources.append(SourceVerification(
        name="Discogs",
        match_score=discogs_match.score,
        verified=discogs_match.confidence > 0.7
    ))
    
    # Only accept if majority of sources agree
    return VerificationReport(
        sources=sources,
        overall_confidence=calculate_consensus(sources),
        needs_manual_review=overall_confidence < 0.75
    )
```

### 3.2 Cost Control Strategies

#### **Problem: API Cost Explosion**
**Cost Drivers:**
1. Input tokens (audio metadata, context)
2. Output tokens (generated metadata)
3. Number of API calls per file
4. Retry attempts for validation
5. Large libraries (10,000+ files)

#### **Cost Estimation:**
```
Per-file cost breakdown:
- Input: ~500 tokens (filename + existing metadata + context)
- Output: ~200 tokens (structured metadata JSON)
- Total: ~700 tokens per file

Cost per 1K tokens (example):
- GPT-4: $0.03 input + $0.06 output = $0.09/1K tokens
- GPT-3.5-turbo: $0.0005 input + $0.0015 output = $0.002/1K tokens

Library of 10,000 files:
- GPT-4: 700 tokens * 10,000 = 7M tokens ≈ $630
- GPT-3.5-turbo: 7M tokens ≈ $14
```

#### **Cost Optimization Techniques:**

**1. Tiered Processing:**
```python
def tiered_processing(file_metadata: dict):
    # Tier 1: Use free sources first (MusicBrainz, filename parsing)
    mb_match = musicbrainz_autotag(file_metadata)
    if mb_match.confidence > 0.9:
        return mb_match.metadata  # No LLM needed
    
    # Tier 2: Cheap LLM for uncertain cases
    if mb_match.confidence > 0.5:
        llm_metadata = cheap_llm_refine(mb_match, file_metadata)
        return llm_metadata
    
    # Tier 3: Premium LLM only for difficult cases
    return premium_llm_full_analysis(file_metadata)
```

**2. Batch API Calls:**
```python
# Process multiple files in single request
def batch_process(files: list[dict]) -> list[dict]:
    batch_prompt = f"""
    Process these {len(files)} files and return JSON array:
    
    Files:
    {json.dumps(files, indent=2)}
    
    Return format: [{{"artist": "...", ...}}, ...]
    """
    
    # 100 files in one call vs 100 individual calls
    # Saves ~50% on input token overhead
    response = llm_api(batch_prompt)
    return parse_batch_response(response)
```

**3. Caching Strategy:**
```python
import hashlib
from functools import lru_cache

# Cache by audio fingerprint
@lru_cache(maxsize=1000)
def cached_llm_analysis(audio_hash: str, metadata_hash: str):
    return llm_analyze(audio_hash, metadata_hash)

def process_with_cache(file_path: str):
    audio_hash = compute_audio_fingerprint(file_path)
    metadata_hash = hash_existing_metadata(file_path)
    
    return cached_llm_analysis(audio_hash, metadata_hash)

# Cache MusicBrainz responses longer
MB_CACHE_TTL = 7 * 24 * 3600  # 7 days
```

**4. Progressive Confidence:**
```yaml
# Configure thresholds to minimize LLM usage
llm_config:
    confidence_thresholds:
        skip_llm: 0.95      # MusicBrainz match this good, skip LLM
        use_small_model: 0.7 # Use cheaper model for this range
        use_large_model: 0.3 # Only premium for very uncertain cases
```

**5. Token Budget Management:**
```python
class TokenBudget:
    def __init__(self, daily_limit: int, cost_per_1k: float):
        self.daily_limit = daily_limit
        self.cost_per_1k = cost_per_1k
        self.used_today = 0
        
    def can_afford(self, estimated_tokens: int) -> bool:
        return (self.used_today + estimated_tokens) <= self.daily_limit
    
    def track_usage(self, actual_tokens: int):
        self.used_today += actual_tokens
        cost = (actual_tokens / 1000) * self.cost_per_1k
        log_usage(f"Used {actual_tokens} tokens, cost: ${cost:.4f}")
        
    def get_remaining_budget(self) -> int:
        return self.daily_limit - self.used_today
```

### 3.3 Token Optimization Techniques

#### **Prompt Engineering for Efficiency:**

**1. Minimal Context Injection:**
```python
# DON'T: Include entire file metadata history
prompt = f"""
Based on this comprehensive metadata history:
{json.dumps(full_metadata_history)}

Generate tags for: {filename}
"""

# DO: Include only relevant context
prompt = f"""
Filename: {filename}
Existing tags: {extract_relevant_tags(existing_metadata)}

Generate: artist, album, title, year (JSON format)
"""
```

**2. Template-Based Prompts:**
```python
# Reuse prompt template to reduce input overhead
TEMPLATE = """
Filename: {filename}
Artist hint: {artist_hint}
Title hint: {title_hint}

Return JSON: {{artist, title, album, year, genre}}
"""

def generate_prompt(filename: str, hints: dict) -> str:
    return TEMPLATE.format(
        filename=filename,
        artist_hint=hints.get('artist', 'unknown'),
        title_hint=hints.get('title', 'unknown')
    )
```

**3. Structured Output Constraints:**
```python
# Force minimal, structured output
prompt = """
Return ONLY JSON object with these exact keys:
{"artist": string, "title": string, "year": integer|null}

No explanations, no markdown, no extra text.
Input filename: "01-artist-track.mp3"
"""
```

**4. Few-Shot Learning with Examples:**
```python
# Include only 2-3 examples to establish pattern
prompt = """
Examples:
"01-The Beatles-Let It Be.mp3" → {"artist":"The Beatles","title":"Let It Be"}
"track02.mp3" → {"artist":null,"title":"Track 2"}

Now process: {filename}
"""
```

#### **Response Parsing Optimization:**

**1. Regex Extraction:**
```python
import re

def extract_json_from_llm(response: str) -> dict:
    # Fast extraction without full JSON parsing overhead
    pattern = r'\{[^}]+\}'
    match = re.search(pattern, response)
    if match:
        return json.loads(match.group())
    return {}
```

**2. Streaming Response Processing:**
```python
def process_streaming_response(llm_stream):
    partial_json = ""
    for chunk in llm_stream:
        partial_json += chunk.text
        # Early termination when complete JSON detected
        if is_complete_json(partial_json):
            return parse_json(partial_json)
    return parse_json(partial_json)
```

### 3.4 Prompt Engineering for Structured Output

#### **Techniques for Reliable JSON Output:**

**1. Schema Enforcement Prompt:**
```python
def create_schema_prompt(metadata: dict) -> str:
    return f"""
You are a music metadata extractor. Return ONLY valid JSON.

Input: {metadata['filename']}
Existing metadata: {json.dumps(metadata['existing'], indent=2)}

Required JSON schema:
{
  "artist": "string (required)",
  "album": "string or null",
  "title": "string (required)",
  "year": "integer between 1900-2030 or null",
  "track_number": "integer between 1-99 or null",
  "genre": "string from standard list or null"
}

Standard genres: Rock, Pop, Electronic, Jazz, Classical, Hip-Hop, 
R&B, Country, Blues, Folk, Metal, Punk, Indie, Alternative

Return JSON ONLY. No explanations, no markdown code blocks.
"""
```

**2. Constraint Checklist:**
```python
CONSTRAINT_PROMPT = """
Constraints checklist (verify before outputting):
- ✓ artist and title are present (required)
- ✓ year is integer between 1900-2030 if provided
- ✓ track_number is integer 1-99 if provided  
- ✓ genre matches standard list if provided
- ✓ JSON is valid and parseable
- ✓ No extra fields beyond schema
- ✓ No null values for required fields

Output JSON: 
"""
```

**3. Error Recovery Prompt:**
```python
def retry_with_error_message(original_response: str, error: str) -> str:
    return f"""
Your previous response was invalid JSON. Error: {error}

Original response: {original_response}

Fix the JSON and return ONLY corrected version:
"""
```

**4. Two-Stage Generation:**
```python
# Stage 1: Extract raw metadata
extract_prompt = """
From filename "{filename}", extract:
- Artist name
- Track title
- Album (if discernible)
- Year (if discernible)

Return as: artist|title|album|year
"""

# Stage 2: Format as JSON
format_prompt = """
Convert this metadata to JSON:
{raw_metadata}

Schema: {"artist": string, "title": string, "album": string|null, "year": int|null}
"""
```

---

## 4. Performance Considerations

### 4.1 Batch Processing Best Practices

#### **Chunk Size Optimization:**

**Recommended Batch Sizes:**
| Library Size | Batch Size | Reasoning |
|--------------|------------|-----------|
| <1,000 files | 100-200 | Process entire library quickly |
| 1,000-10,000 | 200-500 | Balance memory/performance |
| >10,000 files | 500-1000 | Minimize transaction overhead |

**Implementation:**
```python
def batch_import(library_path: str, batch_size: int = 500):
    files = collect_files(library_path)
    
    for i in range(0, len(files), batch_size):
        batch = files[i:i+batch_size]
        
        # Process batch
        results = process_batch(batch)
        
        # Commit to database
        commit_batch(results)
        
        # Progress tracking
        log_progress(i, len(files), batch_size)
        
        # Memory cleanup
        clear_batch_cache()
```

#### **Transaction Management:**

```python
import sqlite3

def optimized_import(files: list):
    conn = sqlite3.connect('library.db')
    
    # Use WAL mode for better concurrency
    conn.execute('PRAGMA journal_mode=WAL')
    
    # Batch inserts in single transaction
    conn.execute('BEGIN TRANSACTION')
    
    for file in files:
        # Import logic
        insert_metadata(conn, file)
    
    # Single commit for entire batch
    conn.execute('COMMIT')
    
    # Periodic integrity check
    if batch_number % 10 == 0:
        conn.execute('PRAGMA integrity_check')
```

### 4.2 Caching Strategies for MusicBrainz

#### **Local Caching Architecture:**

```
Cache Layers:
├── Level 1: In-memory (LRU cache)
│   └── Size: 1000 entries, TTL: 1 hour
│   └── Use for: Recent queries, repeated searches
│
├── Level 2: Disk cache (SQLite)
│   └── Size: Unlimited, TTL: 7 days
│   └── Use for: All MusicBrainz responses
│   └── Structure: query_hash → response_json
│
├── Level 3: MusicBrainz API
│   └── Rate limit: 1 req/sec per IP
│   └── Use for: Cache misses only
```

**Implementation:**
```python
import hashlib
import sqlite3
from datetime import datetime, timedelta

class MusicBrainzCache:
    def __init__(self, cache_db: str = 'mb_cache.db'):
        self.conn = sqlite3.connect(cache_db)
        self._init_cache_table()
        self.lru_cache = {}
        
    def _init_cache_table(self):
        self.conn.execute("""
            CREATE TABLE IF NOT EXISTS mb_cache (
                query_hash TEXT PRIMARY KEY,
                response_json TEXT,
                timestamp TEXT,
                query_type TEXT
            )
        """)
        
    def get(self, query: dict) -> dict | None:
        query_hash = self._hash_query(query)
        
        # L1: In-memory cache
        if query_hash in self.lru_cache:
            return self.lru_cache[query_hash]
        
        # L2: Disk cache
        result = self.conn.execute(
            "SELECT response_json, timestamp FROM mb_cache WHERE query_hash = ?",
            (query_hash,)
        ).fetchone()
        
        if result:
            timestamp = datetime.fromisoformat(result[1])
            if datetime.now() - timestamp < timedelta(days=7):
                response = json.loads(result[0])
                self.lru_cache[query_hash] = response
                return response
        
        return None
        
    def set(self, query: dict, response: dict):
        query_hash = self._hash_query(query)
        
        # Store in L1
        self.lru_cache[query_hash] = response
        
        # Store in L2
        self.conn.execute(
            "INSERT OR REPLACE INTO mb_cache VALUES (?, ?, ?, ?)",
            (query_hash, json.dumps(response), datetime.now().isoformat(), query['type'])
        )
        self.conn.commit()
        
    def _hash_query(self, query: dict) -> str:
        return hashlib.sha256(json.dumps(query, sort_keys=True).encode()).hexdigest()
```

#### **Cache Invalidation Strategies:**

**1. Time-Based:**
```python
# TTL-based expiration
CACHE_TTL = {
    'artist_search': 7 * 24 * 3600,  # 7 days (artists rarely change)
    'release_search': 24 * 3600,      # 1 day (releases can be edited)
    'recording_search': 3 * 24 * 3600 # 3 days
}
```

**2. MusicBrainz Edit Awareness:**
```python
# Check for recent edits to cached entities
def check_cache_validity(mbid: str, cached_timestamp: datetime):
    last_edit = musicbrainz.get_last_edit_time(mbid)
    if last_edit > cached_timestamp:
        invalidate_cache(mbid)
        return False
    return True
```

### 4.3 Rate Limiting Requirements

#### **MusicBrainz Rate Limits:**

| Limit Type | Threshold | Consequence |
|------------|-----------|-------------|
| User-Agent specific | 50 req/sec | HTTP 503 for known apps |
| IP address | 1 req/sec average | All requests blocked if exceeded |
| Global | 300 req/sec | HTTP 503 when overloaded |

#### **Rate Limiting Implementation:**

```python
import time
from collections import deque

class RateLimiter:
    def __init__(self, max_rate: float = 1.0):
        self.max_rate = max_rate  # requests per second
        self.requests = deque()
        self.last_request_time = 0
        
    def acquire(self):
        now = time.time()
        
        # Remove requests older than 1 second
        while self.requests and self.requests[0] < now - 1.0:
            self.requests.popleft()
        
        # Check if we're at rate limit
        if len(self.requests) >= self.max_rate:
            sleep_time = 1.0 - (now - self.requests[0])
            if sleep_time > 0:
                time.sleep(sleep_time)
        
        # Record this request
        self.requests.append(now)
        
    def adaptive_rate_limit(self, response_headers: dict):
        # Check for 503 status
        if 'retry-after' in response_headers:
            retry_after = int(response_headers['retry-after'])
            time.sleep(retry_after)
            
        # Reduce rate if getting 503s
        if response_headers.get('status') == '503':
            self.max_rate *= 0.5  # Halve the rate
```

#### **User-Agent Configuration:**

```python
# CRITICAL: Must identify your application
USER_AGENT = "AutoTagger/1.0.0 (https://github.com/user/auto-tagger)"

# Configure in musicbrainzngs
import musicbrainzngs
musicbrainzngs.set_useragent(USER_AGENT)

# Rate limiting is PER USER-AGENT
# Good User-Agent strings get 50 req/sec instead of 1 req/sec
```

#### **Retry Strategy:**

```python
def api_call_with_retry(api_func, max_retries=3):
    for attempt in range(max_retries):
        try:
            rate_limiter.acquire()
            response = api_func()
            return response
            
        except HTTPError503 as e:
            if attempt < max_retries - 1:
                # Exponential backoff
                sleep_time = (2 ** attempt) + random.uniform(0, 1)
                time.sleep(sleep_time)
            else:
                raise MaxRetriesExceeded(f"Failed after {max_retries} attempts")
```

### 4.4 Memory Management for Large Libraries

#### **Problem: Memory Exhaustion**

**Causes:**
1. Loading entire library into memory
2. Large audio files for fingerprinting
3. Batch processing without cleanup
4. Image data (album art) in memory
5. Query result caching

#### **Memory Optimization Strategies:**

**1. Lazy Loading:**
```python
# DON'T: Load all files at once
all_files = [load_file(f) for f in file_list]

# DO: Stream files one at a time
for file_path in file_stream:
    file_data = load_file(file_path)
    process(file_data)
    del file_data  # Explicit cleanup
```

**2. Generator-Based Processing:**
```python
def file_streamer(library_path: str):
    for root, dirs, files in os.walk(library_path):
        for file in files:
            if is_audio_file(file):
                yield os.path.join(root, file)

# Process with generator (constant memory usage)
for file_path in file_streamer('/music'):
    process_single_file(file_path)
```

**3. Memory Pooling:**
```python
from multiprocessing import Pool

def parallel_process(files: list, max_workers: int = 4):
    # Limit worker count based on available memory
    available_memory = psutil.virtual_memory().available
    memory_per_worker = 100 * 1024 * 1024  # 100MB per worker
    max_workers = min(max_workers, available_memory // memory_per_worker)
    
    with Pool(max_workers) as pool:
        results = pool.map(process_file, files)
    
    return results
```

**4. Chunked Audio Decoding:**
```python
def fingerprint_large_file(file_path: str, chunk_size: int = 1024*1024):
    # Decode in chunks instead of loading entire file
    decoder = AudioDecoder(file_path)
    
    fingerprint_data = []
    for chunk in decoder.stream_chunks(chunk_size):
        partial_fp = compute_fingerprint(chunk)
        fingerprint_data.append(partial_fp)
        
    return combine_fingerprints(fingerprint_data)
```

**5. Database Memory Optimization:**
```python
# SQLite memory settings
conn.execute('PRAGMA cache_size = -10000')  # 10MB cache
conn.execute('PRAGMA temp_store = MEMORY')
conn.execute('PRAGMA mmap_size = 268435456')  # 256MB mmap

# Large library optimization
conn.execute('PRAGMA page_size = 4096')
conn.execute('PRAGMA synchronous = NORMAL')
```

#### **Memory Profiling:**

```python
import tracemalloc
import psutil

def profile_memory_usage(func):
    tracemalloc.start()
    
    before = psutil.Process().memory_info().rss
    
    result = func()
    
    after = psutil.Process().memory_info().rss
    
    snapshot = tracemalloc.take_snapshot()
    top_stats = snapshot.statistics('lineno')
    
    print(f"Memory delta: {(after - before) / 1024 / 1024:.2f} MB")
    for stat in top_stats[:10]:
        print(stat)
    
    tracemalloc.stop()
    return result
```

---

## 5. Warning Signs Detection Guide

### 5.1 Import Process Warning Signs

| Sign | Indication | Action |
|------|------------|--------|
| Import takes >5 min per album | Chromaprint overhead or API rate limiting | Disable fingerprinting or check rate |
| "Unreadable file" messages | Corrupted files or wrong extensions | Validate files with VLC/ffprobe |
| Many "no match found" | Missing MusicBrainz data or poor metadata | Use manual search or submit to MB |
| Database grows >500MB | Large library or inefficient queries | Optimize database, use incremental |
| Memory usage >1GB | Memory leak or large batch processing | Reduce batch size, profile memory |

### 5.2 Tag Quality Warning Signs

| Sign | Indication | Action |
|------|------------|--------|
| Genre field empty | lastgenre plugin misconfigured | Configure lastfm source |
| Missing album art | No art_sources or network issues | Add albumart plugin |
| Wrong track numbers | per_disc_numbering misconfigured | Adjust path format and numbering |
| Artist name varies | artist_credit vs artist confusion | Check artist_credit setting |
| Duplicate albums | Path format collisions | Use %aunique{} disambiguation |

### 5.3 Database Health Warning Signs

| Sign | Indication | Action |
|------|------------|--------|
| Query returns wrong items | Index corruption | Run integrity check |
| Import creates duplicates | Duplicate detection misconfigured | Adjust duplicate_keys |
| Items disappear after update | Database corruption | Backup and restore |
| Slow queries (>1 sec) | Missing indexes or large DB | Optimize query patterns |

---

## 6. Mitigation Strategies Summary

### 6.1 Import Process Mitigation

```yaml
# Recommended configuration for robust imports
import:
    write: yes
    copy: yes
    resume: ask
    incremental: yes
    timid: no
    log: beets_import.log
    
    duplicate_action: ask
    duplicate_keys:
        album: albumartist album
        item: artist title
        
match:
    strong_rec_thresh: 0.04
    preferred:
        countries: ['US', 'GB']
        media: ['CD', 'Digital Media']
        original_year: yes
        
plugins: musicbrainz chroma lastgenre

chroma:
    auto: no  # Manual fingerprinting only
    
threaded: yes
```

### 6.2 Audio Encoding Mitigation

```python
# Encoding-safe tag writing
def safe_write_tags(file_path: str, metadata: dict):
    try:
        audio = mutagen.File(file_path, easy=True)
        
        # Force UTF-8 encoding
        for key, value in metadata.items():
            if isinstance(value, str):
                audio[key] = value.encode('utf-8').decode('utf-8')
            elif isinstance(value, list):
                audio[key] = [v.encode('utf-8').decode('utf-8') for v in value]
        
        # Use ID3v2.3 for compatibility if needed
        if file_path.endswith('.mp3'):
            audio.save(v2_version=3, v23_sep='/')
        else:
            audio.save()
            
    except MutagenError as e:
        log_error(f"Tag write failed: {e}")
        # Fallback: try without encoding conversion
        audio.save()
```

### 6.3 LLM Integration Mitigation

```python
class SafeLLMTagger:
    def __init__(self):
        self.validator = MetadataValidator()
        self.cache = ResponseCache()
        self.rate_limiter = RateLimiter()
        
    def tag_file(self, file_path: str) -> TagResult:
        # 1. Check cache first
        cached = self.cache.get(file_path)
        if cached and self.validator.validate(cached):
            return TagResult(metadata=cached, source='cache')
        
        # 2. Try MusicBrainz (free)
        mb_result = self.musicbrainz_search(file_path)
        if mb_result.confidence > 0.9:
            return TagResult(metadata=mb_result.metadata, source='musicbrainz')
        
        # 3. Use LLM only when necessary
        if self.rate_limiter.can_call():
            llm_result = self.llm_generate(file_path)
            
            # 4. Validate LLM output
            validation = self.validator.validate(llm_result)
            
            if validation.needs_review:
                return TagResult(
                    metadata=llm_result,
                    source='llm_unverified',
                    needs_manual_review=True,
                    confidence=validation.confidence
                )
            
            # 5. Cache successful result
            self.cache.set(file_path, llm_result)
            return TagResult(metadata=llm_result, source='llm_verified')
        
        return TagResult(metadata=None, source='failed', error='rate_limit')
```

### 6.4 Performance Mitigation

```python
class OptimizedImporter:
    def __init__(self, config: ImportConfig):
        self.batch_size = config.batch_size  # 500
        self.cache = MusicBrainzCache()
        self.rate_limiter = RateLimiter(max_rate=1.0)
        self.token_budget = TokenBudget(daily_limit=1000000)
        
    def import_library(self, library_path: str):
        # Stream files to avoid memory issues
        file_stream = self.stream_files(library_path)
        
        # Process in batches
        for batch in self.chunk_stream(file_stream, self.batch_size):
            # Check budget before batch
            if not self.token_budget.can_afford(self.estimate_batch_cost(batch)):
                log_warning("Token budget exhausted, pausing")
                break
            
            # Process batch
            results = self.process_batch(batch)
            
            # Commit transaction
            self.commit_batch(results)
            
            # Cleanup
            self.cleanup_batch_resources()
            
    def estimate_batch_cost(self, batch: list) -> int:
        # Estimate tokens needed for batch
        avg_tokens_per_file = 700
        uncertain_files = sum(1 for f in batch if f.uncertainty > 0.5)
        
        # Only LLM-process uncertain files
        return uncertain_files * avg_tokens_per_file
```

---

## 7. Performance Tips

### 7.1 Import Speed Optimization

1. **Disable Chromaprint for fast imports:**
   ```yaml
   chroma:
       auto: no
   ```

2. **Use incremental mode:**
   ```yaml
   import:
       incremental: yes
   ```

3. **Increase batch size:**
   ```yaml
   import:
       batch_size: 1000  # Default is smaller
   ```

4. **Disable unnecessary plugins:**
   ```yaml
   plugins: musicbrainz  # Remove lyrics, lastgenre if not needed
   ```

5. **Use local MusicBrainz mirror:**
   - Set up local MusicBrainz database
   - Configure beets to use local server
   - Eliminates rate limiting entirely

### 7.2 Query Optimization

1. **Use indexed fields in queries:**
   ```bash
   beet ls artist:Beatles album:Help  # Fast (indexed)
   beet ls title:Yesterday            # Slower (not indexed)
   ```

2. **Avoid regex queries on large libraries:**
   ```bash
   # Slow
   beet ls artist::Beatles
   
   # Fast
   beet ls artist:Beatles
   ```

3. **Use limit for large result sets:**
   ```bash
   beet ls -l 100 artist:Various  # Limit to 100 results
   ```

### 7.3 Database Optimization

1. **Regular maintenance:**
   ```bash
   beet check  # Integrity check
   sqlite3 library.db "PRAGMA optimize"
   ```

2. **Vacuum periodically:**
   ```bash
   sqlite3 library.db "VACUUM"
   ```

3. **Index optimization:**
   ```sql
   CREATE INDEX idx_artist_album ON items (artist, album);
   CREATE INDEX idx_albumartist ON albums (albumartist);
   ```

---

## 8. Quick Reference: Most Common Issues

### Top 10 Pitfalls:

1. **Wrong ID3 version:** Use `id3v23: yes` for Windows compatibility
2. **Rate limiting:** Implement proper User-Agent and rate limiting
3. **Missing matches:** Enable musicbrainz, chroma, discogs plugins
4. **Duplicate albums:** Use `%aunique{}` in path format
5. **Encoding issues:** Force UTF-8, validate non-UTF8 sources
6. **Memory exhaustion:** Use generators, limit batch size
7. **Database corruption:** Regular backups, integrity checks
8. **LLM hallucination:** Validate against MusicBrainz, use structured output
9. **Cost explosion:** Tiered processing, caching, token budgets
10. **Multi-value tags:** Handle format differences (ID3 vs Vorbis)

### Emergency Recovery:

```bash
# Database corruption recovery
sqlite3 library.db ".recover" > recovered.sql
sqlite3 new_library.db < recovered.sql

# Import resume
beet import -p /music  # Resume previous import

# Reset and re-import
rm ~/.config/beets/library.db
beet import -AWC /music  # As-is, Write, Copy
```

---

## References

- Beets Documentation: https://beets.readthedocs.io/
- MusicBrainz API: https://musicbrainz.org/doc/MusicBrainz_API
- Mutagen Library: https://mutagen.readthedocs.io/
- ID3v2.4 Specification: https://id3.org/id3v2.4.0-structure
- Chromaprint/Acoustid: https://acoustid.org/chromaprint
- OpenAI API Best Practices: https://platform.openai.com/docs/guides