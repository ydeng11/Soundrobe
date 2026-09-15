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
    @echo "  just eval-auto-tag        run deterministic metadata-only auto-tag evaluation"
    @echo "  just eval-auto-tag-ground-truth audit the complete reviewed expectation ledger"
    @echo "  just eval-auto-tag-repro  audit frozen pools, equivalence, and cold/warm identity"
    @echo "  just eval-auto-tag-profiles  report deterministic metrics for all three input profiles"
    @echo "  just eval-auto-tag-subset  review the frozen representative corpus subset"
    @echo "  just collect-auto-tag-evidence  plan or run one bounded resumable evidence batch"
    @echo "  just eval-auto-tag-score  score reviewed native evaluation results offline"
    @echo "  just eval-auto-tag-live   run the explicit credentialed native evaluation"
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

# Explicitly selected real-data gate; the Rust test copies media before reading tags.
smoke-auto-tag-latency source:
    SOUNDROBE_LATENCY_SOURCE={{quote(source)}} cargo test --manifest-path src-tauri/Cargo.toml --all-features live_auto_tag_deepseek_latency -- --ignored --nocapture

smoke-assistant-ai:
    cd src-tauri && cargo test --all-features -- --ignored --nocapture same_intent_read_only_produces_safe_outcome mutating_request_always_has_action_batch_or_tool_call live_missing_genre_value_reaches_conditional_patch_after_inspection live_navidrome_artists_intent_preserves_display_credit_and_collaborators live_group_by_base_title_reaches_schema_valid_mutation

smoke-group-albums:
    cd src-tauri && cargo test --all-features -- --ignored --nocapture live_group_by_base_title_reaches_schema_valid_mutation

smoke-assistant: _deps-check
    npm run build:e2e && npx wdio run wdio.conf.ts --spec e2e-tauri/live-openrouter.spec.ts

smoke-cover-picker: _deps-check
    npm run build:e2e && npx wdio run wdio.conf.ts --spec e2e-tauri/live-cover-picker.spec.ts

# Deterministic corpus/profile/selection contracts; no credentials or media writes.
eval-auto-tag:
    cd src-tauri && cargo test --lib commands::auto_tag::eval_tests
    cd src-tauri && cargo test --lib commands::track_matcher::guarded_tests

# Offline reconciliation of every corpus case against its reviewed expectation ledger.
eval-auto-tag-ground-truth:
    #!/usr/bin/env bash
    set -euo pipefail
    run_id="${SOUNDROBE_AUTO_TAG_GROUND_TRUTH_RUN_ID:-ground-truth-$(date -u +%Y%m%dT%H%M%SZ)}"
    output_dir="${SOUNDROBE_AUTO_TAG_GROUND_TRUTH_DIR:-$PWD/.planning/debug/auto-tag-eval/$run_id}"
    if [[ "$output_dir" != /* ]]; then
        output_dir="$PWD/$output_dir"
    fi
    exec python3 scripts/audit_auto_tag_ground_truth.py \
        --corpus "${SOUNDROBE_AUTO_TAG_EVAL_CORPUS:-$PWD/test/fixtures/tauri/auto-tag-eval/corpus.json}" \
        --expectations "${SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS:-$PWD/test/fixtures/tauri/auto-tag-eval/expectations.json}" \
        --output-dir "$output_dir" \
        --expected-case-count 321 \
        --expected-track-count 2841 \
        --run-id "$run_id"

# Offline reproducibility gate for frozen provider snapshots and retained native replay.
eval-auto-tag-repro:
    #!/usr/bin/env bash
    set -euo pipefail
    run_id="${SOUNDROBE_AUTO_TAG_REPRO_RUN_ID:-reproducibility-$(date -u +%Y%m%dT%H%M%SZ)}"
    output_dir="${SOUNDROBE_AUTO_TAG_REPRO_DIR:-$PWD/.planning/debug/auto-tag-eval/$run_id}"
    if [[ "$output_dir" != /* ]]; then
        output_dir="$PWD/$output_dir"
    fi
    native_results="${SOUNDROBE_AUTO_TAG_REPRO_NATIVE_RESULTS:-$PWD/.planning/debug/enya-2026-09-13-followup/synthetic-corpus/results.json}"
    exec python3 scripts/audit_auto_tag_reproducibility.py \
        --candidate-pools "${SOUNDROBE_AUTO_TAG_EVAL_CANDIDATE_POOLS:-$PWD/test/fixtures/tauri/auto-tag-eval/candidate-pools.json}" \
        --equivalence "${SOUNDROBE_AUTO_TAG_REPRO_EQUIVALENCE:-$PWD/.planning/debug/enya-2026-09-13-followup/synthetic-corpus/equivalence.json}" \
        --native-results "$native_results" \
        --output-dir "$output_dir" \
        --run-id "$run_id"

# Offline denominator and profile-shape report; native records remain separate.
eval-auto-tag-profiles:
    #!/usr/bin/env bash
    set -euo pipefail
    run_id="${SOUNDROBE_AUTO_TAG_PROFILE_RUN_ID:-profile-baseline-$(date -u +%Y%m%dT%H%M%SZ)}"
    output_dir="${SOUNDROBE_AUTO_TAG_PROFILE_DIR:-$PWD/.planning/debug/auto-tag-eval/$run_id}"
    if [[ "$output_dir" != /* ]]; then
        output_dir="$PWD/$output_dir"
    fi
    exec python3 scripts/audit_auto_tag_profiles.py \
        --corpus "${SOUNDROBE_AUTO_TAG_EVAL_CORPUS:-$PWD/test/fixtures/tauri/auto-tag-eval/corpus.json}" \
        --expectations "${SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS:-$PWD/test/fixtures/tauri/auto-tag-eval/expectations.json}" \
        --output-dir "$output_dir" \
        --run-id "$run_id" \
        --native "folder_filename=${SOUNDROBE_AUTO_TAG_PROFILE_DISCOVERY_RESULTS:-$PWD/.planning/debug/enya-2026-09-13-followup/synthetic-corpus/results.json}" \
        --native "assisted_without_ids=${SOUNDROBE_AUTO_TAG_PROFILE_ASSISTED_RESULTS:-$PWD/.planning/debug/auto-tag-eval/baseline-assisted-2026-09-14/cold.jsonl}" \
        --native "tagged_recovery=${SOUNDROBE_AUTO_TAG_PROFILE_RECOVERY_RESULTS:-$PWD/.planning/debug/auto-tag-eval/baseline-recovery-2026-09-14/results.json}"

# Offline provider-backed review of the representative subset; unresolved rows remain unscored.
eval-auto-tag-subset:
    #!/usr/bin/env bash
    set -euo pipefail
    run_id="${SOUNDROBE_AUTO_TAG_SUBSET_RUN_ID:-reviewed-subset-$(date -u +%Y%m%dT%H%M%SZ)}"
    output_dir="${SOUNDROBE_AUTO_TAG_SUBSET_DIR:-$PWD/.planning/debug/auto-tag-eval/$run_id}"
    if [[ "$output_dir" != /* ]]; then
        output_dir="$PWD/$output_dir"
    fi
    exec python3 scripts/review_auto_tag_subset.py \
        --corpus "${SOUNDROBE_AUTO_TAG_EVAL_CORPUS:-$PWD/test/fixtures/tauri/auto-tag-eval/corpus.json}" \
        --manifest "${SOUNDROBE_AUTO_TAG_SUBSET_MANIFEST:-$PWD/test/fixtures/tauri/auto-tag-eval/reviewed-subset.json}" \
        --fixture-root "${SOUNDROBE_AUTO_TAG_SUBSET_FIXTURE_ROOT:-$PWD/test/fixtures/tauri/auto-tag-eval}" \
        --output-dir "$output_dir"

# Bounded, resumable live evidence collection; planning is offline by default.
collect-auto-tag-evidence:
    #!/usr/bin/env bash
    set -euo pipefail
    output_dir="${SOUNDROBE_AUTO_TAG_EVIDENCE_DIR:-$PWD/.planning/debug/auto-tag-evidence}"
    if [[ "$output_dir" != /* ]]; then
        output_dir="$PWD/$output_dir"
    fi
    common_args=(
        --corpus "${SOUNDROBE_AUTO_TAG_EVAL_CORPUS:-$PWD/test/fixtures/tauri/auto-tag-eval/corpus.json}"
        --expectations "${SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS:-$PWD/test/fixtures/tauri/auto-tag-eval/expectations.json}"
        --reviewed-manifest "${SOUNDROBE_AUTO_TAG_EVIDENCE_REVIEWED_MANIFEST:-$PWD/test/fixtures/tauri/auto-tag-eval/reviewed-subset.json}"
        --output-dir "$output_dir"
        --profile "${SOUNDROBE_AUTO_TAG_EVIDENCE_PROFILE:-folder_filename}"
        --batch-size "${SOUNDROBE_AUTO_TAG_EVIDENCE_BATCH_SIZE:-4}"
        --max-batches "${SOUNDROBE_AUTO_TAG_EVIDENCE_MAX_BATCHES:-1}"
    )
    if [[ "${SOUNDROBE_AUTO_TAG_EVIDENCE_RUN:-0}" == "1" ]]; then
        exec python3 scripts/collect_auto_tag_evidence.py "${common_args[@]}" --run
    fi
    exec python3 scripts/collect_auto_tag_evidence.py "${common_args[@]}"

# Offline scoring for retained native results and reviewed expectation ledgers.
eval-auto-tag-score:
    #!/usr/bin/env bash
    set -euo pipefail
    run_id="${SOUNDROBE_AUTO_TAG_SCORE_RUN_ID:-scored-$(date -u +%Y%m%dT%H%M%SZ)}"
    output_dir="${SOUNDROBE_AUTO_TAG_SCORE_DIR:-$PWD/.planning/debug/auto-tag-eval/$run_id}"
    if [[ "$output_dir" != /* ]]; then
        output_dir="$PWD/$output_dir"
    fi
    exec python3 scripts/score_auto_tag_eval.py \
        --corpus "${SOUNDROBE_AUTO_TAG_EVAL_CORPUS:-$PWD/test/fixtures/tauri/auto-tag-eval/corpus.json}" \
        --expectations "${SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS:-$PWD/test/fixtures/tauri/auto-tag-eval/expectations.json}" \
        --results "${SOUNDROBE_AUTO_TAG_EVAL_RESULTS:-$PWD/.planning/debug/enya-2026-09-13-followup/synthetic-corpus/results.json}" \
        --reviewed-truth "${SOUNDROBE_AUTO_TAG_EVAL_REVIEWED_TRUTH:-$PWD/test/fixtures/tauri/relapse-deluxe/reviewed-truth.json}" \
        --output-dir "$output_dir" \
        --run-id "$run_id"

# Explicit live gate. Filters are comma-separated artist/case IDs and default to clean discovery.
# Native evaluation uses disposable synthetic FLAC inputs generated from the corpus metadata.
eval-auto-tag-live:
    #!/usr/bin/env bash
    set -euo pipefail
    run_id="${SOUNDROBE_AUTO_TAG_EVAL_RUN_ID:-native-$(date -u +%Y%m%dT%H%M%SZ)}"
    artifact_dir="${SOUNDROBE_AUTO_TAG_EVAL_ARTIFACT_DIR:-$PWD/.planning/debug/auto-tag-eval/$run_id}"
    if [[ "$artifact_dir" != /* ]]; then
        artifact_dir="$PWD/$artifact_dir"
    fi
    mkdir -p "$artifact_dir"
    env \
        SOUNDROBE_AUTO_TAG_EVAL_CORPUS="${SOUNDROBE_AUTO_TAG_EVAL_CORPUS:-$PWD/test/fixtures/tauri/auto-tag-eval/corpus.json}" \
        SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS="${SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS:-$PWD/test/fixtures/tauri/auto-tag-eval/expectations.json}" \
        SOUNDROBE_AUTO_TAG_EVAL_ARTIFACT_DIR="$artifact_dir" \
        SOUNDROBE_AUTO_TAG_EVAL_RUN_ID="$run_id" \
        SOUNDROBE_AUTO_TAG_EVAL_PROFILE="${SOUNDROBE_AUTO_TAG_EVAL_PROFILE:-folder_filename}" \
        SOUNDROBE_AUTO_TAG_EVAL_ARTISTS="${SOUNDROBE_AUTO_TAG_EVAL_ARTISTS:-}" \
        SOUNDROBE_AUTO_TAG_EVAL_CASES="${SOUNDROBE_AUTO_TAG_EVAL_CASES:-}" \
        cargo test --manifest-path src-tauri/Cargo.toml --lib live_auto_tag_eval -- --ignored --nocapture

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
