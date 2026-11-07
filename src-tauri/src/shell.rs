//! Shell commands on what the lists show: opening files in their programs, showing them in
//! Explorer, opening a terminal in a folder and copying a folder's path. Paths come from core (an
//! entry of an open listing, a folder token), never from the UI's text.

use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use jerry_core::path::plain_path;
use jerry_core::{ErrorCode, JerryError};
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::jobs;
use crate::listing::Listings;
use crate::title_bar::hwnd;

/// How long `open_entries` waits for the files to start opening. A program that answers by DDE
/// (AutoCAD with a DWG, say) can take longer; it goes on opening, and the command returns.
const OPEN_WAIT: Duration = Duration::from_secs(5);

/// `FILE_ATTRIBUTE_DIRECTORY`.
const DIRECTORY: u32 = 0x10;

/// Opens entries `entries` of listing `listing` in their programs, as Explorer does (Enter, a
/// double click): the default verb, with Windows' own messages ("How do you want to open this
/// file?", an error box) over this window. Folders open in Jerry, so they are skipped here.
/// Fails with the first file's error, or `NotFound` for an entry or listing that is gone; a
/// prompt the user cancelled isn't an error.
#[tauri::command]
#[specta::specta]
pub async fn open_entries(
    app: AppHandle,
    window: WebviewWindow,
    listing: u32,
    entries: Vec<u32>,
) -> Result<(), JerryError> {
    let listings = app.state::<Listings>().inner().clone();
    let mut files = Vec::with_capacity(entries.len());
    for entry in entries {
        let (path, attributes, _) = listings
            .entry(listing, entry)
            .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
        if attributes & DIRECTORY == 0 {
            files.push(path);
        }
    }
    if files.is_empty() {
        return Ok(());
    }
    let owner = hwnd(&window).ok();
    jobs::background("open files", move || {
        match jerry_win::open::open_files(owner, files).recv_timeout(OPEN_WAIT) {
            Ok(outcome) => outcome,
            Err(RecvTimeoutError::Timeout) => {
                tracing::info!("files still opening; not waiting");
                Ok(())
            }
            Err(RecvTimeoutError::Disconnected) => Err(JerryError::internal()),
        }
    })
    .await
}

/// Shows entries `entries` of listing `listing` selected in an Explorer window on the listing's
/// folder: the context menu's "Show in Explorer". Waits for Explorer to take it, at most a few
/// seconds. `NotFound` for an entry or listing that is gone.
#[tauri::command]
#[specta::specta]
pub async fn show_in_explorer(
    app: AppHandle,
    listing: u32,
    entries: Vec<u32>,
) -> Result<(), JerryError> {
    let listings = app.state::<Listings>().inner().clone();
    let not_found = || JerryError::new(ErrorCode::NotFound);
    let folder = listings.folder(listing).ok_or_else(not_found)?;
    let items = entries
        .into_iter()
        .map(|entry| listings.entry(listing, entry).map(|(path, ..)| path))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(not_found)?;
    jobs::background(
        "show in Explorer",
        move || match jerry_win::open::show_in_explorer(folder, items).recv_timeout(OPEN_WAIT) {
            Ok(outcome) => outcome,
            Err(RecvTimeoutError::Timeout) => {
                tracing::info!("Explorer still opening; not waiting");
                Ok(())
            }
            Err(RecvTimeoutError::Disconnected) => Err(JerryError::internal()),
        },
    )
    .await
}

/// Opens a terminal in the folder listing `listing` shows, or in its subfolder `entry`: Windows
/// Terminal, or Windows PowerShell where it isn't installed. `NotFound` for an entry or listing
/// that is gone.
#[tauri::command]
#[specta::specta]
pub async fn open_terminal(
    app: AppHandle,
    listing: u32,
    entry: Option<u32>,
) -> Result<(), JerryError> {
    let listings = app.state::<Listings>().inner().clone();
    let folder = match entry {
        None => listings.folder(listing),
        Some(entry) => match listings.entry(listing, entry) {
            Some((path, attributes, _)) if attributes & DIRECTORY != 0 => Some(path),
            // The UI offers it on folders only.
            Some(_) => return Err(JerryError::internal()),
            None => None,
        },
    }
    .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
    jobs::background("open terminal", move || {
        jerry_win::open::open_terminal(&folder)
    })
    .await
}

/// Puts the path of folder token `folder` (`ListingOpened::folder`) on the clipboard as text,
/// exactly, without the `\\?\` prefix: a tab's "Copy path". `NotFound` for an unknown token.
#[tauri::command]
#[specta::specta]
pub async fn folder_copy_path(app: AppHandle, folder: u32) -> Result<(), JerryError> {
    let path = app
        .state::<Listings>()
        .folder_path(folder)
        .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
    let text = plain_path(&path).into_os_string();
    jobs::on_main("copy path", move || jerry_win::clipboard::set_text(&text)).await
}
