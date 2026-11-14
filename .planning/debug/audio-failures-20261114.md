# Offline audio failure investigation

## Scope

Investigate the five settled failures in `offline-http-transport-20261114/server-08.log` and focused failure in `flac-copy-14.log`. Do not change limits, alter the OS, activate calls, use credentials, touch original media, clean unrelated files, commit or push. Preserve the existing eight-path implementation diff and frozen evidence.

## Known evidence

- Broad gate failed: 874 passed, five failed, 31 ignored. Three failure sections explicitly retain the 120-second duplicate-group batch deadline. Two assertions omitted their proof diagnostics.
- Focused FLAC-copy rerun failed: zero writes versus sixteen, with batch and decoder/probe deadlines. The subsequent AI-unconfigured fallback is not permission to enable AI.
- One generated-media FFmpeg child was sampled at `_dyld_start` after 40 seconds alive. This does not establish the OS cause or all five failure causes.
- The stored graph describes the removed Electron tree, not the maintained Rust audio proof. It is not evidence for the current failures; no graph rebuild is needed.

## Decisions before experiments

1. Read current `prove_duplicate_groups`, `decode`, `run_tool` and the failed assertions. Retain a source/log binding before instrumentation.
2. Add only temporary diagnostic output to the two existing tests that omitted their actual proof reports. Keep fixtures, assertions, decoder selection, process and batch limits unchanged. Build a separately identified diagnostic executable, retain the exact patch, and restore only these owned diagnostic edits afterward.
3. Run those two exact tests serially once. If either fails with a retained batch/probe deadline, classify that reproduction's direct mechanism. If either passes, do not claim its earlier failure was diagnosed. No full-suite rerun or production fix is part of this investigation.
4. During a slow actual child launch, take read-only parent/child process samples. In particular, check whether native `Command::spawn` blocks the current-thread Tokio executor before its per-process timer starts. This is a hypothesis from source, not a diagnosed cause.
5. If needed, run one isolated public `ffmpeg -version` and `ffprobe -version` control to distinguish ordinary tool startup from media decoding. Record launch, spawn-return and exit separately. These are diagnostic observations, not a performance benchmark or a provider test.

## Success and limits

Produce a failure-by-failure account separating original evidence, new reproduction, direct trigger and unresolved OS cause. Keep original non-green gates non-green. Reverify all original source bindings after restoring instrumentation. Retain raw logs, samples, diagnostics and executable bindings under `.amp/in/artifacts/audio-failure-investigation-20261114/` and update the existing execution decision artifact.

## Observed outcome

- The isolated version controls exited zero: FFmpeg launch through exit 0.0544 seconds, FFprobe 0.0276 seconds. These one-off observations show current ordinary startup works; they do not diagnose the earlier `_dyld_start` sample or measure decoder throughput.
- A separately retained diagnostic binary was built offline in 21.64 seconds. Its only changes were two `eprintln!` statements after obtaining the existing proof. Both affected tests passed, zero failed/ignored, in 0.84 seconds. Metadata observations all completed with stable sources and matching FLAC integrity. Identical PCM hashes joined the differently tagged files; different audio had a different hash. The rejection test retained all six observations with the intended checksum/hash/codec errors and all-zero PCM classification.
- Temporary logging was removed and the test source compared byte for byte with its retained original. The generated target executable was restored from the original frozen executable. No production source, assertion, fixture generator or limit changed.
- Because startup and diagnostic tests now worked, all five formerly failing tests were run once, serially, using the exact original retained native12 executable, SHA256 `c894c2a8d86c3cc5ff574b5f05f2f30bcc3dc2542e00b6038fa020f9d705da66`. Result: five passed, zero failed/ignored, 905 filtered out, in 19.35 seconds. This includes sixteen-write integration success, conflict review, preserved payloads/credits, and all four original grouping assertions. This targeted check is not a full-gate rerun.
- No new slow child was observed, so the conditional parent/child sampling and spawn-block hypothesis were not pursued. The original startup delay is still unexplained. No root cause was established for the two original failures that omitted diagnostics. Fresh fixtures follow the unchanged generators; their exact byte identity to discarded historical fixtures is not established.
- Read-only verification before and after rechecked all 1,894 frozen source bindings, all 24 prior stable evidence hashes, the retained/current original executable, exact eight-path implementation diff, empty index and unchanged completed failed-gate evidence. Original broad/focused gates remain failed and are not relabelled as passes. No full regression gate, native/mock service matrix or real-model evaluation was run.

## Reproduction records

`tool-startup.py` records the separate launch/spawn-return/exit controls. `diagnostic-only.patch`, `diagnostic-build.log`, `diagnostic-test-executable` and `two-diagnostic-tests.log` bind the diagnostic run. `five-original-tests.log` binds the original-executable five-test run. `before-verification.json` and `after-verification.json` retain the source/evidence reconciliation. `verify-investigation.py` rechecks these records without launching tests and emits `investigation.json`.

No source fix is justified by the present non-reproduction. Further OS/root-cause claims would require observing the actual slow decoder/probe and parent during a recurrence. If it recurs, retain both process samples and the exact proof report before taking any recovery action. Do not increase limits or supply credentials as an audio-test workaround.
