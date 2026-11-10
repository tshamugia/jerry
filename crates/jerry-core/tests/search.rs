//! Deep search on real temp folders: Georgian names, paths past 260 characters, every filter,
//! hidden items, links that are entered only when asked (a junction loop among them), the folders
//! left out, cancelling mid-walk, and what a batch carries.

#![allow(
    clippy::expect_used,
    reason = "test helpers: a failed setup fails the test"
)]

use std::fs::File;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use jerry_core::path::long_path;
use jerry_core::search::{
    DateRange, FileType, SearchBatch, SearchItems, SearchProgress, SearchQuery, SizeRange, search,
};
use jerry_core::{ErrorCode, JerryError};
use tempfile::TempDir;
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_HIDDEN, FILE_FLAGS_AND_ATTRIBUTES, GetFileAttributesW, SetFileAttributesW,
};
use windows::core::PCWSTR;

fn temp() -> TempDir {
    tempfile::Builder::new()
        .prefix("jerry-search-")
        .tempdir()
        .expect("temp folder")
}

/// Makes `relative` under `root`: a folder if it ends in `/`, else a file of `size` bytes, with
/// its folders.
fn make(root: &Path, relative: &str, size: u64) -> PathBuf {
    let path = long_path(&root.join(relative.trim_end_matches('/'))).expect("long path");
    if relative.ends_with('/') {
        std::fs::create_dir_all(&path).expect("mkdir");
    } else {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        File::create(&path)
            .and_then(|file| file.set_len(size))
            .expect("write");
    }
    path
}

fn tree(root: &Path, items: &[&str]) {
    for item in items {
        make(root, item, 10);
    }
}

fn hide(path: &Path) {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` is NUL-terminated and outlives both calls.
    unsafe {
        let attributes = GetFileAttributesW(PCWSTR(wide.as_ptr()));
        SetFileAttributesW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(attributes | FILE_ATTRIBUTE_HIDDEN.0),
        )
        .expect("hide");
    }
}

/// A junction at `link` to `target`. Junctions need no admin rights or developer mode.
fn junction(link: &Path, target: &Path) {
    let plain = |path: &Path| jerry_core::error::display_path(path);
    let made = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(plain(link))
        .arg(plain(target))
        .output()
        .expect("mklink runs");
    assert!(made.status.success(), "mklink /J: {made:?}");
}

/// What a search found: every result as a path from `root` with `/`, sorted, and the progress.
#[derive(Debug)]
struct Found {
    paths: Vec<String>,
    progress: SearchProgress,
}

fn query(name: &str) -> SearchQuery {
    SearchQuery {
        name: name.to_owned(),
        ..SearchQuery::default()
    }
}

fn run(root: &Path, query: &SearchQuery) -> Result<Found, JerryError> {
    run_excluding(root, query, &[])
}

/// Searches and checks every batch against the ones before: ids continue, every result has a
/// folder, and the progress counts what arrived.
fn run_excluding(
    root: &Path,
    query: &SearchQuery,
    excluded: &[PathBuf],
) -> Result<Found, JerryError> {
    let root_long = long_path(root).expect("long path");
    let mut folders: Vec<PathBuf> = Vec::new();
    let mut paths = Vec::new();
    let progress = search(
        root,
        query,
        excluded,
        &AtomicBool::new(false),
        |batch: &SearchBatch, progress| {
            assert_eq!(batch.first, paths.len(), "result ids continue");
            assert_eq!(batch.first_folder, folders.len(), "folder ids continue");
            assert_eq!(batch.folders_of.len(), batch.entries.len());
            folders.extend(batch.folders.iter().cloned());
            for (i, &folder) in batch.folders_of.iter().enumerate() {
                let folder = &folders[folder as usize];
                let path = folder.join(batch.entries.name(i));
                let relative = path.strip_prefix(&root_long).expect("under the root");
                paths.push(relative.to_string_lossy().replace('\\', "/"));
            }
            assert_eq!(progress.found, paths.len() as u64);
        },
    )?;
    assert_eq!(progress.found, paths.len() as u64);
    paths.sort();
    Ok(Found { paths, progress })
}

fn names(root: &Path, query: &SearchQuery) -> Vec<String> {
    run(root, query).expect("search").paths
}

const PROJECT: &[&str] = &[
    "პროექტი/ნახაზები/ნახაზი 1.dwg",
    "პროექტი/ნახაზები/ᲜᲐᲮᲐᲖᲘ 2.DWG",
    "პროექტი/ნახაზები/ძველი/ნახაზი 0.dxf",
    "პროექტი/გეგმა.pdf",
    "პროექტი/ELV-001 LAN Layout GF.dwg",
    "Docs/Report final.docx",
    "Docs/report draft.txt",
    "Docs/photo.JPG",
    "Docs/backup.7z",
    "Docs/clip.mp4",
    "Docs/README",
    "Docs/Empty/",
    "plan_rev2.dwg",
];

#[test]
fn names_match_by_part_wildcard_and_every_word_in_georgian_too() {
    let dir = temp();
    tree(dir.path(), PROJECT);

    assert_eq!(
        names(dir.path(), &query("ნახაზ")),
        [
            "პროექტი/ნახაზები",
            "პროექტი/ნახაზები/ნახაზი 1.dwg",
            "პროექტი/ნახაზები/ძველი/ნახაზი 0.dxf",
            "პროექტი/ნახაზები/ᲜᲐᲮᲐᲖᲘ 2.DWG",
        ],
        "Mtavruli matches Mkhedruli; folders match too"
    );
    assert_eq!(
        names(dir.path(), &query("*.dwg")),
        [
            "plan_rev2.dwg",
            "პროექტი/ELV-001 LAN Layout GF.dwg",
            "პროექტი/ნახაზები/ნახაზი 1.dwg",
            "პროექტი/ნახაზები/ᲜᲐᲮᲐᲖᲘ 2.DWG",
        ]
    );
    assert_eq!(
        names(dir.path(), &query("REPORT final")),
        ["Docs/Report final.docx"]
    );
    assert_eq!(
        names(dir.path(), &query("lan *.dwg")),
        ["plan_rev2.dwg", "პროექტი/ELV-001 LAN Layout GF.dwg"]
    );
    assert_eq!(names(dir.path(), &query("rev?.dwg")), Vec::<String>::new());
    assert_eq!(names(dir.path(), &query("*rev?.dwg")), ["plan_rev2.dwg"]);
    assert!(names(dir.path(), &query("zzz")).is_empty());

    let everything = run(dir.path(), &query("")).expect("search");
    // 12 files and 5 folders.
    assert_eq!(everything.paths.len(), 17);
    assert_eq!(everything.progress.checked, 17);
    assert_eq!(everything.progress.skipped, 0);
}

#[test]
fn types_sizes_dates_and_kinds_filter() {
    let dir = temp();
    tree(dir.path(), PROJECT);
    let typed = |file_type| SearchQuery {
        file_type: Some(file_type),
        ..SearchQuery::default()
    };
    assert_eq!(
        names(dir.path(), &typed(FileType::Drawings)),
        [
            "plan_rev2.dwg",
            "პროექტი/ELV-001 LAN Layout GF.dwg",
            "პროექტი/ნახაზები/ნახაზი 1.dwg",
            "პროექტი/ნახაზები/ძველი/ნახაზი 0.dxf",
            "პროექტი/ნახაზები/ᲜᲐᲮᲐᲖᲘ 2.DWG",
        ]
    );
    assert_eq!(
        names(dir.path(), &typed(FileType::Documents)),
        [
            "Docs/Report final.docx",
            "Docs/report draft.txt",
            "პროექტი/გეგმა.pdf"
        ]
    );
    assert_eq!(
        names(dir.path(), &typed(FileType::Images)),
        ["Docs/photo.JPG"]
    );
    assert_eq!(
        names(dir.path(), &typed(FileType::Archives)),
        ["Docs/backup.7z"]
    );
    assert_eq!(
        names(dir.path(), &typed(FileType::Video)),
        ["Docs/clip.mp4"]
    );
    assert_eq!(
        names(
            dir.path(),
            &typed(FileType::Extension {
                extension: ".DXF".to_owned()
            })
        ),
        ["პროექტი/ნახაზები/ძველი/ნახაზი 0.dxf"]
    );
    let drawings_named = SearchQuery {
        name: "ნახაზი".to_owned(),
        ..typed(FileType::Drawings)
    };
    assert_eq!(
        names(dir.path(), &drawings_named).len(),
        3,
        "a type and a name: both must match"
    );

    make(dir.path(), "Sizes/small.bin", 1_000);
    make(dir.path(), "Sizes/big.bin", 5_000_000);
    let sized = |min, max| SearchQuery {
        name: "*.bin".to_owned(),
        size: Some(SizeRange { min, max }),
        ..SearchQuery::default()
    };
    assert_eq!(
        names(dir.path(), &sized(Some(1e6), None)),
        ["Sizes/big.bin"]
    );
    assert_eq!(
        names(dir.path(), &sized(None, Some(1_000.0))),
        ["Sizes/small.bin"]
    );
    let any_size = names(
        dir.path(),
        &SearchQuery {
            size: Some(SizeRange::default()),
            ..SearchQuery::default()
        },
    );
    assert!(any_size.contains(&"Sizes/big.bin".to_owned()));
    assert!(
        !any_size.contains(&"Sizes".to_owned()) && !any_size.contains(&"Docs/Empty".to_owned()),
        "a size filter leaves folders out"
    );

    // 2020-06-15 and 2025-03-01 noon UTC.
    let at = |seconds: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(seconds);
    let old = make(dir.path(), "Dates/old.txt", 1);
    let new = make(dir.path(), "Dates/new.txt", 1);
    File::options()
        .write(true)
        .open(&old)
        .and_then(|file| file.set_modified(at(1_592_222_400)))
        .expect("date");
    File::options()
        .write(true)
        .open(&new)
        .and_then(|file| file.set_modified(at(1_740_830_400)))
        .expect("date");
    // 2025-01-01 and 2026-01-01, in ms.
    let (year, next_year) = (1_735_689_600_000.0, 1_767_225_600_000.0);
    let dated_files = |from: Option<f64>, to: Option<f64>| {
        let query = SearchQuery {
            modified: Some(DateRange { from, to }),
            items: SearchItems::Files,
            name: ".txt".to_owned(),
            ..SearchQuery::default()
        };
        names(&dir.path().join("Dates"), &query)
    };
    assert_eq!(dated_files(Some(year), Some(next_year)), ["new.txt"]);
    assert_eq!(dated_files(None, Some(year)), ["old.txt"]);
    assert_eq!(dated_files(Some(next_year), None), Vec::<String>::new());

    let kind = |items| SearchQuery {
        items,
        ..query("Docs")
    };
    assert_eq!(names(dir.path(), &kind(SearchItems::Folders)), ["Docs"]);
    assert!(names(dir.path(), &kind(SearchItems::Files)).is_empty());
    let empty_folders = SearchQuery {
        items: SearchItems::Folders,
        ..query("empty")
    };
    assert_eq!(names(dir.path(), &empty_folders), ["Docs/Empty"]);
}

#[test]
fn hidden_items_are_left_out_and_hidden_folders_not_searched_unless_asked() {
    let dir = temp();
    tree(
        dir.path(),
        &[
            "seen.txt",
            "secret.txt",
            "Hidden/inside.txt",
            "Open/also.txt",
        ],
    );
    hide(&long_path(&dir.path().join("secret.txt")).expect("long path"));
    hide(&long_path(&dir.path().join("Hidden")).expect("long path"));

    let without = run(dir.path(), &query(".txt")).expect("search");
    assert_eq!(without.paths, ["Open/also.txt", "seen.txt"]);
    let with = SearchQuery {
        hidden: true,
        ..query(".txt")
    };
    assert_eq!(
        names(dir.path(), &with),
        [
            "Hidden/inside.txt",
            "Open/also.txt",
            "secret.txt",
            "seen.txt"
        ]
    );
    // A hidden folder the user starts in is searched.
    assert_eq!(
        names(&dir.path().join("Hidden"), &query("")),
        ["inside.txt"]
    );
}

#[test]
fn paths_past_260_characters_are_searched() {
    let dir = temp();
    let deep: String = (0..12)
        .map(|i| format!("ფოლდერი ნომერი {i:02} - საკმაოდ გრძელი/"))
        .collect();
    let file = make(dir.path(), &format!("{deep}ბოლო ნახაზი.dwg"), 10);
    assert!(
        jerry_core::error::display_path(&file).chars().count() > 300,
        "the path is long enough"
    );
    let found = names(dir.path(), &query("ბოლო"));
    assert_eq!(found, [format!("{deep}ბოლო ნახაზი.dwg")]);
    // And a search that starts down there.
    let start = file.parent().expect("parent").to_path_buf();
    assert_eq!(names(&start, &query("*.dwg")), ["ბოლო ნახაზი.dwg"]);
}

#[test]
fn links_match_but_are_entered_only_when_asked_and_never_loop() {
    let dir = temp();
    let root = make(dir.path(), "root/", 0);
    let outside = make(dir.path(), "outside/", 0);
    tree(&root, &["a/file.txt"]);
    tree(&outside, &["far.txt"]);
    // A loop back up to the root, and a way out of it.
    junction(&root.join("a").join("loop"), &root);
    junction(&root.join("out"), &outside);

    let plain = run(&root, &query("")).expect("search");
    assert_eq!(plain.paths, ["a", "a/file.txt", "a/loop", "out"]);

    let following = SearchQuery {
        follow_links: true,
        ..query("")
    };
    let followed = run(&root, &following).expect("search");
    assert_eq!(
        followed.paths,
        ["a", "a/file.txt", "a/loop", "out", "out/far.txt"],
        "the loop is entered never, the way out once"
    );

    // Two ways into one folder: its files come once.
    junction(&root.join("again"), &outside);
    let twice = run(&root, &following).expect("search");
    let far = twice
        .paths
        .iter()
        .filter(|path| path.ends_with("far.txt"))
        .count();
    assert_eq!(far, 1, "{:?}", twice.paths);
}

#[test]
fn a_link_that_leads_nowhere_is_skipped() {
    let dir = temp();
    let root = make(dir.path(), "root/", 0);
    let gone = make(dir.path(), "gone/", 0);
    junction(&root.join("dangling"), &gone);
    std::fs::remove_dir(&gone).expect("rmdir");
    let following = SearchQuery {
        follow_links: true,
        ..query("")
    };
    let found = run(&root, &following).expect("search");
    assert_eq!(found.paths, ["dangling"]);
    assert_eq!(found.progress.skipped, 1);
}

#[test]
fn excluded_folders_are_not_searched_unless_the_search_starts_in_one() {
    let dir = temp();
    tree(
        dir.path(),
        &[
            "Windows/System32/kernel.dll",
            "Program Files/App/app.dll",
            "Work/plan.dll",
        ],
    );
    let excluded = [
        // As the Shell writes it: without the long-path prefix, in another case.
        dir.path().join("WINDOWS"),
        dir.path().join("Program Files"),
    ];
    let found = run_excluding(dir.path(), &query(""), &excluded).expect("search");
    assert_eq!(
        found.paths,
        ["Program Files", "Windows", "Work", "Work/plan.dll"],
        "an excluded folder can match, but isn't entered"
    );

    let system = long_path(&dir.path().join("Windows")).expect("long path");
    let inside = run_excluding(&system, &query(".dll"), &excluded).expect("search");
    assert_eq!(inside.paths, ["System32/kernel.dll"]);

    let following = SearchQuery {
        follow_links: true,
        ..query(".dll")
    };
    junction(&dir.path().join("Work").join("sys"), &system);
    let through_link = run_excluding(dir.path(), &following, &excluded).expect("search");
    assert_eq!(
        through_link.paths,
        ["Work/plan.dll"],
        "no link leads into an excluded folder"
    );
}

#[test]
fn a_root_that_is_no_folder_fails_the_search() {
    let dir = temp();
    let file = make(dir.path(), "file.txt", 1);
    let error = run(&file, &query("")).expect_err("a file");
    assert_eq!(error.code, ErrorCode::NotFound);
    let missing = run(&dir.path().join("missing"), &query("")).expect_err("missing");
    assert_eq!(missing.code, ErrorCode::NotFound);
}

#[test]
fn cancelling_stops_the_walk_and_no_batch_follows() {
    let dir = temp();
    let items: Vec<String> = (0..120)
        .flat_map(|folder| (0..25).map(move |file| format!("ბლოკი {folder}/ფაილი {file}.txt")))
        .collect();
    for item in &items {
        make(dir.path(), item, 0);
    }

    let cancel = AtomicBool::new(false);
    let mut batches_after = 0;
    let result = search(dir.path(), &query(""), &[], &cancel, |batch, _| {
        if cancel.load(Ordering::Relaxed) {
            batches_after += 1;
        }
        if !batch.is_empty() {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    assert_eq!(
        result.map_err(|error| error.code),
        Err(ErrorCode::Cancelled)
    );
    assert_eq!(batches_after, 0);

    // Cancelled before it starts: nothing is read.
    let mut called = false;
    let result = search(dir.path(), &query(""), &[], &cancel, |_, _| called = true);
    assert_eq!(
        result.map_err(|error| error.code),
        Err(ErrorCode::Cancelled)
    );
    assert!(!called);

    // Not cancelled: all of it, every folder named once.
    let all = run(dir.path(), &query(".txt")).expect("search");
    assert_eq!(all.paths.len(), 3_000);
    assert_eq!(all.progress.checked, 3_120);
}
