# Crowd Lu's request-specific alias and source gap

Recovery rejects Discogs `37231524` because its request has no accepted `Crowd Lu` alias. The known MusicBrainz artist ID suppresses MusicBrainz identity search, even though that search is the resolver's only alias source. Discogs artist search still runs. Its Chinese query returns no exact accepted name. No accepted MusicBrainz alias was fetched and then lost.

There is also fetched identity information that the current parser does not represent. Discogs `37231524` explicitly pairs canonical `Crowd Lu` with credited-name variant `盧廣仲` in the same primary artist object, ID `2461941`. `discogs_artists` reads `name`, not `anv`. This is a demonstrated representation gap, not proof of a regression against an existing ANV-authority contract. Using that variant for native eligibility needs a separately approved, candidate-scoped rule. It cannot certify missing duration or version evidence.

This investigation changes no production code, guards, budgets, certificates, or frozen expectations. It runs no new native evaluation, provider request, model call, or original-media operation. It inspects six existing evaluations, not six new ones. Overall safety remains **non-clean**.

## Actual requests explain the profile difference

The case is `036e0a00e45b`, eleven captured physical tracks. All three profiles use the same album hint `勵志論`. All six preflight requests have an empty alias list. Tag-bearing profiles have tagged artist `盧廣仲`, folder artist `卢广仲`, and year `2021`. Filename-only has artist `卢广仲`, no tagged artist or year, and no provider IDs. The no-ID profile has captured tags but no provider IDs. Recovery retains MusicBrainz artist `2ccc975f-ea40-4f0e-b861-8d80f967fe81`, release `7935e252-463d-4207-bce9-594d0a84e63a`, and captured per-track IDs. Recovery has **neither Discogs artist nor release ID**.

| Profile | Identity lookup flags, MB / Discogs | Aliases at genre guard, cold and warm | Existing wire requests, cold + new warm | `37231524` native eligibility |
| --- | --- | --- | ---: | --- |
| `folder_filename` | true / true | `卢广仲`, `クラウド・ルー`, `Crowd Lu`, `盧廣仲` | 16 + 0 | Eligible |
| `assisted_without_ids` | true / true | `卢广仲`, `クラウド・ルー`, `Crowd Lu` | 16 + 0 | Eligible |
| `tagged_recovery` | false / true | Empty | 15 + 5 | Artist conflict |

The 52 requests are the existing three-job subset of the rehearsal's 574 requests. Warm inventories are cumulative. The extraction checks the cold prefix before subtracting it. All inventories have zero model exchanges and zero fixture misses. All jobs explicitly disable assistance, irrespective of the profile name.

Actual **selected genre receipts** need a distinction missing from the earlier shorthand. Filename-only uses `37231524`. No-ID tags use `37230261`, titled `勵志論 (首批限量精裝版)`. `37231524` is eligible there but is not the selected genre donor. Recovery has no genre receipt and eleven missing genres in each phase. The preceding rehearsal report's broader wording is not edited inside its seal; this report records the correction separately. Its four source findings remain valid.

The recovery genre trace has seven genre-bearing Discogs candidates. Three fail album identity. Four, `25560652`, `37230261`, `37230264`, and `37231524`, fail artist identity. No genre-bearing candidate passes. Failing artist identity prevents later mapping and version checks from running for those candidates. It does not establish that those later predicates would pass. MusicBrainz candidates have no genre.

## Where the identity evidence stops

1. `build_lookup_request` in `src-tauri/src/commands/auto_tag.rs:545–647` takes original captured hints and IDs and initializes aliases empty. It does not borrow another profile's evidence or previous readback.
2. After direct release lookup, `auto_tag.rs:4829–4877` independently computes each provider's missing-artist-ID flag. Recovery invokes resolution with MB disabled and Discogs enabled. Having a MusicBrainz ID is not the same as having MusicBrainz aliases.
3. `resolve_artist_identity_with_clients_result` in `src-tauri/src/state/providers.rs:1512–1577` only searches MB when its flag is true. It only returns aliases from that MB artist. The other profiles receive genuine MB aliases and can then search Discogs for the ASCII alias `Crowd Lu`.
4. Recovery's Discogs `artist=盧廣仲` search is empty. Its `q=盧廣仲` search returns `Crowd Lu` ID `2461941` and `Shen Yu Su` ID `5341590`. `search_artist_exact_result`, `providers.rs:3003–3044`, requires an exact normalized title match. It correctly does not accept the first plausible result as Chinese-name identity. The search results do not carry the release ANV.
5. The direct MB release's `artist-credit` names `盧廣仲` and the known MB ID, but contains no aliases. Recovery never requests MB `/artist/`, nor searches Discogs for `Crowd Lu`. An available capture used by another profile is not available alias evidence for this request.
6. Recovery does fetch Discogs `37231524`, including `artists[0].anv = 盧廣仲`. `parse_discogs_release` and `discogs_artists`, `providers.rs:2100–2269`, retain canonical names and the first artist ID but do not ingest ANV. The parsed donor contains `Crowd Lu` and ID `2461941`, not a Chinese alias. The existing `ProviderAlbum` does not supply such an identity receipt to the guard.
7. `fill_request_artist_identity`, `auto_tag.rs:1253–1280`, appends returned aliases without dropping them. `fill_candidate_artist_identity` fills missing IDs only. `strict_provider_evaluation`, `auto_tag.rs:2869–2950`, accepts the local artist or explicit request aliases. It does not use equal provider IDs or a fetched release as a name-conflict bypass. Chinese simplification in `exact_artist_identity`, line 2566, does not turn a Chinese name into `Crowd Lu`.

Thus the immediate recovery rejection is verified against the actual failing request. The missing MB search explains the empty accepted alias set. ANV is separately fetched but unrepresented. A generic transliteration failure, dropped returned alias, missing donor response, and exhausted discovery budget do not explain this particular first rejection.

The earlier `donor-eligibility-genre-gaps-20261109.md` says recovery has no artist-discovery HTTP lookup. That statement is too broad for these actual traces. Recovery has Discogs artist queries but no MB artist query. The earlier record and its seal remain unchanged. This clarification does not establish that a fetched, accepted alias was dropped.

## Cache and alias lifetime

`ProviderState` caches successful complete requested-provider identity resolutions for 24 hours under normalized artist plus both provider flags. Failures and incomplete resolutions are not cached. Its key distinguishes `mb=true,discogs=true` from recovery's `mb=false,discogs=true`. The recovery Discogs search succeeds as HTTP but resolves no exact ID, so the requested identity is incomplete. It runs again in warm, as the actual two repeated artist queries confirm.

The evaluator creates one provider state, SQLite cache, alias path, and worker root per job, then runs cold and warm in that process. Other profiles' MB aliases remain available in their positive in-memory identity results across phases. Their warm traces retain aliases without new artist HTTP. SQLite candidate and detail caches separately retain provider albums. Recovery's warm release candidate remains available without new Discogs detail HTTP, but caching a canonical-name album does not create a request alias.

The coordinator attempts to save ASCII aliases through `infra/aliases.rs::save_alias`, with normalized keys and case-insensitive deduplication. It uses an isolated temporary alias path here. The auto-tag request path does not load that JSON as request aliases. The persisted alias file's bytes are not exported by this rehearsal, so this investigation does not claim independent persistence readback. No user alias history is inspected or imported. Warm preflight uses fresh profile-equivalent media, not rewritten cold tags.

## Identity does not certify donor fields

Raw `37231524` and actual no-ID donor `37230261` each contain eleven tracks with empty duration strings. Their final title is `英雄 Heroes`, without the captured movie-theme and `[Acoustic Ver.]` qualifiers. Most intermediate titles have bilingual additions. The complete MB representative has all eleven ordered titles and durations, including the acoustic qualifier, but no genre.

The historical certificate diagnostics for `37231524` retain `artist_identity_not_certified` and `complete_unique_title_duration_mapping_not_certified`. The `37230261` diagnostics additionally retain `album_identity_not_certified`. All eleven reviewed occurrences have zero compatible options under that unchanged complete-title/duration checker. The current certificate inventory admits only the MB representative for this case. These are metadata-content certificates, not source-pressing or acoustic truth.

Native fallback genre evaluation is different. `complete_provider_genre`, `auto_tag.rs:2263–2334`, clears edition year, country, release IDs, and per-track IDs on a cloned request. It retains artist, album, mapping, duration handling, and both version predicates. For the two no-ID profiles, the executed traces map all eleven indices for `37231524` and report both existing version checks true, despite absent remote durations and qualifier. That native outcome is not independent evidence that the missing facts are true. Recovery stops at artist identity before those later checks.

The ANV pairs a canonical and credited name inside one primary artist entity. It does not establish arbitrary transliterations, a homonym's identity, joint-artist equivalence, or authority from track guest, writer, producer, or instrument credits. It supplies no recording duration, acoustic version, genre truth, mastering, or source-edition fact. This report neither accepts a new certificate nor removes an existing reason. All historical 84 findings, four rehearsal source findings, and six rehearsal legacy credit disagreements remain visible. Evidence exports remain insufficient, with `writableDelta: []` and `acceptedCertificate: false`.

## Evidence receipts and reproduction

Fresh local evidence is `.amp/in/artifacts/crowd-lu-alias-diagnosis-20261110/`. `receipts.json` binds six original preflight requests, donor-time requests, parsed `37231524`, predicate results, and ordered exchanges to authentic capture paths and hashes. Its SHA-256 is `7b59cc4d879d4d4d7fb8af6a7bf55e6c8106c868f478524b5c4fd39080a6241d`. `field-receipts.json` adds actual selected field receipts, every genre-candidate first rejection, raw selected donors, and unchanged historical failed predicates. Its SHA-256 is `5560148c1c1e6ae1ffa207b6dc3bb1f1f5e9e21476800daab78e3ffa3cc153d5`. These generated files are explanatory extracts, not model inputs or writable certificates.

Representative raw receipts, all under `test/fixtures/tauri/auto-tag-eval/provider-snapshots/`:

| Source and selector | Capture path | SHA-256 |
| --- | --- | --- |
| MB direct release, `artist-credit` | `review-expansion-2-20261108/musicbrainz-release-7935e252-463d-4207-bce9-594d0a84e63a.json` | `7593097ea5f4e5fd98aded79818d61b3c825f93756cd7c2b14145534617620cc` |
| MB traditional-name artist search, `artists[0].aliases[*].name` | `nine-case-discovery-20261108/musicbrainz-525a100914f4df20.json` | `34bf2f9e7f896e580b5fd0eb04d3444db8ec385856850cf98344fbe7cee41d7b` |
| Discogs Chinese `artist` search, `results` | `nine-case-discovery-20261108/discogs-065cd169fc7f73df.json` | `ce2ac9d3a06c62cc666eca1691ae2837d24a23b9dd2b7dbb6bb5f3b8f427c656` |
| Discogs Chinese `q` search, `results[*].title` | `nine-case-discovery-20261108/discogs-09538f0cbd8736fc.json` | `e889a09d07d86437816c053f6a297b3f2808f2218bf0bfd374d8f9f33c5ed02a` |
| Discogs `37231524`, `artists[0]`, `tracklist[*].duration`, `tracklist[10].title` | `reviewed-22-completion-20261108/discogs-036e0c8fc2fc0165.json` | `1798a65076cb0ab25f376751bf30fbc42677702ec54b5ffe29f4139f0c939b1a` |
| Discogs `37230261`, same selectors | `reviewed-22-completion-20261108/discogs-af52a04cbc6efbe6.json` | `70c05b3165fc7f4c873c3dcd09baae4263574db5904d8ca8ec874a3c9a7bebac` |

The MB search receipt is request-available only in the no-ID jobs. The direct release and release-ANV receipts are available in recovery. The extraction preserves that distinction instead of importing aliases across profiles.

To inspect the same native decision without tagging, select the `genre` item from `runs/<profile>-036e0a00e45b/donors-<phase>-036e0a00e45b.json` beneath the sealed rehearsal. Inspect `request.artist_aliases` and the candidate whose `candidate.discogs_release_id` is `37231524`. Actual chosen field evidence is in `<phase>.jsonl`, `native.diagnostics`, where `field == genre`. Original preflight is in `evidence-<phase>-036e0a00e45b.json`, `request`. Those files are large. The extracted receipts provide the same scoped values without loading unrelated observations.

Existing commands reproduce validation without starting tagging. Give postflight a fresh absolute destination outside the sealed rehearsal:

```sh
mkdir .amp/in/artifacts/NEW-DIAGNOSIS
python3 -B .planning/plans/five-album-evaluation-20261110/prepare.py --verify
python3 -B .planning/plans/five-album-evaluation-20261110/test_packet.py
python3 -B .planning/plans/five-album-rehearsal-20261110/test_postflight.py
python3 -B .planning/plans/five-album-rehearsal-20261110/postflight.py \
  .amp/in/artifacts/five-album-offline-rehearsal-20261110-v2 \
  "$PWD/.amp/in/artifacts/NEW-DIAGNOSIS/postflight.json"
```

Fresh postflight passes all 1,960 source/input/tool locks and fifteen job bindings, with zero ordered-route differences. Ten frozen-packet tests and four postflight checker controls pass. The checker controls include model exchanges and incomplete-discovery write rejection. All 542 sealed rehearsal files, 4,267,072,323 logical bytes, pass streamed hash and length checks. All 119 archived native sources match the checkout. The frozen packet seal remains `64ad68b53ca5d97a335cada1e0eb3d4c8c86651b677a24a484ac4e30e03d6567`; the rehearsal manifest remains `d6626296d5af352975a5822a1caecfc9471215216502871686811806d111ec45`.

One extraction attempt failed after incorrectly comparing the packet seal hash with `SHA256SUMS`, a different file. No source drift occurred and it produced no receipts file. The corrected extraction verifies `seal.json` and each entry. `failed-extraction.txt` records the error. This is a diagnostic-command failure, not a tagging attempt.

No new diagnostic executable or checker is added. No new native, broad `just check`, UI, acoustic, cancellation, or acknowledgment pass is claimed. Historical broad gates remain historical. Grounded-assistance accuracy credit remains zero.

## Proposed next bounded step, not implemented

First review whether a provider's **primary-artist ANV on the fetched candidate** may establish that candidate's artist-name correspondence. The smallest source-backed proposal is to retain the paired name, ANV, artist ID, and receipt through the existing Discogs parser and candidate path. Do not add the ANV to global request aliases, persist it as a user decision, replace display credits, infer an MB-to-Discogs ID link, or apply it to other candidates. Do not add MB requests or expand budgets to obtain aliases this profile never fetched.

If that behavior is approved, start with a failing public-provider-boundary test using the authentic Discogs body. Show that the parsed result currently loses the ANV correspondence. Then demonstrate source-scoped retention without granting donor authority. Controls must reject an absent or mismatched ANV, a different artist entity, a homonym search hit, a guest-only or joint-credit shortcut, and leakage to another candidate or later request. Existing wrong-album, ambiguous mapping, duration-conflict, explicit acoustic/version-conflict, numbering, collaborator, local-decision, selected-ID, and content-only-ID contracts must remain unchanged.

Only after reviewing that result should a separately authorized strict-loopback replay measure any eligibility change. A green identity test must not be defined as eleven authorized genre writes: independent missing-duration, bilingual-title, acoustic-qualifier, and edition-label findings still apply. The exporter must remain non-authorizing, and unchanged certification must still reject these donors. No performer-certificate integration is proposed. There is no demonstrated generic loss of an already accepted alias to fix silently today.

All work remains local and uncommitted on `artist-library-inventory` at `e9edf1c0086cc60ffffcea6e5970d4b90ae27327`. Tracked and staged diffs remain empty. Incoming rehearsal tools and unrelated untracked archive/cache files are untouched. Nothing was pushed. No action is needed to retain this diagnosis; production changes or another evaluation require separate approval.
