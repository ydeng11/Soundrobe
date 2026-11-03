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
`unresolvedFiles` includes both. The current legacy worker/audit stores lack a
durable post-write per-file proof binding, so `verifiedCoreFiles` remains zero:
this command does not manufacture a fresh verification record from tags or an old
folder status. Album/edition metadata, optional enrichment and human approval
remain separate and unverified by this command.

Before running the web app against original media, use an isolated
`SOUNDROBE_DATA_DIR`, disable both background tagging and watching in that
directory's `background-tagging.json`, and use `just --no-dotenv web-local`.
Ordinary `just web-local` uses the shared app settings and may tag on startup.
Use copied media for write validation.
