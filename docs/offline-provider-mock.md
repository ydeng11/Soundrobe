# Offline MusicBrainz and Discogs fixtures

The standalone Node utility serves captured API JSON on loopback. It never
contacts upstream providers. Production Rust HTTP clients and parsers consume
the responses through the existing test endpoint injection; application settings
and provider discovery budgets are unchanged.

Import the raw, SHA-256-locked release details already present in the evaluation
candidate pools and the checked-in sanitized discovery bundle at
`test/fixtures/tauri/auto-tag-eval/provider-discovery.json`, then start the
service:

```sh
node scripts/mock-provider-service.cjs import-pools \
  test/fixtures/tauri/auto-tag-eval/candidate-pools.json \
  .planning/debug/provider-mock/manifest.json
node scripts/mock-provider-service.cjs serve \
  .planning/debug/provider-mock/manifest.json 18081
```

The importer verifies every source hash and writes 24 raw release-detail
records plus the locked discovery/search, artist-release, and representative
edition-detail records. Normalized candidate fixtures are rejected and are
never served as upstream responses.

Endpoints are `http://127.0.0.1:18081/musicbrainz/ws/2` and
`http://127.0.0.1:18081/discogs`. An omitted port selects a free port and prints
the endpoints as JSON. Stop with Ctrl-C.

## Captured search and browsing responses

Every page is an explicit fixture; query order is ignored, while query values,
pagination, and repeated parameters remain significant. Credential query fields
are ignored on incoming requests and forbidden in the manifest. To extend the
dataset, add sanitized locked records to `provider-discovery.json`; the normal
`import-pools` command merges them into the generated manifest.

```json
{
  "schemaVersion": 1,
  "records": [
    {
      "provider": "musicbrainz",
      "path": "/release",
      "query": {"artist": "ARTIST_MBID", "offset": "0", "limit": "100", "fmt": "json"},
      "bodyFile": "responses/artist-releases-page-0.json",
      "sha256": "SHA256_OF_CAPTURED_FILE"
    },
    {
      "provider": "discogs",
      "path": "/database/search",
      "query": {"artist": "Enya", "type": "release", "page": "1", "per_page": "25"},
      "bodyFile": "responses/search-page-1.json",
      "sha256": "SHA256_OF_CAPTURED_FILE"
    }
  ]
}
```

Query arrays of `[key, value]` pairs support repeated parameters. Inline `body`
JSON is supported for small synthetic fixtures. Optional `status` and
`headers` model errors and Retry-After/rate-limit headers. Missing routes return
501 `offline_fixture_missing`; they never fall through to the internet or
manufacture an empty successful result. Overlapping routes and hash mismatches
fail startup. Imported release details use the exact production query tuple
(`fmt` and `inc` for MusicBrainz, no query for Discogs); they do not simulate
arbitrary inclusion queries.

## Shared browse captures from 2026-11-07

`provider-snapshots/discovery/browse-2026-11-07/` contains 100 raw HTTP
responses: three artist lookups and 97 catalog pages. Each discovery record
locks the original response bytes and records its credential-free request URL,
capture time and HTTP status. These are provider evidence, not reviewed answers.

The MusicBrainz pages use the current production inclusion tuple,
`artist-credits+media+release-groups`, for Enya, Eminem, Doja Cat, Eagles and
Ellie Goulding. Their captured pages cover all advertised offsets. The corrected
Eminem artist lookup returns `b95ce3ff-3d05-4e87-9e01-c97b66af13d4`, not the
legacy fixture placeholder. Discogs lookups identify Enya as `9807`, not the
invalid legacy artist ID `900397`; the capture includes all 16 advertised Enya
pages and all 55 Eminem pages at 100 items per page, sorted by descending year.

Discogs advertises 5,489 Eminem items but returns 5,488 rows across those pages:
page 3 contains 99 rows. An independent request reproduced that page's exact
SHA-256. The discrepancy is retained as upstream evidence; no row or empty
terminal page was invented. Pagination coverage does not imply a unique or
atomically consistent artist inventory.

Historical body files remain unchanged so earlier replay manifests still resolve
their original hashes. The new responses replace overlapping discovery routes,
including the old unavailable placeholder for Enya's first Discogs page. Other
queries still return 501 when uncaptured. Search and release-detail gaps may
remain after browsing succeeds; neither the matcher nor the reviewed ledger is
changed by these captures.

## Release-detail captures from 2026-11-07

`provider-snapshots/discovery/details-2026-11-07/` contains 242 raw release-detail
responses: 175 from MusicBrainz and 67 from Discogs. The initial capture covers
185 missing detail routes and two detail routes previously marked as uncaptured,
all from the reviewed-case replay after the shared browse repair.

Discogs discovery stops at its first detail error, so that replay did not expose
every candidate request. Another 55 responses cover the uncaptured details in
the production-ranked cold and warm candidate windows. A temporary native probe
used the existing profile builder, provider parser, ranking and detail-window
functions against disposable dummies. It did not change discovery limits or
duplicate the ranking logic. The remaining unavailable placeholders at that
stage were searches. The search captures below supersede those placeholders.

MusicBrainz details use the exact production query,
`fmt=json&inc=recordings+artist-credits+labels+url-rels`. Discogs details have no
query parameters. Each record retains the raw body hash, HTTP status, capture
time and credential-free request URL. Historical bodies remain unchanged.
The captures do not approve candidates or change the reviewed answers, matcher,
provider budgets or source media. Uncaptured queries still return 501.

## Search captures from 2026-11-07

`provider-snapshots/discovery/searches-2026-11-07/` contains raw responses for
the exact searches missing from the reviewed-case replay after detail capture.
The initial 44 requests comprise 12 MusicBrainz artist or album searches and
32 Discogs artist, release or master searches. Nine replace explicit
`provider_evidence_not_captured` placeholders with genuine provider responses.

Successful empty structured searches expose generic Discogs fallbacks. Search
results also expose concrete release details and master-version pages. The
capture inventory includes these dependent requests within the existing
production limits. A request plan derives fallback queries and up to ten
concrete details from each raw release-search page. Offline replay supplies
the remaining observed misses. No missing page is treated as empty.

The completed capture contains 124 HTTP 200 responses: 12 MusicBrainz searches,
42 Discogs searches, 68 Discogs release details and two Discogs master-version
pages. Twelve replace old placeholders, including three reached only after
the initial capture. Historical placeholder bodies remain available for older
replay manifests.

Request keys preserve exact whitespace, punctuation, filters, page sizes and
offsets. In these historical captures, nested box-set searches use the parsed
parent folder as the artist. Capturing a genuine empty result does not establish that
the album is absent from the provider. It establishes only what that exact
query returned. Normalized aliases and guessed empty responses are not added.

Every record retains raw response bytes, SHA-256, HTTP status, capture time and
a credential-free request URL. Historical bodies remain unchanged. Search
captures do not change matcher behavior, budgets, reviewed answers or media.
An unrecorded query still returns 501, including whitespace variants.

## Captures after folder identity repair

`provider-snapshots/discovery/folder-identities-2026-11-07/` contains responses
for requests reached after correcting decorated titles, date prefixes and nested
box-set identities. The initial 64 requests comprise 47 MusicBrainz details,
seven Discogs details, two MusicBrainz searches and eight Discogs searches.
All 64 returned HTTP 200 during capture. They previously returned loopback 501.

Follow-up replay and production-ranked detail planning added 51 dependent
Discogs requests. A complete MusicBrainz response for Storms In Africa also
restores its explicit link to reviewed Discogs release `1459867`. The complete
capture has 116 HTTP 200 responses: 48
MusicBrainz details, 55 Discogs details, two MusicBrainz searches, nine Discogs
searches and two Discogs master-version pages. The imported manifest has 691
routes. Suite membership remains 378 cases and 3,506 tracks, with 730 input locks.

The refreshed Storms response uses a new body filename; its source record names
the historical body and hash it supersedes. The old body remains unchanged for
earlier replay manifests. The evaluator recognizes the existing reviewed answer
through the provider's relationship, without adding an acceptable-ID override.

The new searches use corrected identities such as Enya with A Box of Dreams,
not the enclosing box title as the artist. Raw bytes, hashes, capture times,
credential-free URLs and exact query values remain part of each record.
Historical responses and reviewed answers are unchanged. Capture coverage does
not approve an edition, and subsequent replay can expose additional requests.

Recognizing Wild Child's explicit Germany annotation reaches two additional
Discogs searches and their dependent release details and master-version page.
These add 15 genuine HTTP 200 captures: two searches, twelve release details
and one master-version page. No existing route or body is replaced. The imported
manifest now has 706 routes and the suite has 745 input locks; case membership,
reviewed answers and discovery budgets are unchanged.

## Nine-case discovery captures from 2026-11-08

`provider-snapshots/nine-case-discovery-20261108/` adds 173 raw HTTP 200 responses:
59 MusicBrainz and 114 Discogs requests. These cover the nine newly reviewed
cases' searches, artist catalog pages, release details, and master-version pages.
The original 77 missing requests exposed more dependent requests once answered.
Seven capture/replay passes reached complete coverage without changing the
native binary, request budgets, or reviewed answers. Existing records and bodies
remain unchanged. Each new record retains its exact query, credential-free URL,
capture time, HTTP status, and response hash.

The imported mock now has 889 routes, and the suite locks 931 inputs. The final
54-invocation native run records 560 HTTP 200 responses and zero missing routes.
All 173 new routes also pass direct HTTP status and raw-byte hash probes.
Successful fixture coverage does not imply successful tagging. The
[evaluation results](auto-tag-evaluation.md#provider-review-expansion-2026-11-08)
separate applied writes, reviewed title and artist checks, and edition recovery.

## Reviewed-case fixture completion from 2026-11-08

`provider-snapshots/reviewed-completion-20261108/` contains raw responses for
requests exposed by the 29-case replay. These cover Memory of Trees, Oceans,
Clouds, Japan And Winter Came, Dark Sky Island, and Relapse. Each record keeps
the exact request query, credential-free URL, capture time, HTTP status, and
SHA-256 of the response bytes.

The captured Relapse search replaces an unavailable-evidence placeholder with
a genuine HTTP 200 response. The old placeholder body remains available for
historical replay manifests. Other existing records and bodies are unchanged.
New searches can expose additional release-detail requests, so capture and
replay continue until the affected paths have no missing responses. The suite
keeps its reviewed answers, case membership, native binary, and discovery
budgets unchanged.

## Second reviewed expansion fixture completion, 2026-11-08

`provider-snapshots/reviewed-22-completion-20261108/` adds 478 raw HTTP 200
responses for exact requests reached by the 22 new reviewed cases: 58
MusicBrainz and 420 Discogs responses. Eight capture/replay rounds follow
searches into release details, edition lists and artist-catalog pages. Each
record retains its query, credential-free URL, timestamp, status and raw-byte
SHA-256. All 962 earlier discovery records and bodies remain unchanged.

The 54 initially affected case/profile pairs were rerun cold and warm; 12
fixture-complete baseline pairs were retained. Later rounds reran only pairs
with missing responses. Final accounting covers 132 evaluations with zero
missing routes and 2,352 recorded HTTP 200 requests. All 478 added routes also
pass direct HTTP status and byte-hash probes. The imported mock has 1,451 routes,
and the frozen suite locks 1,497 inputs.

The native executable, production request limits and reviewed answers remain
unchanged. Complete fixture coverage does not imply a clean tagging result:
abstentions, two title discrepancies and the missing Vince Staples credit remain.
See the [fixture-complete results](auto-tag-evaluation.md#fixture-complete-replay-2026-11-08).
Evidence is under `.amp/in/artifacts/reviewed-22-fixture-completion-20261108/`
and `.amp/in/artifacts/reviewed-22-fixtures-replay-8-20261108/`; per-round run
plans distinguish newly executed jobs from reused evidence.

## Native evaluation

Start the service, then run the existing ignored native evaluation against it:

```sh
SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL=http://127.0.0.1:18081 \
SOUNDROBE_AUTO_TAG_EVAL_ARTISTS=Enya \
SOUNDROBE_AUTO_TAG_EVAL_PROFILE=assisted_without_ids \
just eval-auto-tag-live
```

Despite the recipe name, the mock URL selects offline provider fixtures, skips
user-config and environment-config loading and credential requirements, disables environment proxies and
redirects, and records `providerMode: offline_fixtures` per invocation. A forced
loopback proxy also rejects ancillary external HTTP/HTTPS artwork attempts rather
than allowing them onto the network. The offline client does not retain idle
connections because synchronous media work can delay detection of the fixture
server closing a socket between phases. Production pooling and retry rules are
unchanged. The URL must be an HTTP origin on `127.0.0.1`. Synthetic media generation still requires
the runner's existing prerequisites and source hash/equivalence checks. The
native runner saves the final `GET /__fixtures` inventory as
`provider-inventory.json` and marks `command.log` as `incomplete` when the
service recorded any missing route; a passing test process does not hide
incomplete fixture coverage.

## Building reviewed evaluation cases

`GET /__fixtures` exports loaded payloads, source metadata and SHA-256 hashes,
plus request coverage and deduplicated missing requests. Authorization headers
and credential query values are never recorded. Use this inventory to identify
which pages/details need collection and to inspect complete provider tracklists.
Sanitize all captured bodies and source metadata before importing them.

Provider responses are **unreviewed evidence**, not expected answers. Keep
acceptable release IDs, complete track mappings, hard negatives and reviewer
decisions in the separate expectations ledger. Normalized `ProviderAlbum`
snapshots are explicitly skipped by the importer. No audio or source media is
written by the service.

## Native-supervised six-job mock replay (macOS)

`scripts/auto-tag-eval-replay/run-supervised.cjs` selects the maintained cold
evaluator through native supervision. It requires the retained generated dummy
inputs under `.amp/in/eval-dummies/` and frozen scorer-only user labels under
`.planning/plans/crowd-lu-user-acceptance-v1-20261111/`. It never reads the
original library or enables live calls. This is not fresh-clone reproducibility.

Build the server-feature test executable, retain an immutable copy and source
bindings, then pass that copy and a **new** output directory to the launcher.
The executable is the path printed by Cargo, not the desktop application:

```sh
CARGO_NET_OFFLINE=true cargo test --manifest-path src-tauri/Cargo.toml \
  --no-default-features --features server --lib mock_run_authority -- --nocapture --test-threads=1
node --test scripts/auto-tag-eval-replay/test_supervision.cjs
node scripts/auto-tag-eval-replay/run-supervised.cjs NEW_OUTPUT RETAINED_TEST_EXECUTABLE
python3 -B scripts/auto-tag-eval-replay/audit-supervised.py NEW_OUTPUT
node scripts/auto-tag-eval-replay/check-supervision.cjs NEW_FAULT_OUTPUT RETAINED_TEST_EXECUTABLE
```

Output directories must be beneath `.amp/in/artifacts/`. Preserve failed runs;
never resume or promote their staging. The six cold jobs retain 72 physical
occurrences and 216 requested field slots, including incomplete outcomes. Three
independent preparations run concurrently; execution retains the frozen order.
Source/tool changes during a run invalidate it. Mock/scripted agreement supplies
no grounded model accuracy credit and does not activate the live workflow.

The native preparation clock starts before bootstrap I/O and hashing. Preparation
is bounded by 300 seconds; the once-only execution scope by 600 seconds. Native
supervisor and broker owners exchange 250-ms heartbeats with one-second leases.
Stopping revokes admission independently of journal progress. Only the live
anchor may signal its own process group; successful containment additionally
requires actual exit, reaping and whole-group ESRCH. Failure grace is ten seconds,
without a postdeadline scoring or audit drain. Detached evaluators, Node-only
watchdogs and the old broker drain are not used in this mode.

Success authority is **native witnessed completion with timely supervisor exit**.
After every payload writer exits, the broker syncs evidence and stages a
generation-bound provisional report. Joint anchor retirement and actual group
absence precede the supervisor witness and successful exit. An independent native
receiver outside the payload group must verify that staging, read a clean complete
stream, and observe actual successful supervisor exit before the original absolute
monotonic deadline. Cancellation and acceptance have one irreversible winner.
Later recording cannot turn a staged file, missing/late witness or eventual zero
exit into success. The launcher also requires actual receiver zero exit. Lost
receiver/host/recording can leave incomplete recovery, which is explicitly accepted;
the implementation conservatively abandons recording at the original deadline.

Native tests also exercise synthetic finalization through the real broker's
twelve sessions, staging, joint retirement and receiver. These controls bypass
tagging/scoring and are marked `controlOnly`; the launcher rejects them as run
completion. They stall at sync entry, after provisional/recording rename, during
verification, after one retirement request, at retirement intent and after the
acceptance transition. Boundary markers must be observed before a negative can
pass. These are scheduling controls, not proofs of bounded kernel filesystem
calls or power-loss durability. The separate six-job run and read-only audit
remain required for tagging integration evidence.

The journal-lock fixture waits for the acknowledged owner lease and parks the
driver before preparation writes. A separate report verifies that state before
an external `BEGIN IMMEDIATE` lock is acquired. Its required `journal-entered`
marker proves Freeze dispatch reached the pre-transaction boundary under the
held writer lock, not that SQLite had already encountered contention. This is a
transaction-admission fault control, not a completed-preparation Freeze or a
workload-completion test. Readiness uses the declared preparation budget;
post-fault containment bounds remain separate and unchanged.

### Storage admission and incomplete outcomes

The receiver installs one generation before bootstrap content writes. That
generation admits at most 512 MiB of logical file growth and 1 GiB of cumulative
application/VFS write bytes. Both caps include the original terminal reserve of
2 MiB growth and 8 MiB writes, plus issuer bookkeeping. Overwrites and SQLite
journal writes consume allowance. Admitted operations retain their debits after
partial writes and failures. Shrinking, deleting, replacing, or abandoning files
never refunds tickets.

The maintained path meters Journal, cache, and worker SQLite connections; FLAC
writes through WriteQueue; staging, alias, halt, and evidence files; Node and
Python helper output; and captured FFmpeg and log output. Unsupported scoped
writers stop instead of falling back to an ordinary writer. Ordinary desktop
connections remain unchanged. These are application-level byte limits, not
physical-device I/O limits. Directory/inode allocation and flush traffic are not
application content-write bytes.

Unexpected read, stat, and deletion errors stop the scoped process even when a
caller ignores the returned error. Read-only handles also classify failed
truncate, write, flush, and sync operations. Maintained temporary-file and ownership
preflight probes use the same classified metadata boundary instead of masking
metadata failures as missing paths. Expected absence and interrupted operations
remain retryable at those facade boundaries. Writable handles use the stricter
stop-on-any-error policy. Directory creation also stops on any delegated error.
Unscoped filesystem behavior is unchanged.

A shared pinned revocation byte closes admission without waiting for ledger I/O.
The final shared-byte check is the admission point. Already-admitted I/O may
finish during native containment. Issuance that detects revocation after its
durable commit retains the debit and rejects delivery. Owner claims are finite
and never recycled within a generation. Scoped writers cannot replace the
ledger, occupy owner or launcher claim names, or recursively remove the account
root through a path alias. This does not protect against arbitrary host
truncation, replacement, or corruption of the backing file.

Immutable source bindings and initial mutable Journal state commit in one
transaction. For split bindings, native reports and the read-only recovery
exporter check the SHA256 of the exact stored JSON before reconstruction.
Missing, corrupt, mixed, or incomplete split state cannot authorize execution.
Historical inline bindings remain readable without migration. Existing
evaluation journals cannot restart execution.

After actual receiver exit, the launcher may consume its one-shot 64-KiB partition
of the original terminal grant. It cannot issue ordinary tickets or renew the
reserve. A failure receipt is diagnostic, not a group-quiescence certificate.
Missing or damaged recording remains incomplete. The read-only failure audit
retains all 216 unique slots as unavailable with unknown effects, even when the
ledger, configuration, or membership recording is missing:

```sh
python3 -B scripts/auto-tag-eval-replay/audit-supervised.py NEW_OUTPUT --incomplete
```

The storage audit validates nonrefundable grant history and the original caps.
It does not measure physical writes or establish writer coverage by itself.
A successful frozen six-job run, current-source boundary controls, and source
review supply separate evidence. Preserve cap-denied runs as denial evidence;
neither a component calibration nor an incomplete run establishes workload fit.

Unexpected anchor loss or suspension, both-owner/host loss, and uninterruptible
kernel operations remain conditional containment limits. An incomplete return
must not be read as proof of group absence. The runner is trusted mock code, not
a malicious same-user sandbox. Physical flush/power-loss proof, authenticated live
approval, credentials, billing, and privacy guarantees remain outside this mode.
Historical scheduling counterexamples and the superseded stronger standalone
durable-publication requirement remain retained as evidence.

### Bounded reuse of the existing AI-provider interface

The durable mock broker now prepares its request through the existing native
`OpenRouterClient` and the same JSON encoder used by ordinary application calls.
It does not invoke the ordinary completion loop, whose retry and schema-repair
behavior is inappropriate for this evaluation. Admission and fake-credential
lookup remain broker-owned, followed by a fresh send-admission check and one
HTTP execution. Exact response bytes are receipted before output interpretation.

The bounded request entry point exists only in test builds. It accepts only
explicit-port `http://127.0.0.1/...` fixture endpoints with the `/model` base path;
it rejects live URLs, alternate API formats, oversized requests and invalid
output bounds. The request explicitly disables reasoning, streaming and provider
fallback, and requires supported parameters. These flags are fixture-tested,
not evidence that a real hosting endpoint enforces them. Redirects, proxy use
and HTTP-library retries remain disabled by the broker's executor. Preparation
and dispatch must still agree byte-for-byte with the independent request digest.

The broker retains `scripted/genre-v1` as its deliberately fake model identity.
Offline client contracts also exercise a DeepSeek identifier against a local
HTTP fixture, preserving the identifier, messages and schema. This neither
queries DeepSeek nor establishes its response quality or hosted identity. No
private development configuration is read or changed. Ordinary application
provider behavior is unchanged; this does not create a live evaluation mode or
provision real approval, credentials, billing controls or retention guarantees.

### File-only real-model preflight

`preflight-model.cjs` reads an explicitly supplied dotenv file and two public
catalog snapshots. It never sources shell code, changes `process.env`, fetches
metadata, sends inference, exports credentials or creates approval. It requires
Node 20.12 or newer for the standard `util.parseEnv` parser. The file-only view
uses `LLM_MODEL`, `LLM_PROVIDER`, `LLM_BASE_URL` and credential presence. It does
not read private YAML or establish the effective application configuration after
other sources. Only OpenRouter's HTTPS base is supported in this preparation.

```sh
node scripts/auto-tag-eval-replay/preflight-model.cjs \
  .env.local MODEL_SNAPSHOT.json TARGET_ENDPOINTS_SNAPSHOT.json
node --test scripts/auto-tag-eval-replay/test_model_preflight.cjs
```

The first snapshot is OpenRouter's public `GET /api/v1/model/{author}/{slug}`
response for the exact configured identifier. The second is public
`GET /api/v1/models/{author}/{slug}/endpoints` for its observed alias target, or
the model itself when it is not an alias. These are unauthenticated catalog
reads, not connection-test inference. Obtain them separately; this command has
no network implementation. Snapshot age and authenticity remain operator
responsibilities and must be rechecked before final approval.

Preserve a leading `~` in the model name. A moving alias and its observed target
are different facts; do not substitute the target in the request. An alias's
empty endpoint list does not prove it cannot serve. The tool joins the alias
metadata to its concrete target's endpoints, rejects unsupported parameter and
unknown-price routes, and proposes the lowest declared text-token estimate among
compatible candidates. A candidate is not an approved or attested serving host.

The report contains only allowlisted settings, snapshot/configuration hashes,
fixed limits, a $5 USD allowance and explicit unfinished checks. Credentials and
the private whole-file hash are absent. Secret rotation alone does not change
the public settings fingerprint. Price arithmetic uses integer trillionths of a
dollar. Its 8,192-input/1,024-output text-token illustration is not a tokenizer
proof, provider-side billing ceiling, purchase authorization or all-in fee bound.
Unknown account terms and billing exposure still block live execution.

The explicitly selected native test entry
`infra::openrouter::tests::prepare_model_evaluation_plan` accepts paths through
`SOUNDROBE_MODEL_PREFLIGHT_REPORT` and `SOUNDROBE_MODEL_PREFLIGHT_INPUT`. The input
is the public `artist`/`album`/`tracks` object, never the answer labels. Build the
normal server test executable and run only this ignored entry with `--exact`,
`--ignored` and `--nocapture`. Use an environment containing only the required
paths and basic execution variables, not the credential file's environment.
The native entry validates the genre input, uses the app's existing prompt/schema
and shared request encoder, and emits `SOUNDROBE_MODEL_PLAN=` followed by JSON.
It adds exact `provider.only` and `provider.order` route restrictions, disables
fallback/reasoning/streaming, hashes the actual body and binds the input/report
bytes. It constructs no executable HTTP request, reads no credential and sends
nothing. These native preparation helpers remain test-build-only.

Both report and native plan always state `liveAuthority:false`,
`liveDispatchAvailable:false` and `readyForExecution:false`. Neither is signed
approval, a durable attempt reservation or a result. Real approval provisioning,
native-only real credential supply, complete driver/readback/scoring integration
and final-source supervised acceptance remain unfinished. No historical mock run
certifies those paths.

### Offline broker rehearsal of the prepared request

The test-only native broker can bind the prepared DeepSeek request to a distinct
`offline-model-rehearsal-v1` policy. Its native context pins the verifier outside
approval JSON, child IPC and the saved run manifest. Immutable run bindings
include its fingerprint, request digest, exact configured model alias, separately
observed response-model target, candidate route and expected provider label.
Neither a provider's response label nor the fingerprint attests hosted weights,
provider identity or real user trust.

The configured alias stays in the request. The independently observed concrete
target is used for response validation, not silently substituted into the request.
Input drift fails before credential lookup or wire dispatch. A signed approval
must match the exact frozen scope, expiry and nonce and can be consumed once.
Reopening a saved rehearsal without its native verifier context permits read-only
reporting but cannot authorize commands under the old fixture key. The positive
and negative contracts use deliberately public test seed `[42; 32]`; this is
separate fixture trust, not real approval custody or provisioning.

Rehearsal dispatch still permits only a registered explicit-port HTTP
`127.0.0.1` fixture at `/model/chat/completions`. Its injected supplier returns
fake credentials, additionally constrained by a `MOCK_` prefix. That prefix is a
fixture convention, not a general secret classifier. This component neither
reads `.env.local` secrets nor adds a live endpoint or activation switch. The
same durable nonrefundable reservations, lookup count, post-supplier admission,
HTTP executor and bounded response receipts are reused. No retry, repair,
redirect, proxy, model fallback or provider fallback is introduced.

Only a validated genre/confidence projection crosses IPC. Fully received error
responses retain exact digest/byte/status receipts and settle the consumed
operation without making its answer valid. Missing or excessive usage, a wrong
model/provider claim, malformed output and reported nonzero reasoning fail
closed. A truncated or oversized wire response retains an incomplete operation,
not a fabricated complete receipt. Low-confidence genre text is withheld from
IPC and general evidence.

The maintained supervised launcher accepts an optional third argument containing
the public rehearsal-plan path. It passes only that path before native ownership
and storage admission; it does not inherit parent credentials or read the plan
before bootstrap. The plan contains `model`, `observedTarget`, `route`, `provider`
and public `input` (artist, album and track titles), never a verifier or credential.
The driver locks its bytes in the signed scope. Native bootstrap independently
selects the public test verifier and constructs the contextual Journal.

All twelve worker descriptors carry the same plan. Preparations use the existing
app request encoder; the worker rejects input/model drift and records the exact
request alias in assistance provenance. The loopback fixture may report the
separately observed response target. Scoring still checks provenance against the
prepared request, retains the full requested denominator and awards no grounded
accuracy credit for the scripted answer. The maintained read-only auditor joins
the locked plan, six preparations, request/response receipts, fixture inventories,
actual worker SQLite projections, media readback and scores. These joins do not
replace the native witness and timely successful supervisor-exit requirement.

Use a new output root under `.amp/in/artifacts/` and a retained test executable:

```sh
node scripts/auto-tag-eval-replay/run-supervised.cjs NEW_OUTPUT TEST_EXECUTABLE PUBLIC_REHEARSAL_PLAN
python3 -B scripts/auto-tag-eval-replay/audit-supervised.py NEW_OUTPUT
```

The original 300/600-second scopes, 512-MiB growth/1-GiB cumulative application-write
caps and terminal reserve are unchanged. Lost recording remains incomplete;
staging cannot recover success. The real credential reader, actual approval
provisioning, billing enforcement and external transport remain separate
unfinished work. All new evaluation entry points remain absent from non-test
application builds. A successful rehearsal verifies wiring, not real-model
quality, hosted identity, physical disk traffic or power-loss durability.

## Checks

```sh
npx vitest run test/scripts/mock-provider-service.test.ts
cargo test --manifest-path src-tauri/Cargo.toml --lib offline_
```
