//! Folder listing on real temp folders: Georgian names, long paths, empty and denied folders,
//! missing folders and drives, locked files, names that aren't valid Unicode, batching, cancel.

#![allow(
    clippy::expect_used,
    reason = "test helpers: a failed setup fails the test"
)]

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use jerry_core::ErrorCode;
use jerry_core::listing::{FIRST_BATCH, Listing, filetime_to_unix_ms, list};
use jerry_core::path::long_path;
use tempfile::TempDir;

fn temp() -> TempDir {
    tempfile::Builder::new()
        .prefix("jerry-listing-")
        .tempdir()
        .expect("temp folder")
}

fn read(folder: &Path) -> Listing {
    Listing::read(folder, &AtomicBool::new(false)).expect("folder lists")
}

/// The id of the entry called `name`.
fn find(listing: &Listing, name: &str) -> usize {
    let entries = listing.entries();
    (0..entries.len())
        .find(|&id| entries.name(id) == name)
        .unwrap_or_else(|| panic!("no entry {name}"))
}

fn names(listing: &Listing) -> Vec<String> {
    let entries = listing.entries();
    let mut names: Vec<String> = (0..entries.len())
        .map(|id| entries.name(id).into_string().expect("valid Unicode"))
        .collect();
    names.sort();
    names
}

#[test]
fn lists_georgian_names_with_sizes_dates_and_folders() {
    let dir = temp();
    std::fs::write(dir.path().join("ნახაზი 2.dwg"), b"hello").expect("write");
    std::fs::write(dir.path().join("a.txt"), b"").expect("write");
    std::fs::create_dir(dir.path().join("პროექტი")).expect("mkdir");
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    File::options()
        .write(true)
        .open(dir.path().join("a.txt"))
        .and_then(|file| file.set_modified(modified))
        .expect("set modified");

    let listing = read(dir.path());

    assert_eq!(names(&listing), ["a.txt", "ნახაზი 2.dwg", "პროექტი"]);
    let entries = listing.entries();
    let drawing = find(&listing, "ნახაზი 2.dwg");
    assert_eq!(entries.sizes()[drawing], 5);
    assert!(!entries.is_dir(drawing));
    let folder = find(&listing, "პროექტი");
    assert!(entries.is_dir(folder));
    assert_eq!(entries.sizes()[folder], 0);
    let a = find(&listing, "a.txt");
    assert_eq!(
        filetime_to_unix_ms(entries.modified()[a]),
        1_700_000_000_000.0
    );
    assert!(entries.created()[a] > 0);
    assert_eq!(
        listing.path(drawing),
        Some(long_path(&dir.path().join("ნახაზი 2.dwg")).expect("long path"))
    );
}

#[test]
fn lists_folders_deeper_than_max_path() {
    let dir = temp();
    let mut deep = dir.path().to_path_buf();
    while deep.as_os_str().encode_wide().count() < 300 {
        deep.push("ქართული საქაღალდე");
    }
    std::fs::create_dir_all(&deep).expect("deep folders");
    std::fs::write(deep.join("გეგმა.dwg"), b"plan").expect("write");
    std::fs::create_dir(deep.join("შიგნით")).expect("mkdir");

    // Without the `\\?\` prefix: core adds it.
    let listing = read(&deep);
    assert_eq!(names(&listing), ["გეგმა.dwg", "შიგნით"]);
    let plan = find(&listing, "გეგმა.dwg");
    assert_eq!(listing.entries().sizes()[plan], 4);
    let path = listing.path(plan).expect("path");
    assert!(path.as_os_str().to_string_lossy().starts_with(r"\\?\"));
    assert_eq!(std::fs::read(path).expect("read back"), b"plan");

    // And the parent sees the deep folder.
    let parent = read(deep.parent().expect("parent"));
    assert_eq!(names(&parent), ["ქართული საქაღალდე"]);
}

#[test]
fn an_empty_folder_lists_nothing() {
    let dir = temp();
    let mut batches = 0;
    list(dir.path(), &AtomicBool::new(false), |_| batches += 1).expect("lists");
    assert_eq!(batches, 0);
    assert!(read(dir.path()).entries().is_empty());
}

#[test]
fn a_missing_folder_is_not_found() {
    let dir = temp();
    let missing = dir.path().join("აღარ არის");
    let error = Listing::read(&missing, &AtomicBool::new(false)).expect_err("missing");
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(
        error.path,
        Some(long_path(&missing).expect("long path")),
        "the error names the folder"
    );
}

#[test]
fn a_file_is_not_a_folder() {
    let dir = temp();
    let file = dir.path().join("ფაილი.txt");
    std::fs::write(&file, b"x").expect("write");
    let error = Listing::read(&file, &AtomicBool::new(false)).expect_err("not a folder");
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn a_drive_without_a_volume_is_device_gone() {
    // SAFETY: GetLogicalDrives takes no arguments and only returns a bit mask.
    let drives = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
    let letter = (b'D'..=b'Z')
        .rev()
        .find(|letter| drives & (1 << (letter - b'A')) == 0)
        .expect("a free drive letter");
    let folder = PathBuf::from(format!(r"{}:\Work", char::from(letter)));
    let error = Listing::read(&folder, &AtomicBool::new(false)).expect_err("no volume");
    assert_eq!(error.code, ErrorCode::DeviceGone);
}

/// Denies everyone listing `dir` until dropped (`icacls`, so no Security API in the tests).
struct Denied<'a>(&'a Path);

impl<'a> Denied<'a> {
    fn new(dir: &'a Path) -> Self {
        let status = Command::new("icacls")
            .arg(dir)
            .args(["/deny", "*S-1-1-0:(RD)"])
            .output()
            .expect("icacls runs");
        assert!(status.status.success(), "icacls /deny: {status:?}");
        Self(dir)
    }
}

impl Drop for Denied<'_> {
    fn drop(&mut self) {
        let _ = Command::new("icacls")
            .arg(self.0)
            .args(["/remove:d", "*S-1-1-0"])
            .output();
    }
}

#[test]
fn a_denied_folder_is_access_denied() {
    let dir = temp();
    let locked = dir.path().join("დახურული");
    std::fs::create_dir(&locked).expect("mkdir");
    std::fs::write(locked.join("secret.txt"), b"x").expect("write");

    let error = {
        let _denied = Denied::new(&locked);
        // An elevated process (the CI runner) can list past the deny, so there is no denial to
        // map. `std` lists through `FindFirstFileExW` too, so it tells without the code under test.
        if std::fs::read_dir(&locked).is_ok() {
            eprintln!("skipped: this process lists past a deny ACE");
            return;
        }
        Listing::read(&locked, &AtomicBool::new(false)).expect_err("denied")
    };

    assert_eq!(error.code, ErrorCode::AccessDenied);
    assert_eq!(names(&read(&locked)), ["secret.txt"], "access is back");
}

#[test]
fn a_locked_file_is_listed_without_opening_it() {
    let dir = temp();
    let path = dir.path().join("ღია.xlsx");
    std::fs::write(&path, b"12345678").expect("write");
    // No sharing at all: any open of this file fails while `_held` lives.
    let _held = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .expect("exclusive open");
    assert!(File::open(&path).is_err(), "the file is locked");

    let listing = read(dir.path());

    let id = find(&listing, "ღია.xlsx");
    assert_eq!(listing.entries().sizes()[id], 8);
}

#[test]
fn names_that_are_not_valid_unicode_keep_their_exact_path() {
    let dir = temp();
    // A lone high surrogate: legal on NTFS, not valid UTF-16.
    let units: Vec<u16> = [0xD800].into_iter().chain("x.txt".encode_utf16()).collect();
    let name = OsString::from_wide(&units);
    std::fs::write(dir.path().join(&name), b"odd").expect("write");

    let listing = read(dir.path());

    let entries = listing.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries.name_utf16(0), units);
    assert_eq!(entries.name(0), name);
    let path = listing.path(0).expect("path");
    assert_eq!(std::fs::read(path).expect("read back"), b"odd");
}

fn folder_with(count: usize) -> TempDir {
    let dir = temp();
    for i in 0..count {
        File::create_new(dir.path().join(format!("ფაილი {i}.txt"))).expect("create");
    }
    dir
}

#[test]
fn batches_cover_every_entry_and_the_first_comes_early() {
    let dir = folder_with(1_000);
    let mut sizes = Vec::new();
    let mut listing = Listing::new(long_path(dir.path()).expect("long path"));

    list(dir.path(), &AtomicBool::new(false), |batch| {
        sizes.push(batch.len());
        listing.append(batch);
    })
    .expect("lists");

    assert_eq!(sizes.first(), Some(&FIRST_BATCH));
    assert_eq!(sizes.iter().sum::<usize>(), 1_000);
    assert_eq!(listing.entries().len(), 1_000);
    assert_eq!(names(&listing).len(), 1_000);
    assert_eq!(
        listing,
        read(dir.path()),
        "the same entries in the same order"
    );
}

#[test]
fn cancel_stops_the_listing() {
    let dir = folder_with(FIRST_BATCH * 2);
    let cancel = AtomicBool::new(false);
    let mut batches = 0;

    let result = list(dir.path(), &cancel, |_| {
        batches += 1;
        cancel.store(true, Ordering::Relaxed);
    });

    assert_eq!(
        result.map_err(|error| error.code),
        Err(ErrorCode::Cancelled)
    );
    assert_eq!(batches, 1, "no batch after cancel");
}
