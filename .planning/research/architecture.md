# CLI Architecture Patterns & Homebrew Distribution Research

## Recommended Project Structure

### Standard Python CLI Project Layout

```
soundrobe/
├── src/
│   └── soundrobe/
│       ├── __init__.py          # Package init, version
│       ├── __main__.py          # Entry point for `python -m soundrobe`
│       ├── cli.py               # Main CLI definitions
│       ├── commands/            # Subcommand implementations
│       │   ├── __init__.py
│       │   ├── tag.py           # `tag` subcommand
│       │   ├── config.py        # `config` subcommand
│       │   └── list.py          # `list` subcommand
│       ├── core/                 # Core business logic
│       │   ├── __init__.py
│       │   ├── tagger.py        # Tagging logic
│       │   ├── file_handler.py  # File operations
│       │   └── parser.py        # Tag parsing
│       ├── config/               # Configuration management
│       │   ├── __init__.py
│       │   ├── settings.py      # Pydantic settings model
│       │   └── loader.py        # Config file loading
│       ├── utils/                # Utilities
│       │   ├── __init__.py
│       │   ├── logging.py       # Logging setup
│       │   ├── output.py        # Output formatting
│       │   └── progress.py      # Progress reporting
│       └── exceptions.py        # Custom exceptions
├── tests/
│   ├── __init__.py
│   ├── conftest.py              # Test fixtures
│   ├── test_cli.py              # CLI tests
│   ├── test_commands/
│   │   ├── test_tag.py
│   │   ├── test_config.py
│   ├── test_core/
│   │   ├── test_tagger.py
│   │   ├── test_parser.py
├── pyproject.toml               # Project metadata & build config
├── README.md
├── LICENSE
├── .env.example                 # Example environment config
└── config.example.yaml          # Example YAML config
```

### Key Structural Decisions

1. **src layout**: Recommended by PyPA for:
   - Prevents import conflicts during testing
   - Cleaner separation between source and tests
   - Better for packaging

2. **Entry points pattern** (pyproject.toml):
   ```toml
   [project.scripts]
   auto-tag = "soundrobe.cli:main"
   
   [project.entry-points."soundrobe.plugins"]
   # Future plugin extensions
   ```

3. **Module organization**:
   - `cli.py`: CLI definitions only (thin layer)
   - `commands/`: Individual subcommand implementations
   - `core/`: Business logic (testable without CLI)
   - `config/`: Configuration management
   - `utils/`: Shared utilities

---

## CLI Design Patterns

### Framework Selection: Click vs Typer vs argparse

#### Click (Recommended)
- Battle-tested, widely adopted
- Composable subcommands via `@click.group()`
- Automatic help generation
- Context passing for nested commands
- Shell completion support

#### Typer
- Modern, type-hint based
- Built on Click + Rich
- Great for simpler CLIs
- Automatic validation from types

#### argparse
- Standard library (no dependencies)
- More verbose
- Less intuitive for complex CLIs

### Command Structure Pattern

```python
# cli.py - Main CLI entry point
import click
from rich.console import Console

console = Console()

@click.group()
@click.option('--verbose', '-v', is_flag=True, help='Enable verbose output')
@click.option('--config', '-c', type=click.Path(), help='Config file path')
@click.pass_context
def cli(ctx: click.Context, verbose: bool, config: str):
    """Soundrobe - Intelligent file tagging CLI."""
    ctx.ensure_object(dict)
    ctx.obj['verbose'] = verbose
    ctx.obj['config'] = config

@cli.command()
@click.argument('files', nargs=-1, type=click.Path(exists=True))
@click.option('--dry-run', is_flag=True, help='Preview without changes')
@click.option('--recursive', '-r', is_flag=True, help='Process directories recursively')
@click.pass_context
def tag(ctx: click.Context, files: tuple, dry_run: bool, recursive: bool):
    """Tag files with intelligent metadata."""
    from .commands.tag import execute
    execute(ctx.obj, files, dry_run, recursive)

@cli.command()
@click.argument('key', required=False)
@click.argument('value', required=False)
@click.pass_context
def config(ctx: click.Context, key: str, value: str):
    """View or modify configuration."""
    from .commands.config import execute
    execute(ctx.obj, key, value)

def main():
    cli()
```

### Configuration Management Pattern

#### Multi-source Configuration (Priority Order)
1. Command-line arguments (highest)
2. Environment variables
3. Config file (YAML/TOML)
4. Default values (lowest)

#### Pydantic Settings Implementation

```python
# config/settings.py
from pydantic import Field
from pydantic_settings import BaseSettings, SettingsConfigDict
from pathlib import Path

class Settings(BaseSettings):
    model_config = SettingsConfigDict(
        env_prefix='AUTO_TAG_',
        env_file='.env',
        env_file_encoding='utf-8',
        env_nested_delimiter='__',
        case_sensitive=False,
    )
    
    # Core settings
    default_tags: list[str] = Field(default_factory=list)
    output_format: str = 'json'  # json, yaml, plain
    verbose: bool = False
    
    # File handling
    recursive_depth: int = 10
    exclude_patterns: list[str] = Field(default=['*.git', '__pycache__'])
    
    # Paths
    config_file: Path | None = Field(default=None, alias='c')
    
    # API settings (if applicable)
    api_key: str | None = Field(default=None, validation_alias='api_token')
    api_endpoint: str = 'https://api.example.com'

# Usage
settings = Settings()
# Can override: Settings(_env_file='prod.env', verbose=True)
```

### Logging & Output Formatting Pattern

```python
# utils/logging.py
import logging
from rich.console import Console
from rich.logging import RichHandler

def setup_logging(verbose: bool = False) -> logging.Logger:
    """Configure logging with Rich formatting."""
    level = logging.DEBUG if verbose else logging.INFO
    
    logging.basicConfig(
        level=level,
        format="%(message)s",
        datefmt="[%X]",
        handlers=[RichHandler(rich_tracebacks=True, markup=True)]
    )
    
    return logging.getLogger("soundrobe")

# utils/output.py
from rich.console import Console
from rich.table import Table
from rich.panel import Panel
from rich import print as rprint

console = Console()

def output_results(results: list[dict], format: str = 'table'):
    """Format and output results."""
    if format == 'table':
        table = Table(title="Tagging Results")
        table.add_column("File", style="cyan")
        table.add_column("Tags", style="green")
        for result in results:
            table.add_row(result['file'], str(result['tags']))
        console.print(table)
    elif format == 'json':
        rprint(results)
    elif format == 'yaml':
        import yaml
        print(yaml.dump(results, default_flow_style=False))
```

### Progress Reporting Pattern

```python
# utils/progress.py
from rich.progress import Progress, SpinnerColumn, TextColumn, BarColumn
from rich.console import Console

console = Console()

def process_files_with_progress(files: list, processor: callable):
    """Process files with progress bar."""
    with Progress(
        SpinnerColumn(),
        TextColumn("[progress.description]{task.description}"),
        BarColumn(),
        TextColumn("[progress.percentage]{task.percentage:>3.0f}%"),
        console=console,
        transient=True,  # Clears after completion
    ) as progress:
        task = progress.add_task("Processing files...", total=len(files))
        
        for file in files:
            result = processor(file)
            progress.update(task, advance=1, description=f"Processing {file.name}")
            
        return results

# Alternative: Simple track
from rich.progress import track

for file in track(files, description="Processing..."):
    process_file(file)
```

### Error Handling Pattern

```python
# exceptions.py
class SoundrobeError(Exception):
    """Base exception for soundrobe."""
    exit_code = 1

class ConfigError(SoundrobeError):
    """Configuration related errors."""
    exit_code = 2

class FileNotFoundError(SoundrobeError):
    """File processing errors."""
    exit_code = 3

class ValidationError(SoundrobeError):
    """Validation errors."""
    exit_code = 4

# cli.py - Error handling in CLI
import sys
from click import ClickException
from rich.console import Console

console = Console()

@cli.command()
@click.pass_context
def tag(ctx, files):
    """Tag files with error handling."""
    try:
        from .commands.tag import execute
        execute(ctx.obj, files)
    except SoundrobeError as e:
        console.print(f"[red]Error:[/red] {e}")
        sys.exit(e.exit_code)
    except Exception as e:
        if ctx.obj.get('verbose'):
            console.print_exception()  # Full traceback
        else:
            console.print(f"[red]Unexpected error:[/red] {e}")
            console.print("Run with --verbose for details")
        sys.exit(1)

# Custom Click exception for better messages
class TagError(ClickException):
    def __init__(self, message: str, suggestion: str = None):
        super().__init__(message)
        self.suggestion = suggestion
    
    def show(self, file=None):
        console = Console(file=file)
        console.print(f"[red]Error:[/red] {self.message}")
        if self.suggestion:
            console.print(f"[yellow]Suggestion:[/yellow] {self.suggestion}")
```

### Exit Codes Standard

| Code | Meaning | Usage |
|------|---------|-------|
| 0 | Success | Operation completed |
| 1 | General error | Unspecified failure |
| 2 | Usage error | Invalid arguments |
| 3 | File error | File not found, permission denied |
| 4 | Config error | Configuration invalid |
| 5 | Validation error | Input validation failed |
| 64-78 | Standard BSD | See sysexits.h |
| 125 | Script error | Command not executable |

### Graceful Degradation Strategies

```python
# core/tagger.py
def process_file(file_path: Path, fallback: bool = True) -> dict:
    """Process file with graceful degradation."""
    try:
        # Primary method: Full AI tagging
        return ai_tag_file(file_path)
    except APIError as e:
        if not fallback:
            raise
        
        # Fallback: Rule-based tagging
        logger.warning(f"API unavailable, using fallback: {e}")
        return rule_based_tag(file_path)
    except Exception as e:
        # Last resort: Basic metadata extraction
        logger.warning(f"Fallback failed, using basic: {e}")
        return basic_metadata(file_path)

def batch_process(files: list[Path], fail_fast: bool = False) -> list[dict]:
    """Batch processing with error handling."""
    results = []
    errors = []
    
    for file in files:
        try:
            result = process_file(file)
            results.append(result)
        except Exception as e:
            if fail_fast:
                raise
            errors.append({'file': file, 'error': str(e)})
    
    if errors:
        console.print(f"[yellow]Completed with {len(errors)} errors[/yellow]")
    
    return results, errors
```

---

## Homebrew Distribution Steps

### Step 1: PyPI Packaging Requirements

#### pyproject.toml (Complete Example)

```toml
[build-system]
requires = ["hatchling >= 1.26"]
build-backend = "hatchling.build"

[project]
name = "soundrobe"
version = "1.0.0"
description = "Intelligent file tagging CLI tool"
readme = "README.md"
license = "MIT"
license-files = ["LICEN[CS]E*"]
authors = [
    { name = "Your Name", email = "you@example.com" }
]
requires-python = ">=3.10"
classifiers = [
    "Development Status :: 4 - Beta",
    "Environment :: Console",
    "Intended Audience :: Developers",
    "License :: OSI Approved :: MIT License",
    "Operating System :: OS Independent",
    "Programming Language :: Python :: 3",
    "Programming Language :: Python :: 3.10",
    "Programming Language :: Python :: 3.11",
    "Programming Language :: Python :: 3.12",
    "Topic :: Utilities",
]
keywords = ["cli", "tagging", "metadata", "files"]

dependencies = [
    "click>=8.1.0",
    "rich>=13.0.0",
    "pydantic>=2.0.0",
    "pydantic-settings>=2.0.0",
    "pyyaml>=6.0",
]

[project.optional-dependencies]
dev = [
    "pytest>=7.0",
    "pytest-cov>=4.0",
    "mypy>=1.0",
    "ruff>=0.1.0",
]

[project.scripts]
auto-tag = "soundrobe.cli:main"

[project.urls]
Homepage = "https://github.com/yourusername/soundrobe"
Documentation = "https://github.com/yourusername/soundrobe#readme"
Repository = "https://github.com/yourusername/soundrobe.git"
Issues = "https://github.com/yourusername/soundrobe/issues"

[tool.hatch.build.targets.wheel]
packages = ["src/soundrobe"]

[tool.hatch.build.targets.sdist]
include = ["src/", "README.md", "LICENSE"]
```

### Step 2: Build & Publish to PyPI

```bash
# Install build tools
pip install build twine

# Build distributions
python -m build
# Creates:
#   dist/soundrobe-1.0.0-py3-none-any.whl
#   dist/soundrobe-1.0.0.tar.gz

# Check distribution
twine check dist/*

# Upload to TestPyPI first
twine upload --repository testpypi dist/*

# Test installation
pip install --index-url https://test.pypi.org/simple/ soundrobe

# Upload to PyPI
twine upload dist/*
```

### Step 3: Create Homebrew Formula

#### Basic Formula Structure

```ruby
# Formula/soundrobe.rb
class Soundrobe < Formula
  include Language::Python::Virtualenv

  desc "Intelligent file tagging CLI tool"
  homepage "https://github.com/yourusername/soundrobe"
  url "https://files.pythonhosted.org/packages/source/a/soundrobe/soundrobe-1.0.0.tar.gz"
  sha256 "..." # Calculate with: sha256sum dist/soundrobe-1.0.0.tar.gz
  license "MIT"

  # Python version requirement
  depends_on "python@3.12"

  # Declare all Python dependencies as resources
  resource "click" do
    url "https://files.pythonhosted.org/packages/..."
    sha256 "..."
  end

  resource "rich" do
    url "https://files.pythonhosted.org/packages/..."
    sha256 "..."
  end

  resource "pydantic" do
    url "https://files.pythonhosted.org/packages/..."
    sha256 "..."
  end

  resource "pydantic-settings" do
    url "https://files.pythonhosted.org/packages/..."
    sha256 "..."
  end

  resource "pyyaml" do
    url "https://files.pythonhosted.org/packages/..."
    sha256 "..."
  end

  def install
    # Create virtualenv and install with resources
    virtualenv_install_with_resources
  end

  test do
    # Basic functionality test
    assert_match "Soundrobe", shell_output("#{bin}/auto-tag --help")
    
    # More comprehensive test
    test_file = testpath/"test.txt"
    test_file.write "Sample content"
    output = shell_output("#{bin}/auto-tag tag #{test_file}")
    assert_match "test.txt", output
  end
end
```

#### Automated Resource Generation

```bash
# Use brew update-python-resources (recommended)
brew update-python-resources soundrobe

# Alternative: Use homebrew-pypi-poet
cd "$(mktemp -d)"
python3 -m venv venv
source venv/bin/activate
pip install soundrobe homebrew-pypi-poet
poet soundrobe
# Copy output to formula
```

### Step 4: Submit to Homebrew Core

#### Requirements
1. Package must be on PyPI
2. Must have significant user interest/usefulness
3. Must be maintained actively
4. Follow Homebrew guidelines

#### Submission Process

```bash
# Fork homebrew-core on GitHub
git clone https://github.com/YOUR_USERNAME/homebrew-core.git
cd homebrew-core

# Create branch
git checkout -b soundrobe

# Add formula
# Copy formula to Formula/soundrobe.rb

# Test locally
brew install --build-from-source Formula/soundrobe.rb
brew test soundrobe
brew audit --new-formula soundrobe

# Commit and push
git add Formula/soundrobe.rb
git commit -m "soundrobe 1.0.0 (new formula)"
git push origin soundrobe

# Create PR on GitHub
# Title: "soundrobe 1.0.0 (new formula)"
# Include: description, usage examples, test results
```

### Step 5: Version Updates

```ruby
# Update formula for new version
class Soundrobe < Formula
  url "https://files.pythonhosted.org/packages/source/a/soundrobe/soundrobe-2.0.0.tar.gz"
  sha256 "NEW_SHA256..."
  # Update resource versions if needed
  
  # Use livecheck for automatic version detection
  livecheck do
    url :homepage
    regex(/href=.*?soundrobe[._-]v?(\d+(?:\.\d+)+)\.t/i)
  end
end
```

---

## Code Examples from Similar Tools

### Example 1: jrnl (Journal CLI)

**Structure**:
- Entry point: `jrnl/cli.py`
- Commands: Individual modules
- Config: YAML + environment
- Output: Rich formatting

**Key Patterns**:
```python
# jrnl's CLI structure
@click.group()
@click.option('--config', '-c', type=click.Path())
@click.pass_context
def jrnl(ctx, config):
    ctx.obj = load_config(config)

@jrnl.command()
@click.argument('content', required=False)
@click.option('--edit', is_flag=True)
@click.pass_obj
def write(config, content, edit):
    """Write a new entry."""
    ...
```

**Homebrew Formula** (from research):
- Uses `virtualenv_install_with_resources`
- Declares all dependencies explicitly
- Includes encryption test with PTY for TTY

### Example 2: HTTPie (HTTP CLI)

**Structure**:
```
httpie/
├── httpie/
│   ├── cli.py
│   ├── commands.py
│   ├── output/
│   │   ├── processing.py
│   │   ├── formatters.py
│   └── compat.py
```

**Key Patterns**:
- Extensive output formatting options
- Plugin system via entry points
- Comprehensive shell completion

### Example 3: pip (Package Manager)

**Key Patterns**:
- Subcommand organization: `pip install`, `pip list`, etc.
- Configuration: Multiple sources (env, config file, CLI)
- Progress bars for downloads
- Color output with NO_COLOR support

### Example 4: AWS CLI

**Key Patterns**:
```python
# AWS CLI's hierarchical structure
@click.group()
def aws():
    pass

@aws.group()
def s3():
    pass

@s3.command()
@click.argument('source')
@click.argument('destination')
def cp(source, destination):
    """Copy S3 objects."""
    ...
```

---

## Additional Best Practices

### CLI UX Guidelines (from clig.dev)

1. **Human-first design**: Output for humans by default, machines via flags
2. **Help first**: When no args, show concise help + examples
3. **Suggestions on error**: "Did you mean...?"
4. **Progress for long operations**: Always show progress
5. **Color with intention**: Use NO_COLOR env var
6. **Exit codes matter**: 0 for success, non-zero for failure
7. **Stdout/stderr separation**: Output to stdout, logs to stderr

### Testing CLI Applications

```python
# tests/test_cli.py
from click.testing import CliRunner
from soundrobe.cli import cli

def test_tag_command():
    runner = CliRunner()
    with runner.isolated_filesystem():
        # Create test file
        with open('test.txt', 'w') as f:
            f.write('content')
        
        # Run command
        result = runner.invoke(cli, ['tag', 'test.txt'])
        
        assert result.exit_code == 0
        assert 'test.txt' in result.output

def test_config_missing():
    runner = CliRunner()
    result = runner.invoke(cli, ['tag', 'nonexistent.txt'])
    
    assert result.exit_code != 0
    assert 'Error' in result.output

def test_verbose_output():
    runner = CliRunner(mix_stderr=False)
    result = runner.invoke(cli, ['--verbose', 'tag', 'test.txt'])
    
    assert 'DEBUG' in result.stderr
```

### Dependency Management

#### Minimal Dependencies
```toml
# Keep core dependencies minimal
dependencies = [
    "click>=8.1.0",     # CLI framework
    "rich>=13.0.0",     # Output formatting
    "pyyaml>=6.0",      # Config parsing
]

# Optional functionality as extras
[project.optional-dependencies]
ai = ["anthropic>=0.18.0", "openai>=1.0.0"]
full = ["soundrobe[ai,dev]"]
```

### Version Management

```python
# __init__.py
__version__ = "1.0.0"

# CLI version option
@cli.command()
@click.option('--version', is_flag=True, callback=get_version, 
              expose_value=False, is_eager=True)
def version():
    """Show version."""
    pass

def get_version(ctx, param, value):
    if value:
        from . import __version__
        click.echo(f"soundrobe {__version__}")
        ctx.exit()
```

---

## Summary Recommendations

### For soundrobe CLI:

1. **Framework**: Use Click with Rich for output
2. **Config**: Pydantic Settings with YAML file + env vars
3. **Structure**: src layout with commands/ and core/ separation
4. **Progress**: Rich Progress bars for batch operations
5. **Errors**: Custom exceptions with exit codes, graceful degradation
6. **Testing**: pytest with CliRunner for CLI tests
7. **Distribution**: PyPI first, then Homebrew formula with virtualenv

### Next Steps:
1. Implement basic CLI structure
2. Add configuration management
3. Implement core tagging logic
4. Add Rich output formatting
5. Write tests
6. Set up PyPI publishing
7. Create Homebrew formula