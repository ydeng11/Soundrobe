# Release Checklist

## Version

Set the same semantic version in `frontend/package.json`,
`frontend/src-tauri/Cargo.toml`, and `frontend/src-tauri/tauri.conf.json`. The
test suite rejects drift between these manifests, and the Settings footer reads
the compiled Cargo version through the native `app_info` command.

## Tauri Desktop

Run the local quality and credentialed production-client gates:

```bash
just fe-check
just fe-smoke-openrouter
just fe-smoke-assistant
```

On macOS, also exercise the real native image picker and build the unsigned app
and DMG:

```bash
just fe-smoke-cover-picker
CI=true just fe-dist mac
rustup target add x86_64-apple-darwin
just fe-dist-mac-intel
```

`CI=true` uses Tauri's deterministic DMG path and skips Finder-only cosmetic
scripting; the app contents and disk image remain the same release artifacts.
The Intel recipe cross-builds the x86_64 app and DMG from Apple Silicon.

Windows NSIS and Linux AppImage/deb bundles are built and launch-smoked by the
`tauri.yml` CI matrix. A desktop release is not complete until those platform
jobs and the macOS app/DMG job pass.

Do not commit `.env.local`, API keys, signing credentials, or notarization
credentials.

## Legacy Python CLI

The commands below apply only to the legacy Python package, not the maintained
desktop application.

### Local Build

```bash
python -m build
python -m pip install --force-reinstall dist/auto_tagger-*.whl
auto-tag --version
```

### TestPyPI

```bash
python -m twine upload --repository testpypi dist/*
python -m pip install --index-url https://test.pypi.org/simple/ auto-tagger
```

### PyPI

```bash
python -m twine upload dist/*
```

Do not commit API tokens, PyPI tokens, Homebrew credentials, or `.pypirc`.

### Homebrew

1. Update `packaging/homebrew/auto-tagger.rb` with the final PyPI source URL.
2. Replace `UPDATE_AFTER_PYPI_RELEASE` with the source archive SHA256.
3. Run `brew audit --strict --online auto-tagger`.
4. Run `brew test auto-tagger`.
