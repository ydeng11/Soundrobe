# Updater releases

Soundrobe uses the native Tauri updater behind its own Rust commands and the
renderer-neutral `DesktopAPI`. Packaged production builds check this endpoint:

`https://github.com/ydeng11/Soundrobe/releases/latest/download/latest.json`

The app checks once per launch and only downloads after the user accepts the
available release. Development and unpackaged release binaries do not check
for updates.

## Bootstrap limitation

The first Soundrobe release that contains the updater public key cannot update
older builds that do not already have updater support. Users of those older
builds must install the updater-enabled release manually. In-app updates work
only from that bootstrap release forward.

Do not claim the updater is available to an installed version until a real
upgrade from that exact version has been verified. Replacing the embedded
public key also strands installations that trust only the previous key unless
a key-transition release is shipped first.

## Signing-key custody

Tauri update signatures are mandatory. The public key is embedded in
`src-tauri/tauri.conf.json`; it is safe to publish. The corresponding private
key must remain private and stable for the lifetime of installations that
trust this public key.

- Keep at least two encrypted backups of the private key in separate trusted
  locations. Test that the backups can be recovered without exposing their
  contents in logs or shell history.
- Store the private key content in the GitHub Actions repository secret
  `TAURI_SIGNING_PRIVATE_KEY`. The release workflow fails before building when
  the secret is absent. This repository's key has no passphrase, so the
  workflow explicitly sets `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` to an empty
  value to keep Tauri non-interactive.
- Never commit the private key, print it, download it for inspection in CI, or
  place it in `.env.local`. Local development does not require it.
- Treat key loss as loss of the ability to update existing installations. Key
  rotation needs an explicitly planned transition release.

## Release flow

The existing Release workflow keeps all three triggers:

- a pushed `v*.*.*` tag;
- the nightly schedule, when the synchronized app version is unpublished;
- manual `workflow_dispatch`.

Before release, synchronize the version in `package.json`,
`src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`, then finalize the
changelog. A pushed tag must exactly equal `v<version>`.

The matrix retains the packaged-app smoke tests and builds:

- macOS ARM64 and Intel DMGs plus `.app.tar.gz` updater bundles;
- Windows x64 NSIS installers;
- Linux x64 and ARM64 AppImage and deb packages.

Only the workflow adds `src-tauri/tauri.updater.conf.json`, which enables
`createUpdaterArtifacts`. It supplies `TAURI_SIGNING_PRIVATE_KEY` to the Tauri
build and renames each updater bundle together with its `.sig` file. macOS is
explicitly ad-hoc signed with `APPLE_SIGNING_IDENTITY=-`; this is code signing,
not Developer ID signing or notarization, so the README Gatekeeper warning
still applies.

After all matrix artifacts are downloaded, the workflow runs
`scripts/generate-updater-manifest.mjs` once. The generator rejects missing or
duplicate artifacts/signatures, rejects non-stable release tags, copies the
matching version section from `docs/CHANGELOG.md` into the in-app prompt as
plain text, and creates seven installer-aware entries:

- `darwin-aarch64-app` and `darwin-x86_64-app`;
- `windows-x86_64-nsis`;
- `linux-x86_64-appimage`, `linux-x86_64-deb`,
  `linux-aarch64-appimage`, and `linux-aarch64-deb`.

Every entry uses the release tag in its GitHub download URL and embeds the
exact text from its adjacent `.sig` file.

## Verify a published release

Do not consider a release complete only because the GitHub Actions job is
green. Download the published assets into a new temporary directory and
verify the public release surface:

```bash
release_tag=vX.Y.Z
verification_dir="$(mktemp -d)"
gh release download "$release_tag" \
  --repo ydeng11/Soundrobe \
  --dir "$verification_dir"
```

Then check all of the following:

1. `latest.json` parses, reports version `X.Y.Z`, and contains exactly the
   seven target keys listed above.
2. Every manifest URL contains `/releases/download/$release_tag/`, has a
   unique matching downloaded artifact, and returns HTTP 200 without GitHub
   authentication.
3. Every manifest signature is non-empty and exactly equals the trimmed
   contents of `<artifact>.sig`.
4. Both macOS updater archives and their signatures exist; Windows has one
   renamed NSIS installer and signature; each Linux architecture has one
   AppImage/signature pair and one deb/signature pair.
5. The normal DMG, NSIS, AppImage, and deb installers remain published and the
   platform smoke results from the same workflow run are green.
6. From the previous packaged production version, a manual Settings check
   finds the new version. Confirm the notes, defer once with **Later**, retry a
   deliberately interrupted download if practical, then complete a real
   update and verify the relaunched version. Windows installer behavior must
   be checked on Windows because it does not use Soundrobe's explicit
   post-install restart path.

Delete the temporary verification directory when review is complete. Never
place signing keys in it.
