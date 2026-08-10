# <img src="design/logo-final-2026-07-28/soundrobe-s-inline.png" alt="S" width="26" height="32" valign="middle">oundrobe

Soundrobe is a Tauri 2 desktop application for preparing and maintaining
Navidrome-friendly music libraries. It combines metadata automation, an AI
assistant, path-aware metadata auditing, and album/artist artwork downloading
in one workflow.

> **Status: under heavy development.** Soundrobe is useful today, but its UI,
> provider behavior, and supported workflows are still changing. Always work
> on a backup or copy of your music library and review changes before asking
> Navidrome to rescan it.

![Soundrobe desktop workflow](docs/assets/Soundrobe-1.gif)

## What makes Soundrobe different

- **Automation:** Scan an album or library, derive hints from the folder,
  existing tags, and filenames, then resolve candidates through the local
  cache/dataset and enabled MusicBrainz, Discogs, and optional LLM providers.
  Accepted candidates can update tags, lyrics, and artwork as one workflow.
- **AI assistant:** Ask for supported operations in natural language, such as
  inspecting a selection, editing metadata, running auto-tagging, or auditing
  it. Mutating assistant actions are presented as an approval preview and are
  verified with readback after they are applied.
- **Audit:** Check whether tags agree with the album folder, artist folder,
  filename, track/disc numbering, and other local evidence. Deterministic
  checks run first; the LLM is used only for targeted review where judgment is
  useful. Findings are classified as correct, warning, or error, and only
  eligible high-confidence fixes can be applied automatically.
- **Artwork downloading:** Prefer artwork already in the album folder or
  embedded in a track. When it is missing, Soundrobe can use configured remote
  providers to save normalized album artwork as `cover.jpg` and artist
  artwork as `artist.jpg` in the artist folder.

## Start with a good library structure

Organize files before tagging. The recommended starting point is:

```text
/Music/
└── Artist/
    ├── artist.jpg                 # optional artist artwork
    └── Album/
        ├── 01 - Track One.flac
        ├── 02 - Track Two.flac
        └── cover.jpg              # optional album artwork
```

Soundrobe works best when one album is in one folder and tracks are directly
inside that folder. In other words, start with `/artist/album/track` (where
`track` is the audio filename). It also understands common `Artist - Album`
folder names and multi-disc folders such as `CD1` or `Disc 2`, but a regular
`Artist/Album/` hierarchy is the least ambiguous input.

This structure is a workflow convention, not a replacement for tags:
[Navidrome organizes music from embedded metadata rather than from folders](https://www.navidrome.org/docs/usage/library/tagging/).
Soundrobe uses the folders as strong evidence when preparing and auditing
those tags.

## Navidrome-oriented tags

For every track, aim to have consistent values for:

- `TITLE`
- `ARTIST` and plural `ARTISTS` when there are multiple performers
- `ALBUM`
- `ALBUMARTIST` and plural `ALBUMARTISTS` when applicable
- `TRACKNUMBER`
- `DISCNUMBER` for multi-disc albums
- `DATE`/year, `GENRE`, and compilation metadata when known

Keep the album and album-artist values consistent across all tracks in an
album. For compilations, use `Various Artists` and the compilation flag as
appropriate. Soundrobe preserves individual collaborator values in plural
artist fields where the format supports them; this avoids hiding multiple
artists inside one display string.

## A safe workflow

1. Put new music into the `Artist/Album/` structure above and keep a backup.
2. Open the library in Soundrobe and inspect the discovered tracks and folder
   hints.
3. Run auto-tagging on a selected album first. Review the selected candidate
   and progress messages before processing a larger library.
4. Run **Audit** after tagging to find path/tag mismatches and ambiguous
   fields. Apply only the fixes you understand.
5. Use **AI Assistant** for bounded, specific operations. Approve its preview
   before any mutation; use the verification/readback result as the completion
   signal.
6. Download missing album or artist artwork and optionally fetch missing
   lyrics. Then rescan the library in Navidrome.

## Current limits and expectations

- The library scanner recognizes `.mp3`, `.flac`, `.m4a`, `.mp4`, `.wav`,
  `.ogg`, `.opus`, `.aiff`, and `.ape`. Individual metadata capabilities vary
  by format, so test representative files before a large batch.
- Tracks are expected directly inside an album folder. Deeply nested audio
  files, hidden files, and non-audio files are not part of the normal scan.
- Folder names and filenames are evidence and fallbacks, not guaranteed
  provider truth. A provider may have no exact release, may return a different
  edition, or may be unavailable because remote lookup is disabled, offline,
  rate-limited, or missing credentials. Review candidate matches instead of
  assuming every result is correct.
- LLM output is assistive and can be uncertain. It is intentionally lower
  priority than explicit provider evidence for album and track matching; do
  not use it as a substitute for review on unusual releases.
- Artwork downloading needs usable album metadata and network/provider access
  when no local or embedded artwork exists. An explicit cover removal is
  remembered so automatic artwork work does not immediately restore it.
- Lyrics downloading is optional and only fills missing lyrics; local `.lrc`
  or `.txt` files take precedence. Enable it in settings/configuration if you
  want remote lyrics.
- Writes change files in place. Use a copy for experiments, especially with
  unfamiliar tag formats, and let Navidrome rescan after metadata or file
  moves. Keep debug logs and provider credentials private.

## Development

Prerequisites:

- Node.js 22 and npm
- A stable Rust toolchain
- The platform dependencies required by Tauri 2
- `just` for the repository commands

Install dependencies and start the desktop app:

```bash
just fe-install
just fe-dev
```

The Rust backend is in `frontend/src-tauri`, the React renderer is in
`frontend/src`, and renderer/Rust tests live beside those maintained surfaces.

## Verification

Run the complete local quality gate:

```bash
just fe-check
```

Useful targeted commands:

```bash
just fe-typecheck
just fe-test
cd frontend/src-tauri && cargo clippy --all-targets -- -D warnings
```

Credentialed provider smoke tests are separate from the default offline suite;
see `just --list` for the available gates.

## Configuration

Copy `config.example.yaml` to `~/.soundrobe/config.yaml` and set only the
providers and behavior you use. Secrets can instead be supplied through the
environment variables documented in that example. Environment values take
precedence over the config file, and secrets remain in the native process.
Existing settings under `~/.auto-tagger` are migrated into `~/.soundrobe` on
startup. The old directory is removed only after the migration succeeds.

The optional local dataset is read from `dataset_path`. If it is absent or
invalid, the app reports it as unavailable and continues with enabled remote
providers. Useful settings include `remote_lookup_enabled`, `discogs_enabled`,
`lyrics_download_enabled`, `lyrics_api_url`, `chinese_script`, and
`write_concurrency`.

For a low-cost OpenRouter setup, start with
`deepseek/deepseek-v4-flash`. OpenRouter currently lists it at
[$0.09/M input and $0.18/M output](https://openrouter.ai/deepseek/deepseek-v4-flash/pricing);
pricing and availability can change, so check the provider page before making
large runs.

## Distribution

Build the current platform bundle:

```bash
just fe-build
```

Build an explicit unsigned distribution target:

```bash
just fe-dist mac
just fe-dist win
just fe-dist linux
```

Cross-platform bundle and native workflow smoke coverage is defined in
`.github/workflows/tauri.yml`.

## License

MIT — see [LICENSE](LICENSE).
