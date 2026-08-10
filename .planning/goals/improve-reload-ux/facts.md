# Facts — Improve save+reload UX

- Sidebar album navigation filters tracks in-memory instead of re-reading from disk. Clicking an album sets `activeAlbumPath` and immediately shows tracks without any scanning/progress state.
- All library tracks are stored in `state.tracks` at all times. Switching albums does not replace the track array — `SET_ACTIVE_ALBUM` only changes the filter key.
- When selecting an album in the sidebar, `scanning` is not set to `true`. No progress bar appears. The tracks are filtered at render time.
- After a single-track metadata save, no disk re-read occurs. The optimistic update keeps the UI in sync. The only side effect after save is clearing the cover art cache when album/title changes.
- When a metadata save fails, the rollback only restores the track if the in-memory state still matches what was optimistically written. If the user made further edits or switched tracks, the rollback is skipped to prevent data loss.
- After auto-tag completes, only the albums that were tagged are re-read from disk. The library-level scan (`scanLibrary`) is skipped; instead `readAlbum` is called for each modified album.
- A manual refresh button is added to the title bar (next to the library path or toolbar). It triggers a full re-scan of the library, re-reading all albums and tracks from disk. Cmd+R / Ctrl+R also triggers refresh.
- After audit completes, only the audited albums are re-read from disk instead of re-scanning the entire library.
- When switching between albums in the sidebar, the selected track, multi-selection, and cover art are preserved if possible (clear only when the selected file isn't in the new filter scope).
