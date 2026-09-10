# Discogs edition regression fixture

`local.json` contains titles, positions, artists, and durations read from the user's original 18-track My Everything album. It contains no audio and no provider IDs.

`release-16211782.json` contains the matching fields from [Discogs release 16211782](https://api.discogs.com/releases/16211782), fetched on 2026-09-10. This Japanese 2020 reissue lists “Too Close” as 3:45; the local file is 3:36.36. The fixture intentionally retains this discrepancy and the local “Japan Bonus Track” annotation.

The tests require complete title and position evidence before allowing the single bounded duration discrepancy. The release ID is fixture evidence, not an application preference.

Run the explicit native smoke with:

```sh
SOUNDROBE_EDITION_SOURCE='/path/to/original/album' cargo test --manifest-path src-tauri/Cargo.toml live_discogs_edition_smoke --lib -- --ignored --nocapture
```

The gate requires 18 FLAC tracks, copies them into `/private/tmp`, uses an isolated cache, and checks cold/warm selection, native tag readback, audio-payload hashes, and unchanged source hashes. It reads the normal application configuration without printing credentials and disables AI and remote lyrics for the gate. Its JSON results and hash manifests remain in the temporary smoke directory.
