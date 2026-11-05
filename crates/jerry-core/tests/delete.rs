//! Permanent delete on real temp folders: Georgian names, paths past 260 characters, read-only
//! files and folders, a locked file, links that must not be followed, and counting what a delete
//! would remove.

#![allow(
    clippy::expect_used,
    reason = "test helpers: a failed setup fails the test"
)]

use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use jerry_core::ErrorCode;
use jerry_core::delete::{Tally, delete_permanently, measure};
use jerry_core::path::long_path;
use tempfile::TempDir;

fn temp() -> TempDir {
    tempfile::Builder::new()
        .prefix("jerry-delete-")
        .tempdir()
        .expect("temp folder")
}

/// `name` in `dir`, as a long path, as core keeps paths.
fn at(dir: &Path, name: &str) -> PathBuf {
    long_path(&dir.join(name)).expect("long path")
}

fn write(path: &Path, bytes: usize) {
    std::fs::write(path, vec![b'x'; bytes]).expect("write");
}

fn set_read_only(path: &Path, read_only: bool) {
    let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
    permissions.set_readonly(read_only);
    std::fs::set_permissions(path, permissions).expect("set read-only");
}

fn is_read_only(path: &Path) -> bool {
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .readonly()
}

fn tally(paths: &[PathBuf]) -> Tally {
    measure(paths, &AtomicBool::new(false), |_| {}).expect("measures")
}

#[test]
fn deletes_georgian_files_and_folders_with_everything_in_them() {
    let dir = temp();
    let project = at(dir.path(), "პროექტი");
    std::fs::create_dir_all(project.join("ნახაზები").join("ძველი")).expect("mkdir");
    write(&project.join("ნახაზი 1.dwg"), 100);
    write(&project.join("ნახაზები").join("გეგმა.pdf"), 20);
    write(&project.join("ნახაზები").join("ძველი").join("ა.txt"), 3);
    let file = at(dir.path(), "ცხრილი.xlsx");
    write(&file, 7);
    let kept = at(dir.path(), "დარჩება.txt");
    write(&kept, 1);

    let items = [project.clone(), file.clone()];
    assert_eq!(
        tally(&items),
        Tally {
            files: 4,
            folders: 3,
            bytes: 130,
        }
    );
    let results = delete_permanently(&items);
    assert_eq!(results, [Ok(()), Ok(())]);
    assert!(!project.exists());
    assert!(!file.exists());
    assert!(kept.exists(), "only what was asked for goes");
}

#[test]
fn deletes_past_260_characters() {
    let dir = temp();
    let segment = "ს".repeat(60);
    let top = at(dir.path(), "გრძელი");
    let mut deep = top.clone();
    for _ in 0..5 {
        deep.push(&segment);
    }
    std::fs::create_dir_all(&deep).expect("mkdir");
    let file = deep.join("ფაილი.txt");
    write(&file, 5);
    let second = deep.join("მეორე.txt");
    write(&second, 5);
    assert!(
        jerry_core::error::display_path(&file).chars().count() > 300,
        "past MAX_PATH"
    );

    // A long file on its own, then the folder with everything in it.
    assert_eq!(delete_permanently(std::slice::from_ref(&second)), [Ok(())]);
    assert!(!second.exists());
    assert_eq!(tally(std::slice::from_ref(&top)).files, 1);
    assert_eq!(delete_permanently(std::slice::from_ref(&top)), [Ok(())]);
    assert!(!top.exists());
}

#[test]
fn deletes_read_only_files_and_folders() {
    let dir = temp();
    let folder = at(dir.path(), "read-only");
    std::fs::create_dir(&folder).expect("mkdir");
    let inner = folder.join("inner.txt");
    write(&inner, 1);
    set_read_only(&inner, true);
    set_read_only(&folder, true);
    let file = at(dir.path(), "file.txt");
    write(&file, 1);
    set_read_only(&file, true);

    assert_eq!(
        delete_permanently(&[folder.clone(), file.clone()]),
        [Ok(()), Ok(())]
    );
    assert!(!folder.exists());
    assert!(!file.exists());
}

#[test]
fn a_locked_file_stays_and_everything_else_goes() {
    let dir = temp();
    let folder = at(dir.path(), "ფოლდერი");
    std::fs::create_dir_all(folder.join("sub")).expect("mkdir");
    write(&folder.join("a.txt"), 1);
    write(&folder.join("sub").join("b.txt"), 1);
    let locked = folder.join("დაკავებული.dwg");
    write(&locked, 1);
    let read_only_locked = folder.join("read-only.dwg");
    write(&read_only_locked, 1);
    set_read_only(&read_only_locked, true);

    {
        // As AutoCAD holds an open drawing: no sharing at all.
        let _held = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&locked)
            .expect("lock");
        let _held_too = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&read_only_locked)
            .expect("lock");

        let results = delete_permanently(std::slice::from_ref(&folder));
        let error = results[0].clone().expect_err("a file is in use");
        assert_eq!(error.code, ErrorCode::InUse { app: None });
        let path = error.path.expect("names the file");
        assert!(
            path == locked || path == read_only_locked,
            "names a locked file: {path:?}"
        );

        assert!(locked.exists() && read_only_locked.exists());
        assert!(
            is_read_only(&read_only_locked),
            "a file that stays keeps its attribute"
        );
        assert!(folder.exists(), "the folder holding it stays");
        assert!(!folder.join("a.txt").exists());
        assert!(!folder.join("sub").exists());
    }

    // Once the program lets go, the rest goes too.
    assert_eq!(delete_permanently(std::slice::from_ref(&folder)), [Ok(())]);
    assert!(!folder.exists());
}

/// A junction at `link` to `target`. Junctions need no admin rights or developer mode.
fn junction(link: &Path, target: &Path) {
    let plain = |path: &Path| jerry_core::error::display_path(path);
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(plain(link))
        .arg(plain(target))
        .output()
        .expect("mklink runs");
    assert!(status.status.success(), "mklink /J: {status:?}");
}

#[test]
fn links_are_deleted_and_never_followed() {
    let dir = temp();
    let target = at(dir.path(), "target");
    std::fs::create_dir(&target).expect("mkdir");
    let precious = target.join("precious.dwg");
    write(&precious, 10);

    let folder = at(dir.path(), "folder");
    std::fs::create_dir(&folder).expect("mkdir");
    write(&folder.join("own.txt"), 1);
    junction(&folder.join("link"), &target);
    let top_link = at(dir.path(), "top-link");
    junction(&top_link, &target);

    assert_eq!(
        tally(&[folder.clone(), top_link.clone()]),
        Tally {
            files: 1,
            folders: 3,
            bytes: 1,
        },
        "links count as themselves, not what they point to"
    );
    assert_eq!(
        delete_permanently(&[folder.clone(), top_link.clone()]),
        [Ok(()), Ok(())]
    );
    assert!(!folder.exists());
    assert!(std::fs::symlink_metadata(&top_link).is_err());
    assert!(precious.exists(), "what a link points to stays");
}

#[test]
fn missing_items_and_drive_roots_are_errors() {
    let dir = temp();
    let missing = at(dir.path(), "გამქრალი.txt");
    let results = delete_permanently(std::slice::from_ref(&missing));
    assert_eq!(
        results[0].clone().map_err(|error| error.code),
        Err(ErrorCode::NotFound)
    );

    // A root is never an item. A free letter, so nothing could ever be touched.
    let letter = (b'J'..=b'Z')
        .rev()
        .map(char::from)
        .find(|letter| !Path::new(&format!(r"{letter}:\")).exists())
        .expect("a free drive letter");
    let root = PathBuf::from(format!(r"\\?\{letter}:\"));
    let results = delete_permanently(std::slice::from_ref(&root));
    assert_eq!(
        results[0].clone().map_err(|error| error.code),
        Err(ErrorCode::InvalidName)
    );
}

#[test]
fn measuring_reports_progress_and_stops_when_cancelled() {
    let dir = temp();
    let folder = at(dir.path(), "many");
    std::fs::create_dir(&folder).expect("mkdir");
    for n in 0..50 {
        let sub = folder.join(format!("{n}"));
        std::fs::create_dir(&sub).expect("mkdir");
        File::create(sub.join("f.txt")).expect("create");
    }
    let items = [folder];
    assert_eq!(
        tally(&items),
        Tally {
            files: 50,
            folders: 51,
            bytes: 0,
        }
    );
    let cancelled = measure(&items, &AtomicBool::new(true), |_| {});
    assert_eq!(
        cancelled.map_err(|error| error.code),
        Err(ErrorCode::Cancelled)
    );
}
