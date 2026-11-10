# Candidate-scoped Discogs name observations

The parser now retains Discogs primary-artist canonical and credited-name pairs, bound to their release, artist ID, and primary-credit index. This is evidence retention only. No matching predicate, request alias, genre choice, display credit, source certificate, or write authority consumes the new observations.

The completed paired replay passes its unchanged-behavior comparison. Recovery still rejects the genre donors for an artist conflict and leaves eleven genres empty per phase. Overall safety remains **non-clean**. The unchanged source auditor reports four `source_eligibility` findings in each arm. The historical 84 findings and original four rehearsal findings remain unchanged, not replaced by these repeated observations.

## Source ownership and cache compatibility

`DiscogsArtistNameVariant` contains `releaseId`, `artistIndex`, `artistId`, `canonicalName`, and `creditedName`. `ProviderAlbum` exposes `discogsArtistNameVariants`. Auto-tag and manual-search candidates retain the same typed observations. Their native candidate JSON uses the existing snake-case convention, and the shared renderer contract uses camel case.

The parser reads only release-level `artists[index]`. It requires a positive raw release ID equal to the requested release, a positive artist ID, and nonblank `name` and `anv`. Missing or insufficient binding produces no observation. It preserves raw spelling and separate observations for jointly billed primary entities. Guest, track-level, producer, writer, and instrumental credits do not supply this field.

The authentic motivating receipt is Discogs `37231524`, `artists[0]`, pairing `Crowd Lu`, `盧廣仲`, and ID `2461941`. Its captured body is `test/fixtures/tauri/auto-tag-eval/provider-snapshots/reviewed-22-completion-20261108/discogs-036e0c8fc2fc0165.json`, SHA-256 `1798a65076cb0ab25f376751bf30fbc42677702ec54b5ffe29f4139f0c939b1a`. Existing HTTP receipts supply capture provenance. No timestamp, cross-provider link, or capture origin is invented.

Candidate conversion forwards the field. Candidate merging retains only the preferred candidate's observations, not another candidate's name pair. Chinese display conversion leaves the raw observations unchanged. SQLite release-detail and candidate lookup caches retain the field across reopen. Empty observations are omitted. Legacy rows deserialize empty. There is no migration, namespace bump, forced refetch, or additional HTTP request. An existing cached row cannot recover previously discarded ANV until an ordinary fetch supplies it.

Production references to the new field are declarations, parsing, and forwarding only. It never enters request/global aliases, a user alias file, local decisions, eligibility, display rewriting, or a performer certificate.

## Red and green public-boundary tests

The retained red run fails two public Discogs HTTP-boundary tests because the parsed observation is absent. The implementation makes those tests pass. A third authentic homonym-search isolation test also passes.

The three tests cover the authentic body hash, both candidate conversions, display conversion, preferred-candidate ownership, SQLite cache reopen, and legacy rows. Controls cover absent/blank ANV, missing artist binding, mismatched release ID, blank canonical name, guest-only assertions, distinct joint primaries, another artist entity on the next request, and plausible but unaccepted homonym search hits. Altered bodies are labeled test controls, not catalog captures or replay fixtures. The fetched ANV still supplies no accepted resolver alias or provider ID.

## Paired native observations

Fresh execution evidence is `.amp/in/artifacts/primary-artist-variants-20261110/`. `before/` uses the previously frozen executable. `after-retry/` uses the changed build. Both execute exactly Crowd Lu `036e0a00e45b` under all three profiles, with cold and warm phases.

Each successful arm contains three jobs, six evaluations, eleven distinct generated source paths, and 66 repeated file observations. The paired comparison covers twelve evaluations and 132 observations. These are exposed diagnostic fixtures, not holdouts, acoustic evidence, or assistance accuracy.

| Profile | New provider requests, cold + warm | Written files per phase | Missing genres per phase | Chosen genre source |
| --- | ---: | ---: | ---: | --- |
| `folder_filename` | 16 + 0 | 11 | 0 | Discogs `37231524` |
| `assisted_without_ids` | 16 + 0 | 11 | 0 | Discogs `37230261` |
| `tagged_recovery` | 15 + 5 | 11 | 11 | None, artist conflict |

Each arm has 52 provider requests, zero model requests, and zero fixture misses. Every profile explicitly disables assistance. Strict loopback fixtures have no upstream escape or live-model capability. The route pool contains 1,533 authentic captured routes, not invented empty responses. Each job has a fresh process, provider state, aliases path, SQLite cache, and worker root. Warm reuses only the corresponding cold cache and receives fresh profile-equivalent media.

The comparison checks all exported phase-record fields, native candidates, proposed values, prewrite/readback tags, physical paths, full and payload hashes, provider attempts, persisted diagnostics, review state, cache counters, donor predicates/reasons, and ordered request/query/status arrays. It also compares the full non-authorizing snapshots, including guards. Allowed differences are the temporary root UUID, `elapsedMs`, the compiled `auto_tag.rs` hash, and the new source-bound observations. No unexplained differences remain.

The observed pairs belong separately to releases `25560652`, `37230261`, `37230264`, and `37231524`. Each pairs the same primary entity ID `2461941` with `Crowd Lu` and `盧廣仲`. The comparator verifies every retained pair against its own authentic raw primary-credit object. Negative controls reject wrong entity/release binding, cross-provider attachment, eligibility changes, artist-alias insertion, genre changes, and numbering changes.

All twelve paired exports remain `decision: insufficient`, `acceptedCertificate: false`, and `writableDelta: []`. The final acoustic qualifier, selected IDs, content-only ID suppression, numbering, credits, payloads, and persisted review remain unchanged. Missing donor durations, acoustic qualifier, and independent album/title certification remain findings. No AI uncertainty, Keep, or native acknowledgment claim follows from this disabled run. Grounded-assistance accuracy credit remains zero.

## Failures, storage, and retained history

`after/` retains the first changed-build attempt. Two jobs panic while writing cold evidence with `StorageFull`; they have no completed phase records. Recovery completes both phases. After lossless compression, the fresh `after-retry/` run completes all three jobs. The failed arm is not counted as another successful six-evaluation arm.

Across all three execution attempts, nine jobs start and seven finish. Fourteen complete phase records exist, including the two extra recovery records outside the paired comparison. All nine inventories total 156 provider requests, zero model requests, and zero misses. Two partially executed cold phases are not counted as complete evaluations. Their logs and partial files remain.

Large new JSON files were losslessly compressed to `.json.xz`. `compression-receipts.json` records each uncompressed SHA-256, byte length, and old/new compressed size. Each compressed stream was independently decompressed and hashed before its redundant gzip copy was removed. No historical evidence, original media, or unrelated file was removed. The comparison accepts JSON, gzip, and xz inputs.

The initial broad gate also fails because the desktop compiler runs out of disk. Its renderer, provider-mock, review-evaluation, server, and server integration checks complete. The failed log remains. The first comparator attempt rejects a historical documentation difference already committed in HEAD. The corrected comparator checks archived native/shared source scope and separately verifies frozen input locks. Neither failure changes an oracle.

The frozen packet and its seal remain unchanged, SHA-256 `64ad68b53ca5d97a335cada1e0eb3d4c8c86651b677a24a484ac4e30e03d6567`. Streamed checks of 1,816 frozen source locks find only the four authorized production-file differences. All other locks match. The old `prepare.py --verify` requires historical source equality and is not claimed to pass after this approved change.

The original rehearsal manifest remains `d6626296d5af352975a5822a1caecfc9471215216502871686811806d111ec45`. All 542 referenced files were streamed. Only its two external working-copy references, `run-rehearsal.cjs` and `test_rehearsal.cjs`, differ after the bounded-selection extension. The other 540 references match. The old manifest and sealed runtime outputs were not rewritten. A current working-copy verification of that old manifest is therefore not an unconditional pass.

The scheduler extension accepts a locked nonempty set of unique cases while still requiring exactly the three existing profiles and disabled assistance. Its original five-case controls remain. Four scheduler tests pass, including empty/duplicate selections and attempts to enable models. This is not a hard-coded 51-case run relabeled as Crowd Lu coverage.

## Verification and reproduction

Fresh targeted checks pass three provider boundary tests, four scheduler tests, three comparator controls, ten frozen-packet tests, 18 exporter tests, eight safety-audit tests, four source-field tests, and five preservation tests. The unchanged source auditor runs on the twelve paired records and retains four findings per arm.

Fresh `just --no-dotenv check` passes on retry, exit 0. It includes TypeScript checks, 856 renderer tests, nine mock-provider tests, ten review-evaluation tests, 1,267 desktop Rust library tests, 733 server Rust library tests, fourteen media-evidence integration tests, and five recovery CLI tests. The renderer suite includes 82 Tauri-adapter tests. Ignored counts remain 29 in the desktop library, seventeen in the server library, and twelve in the media-evidence suite. These are not claimed as executed. Credentialed, original-library, live-model, packaged UI/Keep, and acoustic verification remain outside this change's proof. `cargo fmt -- --check` and `git diff --check` also pass. The initial failed broad gate remains in `check.log`; the successful full retry is in `check-retry.log` and `check-retry.exit`.

From this checkout, these commands reproduce the parser tests and retained comparison without live calls:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features server --lib discogs_primary_variants -- --nocapture
python3 -B .planning/plans/test-primary-artist-variants-comparison-20261110.py
python3 -B .planning/plans/compare-primary-artist-variants-20261110.py \
  .amp/in/artifacts/primary-artist-variants-20261110/before \
  .amp/in/artifacts/primary-artist-variants-20261110/after-retry \
  /absolute/fresh/comparison.json
```

The last command requires an existing parent and a nonexistent output file. Runtime replay requires a fresh directory with the retained `execution-plan.json`, then `node scripts/auto-tag-eval-replay/run-rehearsal.cjs <fresh-directory>`. Its plan binds the executable, authentic routes, generated media, source, selection, and disabled-assistance settings. It refuses an existing `runs` directory. The historical binary SHA-256 is `69452eb3021d8c2f215a8e45e3b416ec0bd47a8d70221d7b95bc033ef5ac6388`; the changed binary is `add2754fa54176d54b8f857d346f951e700d45ce95ab38cc5bcfdc25a6c61cfb`.

The changed source archive is `after/source.tgz`, SHA-256 `2ead101ecf51523afd53625971fa94fb84dc7465484c3db37661a185bf96758a`. Its 95-file scope differs from the historical 119-file archive. The comparison checks historical native/shared files against current source and permits only the four approved implementation files to differ. New tool, source, runtime, and failure hashes belong to the new manifest, not the old seals.

Production changes are `src-tauri/src/state/providers.rs`, `src-tauri/src/commands/auto_tag.rs`, `src-tauri/src/commands/album_search.rs`, and `src/shared/desktop-api.ts`. `docs/CHANGELOG.md` describes the observable diagnostic addition without claiming better tagging. The incoming scheduler files have the bounded-selection extension. The plan and comparison tools are under `.planning/plans/`; this report is under `.planning/research/`.

All changes remain local and uncommitted on `artist-library-inventory` at `e9edf1c0086cc60ffffcea6e5970d4b90ae27327`. Nothing was pushed or deployed. Unrelated archive/cache files and original music remain untouched. The next decision about using a retained name pair for eligibility is separate and is not implemented here.
