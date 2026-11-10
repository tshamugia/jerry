//! The transfer queue in the app (#49): commands, and `transfer:changed`, `transfer:progress` and
//! `transfer:ended` to every window. The queue itself is `jerry_core::transfer::queue`, and its
//! journal in `jerry.db` is `jerry_core::transfer::journal` (#50): as Jerry starts, the jobs the
//! Jerry before it didn't finish come back as `unfinished`, for Resume or Discard. A job whose
//! name is taken waits as `conflict` for `conflict_resolve` (#51); what Replace replaces goes to
//! the Recycle Bin through `jerry_win::recycle`, on the delete commands' Shell pool.
//!
//! The UI names what to transfer by listing entries and the destination by a listing, an entry
//! or a folder token, so an operation's paths never come from JS text (CLAUDE.md rule 4).

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jerry_core::delete::takes_system_folder;
use jerry_core::storage::Storage;
use jerry_core::transfer::journal::{StorageJournal, Store};
use jerry_core::transfer::queue::{
    Action, Notice, Queue, Shell, TransferEnded, TransferList, TransferProgress,
};
use jerry_core::transfer::{Answer, Bin, Job, Kind};
use jerry_core::{ErrorCode, JerryError};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

use crate::delete::Deleting;
use crate::jobs;
use crate::listing::Listings;
use crate::storage::{Db, SettingsChanged, now_ms};

/// Sent to every window when the queue changes: a job came, went, was reordered or changed state.
/// Keep the list with the highest `generation`.
#[derive(Debug, Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "transfer:changed")]
pub struct TransfersChanged(pub TransferList);

/// How far the running jobs have come, at most 10 times a second.
#[derive(Debug, Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "transfer:progress")]
pub struct TransfersProgressed(pub Vec<TransferProgress>);

/// A job ended, with what it did and what failed.
#[derive(Debug, Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "transfer:ended")]
pub struct TransferFinished(pub TransferEnded);

/// The folder a transfer goes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TransferDestination {
    /// The folder the open listing `listing` shows.
    Listing { listing: u32 },
    /// Subfolder `entry` of the open listing `listing`.
    Entry { listing: u32, entry: u32 },
    /// A folder a listing named before (`ListingOpened::folder`, a crumb).
    Folder { folder: u32 },
}

/// The transfer queue. Clones share it.
#[derive(Clone)]
pub struct Transfers(Queue);

impl Transfers {
    /// A queue that tells every window what happens, following the "parallel" setting, and
    /// writes its jobs down in `db`. The jobs the Jerry before didn't finish are listed once the
    /// database is open.
    pub fn start(app: &AppHandle, db: &Db) -> Self {
        let handle = app.clone();
        let journal = Arc::new(StorageJournal::new(db.clone(), now_ms()));
        let shell = Arc::new(WinShell(app.state::<Deleting>().inner().clone()));
        let queue = Queue::start(true, journal.clone(), shell, move |notice| {
            announce(&handle, notice);
        });
        let following = queue.clone();
        SettingsChanged::listen_any(app, move |event| {
            following.set_parallel(event.payload.0.parallel_transfers);
        });
        let (reading, db) = (queue.clone(), db.clone());
        tauri::async_runtime::spawn_blocking(move || {
            if let Ok(parallel) = db.with(|storage| Ok(storage.settings().parallel_transfers)) {
                reading.set_parallel(parallel);
            }
            let recovered =
                jerry_core::job::run_job("transfer recovery", || Ok(journal.recover(&reading)));
            if recovered.is_err() {
                tracing::error!("couldn't look for transfers the Jerry before didn't finish");
            }
        });
        Self(queue)
    }

    /// How many unfinished jobs read from or write to drive `letter` (upper case).
    pub fn on_drive(&self, letter: u8) -> u32 {
        self.0
            .on_volume(Path::new(&format!(r"\\?\{}:\", char::from(letter))))
    }
}

/// What the queue needs of Windows' Shell: the Recycle Bin a replaced file goes to, on the
/// delete commands' Shell pool, and type icons for the conflict window.
struct WinShell(Deleting);

impl Bin for WinShell {
    fn recycle(&self, path: &Path) -> Result<(), JerryError> {
        self.0.recycle_replaced(path)
    }
}

impl Shell for WinShell {
    fn icon(&self, name: &OsStr, attributes: u32) -> u32 {
        let name: Vec<u16> = name.encode_wide().collect();
        jerry_win::icons::keys::type_key(&name, attributes)
    }
}

/// The journal writes to the app's database, waiting for it to open.
impl Store for Db {
    fn with_storage(
        &self,
        job: &mut dyn FnMut(&mut Storage) -> Result<(), JerryError>,
    ) -> Result<(), JerryError> {
        self.with(job)
    }
}

fn announce(app: &AppHandle, notice: Notice) {
    let sent = match notice {
        Notice::Jobs(list) => TransfersChanged(list).emit(app),
        Notice::Progress(running) => TransfersProgressed(running).emit(app),
        Notice::Ended(ended) => TransferFinished(*ended).emit(app),
    };
    if let Err(error) = sent {
        tracing::error!(%error, "can't send a transfer event");
    }
}

/// Copies or moves `entries` of listing `listing` into `destination`, at the end of the queue,
/// and returns the job's id. Entries the listing no longer has are left out; with none left it
/// fails with `NotFound`. A move of a system folder needs `system`, the user's confirmation
/// (`delete_check` says whether to ask); without it, it fails with `AccessDenied` and nothing is
/// queued.
#[tauri::command]
#[specta::specta]
pub async fn transfer_enqueue(
    app: AppHandle,
    kind: Kind,
    listing: u32,
    entries: Vec<u32>,
    destination: TransferDestination,
    system: bool,
) -> Result<u32, JerryError> {
    let transfers = app.state::<Transfers>().inner().clone();
    let listings = app.state::<Listings>().inner().clone();
    let deleting = app.state::<Deleting>().inner().clone();
    jobs::background("transfer enqueue", move || {
        let job = job_for(&listings, kind, listing, &entries, destination)?;
        if kind == Kind::Move && !system {
            refuse_system_folders(&job.sources, deleting.system())?;
        }
        Ok(transfers.0.enqueue(job))
    })
    .await
}

/// Pauses, resumes or cancels job `job`, or every job with null. Resume also carries on a job
/// that waits for its device, once its state says `ready`, and one Jerry didn't finish
/// (`unfinished`); Cancel discards that one. `NotFound` for a job that isn't listed.
#[tauri::command]
#[specta::specta]
pub async fn transfer_control(
    transfers: State<'_, Transfers>,
    action: Action,
    job: Option<u32>,
) -> Result<(), JerryError> {
    let transfers = Transfers::clone(&transfers);
    jobs::background("transfer control", move || transfers.0.control(action, job)).await
}

/// Puts the jobs of `order` first, in that order; the others follow as they were.
#[tauri::command]
#[specta::specta]
pub async fn transfer_reorder(
    transfers: State<'_, Transfers>,
    order: Vec<u32>,
) -> Result<(), JerryError> {
    transfers.0.reorder(&order);
    Ok(())
}

/// Queues job `job` again if it ended with failed items (only those are tried again) or couldn't
/// start. `NotFound` if there is no such job or nothing to try again.
#[tauri::command]
#[specta::specta]
pub async fn transfer_retry_failed(
    transfers: State<'_, Transfers>,
    job: u32,
) -> Result<(), JerryError> {
    transfers.0.retry_failed(job)
}

/// Answers the conflict job `job` waits on (its state is `conflict`): one of the conflict's
/// `choices`, and with `all`, the same for every later conflict of its kind in the job. The job
/// goes on once its disk is free. `NotFound` if it doesn't wait for an answer any more (answered
/// in another window, or cancelled).
#[tauri::command]
#[specta::specta]
pub async fn conflict_resolve(
    transfers: State<'_, Transfers>,
    job: u32,
    answer: Answer,
) -> Result<(), JerryError> {
    transfers.0.resolve(job, answer)
}

/// The queue as it is now; `transfer:changed` brings every later list. Keep the one with the
/// highest `generation`.
#[tauri::command]
#[specta::specta]
pub async fn transfer_list(transfers: State<'_, Transfers>) -> Result<TransferList, JerryError> {
    Ok(transfers.0.list())
}

/// The job for `entries` of a listing going into `destination`.
fn job_for(
    listings: &Listings,
    kind: Kind,
    listing: u32,
    entries: &[u32],
    destination: TransferDestination,
) -> Result<Job, JerryError> {
    let not_found = || JerryError::new(ErrorCode::NotFound);
    let target = match destination {
        TransferDestination::Listing { listing } => listings.folder(listing),
        TransferDestination::Entry { listing, entry } => {
            listings.entry(listing, entry).map(|(path, ..)| path)
        }
        TransferDestination::Folder { folder } => listings.folder_path(folder),
    }
    .ok_or_else(not_found)?;
    let sources: Vec<PathBuf> = entries
        .iter()
        .filter_map(|&entry| listings.entry(listing, entry).map(|(path, ..)| path))
        .collect();
    if sources.len() < entries.len() {
        tracing::warn!(
            asked = entries.len(),
            found = sources.len(),
            "some entries to transfer are gone"
        );
    }
    if sources.is_empty() {
        return Err(not_found());
    }
    Ok(Job {
        kind,
        sources,
        destination: target,
    })
}

/// `AccessDenied` on the first of `sources` that takes a system folder with it: the safety net
/// under the UI's confirmation (data safety rule 6).
fn refuse_system_folders(sources: &[PathBuf], system: &[PathBuf]) -> Result<(), JerryError> {
    match sources
        .iter()
        .find(|source| takes_system_folder(source, system))
    {
        Some(source) => {
            tracing::warn!(path = %source.display(), "refused to move a system folder unconfirmed");
            Err(JerryError::new(ErrorCode::AccessDenied).at(source))
        }
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
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
    fn a_job_is_made_of_entries_and_a_token_never_of_text() {
        let dir = tempfile::tempdir().expect("temp folder");
        std::fs::write(dir.path().join("ნახაზი.dwg"), b"x").expect("write");
        std::fs::create_dir(dir.path().join("შიგნით")).expect("mkdir");
        let listings = Listings::default();
        let listing = open(&listings, dir.path(), 2);
        let folder = listings.folder(listing).expect("open");
        let (mut file, mut sub) = (None, None);
        for entry in 0..2 {
            let (path, ..) = listings.entry(listing, entry).expect("an entry");
            if path.ends_with("ნახაზი.dwg") {
                file = Some((entry, path));
            } else {
                sub = Some((entry, path));
            }
        }
        let (file_entry, file) = file.expect("the file");
        let (sub_entry, sub) = sub.expect("the folder");

        let cases = [
            (TransferDestination::Listing { listing }, folder.clone()),
            (
                TransferDestination::Entry {
                    listing,
                    entry: sub_entry,
                },
                sub,
            ),
            (
                TransferDestination::Folder {
                    folder: listings.folder_token(&folder),
                },
                folder.clone(),
            ),
        ];
        for (destination, expected) in cases {
            let job = job_for(
                &listings,
                Kind::Copy,
                listing,
                &[file_entry, 99],
                destination,
            )
            .expect("a job");
            assert_eq!(job.sources, std::slice::from_ref(&file), "99 is left out");
            assert_eq!(job.destination, expected);
            assert_eq!(job.kind, Kind::Copy);
        }

        let destination = TransferDestination::Listing { listing };
        let none = job_for(&listings, Kind::Copy, listing, &[99], destination);
        assert_eq!(
            none.err().map(|error| error.code),
            Some(ErrorCode::NotFound)
        );
        let closed = TransferDestination::Listing {
            listing: listing + 1,
        };
        let gone = job_for(&listings, Kind::Copy, listing, &[file_entry], closed);
        assert_eq!(
            gone.err().map(|error| error.code),
            Some(ErrorCode::NotFound)
        );
    }

    /// The original path the `$I` record next to `in_bin` (`$R…`) keeps.
    fn recorded_original(in_bin: &Path) -> String {
        let name = in_bin
            .file_name()
            .and_then(|name| name.to_str())
            .expect("$R");
        let bytes = std::fs::read(in_bin.with_file_name(name.replacen("$R", "$I", 1))).expect("$I");
        // Version 2: a header of 24 bytes, the path length in UTF-16 units, then the path.
        let units: Vec<u16> = bytes[28..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .take_while(|&unit| unit != 0)
            .collect();
        String::from_utf16_lossy(&units)
    }

    #[test]
    fn replace_sends_the_file_there_to_windows_recycle_bin() {
        use jerry_core::transfer::Choice;
        use jerry_core::transfer::queue::TransferState;

        let dir = tempfile::Builder::new()
            .prefix("jerry-replace-")
            .tempdir()
            .expect("temp folder");
        // Long names, as the Recycle Bin records them (`%TEMP%` can be `C:\Users\RUNNER~1\…`).
        let root = std::fs::canonicalize(dir.path()).expect("canonical path");
        let (from, to) = (root.join("from"), root.join("to"));
        std::fs::create_dir(&from).expect("mkdir");
        std::fs::create_dir(&to).expect("mkdir");
        let name = "ნახაზი.dwg";
        std::fs::write(from.join(name), b"new drawing").expect("write");
        std::fs::write(to.join(name), b"old drawing").expect("write");

        let deleting = Deleting::default();
        let shell = Arc::new(WinShell(deleting.clone()));
        let queue = Queue::start(true, Arc::new(()), shell, |_| {});
        let id = queue.enqueue(Job {
            kind: Kind::Copy,
            sources: vec![from.join(name)],
            destination: to.clone(),
        });
        let state = || queue.list().jobs[0].state.clone();
        let until = |what: &str, done: &dyn Fn(&TransferState) -> bool| {
            let started = Instant::now();
            while !done(&state()) {
                assert!(started.elapsed() < Duration::from_secs(30), "{what}");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        until("asks", &|state| {
            matches!(state, TransferState::Conflict { .. })
        });
        let TransferState::Conflict { conflict } = state() else {
            unreachable!()
        };
        assert_eq!(
            conflict.choices,
            [
                Choice::Replace,
                Choice::Skip,
                Choice::KeepBoth,
                Choice::ReplaceIfNewer
            ]
        );
        assert!(
            conflict.existing.icon >= jerry_win::icons::keys::FIRST_TYPE,
            "the type's icon"
        );
        assert_eq!(conflict.existing.size, 11);

        queue
            .resolve(
                id,
                Answer {
                    choice: Choice::Replace,
                    all: false,
                },
            )
            .expect("answered");
        until("done", &|state| *state == TransferState::Done);
        assert_eq!(std::fs::read(to.join(name)).expect("read"), b"new drawing");

        let replaced = deleting.replaced();
        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].original, to.join(name));
        let in_bin = &replaced[0].in_bin;
        assert!(
            in_bin.to_string_lossy().contains("$Recycle.Bin"),
            "{in_bin:?}"
        );
        assert_eq!(
            std::fs::read(in_bin).expect("in the Recycle Bin"),
            b"old drawing"
        );
        assert_eq!(
            recorded_original(in_bin),
            jerry_core::error::display_path(&to.join(name)),
            "Restore puts it back there"
        );
        // Our own item leaves the Recycle Bin again.
        let record = in_bin.with_file_name(
            in_bin
                .file_name()
                .and_then(|name| name.to_str())
                .expect("$R")
                .replacen("$R", "$I", 1),
        );
        std::fs::remove_file(record).expect("$I goes");
        std::fs::remove_file(in_bin).expect("$R goes");
    }

    #[test]
    fn moving_a_system_folder_needs_the_users_confirmation() {
        let windows = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"));
        let windows = jerry_core::path::long_path(&windows).expect("long path");
        let system = [windows.clone()];
        let work = PathBuf::from(r"\\?\C:\Work\x.dwg");
        assert_eq!(
            refuse_system_folders(std::slice::from_ref(&work), &system),
            Ok(())
        );
        let refused = refuse_system_folders(&[work, windows.clone()], &system);
        assert_eq!(
            refused,
            Err(JerryError::new(ErrorCode::AccessDenied).at(&windows))
        );
    }
}
