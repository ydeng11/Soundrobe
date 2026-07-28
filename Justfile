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
# .env vars (LLM_API_KEY, LLM_MODEL, AUTO_TAG_CHINESE_SCRIPT) loaded automatically
# via set dotenv-load. Fails immediately if critical vars are missing.
fe-dev: _fe-deps-check
    #!/usr/bin/env bash
    set -euo pipefail
    : "${LLM_API_KEY:?LLM_API_KEY missing — add it to .env.local}"
    : "${LLM_MODEL:?LLM_MODEL missing — add it to .env.local}"
    cd frontend
    exec env \
        SOUNDROBE_LOG="${SOUNDROBE_LOG:-trace}" \
        AUTO_TAG_CHINESE_SCRIPT="${AUTO_TAG_CHINESE_SCRIPT:-simplified}" \
        npm run dev

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

# Run the AI assistant integration tests (require LLM_API_KEY from .env.local).
# Tests: same_intent_read_only_produces_message,
#        mutating_request_always_has_action_batch_or_tool_call, ...
# For judge-based tests set LLM_JUDGE_MODEL too.
fe-smoke-assistant-ai:
    cd frontend/src-tauri && cargo test --all-features -- --ignored --nocapture same_intent_read_only_produces_safe_outcome mutating_request_always_has_action_batch_or_tool_call

# Exercise the packaged renderer adapter, Tauri command, assistant runtime,
# OpenRouter transport, response schema, and conversation persistence together.
fe-smoke-assistant: _fe-deps-check
    cd frontend && npm run build:e2e && npx wdio run wdio.conf.ts --spec e2e-tauri/live-openrouter.spec.ts

# Open the real macOS image picker and cancel it through System Events. This is
# intentionally a local display smoke, separate from cross-platform CI E2E.
fe-smoke-cover-picker: _fe-deps-check
    cd frontend && npm run build:e2e && npx wdio run wdio.conf.ts --spec e2e-tauri/live-cover-picker.spec.ts

# View or set the LLM model used by the AI Assistant.
# With no argument, shows the current model and key status.
# With a model name, updates .env.local to use that model.
# Examples:
#   just fe-model                     # show current model/key status
#   just fe-model openai/gpt-4o      # switch to GPT-4o
#   just fe-model anthropic/claude-3  # switch to Claude 3
fe-model model_name="":
    #!/usr/bin/env bash
    set -euo pipefail
    ENV_FILE=".env.local"
    if [ -n "{{ model_name }}" ]; then
        # Update — set or replace LLM_MODEL in .env.local
        if [ -f "$ENV_FILE" ] && grep -q '^LLM_MODEL=' "$ENV_FILE" 2>/dev/null; then
            if [[ "$(uname)" == "Darwin" ]]; then
                sed -i '' -E "s|^LLM_MODEL=.*|LLM_MODEL={{ model_name }}|" "$ENV_FILE"
            else
                sed -i -E "s|^LLM_MODEL=.*|LLM_MODEL={{ model_name }}|" "$ENV_FILE"
            fi
            echo "✓ LLM_MODEL updated to {{ model_name }} in $ENV_FILE"
        else
            echo "LLM_MODEL={{ model_name }}" >> "$ENV_FILE"
            echo "✓ LLM_MODEL={{ model_name }} appended to $ENV_FILE"
        fi
    else
        # Show current state
        if [ -f "$ENV_FILE" ]; then
            current=$(grep '^LLM_MODEL=' "$ENV_FILE" | cut -d= -f2- || true)
            api_key=$(grep '^LLM_API_KEY=' "$ENV_FILE" | cut -d= -f2- | cut -c1-12 || true)
            echo "Model:  ${current:-not set}"
            if [ -n "$api_key" ]; then
                echo "API key: ${api_key}... (present)"
            else
                echo "API key: not set"
            fi
            echo ""
            echo "Usage:  just fe-model <model-name>"
            echo "        just fe-model openai/gpt-4o"
            echo ""
            echo "See: https://openrouter.ai/models for available models"
        else
            echo "No .env.local found. Create one with:"
            echo '  LLM_API_KEY=sk-or-v1-...'
            echo '  LLM_MODEL=openai/gpt-4o'
            echo ""
            echo "Then run:  just fe-model <model-name>"
        fi
    fi

# Build platform distributable (requires: fe-build)
# Targets: mac, win, linux — e.g. just fe-dist mac
fe-dist target="":
    cd frontend && npm run dist:{{ target }}

# Cross-build Intel macOS bundles from Apple Silicon. Requires:
# rustup target add x86_64-apple-darwin
fe-dist-mac-intel: _fe-deps-check
    cd frontend && CI=true npm run dist:mac -- --target x86_64-apple-darwin
