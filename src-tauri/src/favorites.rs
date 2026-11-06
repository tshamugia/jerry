//! The sidebar's favorites and recent folders (#35): the `favorites_*` commands,
//! `recent_folders_list`, and `favorites:changed` and `recent-folders:changed` to every window.
//! The favorites live in `jerry.db` (`jerry_core::storage`); `recent_folders` is written by
//! listings, when the user went to a folder and it was read to the end.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use jerry_core::error::display_path;
use jerry_core::storage::Storage;
use jerry_core::{ErrorCode, JerryError, favorites};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::jobs;
use crate::listing::{Listings, crumb_name};
use crate::storage::Db;

/// How many recent folders the sidebar shows (PRD).
pub const RECENT: usize = 10;

/// `FILE_ATTRIBUTE_DIRECTORY`.
const DIRECTORY: u32 = 0x10;

/// The favorites, in sidebar order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct FavoriteList {
    pub groups: Vec<FavoriteGroup>,
    /// The top level first, then group by group.
    pub favorites: Vec<Favorite>,
}

/// A favorite folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct Favorite {
    pub id: u32,
    /// The folder as display text.
    pub path: String,
    /// The name the sidebar shows: the one the user gave it, or the folder's own.
    pub name: String,
    /// The user named it: `name` isn't the folder's own.
    pub renamed: bool,
    /// Its group; none: the top level.
    pub group: Option<u32>,
}

/// A group of favorites.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct FavoriteGroup {
    pub id: u32,
    pub name: String,
}

/// A recently opened folder: its path as display text, and its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct RecentFolder {
    pub path: String,
    pub name: String,
}

/// Sent to every window when the favorites change: one added, renamed, moved or removed, a group
/// added, renamed, moved or removed.
#[derive(Debug, Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "favorites:changed")]
pub struct FavoritesChanged(pub FavoriteList);

/// Sent to every window when the latest recent folders change.
#[derive(Debug, Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "recent-folders:changed")]
pub struct RecentFoldersChanged(pub Vec<RecentFolder>);

/// Notes the folders the user went to in `recent_folders`, and tells every window when the
/// latest [`RECENT`] change. Clones share it.
#[derive(Clone)]
pub struct Recent(Arc<RecentInner>);

struct RecentInner {
    db: Db,
    /// Set once the app is up; visits before that (the session's first folders) aren't sent.
    app: OnceLock<AppHandle>,
    /// What windows last heard, so a visit that changes nothing isn't sent.
    sent: Mutex<Vec<RecentFolder>>,
}

impl Recent {
    pub fn new(db: Db) -> Self {
        Self(Arc::new(RecentInner {
            db,
            app: OnceLock::new(),
            sent: Mutex::new(Vec::new()),
        }))
    }

    /// From now on, changes go to the app's windows.
    pub fn attach(&self, app: &AppHandle) {
        let _ = self.0.app.set(app.clone());
    }

    /// Notes that the user went to `folder` (an exact path). Blocks: listings call it from their
    /// job.
    pub fn visit(&self, folder: &Path) {
        self.0.db.visit_folder(folder);
        let Some(app) = self.0.app.get() else {
            return;
        };
        // Read and sent under the database's lock, so windows hear the lists in order.
        let sent = self.0.db.with(|storage| {
            let latest = recent(storage)?;
            let mut sent = self.0.sent.lock().unwrap_or_else(PoisonError::into_inner);
            if *sent != latest {
                *sent = latest.clone();
                if let Err(error) = RecentFoldersChanged(latest).emit(app) {
                    tracing::error!(%error, "can't send recent-folders:changed");
                }
            }
            Ok(())
        });
        if let Err(error) = sent {
            tracing::warn!(code = ?error.code, "couldn't read the recent folders");
        }
    }
}

/// The favorites; `favorites:changed` brings every later list.
#[tauri::command]
#[specta::specta]
pub async fn favorites_list(db: State<'_, Db>) -> Result<FavoriteList, JerryError> {
    let db = Db::clone(&db);
    jobs::background("favorites", move || db.with(|storage| list(storage))).await
}

/// Adds a folder to the favorites, at the end of the top level: the folder the open listing
/// `listing` shows, or its subfolder `entry`. Returns the favorite's id; a folder that is a
/// favorite already stays where it is. Every window gets `favorites:changed`.
#[tauri::command]
#[specta::specta]
pub async fn favorites_add(
    app: AppHandle,
    db: State<'_, Db>,
    listings: State<'_, Listings>,
    listing: u32,
    entry: Option<u32>,
) -> Result<u32, JerryError> {
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
    let db = Db::clone(&db);
    jobs::background("add favorite", move || {
        db.with(|storage| {
            let (id, added) = storage.add_favorite(&folder)?;
            if added {
                announce(&app, storage)?;
            }
            Ok(id)
        })
    })
    .await
}

/// Names the favorite `id` `name` in the sidebar; empty or none: its folder's own name again.
/// The folder isn't renamed. Every window gets `favorites:changed`.
#[tauri::command]
#[specta::specta]
pub async fn favorites_rename(
    app: AppHandle,
    db: State<'_, Db>,
    id: u32,
    name: Option<String>,
) -> Result<(), JerryError> {
    change(app, &db, "rename favorite", move |storage| {
        storage.rename_favorite(id, name.as_deref())
    })
    .await
}

/// Removes the favorite `id`; its folder is untouched. Every window gets `favorites:changed`.
#[tauri::command]
#[specta::specta]
pub async fn favorites_remove(
    app: AppHandle,
    db: State<'_, Db>,
    id: u32,
) -> Result<(), JerryError> {
    change(app, &db, "remove favorite", move |storage| {
        storage.remove_favorite(id)
    })
    .await
}

/// Moves the favorite `id` into `group` (none: the top level), at `index` there. `NotFound` for a
/// favorite or group that is gone. Every window gets `favorites:changed`.
#[tauri::command]
#[specta::specta]
pub async fn favorites_move(
    app: AppHandle,
    db: State<'_, Db>,
    id: u32,
    group: Option<u32>,
    index: u32,
) -> Result<(), JerryError> {
    change(app, &db, "move favorite", move |storage| {
        storage.move_favorite(id, group, index as usize)
    })
    .await
}

/// Adds a group named `name` after the others; returns its id. `InvalidName` for an empty name.
/// Every window gets `favorites:changed`.
#[tauri::command]
#[specta::specta]
pub async fn favorites_group_add(
    app: AppHandle,
    db: State<'_, Db>,
    name: String,
) -> Result<u32, JerryError> {
    let db = Db::clone(&db);
    jobs::background("add favorite group", move || {
        db.with(|storage| {
            let id = storage.add_favorite_group(&name)?;
            announce(&app, storage)?;
            Ok(id)
        })
    })
    .await
}

/// Renames the group `id`. `InvalidName` for an empty name. Every window gets
/// `favorites:changed`.
#[tauri::command]
#[specta::specta]
pub async fn favorites_group_rename(
    app: AppHandle,
    db: State<'_, Db>,
    id: u32,
    name: String,
) -> Result<(), JerryError> {
    change(app, &db, "rename favorite group", move |storage| {
        storage.rename_favorite_group(id, &name)
    })
    .await
}

/// Removes the group `id`; its favorites go to the end of the top level. Every window gets
/// `favorites:changed`.
#[tauri::command]
#[specta::specta]
pub async fn favorites_group_remove(
    app: AppHandle,
    db: State<'_, Db>,
    id: u32,
) -> Result<(), JerryError> {
    change(app, &db, "remove favorite group", move |storage| {
        storage.remove_favorite_group(id)
    })
    .await
}

/// Moves the group `id`, with its favorites, to `index` among the groups. Every window gets
/// `favorites:changed`.
#[tauri::command]
#[specta::specta]
pub async fn favorites_group_move(
    app: AppHandle,
    db: State<'_, Db>,
    id: u32,
    index: u32,
) -> Result<(), JerryError> {
    change(app, &db, "move favorite group", move |storage| {
        storage.move_favorite_group(id, index as usize)
    })
    .await
}

/// Why the favorite `id` can't be opened, if its folder isn't there: `NotFound` (deleted, moved
/// or renamed) or `DeviceGone` (its drive or network location isn't connected), on its path.
/// Null when it is there. Doesn't read the folder; a share that doesn't answer takes Windows' own
/// timeout, so the UI checks each favorite on its own.
#[tauri::command]
#[specta::specta]
pub async fn favorites_check(db: State<'_, Db>, id: u32) -> Result<Option<JerryError>, JerryError> {
    let db = Db::clone(&db);
    jobs::background("check favorite", move || {
        let favorite = db
            .with(|storage| storage.favorite(id))?
            .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
        // Outside the database's lock: this may wait on the network.
        Ok(favorites::problem(&favorite.path))
    })
    .await
}

/// The latest recently opened folders, at most [`RECENT`]; `recent-folders:changed` brings every
/// later list.
#[tauri::command]
#[specta::specta]
pub async fn recent_folders_list(db: State<'_, Db>) -> Result<Vec<RecentFolder>, JerryError> {
    let db = Db::clone(&db);
    jobs::background("recent folders", move || db.with(|storage| recent(storage))).await
}

/// Runs `job` on the database and, if it changed anything, sends `favorites:changed`.
async fn change(
    app: AppHandle,
    db: &Db,
    name: &'static str,
    job: impl FnOnce(&mut Storage) -> Result<bool, JerryError> + Send + 'static,
) -> Result<(), JerryError> {
    let db = Db::clone(db);
    jobs::background(name, move || {
        db.with(|storage| {
            if job(storage)? {
                announce(&app, storage)?;
            }
            Ok(())
        })
    })
    .await
}

fn list(storage: &Storage) -> Result<FavoriteList, JerryError> {
    let all = storage.favorites()?;
    Ok(FavoriteList {
        groups: all
            .groups
            .into_iter()
            .map(|group| FavoriteGroup {
                id: group.id,
                name: group.name,
            })
            .collect(),
        favorites: all
            .items
            .into_iter()
            .map(|item| Favorite {
                id: item.id,
                path: display_path(&item.path),
                renamed: item.name.is_some(),
                name: item.name.unwrap_or_else(|| crumb_name(&item.path)),
                group: item.group,
            })
            .collect(),
    })
}

fn recent(storage: &Storage) -> Result<Vec<RecentFolder>, JerryError> {
    Ok(storage
        .recent_folders(RECENT)?
        .into_iter()
        .map(|folder| RecentFolder {
            path: display_path(&folder.path),
            name: crumb_name(&folder.path),
        })
        .collect())
}

/// Sends the favorites to every window. Still under the database's lock, so windows hear the
/// changes in the order they were saved.
fn announce(app: &AppHandle, storage: &Storage) -> Result<(), JerryError> {
    if let Err(error) = FavoritesChanged(list(storage)?).emit(app) {
        tracing::error!(%error, "can't send favorites:changed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn favorites_show_their_name_and_display_path() {
        let mut storage = Storage::in_memory().expect("opens");
        let (drawings, _) = storage
            .add_favorite(Path::new(r"\\?\D:\Projects\2026\BMW\Drawings"))
            .expect("adds");
        let (share, _) = storage
            .add_favorite(Path::new(r"\\?\UNC\INSTA-NAS01\Projects\ნახაზები"))
            .expect("adds");
        let (root, _) = storage.add_favorite(Path::new(r"\\?\E:\")).expect("adds");
        let insta = storage.add_favorite_group("Insta").expect("group");
        storage
            .rename_favorite(drawings, Some("BMW ნახაზები"))
            .expect("renames");
        storage.move_favorite(share, Some(insta), 0).expect("moves");

        assert_eq!(
            list(&storage),
            Ok(FavoriteList {
                groups: vec![FavoriteGroup {
                    id: insta,
                    name: "Insta".into(),
                }],
                favorites: vec![
                    Favorite {
                        id: drawings,
                        path: r"D:\Projects\2026\BMW\Drawings".into(),
                        name: "BMW ნახაზები".into(),
                        renamed: true,
                        group: None,
                    },
                    Favorite {
                        id: root,
                        path: r"E:\".into(),
                        name: "E:".into(),
                        renamed: false,
                        group: None,
                    },
                    Favorite {
                        id: share,
                        path: r"\\INSTA-NAS01\Projects\ნახაზები".into(),
                        name: "ნახაზები".into(),
                        renamed: false,
                        group: Some(insta),
                    },
                ],
            })
        );
    }

    #[test]
    fn recent_folders_are_the_latest_ten() {
        let mut storage = Storage::in_memory().expect("opens");
        for n in 0..12 {
            let folder = format!(r"\\?\D:\Projects\{n}");
            storage.visit_folder(Path::new(&folder), n).expect("visits");
        }
        storage
            .visit_folder(Path::new(r"\\?\UNC\nas\share\ნახაზები"), 100)
            .expect("visits");
        storage
            .visit_folder(Path::new(r"\\?\UNC\nas\share"), 99)
            .expect("visits");
        let latest = recent(&storage).expect("reads");
        assert_eq!(latest.len(), RECENT);
        assert_eq!(
            latest[0],
            RecentFolder {
                path: r"\\nas\share\ნახაზები".into(),
                name: "ნახაზები".into(),
            }
        );
        // A share is named as its tab is.
        assert_eq!(latest[1].name, r"\\nas\share");
        assert_eq!(latest[2].name, "11");
        assert_eq!(latest[9].name, "4");
    }
}
