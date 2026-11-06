//! Naming entries of an open listing (#36): `create_folder` makes a new folder in the listing's
//! folder (Ctrl+Shift+N), `rename` gives an entry a new name (F2). The UI checks names as the
//! user types (`src/features/rename/name.ts`); core checks them again before anything is named
//! (`jerry_core::name`). The listing's watcher shows the result.

use jerry_core::create::create_folder as make_folder;
use jerry_core::rename::rename as rename_path;
use jerry_core::{ErrorCode, JerryError};
use tauri::{AppHandle, Manager};

use crate::jobs;
use crate::listing::Listings;

/// Makes a folder named `name` in the folder of listing `listing`, or `name (2)`, `name (3)`…
/// when that name is taken, as Explorer does. Returns the name it got, so the UI can find the
/// new entry and open its name for editing. Fails with `InvalidName` for a name Windows doesn't
/// allow, with `NotFound` for a closed listing, and with the folder's path when Windows refuses
/// (`AccessDenied`, `NotFound` for a folder that is gone).
#[tauri::command]
#[specta::specta]
pub async fn create_folder(
    app: AppHandle,
    listing: u32,
    name: String,
) -> Result<String, JerryError> {
    let listings = app.state::<Listings>().inner().clone();
    jobs::background("create folder", move || {
        new_folder(&listings, listing, &name)
    })
    .await
}

/// Renames entry `entry` of listing `listing` to `name`, in the same folder. A change of case
/// only works too. Fails with `InvalidName` for a name Windows doesn't allow (no path) or another
/// item there has (its path), never replacing it; with `InUse` or `AccessDenied` when Windows
/// refuses; with `NotFound` when the entry is gone.
#[tauri::command]
#[specta::specta]
pub async fn rename(
    app: AppHandle,
    listing: u32,
    entry: u32,
    name: String,
) -> Result<(), JerryError> {
    let listings = app.state::<Listings>().inner().clone();
    jobs::background("rename", move || {
        rename_entry(&listings, listing, entry, &name)
    })
    .await
}

fn new_folder(listings: &Listings, listing: u32, name: &str) -> Result<String, JerryError> {
    let folder = listings
        .folder(listing)
        .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
    let created = make_folder(&folder, name)?;
    tracing::info!("folder created");
    Ok(created)
}

fn rename_entry(
    listings: &Listings,
    listing: u32,
    entry: u32,
    name: &str,
) -> Result<(), JerryError> {
    let (path, _, _) = listings
        .entry(listing, entry)
        .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
    rename_path(&path, name)?;
    tracing::info!("renamed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use jerry_core::sort::Sort;

    use super::*;

    /// Opens `folder` as a listing and waits until it has `count` entries.
    fn open(listings: &Listings, folder: &Path, count: u32) -> u32 {
        let id = listings
            .prestart(1, folder, Sort::default())
            .expect("starts");
        let started = Instant::now();
        while listings.entry(id, count - 1).is_none() {
            assert!(started.elapsed() < Duration::from_secs(10), "listed");
            std::thread::sleep(Duration::from_millis(10));
        }
        id
    }

    #[test]
    fn names_entries_of_an_open_listing() {
        let dir = tempfile::tempdir().expect("temp folder");
        std::fs::write(dir.path().join("ნახაზი.dwg"), b"x").expect("write");
        let listings = Listings::default();
        let listing = open(&listings, dir.path(), 1);

        assert_eq!(
            new_folder(&listings, listing, "New folder").as_deref(),
            Ok("New folder")
        );
        assert_eq!(
            new_folder(&listings, listing, "New folder").as_deref(),
            Ok("New folder (2)")
        );
        rename_entry(&listings, listing, 0, "Ნახაზი v2.dwg").expect("renames");
        assert!(dir.path().join("Ნახაზი v2.dwg").exists());

        let invalid = rename_entry(&listings, listing, 0, "a?b").expect_err("invalid");
        assert_eq!(invalid.code, ErrorCode::InvalidName);
        let gone = rename_entry(&listings, listing, 99, "x").expect_err("no such entry");
        assert_eq!(gone.code, ErrorCode::NotFound);
        let closed = new_folder(&listings, listing + 1, "x").expect_err("no such listing");
        assert_eq!(closed.code, ErrorCode::NotFound);
    }
}
