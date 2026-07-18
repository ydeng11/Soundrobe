# Soundrobe — development commands
# Requires: just (https://github.com/casey/just)

set dotenv-load := true
set dotenv-path := ".env.local"

project := "soundrobe"

# Show available commands (default — runs first when you type `just`)
default:
    @echo "── Soundrobe dev commands ──"
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
    @echo "  just fe-smoke-openrouter run credentialed OpenRouter release gate"
    @echo "  just fe-smoke-assistant run live native assistant loopback"
    @echo "  just fe-smoke-cover-picker run macOS native picker cancel gate"
    @echo ""
    @echo "Ship:"
    @echo "  just fe-dist <target>   build distributable (mac|win|linux)"
    @echo "  just fe-dist-mac-intel  cross-build deterministic Intel macOS bundles"

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

# Exercise the production OpenRouter client with credentials from .env.local.
# The ignored test never prints the API key or response content.
fe-smoke-openrouter:
    cd frontend/src-tauri && cargo test --all-features live_openrouter_returns_schema_constrained_json -- --ignored --nocapture

# Exercise the packaged renderer adapter, Tauri command, assistant runtime,
# OpenRouter transport, response schema, and conversation persistence together.
fe-smoke-assistant: _fe-deps-check
    cd frontend && npm run build:e2e && npx wdio run wdio.conf.ts --spec e2e-tauri/live-openrouter.spec.ts

# Open the real macOS image picker and cancel it through System Events. This is
# intentionally a local display smoke, separate from cross-platform CI E2E.
fe-smoke-cover-picker: _fe-deps-check
    cd frontend && npm run build:e2e && npx wdio run wdio.conf.ts --spec e2e-tauri/live-cover-picker.spec.ts

# Build platform distributable (requires: fe-build)
# Targets: mac, win, linux — e.g. just fe-dist mac
fe-dist target="":
    cd frontend && npm run dist:{{ target }}

# Cross-build Intel macOS bundles from Apple Silicon. Requires:
# rustup target add x86_64-apple-darwin
fe-dist-mac-intel: _fe-deps-check
    cd frontend && CI=true npm run dist:mac -- --target x86_64-apple-darwin
