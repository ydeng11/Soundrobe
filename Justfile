# Auto Tagger — development commands
# Requires: just (https://github.com/casey/just)

set dotenv-load := true
set dotenv-path := ".env.local"

project := "auto-tagger"
python  := ".venv/bin/python"
pip     := ".venv/bin/pip"

# Show available commands (default — runs first when you type `just`)
default:
    @echo "── Auto Tagger dev commands ──"
    @echo ""
    @echo "Start the app:"
    @echo "  just fe-dev             start Tauri app with Vite HMR"
    @echo ""
    @echo "Develop:"
    @echo "  just fe-install         install frontend dependencies"
    @echo "  just fe-build           build for production"
    @echo "  just fe-test            run all unit tests"
    @echo "  just fe-typecheck       run TypeScript type checker"
    @echo "  just fe-check           typecheck + test"
    @echo ""
    @echo "Ship:"
    @echo "  just fe-dist <target>   build distributable (mac|win|linux)"
    @echo ""
    @echo "Dataset (one-time, requires Python venv):"
    @echo "  just dataset-status     check dataset index status"
    @echo "  just dataset-setup      download and build dataset index"
    @echo "  just dataset-plan       preview dataset setup"

# ============================================================================
# Frontend (Tauri v2) — primary dev workflow
# ============================================================================

# Check that frontend deps are installed; auto-install if missing
_fe-deps-check:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -x frontend/node_modules/.bin/vite ]; then
        echo "→ Frontend deps not found, installing..."
        pushd frontend >/dev/null
        npm install
        popd >/dev/null
        echo "✓ Frontend deps installed"
    fi

# Install frontend and Tauri CLI dependencies
fe-install:
    cd frontend
    npm install

# Start Tauri with Vite HMR — hot-reloads on save
# .env vars (LLM_API_KEY, LLM_MODEL) loaded automatically via set dotenv-load
fe-dev: _fe-deps-check
    cd frontend && npm run dev

# Build the Tauri application and platform bundle
fe-build: _fe-deps-check
    cd frontend && npm run build

# Run all frontend tests
fe-test: _fe-deps-check
    cd frontend && npm test

# Run frontend type checker only
fe-typecheck: _fe-deps-check
    cd frontend && npm run typecheck

# Typecheck + test — full quality gate
fe-check: fe-typecheck fe-test
    echo "✓ All frontend checks passed"

# Build platform distributable (requires: fe-build)
# Targets: mac, win, linux — e.g. just fe-dist mac
fe-dist target="":
    cd frontend && npm run dist:{{ target }}

# ============================================================================
# Dataset (Python CLI) — one-time setup, shared with Tauri v2
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
