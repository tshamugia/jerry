//! The sidebar's places, drives and network locations (#34): commands, and `drives:changed` and
//! `network-places:changed` to every window. The drive list is `jerry_core::drives`; a drive
//! plugged in or pulled out reaches it through `jerry_win::devices`; ejecting goes through the
//! Shell (`jerry_win::drives`).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use jerry_core::drives::{
    DriveList, DriveMonitor, Space, System, network_place, server_name, share_name,
};
use jerry_core::error::display_path;
use jerry_core::space::{SPACE_WAIT, SpaceReader};
use jerry_core::{ErrorCode, JerryError};
use jerry_win::icons::Place;
use jerry_win::shell_pool::ShellPool;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, State, WebviewWindow};
use tauri_specta::Event;

use crate::jobs;
use crate::listing::Listings;
use crate::storage::Db;
use crate::title_bar::hwnd;
use crate::transfers::Transfers;

/// How long an eject may take before Jerry stops waiting. Windows first asks every program to
/// let go of the drive, and may show its own message, which waits for a click.
const EJECT_WAIT: Duration = Duration::from_secs(60);

/// Sent to every window when the drives change: one plugged in, pulled out or ejected, a network
/// drive mapped or disconnected, a label or the free space.
#[derive(Debug, Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "drives:changed")]
pub struct DrivesChanged(pub DriveList);

/// Sent to every window when a network location is added to the sidebar or removed.
#[derive(Debug, Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "network-places:changed")]
pub struct NetworkPlacesChanged(pub Vec<NetworkPlace>);

/// The drive monitor, the status bar's reads of free space where the monitor has none, and a
/// Shell pool of its own for Eject, so a slow eject never holds up icons. Clones share them.
#[derive(Clone)]
pub struct Drives(Arc<Inner>);

struct Inner {
    monitor: DriveMonitor,
    space: SpaceReader,
    /// Started with the first eject.
    shell: OnceLock<ShellPool>,
}

impl Drives {
    /// Starts following the drives: every change goes to every window as `drives:changed`, and
    /// Windows' notices of volumes coming and going make the monitor look at once.
    pub fn start(app: &AppHandle) -> Self {
        let handle = app.clone();
        let monitor = DriveMonitor::start(System, move |list| {
            if let Err(error) = DrivesChanged(list.clone()).emit(&handle) {
                tracing::error!(%error, "can't send drives:changed");
            }
        });
        let drives = Self(Arc::new(Inner {
            monitor,
            space: SpaceReader::default(),
            shell: OnceLock::new(),
        }));
        let weak = Arc::downgrade(&drives.0);
        jerry_win::devices::on_drives_change(move || {
            if let Some(inner) = weak.upgrade() {
                inner.monitor.refresh();
            }
        });
        drives
    }

    fn shell(&self) -> &ShellPool {
        self.0.shell.get_or_init(ShellPool::new)
    }
}

/// A standard place: which one (its `icon://` name), its folder as display text, and for OneDrive
/// its folder's own name (`OneDrive - Insta LLC`); the UI names the others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct SidebarPlace {
    pub place: PlaceName,
    pub path: String,
    pub name: Option<String>,
}

/// The standard places, as `icon://` names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum PlaceName {
    Desktop,
    Downloads,
    Documents,
    Pictures,
    OneDrive,
}

/// A network location the user added: its path as display text (`\\server\share`), the name
/// the sidebar shows (the share's) and its server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct NetworkPlace {
    pub path: String,
    pub name: String,
    pub server: String,
}

/// The drives as last read; `drives:changed` brings every later list. Keep the one with the
/// highest `generation`.
#[tauri::command]
#[specta::specta]
pub async fn drives_list(drives: State<'_, Drives>) -> Result<DriveList, JerryError> {
    Ok(drives.0.monitor.drives())
}

/// The size and free space of the volume listing `listing`'s folder is on, for the status bar
/// where the drive list has none: a network drive or share (#41). None if the volume doesn't
/// answer within [`SPACE_WAIT`] (the read goes on alone, and asking again waits for it), can't be
/// read, or the listing isn't open.
#[tauri::command]
#[specta::specta]
pub async fn folder_space(
    drives: State<'_, Drives>,
    listings: State<'_, Listings>,
    listing: u32,
) -> Result<Option<Space>, JerryError> {
    let Some(folder) = listings.place(listing) else {
        return Ok(None);
    };
    let reader = drives.0.space.clone();
    jobs::background("folder space", move || Ok(reader.read(&folder, SPACE_WAIT))).await
}

/// What the UI should warn about before ejecting a drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EjectCheck {
    /// Unfinished transfers that read from or write to the drive.
    pub transfers: u32,
}

/// `"E"` as a drive letter, upper case.
fn drive_letter(letter: &str) -> Result<u8, JerryError> {
    match letter.as_bytes() {
        [letter] if letter.is_ascii_alphabetic() => Ok(letter.to_ascii_uppercase()),
        _ => Err(JerryError::new(ErrorCode::NotFound)),
    }
}

/// Whether ejecting drive `letter` would get in the way of a transfer, so the UI warns first
/// (PRD «დისკები»). Windows itself refuses while a file is open on the drive.
#[tauri::command]
#[specta::specta]
pub async fn eject_check(
    transfers: State<'_, Transfers>,
    letter: String,
) -> Result<EjectCheck, JerryError> {
    Ok(EjectCheck {
        transfers: transfers.on_drive(drive_letter(&letter)?),
    })
}

/// Ejects drive `letter` (`"E"`) as Explorer's Eject does. Windows shows its own message if
/// something holds the drive; the list follows with `drives:changed`.
#[tauri::command]
#[specta::specta]
pub async fn eject(
    window: WebviewWindow,
    drives: State<'_, Drives>,
    letter: String,
) -> Result<(), JerryError> {
    let letter = drive_letter(&letter)?;
    let owner = hwnd(&window).ok();
    let drives = Drives::clone(&drives);
    jobs::background("eject", move || {
        let ejected = drives.shell().run("eject", EJECT_WAIT, move || {
            jerry_win::drives::eject(owner, letter)
        });
        drives.0.monitor.refresh();
        ejected
    })
    .await
}

/// Desktop, Downloads, Documents, Pictures and OneDrive (once it is set up), in sidebar order.
#[tauri::command]
#[specta::specta]
pub async fn sidebar_places() -> Result<Vec<SidebarPlace>, JerryError> {
    jobs::background("sidebar places", || {
        Ok(jerry_win::drives::places()
            .into_iter()
            .filter_map(|(place, path)| {
                let (place, name) = match place {
                    Place::Desktop => (PlaceName::Desktop, None),
                    Place::Downloads => (PlaceName::Downloads, None),
                    Place::Documents => (PlaceName::Documents, None),
                    Place::Pictures => (PlaceName::Pictures, None),
                    Place::OneDrive => (
                        PlaceName::OneDrive,
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned()),
                    ),
                    Place::RecycleBin | Place::NetworkShare => return None,
                };
                Some(SidebarPlace {
                    place,
                    path: display_path(&path),
                    name,
                })
            })
            .collect())
    })
    .await
}

/// The network locations the user added, in sidebar order.
#[tauri::command]
#[specta::specta]
pub async fn network_places_list(db: State<'_, Db>) -> Result<Vec<NetworkPlace>, JerryError> {
    let db = Db::clone(&db);
    jobs::background("network places", move || db.with(|storage| list(storage))).await
}

/// Adds the network location `text` (`\\server\share`, typed) at the end of the sidebar's
/// Network section, without contacting the server. `InvalidName` for text that isn't a share;
/// one that is there already stays where it is. Every window gets `network-places:changed`.
#[tauri::command]
#[specta::specta]
pub async fn network_place_add(
    app: AppHandle,
    db: State<'_, Db>,
    text: String,
) -> Result<(), JerryError> {
    let path = network_place(&text)?;
    let db = Db::clone(&db);
    jobs::background("add network place", move || {
        db.with(|storage| {
            if storage.add_network_place(&path)? {
                announce(&app, list(storage)?);
            }
            Ok(())
        })
    })
    .await
}

/// Removes the network location `path` (as `NetworkPlace::path` gives it) from the sidebar; the
/// share itself is untouched. Every window gets `network-places:changed`.
#[tauri::command]
#[specta::specta]
pub async fn network_place_remove(
    app: AppHandle,
    db: State<'_, Db>,
    path: String,
) -> Result<(), JerryError> {
    let path = network_place(&path)?;
    let db = Db::clone(&db);
    jobs::background("remove network place", move || {
        db.with(|storage| {
            if storage.remove_network_place(&path)? {
                announce(&app, list(storage)?);
            }
            Ok(())
        })
    })
    .await
}

/// Opens Recycle Bin in Explorer; Jerry's own view comes in v1.0.
#[tauri::command]
#[specta::specta]
pub async fn recycle_bin_open(window: WebviewWindow) -> Result<(), JerryError> {
    jerry_win::drives::open_recycle_bin(hwnd(&window).ok());
    Ok(())
}

fn list(storage: &jerry_core::storage::Storage) -> Result<Vec<NetworkPlace>, JerryError> {
    Ok(storage
        .network_places()?
        .iter()
        .filter_map(|path| {
            let path = display_path(path);
            Some(NetworkPlace {
                name: share_name(&path)?,
                server: server_name(&path)?,
                path,
            })
        })
        .collect())
}

/// Still under the database's lock, so windows hear the changes in the order they were saved.
fn announce(app: &AppHandle, places: Vec<NetworkPlace>) {
    if let Err(error) = NetworkPlacesChanged(places).emit(app) {
        tracing::error!(%error, "can't send network-places:changed");
    }
}

#[cfg(test)]
mod tests {
    use jerry_core::storage::Storage;

    use super::*;

    #[test]
    fn network_places_show_their_share_and_server() {
        let mut storage = Storage::in_memory().expect("opens");
        for text in [r"\\INSTA-NAS01\Projects", r"\\nas\პროექტები\2026"] {
            let path = network_place(text).expect("a share");
            storage.add_network_place(&path).expect("saves");
        }
        assert_eq!(
            list(&storage),
            Ok(vec![
                NetworkPlace {
                    path: r"\\INSTA-NAS01\Projects".into(),
                    name: "Projects".into(),
                    server: "INSTA-NAS01".into(),
                },
                NetworkPlace {
                    path: r"\\nas\პროექტები\2026".into(),
                    name: "2026".into(),
                    server: "nas".into(),
                },
            ])
        );
    }
}
