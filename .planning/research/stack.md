# Soundrobe Tech Stack Research

## Executive Summary

Recommended stack for an soundrobe CLI using beets + LLM (OpenRouter):
- **Audio Metadata**: mutagen (primary), eyeD3 (MP3 fallback)
- **ReplayGain**: rgain3
- **CLI Framework**: Typer + Rich
- **LLM Integration**: OpenRouter Python SDK
- **Configuration**: TOML via tomllib (Python 3.11+) or tomli
- **Beets Integration**: Use as library via `beets.library` and `beets.autotag`

---

## 1. Python Audio Libraries

### 1.1 Metadata Reading/Writing

#### **mutagen** (Recommended - Primary Choice)

**Source**: https://mutagen.readthedocs.io/

**Pros**:
- Supports 15+ audio formats: ASF, FLAC, MP4, MP3, Musepack, Ogg (Opus/FLAC/Speex/Theora/Vorbis), True Audio, WavPack, OptimFROG, AIFF
- All ID3v2 versions fully supported (v2.2, v2.3, v2.4)
- No external dependencies - pure Python
- Battle-tested: Used by beets, Picard, Quod Libet, Puddletag
- Simple dictionary-like API across all formats
- GPL-2.0-or-later license
- Python 3.10+ support (CPython and PyPy)
- Reads Xing headers for accurate MP3 bitrate/length
- Can manipulate Ogg streams at packet/page level

**Cons**:
- Higher-level abstractions require more code than eyeD3 for MP3
- GPL license (consider compatibility with your project)

**Installation**:
```bash
pip install mutagen
```

**Example Usage**:
```python
import mutagen
from mutagen.easyid3 import EasyID3

# Simple MP3 tagging
audio = EasyID3("song.mp3")
audio['title'] = 'Song Title'
audio['artist'] = 'Artist Name'
audio['album'] = 'Album Name'
audio['tracknumber'] = '1'
audio.save()

# Format-specific access
from mutagen.mp3 import MP3
audio = MP3("song.mp3")
audio.tags.add(mutagen.id3.TIT2(encoding=3, text="Title"))
audio.save()
```

#### **eyeD3** (Alternative - MP3 Specialist)

**Source**: https://eyed3.readthedocs.io/

**Pros**:
- MP3-focused with excellent ID3 support
- CLI tool included (`eyeD3` command)
- Python library (`import eyed3`)
- ID3 v1.x, 2.2 (read-only), 2.3, 2.4 support
- Plugin system for extensibility
- Built-in plugins: art download, stats, fixup, NFO generation, JSON/YAML output
- MIT-style GPL license

**Cons**:
- MP3-only (not multi-format)
- Higher-level but less flexible than mutagen
- Python 3.9+ required

**Installation**:
```bash
pip install eyeD3
```

**Example Usage**:
```python
import eyed3

audiofile = eyed3.load("song.mp3")
audiofile.tag.artist = "Artist"
audiofile.tag.album = "Album"
audiofile.tag.title = "Title"
audiofile.tag.track_num = 1
audiofile.tag.save()
```

### 1.2 Audio Validation/Corruption Detection

**Approaches**:

1. **mutagen-based validation**:
```python
from mutagen import File
from mutagen.mp3 import MP3, error as MP3Error

def validate_mp3(path):
    try:
        audio = MP3(path)
        # Check for valid header
        audio.info.bitrate
        audio.info.length
        return True, None
    except MP3Error as e:
        return False, str(e)
    except Exception as e:
        return False, f"Corrupted: {e}"
```

2. **pydub** (for audio analysis):
- Source: https://pydub.com
- MIT License
- Requires ffmpeg or libav
- Good for detecting truncated/corrupted files
```python
from pydub import AudioSegment

def validate_audio(path):
    try:
        audio = AudioSegment.from_file(path)
        return len(audio) > 0  # Check duration
    except:
        return False
```

**Recommendation**: Use mutagen for basic validation (check if file loads, has valid info). Use pydub for deep validation if ffmpeg is available.

### 1.3 ReplayGain Calculation

#### **rgain3** (Recommended)

**Source**: https://pypi.org/project/rgain3/

**Pros**:
- Multi-format support: Ogg Vorbis, FLAC, WavPack, MP4/AAC, MP3
- Uses GStreamer for accurate calculation
- Writes tags via mutagen
- CLI tools included: `replaygain`, `collectiongain`
- GPL-2.0-or-later license
- Python 3.5+ support

**Cons**:
- Requires GStreamer (system dependency)
- GPL license

**Installation**:
```bash
pip install rgain3
# Requires GStreamer plugins
# Ubuntu: apt install gstreamer1.0-plugins-good gstreamer1.0-plugins-base
# macOS: brew install gstreamer
```

**Example Usage**:
```python
from rgain3 import rgain

# Calculate and apply ReplayGain
rgain.calculate_replaygain(
    ["song1.mp3", "song2.flac"],
    album=True  # Album mode
)
```

**Alternative**: Use beets' ReplayGain plugin (built-in).

### 1.4 Lyrics/LRC File Handling

**Options**:

1. **Embedded lyrics via mutagen**:
```python
# MP3 - USLT frame (Unsynchronized Lyrics)
from mutagen.id3 import USLT
audio = MP3("song.mp3")
audio.tags.add(USLT(encoding=3, lang='eng', desc='', text="Lyrics text"))
audio.save()

# FLAC - LYRICS tag
from mutagen.flac import FLAC
audio = FLAC("song.flac")
audio['LYRICS'] = ["Lyrics text"]
audio.save()
```

2. **LRC file parsing** (simple Python implementation):
```python
import re

def parse_lrc(filepath):
    """Parse .lrc file into timed lyrics."""
    lyrics = []
    pattern = r'\[(\d{2}):(\d{2})\.(\d{2,3})\](.*)'
    
    with open(filepath) as f:
        for line in f:
            match = re.match(pattern, line.strip())
            if match:
                mins, secs, ms, text = match.groups()
                timestamp = int(mins)*60 + int(secs) + int(ms)/100
                lyrics.append((timestamp, text))
    return sorted(lyrics)

def generate_lrc(lyrics, filepath):
    """Generate .lrc file from timed lyrics."""
    with open(filepath, 'w') as f:
        for timestamp, text in lyrics:
            mins = int(timestamp // 60)
            secs = int(timestamp % 60)
            ms = int((timestamp % 1) * 100)
            f.write(f"[{mins:02d}:{secs:02d}.{ms:02d}]{text}\n")
```

**No dedicated library needed** - LRC format is simple text format. Implement custom parser/generator.

---

## 2. Beets Integration

### 2.1 Beets as a Library

**Key modules** (from beets source structure):
- `beets.library` - Database and item management
- `beets.autotag` - MusicBrainz matching logic
- `beets.importer` - Import workflow
- `beets.plugins` - Plugin system
- `beets.ui` - CLI commands (avoid for library use)
- `beets.config_default.yaml` - Default configuration

**Library Initialization**:
```python
from beets import library, config
from beets.autotag import TagMatch, Proposal
import os

# Initialize beets library programmatically
config.clear()
config.read(user_config=False)  # Don't read user config
config['directory'] = '/path/to/music'
config['library'] = '/path/to/library.db'

lib = library.Library(
    config['library'].as_filename(),
    config['directory'].as_filename()
)

# Add item to library
item = library.Item.from_path('/path/to/song.mp3')
lib.add(item)
```

### 2.2 Accessing MusicBrainz Match Candidates

**Autotagger API** (from `beets.autotag`):
```python
from beets.autotag import match, Proposal
from beets.autotag.mb import match_album, match_track

# Get match candidates from MusicBrainz
def get_candidates(artist, album):
    candidates = match_album(
        artist, 
        album,
        config['match']['search_limit'].get(int)
    )
    
    for candidate in candidates:
        # Each candidate has:
        # - artist, album, tracks
        # - mb_albumid, mb_artistid
        # - distance (match quality score)
        print(f"Match: {candidate.artist} - {candidate.album}")
        print(f"MB ID: {candidate.mb_albumid}")
        print(f"Distance: {candidate.distance}")
    
    return candidates

# Match single track
def match_single_track(path):
    from beets.autotag import match_track
    candidates = match_track(
        path,
        config['match']['search_limit'].get(int)
    )
    return candidates
```

**Match Workflow**:
```python
from beets.autotag import tag_item, Proposal

def autotag_file(filepath):
    """Tag a single file using beets soundrobe."""
    candidates = match_track(filepath)
    
    if candidates:
        best_match = candidates[0]  # Highest similarity
        
        # Apply metadata
        item = library.Item.from_path(filepath)
        item.update(best_match.info)
        item.write()  # Write tags to file
        return best_match
    
    return None
```

### 2.3 Plugin Integration

**Creating a beets plugin**:
```python
from beets.plugins import BeetsPlugin

class SoundrobePlugin(BeetsPlugin):
    def __init__(self):
        super().__init__()
        self.register_listener('import_task_apply', self.on_apply)
    
    def on_apply(self, task, session):
        """Hook into import workflow."""
        # Custom logic here
        pass
    
    def commands(self):
        """Add CLI commands."""
        # Not needed for library use
        return []
```

### 2.4 Best Practices

1. **Configuration**: Initialize beets config programmatically to avoid user config conflicts
2. **Thread safety**: Beets library operations are not thread-safe; use locks if needed
3. **Database**: Use SQLite database for persistence (default)
4. **Caching**: Beets caches MusicBrainz responses; respect cache settings
5. **Plugins**: Load needed plugins explicitly: `plugins.load_plugins()`
6. **Avoid UI modules**: Don't use `beets.ui` for library integration

**Potential Issues**:
- Beets expects certain config values; provide defaults
- MusicBrainz API has rate limits (1 req/sec)
- Match quality depends on existing metadata quality
- Some plugins modify file structure; be aware of side effects

---

## 3. OpenRouter SDK

### 3.1 Python SDK

**Source**: https://openrouter.ai/docs/client-sdks/python/overview.mdx

**Installation**:
```bash
pip install openrouter
# or
uv add openrouter
```

**Features**:
- Auto-generated from OpenAPI specs (always up-to-date)
- Type-safe with Pydantic validation
- Async support (`send_async`)
- Streaming support
- Python 3.9+ required

**Example Usage**:
```python
from openrouter import OpenRouter
import os

with OpenRouter(api_key=os.getenv("OPENROUTER_API_KEY")) as client:
    response = client.chat.send(
        model="anthropic/claude-3.5-sonnet",
        messages=[
            {"role": "system", "content": "You are a music metadata expert."},
            {"role": "user", "content": "Generate tags for this audio file..."}
        ],
        temperature=0.3
    )
    
    print(response.choices[0].message.content)
```

**Async Usage**:
```python
import asyncio

async def generate_tags():
    async with OpenRouter(api_key=os.getenv("OPENROUTER_API_KEY")) as client:
        response = await client.chat.send_async(
            model="anthropic/claude-3.5-sonnet",
            messages=[...]
        )
        return response.choices[0].message.content

# Run
tags = asyncio.run(generate_tags())
```

### 3.2 Cost-Efficient Models for Metadata

**Recommended models** (based on cost/performance):

| Model | Use Case | Cost Tier | Notes |
|-------|----------|-----------|-------|
| `google/gemini-2.0-flash-lite` | Quick categorization | ~$0.0001/1K tokens | Fast, cheap |
| `meta-llama/llama-3.2-3b-instruct` | Basic tagging | Free tier available | Good for simple tasks |
| `anthropic/claude-3.5-haiku` | Accurate metadata | ~$0.001/1K tokens | Best accuracy/cost ratio |
| `openai/gpt-4o-mini` | General metadata | ~$0.0002/1K tokens | Good balance |
| `anthropic/claude-3.5-sonnet` | Complex disambiguation | ~$0.003/1K tokens | When accuracy critical |

**Cost optimization tips**:
1. Use `:free` variant for development: `anthropic/claude-3.5-haiku:free`
2. Use smaller models for initial categorization
3. Cache responses (OpenRouter supports response caching)
4. Batch requests when possible
5. Use structured outputs to reduce token waste

### 3.3 Prompting Strategies for Tag Generation

**System Prompt Template**:
```python
METADATA_SYSTEM_PROMPT = """
You are a music metadata expert. Analyze audio file information and generate accurate, 
standardized metadata tags following these rules:

1. Use standard MusicBrainz field names
2. Normalize artist names (remove "The", standardize spellings)
3. Guess release year from context if missing
4. Identify genre using standard ID3 genres
5. Clean up track/disc numbers (numeric only)
6. Apply title capitalization standards

Output JSON with these fields:
- title, artist, album, albumartist
- year, genre, tracknumber, discnumber
- mb_trackid (if confident), mb_albumid, mb_artistid
"""
```

**User Prompt Template**:
```python
def build_prompt(file_info):
    """Build prompt from file metadata."""
    return f"""
Analyze this audio file and suggest metadata:

Filename: {file_info['filename']}
Existing tags:
- Title: {file_info.get('title', 'Unknown')}
- Artist: {file_info.get('artist', 'Unknown')}
- Album: {file_info.get('album', 'Unknown')}
- Duration: {file_info.get('duration', 'Unknown')} seconds
- Bitrate: {file_info.get('bitrate', 'Unknown')} kbps

Generate accurate metadata in JSON format.
"""
```

**Structured Output** (recommended):
```python
# Use OpenRouter's structured outputs
response = client.chat.send(
    model="anthropic/claude-3.5-haiku",
    messages=[...],
    response_format={
        "type": "json_schema",
        "json_schema": {
            "name": "metadata",
            "schema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "artist": {"type": "string"},
                    "album": {"type": "string"},
                    "year": {"type": "integer"},
                    "genre": {"type": "string"},
                    "tracknumber": {"type": "integer"}
                },
                "required": ["title", "artist"]
            }
        }
    }
)
```

---

## 4. CLI Frameworks

### 4.1 Click vs Typer vs argparse

| Framework | Pros | Cons | Recommendation |
|-----------|------|------|----------------|
| **Typer** | Type hints, auto-help, Rich integration, FastAPI-like API, shell completion | Depends on Click + Rich + shellingham | **Recommended** |
| **Click** | Battle-tested, mature, extensive docs, decorator-based | More verbose, manual type handling | Good alternative |
| **argparse** | Built-in, no dependencies | Verbose, limited features, manual help generation | Avoid for new projects |

#### **Typer** (Recommended)

**Source**: https://typer.tiangolo.com/

**Features**:
- Built on Click (stable foundation)
- Rich integration (beautiful output by default)
- Type hints = auto CLI arguments/options
- Automatic help generation
- Shell completion (bash, zsh, fish, PowerShell)
- MIT license

**Installation**:
```bash
pip install typer
# Includes: click, rich, shellingham
```

**Example**:
```python
import typer
from pathlib import Path
from typing import Optional, List

app = typer.Typer()

@app.command()
def tag(
    path: Path,
    model: str = "anthropic/claude-3.5-haiku",
    dry_run: bool = False,
    force: bool = typer.Option(False, "--force", "-f", help="Overwrite existing tags"),
    genres: Optional[List[str]] = typer.Option(None, help="Restrict to these genres"),
):
    """Tag audio files using LLM."""
    if dry_run:
        typer.echo(f"[dry-run] Would tag: {path}")
    else:
        typer.echo(f"Tagging: {path}")
        # Implementation...

@app.command()
def batch(
    directory: Path,
    recursive: bool = True,
):
    """Batch tag all files in directory."""
    # Implementation...

if __name__ == "__main__":
    app()
```

### 4.2 Rich for CLI Output

**Source**: https://rich.readthedocs.io/

**Features**:
- Beautiful formatted output
- Progress bars, tables, trees
- Syntax highlighting
- Markdown rendering
- Traceback formatting
- Works with Typer seamlessly

**Example**:
```python
from rich.console import Console
from rich.progress import track
from rich.table import Table

console = Console()

# Progress bar
for file in track(files, description="Tagging..."):
    tag_file(file)

# Table output
table = Table(title="Tagging Results")
table.add_column("File", style="cyan")
table.add_column("Status", style="green")
table.add_row("song.mp3", "✓ Tagged")
console.print(table)

# Status messages
console.print("[bold green]Success![/] Tagged 10 files")
console.print("[yellow]Warning:[/] No match found for song.mp3")
console.print("[red]Error:[/] Invalid audio format")
```

### 4.3 Textual for TUI (Optional)

**Source**: https://textual.textualize.io/

**Use case**: Interactive tagging interface (browse, select, approve tags)

**Features**:
- Full TUI framework (not just output)
- Widgets: tables, trees, forms, buttons
- Mouse/keyboard interaction
- CSS-like styling
- Good for review/approve workflows

**Example app structure**:
```python
from textual.app import App
from textual.widgets import DataTable, Button, Label

class TagReviewApp(App):
    """Interactive tag review interface."""
    
    def compose(self):
        return [
            DataTable(),
            Button("Approve", variant="primary"),
            Button("Reject", variant="error"),
        ]
    
    def on_mount(self):
        table = self.query_one(DataTable)
        table.add_columns("File", "Title", "Artist", "Status")
        # Populate with pending tags...
```

**Recommendation**: Use Typer + Rich for basic CLI. Consider Textual for interactive review mode.

### 4.4 Configuration Management

**TOML Configuration** (Recommended):

```python
# Python 3.11+: built-in tomllib
import tomllib

# Python <3.11: tomli
import tomli

def load_config(path: Path):
    with open(path, "rb") as f:
        return tomllib.load(f)  # or tomli.load(f)

def save_config(path: Path, config: dict):
    import tomli_w  # Needs separate package
    with open(path, "wb") as f:
        tomli_w.dump(config, f)
```

**Config file structure** (`config.toml`):
```toml
[model]
provider = "openrouter"
name = "anthropic/claude-3.5-haiku"
api_key_env = "OPENROUTER_API_KEY"

[tagging]
dry_run = false
force = false
genres = ["Rock", "Pop", "Electronic"]

[output]
format = "json"
verbose = true

[replaygain]
enabled = true
album_mode = true

[beets]
library = "~/.config/soundrobe/library.db"
directory = "~/Music"
```

**Alternative**: YAML via `pyyaml` (more common in Python projects, but TOML is cleaner).

---

## 5. Integration Patterns

### 5.1 Complete Architecture

```
soundrobe/
├── __init__.py
├── cli.py              # Typer CLI entry point
├── config.py           # Configuration management
├── audio/
│   ├── __init__.py
│   ├── metadata.py     # mutagen wrapper
│   ├── validation.py   # File validation
│   ├── replaygain.py   # rgain3 integration
│   └── lyrics.py       # LRC handling
├── matching/
│   ├── __init__.py
│   ├── musicbrainz.py  # Beets autotag integration
│   ├── llm.py          # OpenRouter integration
│   └── hybrid.py       # Combine both sources
├── output/
│   ├── __init__.py
│   ├── display.py      # Rich output formatting
│   └── export.py       # JSON/YAML/CSV export
└── plugins/
    └── __init__.py     # Future plugin system
```

### 5.2 Workflow Pattern

```python
# Main workflow
async def tag_file(path: Path, config: Config):
    """Complete tagging workflow."""
    
    # 1. Validate
    valid, error = validate_audio(path)
    if not valid:
        raise ValueError(f"Invalid file: {error}")
    
    # 2. Read existing metadata
    existing = read_metadata(path)
    
    # 3. Get MusicBrainz candidates (via beets)
    mb_candidates = get_musicbrainz_matches(existing)
    
    # 4. Enhance with LLM (OpenRouter)
    llm_tags = await generate_llm_tags(existing, mb_candidates)
    
    # 5. Merge/resolve conflicts
    final_tags = resolve_tags(mb_candidates, llm_tags, config)
    
    # 6. Apply (if not dry_run)
    if not config.dry_run:
        write_metadata(path, final_tags)
        
        # 7. Optional: ReplayGain
        if config.replaygain:
            calculate_replaygain([path])
    
    return final_tags
```

### 5.3 Error Handling

```python
from enum import Enum
from dataclasses import dataclass

class TagStatus(Enum):
    SUCCESS = "success"
    PARTIAL = "partial"
    FAILED = "failed"
    SKIPPED = "skipped"

@dataclass
class TagResult:
    path: Path
    status: TagStatus
    tags: Optional[dict]
    error: Optional[str]
    mb_match: Optional[str]
    llm_match: Optional[str]

def safe_tag(path: Path, config: Config) -> TagResult:
    """Tag with comprehensive error handling."""
    try:
        tags = tag_file(path, config)
        return TagResult(
            path=path,
            status=TagStatus.SUCCESS,
            tags=tags,
            error=None,
            mb_match=tags.get('mb_match'),
            llm_match=tags.get('llm_match')
        )
    except ValidationError as e:
        return TagResult(path=path, status=TagStatus.FAILED, tags=None, error=str(e))
    except MusicBrainzError as e:
        # MB failed, but try LLM-only
        tags = llm_only_tag(path, config)
        return TagResult(path=path, status=TagStatus.PARTIAL, tags=tags, error=f"MB: {e}")
    except Exception as e:
        return TagResult(path=path, status=TagStatus.FAILED, tags=None, error=str(e))
```

---

## 6. Potential Issues to Avoid

### 6.1 Audio Library Issues

1. **ID3 version conflicts**: MP3 files may have mixed ID3v1/v2 tags. Always use ID3v2.4.
   ```python
   # Force ID3v2.4
   audio = MP3(path, ID3=EasyID3)
   audio.tags.update_to_v24()
   ```

2. **Unicode encoding**: Use UTF-8 (encoding=3) for all ID3 text frames.
3. **Cover art**: Large images can slow tagging. Handle separately.
4. **Read-only files**: Check permissions before writing.
5. **Truncated files**: Validate before processing to avoid crashes.

### 6.2 Beets Integration Issues

1. **Config isolation**: Don't load user's beets config - use your own.
2. **Database conflicts**: Don't use existing beets library.db.
3. **Plugin hooks**: Some plugins auto-move files; disable unwanted plugins.
4. **MusicBrainz rate limits**: Implement throttling (1 req/sec max).
5. **Match quality**: Check `distance` value - reject low-quality matches.
6. **Thread safety**: Beets isn't thread-safe for concurrent operations.

### 6.3 OpenRouter Issues

1. **API key security**: Use environment variables, never hardcode.
2. **Rate limits**: OpenRouter has usage limits; implement backoff.
3. **Model availability**: Some models go offline; use fallbacks.
4. **Token limits**: Keep prompts concise; batch processing helps.
5. **Cost tracking**: Monitor usage; set budget limits.
6. **Response validation**: LLM output may be malformed; validate JSON.

### 6.4 CLI Issues

1. **Path handling**: Use `pathlib.Path`, not string paths.
2. **Encoding**: Set terminal encoding for Unicode output.
3. **Progress bars**: Handle cancellation gracefully.
4. **Shell completion**: Test on all target shells.
5. **Exit codes**: Use proper exit codes (0=success, 1=error).

---

## 7. Additional Libraries to Consider

| Library | Purpose | Status |
|---------|---------|--------|
| `tomli_w` | TOML writing | Needed for config saving |
| `pyyaml` | YAML config alternative | Optional |
| `python-dotenv` | Environment variable loading | Recommended |
| `rich-argparse` | Rich + argparse integration | If using argparse |
| `questionary` | Interactive prompts | For review workflows |
| `loguru` | Better logging | Optional |
| `tenacity` | Retry logic | For API calls |
| `httpx` | Async HTTP client | If not using OpenRouter SDK |
| `pydantic` | Data validation | Used by OpenRouter SDK |

---

## 8. Final Recommendations

### Core Stack
1. **mutagen** for metadata (multi-format, battle-tested)
2. **rgain3** for ReplayGain (or beets plugin)
3. **Typer + Rich** for CLI (modern, beautiful, type-safe)
4. **OpenRouter Python SDK** for LLM (type-safe, async)
5. **Beets library** for MusicBrainz matching (via autotag)

### Architecture
- Use async for LLM calls
- Implement retry/backoff for external APIs
- Cache MusicBrainz responses
- Separate validation from tagging
- Support dry-run mode

### Configuration
- TOML for config files
- Environment variables for secrets
- CLI flags override config

### Development Order
1. Metadata reading/writing (mutagen wrapper)
2. CLI structure (Typer)
3. MusicBrainz matching (beets integration)
4. LLM integration (OpenRouter)
5. Hybrid matching logic
6. ReplayGain and lyrics support
7. Batch processing
8. Interactive review mode (Textual)