# UI Redesign Plan: AudioTag Pro — Native macOS HIG Light Mode

## Context

The current app ("Auto Tagger") uses a dark theme with a teal accent, a two-pane layout (file grid + metadata editor), and a custom title bar with emoji icons. The goal is to redesign the entire UI as a pixel-perfect, native macOS desktop application named **"AudioTag Pro"** — strictly following Apple's Human Interface Guidelines in light mode. The redesign requires a shift from dark to light, a three-pane layout, proper macOS window chrome, SF typography, SF Symbols, vibrancy/blur effects, and a premium, airy feel.

## Approach

1. **Electron window config**: Set `titleBarStyle: 'hiddenInset'` and `titleBarOverlay` to use native traffic-light controls while allowing a custom title bar area.
2. **Light mode**: Switch `<html>` from `class="dark"` to `class=""`. Redesign the entire Tailwind color palette for light mode (white/off-white surfaces, charcoal text, subtle grey borders, Apple-blue accent).
3. **Three-pane layout**: Left sidebar (Library/Folders/Playlists) → central FileGrid → right Inspector panel. Use CSS `backdrop-filter: blur()` for sidebar vibrancy.
4. **SF Pro typography**: Set `font-family: -apple-system, 'SF Pro', 'Helvetica Neue', sans-serif` on `<body>`.
5. **Components**:
   - **TitleBar**: Redesigned as macOS title bar area with app title "AudioTag Pro", inline filter capsule, action buttons (toolbar style).
   - **Sidebar**: New component with SF Symbols icons, list items for Library, Folders, Playlists. Frosted glass appearance with backdrop blur.
   - **FileGrid**: macOS-style table view with no vertical borders, alternating white/very-light-grey rows, capsule search bar, Apple-blue selection highlight.
   - **MetadataEditor (Inspector)**: Clean inspector card with rounded input fields (SF Rounded style), prominent album art with rounded border, format details.
   - **SettingsModal**: Updated to light mode styling.

## Files to Modify

| File | Change |
|---|---|
| `frontend/index.html` | Remove `class="dark"`, set light mode meta |
| `frontend/tailwind.config.ts` | Complete color palette overhaul (light mode) |
| `frontend/src/index.css` | New light theme, SF Pro font stack, scrollbar styling, blur utilities |
| `frontend/src/App.tsx` | Three-pane layout, proper TitleBar integration, new Sidebar component |
| `frontend/src/components/TitleBar.tsx` | macOS-native title bar redesign |
| `frontend/src/components/FileGrid.tsx` | macOS table styling, capsule search, light mode |
| `frontend/src/components/MetadataEditor.tsx` | Inspector card redesign, rounded inputs, album art |
| `frontend/src/components/SettingsModal.tsx` | Light mode styling update |
| `frontend/electron/main.ts` | `titleBarStyle: 'hiddenInset'` + `titleBarOverlay` config |

## Files to Create

| File | Description |
|---|---|
| `frontend/src/components/Sidebar.tsx` | New left sidebar with Library, Folders, Playlists |

## Reuse

- **Color palette variables** — Already using Tailwind's `extend.colors`; will re-theme instead of replacing entirely.
- **Component structure** — All existing components (`FileGrid`, `MetadataEditor`, `TitleBar`, `SettingsModal`) keep their logical structure; only styling changes.
- **Electron preload bridge** (`window.api`) — Unchanged.
- **App state & reducer** (`AppState.ts`, `UndoManager.ts`) — Unchanged.

## Steps

- [ ] **1. Update Electron main process** — Set `titleBarStyle: 'hiddenInset'` and `titleBarOverlay` for native traffic-light controls.
- [ ] **2. Re-theme Tailwind config** — Light mode palette:
  - `surface`: `white` / `#f5f5f7` / `#e8e8ed`
  - `text`: `#1d1d1f` (primary), `#6e6e73` (secondary), `#aeaeb2` (muted)
  - `accent`: Apple blue `#007aff` (default), `#0062cc` (dim), `#4da6ff` (light)
  - `border`: `#d2d2d7` / `#c6c6c8`
- [ ] **3. Update index.html** — Remove `class="dark"`, add light mode.
- [ ] **4. Update index.css** — SF Pro font stack, light scrollbar, backdrop-blur utility, selection colors.
- [ ] **5. Create Sidebar component** — Frosted glass sidebar with SF Symbols (using SVG/unicode alternatives since SF Symbols is Apple-native) for 📚 Library, 📁 Folders, 🎵 Playlists. Include album list when library is loaded.
- [ ] **6. Redesign TitleBar** — macOS title bar with:
  - "AudioTag Pro" app title centered
  - Capsule-shaped search bar with 🔍 icon and "Filter files..." placeholder
  - Toolbar-style action buttons (Open Library, Save, Revert, etc.) in NSToolbar style
  - Proper drag region respecting traffic-light spacing
- [ ] **7. Redesign FileGrid** — Light mode macOS table:
  - No vertical borders, only horizontal dividers
  - Alternating `white` / `#f9f9fb` rows
  - Apple-blue (`#007aff` / 15% opacity) selection highlight
  - Charcoal SF Pro typography
  - Capsule search bar moved into TitleBar
  - Column headers with macOS-style ascending/descending indicators
- [ ] **8. Redesign MetadataEditor (Inspector)** — Clean inspector card:
  - Rounded input fields (border-radius: 8px) with subtle grey border
  - Labels in SF Pro small/medium weight
  - Album art as prominent square with `rounded-xl` (12px radius) and thin border
  - Two-column grid for Year/Track row
  - Format details section
- [ ] **9. Update SettingsModal** — Light mode version of the modal with proper styling.
- [ ] **10. Update App.tsx** — Wire up three-pane layout:
  ```
  +------------------+------------------+------------------+
  |        macOS TitleBar (native traffic lights)          |
  +--------+------------------+------------------+----------+
  |Sidebar |   FileGrid       |   Inspector      |
  |(180px) |   (flex-1)       |   (300px)        |
  |        |                  |                  |
  +--------+------------------+------------------+
  ```

## Verification

1. `npm run dev` — Launch the Electron app.
2. Verify native traffic-light controls (red/yellow/green) appear in top-left.
3. Verify three-pane layout renders correctly at various window sizes.
4. Open a music library — verify file grid populates, rows alternate, selection works.
5. Click a track — verify inspector shows metadata, album art loads.
6. Edit a field — verify auto-save and undo work.
7. Resize window — verify layout is responsive within min-width constraints.
8. Toggle sidebar — verify it remains visible and functional.
