# Artist-library inventory

Use the offline inventory to account for an artist's physical audio files and
rank the next evidence-gathering work. It does not start the app, query providers,
call AI, write tags, approve decisions, or enqueue retries. You do not need to
launch `just web-local` to export it.

From the repository root:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --no-default-features --features server \
  --bin soundrobe-inventory -- "/path/to/artist" \
  --worker-data "$HOME/.soundrobe" --format json > /tmp/artist-inventory.json
```

For a readable export, choose `--format markdown`. Both formats go to stdout;
choose an output path outside your media library. The command hashes each file,
so a large library can take several minutes in a debug build.

## Accounting and evidence

The scan walks directories recursively, including loose and hidden audio files,
using Soundrobe's supported extension list. It retains each file's immediate
parent as its folder boundary, including separate disc directories. It lists
excluded symlinks and does not follow them. An unreadable file still counts;
its row includes a source error. A directory traversal error fails the export
rather than reporting an incomplete census as complete.

Each row records a canonical path, a whole-file SHA-256 fingerprint and observation
time. This is a stale-input check, not an audio fingerprint or proof of recording
identity. A scan is not a filesystem-wide snapshot. Changes after a file's
observation require a new scan.

`--worker-data` reads the existing `worker.db` through a read-only SQLite
connection. Omit it to scan without worker evidence. A missing database means
absent evidence; the command does not create a database or migrate its schema.
An unreadable or incompatible database fails the export.

Rows retain stored attempts, run IDs, provider failures and retry eligibility.
The current scheduling run takes precedence over older worker attempts; within
a snapshot the last terminal AcoustID diagnostic takes precedence over a
checkpoint. Without a current run, the export uses the last stored attempt for
triage, without claiming it is the newest provider observation. Scheduling update
times are not provider fetch times. Unknown fetch times remain null.

Legacy `candidateCount` counts eligible hypotheses; visible alternatives can also
include rejected candidates. `candidateSummary` labels the eligible count,
omitted eligible hypotheses and displayed alternatives separately. Legacy evidence
does not record the full alternative count, so that count remains null with
`fullAlternativeCountKnown: false`; the export does not infer it from a bounded
excerpt. Candidate-set completeness remains separate. Null means unknown, not zero.
Candidate disagreements describe the stored observations; they do not introduce
new rejection or acceptance rules. Ordered credits remain ordered, including guests.

## Historical composed audits

You can reconcile the coverage-v4 report with current files:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --no-default-features --features server \
  --bin soundrobe-inventory -- "/path/to/artist" --worker-data "$HOME/.soundrobe" \
  --historical .planning/debug/coverage-v4-duration/composed --format markdown \
  > /tmp/artist-inventory.md
```

The importer reads `preservation.json`, `file-audit.json`, `track-evidence.json`
and `albums.json`. It joins audits to the preservation manifest's absolute source
paths; it does not transfer evidence to a different file with the same name.
Duplicate or incomplete mappings fail. Historical totals come from these records,
not hard-coded report counts. The export lists historical inputs that are missing
or outside the requested root, and counts changed input hashes.

For cohort diagnosis, the export prefers the explicitly supplied historical
audit over worker snapshots, and retains both. This preference reproduces the
cohort's diagnostic context; it does not assert that the audit contains the newest
provider evidence.

An unchanged input hash means the original matches the historical replay's input.
It does **not** mean that the original has the replay copy's corrected tags.
The report's provider-backed core count describes those copied replay results,
not independent identity accuracy.

## Worklist and completion scope

The worklist includes every discovered file. Ranking uses:

1. Unresolved identification in a folder with one identification blocker.
2. Other identification work, favoring a missing fact linked to the same recording
   ID across several files, then provider-failure investigation.
3. Historically supported files that still need source-bound verification and
   readback on the current file.

Ties use the relative path. Hard cases remain in the export; this is not a new
scheduler or an automatic retry loop. These counts do not promise that resolving
one file will complete its folder.

`identificationUnresolvedFiles` separates files needing identity work from
`revalidationFiles`, which have historical support on an unchanged input.
`unresolvedFiles` includes every file without a current valid receipt. Legacy
worker/audit records lack that binding and do not become proof from tags or an
old folder status. Album/edition metadata, enrichment and human approval remain
separate and unverified by this command.

## Durable recording-core receipts

Schema version 2 adds per-file `coreProof`: `absent`, `pending`, `invalid`, or
`valid`. Only a receipt from the current scheduling run's terminal publication
can be valid. The export rechecks its whole-file hash, physical file identity on
Unix, and required receipt/evidence/decision versions. Any later whole-file
change invalidates it, even a metadata-only edit. Legacy fetch times stay unknown;
there is no recognition, donor reuse or provider refresh during export.

The existing strict recording resolver produces receipts inside its queued
write/readback operation. Each records pre/post-write sources, an attempt ID,
accepted and actual title/ordered billed credits/ID, the selected-recording or
existing strict ID-free-consensus basis, retained evidence and its digest,
timestamps, and encoded-payload preservation. The payload digest uses the same
format boundaries and supported preparation as the existing atomic writers,
including trailing FLAC APEv2 removal and bounded WAV orphan-tail repair; it is
not decoded-audio identity. Lookup-bound queued writes copy and hash one held
source into a private staging input, run the existing format writer there, and
recheck the original source before replacing it. A replacement after admission,
including identical bytes on a different Unix inode, is refused rather than
receiving the earlier source's proof. Source hashes are not protection against a hostile concurrent writer.
Unix device/inode identity detects identical-byte replacement; platforms without
that identity retain content checks, not an equivalent replacement guarantee.

The worker validates current sources and publishes receipts, run ownership,
terminal status and recording-review cleanup in one existing SQLite transaction.
Desktop and server completion hold exclusive write-queue coordination across
final source validation and that transaction, alongside the task cancellation
lock; the desktop does not start a scheduler. Publication errors cannot report a
completed task. Desktop review snapshots remain pending diagnostics; the durable
worker history and terminal task result carry published receipts. A new explicit
desktop request can reclaim interrupted desktop work only when its recorded
process owner is demonstrably dead. The claim is transactional, preserves earlier
evidence and starts fresh validation. Live owners, permission-unknown/PID-reused
owners and legacy/server rows without a desktop owner are not reclaimed.
Runtime ownership is stored separately from the original trigger. Server claims
record their own process owner and cannot overwrite a running desktop claim;
startup recovery leaves recorded live or indeterminate owners untouched. Legacy
ownerless rows retain the existing server recovery behavior, not desktop reclaim
permission. Manual server runs checkpoint per-file observations under their run
before terminal publication, including observations that do not yield a match.
Other review obligations remain open. A checkpoint's `pendingCoreReceipts` is
diagnostic only. Media and SQLite cannot commit together: a crash or persistence
failure after writing leaves missing/pending proof requiring revalidation, never
a reusable success. Partial successful files do not complete a folder. Historical
provider-backed copied results and AI/human-approved tags are not upgraded.

New acoustic observations distinguish an attempted lookup, successful empty or
nonempty response, and failure. Counts describe the parsed lookup response, not
the universal catalog; unknown eligible/full counts remain null. An incomplete
legacy zero is unknown, not a confirmed empty lookup; complete empty sets and
recorded positive counts remain distinct. Unattempted rows retain explicit null
counts even when a deadline prevents reaching them. Successful fresh
AcoustID responses retain a sanitized response URL (no userinfo, query or fragment),
HTTP status, fetch time and SHA-256 of the bounded HTTP-decoded response body.
This digest is distinct from the parsed-JSON recording-detail digest; credentials
and raw fingerprint requests are not retained. Legacy unavailable values remain unknown. Recording-detail
observations preserve partial fields even when strict parsing fails: raw ordered
billing and joinphrases, separately labeled role relationships, recording versus
release-track durations, response URL/status/time and parsed-JSON digest. Missing
duration never constructs a verified recording. Existing reasons and dispositions
still govern acceptance; writers/remixers are not promoted into billed performers.

## Opt-in exact audio diagnostics

`--audio-pairs /path/to/pairs.json` adds an `audioEvidence` section without
changing the inventory's counts, ranking, verification or tagging decisions.
Omitting the option does not decode audio. This offline diagnostic never queries
a provider or transfers identity between files. Use copies in an isolated folder
for investigation; no application startup or configuration changes are needed.

Select 1–16 pairs using paths relative to the inventory root and the full
whole-file SHA-256 values from that inventory. For example:

```json
{
  "pairs": [
    {
      "left": { "path": "padded.wav", "sha256": "<full SHA-256>" },
      "right": { "path": "shorter.wav", "sha256": "<full SHA-256>" },
      "offsetFrames": 3
    }
  ]
}
```

```sh
cargo run --manifest-path src-tauri/Cargo.toml --no-default-features --features server \
  --bin soundrobe-inventory -- /path/to/copied-media \
  --audio-pairs /path/to/pairs.json --format json > /tmp/audio-evidence.json
```

The optional signed offset means **left frame = right frame + offsetFrames**.
Without it, alignment is established only by equal whole PCM or equal PCM after
excluding exact-zero edge runs. There is no approximate lag search. Every sample
in the proposed overlap must be byte-identical before bidirectional coverage is
reported. A different ending or an incompatible interpretation cannot be averaged
away. Missing coverage means no exact correspondence was established, not that
two recordings are unrelated.

The export retains whole-PCM inequality, decoded hashes, maximal leading/trailing
zero counts, the zero-edge-excluded interval and diagnostic hash, matching
intervals/fractions on both timelines, and unmatched prefix/suffix intervals with
nonzero-frame counts. Every native channel must equal zero; one-channel silence,
phase cancellation, one integer LSB and tiny nonzero floats are not silence.
Float signed zeros count as zero numerically, but equality/hashes retain the
original decoded bytes. Interior silence is never removed. All-zero audio is
uninformative and gets no alignment; both maximal edge-zero runs span its whole
timeline, and its zero-edge-excluded interval is empty. No files are trimmed.

Supported on Unix with FFmpeg/ffprobe: mono/stereo FLAC (native 16/24-bit), WAV
(PCM 16/24-bit or float32), and MP3 (native float32 decode). There is no resampling,
gain fitting, downmixing or precision-reduction fallback. Other formats, layouts
and native precisions are explicitly incomplete/unsupported. Process versions,
arguments, stream/gapless interpretation, warnings and integrity basis are retained.
FLAC requires both native STREAMINFO MD5 and declared sample-count agreement.
Supported WAV uses little-endian RIFF: bounded chunk extents/padding, one native
format/data stream and declared/decoded frame agreement are required. Ambiguous
or unsupported layouts (including RF64), more than 4096 chunks, and proven
truncation are incomplete, not a complete observation with a warning.
WAV/MP3 decoder EOF does **not** certify an untruncated original master; a complete
observation is not an identity proof. Warnings remain visible even on exit zero.
WAV/MP3 decoding stops on decoder errors rather than accepting recovered partial
PCM. Benign warnings remain allowed; FLAC recovery is governed by checksum/count.

Source binding includes expected and before/after file hashes, held-file identity
and no-follow directory traversal anchored through held directories, including
the inventory root. Both pair inputs are held before either decoder runs;
directory/leaf bindings are checked again after observation and before comparison.
Paths deeper than 256 components are unsupported. A changed/replaced input, failed decode or invalid
integrity check yields incomplete evidence and null comparison values, not a
no-match. Failure/cancellation retains bounded available process diagnostics.
Source finalization is attempted independently of decoder success; an explicit
`finalizationError` explains unavailable hashing or path validation (including
cancellation, where hashing is not attempted). Observations are not a lock against
changes after export.

Each process is limited to 45 seconds, 64 KiB stderr and bounded stdout (256 MiB
decoded PCM per file); sources are limited to 512 MiB and the pair manifest to
64 KiB. Manifest inputs must be stable regular files, not symlinks, pipes or
devices; unsupported objects fail without waiting for a writer. Ctrl-C
cancels the invocation's Unix process group, reaps the direct child, and reports
cancellation. This includes wrapper workers that stay in that group, not hostile
descendants that deliberately detach into another session/group. Tool
locations use PATH or `AUTO_TAG_FFMPEG_PATH` / `AUTO_TAG_FFPROBE_PATH`.
Native-tool contracts are opt-in and can be run explicitly:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features server \
  --test inventory_audio -- --include-ignored
```

## Opt-in verified-audio reuse proposals

`--reuse-proposals /path/to/pairs.json` adds a separate `reuseProposals` section.
Use the same bounded manifest as diagnostics, **without `offsetFrames`**:
`left` is the donor and `right` the recipient. The option is mutually exclusive
with `--audio-pairs`. Omit both options for the unchanged default inventory.
There is no all-pairs scan, provider lookup, application startup or apply endpoint.

```sh
cargo run --manifest-path src-tauri/Cargo.toml --no-default-features --features server \
  --bin soundrobe-inventory -- /path/to/copied-media \
  --worker-data /path/to/isolated-existing-worker-data \
  --reuse-proposals /path/to/pairs.json --format json > /tmp/reuse-proposals.json
```

Choose `--format markdown` for the same proposal and rejection details in a
readable report. Missing worker evidence produces zero proposals, not a new
database. Only selected files are decoded, under the existing native-tool bounds.

Both inputs must have complete native FLAC checksum/sample-count validation,
compatible rate/channel/layout/precision descriptors, and byte-identical **whole**
decoded PCM. All-zero streams, exact-zero-excluded matches, sampled/partial
overlaps and approximate matches are ineligible. WAV and MP3 remain diagnostic:
the current decoder does not certify their untruncated source-master completeness,
even when decoded bytes match. Different FLAC metadata/container bytes are allowed;
no resampling, quantization, downmix, gain or trimming is introduced.

A donor needs a current terminal M2 receipt, valid receipt/evidence/rule versions,
unchanged whole-file hash and Unix physical identity, and no parent reuse proof.
Retained source-bound successful AcoustID lookup and recording-level MusicBrainz
facts must independently support the accepted core. Raw ordered billed names and
joinphrases must agree with the complete accepted billing; roles are not performers.
This first route abstains on sparse old observations, absent raw billing, or title
wording that does not exactly agree with retained authoritative title facts—even
when the underlying M2 receipt remains valid. Fetch-time unknowns remain unknown;
they are not refreshed or filled from file mtimes.

Worker evidence is reread after decoding. Pending/running/cancelled, superseded
or differently owned terminal records cannot supply a root. All held source and
directory bindings are rechecked after the entire selection and root validation;
an earlier pair cannot silently survive a later observed replacement. This is
an observation, not a joint filesystem/SQLite lock or a guarantee against future
external edits. Re-export after any input or proof change.

Conflicting valid core labels or decision bases on equal interpreted PCM among
**selected sources**, including valid recipients, block proposals. Unselected
sources are not searched. Repeated selections deduplicate to one proposal per
recipient path and one entry per identical donor proof/source. Multiple donor
copies never become independent corroboration votes; that count stays null.
Each proposal retains root run/attempt/receipt/evidence references, original and
current sources, provenance/freshness, comparison method/version, interpretation,
native tool versions and both integrity outcomes. Non-null parent lineage is
unsupported: proposals cannot become donors or create self/circular proof.

The proposed patch contains only title, full ordered billed performers and a
supported recording ID. Strict ID-free consensus remains distinctly labeled and
**omits** the ID from the patch; unknown does not mean “erase the recipient ID.”
Album/grouping, positions, dates, edition, enrichment and user approval are not
proposed. A proposal does not verify its recipient, complete a folder, write media,
publish a receipt, change worker state, or modify existing tagging acceptance.
Review is the next action; applying any proposal requires a separate authorized
guarded-write/readback route that is not implemented here.

Before running the web app against original media, use an isolated
`SOUNDROBE_DATA_DIR`, disable both background tagging and watching in that
directory's `background-tagging.json`, and use `just --no-dotenv web-local`.
Ordinary `just web-local` uses the shared app settings and may tag on startup.
Use copied media for write validation.
