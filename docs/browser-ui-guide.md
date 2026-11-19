# Soundrobe browser workspace

## Why split panes

We chose the split-pane layout so users can browse an album, select another file, and check its tags without leaving the track list. In the approved *Tidal Atlas* screenshot, the library rail locates the album, the highlighted “Soundings” row identifies the file, and the adjacent inspector confirms its save. A persistent inspector leaves less width for comparing tag columns, so the default table puts title and filename together and reserves space for tag status and duration. Users can add metadata columns in workspace settings. [Workspace](../src/web/WorkspaceView.tsx) · [Track table](../src/web/TrackTable.tsx) · [Workspace settings](../src/web/WorkspaceSettingsDrawer.tsx)

## Layout and interaction

- **Desktop.** Choose a mounted library when several are available, then browse or filter the folder tree by artist or album. The center pane shows the current folder breadcrumb, album details, and tracks. The inspector stays beside the list by default. Before selection it prompts the user to choose a track. [Library tree](../src/web/DirectoryTree.tsx) · [Workspace](../src/web/WorkspaceView.tsx)
- **Mobile.** At 900px and below, the folder browser closes into a disclosure and the list and inspector stack. The approved mobile screenshot shows the intended reading order from selected row to editor. In the implementation, selecting a file scrolls the inspector into view as needed, and the Save and Cancel controls stay reachable as the workspace scrolls. Extra table columns scroll inside the table. [Workspace styles](../src/web/web.css)
- **Selection and drafts.** Activate the track by pointer, Enter, or Space. Keep its selected row distinct from hover and identify the same file in the inspector by filename and track position when available. Drafts are held per file in the workspace session, so switching tracks and coming back restores the edits. Cancel discards the draft for that file. [Track table](../src/web/TrackTable.tsx) · [Tag inspector](../src/web/TrackDetailEditor.tsx)
- **Save and retry.** Keep the file and its fields visible during a save and block duplicate submissions. On success, show **Tags saved to file** and update the row from writer readback. On failure, keep the selection and draft, show the reason in the inspector, and offer **Retry save**. **Needs attention** includes failed writes and files missing title, artist, album, year, or genre. [Workspace](../src/web/WorkspaceView.tsx) · [Tag inspector](../src/web/TrackDetailEditor.tsx)
- **Empty states.** An unmounted library directs users to mount a music folder and check again. An empty mounted library or folder directs them to add audio and refresh or choose another folder. Offer to clear a folder or track filter with no matches. If a provider returns no releases, broaden the query. If a results filter hides the releases, clear the filter. [Workspace](../src/web/WorkspaceView.tsx) · [Release search](../src/web/SearchTool.tsx)
- **Provider mapping.** Open **Find album metadata** for an album, search MusicBrainz or Discogs, and choose a release. Review each local-to-remote match and the proposed values. Users can correct a match, edit write values, or skip a file before explicitly writing the mapped tracks. A failed search offers a retry and preserves the query and track draft. A failed write keeps the mapping open and reloads the album tags so users can check any partial changes. [Album tools](../src/web/TrackTools.tsx) · [Release search](../src/web/SearchTool.tsx)

## Prototype and implementation

The prototype used a sample library, a preselected track, and simulated outcomes. The implementation starts with mounted folders and waits for track selection. **Find album metadata** opens a drawer from the album header. The inspector shows a file-level save confirmation and the values read back into its fields. The prototype showed **Find release** as an inspector tab and named the changed field in its sample confirmation.

## Music Tag Web V2 parity

Use the reference-derived surfaces, palette, type scale, radii, and control geometry from `tokens.css` in `web.css`. Keep measured colours in `oklch()` and use `columns.ts` for widths. The parity record documents the denser workspace rows, solid selected-row fill, accessible muted and status text, and split-pane geometry. Record measured before and after values there when an approved change diverges from the reference. [Tokens](../src/web/tokens.css) · [Styles](../src/web/web.css) · [Column widths](../src/web/columns.ts) · [Parity record](mtw-parity.md)

## Verification checklist

- [ ] At **1920×1080**, check that the library, album list, and inspector stay visible together. At **390×844**, check the disclosure, selected row, scroll to the inspector, and sticky actions. Keep horizontal overflow inside the table.
- [ ] With keyboard and pointer, verify hover, selection, visible focus, and disabled states. Read computed styles and calculate contrast against rendered backgrounds. Require 4.5:1 for body text and 3:1 for large text and icons.
- [ ] Exercise empty library, no matches, loading, successful save, and retries. Switch files with a draft and confirm it survives. Review, correct, and skip mappings before a write. Verify saved values after refresh using copied or synthetic media.
- [ ] Run the relevant browser workspace and token tests. [Workspace tests](../test/components/WorkspaceView.test.tsx) · [Release review tests](../test/components/WebReviewTools.test.tsx) · [State token tests](../test/components/WebStateTokens.test.ts)
