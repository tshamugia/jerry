//! The watcher on real temp folders: files created, deleted, changed and renamed outside Jerry,
//! mass changes, buffer overflows, one watcher shared by several listings, and a watched folder
//! that is renamed or deleted (Windows must allow both, as Explorer expects).

#![allow(
    clippy::expect_used,
    reason = "test helpers: a failed setup fails the test"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use jerry_core::listing::{Diff, Listing};
use jerry_core::path::long_path;
use jerry_core::volume;
use jerry_core::watcher::{Event, Refresh, Subscription, Watchers};
use jerry_core::{ErrorCode, JerryError};
use tempfile::TempDir;

const WAIT: Duration = Duration::from_secs(15);

fn temp() -> TempDir {
    tempfile::Builder::new()
        .prefix("jerry-watcher-")
        .tempdir()
        .expect("temp folder")
}

/// What a subscriber was told, owned so it can cross to the test thread.
enum Told {
    Changed(Refresh),
    Gone(JerryError),
}

/// A listing kept live by a watcher, as a tab's is.
struct Live {
    subscription: Option<Subscription>,
    told: Receiver<Told>,
    listing: Listing,
}

impl Live {
    fn new(watchers: &Watchers, folder: &Path) -> Self {
        Self::holding_first(watchers, folder, Duration::ZERO)
    }

    /// Like [`Live::new`], with the watcher's thread held for `hold` after the first change it
    /// tells: whatever happens meanwhile piles up in Windows' buffer.
    fn holding_first(watchers: &Watchers, folder: &Path, hold: Duration) -> Self {
        let (sender, told) = channel();
        let held = AtomicBool::new(hold.is_zero());
        let subscription = watchers
            .subscribe(folder, move |event| {
                let told = match event {
                    Event::Changed { now, .. } => Told::Changed(now.clone()),
                    Event::Gone(error) => Told::Gone(error.clone()),
                };
                let _ = sender.send(told);
                if !held.swap(true, Ordering::Relaxed) {
                    std::thread::sleep(hold);
                }
            })
            .expect("watches");
        // Subscribed first, so nothing that happens while reading is missed.
        let listing = Listing::read(folder, &AtomicBool::new(false)).expect("lists");
        Self {
            subscription: Some(subscription),
            told,
            listing,
        }
    }

    /// Applies what the watcher tells until `done` holds; the diffs and whether any of them
    /// came from listing the whole folder again.
    fn until(&mut self, what: &str, done: impl Fn(&Listing) -> bool) -> (Vec<Diff>, bool) {
        let started = Instant::now();
        let mut diffs = Vec::new();
        let mut listed = false;
        while !done(&self.listing) {
            let left = WAIT.saturating_sub(started.elapsed());
            match self.told.recv_timeout(left) {
                Ok(Told::Changed(now)) => {
                    listed |= matches!(now, Refresh::Listed(_));
                    diffs.push(now.apply_to(&mut self.listing));
                }
                Ok(Told::Gone(error)) => panic!("{what}: the folder went away: {error:?}"),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    panic!(
                        "{what}: not seen in {WAIT:?}; live: {:?}",
                        names(&self.listing)
                    )
                }
            }
        }
        (diffs, listed)
    }

    /// Waits for the watcher to say the folder is gone.
    fn gone(&mut self) -> JerryError {
        let started = Instant::now();
        loop {
            let left = WAIT.saturating_sub(started.elapsed());
            match self.told.recv_timeout(left) {
                Ok(Told::Changed(now)) => {
                    now.apply_to(&mut self.listing);
                }
                Ok(Told::Gone(error)) => return error,
                Err(_) => panic!("the watcher didn't notice the folder go in {WAIT:?}"),
            }
        }
    }

    /// The id of the live entry called `name`.
    fn id(&self, name: &str) -> Option<u32> {
        let entries = self.listing.entries();
        self.listing
            .live_ids()
            .into_iter()
            .find(|&id| entries.name(id as usize) == name)
    }
}

fn names(listing: &Listing) -> BTreeSet<String> {
    listing
        .live_ids()
        .into_iter()
        .map(|id| {
            listing
                .entries()
                .name(id as usize)
                .into_string()
                .expect("valid Unicode")
        })
        .collect()
}

fn on_disk(folder: &Path) -> BTreeSet<String> {
    std::fs::read_dir(folder)
        .expect("reads")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .into_string()
                .expect("valid Unicode")
        })
        .collect()
}

fn write(folder: &Path, name: &str, content: &[u8]) {
    std::fs::write(folder.join(name), content).expect("write");
}

#[test]
fn files_created_deleted_and_changed_outside_show_up() {
    let dir = temp();
    write(dir.path(), "a.txt", b"a");
    write(dir.path(), "b.txt", b"b");
    let mut live = Live::new(&Watchers::default(), dir.path());
    let b = live.id("b.txt").expect("b.txt");

    write(dir.path(), "ნახაზი 1.dwg", b"drawing");
    std::fs::create_dir(dir.path().join("პროექტი")).expect("mkdir");
    let (diffs, _) = live.until("created", |listing| names(listing).len() == 4);
    assert!(
        diffs
            .iter()
            .all(|diff| diff.reindex && !diff.added.is_empty())
    );

    std::fs::remove_file(dir.path().join("a.txt")).expect("delete");
    let (diffs, _) = live.until("deleted", |listing| names(listing).len() == 3);
    assert_eq!(diffs.last().map(|diff| diff.removed.len()), Some(1));

    write(dir.path(), "b.txt", b"a longer content");
    live.until("changed", |listing| {
        listing.entries().sizes()[b as usize] == 16
    });
    assert_eq!(live.id("b.txt"), Some(b), "a changed file keeps its id");
    assert_eq!(names(&live.listing), on_disk(dir.path()));
    let folder = live.id("პროექტი").expect("the new folder");
    assert!(live.listing.entries().is_dir(folder as usize));
}

#[test]
fn a_rename_keeps_the_entry_id() {
    let dir = temp();
    write(dir.path(), "plan.dwg", b"plan");
    std::fs::create_dir(dir.path().join("Old folder")).expect("mkdir");
    let mut live = Live::new(&Watchers::default(), dir.path());
    let plan = live.id("plan.dwg").expect("plan.dwg");
    let folder = live.id("Old folder").expect("folder");

    std::fs::rename(dir.path().join("plan.dwg"), dir.path().join("გეგმა.dwg")).expect("rename");
    std::fs::rename(dir.path().join("Old folder"), dir.path().join("ახალი")).expect("rename");
    let (diffs, _) = live.until("renamed", |listing| {
        names(listing) == BTreeSet::from(["გეგმა.dwg".into(), "ახალი".into()])
    });

    assert_eq!(live.id("გეგმა.dwg"), Some(plan));
    assert_eq!(live.id("ახალი"), Some(folder));
    assert!(
        diffs
            .iter()
            .all(|diff| diff.added.is_empty() && diff.removed.is_empty())
    );

    // Only the case changes: still the same entry.
    std::fs::rename(dir.path().join("გეგმა.dwg"), dir.path().join("გეგმა.DWG")).expect("rename");
    live.until("case changed", |listing| {
        names(listing).contains("გეგმა.DWG")
    });
    assert_eq!(live.id("გეგმა.DWG"), Some(plan));
}

#[test]
fn a_mass_change_ends_where_the_folder_is() {
    let dir = temp();
    for i in 0..500 {
        write(dir.path(), &format!("old {i}.txt"), b"");
    }
    let mut live = Live::new(&Watchers::default(), dir.path());

    for i in 0..3_000 {
        write(dir.path(), &format!("ნახაზი {i}.dwg"), b"x");
    }
    for i in 0..250 {
        std::fs::remove_file(dir.path().join(format!("old {i}.txt"))).expect("delete");
    }
    for i in 250..500 {
        let from = dir.path().join(format!("old {i}.txt"));
        std::fs::rename(from, dir.path().join(format!("renamed {i}.txt"))).expect("rename");
    }
    let expected = on_disk(dir.path());
    assert_eq!(expected.len(), 3_250);

    live.until("every change", |listing| names(listing) == expected);
    assert_eq!(live.listing.len(), 3_250);
}

#[test]
fn an_overflow_lists_the_folder_again() {
    let dir = temp();
    // A buffer this small holds a handful of changes; Windows keeps one as big for the changes
    // that come while the watcher is busy.
    let watchers = Watchers::with_buffer(256);
    let mut live = Live::holding_first(&watchers, dir.path(), Duration::from_secs(1));
    write(dir.path(), "first.txt", b"");
    live.until("the first file", |listing| listing.len() == 1);

    // The watcher is held now: 400 changes can't fit.
    for i in 0..400 {
        write(dir.path(), &format!("file {i}.txt"), b"");
    }
    let expected = on_disk(dir.path());
    let (_, listed) = live.until("every file", |listing| names(listing) == expected);
    assert!(listed, "the folder was listed again");
}

#[test]
fn listings_of_one_folder_share_one_watcher_that_stops_with_the_last() {
    let dir = temp();
    let other = temp();
    let watchers = Watchers::default();
    let mut first = Live::new(&watchers, dir.path());
    let mut second = Live::new(&watchers, dir.path());
    let elsewhere = Live::new(&watchers, other.path());
    assert_eq!(watchers.watched(), 2);

    write(dir.path(), "shared.txt", b"");
    first.until("first", |listing| listing.len() == 1);
    second.until("second", |listing| listing.len() == 1);

    first.subscription = None;
    assert_eq!(watchers.watched(), 2, "the second listing still watches");
    write(dir.path(), "again.txt", b"");
    second.until("after the first closed", |listing| listing.len() == 2);

    second.subscription = None;
    assert_eq!(watchers.watched(), 1);
    drop(elsewhere);
    assert_eq!(watchers.watched(), 0);

    // Nothing more is told once closed.
    write(dir.path(), "late.txt", b"");
    std::thread::sleep(Duration::from_millis(300));
    assert!(first.told.try_recv().is_err());
}

#[test]
fn a_watched_folder_can_be_deleted_and_is_gone() {
    let dir = temp();
    let watched = dir.path().join("ნახაზები");
    std::fs::create_dir(&watched).expect("mkdir");
    write(&watched, "a.dwg", b"a");
    let watchers = Watchers::default();
    let mut live = Live::new(&watchers, &watched);

    // Explorer deletes the content, then the folder: Jerry must not stand in the way.
    std::fs::remove_dir_all(&watched).expect("a watched folder can be deleted");
    let error = live.gone();

    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(watchers.watched(), 0);
    // No handle is left behind: the folder above can go too.
    std::fs::remove_dir(dir.path()).expect("the parent can be deleted");
    std::fs::create_dir(dir.path()).expect("put the temp folder back for its drop");
}

#[test]
fn a_watched_folder_can_be_renamed_and_is_gone() {
    let dir = temp();
    let watched = dir.path().join("Work");
    std::fs::create_dir(&watched).expect("mkdir");
    let mut live = Live::new(&Watchers::default(), &watched);

    let renamed = dir.path().join("Work (old)");
    std::fs::rename(&watched, &renamed).expect("a watched folder can be renamed");
    // A new folder in its place must not be mistaken for it.
    std::fs::create_dir(&watched).expect("mkdir");

    let error = live.gone();
    assert_eq!(error.code, ErrorCode::NotFound);
    std::fs::remove_dir_all(&renamed).expect("and deleted");
}

#[test]
fn folders_above_a_watched_one_are_held_as_an_explorer_window_holds_them() {
    let dir = temp();
    let parent = dir.path().join("პროექტები");
    let watched = parent.join("2026");
    std::fs::create_dir_all(&watched).expect("mkdir");
    let live = Live::new(&Watchers::default(), &watched);
    let moved = dir.path().join("არქივი");

    // NTFS refuses to rename a folder while anything below it is open, whatever the share mode:
    // the same happens with an Explorer window open on `2026` (checked by hand, 04.11.2026).
    let refused = std::fs::rename(&parent, &moved).expect_err("refused while watched");
    assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);

    // Closing the listing lets go of it.
    drop(live);
    let started = Instant::now();
    while let Err(error) = std::fs::rename(&parent, &moved) {
        assert!(
            started.elapsed() < WAIT,
            "still held after closing: {error}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn folders_deeper_than_max_path_are_watched() {
    let dir = temp();
    let mut deep: PathBuf = long_path(dir.path()).expect("long path");
    while deep.as_os_str().len() < 300 {
        deep.push("ძალიან გრძელი საქაღალდის სახელი");
    }
    std::fs::create_dir_all(&deep).expect("mkdir");
    let mut live = Live::new(&Watchers::default(), &deep);

    write(&deep, "ბოლო.txt", b"");
    live.until("created deep down", |listing| listing.len() == 1);
    assert!(live.id("ბოლო.txt").is_some());
}

#[test]
fn a_missing_folder_is_not_watched() {
    let dir = temp();
    let error = Watchers::default()
        .subscribe(&dir.path().join("აღარ არის"), |_| {})
        .err()
        .expect("nothing to watch");
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn a_released_volume_holds_nothing_and_a_kept_one_is_listed_again() {
    let dir = temp();
    let parent = dir.path().join("USB");
    let watched = parent.join("ნახაზები");
    std::fs::create_dir_all(&watched).expect("mkdir");
    write(&watched, "a.dwg", b"a");
    let watchers = Watchers::default();
    let mut live = Live::new(&watchers, &watched);
    let root = volume::root(&long_path(&watched).expect("long path")).expect("a root");
    let moved = dir.path().join("USB (moved)");
    assert!(
        std::fs::rename(&parent, &moved).is_err(),
        "held while watched"
    );

    // Windows asks for the volume: once `release` returns, nothing on it is open.
    assert!(watchers.release(&root, Instant::now() + WAIT));
    std::fs::rename(&parent, &moved).expect("nothing holds the folder");
    std::fs::rename(&moved, &parent).expect("and back");
    // Meanwhile no folder on the volume opens, and changes go unseen.
    let error = watchers
        .subscribe(dir.path(), |_| {})
        .err()
        .expect("released");
    assert_eq!(error.code, ErrorCode::DeviceGone);
    write(&watched, "while released.txt", b"");
    std::thread::sleep(Duration::from_millis(300));
    assert!(live.told.try_recv().is_err(), "nothing told while released");

    // The removal failed: the folder opens again and is listed anew.
    watchers.keep(&root);
    let (_, listed) = live.until("listed again", |listing| listing.len() == 2);
    assert!(listed, "the whole folder was listed again");
    assert_eq!(watchers.watched(), 1);
    write(&watched, "after.txt", b"");
    live.until("live again", |listing| listing.len() == 3);
    assert!(std::fs::rename(&parent, &moved).is_err(), "held again");
    let other = watchers.subscribe(dir.path(), |_| {});
    assert!(other.is_ok(), "folders on the volume open again");
}

#[test]
fn a_released_volume_that_goes_ends_its_watchers_with_device_gone() {
    let dir = temp();
    let watchers = Watchers::default();
    let mut live = Live::new(&watchers, dir.path());
    let root = volume::root(&long_path(dir.path()).expect("long path")).expect("a root");

    assert!(watchers.release(&root, Instant::now() + WAIT));
    watchers.gone(&root);

    let error = live.gone();
    assert_eq!(error.code, ErrorCode::DeviceGone);
    assert_eq!(watchers.watched(), 0);
    // A volume that comes back is watched again.
    let again = Live::new(&watchers, dir.path());
    assert_eq!(watchers.watched(), 1);
    drop(again);
}

#[test]
fn releasing_another_volume_or_keeping_an_unreleased_one_changes_nothing() {
    let dir = temp();
    let watchers = Watchers::default();
    let mut live = Live::new(&watchers, dir.path());
    let root = volume::root(&long_path(dir.path()).expect("long path")).expect("a root");

    // No folder is watched on a drive with no volume, so nothing waits.
    assert!(watchers.release(Path::new(r"\?\Q:\"), Instant::now()));
    watchers.keep(&root);
    write(dir.path(), "seen.txt", b"");
    let (_, listed) = live.until("still live", |listing| listing.len() == 1);
    assert!(!listed, "looked up by name, not listed again");
    watchers.gone(Path::new(r"\?\Q:\"));
    assert_eq!(watchers.watched(), 1);
}
