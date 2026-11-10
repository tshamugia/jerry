//! The transfer engine on real temp folders: Georgian names, paths past 260 characters, locked
//! files, links that must not be followed, pausing and cancelling in the middle of a file. Moves
//! across volumes are tested in the crate, where the renaming can be turned off.

#![allow(
    clippy::expect_used,
    reason = "test helpers: a failed setup fails the test"
)]

use std::fs::{self, File, OpenOptions};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use jerry_core::job::run_job;
use jerry_core::path::{long_path, plain_path};
use jerry_core::space::folder_space;
use jerry_core::transfer::{Control, Event, Job, Kind, Phase, Status, Summary, retry, run};
use jerry_core::{ErrorCode, JerryError};
use tempfile::TempDir;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::FSCTL_SET_SPARSE;

const MB: usize = 1024 * 1024;

/// A source and a destination folder in a temp folder, as long paths.
struct Scene {
    dir: TempDir,
    from: PathBuf,
    to: PathBuf,
}

fn scene() -> Scene {
    let dir = tempfile::Builder::new()
        .prefix("jerry-transfer-")
        .tempdir()
        .expect("temp folder");
    let root = long_path(dir.path()).expect("long path");
    let (from, to) = (root.join("from"), root.join("to"));
    fs::create_dir(&from).expect("mkdir");
    fs::create_dir(&to).expect("mkdir");
    Scene { dir, from, to }
}

fn job(kind: Kind, sources: &[&Path], destination: &Path) -> Job {
    Job {
        kind,
        sources: sources.iter().map(|source| source.to_path_buf()).collect(),
        destination: destination.to_owned(),
    }
}

fn copy(sources: &[&Path], destination: &Path) -> Result<Summary, JerryError> {
    run(
        &job(Kind::Copy, sources, destination),
        &Control::new(),
        |_| {},
    )
}

fn write(path: &Path, bytes: usize) {
    fs::write(path, vec![b'x'; bytes]).expect("write");
}

fn attributes(path: &Path) -> u32 {
    fs::symlink_metadata(path)
        .expect("metadata")
        .file_attributes()
}

fn is_hidden(path: &Path) -> bool {
    attributes(path) & FILE_ATTRIBUTE_HIDDEN.0 != 0
}

fn set_read_only(path: &Path, read_only: bool) {
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_readonly(read_only);
    fs::set_permissions(path, permissions).expect("set read-only");
}

/// Every name in `folder`, recursively, sorted. Links are listed and not entered.
fn names(folder: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut folders = vec![folder.to_owned()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).expect("read_dir") {
            let entry = entry.expect("entry");
            found.push(entry.file_name().to_string_lossy().into_owned());
            if entry.file_type().expect("type").is_dir() {
                folders.push(entry.path());
            }
        }
    }
    found.sort();
    found
}

fn junction(link: &Path, target: &Path) {
    let made = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(plain_path(link))
        .arg(plain_path(target))
        .output()
        .expect("mklink runs");
    assert!(made.status.success(), "mklink /J: {made:?}");
}

/// Holds `path` so that nobody else can open it.
fn lock(path: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path)
        .expect("lock")
}

#[test]
fn copies_georgian_files_and_folders_with_everything_in_them() {
    let scene = scene();
    let project = scene.from.join("პროექტი");
    fs::create_dir_all(project.join("ნახაზები").join("ძველი")).expect("mkdir");
    fs::create_dir(project.join("ცარიელი")).expect("mkdir");
    write(&project.join("ნახაზი 1.dwg"), 1000);
    write(&project.join("ნახაზები").join("გეგმა.pdf"), 20);
    write(&project.join("ნახაზები").join("ძველი").join("ა.txt"), 3);
    let single = scene.from.join("სია.xlsx");
    write(&single, 7);

    let summary = copy(&[&project, &single], &scene.to).expect("starts");
    assert_eq!(summary.status, Status::Done);
    assert_eq!(summary.failed, []);
    assert_eq!(
        (summary.files, summary.folders, summary.bytes),
        (4, 4, 1030)
    );

    let copied = scene.to.join("პროექტი");
    assert_eq!(names(&copied), names(&project));
    assert_eq!(
        fs::metadata(copied.join("ნახაზები").join("გეგმა.pdf"))
            .expect("copied")
            .len(),
        20
    );
    assert!(copied.join("ცარიელი").is_dir(), "empty folders come along");
    assert!(scene.to.join("სია.xlsx").exists());
    assert!(
        project.join("ნახაზი 1.dwg").exists(),
        "a copy keeps the source"
    );
}

#[test]
fn a_copy_keeps_attributes_and_the_modified_date() {
    let scene = scene();
    let source = scene.from.join("secret.txt");
    write(&source, 5);
    set_read_only(&source, true);
    let hide = Command::new("attrib")
        .arg("+h")
        .arg(plain_path(&source))
        .output()
        .expect("attrib runs");
    assert!(hide.status.success());
    std::thread::sleep(Duration::from_millis(20));
    let before = fs::metadata(&source).expect("metadata");

    assert_eq!(copy(&[&source], &scene.to).expect("starts").failed, []);

    let target = scene.to.join("secret.txt");
    assert!(is_hidden(&target));
    assert_ne!(attributes(&target) & FILE_ATTRIBUTE_READONLY.0, 0);
    let after = fs::metadata(&target).expect("metadata");
    assert_eq!(after.last_write_time(), before.last_write_time());
    assert_eq!(names(&scene.to), ["secret.txt"]);
}

#[test]
fn a_copied_folder_keeps_its_modified_date() {
    let scene = scene();
    let folder = scene.from.join("folder");
    fs::create_dir(&folder).expect("mkdir");
    write(&folder.join("a.txt"), 1);
    std::thread::sleep(Duration::from_millis(20));
    let written = fs::metadata(&folder).expect("metadata").last_write_time();
    copy(&[&folder], &scene.to).expect("starts");
    assert_eq!(
        fs::metadata(scene.to.join("folder"))
            .expect("copied")
            .last_write_time(),
        written
    );
}

/// The acceptance item of #48: while a file is being written, it has no final name.
#[test]
fn a_file_has_no_final_name_until_it_is_complete() {
    let scene = scene();
    let source = scene.from.join("ნახაზი.dwg");
    write(&source, 8 * MB);
    let target = scene.to.join("ნახაზი.dwg");
    let part = scene.to.join("ნახაზი.dwg.jerrypart");
    let mut chunks = 0;
    let summary = run(
        &job(Kind::Copy, &[&source], &scene.to),
        &Control::new(),
        |event| {
            if let Event::Progress(progress) = event
                && progress.files_done == 0
            {
                chunks += 1;
                assert!(!target.exists(), "no final name at {progress:?}");
                assert!(is_hidden(&part), "hidden at {progress:?}");
            }
        },
    )
    .expect("starts");
    assert!(chunks > 1, "seen in the middle of the file");
    assert_eq!(summary.failed, []);
    assert_eq!(fs::metadata(&target).expect("copied").len(), 8 * MB as u64);
    assert_eq!(
        fs::read(&target).expect("read"),
        fs::read(&source).expect("read")
    );
    assert!(!is_hidden(&target), "only the temporary file is hidden");
    assert_eq!(names(&scene.to), ["ნახაზი.dwg"]);
}

#[test]
fn reports_phases_in_order_and_reaches_the_total() {
    let scene = scene();
    let folder = scene.from.join("folder");
    fs::create_dir(&folder).expect("mkdir");
    write(&folder.join("a.bin"), 3 * MB);
    write(&folder.join("b.bin"), 10);
    let total = 3 * MB as u64 + 10;
    let mut phases = Vec::new();
    let mut last = None;
    let mut highest = 0;
    run(
        &job(Kind::Copy, &[&folder], &scene.to),
        &Control::new(),
        |event| match event {
            Event::Phase(phase) => phases.push(phase),
            Event::Progress(progress) => {
                assert!(progress.bytes_done >= highest, "never goes back");
                highest = progress.bytes_done;
                assert!(progress.bytes_done <= progress.bytes_total);
                last = Some((
                    progress.files_done,
                    progress.files_total,
                    progress.bytes_done,
                    progress.bytes_total,
                ));
            }
            Event::Counted(_) | Event::Failed(_) | Event::Writing(_) => {}
        },
    )
    .expect("starts");
    assert_eq!(
        phases,
        [Phase::Counting, Phase::CheckingSpace, Phase::Working]
    );
    assert_eq!(last, Some((2, 2, total, total)));
}

#[test]
fn paths_past_260_characters_work() {
    let scene = scene();
    let mut deep = scene.from.join("root");
    for _ in 0..8 {
        deep.push("ძალიან გრძელი საქაღალდის სახელი გზის გასაგრძელებლად");
    }
    fs::create_dir_all(&deep).expect("mkdir");
    let file = deep.join("ნახაზი.dwg");
    write(&file, 2000);
    assert!(file.as_os_str().len() > 260);

    let summary = copy(&[&scene.from.join("root")], &scene.to).expect("starts");
    assert_eq!(summary.failed, []);
    assert_eq!(summary.files, 1);
    let copied = scene
        .to
        .join(file.strip_prefix(&scene.from).expect("inside"));
    assert_eq!(fs::metadata(&copied).expect("copied").len(), 2000);
}

#[test]
fn a_name_with_no_room_for_the_temporary_ending_works() {
    let scene = scene();
    let name = "ა".repeat(250);
    let source = scene.from.join(&name);
    write(&source, 10);
    let summary = copy(&[&source], &scene.to).expect("starts");
    assert_eq!(summary.failed, []);
    assert_eq!(names(&scene.to), [name]);
}

#[test]
fn cancelling_removes_the_partial_file_and_keeps_finished_ones() {
    let scene = scene();
    let (first, second) = (scene.from.join("a.bin"), scene.from.join("b.bin"));
    write(&first, 3 * MB);
    write(&second, 3 * MB);
    let control = Control::new();
    let part = scene.to.join("b.bin.jerrypart");
    let mut cancelled = false;
    let summary = run(
        &job(Kind::Copy, &[&first, &second], &scene.to),
        &control,
        |event| {
            // In the middle of the second file.
            if let Event::Progress(progress) = event
                && progress.files_done == 1
                && progress.bytes_done > 3 * MB as u64
            {
                assert!(part.exists());
                control.cancel();
                cancelled = true;
            }
        },
    )
    .expect("starts");
    assert!(cancelled);
    assert_eq!(summary.status, Status::Cancelled);
    assert_eq!(summary.files, 1);
    assert_eq!(names(&scene.to), ["a.bin"], "the part is gone, a.bin stays");
    assert!(first.exists() && second.exists());
}

#[test]
fn a_read_only_file_that_is_cancelled_leaves_nothing() {
    let scene = scene();
    let source = scene.from.join("ro.bin");
    write(&source, 2 * MB);
    set_read_only(&source, true);
    let control = Control::new();
    let summary = run(&job(Kind::Copy, &[&source], &scene.to), &control, |event| {
        if matches!(event, Event::Progress(_)) {
            control.cancel();
        }
    })
    .expect("starts");
    assert_eq!(summary.status, Status::Cancelled);
    assert_eq!(names(&scene.to), Vec::<String>::new());
}

#[test]
fn a_pause_holds_the_copy_and_resume_finishes_it() {
    let scene = scene();
    let source = scene.from.join("big.bin");
    write(&source, 8 * MB);
    let target = scene.to.join("big.bin");
    let part = scene.to.join("big.bin.jerrypart");

    let control = Arc::new(Control::new());
    let (paused, heard) = mpsc::channel();
    let worker_control = control.clone();
    let worker_job = job(Kind::Copy, &[&source], &scene.to);
    let worker = std::thread::spawn(move || {
        let mut told = false;
        run(&worker_job, &worker_control, |event| {
            if matches!(event, Event::Progress(_)) && !told {
                told = true;
                worker_control.pause();
                paused.send(()).expect("the test waits");
            }
        })
    });
    heard
        .recv_timeout(Duration::from_secs(30))
        .expect("the job started");
    std::thread::sleep(Duration::from_millis(150));
    assert!(!worker.is_finished(), "the job is held");
    assert!(
        part.exists() && !target.exists(),
        "in the middle of the file"
    );

    control.resume();
    let summary = worker.join().expect("no panic").expect("starts");
    assert_eq!(summary.status, Status::Done);
    assert_eq!(fs::metadata(&target).expect("copied").len(), 8 * MB as u64);
    assert_eq!(names(&scene.to), ["big.bin"]);
}

#[test]
fn cancelling_ends_a_paused_job() {
    let scene = scene();
    let source = scene.from.join("big.bin");
    write(&source, 4 * MB);
    let control = Arc::new(Control::new());
    let (paused, heard) = mpsc::channel();
    let worker_control = control.clone();
    let worker_job = job(Kind::Copy, &[&source], &scene.to);
    let worker = std::thread::spawn(move || {
        run(&worker_job, &worker_control, |event| {
            if matches!(event, Event::Progress(_)) {
                worker_control.pause();
                let _ = paused.send(());
            }
        })
    });
    heard
        .recv_timeout(Duration::from_secs(30))
        .expect("the job started");
    control.cancel();
    let summary = worker.join().expect("no panic").expect("starts");
    assert_eq!(summary.status, Status::Cancelled);
    assert_eq!(names(&scene.to), Vec::<String>::new());
}

#[test]
fn cancelling_while_counting_starts_nothing() {
    let scene = scene();
    let folder = scene.from.join("folder");
    fs::create_dir(&folder).expect("mkdir");
    write(&folder.join("a.txt"), 1);
    let control = Control::new();
    control.cancel();
    let error =
        run(&job(Kind::Copy, &[&folder], &scene.to), &control, |_| {}).expect_err("cancelled");
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert_eq!(names(&scene.to), Vec::<String>::new());
}

#[test]
fn a_locked_file_goes_on_the_list_and_a_retry_does_only_that() {
    let scene = scene();
    let folder = scene.from.join("folder");
    fs::create_dir(&folder).expect("mkdir");
    let locked = folder.join("დაკავებული.dwg");
    write(&locked, 40);
    write(&folder.join("free.txt"), 2);
    let held = lock(&locked);

    let summary = copy(&[&folder], &scene.to).expect("starts");
    assert_eq!(summary.status, Status::Done, "the job went on");
    assert_eq!(summary.files, 1);
    assert_eq!(summary.failed.len(), 1);
    let failure = &summary.failed[0];
    assert_eq!(failure.error.code, ErrorCode::InUse { app: None });
    assert_eq!(failure.source, locked);
    assert_eq!(
        failure.target,
        scene.to.join("folder").join("დაკავებული.dwg")
    );
    assert!(!failure.target.exists());
    assert_eq!(names(&scene.to), ["folder", "free.txt"]);

    drop(held);
    let free_written = fs::metadata(scene.to.join("folder").join("free.txt"))
        .expect("copied")
        .creation_time();
    let again = retry(&summary, &Control::new(), |_| {}).expect("starts");
    assert_eq!(again.failed, []);
    assert_eq!((again.files, again.bytes), (1, 40));
    assert_eq!(names(&scene.to), ["folder", "free.txt", "დაკავებული.dwg"]);
    assert_eq!(
        fs::metadata(scene.to.join("folder").join("free.txt"))
            .expect("copied")
            .creation_time(),
        free_written,
        "what worked isn't done again"
    );
}

#[test]
fn a_retry_that_fails_again_keeps_the_failure() {
    let scene = scene();
    let locked = scene.from.join("a.txt");
    write(&locked, 1);
    let held = lock(&locked);
    let summary = copy(&[&locked], &scene.to).expect("starts");
    let again = retry(&summary, &Control::new(), |_| {}).expect("starts");
    assert_eq!(again.failed.len(), 1);
    assert_eq!(again.failed[0].error.code, ErrorCode::InUse { app: None });
    drop(held);
    let third = retry(&again, &Control::new(), |_| {}).expect("starts");
    assert_eq!(third.failed, []);
    assert_eq!(names(&scene.to), ["a.txt"]);
}

/// Denies everyone listing `dir` until dropped (`icacls`, so no Security API in the tests).
struct Denied<'a>(&'a Path);

impl<'a> Denied<'a> {
    fn new(dir: &'a Path) -> Self {
        let status = Command::new("icacls")
            .arg(plain_path(dir))
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
            .arg(plain_path(self.0))
            .args(["/remove:d", "*S-1-1-0"])
            .output();
    }
}

#[test]
fn a_folder_that_can_not_be_read_goes_on_the_list() {
    let scene = scene();
    let folder = scene.from.join("folder");
    let closed = folder.join("დახურული");
    fs::create_dir_all(&closed).expect("mkdir");
    write(&closed.join("secret.txt"), 1);
    write(&folder.join("open.txt"), 2);
    let denied = Denied::new(&closed);
    if fs::read_dir(&closed).is_ok() {
        // An account that can read past the denial (an elevated CI runner).
        return;
    }

    let summary = copy(&[&folder], &scene.to).expect("starts");
    assert_eq!(summary.status, Status::Done);
    assert_eq!(summary.failed.len(), 1);
    assert_eq!(summary.failed[0].error.code, ErrorCode::AccessDenied);
    assert_eq!(summary.failed[0].source, closed);
    assert_eq!(names(&scene.to), ["folder", "open.txt"]);

    drop(denied);
    let again = retry(&summary, &Control::new(), |_| {}).expect("starts");
    assert_eq!(again.failed, []);
    assert_eq!(
        names(&scene.to),
        ["folder", "open.txt", "secret.txt", "დახურული"]
    );
}

#[test]
fn an_existing_file_is_never_replaced() {
    let scene = scene();
    let source = scene.from.join("a.txt");
    write(&source, 5);
    let other = scene.from.join("b.txt");
    write(&other, 6);
    let existing = scene.to.join("a.txt");
    fs::write(&existing, b"keep me").expect("write");

    let summary = copy(&[&source, &other], &scene.to).expect("starts");
    assert_eq!(summary.files, 1);
    assert_eq!(summary.failed.len(), 1);
    assert_eq!(summary.failed[0].error.code, ErrorCode::InvalidName);
    assert_eq!(
        summary.failed[0].error.path.as_deref(),
        Some(existing.as_path())
    );
    assert_eq!(fs::read(&existing).expect("read"), b"keep me");
    assert_eq!(names(&scene.to), ["a.txt", "b.txt"]);
}

#[test]
fn a_name_taken_during_the_copy_is_not_replaced() {
    let scene = scene();
    let source = scene.from.join("a.bin");
    write(&source, 3 * MB);
    let target = scene.to.join("a.bin");
    let mut made = false;
    let summary = run(
        &job(Kind::Copy, &[&source], &scene.to),
        &Control::new(),
        |event| {
            if matches!(event, Event::Progress(_)) && !made {
                made = true;
                fs::write(&target, b"arrived first").expect("write");
            }
        },
    )
    .expect("starts");
    assert_eq!(summary.failed.len(), 1);
    assert_eq!(summary.failed[0].error.code, ErrorCode::InvalidName);
    assert_eq!(fs::read(&target).expect("read"), b"arrived first");
    assert_eq!(names(&scene.to), ["a.bin"], "and no temporary file stays");
}

#[test]
fn an_existing_folder_fails_once_for_everything_in_it() {
    let scene = scene();
    let folder = scene.from.join("folder");
    fs::create_dir_all(folder.join("sub")).expect("mkdir");
    write(&folder.join("a.txt"), 1);
    write(&folder.join("sub").join("b.txt"), 1);
    fs::create_dir(scene.to.join("folder")).expect("mkdir");

    let summary = copy(&[&folder], &scene.to).expect("starts");
    assert_eq!(summary.status, Status::Done);
    assert_eq!((summary.files, summary.folders), (0, 0));
    assert_eq!(summary.failed.len(), 1);
    assert_eq!(summary.failed[0].error.code, ErrorCode::InvalidName);
    assert_eq!(names(&scene.to), ["folder"], "nothing was merged");
}

#[test]
fn a_temporary_file_this_job_did_not_make_is_left_alone() {
    let scene = scene();
    let source = scene.from.join("a.txt");
    write(&source, 5);
    let stale = scene.to.join("a.txt.jerrypart");
    fs::write(&stale, b"someone else's").expect("write");
    set_read_only(&stale, true);

    let summary = copy(&[&source], &scene.to).expect("starts");
    assert_eq!(summary.failed, []);
    assert_eq!(fs::read(&stale).expect("read"), b"someone else's");
    assert_eq!(
        fs::metadata(scene.to.join("a.txt")).expect("copied").len(),
        5
    );
    assert_eq!(names(&scene.to), ["a.txt", "a.txt.jerrypart"]);
}

#[test]
fn junctions_are_copied_as_junctions_and_never_entered() {
    let scene = scene();
    let elsewhere = scene.dir.path().join("elsewhere");
    fs::create_dir(&elsewhere).expect("mkdir");
    write(&elsewhere.join("outside.bin"), 5000);
    let folder = scene.from.join("folder");
    fs::create_dir(&folder).expect("mkdir");
    write(&folder.join("own.txt"), 1);
    junction(&folder.join("link"), &elsewhere);

    let summary = copy(&[&folder], &scene.to).expect("starts");
    assert_eq!(summary.failed, []);
    assert_eq!(
        (summary.files, summary.bytes),
        (2, 1),
        "the outside isn't counted"
    );

    let link = scene.to.join("folder").join("link");
    assert!(
        fs::symlink_metadata(&link)
            .expect("copied")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::canonicalize(&link).expect("leads somewhere"),
        fs::canonicalize(&elsewhere).expect("there")
    );
    assert_eq!(names(&scene.to), ["folder", "link", "own.txt"]);
}

#[test]
fn a_junction_loop_does_not_catch_the_job() {
    let scene = scene();
    let folder = scene.from.join("folder");
    fs::create_dir(&folder).expect("mkdir");
    write(&folder.join("a.txt"), 1);
    junction(&folder.join("back"), &folder);

    let summary = copy(&[&folder], &scene.to).expect("starts");
    assert_eq!(summary.status, Status::Done);
    assert_eq!(summary.failed, []);
    assert_eq!(summary.files, 2);
    assert_eq!(names(&scene.to), ["a.txt", "back", "folder"]);
}

/// Symbolic links need Developer Mode or elevation, which CI has and many machines don't.
#[test]
fn symbolic_links_are_copied_as_links() {
    let scene = scene();
    let real_dir = scene.from.join("real");
    fs::create_dir(&real_dir).expect("mkdir");
    let real_file = scene.from.join("real.txt");
    write(&real_file, 3);
    let folder = scene.from.join("folder");
    fs::create_dir(&folder).expect("mkdir");
    let (dir_link, file_link) = (folder.join("to-dir"), folder.join("to-file"));
    if std::os::windows::fs::symlink_dir(&real_dir, &dir_link).is_err() {
        return;
    }
    std::os::windows::fs::symlink_file(&real_file, &file_link).expect("symlink");

    let summary = copy(&[&folder], &scene.to).expect("starts");
    assert_eq!(summary.failed, []);
    for name in ["to-dir", "to-file"] {
        let copied = scene.to.join("folder").join(name);
        assert!(
            fs::symlink_metadata(&copied)
                .expect("copied")
                .file_type()
                .is_symlink()
        );
    }
    assert!(scene.to.join("folder").join("to-dir").is_dir());
}

#[test]
fn a_folder_can_not_go_into_itself_or_inside_itself() {
    let scene = scene();
    let folder = scene.from.join("ფოლდერი");
    let inner = folder.join("შიდა");
    fs::create_dir_all(&inner).expect("mkdir");
    write(&inner.join("a.txt"), 1);

    for kind in [Kind::Copy, Kind::Move] {
        for destination in [&folder, &inner] {
            let error = run(&job(kind, &[&folder], destination), &Control::new(), |_| {})
                .expect_err("blocked");
            assert_eq!(
                error.code,
                ErrorCode::IntoItself,
                "{kind:?} {destination:?}"
            );
            assert_eq!(error.path.as_deref(), Some(folder.as_path()));
        }
    }
    assert_eq!(names(&folder), ["a.txt", "შიდა"], "nothing was touched");
}

#[test]
fn a_junction_does_not_hide_a_folder_going_into_itself() {
    let scene = scene();
    let folder = scene.from.join("folder");
    let inner = folder.join("inner");
    fs::create_dir_all(&inner).expect("mkdir");
    let door = scene.to.join("door");
    junction(&door, &inner);

    let error = copy(&[&folder], &door).expect_err("blocked");
    assert_eq!(error.code, ErrorCode::IntoItself);
}

#[test]
fn one_blocked_source_stops_the_job_before_anything_happens() {
    let scene = scene();
    let free = scene.from.join("free.txt");
    write(&free, 1);
    let folder = scene.from.join("folder");
    fs::create_dir(&folder).expect("mkdir");
    let error = copy(&[&free, &folder], &folder).expect_err("blocked");
    assert_eq!(error.code, ErrorCode::IntoItself);
    assert_eq!(names(&folder), Vec::<String>::new());
}

#[test]
fn a_name_that_only_looks_like_the_parent_is_not_inside_it() {
    let scene = scene();
    let folder = scene.from.join("folder");
    fs::create_dir(&folder).expect("mkdir");
    let sibling = scene.from.join("folder two");
    fs::create_dir(&sibling).expect("mkdir");
    assert_eq!(copy(&[&folder], &sibling).expect("starts").failed, []);
    assert!(sibling.join("folder").is_dir());
}

/// A file as big as `size` that takes up no room.
fn sparse(path: &Path, size: u64) {
    let file = File::create(path).expect("create");
    let mut returned = 0_u32;
    // SAFETY: the handle belongs to `file`, which outlives the call; this control code takes no
    // buffers, and `returned` outlives the call. No OVERLAPPED: `file` is synchronous.
    unsafe {
        DeviceIoControl(
            HANDLE(file.as_raw_handle()),
            FSCTL_SET_SPARSE,
            None,
            0,
            None,
            0,
            Some(&raw mut returned),
            None,
        )
    }
    .expect("sparse");
    file.set_len(size).expect("set_len");
}

#[test]
fn a_job_that_does_not_fit_does_not_start() {
    let scene = scene();
    let free = folder_space(&scene.to).expect("the volume answers").free as u64;
    let huge = scene.from.join("huge.bin");
    let size = free + 1024 * 1024 * 1024;
    sparse(&huge, size);
    let small = scene.from.join("small.txt");
    write(&small, 1);

    let mut working = false;
    let error = run(
        &job(Kind::Copy, &[&small, &huge], &scene.to),
        &Control::new(),
        |event| working |= matches!(event, Event::Phase(Phase::Working)),
    )
    .expect_err("doesn't fit");
    let ErrorCode::NoSpace { needed, available } = error.code else {
        panic!("{error:?}");
    };
    assert_eq!(needed, Some(size + 1));
    assert!(available.expect("known") < size);
    assert!(!working, "the job never started");
    assert_eq!(names(&scene.to), Vec::<String>::new());
}

#[test]
fn files_from_256_mb_on_are_copied_without_the_cache() {
    let scene = scene();
    let source = scene.from.join("big.bin");
    File::create(&source)
        .expect("create")
        .set_len(256 * MB as u64)
        .expect("set_len");
    let summary = copy(&[&source], &scene.to).expect("starts");
    assert_eq!(summary.failed, []);
    assert_eq!(
        fs::metadata(scene.to.join("big.bin"))
            .expect("copied")
            .len(),
        256 * MB as u64
    );
}

#[test]
fn moving_within_a_volume_renames_at_once() {
    let scene = scene();
    let folder = scene.from.join("პროექტი");
    fs::create_dir_all(folder.join("sub")).expect("mkdir");
    write(&folder.join("sub").join("a.dwg"), 1000);
    let file = scene.from.join("b.txt");
    write(&file, 5);
    std::thread::sleep(Duration::from_millis(20));
    let created = fs::metadata(&file).expect("metadata").creation_time();

    let mut chunks = 0;
    let summary = run(
        &job(Kind::Move, &[&folder, &file], &scene.to),
        &Control::new(),
        |event| {
            if let Event::Progress(progress) = event {
                chunks += 1;
                assert_eq!(progress.bytes_total, 0, "nothing is counted or copied");
            }
        },
    )
    .expect("starts");
    assert_eq!(summary.status, Status::Done);
    assert_eq!(summary.failed, []);
    assert_eq!((summary.files, summary.folders, summary.bytes), (1, 1, 0));
    assert_eq!(chunks, 2, "one step per item");
    assert_eq!(names(&scene.from), Vec::<String>::new());
    assert_eq!(names(&scene.to), ["a.dwg", "b.txt", "sub", "პროექტი"]);
    assert_eq!(
        fs::metadata(scene.to.join("b.txt"))
            .expect("moved")
            .creation_time(),
        created,
        "the same file, not a copy"
    );
}

#[test]
fn moving_onto_an_existing_name_keeps_both() {
    let scene = scene();
    let source = scene.from.join("a.txt");
    fs::write(&source, b"mine").expect("write");
    let existing = scene.to.join("a.txt");
    fs::write(&existing, b"theirs").expect("write");

    let summary = run(
        &job(Kind::Move, &[&source], &scene.to),
        &Control::new(),
        |_| {},
    )
    .expect("starts");
    assert_eq!(summary.failed.len(), 1);
    assert_eq!(summary.failed[0].error.code, ErrorCode::InvalidName);
    assert_eq!(fs::read(&source).expect("read"), b"mine");
    assert_eq!(fs::read(&existing).expect("read"), b"theirs");
}

#[test]
fn moving_into_the_folder_it_is_in_changes_nothing() {
    let scene = scene();
    let source = scene.from.join("a.txt");
    write(&source, 3);
    let summary = run(
        &job(Kind::Move, &[&source], &scene.from),
        &Control::new(),
        |_| {},
    )
    .expect("starts");
    assert_eq!(summary.failed, []);
    assert_eq!(fs::metadata(&source).expect("there").len(), 3);
}

#[test]
fn a_locked_folder_move_fails_whole_and_a_retry_moves_it() {
    let scene = scene();
    let folder = scene.from.join("folder");
    fs::create_dir(&folder).expect("mkdir");
    let inside = folder.join("a.txt");
    write(&inside, 1);
    let held = lock(&inside);

    let summary = run(
        &job(Kind::Move, &[&folder], &scene.to),
        &Control::new(),
        |_| {},
    )
    .expect("starts");
    assert_eq!(summary.failed.len(), 1);
    assert!(inside.exists());
    assert_eq!(names(&scene.to), Vec::<String>::new());

    drop(held);
    let again = retry(&summary, &Control::new(), |_| {}).expect("starts");
    assert_eq!(again.failed, []);
    assert_eq!(again.folders, 1);
    assert_eq!(names(&scene.to), ["a.txt", "folder"]);
}

#[test]
fn a_missing_source_goes_on_the_list_and_the_rest_is_copied() {
    let scene = scene();
    let gone = scene.from.join("gone.txt");
    let here = scene.from.join("here.txt");
    write(&here, 1);
    let mut heard = Vec::new();
    let summary = run(
        &job(Kind::Copy, &[&gone, &here], &scene.to),
        &Control::new(),
        |event| {
            if let Event::Failed(failure) = event {
                heard.push(failure.source.clone());
            }
        },
    )
    .expect("starts");
    assert_eq!(heard, std::slice::from_ref(&gone));
    assert_eq!(summary.failed.len(), 1);
    assert_eq!(summary.failed[0].error.code, ErrorCode::NotFound);
    assert_eq!(summary.files, 1);
    assert_eq!(names(&scene.to), ["here.txt"]);
}

#[test]
fn a_destination_that_is_no_folder_stops_the_job() {
    let scene = scene();
    let source = scene.from.join("a.txt");
    write(&source, 1);
    let error = copy(&[&source], &scene.to.join("gone")).expect_err("no folder");
    assert_eq!(error.code, ErrorCode::NotFound);
    let file = scene.to.join("file.txt");
    write(&file, 1);
    let error = copy(&[&source], &file).expect_err("no folder");
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn a_panic_during_a_copy_fails_only_its_job() {
    let scene = scene();
    let source = scene.from.join("a.bin");
    write(&source, 3 * MB);
    let copy_job = job(Kind::Copy, &[&source], &scene.to);
    let error = run_job("transfer test", || {
        run(&copy_job, &Control::new(), |event| {
            assert!(!matches!(event, Event::Progress(_)), "deliberate panic");
        })
    })
    .expect_err("the job failed");
    assert_eq!(error.code, ErrorCode::Internal);
    assert_eq!(names(&scene.to), Vec::<String>::new(), "and left nothing");
    assert!(source.exists());
}

#[test]
fn an_empty_file_is_copied() {
    let scene = scene();
    let source = scene.from.join("ცარიელი.txt");
    write(&source, 0);
    let summary = copy(&[&source], &scene.to).expect("starts");
    assert_eq!(summary.failed, []);
    assert_eq!(summary.files, 1);
    let target = scene.to.join("ცარიელი.txt");
    assert_eq!(fs::metadata(&target).expect("copied").len(), 0);
    assert!(!is_hidden(&target));
    assert_eq!(names(&scene.to), ["ცარიელი.txt"]);
}

#[test]
fn alternate_data_streams_come_along_and_progress_stays_within_the_total() {
    let scene = scene();
    let source = scene.from.join("a.bin");
    write(&source, 2 * MB);
    let stream = |path: &Path| {
        let mut name = path.as_os_str().to_owned();
        name.push(":Zone.Identifier");
        PathBuf::from(name)
    };
    fs::write(stream(&source), b"[ZoneTransfer]").expect("write stream");
    let mut highest = 0;
    let summary = run(
        &job(Kind::Copy, &[&source], &scene.to),
        &Control::new(),
        |event| {
            if let Event::Progress(progress) = event {
                assert!(progress.bytes_done <= progress.bytes_total, "{progress:?}");
                highest = progress.bytes_done;
            }
        },
    )
    .expect("starts");
    assert_eq!(summary.failed, []);
    assert_eq!(highest, 2 * MB as u64);
    assert_eq!(
        fs::read(stream(&scene.to.join("a.bin"))).expect("the stream came along"),
        b"[ZoneTransfer]"
    );
}
