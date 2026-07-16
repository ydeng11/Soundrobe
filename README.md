# Auto Tagger

Intelligent audio metadata editor for Navidrome-oriented libraries. The maintained desktop app uses Tauri 2, Rust, React, and TypeScript. A legacy Python CLI remains available for batch workflows.

## Desktop app

Prerequisites: Node.js, npm, Rust, and the platform dependencies required by Tauri 2.

```bash
just fe-install
just fe-dev
```

Quality gate and production build:

```bash
just fe-check
just fe-build
```

The Tauri backend is in `frontend/src-tauri`; the React renderer is in `frontend/src`. Electron and native Node modules are not used.

## Installation

```bash
pip install auto-tagger
```

External tools used by optional quality features:

- `ffmpeg` / `ffprobe` for audio validation
- `rgain3` or `loudgain` for ReplayGain calculation
- `aria2c` and `7z` for optional local MusicMoveArr dataset setup

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

## Local Dataset Lookup

Auto Tagger can use the MusicMoveArr dataset before remote Beets/MusicBrainz
lookup. Dataset setup is explicit because the upstream archives are large.

```bash
# Show whether the local dataset index is installed
auto-tag dataset status

# Preview the dataset setup plan without downloading
auto-tag dataset setup --dry-run

# Download the default MusicBrainz dataset and build the local SQLite index
auto-tag dataset setup
```

By default, setup stores files under `~/.auto-tagger`:

- `~/.auto-tagger/datasets/` for downloaded archives
- `~/.auto-tagger/staging/` for extracted CSV files
- `~/.auto-tagger/dataset-index.sqlite` for lookup
- `~/.auto-tagger/dataset-state.json` for setup status

Override the location with `AUTO_TAG_DATA_DIR` or `data_dir` in config.
When the local index is missing, tagging prints a warning and continues with
the existing API fallback.

Configuration loads from CLI args, environment variables, YAML config, then defaults.
The default config location is `~/.auto-tagger/config.yaml`.

Common environment variables:

```bash
AUTO_TAG_LLM_API_KEY=...
AUTO_TAG_OUTPUT_FORMAT=table
AUTO_TAG_FFPROBE_PATH=ffprobe
AUTO_TAG_REPLAYGAIN_COMMAND=rgain3
AUTO_TAG_DATA_DIR=~/.auto-tagger
```

## Legacy CLI development

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
