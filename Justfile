# Soundrobe — development commands
# Requires: just (https://github.com/casey/just)

set dotenv-load := true
set dotenv-path := ".env.local"

project := "soundrobe"

default:
    @echo "── Soundrobe dev commands ──"
    @echo ""
    @echo "Start the app:"
    @echo "  just dev                  start Tauri app with Vite HMR"
    @echo ""
    @echo "Develop:"
    @echo "  just install              install app dependencies"
    @echo "  just build                build for production"
    @echo "  just test                 run all unit tests"
    @echo "  just typecheck            run TypeScript type checker"
    @echo "  just check                typecheck + test"
    @echo "  just smoke-openrouter     run credentialed OpenRouter release gate"
    @echo "  just smoke-assistant      run live native assistant loopback"
    @echo "  just smoke-cover-picker   run macOS native picker cancel gate"
    @echo ""
    @echo "Ship:"
    @echo "  just dist <target>        build distributable (mac|win|linux)"
    @echo "  just dist-mac-intel       cross-build deterministic Intel macOS bundles"
    @echo ""
    @echo "Legacy fe-* aliases remain temporarily and print deprecation notices."

_deps-check:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -x node_modules/.bin/vite ]; then
        echo "→ App dependencies not found, installing..."
        npm install
        echo "✓ App dependencies installed"
    fi

install:
    npm install

dev: _deps-check
    #!/usr/bin/env bash
    set -euo pipefail
    : "${LLM_API_KEY:?LLM_API_KEY missing — add it to .env.local}"
    : "${LLM_MODEL:?LLM_MODEL missing — add it to .env.local}"
    exec env \
        SOUNDROBE_LOG="${SOUNDROBE_LOG:-trace}" \
        AUTO_TAG_CHINESE_SCRIPT="${AUTO_TAG_CHINESE_SCRIPT:-simplified}" \
        npm run dev

build: _deps-check
    npm run build

test: _deps-check
    npm test

typecheck: _deps-check
    npm run typecheck

check: typecheck test
    echo "✓ All checks passed"

smoke-openrouter:
    cd src-tauri && cargo test --all-features live_openrouter_returns_schema_constrained_json -- --ignored --nocapture

smoke-assistant-ai:
    cd src-tauri && cargo test --all-features -- --ignored --nocapture same_intent_read_only_produces_safe_outcome mutating_request_always_has_action_batch_or_tool_call live_missing_genre_value_reaches_conditional_patch_after_inspection live_navidrome_artists_intent_preserves_display_credit_and_collaborators live_group_by_base_title_reaches_schema_valid_mutation

smoke-group-albums:
    cd src-tauri && cargo test --all-features -- --ignored --nocapture live_group_by_base_title_reaches_schema_valid_mutation

smoke-assistant: _deps-check
    npm run build:e2e && npx wdio run wdio.conf.ts --spec e2e-tauri/live-openrouter.spec.ts

smoke-cover-picker: _deps-check
    npm run build:e2e && npx wdio run wdio.conf.ts --spec e2e-tauri/live-cover-picker.spec.ts

model model_name="":
    #!/usr/bin/env bash
    set -euo pipefail
    ENV_FILE=".env.local"
    if [ -n "{{ model_name }}" ]; then
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
            echo "Usage:  just model <model-name>"
            echo "        just model openai/gpt-4o"
            echo ""
            echo "See: https://openrouter.ai/models for available models"
        else
            echo "No .env.local found. Create one with:"
            echo '  LLM_API_KEY=sk-or-v1-...'
            echo '  LLM_MODEL=openai/gpt-4o'
            echo ""
            echo "Then run:  just model <model-name>"
        fi
    fi

dist target="":
    npm run dist:{{ target }}

dist-mac-intel: _deps-check
    CI=true npm run dist:mac -- --target x86_64-apple-darwin

fe-install:
    @echo "DEPRECATED: use 'just install'"
    just install

fe-dev:
    @echo "DEPRECATED: use 'just dev'"
    just dev

fe-build:
    @echo "DEPRECATED: use 'just build'"
    just build

fe-test:
    @echo "DEPRECATED: use 'just test'"
    just test

fe-typecheck:
    @echo "DEPRECATED: use 'just typecheck'"
    just typecheck

fe-check:
    @echo "DEPRECATED: use 'just check'"
    just check

fe-smoke-openrouter:
    @echo "DEPRECATED: use 'just smoke-openrouter'"
    just smoke-openrouter

fe-smoke-assistant-ai:
    @echo "DEPRECATED: use 'just smoke-assistant-ai'"
    just smoke-assistant-ai

fe-smoke-group-albums:
    @echo "DEPRECATED: use 'just smoke-group-albums'"
    just smoke-group-albums

fe-smoke-assistant:
    @echo "DEPRECATED: use 'just smoke-assistant'"
    just smoke-assistant

fe-smoke-cover-picker:
    @echo "DEPRECATED: use 'just smoke-cover-picker'"
    just smoke-cover-picker

fe-model model_name="":
    @echo "DEPRECATED: use 'just model'"
    just model "{{ model_name }}"

fe-dist target="":
    @echo "DEPRECATED: use 'just dist'"
    just dist "{{ target }}"

fe-dist-mac-intel:
    @echo "DEPRECATED: use 'just dist-mac-intel'"
    just dist-mac-intel
