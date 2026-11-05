//! The address bar's commands (#33): what typed or pasted text names, completions for it, and a
//! folder's subfolders for a segment's list. The logic is `jerry_core::address`; known folders
//! come from `jerry_win::folders`. Folders go back as tokens, never as text to open by.

use jerry_core::JerryError;
use jerry_core::address::{self, Context};
use jerry_win::folders::{SHORT_NAMES, by_short_name};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::jobs;
use crate::listing::{Crumb, Listings};

/// Completions the address bar shows at most.
const COMPLETIONS: usize = 12;

/// Subfolders a segment's list shows at most; more say so.
const SUBFOLDERS: usize = 500;

/// What the address bar's text names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ResolvedPath {
    /// The folder to open, as a token for `ListingTarget::Folder`.
    pub folder: u32,
    /// A file in it to select (the text named a file), by its display name.
    pub select: Option<String>,
}

/// A folder's subfolders, for a segment's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct Subfolders {
    /// In natural order, at most 500.
    pub folders: Vec<Crumb>,
    /// There are more than these.
    pub more: bool,
}

fn var(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name)
}

/// The folder (and file) `text` names, typed or pasted in the address bar: a path, quoted or as
/// a `file:` URL, with `%VARIABLES%`, a known folder's short name (`Downloads`), a drive alone
/// (`D:`), a share, or relative to folder token `from` (the folder shown). Fails with the error of
/// the likeliest path when nothing is there (`NotFound`, `AccessDenied`, `DeviceGone`), and
/// `InvalidName` for empty text. Nothing opens: the address bar opens the folder it gets.
#[tauri::command]
#[specta::specta]
pub async fn resolve_path(
    app: AppHandle,
    text: String,
    from: Option<u32>,
) -> Result<ResolvedPath, JerryError> {
    let listings = app.state::<Listings>().inner().clone();
    let base = from.and_then(|token| listings.folder_path(token));
    jobs::background("resolve path", move || {
        let context = Context {
            base: base.as_deref(),
            known: &by_short_name,
            var: &var,
        };
        let resolved = address::resolve(&text, &context)?;
        Ok(ResolvedPath {
            folder: listings.folder_token(&resolved.folder),
            select: resolved
                .select
                .map(|name| name.to_string_lossy().into_owned()),
        })
    })
    .await
}

/// Completions for partly typed text, as the text writes them: the subfolders after its last
/// separator that start with what follows it, or known folders' short names and the subfolders
/// of `from` when there is no separator yet. Nothing when the folder can't be read.
#[tauri::command]
#[specta::specta]
pub async fn autocomplete_path(
    app: AppHandle,
    text: String,
    from: Option<u32>,
    show_hidden: bool,
) -> Result<Vec<String>, JerryError> {
    let listings = app.state::<Listings>().inner().clone();
    let base = from.and_then(|token| listings.folder_path(token));
    jobs::background("autocomplete path", move || {
        let context = Context {
            base: base.as_deref(),
            known: &by_short_name,
            var: &var,
        };
        Ok(address::complete(
            &text,
            &context,
            SHORT_NAMES,
            show_hidden,
            COMPLETIONS,
        ))
    })
    .await
}

/// The subfolders of folder token `folder`, in natural order, hidden ones with `show_hidden`:
/// a segment's list. `NotFound` for an unknown token, or the folder's own error.
#[tauri::command]
#[specta::specta]
pub async fn folder_subfolders(
    app: AppHandle,
    folder: u32,
    show_hidden: bool,
) -> Result<Subfolders, JerryError> {
    let listings = app.state::<Listings>().inner().clone();
    let path = listings
        .folder_path(folder)
        .ok_or_else(|| JerryError::new(jerry_core::ErrorCode::NotFound))?;
    jobs::background("subfolders", move || {
        let names = address::subfolders(&path, show_hidden)?;
        let more = names.len() > SUBFOLDERS;
        let folders = names
            .into_iter()
            .take(SUBFOLDERS)
            .map(|name| listings.crumb(&path.join(name)))
            .collect();
        Ok(Subfolders { folders, more })
    })
    .await
}
