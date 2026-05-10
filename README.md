# Auto Tagger

Intelligent audio file tagging CLI tool for Navidrome-oriented libraries.

## Installation

```bash
pip install auto-tagger
```

External tools used by optional quality features:

- `ffmpeg` / `ffprobe` for audio validation
- `rgain3` or `loudgain` for ReplayGain calculation

## Usage

```bash
# Preview one album without writing
auto-tag tag /path/to/Artist/Album --dry-run

# Preview and write a machine-readable health report
auto-tag tag /path/to/Artist/Album --dry-run --health-report health.json

# Prompt before applying changes
auto-tag tag /path/to/Artist/Album --interactive

# Apply safe changes without prompts
auto-tag tag /path/to/Artist/Album --yolo

# Preview a full library
auto-tag batch /path/to/library --dry-run
```

Configuration loads from CLI args, environment variables, YAML config, then defaults.
The default config location is `~/.config/auto-tagger/config.yaml`.

Common environment variables:

```bash
AUTO_TAG_LLM_API_KEY=...
AUTO_TAG_OUTPUT_FORMAT=table
AUTO_TAG_FFPROBE_PATH=ffprobe
AUTO_TAG_REPLAYGAIN_COMMAND=rgain3
```

## Development

```bash
python -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
pytest
auto-tag --help
```

## Release

Release packaging is documented in `docs/release-checklist.md`. Homebrew formula
template lives in `packaging/homebrew/auto-tagger.rb`.

## License

MIT
