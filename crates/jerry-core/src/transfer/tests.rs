//! Moves across volumes, on one volume: renaming is turned off, because a test machine may have
//! only one disk. Everything else is in `tests/transfer.rs`.

use std::fs::{self, File, OpenOptions};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use windows::Win32::Foundation::{ERROR_DEVICE_NOT_CONNECTED, ERROR_NOT_SAME_DEVICE};
use windows::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_WRITE_ATTRIBUTES};

use super::*;
use crate::ErrorCode;
use crate::path::long_path;

const COPYING: Options = Options {
    rename: false,
    mover: crate::rename::move_file,
    copier: copy::copy_file,
};

struct Scene {
    _dir: TempDir,
    from: PathBuf,
    to: PathBuf,
}

fn scene() -> Scene {
    let dir = tempfile::Builder::new()
        .prefix("jerry-move-")
        .tempdir()
        .unwrap();
    let root = long_path(dir.path()).unwrap();
    let (from, to) = (root.join("from"), root.join("to"));
    fs::create_dir(&from).unwrap();
    fs::create_dir(&to).unwrap();
    Scene {
        _dir: dir,
        from,
        to,
    }
}

fn job(sources: &[&Path], destination: &Path) -> Job {
    Job {
        kind: Kind::Move,
        sources: sources.iter().map(|source| source.to_path_buf()).collect(),
        destination: destination.to_owned(),
    }
}

fn move_all(job: &Job) -> Summary {
    run_with(job, COPYING, &Control::new(), &mut |_| {}).unwrap()
}

fn write(path: &Path, bytes: usize) {
    fs::write(path, vec![b'x'; bytes]).unwrap();
}

/// Every name in `folder`, recursively, hidden ones too.
fn names(folder: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut folders = vec![folder.to_owned()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let entry = entry.unwrap();
            found.push(entry.file_name().to_string_lossy().into_owned());
            if entry.file_type().unwrap().is_dir() {
                folders.push(entry.path());
            }
        }
    }
    found.sort();
    found
}

fn empty(folder: &Path) -> bool {
    names(folder).is_empty()
}

#[test]
fn a_moved_file_leaves_its_source_only_after_the_copy_is_done() {
    let scene = scene();
    let source = scene.from.join("ნახაზი.dwg");
    write(&source, 3_000_000);
    let target = scene.to.join("ნახაზი.dwg");
    let part = scene.to.join("ნახაზი.dwg.jerrypart");
    let mut seen = 0;
    let summary = run_with(
        &job(&[&source], &scene.to),
        COPYING,
        &Control::new(),
        &mut |event| {
            if let Event::Progress(progress) = event
                && progress.files_done == 0
            {
                seen += 1;
                assert!(
                    source.exists(),
                    "the source is there until the copy is done"
                );
                assert!(!target.exists(), "no final name while writing");
                assert!(part.exists(), "written under the temporary name");
            }
        },
    )
    .unwrap();
    assert!(seen > 0);
    assert_eq!(summary.status, Status::Done);
    assert_eq!(summary.failed, []);
    assert_eq!((summary.files, summary.bytes), (1, 3_000_000));
    assert!(!source.exists());
    assert_eq!(fs::metadata(&target).unwrap().len(), 3_000_000);
    assert_eq!(names(&scene.to), ["ნახაზი.dwg"]);
}

#[test]
fn a_moved_file_keeps_its_dates_and_attributes() {
    let scene = scene();
    let source = scene.from.join("old.txt");
    write(&source, 10);
    let before = fs::metadata(&source).unwrap();
    let mut permissions = before.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&source, permissions).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));

    let summary = move_all(&job(&[&source], &scene.to));
    assert_eq!(summary.failed, []);
    assert!(!source.exists(), "a read-only source goes too");
    let after = fs::metadata(scene.to.join("old.txt")).unwrap();
    assert_eq!(after.creation_time(), before.creation_time());
    assert_eq!(after.last_write_time(), before.last_write_time());
    assert!(after.permissions().readonly());
}

#[test]
fn a_moved_folder_goes_with_empty_folders_and_keeps_its_date() {
    let scene = scene();
    let project = scene.from.join("პროექტი");
    fs::create_dir_all(project.join("ცარიელი")).unwrap();
    fs::create_dir(project.join("sub")).unwrap();
    write(&project.join("a.txt"), 5);
    write(&project.join("sub").join("b.txt"), 7);
    std::thread::sleep(std::time::Duration::from_millis(20));
    let written = fs::metadata(&project).unwrap().last_write_time();

    let summary = move_all(&job(&[&project], &scene.to));
    assert_eq!(summary.status, Status::Done);
    assert_eq!(summary.failed, []);
    assert_eq!((summary.files, summary.folders, summary.bytes), (2, 3, 12));
    assert!(empty(&scene.from), "everything left the source");
    assert_eq!(
        names(&scene.to),
        ["a.txt", "b.txt", "sub", "პროექტი", "ცარიელი"]
    );
    let moved = scene.to.join("პროექტი");
    assert_eq!(fs::metadata(&moved).unwrap().last_write_time(), written);
}

#[test]
fn a_source_that_changes_during_the_copy_stays() {
    let scene = scene();
    let source = scene.from.join("live.txt");
    write(&source, 3_000_000);
    let mut touched = false;
    let summary = run_with(
        &job(&[&source], &scene.to),
        COPYING,
        &Control::new(),
        &mut |event| {
            if matches!(event, Event::Progress(_)) && !touched {
                touched = true;
                std::thread::sleep(std::time::Duration::from_millis(20));
                OpenOptions::new()
                    .access_mode(FILE_WRITE_ATTRIBUTES.0)
                    .open(&source)
                    .unwrap()
                    .set_modified(std::time::SystemTime::now())
                    .unwrap();
            }
        },
    )
    .unwrap();
    assert_eq!(summary.failed.len(), 1);
    assert_eq!(summary.failed[0].error.code, ErrorCode::InUse { app: None });
    assert!(!summary.failed[0].copied);
    assert_eq!(summary.files, 0);
    assert_eq!(fs::metadata(&source).unwrap().len(), 3_000_000);
    assert!(empty(&scene.to), "no copy and no temporary file");
}

#[test]
fn cancelling_leaves_every_file_in_exactly_one_place() {
    let scene = scene();
    let project = scene.from.join("project");
    let deep = project.join("deep");
    fs::create_dir_all(&deep).unwrap();
    write(&project.join("first.bin"), 2_000_000);
    write(&project.join("second.bin"), 2_000_000);
    write(&deep.join("third.bin"), 2_000_000);
    let control = Control::new();
    let summary = run_with(
        &job(&[&project], &scene.to),
        COPYING,
        &control,
        &mut |event| {
            if let Event::Progress(progress) = event
                && progress.files_done == 1
            {
                control.cancel();
            }
        },
    )
    .unwrap();
    assert_eq!(summary.status, Status::Cancelled);
    assert_eq!(summary.files, 1);

    let moved = names(&scene.to);
    let stayed = names(&scene.from);
    assert!(
        !moved.iter().any(|name| name.ends_with(".jerrypart")),
        "{moved:?}"
    );
    for file in ["first.bin", "second.bin", "third.bin"] {
        let places = u32::from(moved.iter().any(|name| name == file))
            + u32::from(stayed.iter().any(|name| name == file));
        assert_eq!(places, 1, "{file}: {moved:?} / {stayed:?}");
    }
    assert!(project.exists(), "the folder of what stayed stays");
    assert!(
        !scene.to.join("project").join("deep").exists(),
        "a folder not reached isn't made"
    );
}

#[test]
fn a_cancelled_first_file_leaves_no_empty_folders_behind() {
    let scene = scene();
    let project = scene.from.join("project");
    fs::create_dir_all(project.join("a").join("b")).unwrap();
    write(&project.join("a").join("b").join("only.bin"), 2_000_000);
    let control = Control::new();
    let summary = run_with(
        &job(&[&project], &scene.to),
        COPYING,
        &control,
        &mut |event| {
            if matches!(event, Event::Progress(_)) {
                control.cancel();
            }
        },
    )
    .unwrap();
    assert_eq!(summary.status, Status::Cancelled);
    assert!(empty(&scene.to), "{:?}", names(&scene.to));
    assert_eq!(
        fs::metadata(project.join("a").join("b").join("only.bin"))
            .unwrap()
            .len(),
        2_000_000
    );
}

/// Holds `path` so that copying it fails.
fn lock(path: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

#[test]
fn a_locked_file_stays_with_its_folders_and_a_retry_moves_it() {
    let scene = scene();
    let project = scene.from.join("project");
    let deep = project.join("deep");
    fs::create_dir_all(&deep).unwrap();
    write(&project.join("free.txt"), 3);
    let locked = deep.join("locked.txt");
    write(&locked, 4);
    let held = lock(&locked);

    let summary = move_all(&job(&[&project], &scene.to));
    assert_eq!(summary.status, Status::Done);
    assert_eq!(summary.files, 1);
    assert_eq!(summary.failed.len(), 1);
    assert_eq!(summary.failed[0].error.code, ErrorCode::InUse { app: None });
    assert_eq!(summary.failed[0].source, locked);
    assert!(!project.join("free.txt").exists(), "the rest moved");
    assert!(locked.exists());
    assert!(scene.to.join("project").join("deep").exists());

    drop(held);
    let again = retry(&summary, &Control::new(), |_| {}).unwrap();
    assert_eq!(again.status, Status::Done);
    assert_eq!(again.failed, []);
    assert_eq!((again.files, again.bytes), (1, 4));
    assert!(empty(&scene.from), "{:?}", names(&scene.from));
    assert_eq!(
        names(&scene.to),
        ["deep", "free.txt", "locked.txt", "project"]
    );
}

#[test]
fn a_source_that_can_not_be_deleted_keeps_both_and_a_retry_finishes() {
    let scene = scene();
    let source = scene.from.join("held.txt");
    write(&source, 9);
    // Others may read it, as the copy does, but nobody may delete it.
    let held = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(&source)
        .unwrap();

    let summary = move_all(&job(&[&source], &scene.to));
    assert_eq!(summary.failed.len(), 1);
    assert!(summary.failed[0].copied);
    assert_eq!(summary.files, 0);
    assert!(source.exists());
    assert_eq!(fs::metadata(scene.to.join("held.txt")).unwrap().len(), 9);

    drop(held);
    let again = retry(&summary, &Control::new(), |_| {}).unwrap();
    assert_eq!(again.failed, []);
    assert_eq!(again.files, 1);
    assert!(!source.exists());
    assert_eq!(names(&scene.to), ["held.txt"]);
}

#[test]
fn a_retry_never_deletes_a_source_that_differs_from_its_copy() {
    let scene = scene();
    let source = scene.from.join("held.txt");
    write(&source, 9);
    let held = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(&source)
        .unwrap();
    let summary = move_all(&job(&[&source], &scene.to));
    assert!(summary.failed[0].copied);
    drop(held);

    write(&source, 12);
    let again = retry(&summary, &Control::new(), |_| {}).unwrap();
    assert_eq!(again.files, 0);
    assert_eq!(again.failed.len(), 1);
    assert_eq!(
        fs::metadata(&source).unwrap().len(),
        12,
        "the newer source stays"
    );
}

#[test]
fn a_moved_junction_is_the_junction_and_its_target_stays() {
    let scene = scene();
    let real = scene.from.join("real");
    fs::create_dir(&real).unwrap();
    write(&real.join("inside.txt"), 2);
    let link = scene.from.join("link");
    let plain = |path: &Path| crate::path::plain_path(path);
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(plain(&link))
        .arg(plain(&real))
        .output()
        .unwrap();
    assert!(made.status.success());

    let summary = move_all(&job(&[&link], &scene.to));
    assert_eq!(summary.failed, []);
    assert!(fs::symlink_metadata(&link).is_err(), "the link moved");
    assert!(real.join("inside.txt").exists(), "what it points to didn't");
    let moved = scene.to.join("link");
    assert!(
        fs::symlink_metadata(&moved)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::canonicalize(&moved).unwrap(),
        fs::canonicalize(&real).unwrap(),
        "and it still leads there"
    );
}

#[test]
fn a_rename_refused_for_another_volume_copies_instead() {
    let scene = scene();
    let project = scene.from.join("project");
    fs::create_dir(&project).unwrap();
    write(&project.join("a.txt"), 5);
    let file = scene.from.join("b.txt");
    write(&file, 7);
    let refused = Options {
        mover: |_, _| Err(ERROR_NOT_SAME_DEVICE.into()),
        ..Options::default()
    };
    let mut last = None;
    let summary = run_with(
        &job(&[&project, &file], &scene.to),
        refused,
        &Control::new(),
        &mut |event| {
            if let Event::Progress(progress) = event {
                last = Some((
                    progress.files_done,
                    progress.files_total,
                    progress.bytes_done,
                    progress.bytes_total,
                ));
            }
        },
    )
    .unwrap();
    assert_eq!(summary.status, Status::Done);
    assert_eq!(summary.failed, []);
    assert_eq!((summary.files, summary.folders, summary.bytes), (2, 1, 12));
    assert!(empty(&scene.from));
    assert_eq!(names(&scene.to), ["a.txt", "b.txt", "project"]);
    assert_eq!(last, Some((2, 2, 12, 12)), "the totals grew to match");
}

/// Copies, but the device goes away at the file called `b.txt`.
fn device_gone_at_b(
    request: &copy::Request<'_>,
    chunk: &mut dyn FnMut(u64) -> control::Flow,
) -> Result<u64, JerryError> {
    if request.source.ends_with("b.txt") {
        return Err(JerryError::new(ErrorCode::DeviceGone).at(request.source));
    }
    copy::copy_file(request, chunk)
}

#[test]
fn a_volume_going_away_ends_the_job_and_a_retry_carries_on() {
    let scene = scene();
    let project = scene.from.join("project");
    let sub = project.join("sub");
    fs::create_dir_all(&sub).unwrap();
    write(&project.join("a.txt"), 1);
    write(&project.join("b.txt"), 2);
    write(&project.join("c.txt"), 3);
    write(&sub.join("d.txt"), 4);
    let other = scene.from.join("other.txt");
    write(&other, 5);

    let options = Options {
        copier: device_gone_at_b,
        ..COPYING
    };
    let summary = run_with(
        &job(&[&project, &other], &scene.to),
        options,
        &Control::new(),
        &mut |_| {},
    )
    .unwrap();
    assert!(matches!(
        summary.status,
        Status::Interrupted(JerryError {
            code: ErrorCode::DeviceGone,
            ..
        })
    ));
    assert_eq!(summary.failed.len(), 1, "not one failure per file left");
    assert_eq!(summary.failed[0].source, project.join("b.txt"));
    assert_eq!(summary.files, 1);
    assert!(!project.join("a.txt").exists(), "a.txt had moved");
    assert!(
        project.join("c.txt").exists() && other.exists(),
        "the rest waits"
    );
    assert_eq!(names(&scene.to), ["a.txt", "project"]);

    let mut back = summary.clone();
    back.options = COPYING;
    let again = retry(&back, &Control::new(), |_| {}).unwrap();
    assert_eq!(again.status, Status::Done);
    assert_eq!(again.failed, []);
    assert_eq!((again.files, again.bytes), (4, 14));
    assert!(empty(&scene.from), "{:?}", names(&scene.from));
    assert_eq!(
        names(&scene.to),
        [
            "a.txt",
            "b.txt",
            "c.txt",
            "d.txt",
            "other.txt",
            "project",
            "sub"
        ]
    );
}

#[test]
fn the_rest_of_an_interrupted_job_waits_for_a_retry() {
    let scene = scene();
    let (one, two) = (scene.from.join("one.txt"), scene.from.join("two.txt"));
    write(&one, 1);
    write(&two, 2);
    let gone = Options {
        mover: |_, _| Err(ERROR_DEVICE_NOT_CONNECTED.into()),
        ..Options::default()
    };
    let summary = run_with(
        &job(&[&one, &two], &scene.to),
        gone,
        &Control::new(),
        &mut |_| {},
    )
    .unwrap();
    assert!(matches!(
        summary.status,
        Status::Interrupted(JerryError {
            code: ErrorCode::DeviceGone,
            ..
        })
    ));
    assert_eq!(summary.failed.len(), 1, "not one failure per file left");
    assert!(one.exists() && two.exists());

    // The device is back, and the retry renames for real this time.
    let mut resumed = summary.clone();
    resumed.options = Options::default();
    let again = retry(&resumed, &Control::new(), |_| {}).unwrap();
    assert_eq!(again.status, Status::Done);
    assert_eq!(again.failed, []);
    assert_eq!(again.files, 2);
    assert!(empty(&scene.from));
    assert_eq!(names(&scene.to), ["one.txt", "two.txt"]);
}
