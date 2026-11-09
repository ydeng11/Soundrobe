# Continue unified tagging with a validated feedback loop

## Start with the user's goal

Get every physical track tagged through provider evidence, agent assistance, or their combined evidence. Highlight assisted and uncertain fields for review. Preserve stronger evidence, meaningful version distinctions, and the user's decisions. Do not mistake a populated tag for a correct tag or a review acknowledgment for verification.

The next priority is the feedback loop that validates changes to this unified workflow. Do not spend the next session only repairing known provider-positive cases or creating a second agent-only tagger. Retain useful partial provider matches while assistance investigates the remaining files and fields. Deterministic code owns routing, budgets, retries, validation, and writes. The agent supplies semantic interpretation and grounded proposals.

This handoff is a continuation brief. The linked records contain the detailed execution history and raw evidence. The feedback-loop steps below are proposed next work, not implemented capabilities or completed verification.

## Resume the exact working state

- Repository: `https://github.com/ydeng11/Soundrobe.git`.
- Runner: `soundrobe-local`, macOS arm64.
- Checkout: `/Users/ihelio/code/auto_tagger`.
- Base before this checkpoint: `c7e99fe2a02c9fd2b94f04615558015f69fc1aba`.
- Native implementation checkpoint: `3e3c99a9c21cfb73b9bc2fe2c13986421eed8e15`. The following documentation commit preserves this brief and the replay procedure. Use `git log --oneline c7e99fe2..HEAD` for the complete local checkpoint series.
- Source thread: https://ampcode.com/threads/T-01a1b234-730d-7557-9ba6-fc105c23f683.
- Original design thread: https://ampcode.com/threads/T-01a1b1e8-a931-721f-a095-d50901931479.
- Completed implementation, fixtures, evaluation tools, and documentation are checkpointed in logical local commits. Nothing from this work has been pushed, merged, or deployed. Preserve unrelated shared-worktree changes and untracked files.
- Inspect current processes before starting another evaluation. Historical run directories are immutable evidence, not resumable output directories.

Use this checkout to retain the unpushed commits, dummy media, and hidden evidence. A default cloud thread will not have them. If another environment is required, transfer the local commits and required evidence, not just the remote branch or a thread URL. The tracked replay procedure recreates dummy media from captured metadata without the original library. Do not reset the checkout or regenerate frozen answers to make a run pass.

Read root `AGENTS.md` and applicable nested guidance before changes. Keep plans in `.planning/` and review evidence in `.amp/in/artifacts/`. `/.amp/in/` is already in `.git/info/exclude`. Original music must never be a write target. Use disposable synthetic media or explicitly scoped copies under `/private/tmp`. Secrets stay native and must not appear in prompts, logs, or artifacts. No paid live-model run or production write is authorized merely by this handoff.

## Read these records instead of reconstructing the thread

1. [Combined tagging workflow design](https://ampcode.com/artifacts/combined-tagging-workflow-design-c015f92a-30f9-4830-a776-b021c6b3a6f0), revision 8 when checked. Read the sections on one coordinator, bounded research, field authorization, and evaluation. Its referenced `.planning/design/tagging-recovery-architecture.md` is absent from this checkout. The artifact is accessible; do not assume the old file path exists locally.
2. [Progress and evidence report](https://ampcode.com/artifacts/dummy-eval-set-integrity-and-offline-replay-6b0dd7d6-6b86-4dee-bce3-d9741a2b219b), revision 56 before this handoff. Its workflow-framing correction supersedes earlier suggestions to split tagging into two products. Historical sections retain earlier outcomes and must not be read as current totals.
3. `docs/auto-tag-evaluation.md`, `docs/offline-provider-mock.md`, and `test/fixtures/tauri/auto-tag-eval/README.md` describe current tooling and evidence boundaries. Older introductory descriptions can refer to earlier populations. Check the expanded-suite and latest-result sections.
4. This handoff records the next work. The progress report records what actually ran.

## Completed work in this thread

The following groups cover the completed work. Use the report's matching headings and the named evidence directories for exact commands, diffs, source locks, and per-case results.

| Completed work | Result and reference |
| --- | --- |
| Corpus and dummy generation | Captured metadata-only inventories and generated 3,506 silent FLACs across 378 cases. All files were decoded and integrity-checked. Populations are original 321 cases, artist extension 33, curated 10, and library sample 14. The initial interrupted partial population was replaced and removed after verification. |
| Ground-truth and suite tooling | Expanded independently reviewed truth to 51 positive cases and 539 tracks. Added append-only review manifests, overlap-conflict checks, frozen input hashes, artist and release-family partitions, native readback exports, scoring, and script tests. `scripts/build_auto_tag_eval_suite.py` and `batch-suite.json` freeze membership. Original ledgers were preserved. |
| Provider replay and fixture completion | Repaired exact request coverage for artist browsing, pagination, searches, concrete details, and versions using genuine upstream captures with URLs, times, statuses, raw bodies, and hashes. The latest completion added 72 responses without replacing 1,450 previous discovery records. The imported mock now has 1,533 routes and the suite locks 1,579 inputs. |
| Native replay and readback | Repaired replay preflight and readback boundaries, completed diagnostic cold/warm runs for all 378 cases, and subsequently ran reviewed subsets and the full reviewed matrix. Real native coordinator, parsers, WriteQueue, and tag readers were exercised against loopback providers and disposable media. Unreviewed cases were not promoted to accuracy credit. |
| Folder and artist identity | Improved date/edition annotation cleanup, nested box/disc/container handling, artist-prefix cleanup, provider-backed CJK artist aliases, and canonical solo-name handling. Provider discovery order now tries scoped album/version work before expensive broad browsing where applicable. |
| Content and recording recovery | Added fully mapped album-content output when exact edition evidence is insufficient. Retained recording-only recovery and existing exact-audio grouping safeguards. Kept content, edition, and acoustic claims separate. Basic content output suppresses release/recording IDs while retaining independently resolved artist IDs. |
| Title precedence and guarded comparison | Preserved provider-supported skit/instrumental descriptors, guarded punctuation/diacritic/ligature/word-boundary comparisons with uniqueness and duration evidence, and canonical display titles. Watermark's `The Long Ships` comparison no longer needs a literal song exception. Version/remaster conflicts remain vetoes. |
| Credits and numbering | Preserved matched per-track display credits and ordered performer lists rather than replacing them with the album artist. Corrected solo alias handling. Implemented the previously requested sequential local-file-order numbering after alignment. This policy is deliberate, not proof of provider release positions. |
| Native writer and test reliability | Fixed ID3 disc-total removal retaining the wrong disc number. Repaired fixture teardown and a flaky OpenRouter shared-deadline test with controlled timing. These are verified fixes, not reasons to weaken production timeouts or budgets. |
| Expanded-Enya blockers | Verified reviewed representatives were already fetched but rejected by edition-year gates. Added a narrowly guarded content fallback for unrelated Discogs detail-budget exhaustion, without increasing discovery allowances. Matched full occurrences and retained version safeguards. |
| Utada evidence and aliases | Corrected the reviewed release's identity to the 2014 remastered digital edition. Captured independent official/store evidence and Apple bilingual receipts. Added provenance-backed catalog validation and one exact reviewed MusicBrainz occurrence binding in `data/provider-title-aliases.json`. Comparison aliases do not replace canonical display titles. Utada passes all six full-matrix evaluations. This is curated knowledge, not automatic alias research. |
| Remaining-abstention investigation | Executed 43 controlled native baseline/counterfactual probes. Established the Bellyache standalone-year defect and the missing Enya remaster-evidence boundary. No speculative counterfactual was promoted to provider truth. No fix for the Bellyache year defect was implemented. |
| Generality and workflow review | Reviewed core rules and alias ingestion independently, checked evaluator bias, identified remaining evidence-policy risks, and corrected the architectural framing. LLM-disabled replay is component evidence, not verification of the complete unified workflow. |

## Latest verified result, not a clean tagging pass

The full reviewed run used 153 fresh processes for 51 cases, three input profiles, and cold/warm phases. It produced 306 evaluations, 294 applied outcomes, and 3,132 written-track readbacks. The independent audit accepted 288 written evaluations and retained six title/credit findings. Twelve evaluations abstained.

| Input profile | Applied cases per phase | Independently clean written cases per phase |
| --- | --- | --- |
| `folder_filename` | 49 of 51 | 49 of 51 |
| `assisted_without_ids` | 49 of 51 | 48 of 51 |
| `tagged_recovery` | 49 of 51 | 47 of 51 |

The word `assisted` in the second profile means existing tags without IDs. It does not mean an active LLM. The native evaluator unconditionally sets `config.llm_api_key = None` in `src-tauri/src/commands/auto_tag_eval.rs`.

The initial full matrix had three missing HTTP routes. A later targeted completion replay closed all reached routes for the two affected filename-only jobs. Both still abstain cold and warm. That replay is four evaluations, not a second full matrix. It reports zero misses across 155 requests and no writes.

Native readback and payload checks passed. Cold/warm outputs and classifications agreed. Thirty exported per-file payload records were redacted; native comparisons used unsanitized values. Do not claim independent visibility of the redacted exports.

Historical broad checks include 706 passing native server-library tests with 17 gated tests ignored, 853 renderer tests, typecheck, and desktop library check. Strict Clippy had six verified pre-existing findings. These are prior results tied to their snapshots, not fresh passes for future changes. Later fixture completion passed 17 mock/review-tool tests and `just eval-auto-tag-suite`.

## Checkpoint verification on 2026-11-09

Fresh `just --no-dotenv check` passed on the committed source. It ran TypeScript checking, 853 renderer tests, 1,240 desktop-library tests with 29 ignored, 706 server-library tests with 17 ignored, 14 audio integration tests with 12 ignored, five recovery CLI tests, seven mock-provider tests, and ten review-tool tests. The gated tests were not selected. Additional checks passed all 79 script tests, the frozen suite check, archive hashes, script syntax, and 15 independent title-oracle contracts.

The initial `just check` exposed three desktop-only test initializers missing the alias-evidence field. These now explicitly carry empty evidence. That run also failed two existing server configuration tests because `just` loaded the local model and key settings from `.env.local`. Both passed without that environment, and the full gate passed with dotenv disabled. No test expectations, matching rules, or local credentials were changed to make the checks pass.

Logs are retained under `.amp/in/artifacts/checkpoint-verification-20261109/`, including the initial failed gate and the successful clean-environment gate. The complete reviewed cold/warm matrix was not rerun during commit preparation. Its results above remain historical evidence, not a fresh clean-metadata verdict. Credentialed and real-display smoke tests were not selected, and strict Clippy was not rerun.

## Evidence and code entrypoints

All paths below are relative to the checkout.

- Latest full run: `.amp/in/artifacts/reviewed-51-cold-warm-20261108/`. Start with `verification-summary.json`, `verification.json`, `selection.json`, `executions.json`, `run-native.cjs`, and `audit-expanded.py`. The frozen executable SHA-256 is `cf1f241375a820929bf8f3495b3696a808c4d654c7a67ff230ded9f5adede181`.
- Tracked reproduction tools: `scripts/auto-tag-eval-replay/README.md`, `run-native.cjs`, `audit.py`, `score-best-aligned.py`, `baseline-summary.json`, and `SHA256SUMS`. The driver, expanded auditor, title rubric, and historical summary are byte-identical copies of their original artifacts. The procedure restores their required relative layout in a fresh output directory. Raw binaries, generated media, caches, and large runtime logs remain local.
- Latest fixture captures: `.amp/in/artifacts/reviewed-51-fixture-completion-20261109/`.
- Latest affected-case rerun: `.amp/in/artifacts/reviewed-51-fixture-replay-3-20261109/verification-partial.json`.
- Controlled investigations: `.amp/in/artifacts/remaining-abstentions-20261109/`. Read `verification.json`, `baselines.json`, `probe-results.json`, source locks, and `native-probe-source.tgz`. Temporary probe source/media were removed after archiving.
- Title rubric: `.amp/in/artifacts/album-content-comparison-20261107/score-best-aligned.py`. The latest auditor imports it. Its standalone scoring mode is hard-coded to an older population; do not use that mode as the new workflow scorer.
- Earlier production fallback proof: `.amp/in/artifacts/album-content-implementation-20261107/`.
- Credits/numbering proof: `.amp/in/artifacts/reviewed-29-track-credits-20261108/`.
- Alias capture and implementation proof: `.amp/in/artifacts/utada-alias-evidence-20261108/` and `.amp/in/artifacts/utada-alias-ingested-filename-replay-20261108/`.
- Dummy sources: `.amp/in/eval-dummies/{original,artist-batch-extension,curated-hard-cases,library-sample}/`. Each has a corpus and `media/`. Do not use the original library as `SOURCE_ROOT`.
- Coordinator and field selection: `src-tauri/src/commands/auto_tag.rs`, `auto_tag_editions.rs`, and `album_search.rs`.
- Matcher and contracts: `track_matcher.rs`, `track_matcher_guarded_tests.rs`, `auto_tag_collections_tests.rs`, and `auto_tag_relapse_tests.rs`.
- Provider and alias ingestion: `src-tauri/src/state/providers.rs`, `provider_title_aliases.rs`, and `data/provider-title-aliases.json`.
- Existing LLM request/validation: `tag_correction_request`, `resolve_tags_via_llm`, `validated_ai_candidate`, `providers_confirmed_no_match`, and `fill_genre_if_missing` in `auto_tag.rs`; transport in `src-tauri/src/infra/openrouter.rs`.
- Review persistence: `src-tauri/src/state/worker.rs`; assistant orchestration in `src-tauri/src/commands/assistant.rs`. Existing review acknowledgment UI is in `src/web/MonitoringView.tsx`. Per-field highlights are not established by the replay and need inspection and verification.
- Native evaluator: `src-tauri/src/commands/auto_tag_eval.rs`; HTTP service: `scripts/mock-provider-service.cjs`; truth/review/scoring tools: `scripts/{review_auto_tag_subset,score_auto_tag_eval,build_auto_tag_eval_suite}.py`.

## Keep these unresolved findings visible

1. `0e7c8a578cea`, Bellyache Marian Hill Remix. A separate `(2017)` filename annotation contaminates title/version comparison. Removing only that bare year from all filename-derived channels permits the reviewed remix in probes. Wrong remix and studio negatives still reject. Implement a general metadata-year parser, not a literal Billie rule, and preserve `2017 Remix` and remaster-year claims.
2. `f7756354af37`, Paint the Sky with Stars. Fourteen filenames claim `Remastered 1997`. Discogs 636215 has compatible content but no captured remaster support. `Mastered By` does not establish remastering. Research authentic source-bound evidence within the unified workflow or retain highlighted uncertainty. Do not delete the claim or inject synthetic context to force acceptance.
3. `54e7923a3000`, dont smile at me. Four existing-tag readbacks omit Vince Staples on `&burn`. Selected Discogs 37353822 omits him; reviewed 13898510 credits him. A faithful copy of the selected provider can still be incomplete. This is a useful provider-plus-agent cooperation case.
4. `872fee627bfd`, Shepherd Moons. Two ID-assisted outputs include `Book of Days (Gaelic only version)` outside the frozen strict-title oracle. Adjudicate the source and recording meaning. Do not strip meaningful wording or silently broaden the oracle.
5. Content-only output may take an arbitrary representative edition's year when local year is absent and may take its genre. Review `album_content_candidate` and writer patches. Edition identity is explicitly unresolved, so date claims need their own support. This is source-confirmed policy risk, not a newly reproduced wrong write.
6. Local-file-order numbering is requested behavior but needs a scoped contract for flat multi-disc folders, partial albums, duplicate copies, and existing valid positions. Do not silently reverse that user policy. Test its boundary and report it separately from identity matching.
7. Alias duration validation lacks an explicit finite filter in one branch. Positive infinity can pass the tolerance predicate. No production source of infinity was established. Treat this as a predicate gap, not a demonstrated bad media write.

The corpus has 305 unverified and 22 unresolved cases in addition to the 51 positives. Twenty-seven positives are Enya, eight are Billie Eilish, and eight are 卢广仲. There are no scored verified-negative cases. Targeted negative tests exist, but the retrospective holdout has already been inspected. Do not call it blind generalization evidence.

## Build the feedback loop first

### 1. Freeze a baseline of the unified workflow as it exists

Start by checking the current Git state and checkpoint log, then run `just eval-auto-tag-suite`. Preserve the existing evidence. Freeze source hashes, the built native test executable, input and oracle hashes, model/provider configuration without secrets, prompt and tool versions, and explicit provider/model allowances.

Inspect assistance inside the same coordinator before changing it. Existing code already does LLM metadata fallback and genre enrichment. Its no-match-only metadata gate is an implementation limitation, not the target design. Do not replace the whole workflow with a new tagger.

Establish a repeatable native evaluation entrypoint that can exercise existing assistance under controlled configuration. The current evaluator disables the key, including for its nominal live mode. Do not merely delete that guard and accidentally enable paid/network calls in default tests. Use TDD to make offline mode deny unexpected network/model requests and make explicitly configured assistance observable.

### 2. Make every track's outcome auditable

Extend existing evidence exports rather than build an unrelated observability system. For each case, physical file, and affected field, retain:

- Original value, proposed value, written value, readback value, and the accepted or rejected action.
- Provider, agent, local, or combined provenance with source references and source-content hashes.
- File identity, recording/grouping/edition scope, mappings, competing evidence, and rejection reasons.
- Agent request, sanitized tool observations, model identifier, prompts, response, and consumed allowance. Do not expose secrets or unnecessary absolute paths.
- Whether the value is verified, an assisted fill requiring review, a held conflict, or still unresolved. These are proposed evaluation categories, not existing API enum names.
- Persisted highlight/review state and the state visible to the user. Record acknowledgment separately from verification and pending research.

Do not accept a model's score or another model's approval as the oracle. Structural validation checks shape and scope; source-grounded human review establishes semantic expectations.

### 3. Exercise provider and assistance cooperation

Start with a small diagnostic selection that covers each failure class before a full matrix. Include a straightforward provider success; a partial provider match plus grounded assisted fill; an agent-discovered release returned through normal provider validation; a collaborator disagreement; a bilingual alias absent from the bundled catalog; a version conflict; incomplete discovery; and cancellation or service failure.

Use the known Bellyache, Enya, Utada, and collaborator cases as development regressions, not untouched accuracy cases. Add independently reviewed negatives and new artist/release families. Include ambiguous repeated titles, missing durations, similar-duration wrong remasters/remixes, partial and mixed-disc folders, and conflicting existing tags. Review expectations before tuning behavior.

Retain all three input profiles and cold/warm isolation. Bind writes to files rather than provider array order. A provider match must survive unrelated agent failure. An assisted proposal must not overwrite stronger provider evidence or user decisions. Partial evidence must remain usable on resume.

### 4. Use two kinds of evidence in the same loop

For deterministic regressions, replay frozen provider responses and captured model/tool exchanges through the real coordinator, writer, and reader. Scripted adversarial responses are useful tests, but label them as scripted rather than observed model quality.

For actual research quality, run explicitly approved, bounded native model evaluations on disposable inputs. Record real model outputs and fetched source observations, then independently audit them. Freeze those observations for later replay. A model-generated URL is not evidence until the controlled tool fetches it. Do not claim replayed responses prove fresh model behavior.

Use the current full workflow as the before baseline and the changed unified workflow as the after condition. Hold input profiles, initial caches, evidence corpus, and total HTTP/model allowances comparable. Record actual requests, retries, failures, cache hits, turns, tool calls, latency, tokens, and cost when available. Model-disabled runs are diagnostic ablations only, not a separate tagging architecture.

### 5. Score useful completion and safety separately

Report a fixed denominator of requested physical files and cases, including failures and unavailable services. Track provider-verified fields, grounded assisted fields with highlights, held conflicts, unresolved fields, unsupported writes, wrong versions, and missing collaborator credits separately. Report exact-edition correctness separately from useful album tagging.

Fail the regression gate for new wrong writes, lost trusted values, dropped meaningful versions/credits, missing assisted highlights, payload changes, oracle changes without review, missing fixtures, or suppressed incomplete work. A correct assisted fill counts toward useful tagging coverage only with its required provenance and review highlight. An unresolved hard case is not a successful tag, but a highlighted unsupported guess is not success either.

Classify every difference as input parsing, provider coverage, provider selection, mapping/version checks, agent research, source grounding, field authorization, write/readback, review/highlight, or evaluator/oracle defect. Export machine-readable findings and a short human-readable summary. Make non-clean runs exit nonzero while retaining diagnostics; existing auditors sometimes exit zero with failure arrays.

### 6. Close one feedback cycle before expanding architecture

Select one observed failure with the largest supported effect on useful tagging. State the hypothesis and the expected observable difference. Add a regression that fails on the unchanged code and includes a counterexample where the tempting wrong fix would pass. Make the smallest change in the existing workflow.

Rerun the affected cases cold and warm, then the safety set and previous successes. Compare independent field outcomes and review/highlight state, not just native `Applied` or process exit. Only expand to all reviewed cases after the small loop produces trustworthy differences. Keep an untouched artist-family set for scheduled final evaluation; once used for tuning, relabel it as development evidence.

Keep a short decision record per iteration with the failure, hypothesis, exact source/input snapshot, change, before/after evidence, and unresolved findings. Carry real discoveries into versioned fixtures after review. Never turn a promising candidate, synthetic probe, model answer, or user acknowledgment directly into ground truth.

The first milestone is one executed baseline-to-change cycle with an independently audited result and user-visible assisted highlights, followed by a reproducible command the next session can rerun. The loop is not complete merely because a new runner script or score file exists.

## Reuse the native replay correctly

Use `cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features server --lib live_auto_tag_eval --no-run --message-format=json` to build the current native test executable. Select the compiler artifact whose target is `soundrobe_lib` and which has an `executable`. A changed binary needs fresh source locks; never pretend the old frozen executable contains new code.

The full-matrix `run-native.cjs` is tied to its own `__dirname`, requires sibling `build.jsonl` and audit files, uses exclusive creation, and freezes selections. Reuse it only in a fresh sibling artifact directory after inspecting its assumptions. Do not execute it inside the historical directory. Some earlier subset runners copy prior successes unless passed `--all`; inspect the chosen runner and force fresh execution for changed behavior. Inherited selection metadata is not evidence that every selected case ran.

Each native invocation creates a new temporary cache and separate cold/warm media. Warm reuses the cold cache, not the already tagged cold files. Phase JSONL files append, so do not reuse output directories. A missing mock URL enters live provider mode in the current evaluator. Require explicit mode checks in the new loop. Mock fixtures need completion before interpreting resolver failures as semantic failures.

Useful existing gates are `just eval-auto-tag-suite`, `npm run test:review-eval`, `npx vitest run test/scripts/mock-provider-service.test.ts`, targeted Cargo tests, and the normal repository checks. `just eval-auto-tag-live` still disables LLM assistance and is not a ready-made unified evaluation command.

For review/highlight changes, inspect the current native/renderer contract and existing UI first. Verify DOM/accessibility behavior for interaction changes. If appearance changes, render and inspect screenshots at 1920×1080 and 390×844 as required by `AGENTS.md`. Do not claim field-level highlighting is implemented because an album-level `needs_review` label exists.

## Suggested skills

- Load `using-superpowers` at session start and `graphify` for source questions. The existing graph points to obsolete Electron code; use current source when the graph is stale and do not rebuild it outside scope.
- Load `brainstorming` before implementation decisions. Use the existing combined-workflow design rather than restart the architecture discussion.
- Load `writing-plans` for the scoped feedback-loop implementation and `test-driven-development` for code changes, as required by repository guidance.
- Load `systematic-debugging` for observed failures and `verification-before-completion` before success claims.
- Load `proving-changes` when validating the executed before/after loop. Use `frontend-design` only if UI details change, and follow the reference tokens.
- Load `pstack:technical-writing` and `pstack:unslop` for updates to this handoff or operator docs.
- Use `handoff` when transferring the next session. Keep the working handoff in `.planning/handoffs/` under the repository's explicit artifact-location rules.

## Delivery and next action

The completed work is committed locally in groups for evaluation fixtures and tools, deterministic OpenRouter deadline tests, the MP3 disc-total fix, native tagging and evaluation changes, and documentation with the reproducible replay procedure. No push, merge, deployment, or new thread was performed. The final checkpoint preparation only completed three desktop-only test initializers for the alias-evidence field. It did not change matching behavior or rerun the full reviewed matrix.

Start the next session by reading this brief, the original design artifact, and the latest evidence summary. Then implement the smallest repeatable unified evaluation loop, beginning with its baseline and observable review/highlight contract. Preserve the completed provider/matcher evidence as regression coverage and continue toward every track being usefully tagged, with assisted results highlighted.
