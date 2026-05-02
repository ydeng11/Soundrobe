# Auto Tagger

Intelligent audio file tagging CLI tool.

## Installation

```bash
pip install auto-tagger
```

## Development

```bash
# Clone repository
git clone https://github.com/yourusername/auto-tagger.git
cd auto-tagger

# Create virtual environment
python -m venv .venv
source .venv/bin/activate  # or `.\.venv\Scripts\activate` on Windows

# Install in development mode
pip install -e ".[dev]"

# Run tests
pytest

# Run CLI
auto-tag --help
```

## Usage

```bash
# Tag a single album
auto-tag tag /path/to/album

# Batch process library
auto-tag batch /path/to/library

# View configuration
auto-tag config
```

## License

MIT