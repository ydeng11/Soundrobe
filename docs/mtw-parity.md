# Music Tag Web V2 parity record

This is the durable measurement record for the browser workspace's visual parity with
Music Tag Web V2, and the authority for every deliberate divergence from it. Source
comments cite it by section number, so the numbering is stable.

The raw recon (screenshots, per-view DOM inventories, reference shots) stays as a working
artifact under `.planning/mtw-parity/`, which is gitignored. Only this record is versioned.

# Music Tag Web V2 → Soundrobe web parity matrix

Phase 0 deliverable, scoped to **tagging only** per owner decision
(2026-09-18). Page specs live in `reference-inventory/`; captures in
`reference-shots/`; baseline provenance in `README.md`.

## 0. Scope decision

Tagging-centric. Explicitly **out of scope**, not deferred:

- Streaming and playback: the whole `音乐收藏` library surface (Albums,
  Artists, Tracks, Genres, Favorites, Directory), the persistent player bar,
  now-playing, playback telemetry.
- The Home dashboard (stats, monthly-trend chart, ranking list).
- System settings.
- Cloud-drive/WebDAV, notifications, and any audio-streaming endpoint.

This removes the library-index/streaming infrastructure requirement. It also
removes six of twelve reference pages and their captures from the deliverable.

**In scope:** the tagging workspace, the track detail editor, the file table,
field-group management, tag-source (metadata provider) configuration, operation
history for tagging runs, and the plugin/tag-source management surface.

## 1. Verification legend

| Mark | Meaning |
| --- | --- |
| **DOM** | Read from computed styles or element geometry on the local instance. Exact. |
| **VISION** | Read from a screenshot by a vision model. Approximate. |
| **GAP** | Not observable yet. |

Cross-checks: vision reported table row height ~53px and header ~41px on the
operation-history table; the DOM reports exactly `rowH: 53`, `headH: 41`. Vision
density and structure claims are reliable. Vision colour hexes are
approximations of `oklch()` tokens and are superseded by DOM values below.

## 2. Reference runtime (DOM)

- Public demo and the local instance serve the **same V2 bundle**
  (`static/dist/js/index.635637ec.js`), so the local instance is a faithful
  reference. It runs against a synthetic library built for this audit, which is
  what makes the populated editor observable.
- Vue SPA + Tailwind + **DaisyUI** (`drawer`, `drawer-side`, `table table-sm
  table-pin-rows`, `input input-bordered input-sm`, `bg-base-200`, `bg-neutral`,
  `rounded-xl`).
- **Colours are `oklch()`**, not hex.
- The app is a **fixed shell**: `document.scrollHeight` is 904 in a 900px
  viewport. Panes scroll independently, so page-level full-page screenshots
  capture almost nothing extra. Reference panes:
  - `.drawer-side` — left column holding the folder tree and the detail editor,
    `clientHeight 900`, `scrollHeight 2727`.
  - `.flex-1.overflow-y-auto.p-6` — main pane, `clientHeight 733`,
    `scrollHeight 2329`.
  - `.max-h-60.overflow-y-auto` — field-group list, `clientHeight 240`,
    `scrollHeight 1696`.
- Scripted `scrollTop` on `.drawer-side` is reverted to 0, and the wrapper's
  `scroll <selector>` reports success without movement. Capture below-fold
  content by **enlarging the viewport** (see `16-editor-fullheight-1440x2900.png`)
  rather than by scrolling.

## 3. Design tokens

| Role | Value | Source |
| --- | --- | --- |
| Surface / header | `oklch(1 0 0)` white | DOM |
| Page background | `oklch(0.984152 0.003413 247.858)` | DOM |
| Primary accent | `oklch(0.708969 0.159168 293.541)` lavender | DOM |
| Secondary accent | `oklch(0.833033 0.132193 321.434)` pink | DOM |
| Active rail / neutral | `oklch(0.454793 0.03391 305.475)` | DOM |
| Text primary | `oklch(0.278078 0.029596 256.848)` | DOM |
| Text muted | text primary at 40% alpha | DOM |
| Border | `~#E3E7ED` | VISION |
| Success / warning / danger | `~#00B879` / `~#E5A500` / `~#FF4D5F` | VISION |
| Radius | 999px pills, 24/20/16/12px panels, 8px controls | DOM |
| Type | 16px base, 14px body, 13.12/12.48/11.52px small, 36px, 72px hero; weights to 850 | DOM |

## 4. Workspace layout (DOM + VISION)

- Floating rounded header `x≈8–1429, y≈9–73`, height ~64px, white.
- Left column `x≈16–368` (width ~352): folder tree, then the detail editor form
  (~2647px tall).
- Main pane from `x≈420`: the file table
  (`table table-sm table-pin-rows`, `x=437, y=202, w=971`).
- Breadcrumb navigation: `Root Directory / <Artist> / <Album>`.
- Workspace toolbar: `排序` (sort), `搜索文件或文件夹` (search files/folders),
  a page-size combobox (shown `32`), and a filter select.
- Collapsible sidebar via a `close sidebar` checkbox.

## 5. File table — 10 columns (exact, DOM)

Element: `table table-sm table-pin-rows` at `x=437, y=202, w=971, h=251`,
font-size 14px. Header row **33px**, data row **73px**. Widths below sum to 970
against a 971px table, so there is no horizontal overflow.

| # | Column | Width | Content |
| --- | --- | ---: | --- |
| 1 | Cover | 80px | artwork thumbnail |
| 2 | File Name | 174px | exact file name |
| 3 | Title | 118px | tag |
| 4 | Artists | 92px | tag |
| 5 | Albums | 148px | tag |
| 6 | Album Artist | 100px | tag |
| 7 | Lyrics | 62px | `No` or value |
| 8 | LRC | 62px | `No` or value |
| 9 | Year | 56px | tag |
| 10 | Genres | 78px | tag |

Multi-select is **not** checkbox-driven. Each body row contains exactly 10
cells, matching the 10 columns, so no checkbox column exists; clicking the
header checkbox left every box unchecked. Selecting a track is done by
clicking its file-name cell. The reference's multi-select gesture remains
unidentified (see section 10).

### Vision cross-check on this table

Three independent vision passes agreed with the DOM: **10/10 columns in the
correct order**, row height 73/73/74px against an exact 73px, and all three
independently volunteered that no select-all and no per-row checkboxes are
visible. Vision header height was 31px and 32px on two passes (correct) and
56px on the third (wrong) — the DOM value of 33px is authoritative. Vision
column widths were mostly within a few pixels but drifted on some columns
(Albums 125 vs 148, Album Artist 120 vs 100, Year 65 vs 56), so use the DOM
widths above for pixel parity and treat vision widths as structural only.

## 6. Detail editor (DOM, authoritative)

Sections in visual order, with the control type and the observed value for the
audited track. Buttons and labels are quoted as shown.

| Section | Field | Type | Value |
| --- | --- | --- | --- |
| Basic Information / Core Fields | Title | text | `Opening Track` |
| | File Name | text | `01 - Opening Track.flac` |
| | Artists | chip input | `Artist Alpha` (chip; input itself empty) |
| | Albums | text | `Reference Album One` |
| | Album Artist | text | `Artist Alpha` |
| | Year | text | `2019` |
| | Date | text | `2019` |
| Media Information / Media Files | cover + media area | — | artwork present |
| Lyric Information / Content Editing | lyrics | textarea | empty |
| | LRC | textarea | empty |
| Category Information / Category Tags | Genres | text | `Ambient` |
| | Language | text | empty |
| | Album Type | text | empty |
| Metadata Information / Additional Info | 专辑版本 (Album Version) | text | empty |
| | summary | textarea | empty |
| | description | textarea | empty |
| Technical Information / File Attributes | Disc Number | text | `1` |
| | Track No. | text | `1` |
| | Total Tracks | text | empty |
| | Total Discs | text | empty |
| | Duration / Bitrate / Sample Rate / Bit Depth / Channel Count / File Size | read-only | `12`, `95`, `44`, `16`, `1`, `0.15 MB` |

The lyrics pane also exposes `Lyrics Timeline Editor`, `Save Lyrics`
(disabled until edited), `Clear`, `Distribute Time Evenly` (disabled),
`Reset timeline`, and `-5s` / `+5s` (disabled).

Note: `专辑版本` is an untranslated Chinese label inside an otherwise English
UI. This is an i18n gap in the reference, not a Soundrobe requirement.

## 7. Field-group management (DOM)

An advanced-settings panel in the same left column lists field groups with
field counts, each renameable via an inline `Group Name` input:

| Group | Field count |
| --- | --- |
| Basic Information | 6 |
| Category Information | 4 |
| Lyric Information | 2 |
| Metadata | 5 |
| Media Files | 2 |
| Technical Parameters | 11 |
| Audio Gain | 4 |

The panel also exposes `Tag Source` / `Basic Configuration` (metadata source
select, observed `网易云音乐` NetEase Cloud Music) and a `Search fields...`
filter. Soundrobe already has `WebWorkspaceSettingsDrawer.tsx`, which is the
natural counterpart.

## 8. Other in-scope surfaces

| Surface | Reference evidence | Notes |
| --- | --- | --- |
| Operation history `/log` | VISION + DOM | `table log-table`, 8 columns: ID 66px, 用户 74, 操作 109, 等级 96, 状态 73, 详情摘要 414, 时间 192, 操作 79. Row 53px, header 41px, 223 records, 50/page. Status/success/warning badge colours observed. |
| Plugin management `/mod` | VISION | `标签源插件` (tag-source plugins): 20 installed, 20 running, category split `标签源 / 操作台`, 5-column card grid ~266px wide, ~12px gaps. |
| Login `/login` | VISION | Soundrobe already has `WebLoginScreen.tsx`. |

## 9. Capability gap

Soundrobe's browser tagging surface already has: `FileGrid` (shared web
columns), `MetadataEditor`, `BatchEditor`, `ExtraTagsEditor`,
`BatchExtraTagsEditor`, `WebWorkspaceSettingsDrawer`, `SearchDialog`,
`AutoTagReview`, `AutoTagSummaryDialog`, `ConfirmWriteDialog`,
`WebTaggingDebugPortal`, plus `web-adapter.ts` over `DesktopAPI`.

Against the reference, the notable gaps are:

1. **Editor field coverage** — no `File Name`, `Language`, `Album Type`, or
   Album Version field; no read-only technical panel (Duration, Bitrate,
   Sample Rate, Bit Depth, Channel Count, File Size); summary and description
   are not distinguished.
2. **Field-group management** — grouping by field count with renameable groups
   and per-group membership does not exist.
3. **Table columns** — no `Cover`, `LRC`, `Album Artist`, or `Lyrics` status
   column in the web table.
4. **Lyrics** — no LRC timeline editor, time distribution, or ±5s controls.
5. **Tag-source selection** — no per-run metadata-source picker of this shape.

Not required under the agreed scope: playback, library index, favourites,
playlists, genre aggregation, charts, WebDAV, notifications.

## 10. Baseline status

Captured and verified (17 files, 5.3 MB, all dimensions checked):

| Group | Files |
| --- | --- |
| Tagging in scope | `09-workspace`, `13-workspace-populated` (+full), `14-track-selected` (+full), `16-editor-fullheight` |
| Out-of-scope context, kept for reference | `00-login`, `01-home`, `02-albums`, `03-artists`, `04-tracks`, `05-genres`, `06-favourites`, `07-directory`, `08-system`, `10-operation-history`, `11-plugin-management` |

Structured specs: 15 documents in `reference-inventory/`. The twelve
out-of-scope/shell pages each carry all nine required sections (44 uncertainty
items total); the three tagging captures also carry all nine sections and were
cross-checked against the DOM as above.

Vision passes are reliable for structure, counts, and density, and unreliable
for colour and fine geometry. Every value in sections 3, 5, and 6 is DOM-read.

Still missing:

- Multi-select / batch-edit state (no checkbox path found; selection gesture
  unidentified).
- Tag-source search, candidate list, and review/preview states.
- Save/confirm dialogs and the cover upload/removal dialogs.
- Error and loading states inside the workspace.
- Viewports other than 1440×900. The 1440×2900 capture is a capture aid, not a
  parity target.
- Soundrobe-side captures to diff against.

`14-track-selected-full` is 1440×904, confirming full-page capture adds nothing
for this fixed-shell app; below-fold content requires viewport enlargement.

## 11. Soundrobe implementation status (verified 2026-09-18)

The browser runtime now mounts its own application (`src/web/`), selected in
`src/main.tsx` by `isWebRuntime()`. It shares the Rust backend, the
`DesktopAPI` contract and the state helpers, but no desktop layout.

| File | Role |
| --- | --- |
| `src/web/tokens.css` | Reference tokens: `oklch()` colours, radius and type scales, chrome and table geometry |
| `src/web/web.css` | Shell, workspace, table, tree, editor and log layout |
| `src/web/columns.ts` | The ten-column contract with reference widths |
| `src/web/WebShell.tsx` | Floating header, brand, nav, session, sign out |
| `src/web/DirectoryTree.tsx` | Folder tree with on-demand expansion |
| `src/web/TrackTable.tsx` | Ten-column file table |
| `src/web/TrackDetailEditor.tsx` | Six-section detail editor with draft + Apply |
| `src/web/OperationLogView.tsx` | Operation log over real server task state |
| `src/web/WebApp.tsx` | Auth gate and view routing |
| `src/web/WorkspaceView.tsx` | Workspace composition and data flow |

### Measured parity (real browser, 1440×900, local service)

Measured from the rendered DOM, not from intent:

| Property | Reference | Soundrobe | Result |
| --- | --- | --- | --- |
| Table width | 971px | fills the pane | deliberate divergence (see below) |
| Column total | 970px | 970px floor, scaled proportionally above it | ratios match |
| Header height | 33px | 33px | match |
| Row height | 73px | 73px | match |
| Column widths | 80/174/118/92/148/100/62/62/56/78 | identical | match |
| Shell header height | 64px | 64px | match |
| Editor sections | Basic, Media, Lyric, Category, Metadata, Technical | identical | match |
| File details fields | lists `File Name` between `Title` and `Artists` (a `<label>` outside the table) | the file name is omitted | operator request (below) |

Two earlier attempts are worth recording because they show why the numbers are
load-bearing. With `min-width: 100%` the table stretched to its pane and every
column grew (85/184/125/…). With `width: max-content` nine columns matched but
`File Name` inflated to 193px, because the longest cell widened the table and
the browser redistributed the difference. Pinning a 970px width produced exact
pixel parity, and that is what shipped first.

**The table now fills its pane instead.** This is an explicit operator request,
so the difference from the reference is deliberate rather than a parity gap.
The columns are not simply stretched: each renders as its share of the reference
widths, so the reference's proportions survive at any pane width, and hiding a
column re-splits the whole pane between the columns that remain. `min-width`
holds a floor at the sum of the visible reference widths, so a pane narrower than
its columns scrolls the wrap instead of compressing text — the earlier
`min-width: 100%` attempt had no such floor and would have squeezed columns past
their reference width.

Measured on the running service: at 1920×1080 with a 1518px pane all ten columns
scaled from 970 to 1518 exactly (Cover 80 → 125, File Name 174 → 272, Year
56 → 88) with zero gap at the right. Hiding Genres and Year left eight columns
summing to exactly 100% of the pane against an 836px floor. At 768×1024 the 734px
pane scrolled 102px while the columns held 80/174/118/92/148/100/62/62 rather
than compressing.

**The file name is likewise gone from the file details pane**, also on request.
The reference does list it: querying the running reference DOM for its detail
labels returns `Title`, `File Name`, `Artists`, `Albums`, `Album Artist`, …, with
the `File Name` entry a `<label>` outside the table (the table's own `File Name`
header is a separate element). Soundrobe omits it because the name is not
editable and the file table already shows it as its own column.

### Feature parity closed

- `directory:read` was **unreachable** in the browser service: `web-adapter`
  mapped `readDirectory` onto it, but the server rejected it as an unsupported
  command, so the workspace could not list a folder. Implemented in
  `server.rs` (`read_directory_for_web`), confined to library roots, run on the
  blocking pool, with per-file read-failure fallback so one unreadable track
  cannot blank a listing.
- LRC sidecar presence is derived from the directory listing the workspace
  already fetches, so the `LRC` column reports real sidecar presence without a
  per-track backend field.
- Artwork rendering is verified: the Cover column loads real artwork through
  `cover:data-url`, measured as three 500×500 JPEG data URLs displayed at 56×56
  with zero placeholders — the same 56×56 cover size the reference uses.

### Verified write path

A title edit made in the browser was read back with `metaflac` (an independent
tool, not Soundrobe): the new title was on disk, the mtime had advanced, and the
neighbouring track was untouched. This exercises the shared `WriteQueue` and
per-file scoping end to end through the new UI.

### Tests

- 31 new web tests in `test/components/WebTaggingParity.test.tsx`, including the
  ten-column contract, the measured widths, selection semantics, patch
  semantics and write-failure surfacing. Suite: 648 passing.
- 2 new Rust tests covering `directory:read` payload shape and root confinement.
  Server-feature library suite: 420 passing.

### Known gaps

1. **Editor fields still missing against the reference:** Language, Album Type,
   Album Version, and the read-only Bit Depth and Channel Count. These need
   `TrackPatch`/`TrackData` support in Rust plus writers per format.
2. **LRC is read-only.** Presence and sidecar detection are shown, and there is
   no lyrics-sidecar *write* path anywhere in the backend (the lyrics module
   reads `.lrc`/`.txt` sidecars and enforces precedence, but nothing writes
   one). Closing this needs a new queued, journaled, root-confined write
   command exposed through both adapters, registered in the Tauri handler list
   and the web command dispatch, plus the editor action and its tests. That is
   the next slice; it is deliberately not half-implemented here.
3. **Multi-select is a deliberate extension, not a parity gap.** The reference
   file table has no selection surface at all: measured `tbody tr` classes are
   empty, no `aria-selected` is exposed, and the table contains zero checkboxes
   — measured both before and after clicking a row. Clicking a row only rebinds
   the detail editor (verified: clicking the second row set the editor's Title
   to that track). Soundrobe keeps that single-select behaviour and adds
   Ctrl-click and Shift-click range selection because its batch editor needs
   a way to reach several tracks.
4. **The standalone `App.tsx` still contains its web branches**, which are now
   unreachable because `main.tsx` routes the browser runtime to `WebApp`.
   Removing them is the remaining refactor.
5. **Six pre-existing test failures**, caused by an incorrect merge. Not
   regressions from this work; see section 14.
6. The tag-source search and candidate states are unverified.

Resolved since the first draft of this list: tag-source configuration is now
reachable (section 11), the non-1440 viewports are now verified (section 12),
and background tagging is now scheduled, monitored and configurable rather than
an unconditional startup sweep (section 15) — together with the Workspace
settings drawer and the application settings page.

## 12. Responsive verification (measured)

Measured on the rendered browser UI at each required viewport. Captures are in
`soundrobe-shots/06..08-responsive-*.png`.

| Viewport | Body overflow | Workspace | Table | Clipped text |
| --- | --- | --- | --- | --- |
| 1440×900 | none | 352px side + main | fills the 1038px pane, zero gap, reference proportions | none |
| 1280×800 | none | 352px side + main | fills the 878px pane above its floor; scrolls below it | none |
| 768×1024 | none | single column, rows `auto 1fr`; side stacks on top | 836px floor in a 734px pane, scrolls 102px, columns hold their reference widths | none |
| 390×844 | none | single column | 836px floor in a 356px pane, scrolls | none |

At ≤900px the shell switches to icon-only navigation. Each control keeps a 20px
icon and its `aria-label`, so a destination stays reachable and named for
assistive technology even when its visible label is hidden.

Horizontal scrolling appears only inside `.sr-table-wrap`, which is the same
intentional overflow the reference has. The page body never scrolls
horizontally at any tested width, and no measured control clipped its text.

## 13. Files changed by this work

| Path | Change |
| --- | --- |
| `src/main.tsx` | Routes the browser runtime to `WebApp` |
| `src/web/*` | New independent browser UI (tokens, shell, workspace, table, tree, editor, log, tag sources) |
| `src-tauri/src/server.rs` | Implements `directory:read` with root confinement |
| `src-tauri/src/commands/mutations.rs` | Journals queued track writes made inside an active review |
| `test/components/WebTaggingParity.test.tsx` | 31 browser-parity tests |
| `test/components/WebTagSources.test.tsx` | 14 tag-source and secret-handling tests |
| `docs/CHANGELOG.md` | User-visible entries |

## 14. Pre-existing failures found, not introduced

Seven tests fail on this branch, and every one was confirmed failing on a clean
tree with this work stashed. This work introduces **no** regressions and fixes
one of them.

| Failure | Count | Disposition |
| --- | --- | --- |
| `test/components/App.test.tsx` (auto-tag review) | 6 | Pre-existing, left as found (see below) |
| `commands::album_search` `manual_release_reuses_auto_tag_genre_fill_…` | 1 | Pre-existing, unrelated to this work |
| `commands::auto_tag_review` `queued_tag_and_lyrics_changes_restore_…` | — | **Fixed** by the journaling change in section 11 |

The `App.test.tsx` cause is an incorrect merge, and it is worth recording
precisely because it is not obvious from the failure messages. Commit `45505811`
("feat: add self-hosted web service") is a two-parent merge. Its **first
parent** `7e9cc48b` carries the main-line auto-tag review wiring; its second
parent carries the web additions. HEAD has the web additions but **none** of the
auto-tag wiring:

```
HEAD src/App.tsx:  autoTagSummary 0, runAutoTagBatch 0, clearAutoTagReviews 0
45505811^1:        autoTagSummary 2, runAutoTagBatch 3, clearAutoTagReviews 1
```

The merge kept the web branch's older copy of the auto-tag region and dropped
the main-line version, together with the `Needs review (N): …` notice that told
operators which albums still needed review.

The test file is meanwhile a union of expectations from three lineages that were
never reconciled (PR #18's command-level modification history, `7398f99c`'s
review flow, and `7e9cc48b`'s release line). Satisfying it by transplanting one
historical `App.tsx` region was tried and made things worse (6 → 9 failures),
because the transplanted runner changes retry and error-message semantics that
other tests assert. Reconciling this needs a deliberate re-merge of the two
parents rather than a patch, so it is left as found rather than guessed at —
the objective requires the existing desktop auto-tag behaviour to be preserved,
and an unverifiable rewrite of that path would be the opposite.

Final measured state: web suite 662 passing / 6 failing (all pre-existing);
Rust library suite 970 passing / 1 failing (pre-existing); the server-feature
suite is green including the two new `directory:read` tests.

## 15. Background tagging: schedule, monitoring and settings (verified 2026-09-18)

### What the reference actually has

Read from the reference's own bundle (`static/dist/js/index.635637ec.js`, the
same hash the local container serves) rather than inferred from screenshots,
then confirmed against the running container.

| Reference surface | Evidence | Soundrobe equivalent |
| --- | --- | --- |
| `系统设置` → 后台刮削 | Settings submenu `system.menu.scraping`; form keys include `enabled`, `watch_subdir`, `is_copy_file`, `is_fix_encoding`, `is_tran_sim`, `is_ai_preprocess`, `is_split_cue`, `auto_mod`, `manual_mod`, `tidy_music` | Settings → Background tagging |
| 配置扫描周期 (crontab) | `scrapeSchedule {crontab, last_run_at}` + 7 presets (`*/1`, `*/3`, `*/10`, `0 * * * *`, `0 */2 * * *`, `0 0 * * *`, custom) | Same preset set and a custom expression, evaluated in-process |
| 后台任务 | Settings submenu `system.menu.tasks` | Monitoring → Running now / Recent runs |
| Monitoring (service logs) | Log source selector with `gunicorn` [Web], `worker` [Async], `beat` [Scheduler], `redis` [Cache], `xiaoai` [Device] | Monitoring → Service logs, sources `app` and `auto-tag` |
| Background scrape endpoints | `/apimt/celery_task/get_schedule`, `/update_schedule`, `/apimt/sys/background_scrape_configs`, `/apimt/batch_background_scrape` | `background:get-settings`, `background:set-settings`, `background:run-now`, `background:list-runs`, `monitoring:status`, `monitoring:releases`, `monitoring:logs` |
| Celery + Beat + Redis | Bundled log descriptions (`worker` = Celery async tasks, `beat` = scheduler) | In-process `tokio` scheduler. No broker, no Celery, no Redis: a desktop-class app has one process and no message queue. |

The reference's `redis` and `xiaoai` sources have no counterpart, and its
`gunicorn`/`worker`/`beat` split collapses into this app's two real logs, so the
source list is deliberately shorter rather than mirrored one-for-one.

### Measured on the running service

Verified in a browser against the rebuilt service, not by unit test alone:

| Check | Result |
| --- | --- |
| Cron fires | With `*/1 * * * *`, the service logged `schedule came due slot=...14:41:00.0` then started a run, at 14:41, 14:42, 14:43 — once per slot, no repeats |
| History | Six consecutive `Schedule` rows, each `Completed`, `Tagged 0, 2 need review` |
| Next run | Displayed one minute ahead of the fired slot, in local time |
| Persistence | `background-tagging.json` survived a service restart, and Monitoring reloaded the prior runs |
| Coverage | Falls back to the mounted library root when no folder is configured |
| Release states | Both synthetic releases reported `Needs review` (they carry no provider ID, so `release_is_healthy` is false — `skip unchanged` correctly did not skip them) |
| Log source | 22 lines in `auto-tagger.log`, tailing the live scheduled-run line |
| Refusal | With tagging disabled the *Tag now* control is disabled; an enabled but refused start reports the service's reason verbatim |
| Workspace columns | Hiding Genres + Year left 8 headers summing to exactly 100% of the pane, against a floor of **836px** = 970 − 78 − 56 |
| Workspace details | Turning off "Show file details" removed the whole details pane, and `localStorage` recorded `details: "false"` |
| Watched folders | With watching on and a baseline taken, an album created afterwards was logged as `watched directory gained an album` 9 seconds later on the next poll, and its run covered **one** album (`Tagged 0, 1 need review`), not the library |

### Duplicate web UI: which one is live

There are two browser compositions in the tree, and they must not be averaged:

1. `src/components/WebServiceShell.tsx` (+ `WebSettingsPage`,
   `WebWorkspaceSettingsDrawer`, `WebTaggingDebugPortal`), rendered from the web
   branch inside `src/App.tsx`. **Unreachable at runtime**, because `main.tsx`
   routes the browser runtime to `WebApp`. Its components are still unit-tested
   in isolation, so their tests pass while nothing mounts them.
2. `src/web/*` (the `WebApp` composition). **This is the live UI.**

New surfaces therefore went into `src/web/`, and the functionality that existed
only in the dead `WebWorkspaceSettingsDrawer` (details toggle, visible columns)
and `WebSettingsPage` (lyrics URL, assistant autonomy, debug mode, runtime info)
was carried over rather than lost. Removing the dead branch and its four
components remains the outstanding refactor (section 11, known gap 4).

One test encoded the older scope — `WebTaggingParity` asserted that no
`Settings` destination existed. That assertion is now inverted to require
Monitoring and Settings, because this objective puts both in scope; the
library/player exclusions it was really protecting are still asserted.

## 17. Review surfacing and the three fix-up tools (verified 2026-09-19)

Scope note: this section is **not** a parity claim. The reference's workspace has
no equivalent review surface, and its batch action pane offers a different,
larger set of operations. What follows was requested explicitly and is recorded
here so a later reader does not mistake it for a measured reference match.

The reference's batch panel (`batch-action` in `index.635637ec.js`, rendered in
`workspace-pane--actions`) offers thirteen actions — `批量编辑`, `自动刮削`,
`后台刮削`, `整理文件`, `删除文件夹`, `格式转换`, `分割音轨`, `替换文本`,
`文件索引`, `导入收藏`, `重复文件检查`, `简繁体转换`, `乱码修复` — plus a
`更多操作` grid. The bundle contains **no** numbering function: `音轨号` appears
only as an editable batch-edit field, and `自动编号`/`重新编号`/`序号`/`Renumber`
have zero occurrences. So "search / number / convert" is not reference
vocabulary; it is the desktop app's own `TitleBar` toolbar
(`src/components/TitleBar.tsx`: `Search` = "Search for releases on MusicBrainz or
Discogs", `Number` = "Auto-number tracks within the selected album" with
`ORDERING_RULE_LABELS`, `Convert`). The work is therefore **browser/desktop
parity inside Soundrobe**, with the reference consulted only for layout.

### What was measured on the running service

Library: five synthetic albums under `Music/`, plus a ten-track `Radiohead/Kid A`
staged to exercise a real provider match.

| Check | Result |
| --- | --- |
| Startup classification | `Tagged 0, 5 need review` — all five albums `needs_review` |
| Banner | "5 albums need review", all five named with the service's own paths |
| Path keys | Service reports `/private/tmp/...` (canonical); `library:list-roots` reports the same form, so the badge keys match |
| Tree badge | `Reference Album One` badged `Review`, tooltip "Background tagging left this album needing review"; no other row badged |
| Tool enablement | Disabled until an album is loaded, then all three enabled |
| Number | Rule "By existing track # (desc)" turned on-disk `track` 1,2,3 → 3,2,1 and filled `tracktotal=3`; titles and decoded audio unchanged |
| Convert | `%{track}% - %{title}%` read the file names and restored `track` to 1,2,3; `tracktotal` preserved; **no other album modified** |
| Search | Ten MusicBrainz releases returned; the 2000 pressing previewed as "10 of 10 tracks matched against 10 local files"; applying wrote the correct ten `title` values with track numbers and audio intact |
| Providers reachable | Confirmed independently: MusicBrainz returned 10 results for a known album, so "No releases matched" for the synthetic album was a genuine empty result, not a silent failure |

### Deliberate differences

| Behaviour | Reference | Soundrobe | Reason |
| --- | --- | --- | --- |
| Review queue | none | banner + tree badge from the service's classification | requested |
| Actions offered | thirteen batch actions in a drawer | Search, Number, Convert on the open album | requested; desktop parity |
| Review lifetime | n/a | a flag persists until the service classifies the album again | the browser must not assume a hand repair fixed the match problem |

Review state is read from the service's in-memory per-release classification, so
it is re-derived by the startup survey after a restart rather than persisted
separately. A failed release is shown in Monitoring but deliberately kept out of
the review list, which is a test-encoded decision: a failure is a problem to
retry, not a tagging judgement for a person.

### Tool parity against the desktop (completed 2026-09-19)

The first version of these three tools was a reduced version, and the gap was
worth recording because every item was a real capability rather than styling:

| Tool | Was missing |
| --- | --- |
| Number | The whole album was scoped from the table's *filtered* rows, so typing in the filter silently narrowed what got numbered. Partial write failures were dropped. |
| Convert | Presets were listed across all directions, the pattern was not re-validated when the direction changed, a result identical to the stored value would still be written, a rename could overwrite an existing file, and per-file failures were dropped. |
| Search | Only provider/artist/album were queried: year, country, format, catalog number and barcode were absent. No result filters, no sort, no per-release track counts, no paging, no release detail, and the apply wrote every row because the per-file mapping could not be edited. |

All of the above is now implemented against the desktop's own logic:

- Number and Convert reuse `shared/track-numbering` and `shared/convert` as the
  desktop does; the web does not reimplement either transform.
- Number covers every track in the open album regardless of the table filter,
  and reports the failures the writer returns alongside its successes.
- Convert filters presets by direction, re-derives the preset when the direction
  changes, refuses a no-change run, guards renames with `file:exists`, and
  collects per-file errors instead of abandoning the batch.
- Search carries all eight query fields, filters/sorts/pages results, loads
  counts on request with a working cancel, and its mapping step can re-point a
  file, hand-edit the values that will be written, or exclude a file — writing
  only the mapped local indices with the snake_case `TrackEdit` keys native
  apply expects.

One backend gap had to be closed to get there: `album_release_track_count` was
`#[cfg(feature = "desktop")]` and absent from both `supported_web_command` and
the command table, so the browser could not read a provider track count at all.
Its body moved to a `release_track_count_inner` used by both transports, matching
the existing `resolve_release_inner` arrangement, and
`album:release-track-count` was added to the service. A server test now asserts
the route is registered (an unknown provider must answer 400, not 404).

Verified on the running service with a ten-track album: counts loaded for all 17
provider results with no error; the mapping step showed 10 rows; re-pointing row
2 to provider track 10 wrote "Motion Picture Soundtrack" and track 10; a
hand-edited title wrote through as "Hand edited"; a file set to "Do not update"
stayed at "Untitled 4" while the other nine were written.

## 18. State layer: the reference's theme, measured and ported (Phase 1, 2026-09-19)

The reference is **daisyUI with `data-theme="light"`**. That was not previously
recorded, and it changes the character of the work: its theme variables map almost
one-to-one onto `src/web/tokens.css`, which confirms where those tokens came from,
and it means the missing state values were **ported, not designed**. Full evidence:
`.planning/plans/phase1-evidence.md`; every figure recomputes with
`node .planning/plans/contrast-check.js`.

### Ported

| Reference var | Value | New Soundrobe token |
| --- | --- | --- |
| `--b3` base-300 | `oklch(0.924169 0.00108 197.138)` = `#e5e6e6` | `--sr-surface-strong`, and `--sr-hover` |
| `--nc` neutral-content | `oklch(0.978315 0.006688 295.453)` = `#f8f7fc` | `--sr-neutral-content`, and `--sr-selected-text` |

Selection now follows the reference's `.segmented-tab.is-active` pattern: a solid
`--sr-neutral` fill with `--sr-neutral-content` text. Measured **6.93:1**, against
the previous 10% accent tint, which was 1.09:1 as a fill.

### Ours, and recorded as ours

`--sr-primary-strong` is `oklch(0.628969 0.159168 293.541)` — the primary at
L − 0.08. It is **not** ported: the reference defines no `--pf` (primary-focus), so
there is no reference hover step for a solid primary button to copy. The previous
`filter: brightness(1.05)` was likewise an invention with no reference counterpart.
White on the base primary measures 2.72:1, which is below AA and is inherited from
the reference's own `--p`/`--pc` pair; the hover step raises it to 3.72:1 rather
than pretending to fix the base palette.

### Deliberate divergence

- `.sr-nav__item.is-active` and the active folder-tree row now use the solid neutral
  selection treatment. The reference's *admin* navigation instead keeps an accent
  tint for the active item, and its `.monitor-navigation-admin .nav-item:hover` is
  `rgb(240, 241, 245)` rather than `--b3`. The reference is not internally uniform
  here; `--b3` was chosen because it is the theme's declared strong surface and is
  already adjacent to `--sr-border`. Both values are recorded in the evidence file.

### Pre-existing parity defects found, NOT fixed

| Token | Soundrobe | Reference | Note |
| --- | --- | --- | --- |
| `--sr-secondary` | `oklch(0.833033 0.132193 321.434)` `#f0abfc` | `--s` `oklch(0.894268 0.054882 293.283)` `#ddd6fe` | hue 321.4 vs 293.3, chroma 0.132 vs 0.055 — a different colour, not an approximation |
| `--sr-warning` | `#e5a500` | `--wa` `oklch(0.8471 0.199 83.87)` `#ffbe00` | linear delta 0.355 — the largest of the three |
| `--sr-success` | `#00b879` | `--su` `oklch(0.648 0.15 160)` `#00a96e` | linear delta 0.116 |
| `--sr-danger` | `#ff4d5f` | `--er` `oklch(0.7176 0.221 22.18)` `#ff5861` | linear delta 0.029 — close |

`--sr-border` `#e3e7ed` sits beside base-300 `#e5e6e6`, so those two are
near-duplicates. Correcting any of these edits an existing token value and is
therefore a separate, deliberate slice.

### Verified on the running service

| Check | Result |
| --- | --- |
| Hovered non-active nav item background | `oklch(0.924169 0.00108 197.138)` — exactly `--sr-hover` |
| Hover collides with the shell background? | No (was 1.00:1) |
| Selected row (real click) | `aria-selected="true"`, fill `--sr-neutral`, text `--sr-neutral-content` |
| Disabled buttons | `opacity: 0.5` = `--sr-disabled-opacity`, `cursor: not-allowed` |
| Focusable controls with the token ring | 61 of 61 (was 50 of 61) |
| Narrow viewport 390×844 | no page overflow, table holds its 970px floor and scrolls |

## 19. Council decision: muted text and status colours (2026-09-19)

Both decisions were taken after a council (fallback roster, one pass). Full memo:
`.planning/plans/council-memo-2026-09-19.md`. The council's most useful output was not
its recommendation but its challenge to my brief.

### The measurement that decided it

The brief asked where the reference and accessibility conflict. The answer turned on
which values are the reference's. I sampled the reference's own de-emphasised text:

| Alpha the reference uses | Occurrences |
| --- | --- |
| 0.60 | 68 |
| 0.55 | 13 |
| 0.70 | 10 |
| **0.62** | **8** (`magic-nav__item`) |
| **0.40** | **none — it does not occur** |

So `--sr-text-muted` at 40% was **Soundrobe's own value**, never measured from the
reference, and `--sr-text-soft` at 62% **is** the reference's. Advisor A had assumed the
opposite of the first and advisor B the opposite of the second. Rejected accordingly.

### Changed

| Token | Was | Now | Why |
| --- | --- | --- | --- |
| `--sr-text-muted` | `... / 0.4` (2.33:1, 12 text rules) | `... / 0.65` (4.64:1) | 0.40 was ours and far below the floor; 0.65 is the smallest alpha clearing 4.5:1 on both the page and white |
| `--sr-text-soft` | `... / 0.62` | unchanged | Reference-measured; kept as read even though 4.24:1 is sub-AA |
| `--sr-secondary` | `oklch(0.833033 0.132193 321.434)` | `oklch(0.894268 0.054882 293.283)` | Reference `--s`; unused in `src/`, so zero-risk |
| `--sr-success` | `#00b879` | `oklch(0.648 0.15 160)` | Reference `--su` |
| `--sr-warning` | `#e5a500` | `oklch(0.8471 0.199 83.87)` | Reference `--wa` |
| `--sr-danger` | `#ff4d5f` | `oklch(0.7176 0.221 22.18)` | Reference `--er` |

The status section of `tokens.css` is no longer labelled "observed".

### Added, and NOT ported

`--sr-success-on-soft`, `--sr-warning-on-soft`, `--sr-danger-on-soft`. The reference uses
its status colours as fills, never as text on a tint, and as text they fail badly
(warning on its own tint: 1.46:1). Each darkens the same hue and chroma to the 4.5:1
floor: 4.68:1, 4.89:1, 4.76:1 measured. Badge rules now use these; the base status
colours remain for fills, borders and icons.

### Verified

`.planning/plans/audit-contrast.js` reads the **shipped** token values and checks every
rendered pair — 9 pass, 3 deliberate reference-inherited failures. Rendered on the
running service: 20 elements at alpha 0.65, 5 at the reference's 0.62.

### Still failing, by choice, and awaiting an owner decision

| Pair | Ratio | Note |
| --- | --- | --- |
| Primary button label on `--sr-primary` | **2.72:1** (hover 3.72:1) | The app's primary action. Reference-inherited (`--p`/`--pc`); not in the council brief, so no advisor ruled on it |
| `--sr-text-soft` text | 4.24:1 | The reference's own 0.62 value, kept as read |
| `--sr-<status>-soft` tints | — | Still unmeasured approximations; only their foregrounds were re-derived |

## 20. Corrections from the second council (2026-09-19)

A second council ran with the real `council-*` profiles (`council-astra` on GPT-6 Astra,
`council-sol` on GPT-5.6 Sol — both fresh context, read-only, models permitted by
`modelScope`). Both advisors fetched the reference's own stylesheet
(`/static/dist/css/index.916fa5cc.css`) and **falsified two claims recorded in §19**.
Both are corrected here, because a wrong provenance comment is worse than no comment.

### Correction 1 — 0.40 IS reference-attested

§19 claimed "0.40 never occurs in the reference". **That was false.** The reference
applies `var(--bc)/.4` six times. My original method was wrong: I sampled *rendered*
elements, so unrendered rules (drawers, menus) were invisible; and a literal-value grep
cannot work because the reference writes `var(--bc)/0.XX` through an indirection.

Authoritative list of alphas the reference applies to `--bc`: `.08 .1 .2 .72 .58 .62
.14 .12 .06 .6 .68 .56 .52 .16 .78 .54 .5 .18 .55 .09 .07 .9 .42 .3 .92 .7 .4 …`

Consequence: **`--sr-text-muted: 0.65` is a deliberate divergence**, not a correction of
an unmeasured value. It is recorded as such in `tokens.css` and in the changelog.

Note the advisors' specific evidence was itself imprecise — astra cited `.menu-title` as
a 0.40 consumer, and `.menu-title` actually uses `var(--bc)/.1`; the only literal `/0.4`
in the CSS is a gradient stop (`.to-base-200/40`). The conclusion still holds, which is
why the parent checks the claim rather than accepting the citation.

### Correction 2 — the reference DOES use status colours as text

§19 claimed the reference "uses its status colours as fills, never as text on a tint".
**That was false.** The reference ships `.text-success`, `.text-warning`, `.text-error`,
`.text-error\/70` and `.text-info`, and pairs them with tinted backgrounds in its badge
classes. The derived `--sr-<status>-text` roles are therefore still justified, but on
legibility grounds only: its own base status colours measure 1.46:1 to 2.67:1 as text on
those tints.

### Correction 3 — the audit was not exhaustive

§19 reported "9 pass, 3 fail". The pair list omitted the banners and the inline error
text. Both advisors caught it. The audit now composites translucent tints over the surface
before measuring, and covers badges, banners, the primary label and inline status text.

### Decisions

| Item | Decision | Detail |
| --- | --- | --- |
| D1 primary label | **H** (both advisors) | Change the label, not the reference palette. Label is now `--sr-text` on the reference's `--sr-primary`: **5.39:1** at rest. |
| D1 hover direction | **lighter**, per sol | astra argued for keeping the darker step with a near-black label. Measured: a dark label on the darker step is **3.94:1 — below the floor**. So the hover lightens, `--sr-primary-strong` became `--sr-primary-hover`, and the label reaches **5.82:1** hovered. Contrast rises on hover instead of falling. |
| D2 `--sr-text-soft` | **K** (both advisors) | The 0.62 token is kept as reference-measured, but all four body-sized consumers moved to `--sr-text-muted`, because 4.24:1 fails the floor. It now has no consumer in `web.css`. |
| D3 status tints | **L** (both advisors) | Ported the reference's construction: `bg-success/.1`, `bg-warning/.2`, `bg-error/.1`. The hand-sampled hexes (`#dff5ed`, `#fff0bf`, `#ffe4e7`) are gone. |

### Final measured state

`.planning/plans/audit-contrast.js` — **14 pass, 0 fail, 1 informational**, covering body,
muted, selected, primary at rest and hovered, three badge pairs, two banner pairs and
inline error text. The one informational row records the forbidden pairing
(status base colour as text on its tint: 1.49:1) so the trap stays visible.

Rendered on the running service: primary label `#1f2937` on `#a78bfa` = 5.39:1;
hovered `#1f2937` on `#ad92ff` = 5.82:1 with the fill confirmed lighter; notice banner
4.60:1.

Still reference-inherited and below AA, by choice, with no consumer able to raise it
without changing the reference palette: none remaining. The three failures recorded in
§19 are all resolved.

## 21. Divergence: a selected row uses the nav treatment, not the row treatment (2026-09-19)

**Decision (owner, 2026-09-19): keep the solid neutral fill for selected data rows and
record it as a deliberate divergence. Phase 4 is closed without running the prototype.**

The reference has two selection treatments and Soundrobe uses the one meant for compact
controls on its data rows:

| Reference context | Treatment | Note |
| --- | --- | --- |
| `.segmented-tab.is-active` | subtle `--b2` fill | |
| `.system-nav-item.active` | solid neutral `--n` / `--nc` | |
| `.magic-nav__item.is-active` | solid accent `--p` / `--pc` | white on primary = **2.72:1**, the pairing removed in §20 |
| **`.table tr.active`** ← the data-row analogue | **subtle `--b2` fill** | |

Soundrobe ships:

```css
.sr-table tbody tr.is-selected td { background: var(--sr-selected); color: var(--sr-selected-text); }
```

i.e. `#5b5266` fill with `#f8f7fc` text, **6.93:1**.

### Why the reference's row treatment was not adopted

It cannot work by colour alone here, which is why this was a real decision rather than a
copy-paste:

- selected = `--b2` → **1.00:1 against the shell background** (invisible — the exact
  defect Phase 1 removed);
- selected = `--b3` → **identical to the hover colour** (the other Phase 1 defect).

A subtle-fill row therefore needs a second, *structural* signal — a leading rail, a weight
change, an accent border — to distinguish it from both the page and the hover. That is a
design judgement, not a parity lookup, and it is what the `prototype` skill would have
been spent on.

### Why the accent fill was rejected

`.magic-nav__item.is-active` pairs `--pc` (white) with `--p` (the primary purple). Measured
in §20 at **2.72:1**, it fails the floor on the most important state in the table. It was
eliminated by measurement, not by taste.

### Consequence

A selected row is unmistakable and legible, and it shares one treatment with the active
navigation item and the active folder-tree row. It is visually heavier than the
reference's row fill, and that heaviness is the accepted cost of keeping selection
distinguishable from hover without adding a new structural affordance.

Reconsider only if the table gains a second selection mode (for example a "current" row
distinct from a multi-selected set), at which point a structural indicator becomes
necessary anyway.

## 22. Surface audit: drawers, views and the merged Monitoring page (2026-09-19)

The state/contrast/type audit was extended from the workspace to every other surface —
the Search, Number and Convert drawers, the workspace settings drawer, and the Tag
sources, Settings and Monitoring pages — by walking each one and measuring every
focusable element and every rendered text/background pair in the browser.

### Result

| Surface | Focusables | Without a token ring | Contrast failures |
| --- | --- | --- | --- |
| Workspace | 44 | 0 | 0 |
| Number drawer | 47 | 0 | 0 |
| Convert drawer | 54 | 0 | 0 |
| Search drawer | 54 | 0 | 0 |
| Workspace settings drawer | 67 | 0 | 0 |
| Monitoring | 10 | 0 | 0 |
| Tag sources | 13 | 0 | 0 |
| Settings | 19 | 0 | 0 |

### Two defects the sweep found, and the fix

1. **The accent-as-text trap, in four more places.** The brand mark, the brand badge
   (`WEB`), the status chips (`Running`, `Tagging`) and `.sr-badge--info` all painted
   `--sr-primary` on `--sr-primary-soft` — **2.49:1**, the same failing combination the
   primary button had. They now use a new derived token:

   `--sr-primary-text: oklch(0.557 0.159168 293.541)` — same hue and chroma as
   `--sr-primary`, darkened to **4.60:1** on the tint and **5.02:1** on a card. This
   mirrors the `--sr-<status>-text` roles; `--sr-primary` stays for fills and borders.

2. **The tree expander was invisible on a selected row.** `.sr-tree__twisty` declared
   its own `--sr-text-muted`, so inside `.sr-tree__node.is-active` it was dark text on
   the solid neutral fill: **1.59:1**. It now follows the node with
   `.sr-tree__node.is-active .sr-tree__twisty { color: var(--sr-selected-text) }`,
   which is the same `--nc` on `--n` pairing at 6.93:1.

Both are the same class of defect as the original hover bug: a state that resolves to
something it cannot be distinguished from.

### The consolidation

The Operation log is no longer a destination. It renders as a section of Monitoring, so
the navigation has four entries. The two surfaces were answering the same question from
different angles — Monitoring showed the schedule, the runs and the service log, while
the Operation log showed the task queue — and a reader had to know which one held the
answer they wanted.

`docs/web-service.md` describes the merged page. The nav tests now assert four
destinations and that `Operation log` is absent, and a test asserts the section renders
inside Monitoring with its table, so the merge cannot silently regress.

## 23. Monitoring and Settings: decisions and what shipped (2026-09-19)

The audit lives in `.planning/plans/monitoring-settings-ux-plan.md`. Four decisions
were the agent's to make, and are recorded here because each constrains future work:

1. **Save model: hybrid.** Instant apply for toggles and selects with a per-field
   confirmation; explicit `Save` / `Cancel` with an unsaved-changes guard for free-text
   values. Published practice draws the line there, and a single page-level save bar
   would turn an instantly reversible toggle into two steps while hiding which field
   failed.
2. **Tabs, not stacked sections.** Four peer tabs over a permanent count strip. This is
   the reference's own structure for the same material (`操作日志` / `运行日志` over a
   faceted list) and it corrects an earlier merge of ours that left six sections on one
   page.
3. **State the timezone.** Times render in the browser's zone, labelled, with the
   schedule stated to run in the service's local time. A wrong guess about which clock a
   schedule uses fails silently.
4. **Retention stays at 50 runs, and the UI says `last 50`.** A trend view needs more
   history, and raising the cap is a persistence decision about real run records — not a
   side effect of a UI pass. Revisit as its own change.

### Measured before and after

| | Before | After |
| --- | --- | --- |
| Counts | Four inert tiles | Every count with a list behind it is a button |
| Release list | Unbounded `map` | Filtered, searched, paged in 50s (`Showing 50 of 120`) |
| Row action | Path text | Opens the album in the Workspace |
| Log | Snapshot from page load | Follows live, filters by level with counts, searches |
| Run time | Absolute start only | Relative start, duration, countdown to next |
| Settings groups | 3 flat sections | 6 named groups with navigation and 15 help texts |
| Confirmation | Never cleared | Per field, expires on its own |
| Validation | Page-level banner | At the field, `aria-invalid`, focused |
| Covered folder | Accepted unchecked | Refused when outside every served root or unreadable |
| Busy state | Whole page locked | Only the field being saved |

Verified on the running service at 1440×900: the tiles report
`On · in 6h 56m · 6 Needs review`, `Means: Every day at 00:00`, a runs table whose first
row reads `just now · Manual · Completed · 23s`, the count click landing on
`Showing 6 of 6 matching`, the log reporting `showing 76 of 76 lines · live` with level
facets `All 76 / Info 26 / Debug 50`, and the drill-in loading
`Artist Alpha/Reference Album One` with three tracks.

The log level classifier was checked against the real file rather than assumed: the log
holds exactly 50 `DEBUG` and 26 `INFO` lines, and the classifier agrees line for line
with **zero** `unknown`, so the absent error facet is an honest "no errors" rather than
a hidden one.

The rendered sweep reports **0 focusables without a ring and 0 contrast failures** on
all five views. It also caught a regression from this pass: the new tabs, facets and
count tiles had no focus ring, so they were added to the ring group. De-emphasised text
in the new rules uses `--sr-text-muted` (4.64:1), not `--sr-text-soft` (4.24:1), which
the state-token contract test enforces; and every new font size is paired with a
leading, which the type-scale contract test enforces. Both tests failed on the first
attempt and were the reason those rules are correct.

## 24. The Search dialogue: audit and the one deliberate divergence (2026-09-19)

Audited against the desktop `SearchDialog` (924 lines) and the shared
`album:search-apply-candidate` write path. Two of the findings were **parity
regressions in the web port** — the desktop already had the fix:

| | Desktop | Web before | Web now |
| --- | --- | --- | --- |
| Filtering resets the results page | `setResultPage(1)` in 12 places | Never | On every narrowing change |
| Enter runs the search | `onKeyDown` on Agent/Album | Nothing | On every query field |

The page-reset gap was a real dead end, not a cosmetic one: filtering while on page 3
of a list that now held five matches sliced from index 20 and rendered nothing, and the
pager hid itself because fewer than a page remained — so nothing on screen could bring
the list back.

The rest were web-only:

- **Counts locked the dialog.** `loadCounts` ran through the shared busy flag, so a
  hundred sequential provider requests disabled every "Use release" button. Counting now
  has its own flag, and counting is the only thing it disables.
- **Two values for one filter.** `sameTrackCount` was stored separately from the Tracks
  field it stood for, so ticking the box and then editing the number left the box ticked
  while filtering by something else. It is now derived and cannot disagree.
- **A written value that was invisible.** The provider's track total was applied with no
  field to see or correct it. The mapping table gained a Total column.
- **A modal that never took focus.** `ToolDrawer` had `role="dialog"` and `aria-modal`
  but nothing focused it, so a keyboard user stayed behind the backdrop. It now focuses
  the first control, traps Tab at both ends, restores focus on close, and is named by its
  heading via `aria-labelledby`. This affects all three tool drawers.
- **A run-on result line.** Results rendered as chips in a `<ul>`, with title, artist,
  year, count, country and format concatenated into one string and no way to compare
  them. They are now rows with a fact per column, the whole row as the target, and
  arrow-key navigation.
- **An outcome below its own results**, and never announced. It now reads first with
  `role="status"`.
- **A dialog that never named the album**, now `Search releases — <album>`.

### The deliberate divergence

The web version now prefers the sort **"Tracks closest to local album"** once the
album's own file count is known; the desktop always starts on provider relevance. This
is recorded as a divergence rather than blended.

The evidence, measured on the running service against a real ten-track album:

| Result | Tracks | Files mapped |
| --- | --- | --- |
| First provider-relevance hit (2009 CD/DVD edition) | 26 | **0 of 10** |
| Ten-track edition | 10 | **10 of 10** |

The provider's own order is not the album in hand, and the default is what a reader acts
on before they know to change it. The choice is only an initial value: once the reader
picks an order, the page stops choosing for them, which is what the second test asserts.

### Verified on the running service

The drawer opens titled `Search releases — Kid A` with focus inside it, seeded to
`Kid A` / `Radiohead`, reporting `Searching for a release for Kid A (10 local files)`.
A search returned `37 releases found` and `Showing 10 of 37 matching — 37 from
musicbrainz`, with rows carrying `Year 2009 · Tracks 26 · Country US · Format CD/CD/DVD-Video`.
Filtering while on page 2 showed the match and the page-1 content rather than an empty
list. Filtering to nothing showed `No release in these results matches the filters above`
with the pager's own `Showing 0 of 0 matching`. Arrow keys moved `0 → 1 → 2 → 9 → 0`
across `ArrowDown`, `ArrowDown`, `End`, `Home`. The mapping table reported headers
`Local file · Provider track · Title · Artist · Track · Total · Disc` with `scope="col"` on
all seven.

Both parity regressions were re-checked by reintroducing them, and the two tests failed
as they should before the fixes were restored.
