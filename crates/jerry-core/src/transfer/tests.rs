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

/// No Recycle Bin: what Replace would replace stays, and the user is asked again.
fn keep(path: &Path) -> Result<(), JerryError> {
    Err(JerryError::new(ErrorCode::NoRecycleBin).at(path))
}

fn move_all(job: &Job) -> Summary {
    run_with(job, COPYING, &Control::new(), &keep, &mut |_| {}).unwrap()
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
        &keep,
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
        &keep,
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
        &keep,
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
        &keep,
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
    let again = retry(&summary, &Control::new(), &keep, |_| {}).unwrap();
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
    let again = retry(&summary, &Control::new(), &keep, |_| {}).unwrap();
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
    let again = retry(&summary, &Control::new(), &keep, |_| {}).unwrap();
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
        &keep,
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
    step: &mut dyn FnMut(copy::Step<'_>) -> control::Flow,
) -> Result<copy::Written, JerryError> {
    if request.source.ends_with("b.txt") {
        return Err(JerryError::new(ErrorCode::DeviceGone).at(request.source));
    }
    copy::copy_file(request, step)
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
        &keep,
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
    let again = retry(&back, &Control::new(), &keep, |_| {}).unwrap();
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
        &keep,
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
    let again = retry(&resumed, &Control::new(), &keep, |_| {}).unwrap();
    assert_eq!(again.status, Status::Done);
    assert_eq!(again.failed, []);
    assert_eq!(again.files, 2);
    assert!(empty(&scene.from));
    assert_eq!(names(&scene.to), ["one.txt", "two.txt"]);
}

/// Resumes `job`, skipping what it finds taken, and returns its summary, the names of the files
/// it began to write and the names it found taken.
fn resume_it(job: &Job, resume: &Resume, control: &Control) -> (Summary, Vec<String>, Vec<String>) {
    let (mut written, mut taken) = (Vec::new(), Vec::new());
    let name = |path: &Path| path.file_name().unwrap().to_string_lossy().into_owned();
    let summary = resume_with(
        job,
        resume,
        COPYING,
        control,
        &keep,
        &mut |event| match event {
            Event::Writing(writing) => written.push(name(writing.source)),
            Event::Conflict(conflict) => {
                taken.push(name(conflict.target));
                control.answer(Answer {
                    choice: Choice::Skip,
                    all: false,
                });
            }
            _ => {}
        },
    )
    .unwrap();
    written.sort();
    (summary, written, taken)
}

#[test]
fn a_resumed_copy_writes_only_what_is_not_there() {
    let scene = scene();
    let project = scene.from.join("პროექტი");
    let sub = project.join("sub");
    fs::create_dir_all(&sub).unwrap();
    write(&project.join("a.txt"), 1);
    write(&project.join("b.txt"), 2);
    write(&project.join("c.txt"), 3);
    write(&sub.join("d.txt"), 4);
    // What the Jerry before did: the folder, a.txt, and b.txt as it stopped. c.txt is someone
    // else's file of that name.
    let copied = scene.to.join("პროექტი");
    fs::create_dir(&copied).unwrap();
    fs::copy(project.join("a.txt"), copied.join("a.txt")).unwrap();
    fs::copy(project.join("b.txt"), copied.join("b.txt")).unwrap();
    write(&copied.join("c.txt"), 9);
    let job = Job {
        kind: Kind::Copy,
        sources: vec![project.clone()],
        destination: scene.to.clone(),
    };
    let resume = Resume {
        writing: Some((project.join("b.txt"), copied.join("b.txt"))),
    };

    let (summary, written, taken) = resume_it(&job, &resume, &Control::new());
    assert_eq!(summary.status, Status::Done);
    assert_eq!(written, ["d.txt"], "a.txt and b.txt are there");
    assert_eq!(taken, ["c.txt"], "someone else's file is a conflict");
    assert_eq!(summary.failed, []);
    assert_eq!(summary.skipped, 1);
    assert_eq!(
        fs::metadata(copied.join("c.txt")).unwrap().len(),
        9,
        "skipped, so never replaced"
    );
    assert_eq!(
        names(&scene.to),
        ["a.txt", "b.txt", "c.txt", "d.txt", "sub", "პროექტი"]
    );
    assert_eq!(
        names(&scene.from),
        names(&scene.to),
        "the copy left the source"
    );
}

#[test]
fn a_resumed_move_removes_only_the_source_it_was_writing() {
    let scene = scene();
    let project = scene.from.join("project");
    fs::create_dir(&project).unwrap();
    write(&project.join("b.txt"), 2);
    write(&project.join("c.txt"), 3);
    write(&project.join("d.txt"), 4);
    // a.txt and other.txt were moved before Jerry stopped; b.txt's copy was complete and its
    // source not yet removed; c.txt is there with the same size and date, but not by this job.
    let moved = scene.to.join("project");
    fs::create_dir(&moved).unwrap();
    write(&moved.join("a.txt"), 1);
    fs::copy(project.join("b.txt"), moved.join("b.txt")).unwrap();
    fs::copy(project.join("c.txt"), moved.join("c.txt")).unwrap();
    let other = scene.from.join("other.txt");
    write(&scene.to.join("other.txt"), 5);
    let job = Job {
        kind: Kind::Move,
        sources: vec![project.clone(), other],
        destination: scene.to.clone(),
    };
    let resume = Resume {
        writing: Some((project.join("b.txt"), moved.join("b.txt"))),
    };

    let (summary, written, taken) = resume_it(&job, &resume, &Control::new());
    assert_eq!(summary.status, Status::Done);
    assert_eq!(written, ["d.txt"]);
    assert_eq!(taken, ["c.txt"], "not this job's copy: a conflict");
    assert_eq!(
        summary.failed,
        [],
        "other.txt's source is gone: it was moved"
    );
    assert_eq!(summary.skipped, 1);
    assert!(!project.join("b.txt").exists(), "its copy is complete");
    assert!(project.join("c.txt").exists(), "not this job's copy");
    assert_eq!(names(&scene.from), ["c.txt", "project"]);
    assert_eq!(
        names(&scene.to),
        ["a.txt", "b.txt", "c.txt", "d.txt", "other.txt", "project"]
    );
}

#[test]
fn a_resumed_move_keeps_a_source_that_changed() {
    let scene = scene();
    let source = scene.from.join("b.txt");
    write(&source, 2);
    let target = scene.to.join("b.txt");
    fs::copy(&source, &target).unwrap();
    write(&source, 3);
    let job = job(&[&source], &scene.to);
    let resume = Resume {
        writing: Some((source.clone(), target.clone())),
    };

    let (summary, _, taken) = resume_it(&job, &resume, &Control::new());
    assert_eq!(
        taken,
        ["b.txt"],
        "the file there isn't a copy of it any more"
    );
    assert_eq!(summary.failed, []);
    assert_eq!(fs::metadata(&source).unwrap().len(), 3, "kept");
    assert_eq!(fs::metadata(&target).unwrap().len(), 2);
}

#[test]
fn a_cancelled_resume_leaves_the_folder_that_was_there() {
    let scene = scene();
    let project = scene.from.join("project");
    fs::create_dir(&project).unwrap();
    write(&project.join("a.txt"), 3_000_000);
    fs::create_dir(scene.to.join("project")).unwrap();
    let job = Job {
        kind: Kind::Copy,
        sources: vec![project],
        destination: scene.to.clone(),
    };
    let control = Control::new();
    let summary = resume_with(
        &job,
        &Resume::default(),
        COPYING,
        &control,
        &keep,
        &mut |event| {
            if let Event::Writing(_) = event {
                control.cancel();
            }
        },
    )
    .unwrap();
    assert_eq!(summary.status, Status::Cancelled);
    assert_eq!(names(&scene.to), ["project"]);
}

/// Runs `job` with `options`, answering every conflict by its name with `answers`, and returns
/// its summary and the names it asked about.
fn answering(
    job: &Job,
    options: Options,
    bin: &dyn Bin,
    answers: &[(&str, Choice, bool)],
) -> (Summary, Vec<String>) {
    let control = Control::new();
    let mut asked = Vec::new();
    let summary = run_with(job, options, &control, bin, &mut |event| {
        if let Event::Conflict(conflict) = event {
            let name = conflict.target.file_name().unwrap().to_string_lossy();
            let &(_, choice, all) = answers
                .iter()
                .find(|(each, ..)| *each == name)
                .unwrap_or_else(|| panic!("no answer for {name}"));
            asked.push(name.into_owned());
            control.answer(Answer { choice, all });
        }
    })
    .unwrap();
    (summary, asked)
}

/// A Recycle Bin that is a folder of the test's.
fn bin_in(folder: &Path) -> impl Fn(&Path) -> Result<(), JerryError> + Send + Sync + use<> {
    fs::create_dir_all(folder).unwrap();
    let folder = folder.to_owned();
    move |path: &Path| {
        fs::rename(path, folder.join(path.file_name().unwrap()))
            .map_err(|error| JerryError::from(error).at(path))
    }
}

#[test]
fn a_move_across_volumes_merges_replaces_and_skips() {
    let scene = scene();
    let project = scene.from.join("project");
    fs::create_dir_all(project.join("sub")).unwrap();
    fs::write(project.join("a.txt"), b"new a").unwrap();
    fs::write(project.join("sub").join("b.txt"), b"b").unwrap();
    let kept = scene.from.join("c.txt");
    fs::write(&kept, b"mine").unwrap();
    let there = scene.to.join("project");
    fs::create_dir_all(there.join("sub")).unwrap();
    fs::write(there.join("a.txt"), b"old a").unwrap();
    fs::write(scene.to.join("c.txt"), b"theirs").unwrap();
    let recycled = scene.from.parent().unwrap().join("bin");

    let (summary, asked) = answering(
        &job(&[&project, &kept], &scene.to),
        COPYING,
        &bin_in(&recycled),
        &[
            ("project", Choice::Merge, false),
            ("a.txt", Choice::Replace, false),
            ("c.txt", Choice::Skip, false),
        ],
    );
    assert_eq!(
        asked,
        ["project", "a.txt", "c.txt"],
        "sub merges on its own"
    );
    assert_eq!(summary.failed, []);
    assert_eq!((summary.files, summary.skipped), (2, 1));
    assert_eq!(fs::read(there.join("a.txt")).unwrap(), b"new a");
    assert_eq!(fs::read(recycled.join("a.txt")).unwrap(), b"old a");
    assert_eq!(fs::read(there.join("sub").join("b.txt")).unwrap(), b"b");
    assert_eq!(fs::read(scene.to.join("c.txt")).unwrap(), b"theirs");
    assert_eq!(names(&scene.from), ["c.txt"], "the merged folder went");
}

#[test]
fn a_move_that_skips_keeps_the_source_folder_with_what_stayed() {
    let scene = scene();
    let project = scene.from.join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("a.txt"), b"mine").unwrap();
    fs::write(project.join("b.txt"), b"b").unwrap();
    fs::create_dir(scene.to.join("project")).unwrap();
    fs::write(scene.to.join("project").join("a.txt"), b"theirs").unwrap();

    let (summary, _) = answering(
        &job(&[&project], &scene.to),
        COPYING,
        &keep,
        &[
            ("project", Choice::Merge, false),
            ("a.txt", Choice::Skip, false),
        ],
    );
    assert_eq!(summary.failed, []);
    assert_eq!(names(&scene.from), ["a.txt", "project"]);
    assert_eq!(names(&scene.to), ["a.txt", "b.txt", "project"]);
    assert_eq!(
        fs::read(scene.to.join("project").join("a.txt")).unwrap(),
        b"theirs"
    );
}

#[test]
fn a_retry_keeps_what_apply_to_all_answered() {
    let scene = scene();
    let (a, b) = (scene.from.join("a.txt"), scene.from.join("b.txt"));
    fs::write(&a, b"new").unwrap();
    fs::write(&b, b"new").unwrap();
    fs::write(scene.to.join("a.txt"), b"old").unwrap();
    let held = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&b)
        .unwrap();
    let (summary, asked) = answering(
        &job(&[&a, &b], &scene.to),
        COPYING,
        &keep,
        &[("a.txt", Choice::Skip, true)],
    );
    assert_eq!(asked, ["a.txt"]);
    assert_eq!(summary.failed.len(), 1, "b.txt is held");
    drop(held);
    fs::write(scene.to.join("b.txt"), b"old").unwrap();

    let control = Control::new();
    let again = retry(&summary, &control, &keep, |event| {
        assert!(!matches!(event, Event::Conflict(_)), "Skip applies to all");
    })
    .unwrap();
    assert_eq!(again.failed, []);
    assert_eq!(again.skipped, 1);
    assert_eq!(fs::read(scene.to.join("b.txt")).unwrap(), b"old");
    assert!(b.exists(), "skipped, so it stays");
}

#[test]
fn a_resumed_copy_counts_its_file_under_a_kept_name_as_done() {
    let scene = scene();
    let source = scene.from.join("a.txt");
    write(&source, 4);
    write(&scene.to.join("a.txt"), 9);
    fs::copy(&source, scene.to.join("a (2).txt")).unwrap();
    let job = Job {
        kind: Kind::Copy,
        sources: vec![source],
        destination: scene.to.clone(),
    };

    let (summary, written, taken) = resume_it(&job, &Resume::default(), &Control::new());
    assert_eq!((written, taken), (vec![], vec![]), "nothing to do");
    assert_eq!(summary.failed, []);
    assert_eq!(names(&scene.to), ["a (2).txt", "a.txt"]);
}
