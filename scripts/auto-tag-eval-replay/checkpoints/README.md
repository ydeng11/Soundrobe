# Reanalyse the retained tagging checkpoints

These are historical checker sources, receipts, and expectation records. They
are not another tagger or a new execution route. `copies.json` binds thirty
byte-identical copies to their original local files. The original runs, failed
checks, and contracts remain unchanged under `.amp/in/artifacts/`.

Use the [main replay procedure](../README.md) for a fresh native execution from
tracked provider fixtures and generated silent media. Use these checkpoints for
read-only reanalysis of retained native outputs. Do not run the historical tools
directly in this source directory or inside a sealed run.

## What a checkout can reproduce

With the repository dependencies, Python 3, and FFmpeg installed, a checkout can
run the renderer/native regressions, strict mock tests, exporter contracts, and
the maintained 24- and 72-evaluation assistance scripts. It can regenerate all
four dummy populations and execute the maintained 51-case provider-only replay.
The genuine provider captures, review ledgers, request pools, and suite locks
already live under `test/fixtures/tauri/auto-tag-eval/`.

These additional checker tests need no historical runtime outputs:

```sh
python3 -B scripts/auto-tag-eval-replay/test_evidence_export.py
python3 -B scripts/auto-tag-eval-replay/test_safety_audit.py
python3 -B scripts/auto-tag-eval-replay/checkpoints/content-preservation/test_field_audit.py
python3 -B scripts/auto-tag-eval-replay/checkpoints/content-preservation/test_preservation_audit.py
```

The checkpoint comparators, receipt regressions, and genre-routing regressions
also use actual historical negative inputs. Those inputs are not bundled here.
Their absence is a missing reproduction dependency, not a reason to skip a
failed check or substitute a new passing fixture.

## What still needs local evidence

Exact reproduction of the preservation comparisons and 440-evaluation genre
experiment requires the retained local evidence, including frozen executables,
source archives, original/profile hashes, certificates, manifests, inputs,
phase records, and wire inventories. There is no public artifact download or
fresh-clone restoration command for those data. Markdown reports and attachment
links do not supply the complete raw experiment.

The required directories are beneath `.amp/in/artifacts/`:

- `performer-evidence-export-20261109` supplies the strict comparison helpers.
- `reviewed-51-provider-first-20261109` supplies the original full-matrix baseline.
- `reviewed-51-preservation-20261110` retains the demonstrated composite-credit
  failure used by the receipt and comparator regression tests.
- `reviewed-51-content-preservation-20261110` supplies the corrected 306-evaluation
  baseline, certificates, profile inputs, and independent field auditor.
- `genre-ai-gaps-20261110` supplies the original 440 evaluations and contract.
- `genre-ai-routing-contract-v2-20261110` supplies the separately corrected
  Enya routing contract and its reanalysis.

The independent review also needs the provider-first manifest. Its copied
extractor reads tracked original metadata and hash-verified catalog captures,
not native results or original audio. `evidence-locks.json` and
`review-receipt.json` preserve its source receipts. The separate field-scoped
certificate proposal remains a proposal, not permission to write.

Historical manifests and full `freeze.json` files retain absolute paths from the
original workstation. Those paths are historical bindings, not portable defaults.
`genre-gaps/selection-v1.json` is a portable excerpt of all 220 original jobs,
scripts, budgets, and expectations. Only its three case input paths become
repository-relative. It omits runtime/source locks and Git state, so it is not a
replacement execution freeze. `genre-gaps/freeze.py` records the original
derivation from the preceding baseline. Rebinding a new execution needs fresh
source, executable, generated-media, and tool locks. It must not modify a frozen
contract or pretend a newly compiled executable has an old hash.

## Verify the local evidence before reanalysis

Use streaming SHA-256 checks of each manifest's `files` entries. Compare both
file sizes and hashes, one file at a time. For the v2 addendum, also verify its
21 symlink targets and the separately sealed original experiment.

| Local evidence manifest | SHA-256 |
| --- | --- |
| `reviewed-51-content-preservation-20261110/evidence-manifest.json` | `3177b40527d7ef0b9ba5fed21341efe17c49bfc6274fc4382fe670e03cda5b5a` |
| `genre-ai-gaps-20261110/evidence-manifest.json` | `86d69ab72c3c8e62806763f72ddd063d90eb5453bd1e54728e99b2b5334bdfd8` |
| `genre-ai-routing-contract-v2-20261110/evidence-manifest.json` | `8b96640e38aeef7a26e7e572a813ae8195d13f8db45de1e11b7c7b0de26781a1` |

The first two manifests bind 3,361 and 5,358 files. The v2 addendum binds 44 new
files and 21 references. Keep every manifest and original failed verdict. The
active preview log is explicitly outside the original genre-experiment seal.
Do not compare Git HEAD or staging with a precommit postflight receipt after
delivery commits. Historical Git-state checks are evidence about that execution,
not an invariant for future commits.

## Reanalyse without another native or model request

Run from the repository root after verifying the local dependencies. The output
directory must be fresh and exactly four levels below the repository root,
under `.amp/in/artifacts/`. The copied tools retain their original relative-path
contract. The main README explains the repository-local exclude rule.

```sh
set -euo pipefail
tools="$PWD/scripts/auto-tag-eval-replay/checkpoints"
old="$PWD/.amp/in/artifacts/genre-ai-gaps-20261110"
v2="$PWD/.amp/in/artifacts/genre-ai-routing-contract-v2-20261110"
view="$PWD/.amp/in/artifacts/genre-reanalysis-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir "$view"
for file in audit.py test-audit.py exchange-summary.py construction-diff.py paired-hashes.py supplement.py; do
    cp "$tools/genre-gaps/$file" "$view/$file"
done
cp "$tools/genre-routing-v2/test-contract.py" "$view/test-contract.py"
cp "$v2/freeze.json" "$view/freeze.json"
cp "$old/exchange-tool-lock.json" "$view/exchange-tool-lock.json"
cp "$old/paired-hashes-tool-lock-v2.json" "$view/paired-hashes-tool-lock-v2.json"
ln -s "$old/runs" "$view/runs"
ln -s "$old/pilot" "$view/pilot"
python3 -B "$view/test-audit.py"
python3 -B "$view/test-contract.py"
python3 -B "$view/audit.py"
python3 -B "$view/exchange-summary.py"
python3 -B "$view/construction-diff.py"
python3 -B "$view/paired-hashes.py"
python3 -B "$view/supplement.py"
```

This executes analysis only. The primary audit, strict exchange checker, and
construction/hash checks pass under v2, with the non-clean findings below still
reported. The four original negative controls and six routing controls must
pass. The five complete-discovery no-match controls still require metadata
assistance. Removing those requests must fail.

To inspect the original verdict, make a second fresh view with the original
`freeze.json` instead. Do not run `test-contract.py` as an old-contract positive
test. Run the primary and strict exchange checks separately and retain their
exit codes. Both exit 1 under v1 because the two Enya phases expected metadata
requests. They are not new native failures. No sealed output is overwritten.

`genre-routing-v2/contract-correction.json` binds the sole authorized change,
`/jobs/215/expectedModelRequests`, from 2 to 0. Its script and allowance stay
unchanged. Incomplete forty-detail Discogs discovery is not confirmed no-match.
The source gate therefore prevents metadata fallback and leaves that script
unexercised. The original freeze hash is
`135fea8766e5a01355065b5a0a1d56bff8b8f11098bc33d018ecca2d200d5fc3`.
The corrected freeze hash is
`5bdfa3f5af43ed1237bd6ca3a1dc3937072419120c264794f036c165f8256d86`.
The copied `prepare-correction.py` records the derivation but is not part of
the read-only command sequence above.

## Expected findings are part of the result

The final full matrix has 153 jobs, 306 evaluations, 539 unique physical paths,
and 3,234 repeated file observations. It writes 294 evaluations and holds twelve.
The 68 written genre-incomplete evaluations motivated the genre experiment.
All 28 preservation receipts are fulfilled. Provider requests total 5,068, with
zero model requests and no fixture misses. Assistance is disabled in that matrix.

The original independent source auditor still reports 84 certification findings.
The unchanged legacy audit still reports two title, 32 selected-credit, twenty
collaborator, and four numbering disagreements. These are distinct contracts.
Do not replace their expectations with the preservation outputs.

The genre experiment reuses 34 gap jobs across six scenarios and adds controls.
Its 220 jobs produce 440 evaluations, 258 unique physical paths, and 5,322
repeated file observations. It has 348 written, 24 held, and 68 cancelled
evaluations. The observed genre ledger contains 852 AI-inferred/unverified,
26 provider, ten explicit local, and 4,434 missing observations. Its exact
exchanges contain 10,529 provider and 352 scripted model requests, with 342
genre and ten metadata roles. No fixture misses or additional retries occur.

The disabled-only field auditor still reports 360 assistance-boundary, seventy
missing-genre-receipt, seventy unsupported-genre, and two source-eligibility
findings on this repeated selection. Legacy results remain 42 selected-credit,
ten collaborator, eight numbering, and zero title findings across 278 applied
rows. The old 84 findings do not disappear under scripted genre values.

Every evidence export remains insufficient with an empty writable delta and
no accepted certificate. Scripted answers earn zero grounded-assistance accuracy
credit. Redacted payload hashes, unexported tags, acoustic identity, and live-model
quality remain unverified. Native persistent review is checked. The inspected
UI Keep interaction uses a component stub, not packaged acknowledgment.

The [current feedback record](../../../docs/tagging-feedback-status.md) links
the detailed reports, original failures, independent review, and proposal.
