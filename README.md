# Jerry

A fast, keyboard-first file manager for Windows 10/11. Jerry works on your real Windows files next to Explorer, not instead of it: tabs and dual panes, a background transfer queue, instant filter and deep search, Undo for every file operation, and full drag & drop and clipboard compatibility with Explorer, Outlook, Telegram and other apps.

> **Status:** planning / pre-alpha. Nothing to install yet. Follow the [milestones](https://github.com/tshamugia/jerry/milestones) for progress.

## Stack

- **Core:** Rust (edition 2024) with the official [`windows`](https://crates.io/crates/windows) crate, split into `jerry-core` (file operations, testable without COM) and `jerry-win` (Shell, COM, OLE)
- **App shell:** Tauri 2 on WebView2
- **UI:** React 19, TypeScript (strict), Vite, Tailwind CSS 4, Zustand, TanStack Virtual
- **Storage:** one local SQLite file (rusqlite, WAL)
- **Platform:** Windows 10 22H2+ and Windows 11, x64 only. Runs with normal user rights, no admin, no telemetry.

## Documents

The product documents are written in Georgian. The app's interface is English-only.

- [PRD](docs/PRD.md): what Jerry does (features, shortcuts, release phases)
- [Tech PRD](docs/TECH-PRD.md): how it is built (architecture, IPC, storage, testing, spike)
- [UI design](design/README.md): layout, sizes, colors and states; tokens in [`design/tokens.css`](design/tokens.css)

## Roadmap

Development follows the phases in the PRD. Each phase ends with a version that is usable every day.

| Milestone | Scope |
| --- | --- |
| Setup & Spike | Scaffold, CI, and the eight risky techniques checked before building |
| v0.1 Foundation | Dual-pane window with tabs, listing, sorting, selection, address bar, sidebar, rename, Recycle Bin, clipboard, filter, session, installer and updater |
| v0.2 Transfers | Transfer engine and queue, conflicts, crash recovery, OLE drag & drop, Undo/Redo and history |
| v0.3 Search & Preview | Deep search, Quick Look and preview panel, List and Grid views, thumbnails, Go to folder |
| v1.0 Full | Batch rename, ZIP, command palette, Windows context menu, settings, global hotkey, Open in Jerry, OneDrive, Recycle Bin view, multiple windows |

The ordered task list for each phase lives in its epic issue.
