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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use jerry_core::job::run_job;
use jerry_core::path::{long_path, plain_path};
use jerry_core::space::folder_space;
use jerry_core::transfer::conflict::{
    FILE_CHOICES, FOLDER_CHOICES, NO_BIN_CHOICES, ONTO_FILE_CHOICES, ONTO_FOLDER_CHOICES,
};
use jerry_core::transfer::{
    Answer, Bin, Choice, Control, Event, Job, Kind, Phase, Status, Summary, retry, run,
};
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
        &keep,
        |event| assert!(!matches!(event, Event::Conflict(_)), "no name is taken"),
    )
}

/// No Recycle Bin: what Replace would replace stays, and the user is asked again.
fn keep(path: &Path) -> Result<(), JerryError> {
    Err(JerryError::new(ErrorCode::NoRecycleBin).at(path))
}

/// A Recycle Bin that is a folder of the test's, so that what goes there can be looked at.
fn bin_in(folder: &Path) -> impl Fn(&Path) -> Result<(), JerryError> + Send + Sync + use<> {
    fs::create_dir_all(folder).expect("mkdir");
    let folder = folder.to_owned();
    move |path: &Path| {
        let name = path.file_name().expect("a name");
        fs::rename(path, folder.join(name)).map_err(|error| JerryError::from(error).at(path))
    }
}

/// A conflict a job asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Asked {
    /// The name that is taken.
    name: String,
    choices: Vec<Choice>,
    no_recycle_bin: bool,
}

fn asked(name: &str, choices: &[Choice]) -> Asked {
    Asked {
        name: name.to_owned(),
        choices: choices.to_vec(),
        no_recycle_bin: false,
    }
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned()
}

/// Runs `job`, answering its conflicts in order with `answers` (choice and "Apply to all"), and
/// returns its summary and what it asked.
fn answering(job: &Job, bin: &dyn Bin, answers: &[(Choice, bool)]) -> (Summary, Vec<Asked>) {
    let control = Control::new();
    let mut questions = Vec::new();
    let mut answers = answers.iter();
    let summary = run(job, &control, bin, |event| {
        if let Event::Conflict(conflict) = event {
            questions.push(Asked {
                name: name_of(conflict.target),
                choices: conflict.choices.to_vec(),
                no_recycle_bin: conflict.no_recycle_bin,
            });
            let &(choice, all) = answers.next().expect("an answer for every question");
            control.answer(Answer { choice, all });
        }
    })
    .expect("starts");
    assert_eq!(answers.next(), None, "every answer was asked for");
    (summary, questions)
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
        &keep,
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
        &keep,
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
            Event::Counted(_) | Event::Failed(_) | Event::Writing(_) | Event::Conflict(_) => {}
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
        &keep,
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
    let summary = run(
        &job(Kind::Copy, &[&source], &scene.to),
        &control,
        &keep,
        |event| {
            if matches!(event, Event::Progress(_)) {
                control.cancel();
            }
        },
    )
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
        run(&worker_job, &worker_control, &keep, |event| {
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
        run(&worker_job, &worker_control, &keep, |event| {
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
    let error = run(
        &job(Kind::Copy, &[&folder], &scene.to),
        &control,
        &keep,
        |_| {},
    )
    .expect_err("cancelled");
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
    let again = retry(&summary, &Control::new(), &keep, |_| {}).expect("starts");
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
    let again = retry(&summary, &Control::new(), &keep, |_| {}).expect("starts");
    assert_eq!(again.failed.len(), 1);
    assert_eq!(again.failed[0].error.code, ErrorCode::InUse { app: None });
    drop(held);
    let third = retry(&again, &Control::new(), &keep, |_| {}).expect("starts");
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
    let again = retry(&summary, &Control::new(), &keep, |_| {}).expect("starts");
    assert_eq!(again.failed, []);
    assert_eq!(
        names(&scene.to),
        ["folder", "open.txt", "secret.txt", "დახურული"]
    );
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
        &keep,
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
            let error = run(
                &job(kind, &[&folder], destination),
                &Control::new(),
                &keep,
                |_| {},
            )
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
        &keep,
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
        &keep,
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
fn moving_into_the_folder_it_is_in_changes_nothing() {
    let scene = scene();
    let source = scene.from.join("a.txt");
    write(&source, 3);
    let summary = run(
        &job(Kind::Move, &[&source], &scene.from),
        &Control::new(),
        &keep,
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
        &keep,
        |_| {},
    )
    .expect("starts");
    assert_eq!(summary.failed.len(), 1);
    assert!(inside.exists());
    assert_eq!(names(&scene.to), Vec::<String>::new());

    drop(held);
    let again = retry(&summary, &Control::new(), &keep, |_| {}).expect("starts");
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
        &keep,
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
        run(&copy_job, &Control::new(), &keep, |event| {
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
        &keep,
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

/// Sets the modified date of the file at `path` to `days` days ago.
fn age(path: &Path, days: u64) {
    let when = std::time::SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
    File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(when))
        .expect("set the date");
}

#[test]
fn replace_sends_the_file_there_to_the_recycle_bin_once_the_copy_is_complete() {
    let scene = scene();
    let source = scene.from.join("ნახაზი.dwg");
    write(&source, 3 * MB);
    let target = scene.to.join("ნახაზი.dwg");
    fs::write(&target, b"old drawing").expect("write");
    let part = scene.to.join("ნახაზი.dwg.jerrypart");
    let recycled = long_path(scene.dir.path()).expect("long path").join("bin");
    let into_bin = bin_in(&recycled);
    let checked = Arc::new(AtomicBool::new(false));
    let bin = {
        let checked = checked.clone();
        move |path: &Path| {
            let written = fs::metadata(&part)
                .expect("the copy is written first")
                .len();
            assert_eq!(written, (3 * MB) as u64, "and complete");
            checked.store(true, Ordering::SeqCst);
            into_bin(path)
        }
    };

    let (summary, questions) = answering(
        &job(Kind::Copy, &[&source], &scene.to),
        &bin,
        &[(Choice::Replace, false)],
    );
    assert_eq!(questions, [asked("ნახაზი.dwg", FILE_CHOICES)]);
    assert!(checked.load(Ordering::SeqCst));
    assert_eq!(summary.failed, []);
    assert_eq!((summary.files, summary.skipped), (1, 0));
    assert_eq!(fs::metadata(&target).expect("there").len(), (3 * MB) as u64);
    assert_eq!(
        fs::read(recycled.join("ნახაზი.dwg")).expect("in the bin"),
        b"old drawing"
    );
    assert_eq!(names(&scene.to), ["ნახაზი.dwg"], "no temporary file stays");
}

#[test]
fn skip_leaves_what_is_there_and_keep_both_takes_a_free_name() {
    let scene = scene();
    let (a, b) = (scene.from.join("a.txt"), scene.from.join("b.txt"));
    fs::write(&a, b"new a").expect("write");
    fs::write(&b, b"new b").expect("write");
    fs::write(scene.to.join("a.txt"), b"old a").expect("write");
    fs::write(scene.to.join("b.txt"), b"old b").expect("write");
    fs::write(scene.to.join("b (2).txt"), b"older b").expect("write");

    let (summary, questions) = answering(
        &job(Kind::Copy, &[&a, &b], &scene.to),
        &keep,
        &[(Choice::Skip, false), (Choice::KeepBoth, false)],
    );
    assert_eq!(
        questions,
        [asked("a.txt", FILE_CHOICES), asked("b.txt", FILE_CHOICES)]
    );
    assert_eq!((summary.files, summary.skipped), (1, 1));
    assert_eq!(fs::read(scene.to.join("a.txt")).expect("read"), b"old a");
    assert_eq!(fs::read(scene.to.join("b.txt")).expect("read"), b"old b");
    assert_eq!(
        fs::read(scene.to.join("b (3).txt")).expect("read"),
        b"new b"
    );
    assert_eq!(
        names(&scene.to),
        ["a.txt", "b (2).txt", "b (3).txt", "b.txt"]
    );
}

#[test]
fn replace_if_newer_replaces_only_what_is_older() {
    let scene = scene();
    let (fresh, stale) = (scene.from.join("fresh.txt"), scene.from.join("stale.txt"));
    fs::write(&fresh, b"fresh").expect("write");
    fs::write(&stale, b"stale").expect("write");
    fs::write(scene.to.join("fresh.txt"), b"was there").expect("write");
    fs::write(scene.to.join("stale.txt"), b"was there").expect("write");
    age(&scene.to.join("fresh.txt"), 2);
    age(&stale, 2);
    let recycled = long_path(scene.dir.path()).expect("long path").join("bin");

    let (summary, questions) = answering(
        &job(Kind::Copy, &[&fresh, &stale], &scene.to),
        &bin_in(&recycled),
        &[(Choice::ReplaceIfNewer, true)],
    );
    assert_eq!(questions.len(), 1, "Apply to all: {questions:?}");
    assert_eq!((summary.files, summary.skipped), (1, 1));
    assert_eq!(
        fs::read(scene.to.join("fresh.txt")).expect("read"),
        b"fresh"
    );
    assert_eq!(
        fs::read(scene.to.join("stale.txt")).expect("read"),
        b"was there"
    );
    assert_eq!(names(&recycled), ["fresh.txt"]);
}

#[test]
fn apply_to_all_answers_files_and_folders_apart() {
    let scene = scene();
    let mut sources = Vec::new();
    for name in ["1.txt", "2.txt", "3.txt"] {
        let file = scene.from.join(name);
        fs::write(&file, b"new").expect("write");
        fs::write(scene.to.join(name), b"old").expect("write");
        sources.push(file);
    }
    for name in ["A", "B"] {
        let folder = scene.from.join(name);
        fs::create_dir(&folder).expect("mkdir");
        fs::write(folder.join("in.txt"), b"in").expect("write");
        fs::create_dir(scene.to.join(name)).expect("mkdir");
        sources.push(folder);
    }
    let sources: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();

    let (summary, questions) = answering(
        &job(Kind::Copy, &sources, &scene.to),
        &keep,
        &[(Choice::KeepBoth, true), (Choice::Merge, true)],
    );
    assert_eq!(
        questions,
        [asked("1.txt", FILE_CHOICES), asked("A", FOLDER_CHOICES)]
    );
    assert_eq!(summary.failed, []);
    assert_eq!((summary.files, summary.folders), (5, 0));
    assert_eq!(
        names(&scene.to),
        [
            "1 (2).txt",
            "1.txt",
            "2 (2).txt",
            "2.txt",
            "3 (2).txt",
            "3.txt",
            "A",
            "B",
            "in.txt",
            "in.txt"
        ]
    );
}

#[test]
fn merging_fills_the_folders_there_at_every_depth() {
    let scene = scene();
    let project = scene.from.join("პროექტი");
    fs::create_dir_all(project.join("sub").join("deep")).expect("mkdir");
    fs::create_dir_all(project.join("new")).expect("mkdir");
    fs::write(project.join("a.txt"), b"new a").expect("write");
    fs::write(project.join("sub").join("b.txt"), b"b").expect("write");
    fs::write(project.join("sub").join("deep").join("c.txt"), b"new c").expect("write");
    fs::write(project.join("new").join("d.txt"), b"d").expect("write");
    let there = scene.to.join("პროექტი");
    fs::create_dir_all(there.join("sub").join("deep")).expect("mkdir");
    fs::write(there.join("a.txt"), b"old a").expect("write");
    fs::write(there.join("x.txt"), b"x").expect("write");
    fs::write(there.join("sub").join("deep").join("c.txt"), b"old c").expect("write");
    let recycled = long_path(scene.dir.path()).expect("long path").join("bin");

    let (summary, questions) = answering(
        &job(Kind::Copy, &[&project], &scene.to),
        &bin_in(&recycled),
        &[
            (Choice::Merge, false),
            (Choice::Skip, false),
            (Choice::Replace, false),
        ],
    );
    assert_eq!(
        questions,
        [
            asked("პროექტი", FOLDER_CHOICES),
            asked("a.txt", FILE_CHOICES),
            asked("c.txt", FILE_CHOICES)
        ],
        "the folders inside merge without asking"
    );
    assert_eq!(summary.failed, []);
    assert_eq!(
        (summary.files, summary.folders, summary.skipped),
        (3, 1, 1),
        "b, c and d; only the folder called new is made"
    );
    assert_eq!(fs::read(there.join("a.txt")).expect("read"), b"old a");
    assert_eq!(fs::read(there.join("x.txt")).expect("read"), b"x");
    assert_eq!(
        fs::read(there.join("sub").join("deep").join("c.txt")).expect("read"),
        b"new c"
    );
    assert_eq!(
        fs::read(recycled.join("c.txt")).expect("in the bin"),
        b"old c"
    );
    assert_eq!(
        names(&scene.to),
        [
            "a.txt",
            "b.txt",
            "c.txt",
            "d.txt",
            "deep",
            "new",
            "sub",
            "x.txt",
            "პროექტი"
        ]
    );
}

#[test]
fn keep_both_for_a_folder_puts_all_of_it_under_the_new_name() {
    let scene = scene();
    let project = scene.from.join("project");
    fs::create_dir_all(project.join("sub")).expect("mkdir");
    fs::write(project.join("a.txt"), b"a").expect("write");
    fs::write(project.join("sub").join("b.txt"), b"b").expect("write");
    fs::create_dir(scene.to.join("project")).expect("mkdir");
    fs::write(scene.to.join("project").join("other.txt"), b"o").expect("write");

    let (summary, questions) = answering(
        &job(Kind::Copy, &[&project], &scene.to),
        &keep,
        &[(Choice::KeepBoth, false)],
    );
    assert_eq!(questions, [asked("project", FOLDER_CHOICES)]);
    assert_eq!(summary.failed, []);
    assert_eq!((summary.files, summary.folders), (2, 2));
    let kept = scene.to.join("project (2)");
    assert_eq!(
        fs::read(kept.join("sub").join("b.txt")).expect("read"),
        b"b"
    );
    assert_eq!(names(&scene.to.join("project")), ["other.txt"]);
    assert_eq!(names(&kept), ["a.txt", "b.txt", "sub"]);
}

#[test]
fn a_file_onto_a_folder_and_a_folder_onto_a_file_keep_both() {
    let scene = scene();
    let file = scene.from.join("x");
    fs::write(&file, b"file").expect("write");
    let folder = scene.from.join("y");
    fs::create_dir(&folder).expect("mkdir");
    fs::write(folder.join("f.txt"), b"f").expect("write");
    fs::create_dir(scene.to.join("x")).expect("mkdir");
    fs::write(scene.to.join("y"), b"a file").expect("write");

    let (summary, questions) = answering(
        &job(Kind::Copy, &[&file, &folder], &scene.to),
        &keep,
        &[(Choice::KeepBoth, false), (Choice::KeepBoth, false)],
    );
    assert_eq!(
        questions,
        [
            asked("x", ONTO_FOLDER_CHOICES),
            asked("y", ONTO_FILE_CHOICES)
        ]
    );
    assert_eq!(summary.failed, []);
    assert_eq!(fs::read(scene.to.join("x (2)")).expect("read"), b"file");
    assert_eq!(
        fs::read(scene.to.join("y (2)").join("f.txt")).expect("read"),
        b"f"
    );
}

#[test]
fn copying_into_its_own_folder_keeps_both_and_never_replaces_itself() {
    let scene = scene();
    let file = scene.from.join("ნახაზი.dwg");
    fs::write(&file, b"drawing").expect("write");
    let folder = scene.from.join("project");
    fs::create_dir(&folder).expect("mkdir");
    fs::write(folder.join("a.txt"), b"a").expect("write");

    let (summary, questions) = answering(
        &job(Kind::Copy, &[&file, &folder], &scene.from),
        &keep,
        &[(Choice::KeepBoth, false), (Choice::KeepBoth, false)],
    );
    assert_eq!(
        questions,
        [
            asked("ნახაზი.dwg", ONTO_FOLDER_CHOICES),
            asked("project", ONTO_FILE_CHOICES)
        ]
    );
    assert_eq!(summary.failed, []);
    assert_eq!(
        names(&scene.from),
        [
            "a.txt",
            "a.txt",
            "project",
            "project (2)",
            "ნახაზი (2).dwg",
            "ნახაზი.dwg"
        ]
    );
}

#[test]
fn without_a_recycle_bin_replace_asks_again_and_then_deletes_for_good() {
    let scene = scene();
    let mut sources = Vec::new();
    for name in ["a.txt", "b.txt", "c.txt"] {
        let file = scene.from.join(name);
        fs::write(&file, b"new").expect("write");
        fs::write(scene.to.join(name), b"old").expect("write");
        sources.push(file);
    }
    let sources: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();
    let no_bin = |name: &str| Asked {
        no_recycle_bin: true,
        ..asked(name, NO_BIN_CHOICES)
    };

    let (summary, questions) = answering(
        &job(Kind::Copy, &sources, &scene.to),
        &keep,
        &[
            (Choice::Replace, true),
            (Choice::Skip, false),
            (Choice::Replace, true),
        ],
    );
    assert_eq!(
        questions,
        [
            asked("a.txt", FILE_CHOICES),
            no_bin("a.txt"),
            no_bin("b.txt")
        ],
        "c.txt goes as b.txt did"
    );
    assert_eq!((summary.files, summary.skipped), (2, 1));
    assert_eq!(fs::read(scene.to.join("a.txt")).expect("read"), b"old");
    assert_eq!(fs::read(scene.to.join("b.txt")).expect("read"), b"new");
    assert_eq!(fs::read(scene.to.join("c.txt")).expect("read"), b"new");
    assert_eq!(
        names(&scene.to),
        ["a.txt", "b.txt", "c.txt"],
        "no temporary file"
    );
}

#[test]
fn cancelling_while_the_job_asks_ends_it_and_replaces_nothing() {
    let scene = scene();
    let source = scene.from.join("a.txt");
    fs::write(&source, b"new").expect("write");
    let target = scene.to.join("a.txt");
    fs::write(&target, b"old").expect("write");
    let control = Control::new();
    let summary = run(
        &job(Kind::Copy, &[&source], &scene.to),
        &control,
        &keep,
        |event| {
            if let Event::Conflict(_) = event {
                control.cancel();
            }
        },
    )
    .expect("starts");
    assert_eq!(summary.status, Status::Cancelled);
    assert_eq!(fs::read(&target).expect("read"), b"old");
    assert_eq!(names(&scene.to), ["a.txt"]);
}

#[test]
fn a_move_by_renaming_replaces_merges_and_skips() {
    let scene = scene();
    let file = scene.from.join("a.txt");
    fs::write(&file, b"new a").expect("write");
    let skipped = scene.from.join("s.txt");
    fs::write(&skipped, b"mine").expect("write");
    let project = scene.from.join("project");
    fs::create_dir_all(project.join("sub")).expect("mkdir");
    fs::write(project.join("sub").join("b.txt"), b"new b").expect("write");
    fs::write(project.join("c.txt"), b"c").expect("write");
    fs::write(scene.to.join("a.txt"), b"old a").expect("write");
    fs::write(scene.to.join("s.txt"), b"theirs").expect("write");
    let there = scene.to.join("project");
    fs::create_dir_all(there.join("sub")).expect("mkdir");
    fs::write(there.join("sub").join("b.txt"), b"old b").expect("write");
    let recycled = long_path(scene.dir.path()).expect("long path").join("bin");

    let (summary, questions) = answering(
        &job(Kind::Move, &[&file, &skipped, &project], &scene.to),
        &bin_in(&recycled),
        &[
            (Choice::Replace, false),
            (Choice::Skip, false),
            (Choice::Merge, false),
            (Choice::KeepBoth, false),
        ],
    );
    assert_eq!(
        questions,
        [
            asked("a.txt", FILE_CHOICES),
            asked("s.txt", FILE_CHOICES),
            asked("project", FOLDER_CHOICES),
            asked("b.txt", FILE_CHOICES)
        ],
        "sub merges without asking"
    );
    assert_eq!(summary.failed, []);
    assert_eq!(summary.skipped, 1);
    assert_eq!(fs::read(scene.to.join("a.txt")).expect("read"), b"new a");
    assert_eq!(fs::read(recycled.join("a.txt")).expect("read"), b"old a");
    assert_eq!(fs::read(scene.to.join("s.txt")).expect("read"), b"theirs");
    assert_eq!(fs::read(&skipped).expect("stays"), b"mine");
    assert_eq!(
        fs::read(there.join("sub").join("b (2).txt")).expect("read"),
        b"new b"
    );
    assert_eq!(names(&scene.from), ["s.txt"], "the merged folders went");
    assert_eq!(
        names(&scene.to),
        [
            "a.txt",
            "b (2).txt",
            "b.txt",
            "c.txt",
            "project",
            "s.txt",
            "sub"
        ]
    );
}
