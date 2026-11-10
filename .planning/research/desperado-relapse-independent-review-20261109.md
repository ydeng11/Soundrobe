# Independent Desperado and Relapse evidence review

Two separately scoped, read-only reviewers examined original metadata records and frozen provider captures before comparing native diagnostics. Desperado has strong metadata support for the short instrumental occurrence, but no acoustic or mastering proof and no new filename-only genre authorization. Relapse has explicit deluxe metadata and source-bound performer credits supporting a proposed content/credit-only certificate, not exact edition identity. No certificate is issued or changed.

This is a review, not a production change or new replay. All 84 source-eligibility findings, old certificates, expectations, and sealed runs remain unchanged. The earlier field-auditor exit 1 is retained, not freshly re-executed or converted into success.

## Review method and independence

The independent reviewer contexts are `TU-034unEpsmjtVmwxSNBesVX` for Desperado and `TU-034unEpxky1onCbmZwP1LO` for Relapse. Each received only its album question, original-input/capture locations, scope boundaries, and the diagnostic claim to inspect afterward. Neither edited files, called providers or application-model APIs, read original audio, ran native writes, or changed expectations. These are separate agent assessments, not human sign-off, acoustic tests, blind holdouts, or two competing votes on each case.

This document records their returned findings and my integration. I independently checked the consequential original tag/filename distinction, MusicBrainz deluxe disambiguation, track/recording artist-credit levels, and bonus numbering. `source-evidence.json` reproduces the raw fields without importing native matchers or reading native outputs. It binds nineteen provider captures to their actual bytes and manifest receipts.

The original metadata-only inventory is `test/fixtures/tauri/auto-tag-eval/corpus.json`, SHA-256 `bc081d05a3d661eb7ac99a41c605a9562a0f4714c604a8a08aabf05d7fd8db18`. The generated corpus is `.amp/in/eval-dummies/original/corpus.json`, SHA-256 `865873a108f0992da7b9b67f4bfe64da075c0de7b89b65245560891cefd12112`. Its `inputCorpusSha256` binds the original, and it explicitly declares `dummy_silent_flac` and `acousticGroundTruth: false`.

The inventory's Eagles provenance says, "Captured assisted metadata; independent provider content and mapping validation required before scoring." Eminem is marked diagnostic/unverified. Captured tags and IDs are observed input claims, not independent acoustic truth. The inventory records original hashes, but this review does not recompute those hashes from original music. Generated media hashes and their original-metadata bindings are verified for all 33 physical inputs.

No sidecar inventory was captured. The reviewers cannot establish whether original CUE sheets, purchase receipts, rip logs, or artwork inscriptions existed. The documented candidate folders under `~/Downloads/Music/Curated` and `~/Downloads/Music` are absent at the exact two relative paths checked. That limited directory check is not a search of all storage and does not prove original media are unavailable everywhere. No original audio is opened.

## Desperado: supported, contradicted, and unresolved

### Supported metadata conclusions

Case `9373204e076a` has eleven distinct physical paths in `Eagles/1973.04.17 - Desperado (2013 Remaster) [24bit 192kHz] [flac]`. Captured album/artist are `Desperado` and `Eagles`, year 1973, disc 1. Original ordinal positions, captured titles, measured durations, and recording-ID claims distinguish the opening track, instrumental, and closing reprise.

The original inventory selector is `.cases[] | select(.caseId == "9373204e076a")`. Array indexes below are zero-based when used in selectors. The table uses original measurements, not silent-file rounding or native readback. Discogs is source `2398669`; MusicBrainz lengths are release-track lengths from reviewed representative `e95722b4-c235-41f5-94db-941bcd24fc09`.

| Physical occurrence / captured title | Original seconds | Discogs position / duration | MB track seconds |
| --- | ---: | --- | ---: |
| 1 Doolin-Dalton | 207.059 | A1 / 3:26 | 209.000 |
| 2 Twenty-One | 129.978 | A2 / 2:11 | 130.000 |
| 3 Out of Control | 184.548 | A3 / 3:04 | 185.093 |
| 4 Tequila Sunrise | 173.057 | A4 / 2:52 | 174.133 |
| 5 Desperado | 213.713 | A5 / 3:33 | 217.000 |
| 6 Certain Kind of Fool | 181.794 | B1 / 3:02 | 182.000 |
| 7 Doolin-Dalton (instrumental) | 47.827 | B2 / 0:48 | 48.000 |
| 8 Outlaw Man | 213.432 | B3 / 3:34 | 214.893 |
| 9 Saturday Night | 200.596 | B4 / 3:20 | 201.000 |
| 10 Bitter Creek | 301.049 | B5 / 5:00 | 303.173 |
| 11 Doolin-Dalton / Desperado (reprise) | 291.033 | B6 / 4:50 | 291.000 |

The critical distinction is present before this review and before the recent native runs:

- `.tracks[6].filename` is `07. Doolin-Dalton.flac`, but `.tracks[6].title` is `Doolin-Dalton (instrumental)`.
- `.tracks[6].providerIds.musicbrainzTrackId` is recording `5474134a-f411-4493-a370-7a6329bf760a`.
- Opening recording is `f7d5bd78-a80c-452d-85cf-3482d7857c34`; closing reprise is `eba041a7-01f2-49b4-a5c3-9f11d0c6217e`.
- `.tracks[10].filename` is `11. Doolin-Dalton  Desperado (Reprise).flac`, while its captured tag preserves the slash and reprise qualifier.
- Recorded original track-7 full-file SHA is `903c41620516195e0fc21996f012e4cc78e9fda8ed7aa0dc2c68ababa2060717`. It is an inventory claim, not a newly verified original-audio digest.

Frozen MusicBrainz source-ID release `3cd3587f-8eaf-427b-bd84-cb2f21bed994` independently exposes distinct catalog occurrences A1, B2, and B6. Its B2 title explicitly includes `instrumental` and refers to the captured track-7 recording ID. Its recording length is 47.840 seconds. That corroborates the metadata association, not the sound of the local file.

### Contradicted interpretations

The previous investigation's "original unqualified local title" refers to the original **filename-only profile request**, not the captured original tag. The tag is already qualified. This report is an addendum; the sealed earlier report is not rewritten.

The filename-only profile intentionally removes tags and IDs. It therefore cannot borrow the captured instrumental title or recording ID as if they were available in its request. Two unqualified filename stems still require disambiguation. The 159.232-second difference between opening and instrumental durations corroborates distinct occurrences but does not acoustically identify either.

Nine duration-bearing Discogs sources agree on an eleven-title sequence and the short B2 instrumental. They are not nine independent acoustic observations and do not establish the 2013 hi-resolution remaster. Counterexamples include 1973 Presswell vinyl `2398669`, 1973 Monarch vinyl `5390450`, 1980 reissue `1151167`, 2014 remastered vinyl `6425118`, and 2021 Mobile Fidelity SuperVinyl `20260261`. Shared recording IDs across a 1973 vinyl source and the reviewed 2013 digital representative do not prove the local edition.

Discogs `20260261` has all track durations empty and side-relative positions across four sides. Its seventh catalog track is still the instrumental. The native `ambiguous_comparison` on local occurrences 1 and 7 is insufficient mapping evidence, not proof of different musical content. Ordering alone cannot replace the missing duration/identity evidence.

### Unresolved authorization

The earlier cold/warm predicates remain valid observations. Nine Discogs donors pass strict alignment but fail `recording_versions_compatible` at occurrence 7; `track_versions_compatible` passes. Source `20260261` fails alignment. This review does not remove either guard, reverse its arguments, strip a meaningful qualifier, or reuse rewritten output as evidence.

The instrumental interpretation is strongly supported as a metadata conclusion. Acoustic identity, mastering, the folder's 2013 remaster claim, and exact pressing remain unproven. Under the current fallback contract, these failed donors still do not authorize genre filling. They also cannot donate release IDs, edition years, format, country, mastering claims, or release-track UUIDs. The reviewed representative's `192kHz/24bit` disambiguation supports catalog relevance, not local mastering authentication.

All donors contain provider genre `Rock` and style `Country Rock`; extra styles differ. This does not authorize constructing a consensus genre, choosing extra styles arbitrarily, or bypassing the established provider-first eligibility/selection policy. Absence of a genre key in the original inventory does not prove the original files lacked genre, because the inventory generator does not extract that field.

## Relapse: supported, contradicted, and unresolved

### Supported complete content and explicit deluxe evidence

Case `8452b6f2b3ea` has 22 physical files in `Eminem/2009 Eminem - Relapse (With Bonus)`. The captured album is `Relapse (Deluxe)`, artist/album artist Eminem, year 2009, and selected Discogs ID `36441795`.

MusicBrainz `10e769d6-c110-4e1f-934e-2d12677d4fbb` has title `Relapse`, explicit `.disambiguation == "deluxe"`, date `2009-05-15`, one 22-track Digital Media medium, and barcode `00602527081113`. Thus its title lacking `(Deluxe)` is not evidence that it is a standard edition. Native normalization is unnecessary to discover this raw deluxe evidence.

Selected Discogs `36441795` has title `Relapse (Deluxe)`, year 2009, US country, and 22 AAC files. Both providers name Aftermath Entertainment. Those facts do not prove identical commercial editions, encoding ancestry, or local FLAC provenance.

The table uses original captured durations. MB distinguishes release-track length from nested recording length. The original filename order and selected/tag numbering differ for the last two files; that is not silently corrected to MB order.

| Physical filename/title | Original seconds | MB position; track / recording seconds | Selected Discogs position / duration |
| --- | ---: | --- | --- |
| 01 Dr. West (Skit) | 89.547 | 1; 89 / 89.546 | 1 / 1:30 |
| 02 3 A.M. | 319.853 | 2; 319 / 319.853 | 2 / 5:20 |
| 03 My Mom | 319.867 | 3; 319 / 319.866 | 3 / 5:20 |
| 04 Insane | 181.360 | 4; 181 / 181.360 | 4 / 3:01 |
| 05 Bagpipes From Baghdad | 283.067 | 5; 283 / 283.066 | 5 / 4:43 |
| 06 Hello | 248.200 | 6; 248 / 248.200 | 6 / 4:08 |
| 07 Tonya (Skit) | 42.213 | 7; 42 / 42.226 | 7 / 0:42 |
| 08 Same Song & Dance | 246.653 | 8; 246 / 246.640 | 8 / 4:07 |
| 09 We Made You | 269.613 | 9; 269 / 269.613 | 9 / 4:30 |
| 10 Medicine Ball | 237.333 | 10; 237 / 237.346 | 10 / 3:57 |
| 11 Paul (Skit) | 19.413 | 11; 19 / 19.400 | 11 / 0:19 |
| 12 Stay Wide Awake | 319.973 | 12; 319 / 319.973 | 12 / 5:20 |
| 13 Old Time's Sake | 278.160 | 13; 278 / 278.160 | 13 / 4:38 |
| 14 Must Be The Ganja | 242.907 | 14; 242 / 242.906 | 14 / 4:03 |
| 15 Mr. Mathers (Skit) | 42.387 | 15; 42 / 42.386 | 15 / 0:42 |
| 16 Déjà Vu | 283.520 | 16; 283 / 283.520 | 16 / 4:44 |
| 17 Beautiful | 392.827 | 17; 392 / 392.826 | 17 / 6:33 |
| 18 Crack A Bottle | 297.520 | 18; 297 / 297.520 | 18 / 4:58 |
| 19 Steve Berman (Skit) | 89.187 | 19; 89 / 89.186 | 19 / 1:29 |
| 20 Underground | 371.280 | 20; 370 / 371.453 | 20 / 6:11 |
| 21 My Darling | 320.427 | 21; 320 / 320 | 22 / missing |
| 22 Careful What You Wish For | 227.173 | 22; 227 / 227 | 21 / missing |

Captured `.tracks[20].trackNumber == 22` and `.tracks[21].trackNumber == 21` agree with selected Discogs positions, not physical filename order. A future credit supplement must not replace those decisions with donor numbering. The MB `Underground` release-track length of 370 seconds and recording length of 371.453 are distinct source fields; neither is silently equated with the local 371.280 seconds.

The older `test/fixtures/tauri/relapse-deluxe/local.json` snapshot differs from the later original corpus. Its README describes missing title/track-number tags; it has album `Relapse` and fewer captured collaborators. It corroborates historical filenames/durations, not a claim that the later input inventory lacked its observed tags. `reviewed-truth.json` records a user manual match, corroborated by captured Discogs and copied-media replay. That selection history is not new independent proof of MB/Discogs edition equivalence.

### Supported source-bound performer fields

For `Old Time's Sake`, MB `.media[0].tracks[12]` has recording `776ac3ca-51fe-4fac-b78f-82b587a34832`, length 278160 ms. Both the release-track `artist-credit` and nested `recording.artist-credit` explicitly say Eminem `feat.` Dr. Dre. Dr. Dre's artist ID is `5f6ab597-f57a-40da-be9e-adad48708203`. This is direct performer/featured-artist evidence, not an inference from the artist's producer description or a production relation. The original corpus says Eminem alone; selected Discogs has no track-13 `extraartists`. That omission is not affirmative evidence against the MB credit.

For `Crack a Bottle`, MB `.media[0].tracks[17]` and recording `6688538b-ac29-4403-ba01-c83ade5b58b6` explicitly credit Eminem, Dr. Dre, and 50 Cent at both levels. The original tag says `Eminem & 50 Cent & Dr. Dre`; selected Discogs directly lists Dr. Dre and 50 Cent as `Featuring`. All collaborators must remain. The difference in ordering/joinphrases is preserved as provenance, not treated as permission to rewrite a local presentation choice.

The reviewer concludes that a separate metadata-content/credit-only certificate is defensible for later approval. Its justification is explicit deluxe evidence, all 22 mapped original occurrences, correct bonus recordings, preserved duration discrepancies, and direct artist-credit selectors. It is not justified merely by native acceptance or a plausible guest. The old album certificate still fails and is unchanged.

### Contradicted or insufficient alternatives

- MB `c5706707-9bbd-42bc-ad1f-8f9d9450f217` also has 22 tracks and the useful Dr. Dre credit, but its last two tracks are `We Made You (single version)` at 287.080 seconds and `Crack a Bottle (single version)` at 315.600 seconds. They are not local `My Darling` and `Careful What You Wish For`. Count and guest credit do not certify complete content. Discogs `16649340` independently records that incompatible bonus structure.
- Standard Discogs `2058854` ends at track 20, lacks both bonus songs, and still explicitly credits Dr. Dre as featuring on track 13. Its producer entries are separate roles. It corroborates one recording credit but cannot become a complete 22-file donor under the current full-coverage contract.
- MB `88a856de-d5d1-4225-9612-8858b2f5353f` has 26 tracks. Local bonus songs appear at 23/24 after two single versions; the last two recordings are videos. Matching contained recordings do not establish the exact local package or authorize the whole edition.
- Discogs `38833854` is `Radio Hits 06-34 (Diciembre - A)`, a 2006 Various Artists release containing Akon featuring Eminem on `Smack That`. A guest-artist hit does not identify a Relapse recording or album.

### Unresolved or unauthorized fields

Exact edition equivalence, local acoustic recording identity, mastering, encoding ancestry, barcode, country, exact release date, and clean/explicit audio identity are not established. Keep selected Discogs `36441795`, local album display, numbering, and user decisions. Do not copy the MB release ID or its commercial attributes, or create content-only edition IDs. Missing artist credits do not authorize removing existing collaborators.

The MB detail has no genre/style evidence. A credit certificate cannot authorize genre donation. Selected Discogs directly contains `Hip Hop`, `Thug Rap`, and `Pop Rap`, but the old missing-duration certificate gaps for its two bonus tracks remain. Provider-first genre, eligible-source requirements, explicit per-file choices/cleanup, and assisted uncertainty are unchanged.

## Exact source receipts

All provider bodies resolve from `.amp/in/artifacts/reviewed-51-provider-first-20261109/providers/manifest.json`, SHA-256 `d9f63a13681cba375b29e8d304c7ce8e2cf6ae6fc692fdb4ec1b3dec7cb55521`. `source-evidence.json` retains all nineteen paths, raw selected fields, complete tracklists/media, hashes, and available capture request provenance. Important receipts are:

| Source | Repository-relative body | SHA-256 |
| --- | --- | --- |
| Desperado Discogs 2398669 | `test/fixtures/tauri/auto-tag-eval/provider-snapshots/discovery/folder-identities-2026-11-07/discogs-1eca3fc5640cce90.json` | `3d28e8416591895e91d7e645bdba00e19114f99e2cff04ee725e5afe98f7d62b` |
| Desperado MB captured source ID | `test/fixtures/tauri/auto-tag-eval/provider-snapshots/discovery/details-2026-11-07/musicbrainz-09168a7384ddcb19.json` | `54d9bc42d080591fd3833bae6f43ff86e11b455a9219dd5c05f0045740231923` |
| Desperado reviewed digital representative | `test/fixtures/tauri/auto-tag-eval/provider-snapshots/extra/e95722b4-c235-41f5-94db-941bcd24fc09.json` | `ac170110b1e79ca79ad2a5ab4488f45c488d1db4b1420b118bcd24e74e9c5295` |
| Relapse MB content/credit donor | `test/fixtures/tauri/auto-tag-eval/provider-snapshots/discovery/details-2026-11-07/musicbrainz-22be98f32a7f4174.json` | `cfa111b6bf92a5689262e13929fa8451443668b273306deaaaf017f3bdad6a62` |
| Relapse selected Discogs | `test/fixtures/tauri/relapse-deluxe/release-36441795.json` | `791a57e61770548bc7964c53fb7aa601702bf90528ea3bb2f7362c22a6f17c8f` |
| Relapse wrong-bonus MB | `test/fixtures/tauri/auto-tag-eval/provider-snapshots/discovery/details-2026-11-07/musicbrainz-0b5d8d713c375676.json` | `b59e88c6862a3060bbce8297f35bfc46d7e86ce32443dd11c9261e3178a5b4fa` |
| Relapse standard Discogs 2058854 | `test/fixtures/tauri/auto-tag-eval/provider-snapshots/discovery/details-2026-11-07/discogs-53635687a0526e2f.json` | `0c7636f3debe231af4621a6f15a9de831a345b32e61ea1e53cfb38895adfead6` |

## Reviewer qualifications and proposed general contract

There is no same-case reviewer disagreement to resolve: the two reviewers own different questions. Neither uses native output as truth. Both distinguish metadata-content support from acoustic/mastering and exact-edition proof. They qualify the previous report in two material ways:

1. Desperado's captured tag is instrumental; only its filename-only request lacks the qualifier. Available evidence and authority must be scoped to the active input profile.
2. Relapse's donor explicitly says deluxe in another raw field and has correct bonus content. The dispute is not adequately described as native annotation stripping alone. A content/credit certificate may be justified separately, but the exact album/edition certificate is not thereby passed.

The recommended contract for a later proposal separates observed, prewrite local evidence; explicitly mapped content occurrences and field receipts; and selected edition/mastering identity. A proposed content certificate must name physical paths, every repeated-title alternative, original/profile evidence, provider occurrence IDs, meaningful qualifiers, duration fields, bonus scope, exact allowed fields, excluded claims, raw selectors, and hashes. It must preserve current version guards and reject contradictory or unresolved mappings. It cannot use stripped profile information, rewritten tags, count alone, common genre, or plausible guest credits as authority.

For Relapse, a proposal can scope authorization to the evidenced track-13 featured-artist credit and preservation of the full track-18 performer set without replacing album display, numbering, edition IDs, or genre. For Desperado, this review does not propose authorizing the currently failed fallback donors. Any future occurrence-resolution design must first show how its evidence reaches the unchanged guards without circular inference or a case/profile exception. Neither proposal is implemented here.

## Reproduction, preservation, and delivery

Review artifacts are `.amp/in/artifacts/desperado-relapse-independent-review-20261109/`. `extract-evidence.py` reads only original metadata and frozen raw captures and prints reproducible JSON. `preserve.py` verifies prior input/source locks, the sealed diagnostic evidence index, incoming tracked/untracked changes, staging, and original-to-dummy bindings. `preservation-before.json` records the initial state; `preservation-after.json` records the final check. The protected union contains 3,564 files, including existing unrelated files. No application or maintained replay source changes.

From repository root, these commands regenerate read-only checks without overwriting sealed evidence:

```sh
python3 -B .amp/in/artifacts/desperado-relapse-independent-review-20261109/extract-evidence.py > /tmp/soundrobe-independent-review-evidence.json
cmp /tmp/soundrobe-independent-review-evidence.json .amp/in/artifacts/desperado-relapse-independent-review-20261109/source-evidence.json
python3 -B .amp/in/artifacts/desperado-relapse-independent-review-20261109/preserve.py --verify
```

This review executes no new native replay, safety/field-auditor pass, broad test gate, application-model evaluation, UI inspection, acoustic check, or original-media mutation. Earlier passing deterministic checks and the failed 84-finding field audit remain prior results, not fresh review tests. Book of Days remains unresolved. The same checkout and `artist-library-inventory` branch retain all uncommitted work above [d7659077](https://github.com/ydeng11/Soundrobe/commit/d7659077a5030c5ca627d99b6e967343d3597e28). No commit, push, deployment, new thread, or worktree occurs.

The next bounded step is to review a field-scoped Relapse content/credit certificate proposal and its genuine counterexamples before any implementation. Desperado remains a separate occurrence-evidence question for the filename-only workflow; no missing qualifier is silently treated as version agreement. No immediate user action is required to preserve or inspect these findings.
