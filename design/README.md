# Jerry UI design

The visual source of truth for Jerry's UI. The PRD says **what** Jerry does; this design says **how it looks**. When building a screen, match this design for layout, sizes and colors, and the PRD for behavior and shortcuts.

- **Live canvas (editable source):** https://claude.ai/artifact/SyhnbNFneF1wHCtgTWaB76 ("Jerry — Main Window", Design canvas)
- **Local snapshot:** `design/project/` (pulled 2026-11-01, version `1793480700-8711`). Re-pull after canvas edits so the copy doesn't drift.
- **Tokens for the app:** [`tokens.css`](tokens.css): light/dark/compact as CSS variables (`--jerry-*`), mapped into Tailwind 4 `@theme` in `src/app/styles.css` (Tech PRD: "Design tokens as CSS variables"). The app switches them with attributes on `<html>`: `data-theme`, `data-density`, `data-accent` (Jerry's six colors) and `data-backdrop="mica"` (Mica areas turn transparent); "Match Windows" sets `--jerry-accent-base` / `--jerry-accent-base-dark` from the Windows accent (AccentDark1 / AccentLight2).

The `.dc.html` files are Design Component pages: they render only inside the canvas (they need its `support.js` runtime). Read them as reference markup, not as code to ship. All colors in `Main.dc.html` come from the `c` object in its `renderVals()`, sizes from `d`.

## Artboards

| File | What it shows | Size |
| --- | --- | --- |
| `Main.dc.html` | Main window, light, comfortable rows, details view. The only real artboard: the next four mount it with different props. | 1440×900 |
| `Dark.dc.html` | `Main` with `theme="dark"` | 1440×900 |
| `Compact.dc.html` | `Main` with `density="compact"` | 1440×900 |
| `Palette.dc.html` | `Main` with `overlay="palette"` (command palette open) | 1440×900 |
| `Grid.dc.html` | `Main` with `view="grid"` (thumbnail grid in the left pane) | 1440×900 |
| `Settings.dc.html` | Settings window, Appearance page | 960×640 |

`Main` props: `theme` light|dark · `density` comfortable|compact · `view` details|grid · `overlay` none|palette · `accent` color (#005FB8, #0F7B74, #5B5FC7, #4A5560).

## Look

Windows 11 Fluent: Mica window background, light content panes, 4 px control radius, 8 px flyout radius, Segoe UI Variable, monochrome line icons (16 px, 1.2 stroke) with colored file-type icons. Accent is used **only** for selection, progress, focus and the active-pane bar.

- **Font:** `'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif`. Body 13 px (12 px compact), line-height 1.35. Secondary text and table cells 12 px, captions/kbd 11 px, preview title 14/600, settings page title 28/600, palette input 15 px.
- **Numbers** (dates, sizes, progress): `font-variant-numeric: tabular-nums`. Date format `DD.MM.YYYY HH:mm`.
- **Icons:** inline stroke SVG on a 16×16 viewBox. Folder = two-tone yellow (#D9A21B back, #FFC83D front). Documents = white page with a colored letter badge: DWG `A` #A4262C, PDF `P` #C4314B, XLSX `X` #107C41. Images = blue photo glyph.

## Main window layout (1440×900)

Top to bottom:

1. **Title bar, 40 px.** App mark + "Jerry" over the sidebar (232 px). Then the tab strip **split at the pane divider**: left-pane tabs (460 px) | 1 px divider | right-pane tabs (459 px). Tabs 32 px tall, active tab 168 px wide on the `bar` color, inactive 140 px, top radius 8 px, 16 px icon + 12 px label + 24 px close button. Pinned tab = 40 px icon-only with a pin glyph. `+` new-tab button 28 px. Caption buttons 46×40 at the right.
2. **Body**, three columns:
   - **Sidebar, 232 px**, on Mica. Header row 40 px (collapse sidebar, settings). Sections: Favorites (collapsible, with sub-groups "Insta" and "TSGroup" as 11 px captions), Recent, places (Desktop, Downloads, Documents, Pictures, OneDrive), Drives (name, free space, 4 px usage bar; bar and text turn red at ≥90 % used), Network, Recycle Bin. Items 28 px (24 compact), selected item gets `navSel` fill + 3×16 px accent pill on the left edge.
   - **Two panes, 460 + 1 + 459 px.** Each pane: toolbar block on `bar` (padding 6/8/8): row 1 = Back, Forward, Up (30 px buttons), breadcrumb address bar (30 px, current segment bold, refresh at the end), command palette button; row 2 = quick filter field (28 px, `Ctrl+F` kbd hint) + Details/List/Grid segmented control. Under the toolbar a **2 px bar: accent on the active pane, `bar` color on the inactive one.** Column header 28 px (Name, Date modified 104 px, Type 84 px, Size 60 px right-aligned). Rows 30 px (24 compact), inset 4 px, radius 4 px.
   - **Preview panel, 288 px**, on `bar`. Header 44 px ("Preview · 1 of 3 selected", open-in-default-app, hide preview). 196 px preview well, page pager, file icon + name (14/600) + type·size, **Open** (accent) and **Open with ▾** buttons, then a metadata list (84 px label column): Size, Pages, Title, Author, Producer, Created, Modified, Location, Tags, Checksum.
3. **Transfer panel, 56 px** (shown once a transfer has run for 1 s): copy icon, "Copying 214 items to …" + current file and "item N of M", progress (bytes text, %, 4 px bar), speed + time left, Pause, Cancel, "Queue · 2 ▴". Built in #52, beyond the canvas frame:
   - A ✕ (28 px, "Hide transfers") after "Queue" closes the panel while the transfers go on; a new transfer brings it back. Closed, the **status bar** shows them before the drive's free space: a 16 px copy icon, the percent of all transfers together and a 48 × 4 px bar ("Paused" when none runs); a click opens the panel.
   - **"Queue · N"** opens the other jobs above the row (at most 240 px, then it scrolls): a 40 px line "4 transfers" with Pause all, Resume all and Cancel all as the panel's buttons, then a 44 px line per job: a grip (drag to reorder, or Space and the arrow keys on it), a 16 px type icon in `text2`, "Copying 3 items to Archive" and a 12 px `text2` line (bytes · speed · time left, or "Queued" / "Paused" · files · bytes), the percent over a 160 px bar, and Pause or Resume and Cancel as 28 px icon buttons.
   - A job that ended with failed items, or couldn't start, leaves a **summary** in the row: a 20 px `red` "!" icon, "3 files could not be copied" over "211 files copied to Archive", then Show list (24 px lines above the row: the item's name, why in `text2`), Retry and Dismiss. Closed, the status bar shows its line in `red`.
   - Jobs an earlier Jerry didn't finish (#50) say "Not finished" with Resume and Discard; a job whose drive went away says "Waiting for the drive", Resume disabled until it is back.
4. **Status bar, 28 px:** item count | selection count + size … drive free space | density toggle.

Overlays:

- **Toast** (bottom right, 336 px, 16 px from the right, 96 px from the bottom or 12 px above the transfer panel when its queue makes it taller, radius 8, `shadow`): green check, "Copied 12 files · **Undo**", destination path, dismiss.
- **Update notice** (no canvas frame yet): the toast's card, above the toasts, staying until answered. Bold "Jerry X.Y.Z is available", up to four changes in secondary text, `Later` and `Update` as 30 px dialog buttons. While installing, a 4 px accent bar on `track` instead of the buttons. While transfers run, Update first turns the card into a warning (#52): bold "2 transfers are still running", "Jerry X.Y.Z is available" in secondary text, a 12 px `text2` line saying Jerry closes and they can be resumed after, `Not now` and `Update anyway` (semibold).
- **Unfinished transfers notice** (no canvas frame yet, #50): the update notice's card, above it, staying until answered. Bold "Jerry closed before a transfer finished" (or "… 2 transfers finished"), then per job (up to three, then "and N more") a 12 px line "Copying 3 items to Archive" and a secondary line "1 of 3 files · 8 MB of 24 MB" (`tabular-nums`), and `Discard` and `Resume` as 30 px dialog buttons, Resume semibold.
- **Conflict window** (no canvas frame yet, #51): the palette's dialog (scrim, 640 px at top 96 px, 52 px header with the title and an `Esc` kbd), one question at a time, for the first job in queue order that asks. Title "A file with this name is already there" ("A folder …" when a folder is there; "This file can't go to the Recycle Bin" when Replace found none). Body: "ნახაზი.dwg is already in Archive.", then, without a Recycle Bin, a 12 px `red` line saying Replace deletes for good. Two cards side by side (4 px radius, `stroke` border): an 11 px `text3` caption ("Copying from Projects" / "Already in Archive"), a 32 px type icon (thumbnails come with #67), the name, the size in 12 px `text2` ("Folder" for a folder) and the date modified in 12 px `tabular-nums`; the newer one's date is semibold `text` with "· Newer". Then the choices the core offers, as full-width buttons (`ctrl` fill and border, 4 px radius): the label semibold, a 12 px `text2` line on what it does, its key as a kbd chip at the end (R Replace, S Skip, K Keep both, N Replace if newer, M Merge). Footer under a `stroke` line: an "Apply to all" checkbox with an `A` kbd, and `Cancel transfer` as a 30 px dialog button. A letter answers, A ticks Apply to all, ↑↓ move between the choices, Enter answers the focused one, Esc cancels the transfer. The focus starts on the first choice, on Skip where Replace would delete for good.
- **Command palette** (`Ctrl+Shift+P`): scrim over the window, dialog 640 px wide at top 96 px, centered. 52 px input row with mode chip ("Commands") and `Esc` kbd. Group caption "Commands · 6 matches", 38 px result rows (icon, matched text bold, description in `text3`, kbd chips), selected row = `sel` fill + accent pill. "Recently used" group. 36 px footer: `↑↓ Navigate`, `↵ Run`, `Tab Switch scope`, and prefixes `>` commands, `@` folders, `#` tags.

## States

| State | Look |
| --- | --- |
| Hover row | `hover` fill |
| Selected, active pane | `sel` (accent mixed 14 % into white; 20 % into #1C1C1C in dark) |
| Selected, inactive pane | `selInactive` (neutral gray) |
| Keyboard focus | 2 px `focus` outline, offset −2 px (black in light, white in dark) |
| Disabled button | icon in `disabled` |
| Hidden/system files | shown dimmed when "Show hidden" is on |
| OneDrive status (right after the name) | cloud outline = online-only · green check outline = on this device · solid green check = always keep · accent sync arrows = sync pending |

## Grid view

Left pane in `view="grid"`: 4 columns, 124 px rows, 4 px gap, 10/8 px padding. Each cell: 92×70 thumbnail area (folders 64×54 icon, drawings as A-series sheet thumbnails, spreadsheets as a sheet preview, photos as image placeholders) + up to two lines of 12 px centered name.

## Settings window (960×640)

Own title bar (40 px, minimize + close). Left nav 260 px with "Find a setting" search and sections: General, Appearance, Panes & tabs, Keyboard, File operations, Preview, Integrations, About (selected = `navSel` + accent pill). Content on #F9F9F9 with a 28 px page title and setting cards (white, 1 px #E5E5E5, radius 4, padding 14/16; icon, label + 12 px description, control on the right). Appearance shows: App theme (Use system setting), Accent color (Match Windows + 6 swatches: Blue #005FB8, Teal #0F7B74, Indigo #5B5FC7, Graphite #4A5560, Plum #8A3B7A, Rust #A4432A), Row density (Comfortable · 30 px / Compact · 24 px with a live sample of hover, selected and focus rows), Mica background (on), Show folders first (on), Show hidden and system files (off). Toggles are 40×20 Windows switches.

## Density

| | Comfortable (default) | Compact |
| --- | --- | --- |
| File row | 30 px | 24 px |
| Sidebar item | 28 px | 24 px |
| Base font | 13 px | 12 px |
| Drive item padding | 6 px | 4 px |

## Design ↔ PRD differences (settled 2026-11-01)

The PRD decides behavior and shortcuts; the design decides looks. Where the mockup disagrees with the PRD, build what the PRD says.

Added to the PRD from the design:

- Shortcuts: `Shift+F5` / `Shift+F6` Copy to… / Move to…, `Ctrl+Alt+C` copy name, `Ctrl+Alt+D` toggle density, `Ctrl+F9` Compare panes.
- **Compare panes** (v1.0): highlights newer and missing files across the two panes and can select them; the user copies with `F5`. Jerry never copies or syncs on its own.
- Command palette prefixes: `>` commands (default), `@` folders. `Tab` switches scope.
- Accent default = Windows accent ("Match Windows"), or one of the 6 Jerry colors. Mica background setting, on by default.
- Preview metadata includes document title, author and the file's own Windows Tags property (read-only).

Where the mockup is wrong and the build follows the PRD:

1. **View switch** is `Ctrl+Shift+1/2/3`, not the mockup's `Ctrl+1/2/3` (`Ctrl+1…9` switches tabs).
2. **Refresh** tooltip should read "Refresh (Ctrl+R)"; `F5` copies to the other pane.
3. **No tags:** drop the palette's `#` tags prefix. The preview's Tags row shows only the file's own Windows property.
4. **No "Copy newer files to other pane"** (`Ctrl+Shift+F5`): sync is out of scope. Compare panes + `F5` covers it.
5. **Font size** (PRD Appearance setting, default 100 %) still needs a control on the Settings screen.
6. **Checksum** row in the preview metadata isn't in the PRD; compute on demand or leave out.

Where the mockup breaks its own rules, and the build follows the rule:

1. **App mark** in the title bar is filled with Jerry blue (`--jerry-brand`, the app icon's tile), not the accent: the accent is only for selection, progress, focus and the active-pane bar.
2. **Caption buttons** use Windows' own glyphs (Segoe Fluent Icons, Segoe MDL2 Assets on Windows 10) and Windows 11's hover colors (`--jerry-subtle-hover`, `--jerry-caption-close`), which the canvas doesn't draw.
