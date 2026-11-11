# Independent reviews and dispositions

Both reviews were read-only and separately scoped. Neither ran tests, tagging, services, model/provider requests, media operations or credential lookup. Neither edited files. These are agent source reviews, not qualified human musical/acoustic labels. HEAD during review was `6cb93861c12628aff6d87332f1721e81ba761080`.

## Raw source and label review

Reviewer `TU-034vimKaQ7GTOOFQGqRtgH` read the raw inventory and provider bodies for Crowd Lu, Amala, Billie and Paint the Sky. The brief excluded native/model outputs, scripted desired answers, planning expectations and prior label conclusions. Its first broad inventory print incidentally exposed embedded generic expectation blocks with empty mappings. The reviewer switched to metadata allowlists, did not use those blocks as evidence, and disclosed that this was not strict zero-exposure review. All selected examples are exposed development cases regardless.

| Raw evidence | Review finding | Disposition |
| --- | --- | --- |
| `artist-batch-extension/corpus.json#/cases/0/tracks/0..10`; MB `musicbrainz-release-7935e252-463d-4207-bce9-594d0a84e63a.json#/media/0` | Eleven populated title/artist rows, explicit null totals, disc zero. Catalog supports totals 11/1 and medium 1; digital release differs from captured CD. | Totals are deterministic controls, not model labels. Disc zero is a captured sentinel/correction candidate, not an absent-key/null fill or verified positive disc number. Any runtime missing-scope interpretation requires separate coordinator evidence. No catalog-number or edition authority added. |
| Same MB `/media/0/tracks/10`; Discogs `discogs-036e0c8fc2fc0165.json#/tracklist/10` | MB supports full movie-theme `[Acoustic Ver.]` title; Discogs short `英雄 Heroes` does not establish version equivalence. | Existing exact title alternative retained. No simplified-title or acoustic-authentication claim. |
| Main `corpus.json#/cases/98/tracks/8`; MB `musicbrainz-60358d77da081c4e.json#/media/0/tracks/8/artist-credit` and `/recording/artist-credit`; Discogs `discogs-c7758322f5bfad50.json#/tracklist/8/extraartists` | MB has Billie Eilish with/& Vince Staples; Discogs has Featuring. Captured artist is Billie alone. Captured selected Discogs ID differs from compatible donor. | Retain existing exact complete ordered artist alternatives; score catalog consistency separately from provider eligibility and selected-edition identity. Guest completion is not a missing-only AI fill. |
| Main `corpus.json#/cases/17/tracks/3`; MB `musicbrainz-ad6d0d2fac466bbc.json#/media/0/tracks/3/artist-credit` | Wine Pon You retains Doja Cat feat. Konshens; catalog corroborates the guest. | Historical control remains valid, but not selected to minimize sample size. |
| Main `corpus.json#/cases/313`; Discogs `discogs-f46bfbbb038a012f.json`; MB `musicbrainz-40c683534bd90266.json` | Catalog totals 16/1; CD/DG ID and EU folder agree, while MB suggests JP catalog WPCR-1900. Duration agreement does not settle physical edition. | No edition/remaster truth label. Only filename-only incomplete/version control selected. |
| All four inventories | Genre is unrepresented, not observed absent on original disk. Titles/artists populated; totals explicitly null. | Sample missingness comes from bound historical dummy `beforeTracks`, not inventory omission alone. |
| Crowd Discogs `/genres`, `/styles` | Electronic, Rock, Pop, Folk, World, & Country; Folk style. Album catalog attribution only. | No accepted true-genre values. Proposed taxonomy is explicitly unapproved and cannot serve as its own label. |

Raw source SHA-256 values independently returned by the reviewer match the older source register: Crowd MB `7593097ea5f4e5fd98aded79818d61b3c825f93756cd7c2b14145534617620cc`, Crowd Discogs `1798a65076cb0ab25f376751bf30fbc42677702ec54b5ffe29f4139f0c939b1a`, Billie MB `d02496332ab0c00399248d3d7281acac7207613aaa73dcc338b96e0127c0dcf6`, Billie Discogs `da0697025847bf46d377f36ff2de7bdf4e4569536adddbb5168a0b6139a5104f`. The checker verifies the complete original register, not just these four.

No independent genre label was offered. The reviewer required taxonomy/version, genre/style policy, album/track scope, multilabel alternatives and qualified annotation/listening. Captured MB track IDs inspected are recording MBIDs, not release-track IDs. None was revalidated against audio. These limits are accepted.

## Contract and security source review

Reviewer `TU-034viovWI5PUTKjHwC3N1J` inspected current Rust/driver/scorer source and the prior public plan. It received the prospective three jobs and two-attempt ceiling, not desired system outputs. It found that the current live contract cannot be satisfied. No service capability was tested.

| Source | Finding | Accepted disposition |
| --- | --- | --- |
| `infra/mod.rs:25–26`, `bounded_eval.rs:143–219`, `auto_tag_eval.rs:4235–4255` | Scope is cfg(test), mock-only; matching caller digest is not authenticated approval. | Keep external denial. Proposal is not a native manifest. Require separate native/trust design and review later. |
| `run-bounded.cjs:39–45,213–231`; prior docs proposal | Runtime mock selection and older four-job proposal differ from new three-job paired experiment. Driver gives each job its own allowance. | New proposal explicitly supersedes only future selection. Derive 12/144/432 membership; require whole-selection two-attempt authority, not per-job multiplication. |
| `bounded_eval.rs:245–264,488–607` | Job admission not independently one-shot; reservations/usage checks cover one Budget only. Missing usage/model must stop; no billing authentication. | Require durable consumed run plus exact job/phase admission, no unknown-usage refunds, global limits, provider-side cap evidence. |
| `openrouter.rs:332–343,489–510,587–690,1039–1059` | Ordinary transport retries/repairs and fills missing usage/model with defaults. Mock repair denial is test-only. | Proposed zero retries/repairs needs enforcement before dispatch. Defaults cannot be accounting facts. |
| `openrouter.rs:349–372,816–845` | API format is not upstream vendor. Targeted correction policy permits fallback routing; genre does not inherit that policy. | Exclude correction, pin exact final genre body and separately reviewed provider/retention policy. No existing pinning guarantee. |
| `auto_tag.rs:4223–4309,4109–4128,4332–4338` | Genre has conditional eligibility and small payload; correction includes paths. Genre errors may propagate remote text. | Conditional role only; exclude correction. Add native-only secret boundary and pending bounded sanitized errors. |
| `auto_tag.rs:1888–1968,5572–5633,6526–6639` | Provider-first does not inherently preclude genre fallback; role does not constrain whole-candidate writes. | Billie/Paint explicitly authorize no model requests. Disposable-only writes and frozen per-field transformations; no genre-only-write claim. |
| `mutations.rs:801–845` | Scope health hooks are test-only. Admitted atomic dispatch may finish; prior writes are not rolled back. | Keep no-rollback/local-cancel limitations. No production write capability added. |
| `auto_tag_eval.rs:3738–3799,4031–4045`; `auto_tag.rs:5433–5470` | Cold/warm refers to local cache; failure flag not universal for correction failure paths. | Separate caches between arms. Do not claim remote cache control or universal model-failure coverage. Correction remains excluded. |
| Scorer `score-observations.py:184–262`; driver `280–294` | Frozen membership and failure envelope are useful, but do not establish semantic labels or native decision visibility. | Keep unavailable/unknown/unscorable categories and no certificate/write authority. |

No review disagreement was resolved by vote. Catalog totals were accepted as source-backed but rejected as an AI opportunity under the actual runtime. Genre truth, provider-cap authentication, immutable model revision and real approval remain unresolved. Later narrow draft rechecks, if any, are recorded below without replacing these original findings.

## Contract draft recheck

The same security reviewer independently inspected the four new contract/report/review/documentation drafts. It found one remaining ambiguity: `approval.requiredBinding` said profile inputs/readbacks, which could imply future observed readbacks or desired outputs were prior authorization inputs. The final text now says **actual prewrite reader snapshots**. Subsequent readbacks bind only to the execution receipt. No approval is inferred from either.

The reviewer found no other concrete contradiction or unsafe activation/semantic-proof claim in that narrow document scope. It did not verify membership/hashes, historical evidence, public documentation accuracy or service capabilities. The correction is textual, not a native boundary change.

## Label draft recheck

The raw-source reviewer then read README, sample and these dispositions. It required the explicit zero-versus-null wording now retained above. It otherwise found the scoped catalog alternatives/selectors, exposure, historical dummy missingness distinction, proposed taxonomy and withheld AI credit accurately represented within that narrow review. It did not independently reopen historical prewrite receipts. This follow-up is deliberately exposed to the draft/labels, and is not execution approval or musical truth validation.
