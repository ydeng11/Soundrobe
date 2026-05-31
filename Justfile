# Auto Tagger — development commands
# Requires: just (https://github.com/casey/just)

set dotenv-load := true

project := "auto-tagger"
python  := ".venv/bin/python"
pip     := ".venv/bin/pip"

# Show available commands (default — runs first when you type `just`)
default:
    @echo "── Auto Tagger dev commands ──"
    @echo ""
    @echo "Start the app:"
    @echo "  just fe-dev             start dev server (Vite HMR + Electron)"
    @echo ""
    @echo "Develop:"
    @echo "  just fe-install         install frontend dependencies"
    @echo "  just fe-build           build for production"
    @echo "  just fe-test            run all tests"
    @echo "  just fe-typecheck       run TypeScript type checker"
    @echo "  just fe-check           typecheck + test"
    @echo ""
    @echo "Ship:"
    @echo "  just fe-dist <target>   build distributable (mac|win|linux)"
    @echo "  just fe-rebuild-native  rebuild native modules for Electron ABI"
    @echo ""
    @echo "Dataset (one-time, requires Python venv):"
    @echo "  just dataset-status     check dataset index status"
    @echo "  just dataset-setup      download and build dataset index"
    @echo "  just dataset-plan       preview dataset setup"

# ============================================================================
# Frontend (Electron v2) — primary dev workflow
# ============================================================================

# Install frontend dependencies
fe-install:
    cd frontend && npm install

# Start dev server (Vite HMR + Electron) — hot-reloads on save
# .env vars (LLM_API_KEY, LLM_MODEL) loaded automatically via set dotenv-load
fe-dev:
    cd frontend && npm run dev

# Build frontend for production (tsc + Vite)
fe-build:
    cd frontend && npm run build

# Run all frontend tests
fe-test:
    cd frontend && npm test

# Run frontend type checker only
fe-typecheck:
    cd frontend && npm run typecheck

# Typecheck + test — full quality gate
fe-check: fe-typecheck fe-test
    echo "✓ All frontend checks passed"

# Build platform distributable (requires: fe-build)
# Targets: mac, win, linux — e.g. just fe-dist mac
fe-dist target="":
    cd frontend && npm run dist:{{ target }}

# Rebuild native modules for Electron's ABI
# Run once before first fe-dist after npm install
fe-rebuild-native:
    cd frontend && npm run rebuild-native

# ============================================================================
# Dataset (Python CLI) — one-time setup, shared with Electron v2
# ============================================================================

# Check local dataset index status
dataset-status:
    {{ python }} -m auto_tagger dataset status

# Preview dataset setup plan without downloading
dataset-plan:
    {{ python }} -m auto_tagger dataset setup --dry-run

# Download and build the local SQLite dataset index
dataset-setup:
    {{ python }} -m auto_tagger dataset setup
