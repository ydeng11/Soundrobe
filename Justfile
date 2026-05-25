# Auto Tagger — development commands
# Requires: just (https://github.com/casey/just)

project := "auto-tagger"
python  := ".venv/bin/python"
pip     := ".venv/bin/pip"
pytest  := ".venv/bin/pytest" + " --tb=short"

# ============================================================================
# Setup
# ============================================================================

# Create and activate virtual environment, install dev dependencies
venv:
    python3 -m venv .venv
    {{ pip }} install -e ".[dev]"

# Install the package in editable mode (re-run after dependency changes)
install:
    {{ pip }} install -e ".[dev]"

# ============================================================================
# Quality — lint, type-check, test
# ============================================================================

# Run ruff linter
lint:
    ruff check src tests

# Auto-fix lint issues
lint-fix:
    ruff check --fix src tests

# Run mypy type checker
typecheck:
    mypy src

# Run pytest with coverage
test:
    {{ pytest }} --cov={{ project }}

# Run a specific test file (usage: just test-file tests/test_cli.py)
test-file path:
    {{ pytest }} "{{ path }}"

# Run a specific test by name (usage: just test-match test_tag)
test-match pattern:
    {{ pytest }} -k "{{ pattern }}"

# Run the full quality pipeline: lint, typecheck, test
check-all: lint typecheck test

# ============================================================================
# Build & distribution
# ============================================================================

# Build source distribution and wheel
build:
    {{ python }} -m build

# Show installed package info
show:
    {{ pip }} show {{ project }}

# ============================================================================
# Development CLI
# ============================================================================

# Run the CLI (passes remaining args directly)
# Examples:
#   just run --help
#   just run version
#   just run tag path --interactive
#   just run batch path --parallel 4 --health-report report.json
#   just run dataset setup
#   just run config
run *args:
    {{ python }} -m auto_tagger {{ args }}

# Tag a single album with optional flags
# Examples:
#   just tag                          # tag current dir (dry-run)
#   just tag /path/to/album           # tag specific album (dry-run)
#   just tag /path --yolo             # auto-approve and apply
#   just tag /path --interactive      # prompt before applying
#   just tag /path --health-report r.json
tag path="." *flags:
    {{ python }} -m auto_tagger tag "{{ path }}" {{ flags }}

# Batch process a music library with optional flags
# Examples:
#   just batch /library               # dry-run preview of entire library
#   just batch /library --yolo        # apply all changes automatically
#   just batch /library --parallel 4  # process 4 albums in parallel
#   just batch /library --interactive # prompt per album
#   just batch /library --health-report report.json
batch path="." *flags:
    {{ python }} -m auto_tagger batch "{{ path }}" {{ flags }}

# ============================================================================
# Dataset
# ============================================================================

# Check dataset status
dataset-status:
    {{ python }} -m auto_tagger dataset status

# Preview dataset setup plan
dataset-plan:
    {{ python }} -m auto_tagger dataset setup --dry-run

# Download and build the local dataset index
dataset-setup:
    {{ python }} -m auto_tagger dataset setup

# ============================================================================
# Frontend (Electron v2)
# ============================================================================

# Install frontend dependencies
fe-install:
    cd frontend && npm install

# Run frontend dev server (Vite HMR + Electron)
fe-dev:
    cd frontend && npm run dev

# Build frontend for production
fe-build:
    cd frontend && npm run build

# Run frontend tests
fe-test:
    cd frontend && npm test

# Run frontend type check
fe-typecheck:
    cd frontend && npm run typecheck

# ============================================================================
# Cleanup
# ============================================================================

# Remove build artifacts and __pycache__ dirs
clean:
    rm -rf build/ dist/ *.egg-info .coverage coverage.json
    find . -type d -name __pycache__ -exec rm -rf {} + 2>/dev/null || true
    find . -type d -name .pytest_cache -exec rm -rf {} + 2>/dev/null || true
    find . -type d -name .ruff_cache -exec rm -rf {} + 2>/dev/null || true
    find . -type d -name .mypy_cache -exec rm -rf {} + 2>/dev/null || true

# Remove everything including the virtual env
nuke: clean
    rm -rf .venv

# ============================================================================
# Help
# ============================================================================

# Show all available commands
default:
    @just --list
