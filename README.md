# Auto Tagger

Auto Tagger is a Tauri 2 desktop application for editing and enriching audio
metadata in Navidrome-oriented libraries. The native backend is Rust; the
renderer uses React and TypeScript.

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

Copy `config.example.yaml` to `~/.auto-tagger/config.yaml` and set only the
providers and behavior you use. Secrets can instead be supplied through the
environment variables documented in that example.

The optional local dataset is read from `dataset_path`. If it is absent or
invalid, the app reports it as unavailable and continues with enabled remote
providers.

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

MIT
