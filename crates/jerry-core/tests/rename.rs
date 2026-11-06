//! Renaming and new folders on real temp folders: Georgian names, a change of case only, names
//! that are taken or not allowed, a file open in another program, and paths past 260 characters.

#![allow(
    clippy::expect_used,
    reason = "test helpers: a failed setup fails the test"
)]

use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use jerry_core::ErrorCode;
use jerry_core::create::create_folder;
use jerry_core::path::long_path;
use jerry_core::rename::rename;
use tempfile::TempDir;

fn temp() -> TempDir {
    tempfile::Builder::new()
        .prefix("jerry-rename-")
        .tempdir()
        .expect("temp folder")
}

/// `name` in `dir`, as a long path, as core keeps paths.
fn at(dir: &Path, name: &str) -> PathBuf {
    long_path(&dir.join(name)).expect("long path")
}

/// The names in `dir` as the folder has them, exact case, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("read folder")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn renames_files_and_folders_keeping_their_content() {
    let dir = temp();
    let file = at(dir.path(), "ნახაზი.dwg");
    std::fs::write(&file, b"drawing").expect("write");
    let folder = at(dir.path(), "პროექტი");
    std::fs::create_dir(&folder).expect("mkdir");
    std::fs::write(folder.join("a.txt"), b"a").expect("write");

    let renamed = rename(&file, "გეგმა v2.dwg").expect("renames the file");
    assert_eq!(renamed, at(dir.path(), "გეგმა v2.dwg"));
    assert_eq!(std::fs::read(&renamed).expect("read"), b"drawing");
    rename(&folder, "Project 2026").expect("renames the folder");

    assert_eq!(names(dir.path()), ["Project 2026", "გეგმა v2.dwg"]);
    assert!(at(dir.path(), "Project 2026").join("a.txt").exists());
}

#[test]
fn a_change_of_case_only_renames() {
    let dir = temp();
    let file = at(dir.path(), "file.txt");
    std::fs::write(&file, b"x").expect("write");
    let folder = at(dir.path(), "drawings");
    std::fs::create_dir(&folder).expect("mkdir");

    rename(&file, "File.TXT").expect("case-only rename");
    rename(&folder, "Drawings").expect("case-only rename of a folder");

    assert_eq!(names(dir.path()), ["Drawings", "File.TXT"]);
}

#[test]
fn the_same_name_changes_nothing() {
    let dir = temp();
    let file = at(dir.path(), "same.txt");
    std::fs::write(&file, b"x").expect("write");
    assert_eq!(rename(&file, "same.txt"), Ok(file.clone()));
    assert_eq!(names(dir.path()), ["same.txt"]);
}

#[test]
fn a_name_another_item_has_is_refused_never_replaced() {
    let dir = temp();
    let file = at(dir.path(), "a.txt");
    std::fs::write(&file, b"mine").expect("write");
    let other = at(dir.path(), "B.txt");
    std::fs::write(&other, b"other").expect("write");
    std::fs::create_dir(at(dir.path(), "folder")).expect("mkdir");

    let error = rename(&file, "b.txt").expect_err("taken, in another case");
    assert_eq!(error.code, ErrorCode::InvalidName);
    assert_eq!(error.path, Some(at(dir.path(), "b.txt")));
    let error = rename(&file, "Folder").expect_err("a folder has it");
    assert_eq!(error.code, ErrorCode::InvalidName);

    assert_eq!(std::fs::read(&other).expect("read"), b"other");
    assert_eq!(std::fs::read(&file).expect("read"), b"mine");
}

#[test]
fn names_that_break_a_rule_are_refused_before_anything_happens() {
    let dir = temp();
    let file = at(dir.path(), "a.txt");
    std::fs::write(&file, b"x").expect("write");
    // `\\?\` paths would let Windows make most of these; the rules stop them first.
    for name in [
        "",
        "a:b",
        r"sub\b.txt",
        "CON",
        "nul.txt",
        "trailing.",
        "trailing ",
    ] {
        let error = rename(&file, name).expect_err(name);
        assert_eq!(error.code, ErrorCode::InvalidName, "{name:?}");
        assert_eq!(error.path, None);
    }
    assert_eq!(names(dir.path()), ["a.txt"]);
}

#[test]
fn a_file_open_in_another_program_is_in_use() {
    let dir = temp();
    let file = at(dir.path(), "ცხრილი.xlsx");
    std::fs::write(&file, b"x").expect("write");
    // Opened as Excel does: no delete sharing, so it can't be renamed.
    let _held = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&file)
        .expect("lock");

    let error = rename(&file, "ცხრილი 2.xlsx").expect_err("in use");
    assert_eq!(error.code, ErrorCode::InUse { app: None });
    assert_eq!(error.path, Some(file));
}

#[test]
fn an_item_that_is_gone_is_not_found() {
    let dir = temp();
    let error = rename(&at(dir.path(), "gone.txt"), "new.txt").expect_err("gone");
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn renames_past_260_characters() {
    let dir = temp();
    let mut deep = long_path(dir.path()).expect("long path");
    for _ in 0..6 {
        deep.push("ქვესაქაღალდე-ძალიან-გრძელი-სახელით-0123456789");
    }
    std::fs::create_dir_all(&deep).expect("mkdir deep");
    assert!(deep.as_os_str().len() > 260);
    let file = deep.join("ნახაზი.dwg");
    std::fs::write(&file, b"x").expect("write");

    let renamed = rename(&file, "ახალი სახელი.dwg").expect("renames");
    assert!(renamed.exists());
    assert!(!file.exists());
}

#[test]
fn new_folders_are_numbered_as_in_explorer() {
    let dir = temp();
    let folder = long_path(dir.path()).expect("long path");

    assert_eq!(
        create_folder(&folder, "New folder").expect("first"),
        "New folder"
    );
    assert_eq!(
        create_folder(&folder, "New folder").expect("second"),
        "New folder (2)"
    );
    // A file with the next name counts as taken.
    std::fs::write(folder.join("New folder (3)"), b"x").expect("write");
    assert_eq!(
        create_folder(&folder, "New folder").expect("fourth"),
        "New folder (4)"
    );
    assert_eq!(
        create_folder(&folder, "ახალი საქაღალდე").expect("Georgian"),
        "ახალი საქაღალდე"
    );
    assert!(folder.join("New folder (4)").is_dir());
    assert_eq!(
        names(dir.path()),
        [
            "New folder",
            "New folder (2)",
            "New folder (3)",
            "New folder (4)",
            "ახალი საქაღალდე"
        ]
    );
}

#[test]
fn a_new_folder_needs_a_valid_name_and_a_folder_that_is_there() {
    let dir = temp();
    let folder = long_path(dir.path()).expect("long path");
    let error = create_folder(&folder, "a|b").expect_err("invalid");
    assert_eq!(error.code, ErrorCode::InvalidName);

    let gone = folder.join("gone");
    let error = create_folder(&gone, "New folder").expect_err("no folder");
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(error.path, Some(gone));
}
