//! Deleting entries of an open listing (#37): Del sends them to the Recycle Bin
//! (`jerry_win::recycle`), Shift+Del deletes them for good (`jerry_core::delete`). The UI decides
//! which entries (the shown ones, selected or focused) and asks the user first; these commands do
//! the work and say what happened to each entry. The listing's watcher shows the result.
//!
//! - `delete_check`: whether the entries take a system folder with them (data safety rule 6).
//! - `delete_measure`: what a permanent delete would remove, for the confirmation; streams its
//!   count so far and stops with `delete_measure_cancel`.
//! - `delete_recycle`: to the Recycle Bin. Entries it can't take come back in `noRecycleBin`,
//!   untouched, for the UI to ask whether to delete them for good (data safety rule 5).
//! - `delete_permanently`: for good, Jerry's own recursive delete.
//!
//! Both deletes refuse a system folder unless the user confirmed it (`system`), whatever the UI
//! asked. What went to the Recycle Bin is kept in memory for Undo (#56).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use jerry_core::delete::{
    Tally, delete_permanently as delete_for_good, measure, takes_system_folder,
};
use jerry_core::{ErrorCode, JerryError};
use jerry_win::recycle::{Outcome, RECYCLE_TIMEOUT, recycle};
use jerry_win::shell_pool::ShellPool;
use serde::Serialize;
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

use crate::jobs;
use crate::listing::Listings;

/// Deletes the Recycle Bin record keeps, newest last (PRD «Undo»: the last 50 operations).
const RECORDED: usize = 50;

/// Whether deleting some entries needs the extra confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeleteCheck {
    /// An entry is a system folder (Windows, Program Files, the Users root) or holds one.
    pub system: bool,
}

/// What a delete did, entry by entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOutcome {
    /// How many entries are gone.
    pub deleted: u32,
    /// Entries the Recycle Bin can't take (a network or removable drive, a path too long for the
    /// Shell, too big for the Recycle Bin): untouched.
    pub no_recycle_bin: Vec<u32>,
    /// Entries still there, all or in part, with the first reason.
    pub failed: Vec<DeleteFailure>,
}

/// An entry that couldn't be deleted. `error.path` is the file that stayed: the entry itself, or
/// a file in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct DeleteFailure {
    pub entry: u32,
    pub error: JerryError,
}

/// An item Del sent to the Recycle Bin: where it was and where it is now. Undo (#56) puts it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recycled {
    pub original: PathBuf,
    pub in_bin: PathBuf,
}

/// The delete commands' state. Clones share it.
#[derive(Clone, Default)]
pub struct Deleting(Arc<Inner>);

#[derive(Default)]
struct Inner {
    /// A Shell pool of its own for `IFileOperation`, so a long recycle never holds up icons.
    /// Started with the first delete.
    shell: OnceLock<ShellPool>,
    /// The measure running for each listing, by its cancel flag.
    measures: Mutex<HashMap<u32, Arc<AtomicBool>>>,
    /// What each recycle sent to the Recycle Bin, newest last.
    recycled: Mutex<VecDeque<Vec<Recycled>>>,
    /// Windows' system folders (`jerry_win::folders::system_folders`), read once.
    system: OnceLock<Vec<PathBuf>>,
}

impl Deleting {
    /// What the last recycles sent to the Recycle Bin, oldest first, one list per recycle.
    pub fn recycled(&self) -> Vec<Vec<Recycled>> {
        lock(&self.0.recycled).iter().cloned().collect()
    }

    fn shell(&self) -> &ShellPool {
        self.0.shell.get_or_init(ShellPool::new)
    }

    /// Windows' system folders, which neither a delete nor a move takes unconfirmed.
    pub fn system(&self) -> &[PathBuf] {
        self.0
            .system
            .get_or_init(jerry_win::folders::system_folders)
    }

    /// Starts measuring for `listing`: a new cancel flag, and the previous measure's is set.
    fn start_measure(&self, listing: u32) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(previous) = lock(&self.0.measures).insert(listing, cancel.clone()) {
            previous.store(true, Ordering::Relaxed);
        }
        cancel
    }

    /// Forgets `listing`'s measure if `cancel` is still its flag.
    fn end_measure(&self, listing: u32, cancel: &Arc<AtomicBool>) {
        let mut measures = lock(&self.0.measures);
        if measures
            .get(&listing)
            .is_some_and(|current| Arc::ptr_eq(current, cancel))
        {
            measures.remove(&listing);
        }
    }

    fn cancel_measure(&self, listing: u32) {
        if let Some(cancel) = lock(&self.0.measures).remove(&listing) {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    fn record(&self, recycled: Vec<Recycled>) {
        if recycled.is_empty() {
            return;
        }
        let mut log = lock(&self.0.recycled);
        if log.len() == RECORDED {
            log.pop_front();
        }
        log.push_back(recycled);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The entries' exact paths. Entries the listing no longer has are failures (`NotFound`).
fn resolve(
    listings: &Listings,
    listing: u32,
    entries: &[u32],
) -> (Vec<(u32, PathBuf)>, Vec<DeleteFailure>) {
    let mut found = Vec::with_capacity(entries.len());
    let mut failed = Vec::new();
    for &entry in entries {
        match listings.entry(listing, entry) {
            Some((path, _, _)) => found.push((entry, path)),
            None => failed.push(DeleteFailure {
                entry,
                error: JerryError::new(ErrorCode::NotFound),
            }),
        }
    }
    (found, failed)
}

/// Splits off the entries that take a system folder unless `system` says the user confirmed
/// them: those fail with `AccessDenied`. The safety net under the UI's own check.
fn guard_system(
    deleting: &Deleting,
    found: Vec<(u32, PathBuf)>,
    system: bool,
    failed: &mut Vec<DeleteFailure>,
) -> Vec<(u32, PathBuf)> {
    if system {
        return found;
    }
    let folders = deleting.system();
    found
        .into_iter()
        .filter(|(entry, path)| {
            let takes = takes_system_folder(path, folders);
            if takes {
                tracing::warn!(path = %path.display(), "refused to delete a system folder unconfirmed");
                failed.push(DeleteFailure {
                    entry: *entry,
                    error: JerryError::new(ErrorCode::AccessDenied).at(path),
                });
            }
            !takes
        })
        .collect()
}

/// Whether deleting `entries` of listing `listing` takes a system folder with it, so the UI asks
/// again. Entries the listing no longer has don't count.
#[tauri::command]
#[specta::specta]
pub async fn delete_check(
    app: AppHandle,
    listing: u32,
    entries: Vec<u32>,
) -> Result<DeleteCheck, JerryError> {
    let (deleting, listings) = state(&app);
    jobs::background("delete check", move || {
        let (found, _) = resolve(&listings, listing, &entries);
        let folders = deleting.system();
        let system = found
            .iter()
            .any(|(_, path)| takes_system_folder(path, folders));
        Ok(DeleteCheck { system })
    })
    .await
}

/// Counts what deleting `entries` of listing `listing` for good would remove: files, folders and
/// bytes, through every subfolder but not through links. Sends the count so far to `on_progress`
/// every 100 ms. One measure runs per listing: a new one, or `delete_measure_cancel`, stops it
/// with `Cancelled`.
#[tauri::command]
#[specta::specta]
pub async fn delete_measure(
    app: AppHandle,
    listing: u32,
    entries: Vec<u32>,
    on_progress: Channel<Tally>,
) -> Result<Tally, JerryError> {
    let (deleting, listings) = state(&app);
    let cancel = deleting.start_measure(listing);
    let job_cancel = cancel.clone();
    let tally = jobs::background("delete measure", move || {
        let (found, _) = resolve(&listings, listing, &entries);
        let paths: Vec<PathBuf> = found.into_iter().map(|(_, path)| path).collect();
        measure(&paths, &job_cancel, |tally| {
            if let Err(error) = on_progress.send(*tally) {
                tracing::debug!(%error, "measure progress not delivered");
            }
        })
    })
    .await;
    deleting.end_measure(listing, &cancel);
    tally
}

/// Stops listing `listing`'s measure, if one runs.
#[tauri::command]
#[specta::specta]
pub async fn delete_measure_cancel(app: AppHandle, listing: u32) {
    app.state::<Deleting>().cancel_measure(listing);
}

/// Sends `entries` of listing `listing` to the Recycle Bin, as Explorer does. Entries the
/// Recycle Bin can't take stay untouched and come back in `noRecycleBin`. `system`: the user
/// confirmed deleting system folders; without it they stay (`AccessDenied`).
#[tauri::command]
#[specta::specta]
pub async fn delete_recycle(
    app: AppHandle,
    listing: u32,
    entries: Vec<u32>,
    system: bool,
) -> Result<DeleteOutcome, JerryError> {
    let (deleting, listings) = state(&app);
    let guarding = deleting.clone();
    let (found, mut failed) = jobs::background("delete guard", move || {
        let (found, mut failed) = resolve(&listings, listing, &entries);
        let found = guard_system(&guarding, found, system, &mut failed);
        Ok((found, failed))
    })
    .await?;
    if found.is_empty() {
        return Ok(DeleteOutcome {
            failed,
            ..DeleteOutcome::default()
        });
    }

    let paths: Vec<PathBuf> = found.iter().map(|(_, path)| path.clone()).collect();
    let (sender, result) = tokio::sync::oneshot::channel();
    deleting.shell().submit(
        "recycle",
        RECYCLE_TIMEOUT,
        move || recycle(&paths),
        move |outcomes| {
            let _ = sender.send(outcomes);
        },
    );
    let outcomes = result
        .await
        .unwrap_or_else(|_| Err(JerryError::internal()))?;

    let mut outcome = DeleteOutcome::default();
    let mut recycled = Vec::new();
    for ((entry, original), result) in found.into_iter().zip(outcomes) {
        match result {
            Outcome::Recycled { in_bin } => {
                outcome.deleted += 1;
                if let Some(in_bin) = in_bin {
                    recycled.push(Recycled { original, in_bin });
                }
            }
            Outcome::NoRecycleBin => outcome.no_recycle_bin.push(entry),
            Outcome::Failed(error) => failed.push(DeleteFailure { entry, error }),
        }
    }
    tracing::info!(
        recycled = outcome.deleted,
        no_recycle_bin = outcome.no_recycle_bin.len(),
        failed = failed.len(),
        "recycled"
    );
    deleting.record(recycled);
    outcome.failed = failed;
    Ok(outcome)
}

/// Deletes `entries` of listing `listing` for good, folders with everything in them, past 260
/// characters too. Only after the user confirmed (Shift+Del, or no Recycle Bin). A file that
/// can't be deleted stays, with the folders above it; the rest goes. `system`: the user
/// confirmed deleting system folders; without it they stay (`AccessDenied`).
#[tauri::command]
#[specta::specta]
pub async fn delete_permanently(
    app: AppHandle,
    listing: u32,
    entries: Vec<u32>,
    system: bool,
) -> Result<DeleteOutcome, JerryError> {
    let (deleting, listings) = state(&app);
    jobs::background("delete permanently", move || {
        Ok(permanently(&deleting, &listings, listing, &entries, system))
    })
    .await
}

fn permanently(
    deleting: &Deleting,
    listings: &Listings,
    listing: u32,
    entries: &[u32],
    system: bool,
) -> DeleteOutcome {
    let (found, mut failed) = resolve(listings, listing, entries);
    let found = guard_system(deleting, found, system, &mut failed);
    let paths: Vec<PathBuf> = found.iter().map(|(_, path)| path.clone()).collect();
    let mut outcome = DeleteOutcome::default();
    for ((entry, _), result) in found.into_iter().zip(delete_for_good(&paths)) {
        match result {
            Ok(()) => outcome.deleted += 1,
            Err(error) => failed.push(DeleteFailure { entry, error }),
        }
    }
    tracing::info!(
        deleted = outcome.deleted,
        failed = failed.len(),
        "deleted permanently"
    );
    outcome.failed = failed;
    outcome
}

fn state(app: &AppHandle) -> (Deleting, Listings) {
    (
        app.state::<Deleting>().inner().clone(),
        app.state::<Listings>().inner().clone(),
    )
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use jerry_core::sort::Sort;

    use super::*;

    /// Opens `folder` as a listing and waits until it has `count` entries; their ids by name.
    fn open(listings: &Listings, folder: &Path, count: usize) -> (u32, HashMap<String, u32>) {
        let id = listings
            .prestart(1, folder, Sort::default())
            .expect("starts");
        let started = Instant::now();
        loop {
            let names: HashMap<String, u32> = (0..)
                .map_while(|entry| listings.entry(id, entry).map(|(path, ..)| (entry, path)))
                .map(|(entry, path)| {
                    let name = path.file_name().expect("a name").to_string_lossy();
                    (name.into_owned(), entry)
                })
                .collect();
            if names.len() == count {
                return (id, names);
            }
            assert!(started.elapsed() < Duration::from_secs(10), "listed");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn deletes_for_good_and_reports_entries_that_stay() {
        let dir = tempfile::tempdir().expect("temp folder");
        let folder = dir.path().join("პროექტი");
        std::fs::create_dir(&folder).expect("mkdir");
        std::fs::write(folder.join("a.dwg"), b"a").expect("write");
        std::fs::write(dir.path().join("ნახაზი.dwg"), b"x").expect("write");
        let locked = dir.path().join("დაკავებული.xlsx");
        std::fs::write(&locked, b"x").expect("write");
        std::fs::write(dir.path().join("დარჩება.txt"), b"x").expect("write");

        let listings = Listings::default();
        let deleting = Deleting::default();
        let (listing, ids) = open(&listings, dir.path(), 4);
        let _held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&locked)
            .expect("lock");
        let entries = [
            ids["პროექტი"],
            ids["ნახაზი.dwg"],
            ids["დაკავებული.xlsx"],
            999,
        ];
        let outcome = permanently(&deleting, &listings, listing, &entries, false);

        assert_eq!(outcome.deleted, 2);
        assert!(outcome.no_recycle_bin.is_empty());
        let failed: Vec<(u32, ErrorCode)> = outcome
            .failed
            .iter()
            .map(|failure| (failure.entry, failure.error.code.clone()))
            .collect();
        assert_eq!(
            failed,
            [
                (999, ErrorCode::NotFound),
                (ids["დაკავებული.xlsx"], ErrorCode::InUse { app: None }),
            ]
        );
        assert!(!folder.exists());
        assert!(locked.exists());
        assert!(dir.path().join("დარჩება.txt").exists(), "not asked for");
    }

    #[test]
    fn system_folders_need_the_users_confirmation() {
        let deleting = Deleting::default();
        let windows = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"));
        let windows = jerry_core::path::long_path(&windows).expect("long path");
        let other = PathBuf::from(r"\\?\C:\Work\x.dwg");
        let mut failed = Vec::new();
        let kept = guard_system(
            &deleting,
            vec![(1, windows.clone()), (2, other.clone())],
            false,
            &mut failed,
        );
        assert_eq!(kept, [(2, other.clone())]);
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].entry, 1);
        assert_eq!(failed[0].error.code, ErrorCode::AccessDenied);

        let mut none = Vec::new();
        let confirmed = guard_system(
            &deleting,
            vec![(1, windows.clone()), (2, other)],
            true,
            &mut none,
        );
        assert_eq!(confirmed.len(), 2);
        assert!(none.is_empty());
    }

    #[test]
    fn a_new_measure_cancels_the_last_one_for_its_listing() {
        let deleting = Deleting::default();
        let first = deleting.start_measure(7);
        let other = deleting.start_measure(8);
        let second = deleting.start_measure(7);
        assert!(first.load(Ordering::Relaxed), "replaced");
        assert!(!second.load(Ordering::Relaxed));
        // The first one ending doesn't forget the second.
        deleting.end_measure(7, &first);
        deleting.cancel_measure(7);
        assert!(second.load(Ordering::Relaxed));
        assert!(!other.load(Ordering::Relaxed));
    }

    #[test]
    fn the_recycle_record_keeps_the_last_fifty() {
        let deleting = Deleting::default();
        deleting.record(Vec::new());
        assert!(
            deleting.recycled().is_empty(),
            "nothing recycled, nothing kept"
        );
        for n in 0..=RECORDED {
            deleting.record(vec![Recycled {
                original: PathBuf::from(format!(r"\\?\C:\{n}.txt")),
                in_bin: PathBuf::from(format!(r"C:\$Recycle.Bin\S-1\$R{n}.txt")),
            }]);
        }
        let log = deleting.recycled();
        assert_eq!(log.len(), RECORDED);
        assert_eq!(log[0][0].original, PathBuf::from(r"\\?\C:\1.txt"));
    }
}
