# Relapse Deluxe regression fixtures

Captured 2026-09-11. `local.json` contains metadata read with ffprobe from
`/Users/ihelio/Downloads/Music/Eminem/2009 Eminem - Relapse (With Bonus)/`.
The 22 FLAC files have no title or track-number tags; those fields come from filenames.
No audio, artwork, credentials, or user configuration is included.

`release-36441795.json` is the public response from https://api.discogs.com/releases/36441795.
`candidate-36441795.json` is the corresponding candidate from Soundrobe's saved lookup.
`provider-album-16649340.json` is a cached parsed Discogs response for the competing
22-track release; its two bonus tracks are different single versions.
The ordinary 20-track negative is derived by removing the target's bonus tracks in tests.

`reviewed-truth.json` is the narrow reviewed ledger for this exact source fixture. It
blesses Discogs release `36441795`, rejects the 20-track and alternate-bonus editions,
and records the complete one-to-one mapping, including the reversed bonus positions.
It also freezes the pre-fix 16-strong/6-position-only baseline and the post-fix
22-strong/0-position-only result, with six guarded-title matches.
