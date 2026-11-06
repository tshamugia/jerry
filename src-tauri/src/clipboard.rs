//! The clipboard (#38): Copy, Cut and Paste of files the way Explorer does them, so files go both
//! ways between Jerry and Explorer (and out to Outlook, Telegram), and Copy path / Copy name as
//! text. Entries come by id from an open listing; paths stay here.
//!
//! The OLE work is `jerry_win::clipboard`'s, on the main thread (`jobs::on_main`, ADR-0002).
//! Paste reads the clipboard there, and the Shell copies or moves on a thread of its own with
//! Windows' progress window (`jerry_win::paste`, v0.1). `clipboard:changed` goes to every window on
//! every clipboard change: Jerry's cut shows dimmed while the clipboard holds it
//! (`clipboard_cut_entries`).

use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::OnceLock;

use jerry_core::delete::takes_system_folder;
use jerry_core::path::plain_path;
use jerry_core::{ErrorCode, JerryError};
use jerry_win::clipboard::{self, Op};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_specta::Event;

use crate::jobs;
use crate::listing::Listings;
use crate::title_bar::hwnd;

/// `FILE_ATTRIBUTE_DIRECTORY`.
const DIRECTORY: u32 = 0x10;

/// Sent to every window on every clipboard change, Jerry's own included.
#[derive(Debug, Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "clipboard:changed")]
pub struct ClipboardChanged {
    /// Jerry's cut is on the clipboard: its rows show dimmed (`clipboard_cut_entries`).
    pub cut: bool,
}

/// What Copy path and Copy name put on the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CopyText {
    /// The full path (Ctrl+Shift+C).
    Path,
    /// The name, with its extension (Ctrl+Alt+C).
    Name,
}

/// Starts telling the windows about clipboard changes. Call from `setup`, on the main thread,
/// after `jerry_win::main_thread::init`.
pub fn watch(app: &AppHandle) {
    let app = app.clone();
    let watched = clipboard::watch(move || {
        let changed = ClipboardChanged {
            cut: clipboard::has_cut(),
        };
        if let Err(error) = changed.emit(&app) {
            tracing::error!(%error, "can't send clipboard:changed");
        }
    });
    if let Err(error) = watched {
        tracing::error!(code = ?error.code, "the clipboard isn't watched: cut rows stay dimmed");
    }
}

/// Copy (Ctrl+C), or Cut (`cut`, Ctrl+X): puts entries `entries` of listing `listing` on the
/// clipboard as Explorer does. `NotFound` for an entry or listing that is gone.
#[tauri::command]
#[specta::specta]
pub async fn clipboard_copy(
    app: AppHandle,
    listing: u32,
    entries: Vec<u32>,
    cut: bool,
) -> Result<(), JerryError> {
    let paths = paths_of(&app.state::<Listings>(), listing, &entries)?;
    let op = if cut { Op::Cut } else { Op::Copy };
    jobs::on_main("clipboard copy", move || clipboard::put_files(&paths, op)).await
}

/// Paste (Ctrl+V): copies the files on the clipboard, or moves them after a Cut, whoever put them
/// there, into the folder of listing `listing`, or into its subfolder `into` (an entry that is a
/// file pastes into the listing's folder). Windows shows the progress and its own messages over
/// this window; a name that is taken gets a new one, never replaced (`jerry_win::paste`). After a
/// cut, the clipboard is emptied as Explorer does. Nothing on the clipboard but text or an image:
/// nothing happens. A cut that would move a system folder fails with `AccessDenied` (data safety
/// rule 6).
#[tauri::command]
#[specta::specta]
pub async fn clipboard_paste(
    app: AppHandle,
    window: WebviewWindow,
    listing: u32,
    into: Option<u32>,
) -> Result<(), JerryError> {
    let destination = destination(&app.state::<Listings>(), listing, into)?;
    let Some(files) = jobs::on_main("clipboard read", clipboard::read_files).await? else {
        tracing::debug!("no files on the clipboard to paste");
        return Ok(());
    };
    let (sources, op) = (files.paths.clone(), files.op);
    let owner = hwnd(&window).ok();
    let pasted = jobs::background("paste", move || {
        if op == Op::Cut {
            refuse_system_folders(&sources)?;
        }
        jerry_win::paste::paste(owner, sources, destination, op)
            .recv()
            .unwrap_or_else(|_| Err(JerryError::internal()))
    })
    .await?;
    tracing::info!(
        files = files.paths.len(),
        ?op,
        aborted = pasted.aborted,
        "pasted"
    );
    if op == Op::Cut && !pasted.aborted {
        jobs::on_main("clipboard finish cut", move || {
            clipboard::finish_cut(&files);
            Ok(())
        })
        .await?;
    }
    Ok(())
}

/// Copy path (Ctrl+Shift+C) or Copy name (Ctrl+Alt+C): puts the full paths, or the names, of
/// entries `entries` of listing `listing` on the clipboard as text, one per line, exactly (paths
/// without the `\\?\` prefix). `NotFound` for an entry or listing that is gone.
#[tauri::command]
#[specta::specta]
pub async fn clipboard_copy_text(
    app: AppHandle,
    listing: u32,
    entries: Vec<u32>,
    text: CopyText,
) -> Result<(), JerryError> {
    let paths = paths_of(&app.state::<Listings>(), listing, &entries)?;
    let joined = lines(&paths, text);
    jobs::on_main("clipboard copy text", move || clipboard::set_text(&joined)).await
}

/// The entries of listing `listing` that Jerry cut, while the clipboard still holds that cut:
/// they show dimmed. Asked again on every `clipboard:changed` and when a listing is in.
#[tauri::command]
#[specta::specta]
pub async fn clipboard_cut_entries(app: AppHandle, listing: u32) -> Result<Vec<u32>, JerryError> {
    if !clipboard::has_cut() {
        return Ok(Vec::new());
    }
    let listings = app.state::<Listings>().inner().clone();
    jobs::background("clipboard cut entries", move || {
        let cut = clipboard::cut_paths();
        let Some(folder) = listings.folder(listing) else {
            return Ok(Vec::new());
        };
        let names: HashSet<Vec<u16>> = cut
            .iter()
            .filter(|path| path.parent() == Some(folder.as_path()))
            .filter_map(|path| path.file_name())
            .map(|name| name.encode_wide().collect())
            .collect();
        Ok(listings.entries_named(listing, &names))
    })
    .await
}

/// At exit, on the main thread: Jerry's files stay on the clipboard once it is gone.
pub fn flush_on_exit() {
    clipboard::flush();
}

/// The exact paths of entries `entries` of listing `listing`.
fn paths_of(
    listings: &Listings,
    listing: u32,
    entries: &[u32],
) -> Result<Vec<PathBuf>, JerryError> {
    if entries.is_empty() {
        return Err(JerryError::new(ErrorCode::NotFound));
    }
    entries
        .iter()
        .map(|&entry| {
            listings
                .entry(listing, entry)
                .map(|(path, _, _)| path)
                .ok_or_else(|| JerryError::new(ErrorCode::NotFound))
        })
        .collect()
}

/// Where Paste goes: subfolder `into` of listing `listing` if it is a folder, else the listing's
/// own folder.
fn destination(
    listings: &Listings,
    listing: u32,
    into: Option<u32>,
) -> Result<PathBuf, JerryError> {
    if let Some((path, attributes, _)) = into.and_then(|entry| listings.entry(listing, entry))
        && attributes & DIRECTORY != 0
    {
        return Ok(path);
    }
    listings
        .folder(listing)
        .ok_or_else(|| JerryError::new(ErrorCode::NotFound))
}

/// Copy path's or Copy name's text: one line per path, Windows' line breaks between them.
fn lines(paths: &[PathBuf], text: CopyText) -> OsString {
    let mut joined = OsString::new();
    for (at, path) in paths.iter().enumerate() {
        if at > 0 {
            joined.push("\r\n");
        }
        match text {
            CopyText::Path => joined.push(plain_path(path)),
            CopyText::Name => joined.push(path.file_name().unwrap_or(path.as_os_str())),
        }
    }
    joined
}

/// A move that takes Windows, Program Files or the Users root with it is refused: Jerry never
/// moves them (data safety rule 6). `AccessDenied` with the first such path.
fn refuse_system_folders(sources: &[PathBuf]) -> Result<(), JerryError> {
    static SYSTEM: OnceLock<Vec<PathBuf>> = OnceLock::new();
    let system = SYSTEM.get_or_init(jerry_win::folders::system_folders);
    match sources
        .iter()
        .find(|source| takes_system_folder(source, system))
    {
        Some(source) => {
            tracing::warn!(path = %source.display(), "refused to move a system folder");
            Err(JerryError::new(ErrorCode::AccessDenied).at(source))
        }
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn copy_path_and_name_give_one_line_per_item() {
        let paths = [
            PathBuf::from(r"\\?\C:\Work\ნახაზი — სართული 1.dwg"),
            PathBuf::from(r"\\?\UNC\server\share\ხარჯთაღრიცხვა.pdf"),
            PathBuf::from(r"C:\Work\Folder"),
        ];
        assert_eq!(
            lines(&paths, CopyText::Path),
            OsString::from(
                "C:\\Work\\ნახაზი — სართული 1.dwg\r\n\\\\server\\share\\ხარჯთაღრიცხვა.pdf\r\nC:\\Work\\Folder"
            )
        );
        assert_eq!(
            lines(&paths, CopyText::Name),
            OsString::from("ნახაზი — სართული 1.dwg\r\nხარჯთაღრიცხვა.pdf\r\nFolder")
        );
        assert_eq!(
            lines(&paths[..1], CopyText::Name),
            OsString::from("ნახაზი — სართული 1.dwg")
        );
    }

    #[test]
    fn moving_a_system_folder_or_what_holds_it_is_refused() {
        let system = jerry_win::folders::system_folders();
        let windows = system.first().expect("Windows' folder");
        let error = refuse_system_folders(&[PathBuf::from(r"D:\Work\a.txt"), windows.clone()])
            .expect_err("refused");
        assert_eq!(error.code, ErrorCode::AccessDenied);
        assert_eq!(error.path.as_deref(), Some(windows.as_path()));
        let root = windows.ancestors().last().unwrap_or(Path::new(r"C:\"));
        assert!(refuse_system_folders(&[root.to_path_buf()]).is_err());
        assert!(refuse_system_folders(&[windows.join("Temp").join("x.txt")]).is_ok());
    }
}
