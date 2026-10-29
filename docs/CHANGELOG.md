# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Auto-tagging now ranks locally discovered editions by total track count before fetching release details, continues discovery when pagination totals are missing, honors provider retry delays, and asks for review when a folder has no usable artist or album identity.
- Background tagging now resumes provider catalog discovery and queued album work from SQLite after a restart; provider outages or incomplete catalogs remain deferred instead of triggering guessed AI tags, cancelled work stays stopped until retried, interrupted runs are closed visibly, and AI-written metadata is durably marked for review before files change and remains flagged until acknowledged in Monitoring or Workspace.
- The browser service no longer prompts for a password or maintains a login session; access is controlled by the deployment boundary instead.
- Tagging cleanup can now clear a validated set of ID3/Navidrome-supported tags (including Comment and Description) in the same write as new metadata. Desktop and web Settings autocomplete only supported tags, explicitly label selected tags as cleared, and use an × control to remove them; Monitoring reports the tags kept and cleared when a run finishes.
- Browser Workspace's Refresh button now retries the selected library after an initial directory load failure instead of becoming a no-op.
- Browser Workspace now shows all loaded tracks in the selected artist folder when no specific album is open; selecting an album still narrows the table to that album.
- Browser Search results are rows you compare rather than one run-on line: year, track
  count, country and format each get their own place, the whole row is the target, and
  arrow keys move between rows.
- Browser Search now leads with the release closest to the album's own track count
  instead of the provider's own order. Measured: the first provider result for a
  ten-track album was a twenty-six-track edition that matched none of its ten files,
  while the ten-track edition matched all ten. Choosing an order yourself stops the page
  choosing one for you.
- Browser Search now moves through Search, Select release, and Match tracks as three
  visible steps. It shows one step at a time, keeps the album context visible, gives
  release candidates a clearer match summary, and presents each local file beside the
  remote track and values that will be written. The wide comparison drawer remains
  usable on a phone, with actions kept in view.

- The browser service's **Monitoring** page is now four tabs — Overview, Releases,
  Operations, Logs — under a strip of counts that stays visible. Every count that has
  a list behind it is a button: "Needs review 5" is how you get to those five albums.
  The Releases tab filters by state and searches by path, and each row opens the album
  in the Workspace, so a problem reported here can be fixed from here. The previous
  version stacked six sections on one page and every count was inert.
- Monitoring now shows how long each run took, how long ago it started, and how long
  until the next run, instead of only absolute timestamps. The record has always
  carried this.
- Monitoring's log viewer is live: it follows the log while you watch it, filters by
  level (error, warning, info, debug) with counts, and searches lines. It previously
  loaded once and never changed — precisely when a run in progress made you want it.
- Monitoring says which clock its times are shown in, and that the schedule runs in
  the service's local time. A schedule's timezone was previously left to be guessed.
- The browser **Settings** page is grouped into six named sections — Metadata sources,
  Tag writing, Lyrics, Automation, Assistant, System — with navigation between them and
  a description under every setting.
- Desktop Settings now highlights each editable option and puts its longer explanation in an
  accessible tooltip.
- Browser Settings now keeps its section navigation visible while scrolling, moves keyboard
  focus to the selected section, exposes the current location, and stacks fields and actions
  cleanly on narrow screens.
- Settings now explains a crontab expression in words ("Every day at 00:00"), and says
  so plainly when it cannot be sure rather than guessing at a schedule's meaning.
- Browser-workspace polling now pauses while the tab is not visible.

- The browser service's Operation log is now a section of the Monitoring page rather
  than a destination of its own. Both answer the same question — what has the
  background tagger been doing — so the navigation now has four entries instead of
  five, and everything about a tagging run is on one page.
- Browser-workspace text now carries a line height paired to its size, taken from the
  reference, whose own text sets a leading on every size. Text blocks read slightly
  looser than before; the file table's row, header and column geometry is unchanged.
- Browser-workspace status colours (success, warning, danger) now come from the
  reference's measured theme values instead of approximations read off screenshots,
  so success, warning and danger read slightly differently. The secondary accent was
  corrected the same way. Its value was wrong in a way that mattered: it was a
  different hue and chroma, not a rounding difference.
- Browser-workspace interactive states are now driven by tokens ported from the
  reference's own theme rather than by values invented per rule: hover uses the
  reference's base-300 surface, and the selected row, the active navigation item
  and the active folder-tree row share the reference's neutral fill with its
  neutral-content text, so a selected row is now unmistakable and legible rather
  than a 10% accent tint.
- The browser runtime now mounts its own application instead of reusing the standalone app's composition. It shares the Rust backend, the `DesktopAPI` contract and the state helpers, but not the desktop window chrome or layout.
- The browser Workspace now follows Music Tag Web V2's tagging layout: a floating header, a folder tree beside a track detail editor, and a ten-column file table (Cover, File Name, Title, Artists, Albums, Album Artist, Lyrics, LRC, Year, Genres) sized to the reference's measured column widths and row density.
- The browser file table now fills the width of its pane while keeping the reference's column proportions, instead of holding a fixed width. Hiding columns re-splits the pane between the columns that remain, and the table never narrows past the reference column widths, so a narrower pane scrolls rather than compressing text.
- The browser file details editor no longer lists the file name. It is not editable, and the file table already shows it as its own column.
- The browser track detail editor is grouped into Basic, Media, Lyric, Category, Metadata and Technical Information sections, and reports read-only duration, bitrate, sample rate, format and file size.
- The browser service now reports tagging operations in an Operation log with status, progress, outcome counts and cancellation.
- Browser navigation covers the tagging surfaces: Workspace, Monitoring, Operation log, Tag sources and Settings. The previous library, home and activity browser views are replaced by these tagging-focused destinations.
- Background tagging is now configurable and observable instead of an unconditional sweep on every service start. It still runs once at startup, and every run is recorded with its trigger, outcome and counts. The schedule and run history live in `~/.soundrobe/background-tagging.json`.
- Background runs now record what they did release by release: each run keeps the release path, its outcome, the reason it needs attention, and which source supplied the tags, and Monitoring shows that list per run. The ledger keeps failures and reviews and drops later successes when a run covers more releases than it stores, and says when it did.
- Monitoring can re-run the releases a background run left needing action. Only those releases are tagged; releases the run already tagged, and paths that no longer exist, are left alone. A retry with no run named retries the most recent run that finished, so asking while another run is in progress reports the run that is actually happening instead of "nothing left to retry".

### Fixed

- Desktop library loading, auto-tag results, and review actions now work in the native app instead of being unavailable after the tagging migration.
- Browser Settings section links no longer trap the page at the selected section. The main pane remains scrollable, and the sticky navigation can focus another section.
- Background tagging no longer loses music added while a run is in progress. A watched album that could not start because another tagging operation held the lock is queued and retried on the next poll, instead of being remembered as seen and never tagged until its folder changed again.
- Field hints in the browser workspace were laid out in the label gutter instead of
  under the control they describe, so "Required, unless an album is given" was squeezed
  into a 104-pixel column and wrapped to three lines in Search, and the Chinese-script
  note to seven lines in Settings, while the space beside them sat empty. Hints now sit
  under their control at full width, one step smaller and quieter than the value.
- Setting names on the browser Settings page were cut off: nine of twelve had to fit a
  104-pixel label gutter, and "Let the assistant apply its own actions without
  confirmation" read as "Let the assistant apply its own ac…". Labels now wrap, in a
  gutter wide enough for a real field name.
- A checkbox setting's name sat inside the label gutter alongside its box, cramping both
  into one narrow column. Those fields now use the whole row.
- A field's hint sat the same distance below its control as fields sit from each other,
  so it read as belonging to the field below it.

- Browser Search lost your place when you filtered the results: narrowing while on a
  later page showed an empty list with the pager hidden and no way back. It now returns
  to the first page, which is what the desktop app already did.
- Pressing Enter in a Browser Search field did nothing. It now runs the search, as the
  desktop app does.
- "Same track count as this album" could claim a filter it was not applying: ticking the
  box and then editing the Tracks field left it ticked while filtering by a different
  number. The box now reflects the value it stands for.
- Browser Search wrote a track total taken from the provider with no field to see or
  correct it. The mapping table now shows it.
- The browser's tool dialogs never took focus, so a keyboard user stayed outside the
  dialog they had just opened and focus was not returned on close.
- Loading track counts disabled every "Use release" button for as long as it ran, which
  can be a hundred provider requests — a release already on screen could not be chosen.
- The Search dialog's outcome line appeared below the entire result list, and was not
  announced to screen readers. It now reads first and is announced.
- The Search dialog never named the album it was searching for.

- Browser Settings could confirm a change that was no longer true: the "saved"
  message was cleared only by the *next* save, so it stayed on screen indefinitely.
  Confirmations now belong to the field that changed and expire on their own.
- Browser Monitoring showed a release list of unbounded length; a library of thousands
  of albums rendered thousands of rows. The list is now bounded and pages in.
- Browser Settings accepted a covered folder that no library could read — outside every
  served directory, or not a directory at all — which would have been configured and
  then silently never tagged. It is now checked and refused with the reason.
- Browser Settings reported an unusable crontab in a page-level banner at the top, far
  from the field that caused it. The reason now appears at the field, which is marked
  invalid and focused.
- Saving one setting in the browser Settings page disabled every other control until it
  finished. Only the field being saved is now busy.
- The browser service's log viewer never refreshed after the page loaded.

- Browser-workspace badges and chips were hard to read: the brand badge, the header
  mark, the status chips and the informational badge all painted the accent colour on
  its own 10% tint, which measures 2.49:1. They now use a darker step of the same hue
  (4.60:1 on the tint, 5.02:1 on a card). The folder-tree expander was darker still —
  1.59:1 — because it kept its own colour inside a selected row instead of following
  the row's text, which is now fixed.
- The browser workspace's primary button was hard to read: its white label sat at
  2.72:1 on the reference's primary fill, and hovering it emptied the button into a
  pale tint at 2.49:1. The label is now a dark colour on the same fill - 5.39:1, and
  5.82:1 when hovered - and the hover lightens rather than darkens, so the label
  never loses contrast at the moment it is pressed.
- De-emphasised browser-workspace text was too faint to read comfortably. The muted
  role sat at 2.33:1 against the page while carrying labels, table headers, notes
  and technical readouts; it now meets the 4.5:1 floor. Status badge and banner
  labels were worse: warning on its own tint was 1.90:1, and the notice and error
  banners were 2.66:1 and 2.56:1, so status text now uses a darkened step of each
  status hue while the status colour itself remains for fills and borders. The
  status tints are now built the way the reference builds them, as the status colour
  at its own alpha, instead of values sampled from screenshots.
- Hovering a browser-workspace control gave no visible feedback: hover painted the
  same colour as the page background, so navigation items, folder-tree rows and
  buttons looked identical hovered and idle. They now use a colour taken from the
  reference's own theme.
- Eleven focusable browser-workspace controls - the ten file-name cells and the
  breadcrumb buttons - had no visible focus ring of their own and fell back to the
  browser default. They now use the same focus ring as every other control.
- The self-hosted service can read a release's provider track count. The command
  existed for the desktop app only, so the browser search could neither show nor
  filter on provider track counts.
- Browser Number and Convert report the files a writer refused instead of
  reporting success for a partly completed run.
- Auto-tag reviews now journal queued track writes, so a review that changed a
  file is revertible again. Queued writes previously bypassed the recovery
  journal, which left the review reporting that it had nothing to undo while
  holding changes the user could not revert, and skipped the full-file recovery
  backup.
- Local browser-service sign-in accepts both standard loopback addresses.
- Switching mounted browser-service roots clears the previous library while the
  replacement scan is in progress or reports an error.
- Browser Settings now keeps saving unavailable until configuration loads
  successfully, tests protected provider credentials after a masked key is
  cleared, and sorts the browser Format column by codec.

### Added

- Browser Settings takes a free-text value — the lyrics service URL, a custom crontab —
  as a draft with Save and Cancel and an "unsaved changes" warning, rather than saving
  whatever was typed so far or losing it on navigation.

- The browser Workspace now shows which albums background tagging left needing
  review: they are named above the file table and marked in the folder tree, so
  a flagged album can be opened where it is listed instead of only being found
  by running a scan again.
- The browser Workspace gained Search, Number and Convert for the album being
  viewed, with the desktop app's behaviour rather than a reduced version of it.
  Search narrows a provider query by artist, album, year, country, format,
  catalog number and barcode, filters, sorts and pages the results, loads
  per-release track counts on request, and finishes in a mapping step where each
  local file can be re-pointed at a different provider track, hand-edited, or
  left out of the write. Number writes sequential track numbers across the whole
  open album using the desktop's ordering rules. Convert rewrites tags from file
  names, one tag from another, or file names from tags, using the desktop's
  presets, and refuses a run whose result would be identical to what is already
  there.
- Soundrobe can now run as a self-hosted web service: a password-protected
  browser dashboard manages mounted music libraries through the existing Rust
  metadata pipeline.
- Background tagging can be scheduled. The service evaluates a crontab in the
  operator's local time and starts unattended tagging runs on it, with the
  reference product's quick presets (every minute through daily at midnight)
  plus a custom expression. An unusable expression is refused rather than
  stored, so a schedule can never be saved in a state where it silently never
  fires. A report is visible under Monitoring.
- Covered folders can be watched, so music added to them is tagged
  automatically. Detection uses the folder's change time, so a watched folder
  does not trigger a full library rescan, and a run started this way covers only
  the new arrivals.
- A Monitoring page shows whether background tagging is on, the crontab and its
  next run, which folders are actually covered, in-progress runs with their
  live step, the outcome of recent runs, the tagging state of each release the
  service surveyed, and the service's own log.
- A Settings page exposes the background-tagging controls (enable, schedule,
  covered folders, watching, and whether to skip already-complete releases)
  beside the application settings (online lookups, Discogs, lyrics download and
  lyrics service URL, Chinese script conversion, assistant autonomy and debug
  logging) and the running version.
- A Workspace settings drawer controls whether the file-details editor is shown
  and which metadata columns the file table displays. These are per-browser
  preferences that survive a reload, and the file-name column cannot be hidden
  because it identifies the rows.
- The service writes its own log to `auto-tagger.log`, so Monitoring shows what
  the running service did without attaching to its stdout.
- The service scans mounted libraries, maintains a health index for releases
  already tagged, automatically processes only releases that need tagging,
  and leaves failed or ambiguous releases in the review queue for user input.
- Docker and Compose deployment files provide a hardened non-root service,
  persistent configuration, direct per-library mounts, and a health endpoint.
- `just web-local <libraries-root>` now builds and starts the browser service
  on loopback for local development, using the standalone app's settings by
  default while still allowing an isolated data directory override.
- The browser service now supports the `directory:read` command, returning a
  folder's subdirectories and parsed tracks in one round trip so the workspace
  can browse and list a folder without a request per file.

## [0.1.11] - 2026-09-16

### Fixed

- Auto-tag Results now retains only the most recent batch and is cleared when loading a library or beginning a new auto-tag run.

## [0.1.10] - 2026-09-15

### Changed

- Auto-tag evaluation corpus refreshes accept an explicit source root through `--source-root` or `SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT`, while committed fixture manifests retain only relative paths.
- Reproducibility audits evaluate cold/warm completeness from the retained native replay case set, allowing that set to differ from the selected equivalence sample.

### Fixed

- Auto-tag evaluation audits now fail closed when ground truth is unscored, supplied native profile phases are incomplete, equivalence evidence is empty, or cold/warm replay identities drift.

## [0.1.9] - 2026-09-15

### Added

- A provider-backed representative auto-tag evaluation ledger and offline replay command make edition mappings, hard negatives, and unresolved cases reproducible across the corpus.
- Search shows local and release track totals, offers a same-track-count toggle and closest-count sorting, and can load missing counts on demand with progress, cancellation, and retry.
- Search results show country and catalog number, with country/format filters, catalog-number and barcode text matching, and a clear-filters action.

### Fixed

- Auto-tag now removes a known artist prefix and matching release year from folder-derived album search hints, so decorated folder names can reach their provider edition without overriding tagged album metadata.
- Auto-tag's guarded filename comparison now removes a known artist prefix from the comparison key, so punctuation and Latin-diacritic variants can match a provider track without changing the displayed local title or weakening duration and uniqueness checks.
- Auto-tag recognizes skit labels and Latin accent differences when unique track titles and matching durations confirm the same recording, allowing complete bonus-track editions to match without manual selection.
- Auto-tag ignores known disc and edition suffixes in album names when selecting a complete provider release, so boxed and maxi-single folders can match their corresponding edition without positional-only guesses.
- Auto-tag keeps Discogs CD audio separate from DVD extras when a release detail contains both media groups, so audiovisual bonus content cannot displace the selected audio tracklist.
- FLAC album reads prefer canonical Vorbis comments when a legacy ID3 prefix contains stale metadata, keeping successful writes and readback aligned while preserving unrelated tags.
- Opening a different artist library now clears the previous auto-tag results.
- Search track-count controls become available when counts are loaded, including for Discogs; unavailable counts are explained instead of silently disabling comparison.

## [0.1.8] - 2026-09-10

### Fixed

- Auto-tag searches additional Discogs editions when initial releases do not match. Complete, ordered tracklists can match a reissue or tolerate one timing discrepancy of up to ten seconds; incomplete or conflicting matches still require review.
- Discogs tagging includes featured performers and preserves existing track credits when provider credits only partly overlap.
- Malformed Discogs search responses no longer trigger AI tagging by being mistaken for an empty catalog.

## [0.1.7] - 2026-09-09

### Added

- Auto-tag Results now opens an album review with complete metadata comparisons, lyrics, artwork previews, matching evidence, and retry/manual-search actions. Reviews remain available during the current session.
- Keep or fully revert an album’s auto-tag changes, including artwork and lyrics, from Review or Undo. Recovery checks for later edits before restoring files; keeping changes retains session undo.

### Changed

- **SACD-R DSD extraction** — the media toolbox now decodes sector-packed DSD64
  tracks to 96 kHz FLAC instead of interpreting them as PCM noise, extracts raw
  SACD-R images through `sacd_extract`, and retries transient ISO listings on
  network volumes.
- Raw CD ISO conversion now requires `slice-iso --raw-cd`; failed SACD
  extraction no longer falls back to interpreting the image as PCM audio.

## [0.1.6] - 2026-09-06

### Fixed

- **DeepSeek auto-tag response time** — album corrections using DeepSeek V4
  Flash 0731 on OpenRouter disable extended reasoning and prefer affordable
  providers with faster generation, while retaining metadata validation and
  provider fallback during outages.
- **Auto-tag benchmark resilience** — malformed non-object AI responses are
  recorded as validation failures without aborting remaining benchmark trials.
- **Large-album AI fallback** — automatic tagging now scales structured-output
  capacity with album size and allows longer responses, reducing timeouts and
  truncated metadata for live sets and other high-track-count releases.
- **Provider outage backoff** — MusicBrainz overload responses with a zero retry
  hint now use bounded backoff, and failed batch albums wait before their one
  automatic retry instead of immediately repeating the outage.

## [0.1.5] - 2026-08-31

### Fixed

- **Reliable library auto-tagging** — transient MusicBrainz and Discogs
  failures now use bounded retries, and batch auto-tagging retries only
  provider-unavailable albums once before showing a complete per-album result
  summary.

## [0.1.4] - 2026-08-30

### Fixed

- **Manual release search** — result filtering and sorting now cover the full
  provider catalog, with title/artist, year, and MusicBrainz track-count
  controls; multi-word MusicBrainz artist searches also recover from transient
  provider overload responses.
- **Safer automatic release matching** — auto-tag now keeps embedded album
  tags separate from folder edition hints, rejects conflicting or weak
  provider releases, and avoids copying IDs between unrelated editions.
- **Validated AI fallback** — when every enabled metadata provider confirms
  that no credible release exists, a complete high-confidence AI result can
  update standard tags while clearing stale provider IDs.
- **Fail-closed auto-tagging** — provider outages and invalid, unavailable, or
  low-confidence AI results now leave files unchanged and report a dismissible
  Needs Review warning instead of writing folder-derived metadata; provider
  identity and release-detail failures remain unavailable, selected-disc
  matching stays scoped, and unknown Discogs formats are not guessed.

## [0.1.3] - 2026-08-27

### Fixed

- **Legacy MP3 tag compatibility** — loading an album now upgrades ID3v2.2
  and ID3v2.3 MP3 tags to the supported writable version, repairs recognized
  non-standard dates, and keeps lyric-compatible ID3v2.3 tags unchanged.

## [0.1.2] - 2026-08-26

### Fixed

- **Legacy MP3 duration display** — playable MP3 files with non-standard ID3
  year values now retain their audio duration and metadata instead of appearing
  unreadable with a zero duration.

## [0.1.1] - 2026-08-21

### Added

- **Signed in-app updates** — packaged production builds check once at startup
  and can also check from Settings. Available releases show their version,
  date, and notes before an explicit download; installation reports progress,
  refuses to interrupt protected library work, cannot be bypassed by quitting,
  and can be retried after failure.

### Changed
- **Faster WAV library reads** — embedded lyrics now reuse the WAV metadata
  parse, avoiding a second file open and full Lofty pass while preserving
  large-file metadata-only reads and compatibility fallbacks.

- **Keka-backed RAR extraction** — the media toolbox now prefers Keka’s bundled
  RAR decoder and reports its password or access errors directly.

- **Verifiable updater releases** — the release workflow now signs every
  platform updater bundle, publishes installer-aware `latest.json` metadata
  for macOS ARM/Intel, Windows x64, and Linux x64/ARM64 AppImage and deb
  packages, includes the versioned changelog notes, rejects prereleases, and
  fails when a required artifact or signature is missing.
  
- **Command-level modification history** — metadata edits now appear in a
  20-command session history with explicit Undo and history controls, including
  grouped Extra Tags, auto-tag, assistant, search, conversion, numbering, and
  audit changes that can be reverted to an earlier point without a keyboard
  shortcut.

## [0.1.0] - 2026-08-11

### Changed

- **Reliable embedded lyrics** — lyrics now preserve valid embedded content
  before checking `.lrc`, `.txt`, or LRCLIB sources; strictly reject uncertain
  or malformed text instead of writing mojibake; embed synchronized and plain
  lyrics in Navidrome-compatible tags without modifying sidecar files; and
  report written, preserved, unavailable, unsupported, and failed tracks.

- **Explicit release artifact names** — macOS and Linux GitHub release
  downloads are now named `soundrobe-{version}-{os}-{arch}` (for example
  `soundrobe-0.1.0-macos-arm64.dmg`, `soundrobe-0.1.0-macos-intel.dmg`, or
  `soundrobe-0.1.0-linux-arm64.deb`), so the app name, version, OS, and
  architecture are visible in each filename.

- **Linux ARM64 releases** — the release pipeline now builds and publishes
  ARM64 AppImage and deb packages alongside the existing x64 ones.

- WAV files whose terminal ID3 chunk omits its optional RIFF padding byte can
  now be edited without changing their PCM audio.

- Pull requests now report a single required test result, while cross-platform
  release bundles are built nightly only when the app version is unpublished.

Work since the 1.0.0 milestone. This period covers the migration of the
project from the legacy Python CLI to the current Tauri 2 desktop application
**Soundrobe**, followed by the hardening of that application toward its first
desktop release.

### Added

- **Offline assistant organization previews** — unambiguous requests to group
  files into album folders now create a preview without requiring an LLM
  connection.

- **Isolated advisor consultations** — project agents can explicitly consult a
  separate Codex or Claude session with bounded evidence, filtered environment
  access, and no workspace tools before the main agent decides how to proceed.

- **New-conversation button in the AI Assistant** — the panel header now has
  a "New chat" button that starts a fresh conversation (same reset as the
  `/clear` command), disabled while a request is in flight.

- **Multi-disc cue-splitting** — `cue-split` now detects folders holding
  multiple CUE sheets (2CD/3CD sets) and slices each disc into its own
  subfolder under `<album>-tracks/<disc>` with `disc=N/M` tags, instead of
  overwriting tracks when discs restart at track 01.

- **Unified media toolbox script** — `scripts/toolbox.sh` is now the single
  entry point for media/library utilities: cue-splitting (FLAC/WAV album
  images per CUE sheet, with image copies and optional doctor health check),
  DSF/DSD-to-FLAC conversion, audio ISO slicing (K2HD SACD and raw CD), RAR
  extraction, and the FLAC doctor/QA pipeline. The `doctor` command renders
  an HTML corruption report automatically whenever it saves a scan report.

- **Tauri 2 desktop application** — Rust backend with a React/TypeScript
  renderer, replacing the legacy Python CLI and the intermediate Electron
  prototype. Renderer code talks to the native process exclusively through the
  shared `DesktopAPI` contract, and all media writes are serialized through a
  shared Rust write queue with atomic validation.
- **Auto-tag pipeline** — folder hints → exact provider IDs / artist release
  browsing → MusicBrainz → Discogs → conditional LLM fallback, with genre
  fill, per-track artist credit preservation, multi-disc and compilation
  handling, provider-ID persistence (MusicBrainz + Discogs), and
  Simplified/Traditional Chinese artist matching.
- **Manual release search** — MusicBrainz/Discogs search dialog with release
  preview and an explicit confirm-to-write step.
- **AI Assistant** — LLM-driven assistant with a native tool protocol
  (`metadata.patch`, `metadata.transform`, `files.transform`,
  `auto_numbering_tracks`), deterministic routing, durable task state, session
  persistence, self-review, and approval-gated, preview-first mutations.
- **Cover and artwork** — resolution in the order local → Cover Art Archive →
  Discogs → TheAudioDB, artist artwork, embedded-cover removal and
  suppression, stale-cache invalidation, and an in-memory cover cache with
  preload for fast track selection.
- **Lyrics** — LRCLIB fetch and album lyrics download with encoding repair;
  local lyrics take precedence over remote ones.
- **Batch editing** — origin-scoped extra-tags editor, per-track write
  resilience with structured results, configurable write concurrency,
  worker-backed write queue, and undo restricted to successful writes.
- **Format support** — atomic readers/writers for MP3, FLAC, M4A/MP4,
  OGG/Opus, APEv2, WAV, and AIFF; FLAC covers stored as native `PICTURE`
  blocks; legacy UTF-16 MP3 artwork; WAV reads prefer ID3v2 over LIST INFO.
- **Configuration** — unified `~/.soundrobe/config.yaml` with environment
  precedence and secret redaction, LLM provider selection with a test
  connection, `AUTO_TAG_CHINESE_SCRIPT` for Simplified/Traditional conversion,
  and configurable write concurrency. Existing `~/.auto-tagger` application
  data is migrated into the new directory on startup.
- **Diagnostics** — real-write probe that tests the write path against a copy
  of a track, and a volume write-probe command.
- **UI** — tabbed settings, audit panel with findings highlighted in the track
  grid, redesigned Convert dialog with presets and live preview, ctrl-click
  track selection, right-click context menu, refreshed app icons, and an
  explicit Apply Changes button for right-panel metadata edits.
- **Local dataset** — optional MusicMoveArr dataset lookup with read-only
  status reporting.
- **Platform packaging** — macOS (Apple Silicon and Intel) app/DMG, Windows
  NSIS, and Linux AppImage/deb bundles validated by a CI matrix.

### Changed

- **AI Assistant side sheet** — the assistant now uses Soundrobe's native
  light and dark styling, a responsive overlay layout, context-aware composer,
  prompt suggestions, clearer progress states, and streamlined review cards
  for approval-gated changes.

- **Application rewritten** — the project moved from the Python `auto-tag`
  CLI to the Tauri desktop application; the intermediate Electron prototype
  was dropped in favor of the native Rust shell.
- **Configuration path** — the canonical configuration and application-data
  directory is now `~/.soundrobe`; existing `~/.auto-tagger` data is migrated
  there before the legacy directory is removed.

### Fixed

- **Manual Track Match artist and genre consistency** — provider collaborator
  identities now survive track mapping and reassignment, edited collaborative
  credits produce repeated artist tags, and missing provider genre is resolved
  when available or explicitly preserved when unavailable.

- **Canonical CJK artist credits during auto-tagging** — verified provider
  aliases now resolve back to the trusted folder artist across strong and weak
  track matches, while preserving genuine collaborators and only using unique
  track identities—scoped by disc whenever multi-disc evidence exists—to align
  alphabetically enumerated files.

- **Cover removal priority** — removing album artwork now selects the highest-
  priority standard cover name consistently, regardless of filesystem directory
  enumeration order.

- **Windows album selection** — album filters and album-scoped track actions now
  work correctly with native Windows backslash paths.

- **Complete MusicBrainz artist searches** — manual Search now resolves artist
  aliases to the exact MusicBrainz artist, loads every matching release page,
  lets users filter the cached release titles, and reuses an identical completed
  search after returning to the form or reopening the dialog without additional
  provider requests.

- **Discogs release title filtering** — manual Search now provides the same
  release-title filter for the currently loaded Discogs results page.

- **Faster AI Assistant applies** — approved standard metadata/tag batches now
  write album folders concurrently using the configured write limit, show
  preflight, writing, and verification progress, and still finish only after
  native readback confirms the requested changes.

- **Safer AI Assistant approvals** — file-operation previews now show their
  exact source and destination paths, approved moves refuse newly occupied
  destinations, dependent plan steps enforce approval order, and ordinary
  answers ending in a question no longer trigger the clarification-loop guard.
- **Auto-tag folder years** — release years at the start of a quoted album
  title are now detected even when the artist name precedes the title.

- **Faster WAV library reads** — standard WAV metadata loading now seeks past
  PCM audio and reads only chunk headers and metadata, while padded and
  malformed files retain the compatibility parser.

- **Faster metadata reads** — Opus, M4A/MP4, AIFF, and Monkey's Audio
  property/tag fallbacks now seek around encoded audio instead of loading the
  complete file, while unusual layouts retain their compatibility readers.

- **Faster metadata writes** — WAV writes reuse one loaded source, stream
  LIST/INFO cleanup, and compare PCM in bounded blocks; common FLAC edits now
  update only the metadata prefix; MP4 and OGG/Opus validation avoids cloning
  encoded audio and reuses already loaded bytes for local staging. Batch writes
  now use four folder workers by default after controlled local and SMB tests.

- **Auto-tag genre fallback** — structured LLM genre responses now retain
  numeric confidence values serialized as text, allow longer repair responses,
  and explicitly disclose when genre inference fails or remains below the
  confidence threshold instead of ending with a silent missing genre.

- **Assistant IME input** — confirming a candidate with Enter in Chinese and
  other input methods no longer sends the message prematurely, including the
  post-composition Enter sequence emitted by macOS WebKit.

- Batch edit: tag changes are no longer written when you click anywhere
  inside the batch editor panel — edits persist only when the Apply changes
  button is clicked, matching the single-track inspector.

- Manual release search: confirming a track match now writes the exact
  disc/track numbers and titles shown in the match table, honors the
  Simplified/Traditional Chinese script setting like auto-tag, keeps manual
  assignments and unused-track counts stable while writing, and leaves rows
  set to "Do not update" entirely untouched. Track totals from MusicBrainz
  and Discogs are now per disc (e.g. 14/14 for a two-CD set) instead of the
  whole-release count.
- A candidate without a year no longer removes an existing year tag when
  writing auto-tag or manual-search results; a year is written only when the
  matched release provides one.

- Manual MusicBrainz release search: resolving a release that exists no
  longer reports "MusicBrainz release not found" when the failure is a
  transient network error or a MusicBrainz rate limit — the dialog now shows
  the actual HTTP status (including the `Retry-After` hint when present)
  instead of claiming the release is missing.

- Assistant folder-derived metadata: instructions like "set album based on
  their folder name" now derive each track's album from its containing
  folder instead of writing the instruction text as a literal tag value. The
  preview shows the derived per-track values, verification re-checks the
  folder source after writing, and one identical literal value planned across
  many different folders raises a preview and post-write warning.

- WAV metadata: garbled LIST INFO reads, trailing junk/null-byte padding,
  over-eager orphan-tail repair, and reads preferring ID3v2 over stale LIST
  INFO.
- FLAC integrity: ghost Vorbis Comment blocks, stray APEv2 tags, ID3v2
  prefixes, the 20 MB block-size limit with broken-chain detection, and audio
  offset preservation during metadata rewrites.
- Auto-tag matching: `feat.` tracks with missing titles, album-subtitle
  poisoning, `leading_cjk` collisions, accidental `Various Artists`
  overwrites, artist-only fallback regressions, and direct MBID lookups with
  mismatched folder names.
- Batch writes: masked per-track failures, silently ignored disc fields, and
  SMB volume failures (data-only copy instead of `fs::copy`).
- Assistant reliability: false completions, LLM timeouts, malformed JSON,
  planned-action verbosity, and low-confidence candidate rejection.
- Assistant folder grouping: the assistant can now group tracks into album
  folders via the new `files.relocate` tool (destination derived from a tag
  field or filename transformation), and it stops asking the same clarifying
  question repeatedly — after a user answers, it either creates an approval
  preview or states the limitation.
- Assistant no longer guesses an action after a clarifying question stays
  unanswered: if the model repeats a question the user already answered, the
  app refuses to run a guessed mutating tool and asks the user to pick an
  option instead.
- Assistant `plan.create` now shows its real step schema to the model and to
  validation (it previously fell through to an empty schema), so multi-step
  requests like "group tracks into albums by the title before (" produce a
  valid two-step preview instead of repeatedly failing argument validation.
- Assistant tool guidance for `metadata.transform`/`files.relocate` now
  documents the regex_extract capture-group requirement and the {value}-only
  destination placeholder, so grouping by a title prefix no longer silently
  skips paren-less tracks or creates placeholder-named folders.
- Assistant title-based album grouping: `metadata.transform` regex extraction
  now writes the captured value even when it equals the whole source string,
  so setting the album to the title before '(' works for paren-less original
  tracks too (previously only version-suffixed tracks such as `(伴奏)` were
  fixed, or the request reported no changes at all). No-change tool results
  now state how many tracks were scanned, skipped, or already correct, and
  the assistant's clarification guard recognizes Chinese questions ending in
  the full-width `？`.
- Undo: auto-tag and batch writes are now correctly undoable.
- Tag hygiene: stale TXXX album-artist aliases shadowing TPE2 and leftover
  Vorbis ALBUMARTIST variants are cleared before writes.
- Configuration: `AUTO_TAG_CHINESE_SCRIPT` validated at load, concurrent
  config writes serialized, API-key credential flow fixed, and the configured
  lyrics URL honored for single-track fetches.
- UI: stale invoke callbacks after page reload, rapid right-click promise
  rejection, and optimistic updates shown before writes complete.

### Removed

- **Legacy `grill-with-docs` agent skill** — the retired planning interview
  workflow is no longer installed in the project skill catalog.

- Standalone `fix-medium` toolbox command and its obsolete
  `fix-medium-flac-tracks.js` implementation.
- Legacy Python CLI and Electron prototype artifacts (2026-07-18).

## [1.0.0] - 2026-05-10

Initial release milestone of the legacy Python `auto-tag` CLI for
Navidrome-oriented libraries:

- `tag` and `batch` commands with `--dry-run`, `--interactive`, and `--yolo`
  modes.
- Album matching via MusicBrainz, Discogs, and beets, with smart compilation
  detection and Chinese folder-name parsing.
- Health reports (`--health-report`) with cover-art checks.
- ReplayGain calculation and ffprobe-based audio validation.
- Local MusicMoveArr dataset lookup.

[Unreleased]: https://github.com/ydeng11/Soundrobe/compare/v0.1.10...HEAD
[0.1.10]: https://github.com/ydeng11/Soundrobe/releases/tag/v0.1.10
[0.1.3]: https://github.com/ydeng11/Soundrobe/releases/tag/v0.1.3
[0.1.2]: https://github.com/ydeng11/Soundrobe/releases/tag/v0.1.2
[0.1.1]: https://github.com/ydeng11/Soundrobe/releases/tag/v0.1.1
[0.1.0]: https://github.com/ydeng11/Soundrobe/releases/tag/v0.1.0
[1.0.0]: https://github.com/ydeng11/Soundrobe/releases/tag/v1.0
