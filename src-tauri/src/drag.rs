//! Dragging files out of Jerry (#58): the file list notices a press on an item moving past
//! Windows' drag threshold (`drag_threshold`) and calls `drag_start` with the entries; Windows runs
//! the drag (`jerry_win::drag`, on the main thread through Jerry's own window, ADR-0002). The
//! target copies or moves the files, as from Explorer; a moved file leaves the list through the
//! folder's watcher. Entries come by id from an open listing; paths stay here.

use jerry_core::JerryError;
use jerry_win::drag::{self, DragOutcome};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::clipboard::paths_of;
use crate::jobs;
use crate::listing::Listings;
use crate::title_bar::hwnd;

/// Windows' drag threshold in CSS px: a press that moves farther than this with the button down
/// is a drag, not a click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
pub struct DragThreshold {
    pub x: u32,
    pub y: u32,
}

/// Windows' drag threshold (`SM_CXDRAG`, `SM_CYDRAG`), in CSS px.
#[tauri::command]
#[specta::specta]
pub async fn drag_threshold() -> DragThreshold {
    let (x, y) = drag::threshold();
    DragThreshold { x, y }
}

/// Drags entries `entries` of listing `listing` out of Jerry, with Windows' drag image, and
/// resolves once the user drops or cancels (the page gets no `pointerup` after a drag). Call it
/// while the mouse button is still down, once the pointer moved past `drag_threshold`; if the
/// button is up by the time Windows gets to it, nothing happens. `NotFound` for an entry, a
/// listing or a file that is gone.
#[tauri::command]
#[specta::specta]
pub async fn drag_start(
    app: AppHandle,
    window: WebviewWindow,
    listing: u32,
    entries: Vec<u32>,
) -> Result<(), JerryError> {
    let paths = paths_of(&app.state::<Listings>(), listing, &entries)?;
    let owner = hwnd(&window)?;
    let outcome = jobs::on_main("drag", move || drag::drag_files(owner, &paths)).await?;
    if outcome == DragOutcome::NotStarted {
        tracing::debug!("the button came up before the drag started");
    }
    Ok(())
}
