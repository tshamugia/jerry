//! The queue with a fake engine, so that a job takes exactly as long as a test wants and a test
//! machine with one disk has as many as the test needs.

use std::collections::BTreeSet;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Storage::FileSystem::{
    DDD_REMOVE_DEFINITION, DEFINE_DOS_DEVICE_FLAGS, DefineDosDeviceW,
};
use windows::core::PCWSTR;

use super::*;
use crate::transfer::conflict::{FILE_CHOICES, Policy};
use crate::transfer::control::Flow;
use crate::transfer::journal::Writing;
use crate::transfer::{Choice, Kind, Options, Progress};

/// What a fake job does.
#[derive(Clone)]
enum Script {
    /// Reports some progress and ends well.
    Finish,
    /// Begins a file whose temporary file is this, then ends well.
    Writes(PathBuf),
    /// Runs until the test says so.
    Hold(Arc<AtomicBool>),
    /// Stopped by its volume going away; the next turn finishes.
    GoneOnce,
    /// Its volume is gone before it can start.
    GoneAtStart,
    /// Fails one item.
    FailOne,
    /// Doesn't fit.
    NoSpace,
    Panic,
    /// Reports progress as fast as it can for this long.
    Chatter(Duration),
    /// Meets a conflict, waits for the answer, then ends well.
    Ask,
}

#[derive(Default)]
struct Fake {
    /// By a job's name, the last part of its destination.
    scripts: Mutex<HashMap<String, Script>>,
    /// `run a`, `end a`, `retry a` as they happen.
    log: Mutex<Vec<String>>,
    /// Paths that don't answer.
    missing: Mutex<HashSet<PathBuf>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned()
}

fn summary(status: Status, files: u64, failed: Vec<Failure>) -> Summary {
    Summary {
        status,
        files,
        folders: 0,
        bytes: files * 10,
        skipped: 0,
        failed,
        kind: Kind::Copy,
        destination: PathBuf::from(r"\\?\C:\dest"),
        options: Options::default(),
        policy: Policy::default(),
        pending: Vec::new(),
        leftovers: Vec::new(),
        resuming: None,
    }
}

fn failure(error: JerryError) -> Failure {
    Failure {
        source: PathBuf::from(r"\\?\C:\a.txt"),
        target: PathBuf::from(r"\\?\C:\dest\a.txt"),
        error,
        copied: false,
    }
}

impl Fake {
    fn note(&self, what: &str, name: &str) {
        lock(&self.log).push(format!("{what} {name}"));
    }

    fn report(observer: &mut dyn FnMut(Event<'_>), done: u64) {
        observer(Event::Progress(&Progress {
            file: Path::new(r"\\?\C:\ნახაზი.dwg"),
            files_done: done,
            files_total: 3,
            bytes_done: done * 10,
            bytes_total: 30,
            speed: 5,
        }));
    }

    fn perform(
        &self,
        name: &str,
        retried: bool,
        control: &Control,
        observer: &mut dyn FnMut(Event<'_>),
    ) -> Result<Summary, JerryError> {
        let script = lock(&self.scripts)
            .get(name)
            .cloned()
            .unwrap_or(Script::Finish);
        let result = match script {
            Script::Finish => self.finish_it(observer),
            Script::Writes(part) => {
                Self::report(observer, 1);
                observer(Event::Writing(&Writing {
                    source: Path::new(r"\\?\C:\from\b.txt"),
                    target: Path::new(r"\\?\C:\to\b.txt"),
                    part: Some(&part),
                }));
                self.finish_it(observer)
            }
            Script::Hold(release) => {
                Self::report(observer, 1);
                loop {
                    if control.checkpoint() == Flow::Cancelled {
                        break Ok(summary(Status::Cancelled, 0, Vec::new()));
                    }
                    if release.load(Ordering::SeqCst) {
                        break self.finish_it(observer);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            Script::GoneOnce if !retried => {
                Self::report(observer, 1);
                let gone = JerryError::new(ErrorCode::DeviceGone);
                Ok(summary(
                    Status::Interrupted(gone.clone()),
                    1,
                    vec![failure(gone)],
                ))
            }
            Script::GoneAtStart => Err(JerryError::new(ErrorCode::DeviceGone)),
            Script::GoneOnce => self.finish_it(observer),
            Script::FailOne if !retried => {
                observer(Event::Failed(&failure(JerryError::from_win32(5))));
                Ok(summary(
                    Status::Done,
                    2,
                    vec![failure(JerryError::from_win32(5))],
                ))
            }
            Script::FailOne => self.finish_it(observer),
            Script::NoSpace => Err(JerryError::new(ErrorCode::NoSpace {
                needed: Some(10),
                available: Some(1),
            })),
            Script::Panic => panic!("deliberate panic"),
            Script::Chatter(duration) => {
                let start = Instant::now();
                while start.elapsed() < duration {
                    Self::report(observer, 1);
                    std::thread::sleep(Duration::from_micros(200));
                }
                self.finish_it(observer)
            }
            Script::Ask => {
                Self::report(observer, 1);
                let side = Side {
                    folder: false,
                    attributes: 0x20,
                    size: 10,
                    modified: 0,
                };
                let conflict = Conflict {
                    source: Path::new(r"\\?\C:\from\a.txt"),
                    target: Path::new(r"\\?\C:\to\a.txt"),
                    incoming: side,
                    existing: side,
                    choices: FILE_CHOICES,
                    no_recycle_bin: false,
                };
                control.forget_answer();
                observer(Event::Conflict(&conflict));
                match control.wait_answer() {
                    Some(answer) => {
                        self.note("answered", &format!("{name} {:?}", answer.choice));
                        self.finish_it(observer)
                    }
                    None => Ok(summary(Status::Cancelled, 0, Vec::new())),
                }
            }
        };
        self.note("end", name);
        result
    }

    fn finish_it(&self, observer: &mut dyn FnMut(Event<'_>)) -> Result<Summary, JerryError> {
        Self::report(observer, 3);
        Ok(summary(Status::Done, 3, Vec::new()))
    }
}

impl Engine for Arc<Fake> {
    fn run(
        &self,
        job: &Job,
        control: &Control,
        observer: &mut dyn FnMut(Event<'_>),
    ) -> Result<Summary, JerryError> {
        let name = name_of(&job.destination);
        self.note("run", &name);
        self.perform(&name, false, control, observer)
    }

    fn retry(
        &self,
        summary: &Summary,
        control: &Control,
        observer: &mut dyn FnMut(Event<'_>),
    ) -> Result<Summary, JerryError> {
        // A retry has only the summary to go by: fake ones name the job in `pending`.
        let name = summary
            .pending
            .first()
            .map(|(_, target)| name_of(target))
            .unwrap_or_else(|| name_of(&summary.destination));
        self.note("retry", &name);
        self.perform(&name, true, control, observer)
    }

    fn resume(
        &self,
        job: &Job,
        resume: &Resume,
        control: &Control,
        observer: &mut dyn FnMut(Event<'_>),
    ) -> Result<Summary, JerryError> {
        let name = name_of(&job.destination);
        let writing = resume
            .writing
            .as_ref()
            .map_or_else(String::new, |(source, _)| {
                format!(" at {}", name_of(source))
            });
        self.note("resume", &format!("{name}{writing}"));
        self.perform(&name, true, control, observer)
    }

    fn lane(&self, destination: &Path) -> Lane {
        // `disk1\job`: the parent's name says which disk.
        let disk = destination
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("disk"))
            .and_then(|number| number.parse().ok());
        disk.map_or_else(|| Lane::Volume("?".into()), Lane::Disk)
    }

    fn root(&self, path: &Path) -> Option<PathBuf> {
        volume_of(path)
    }

    fn reachable(&self, path: &Path) -> bool {
        !lock(&self.missing).contains(path)
    }
}

/// `\\?\E:\` for a path that starts with `E:` once the test's prefix is off.
fn volume_of(path: &Path) -> Option<PathBuf> {
    let text = path.to_string_lossy();
    let letter = text.strip_prefix(r"\\?\")?.chars().next()?;
    Some(PathBuf::from(format!(r"\\?\{letter}:\")))
}

/// A journal that notes what it hears: `queued 1`, `started 1 3/10`, `writing 1 b.txt.jerrypart
/// 3/10`, `part gone 1`, `ended 1`. Records count from 1.
#[derive(Default)]
struct Notes {
    lines: Mutex<Vec<String>>,
    records: Mutex<i64>,
}

impl Notes {
    fn lines(&self) -> Vec<String> {
        lock(&self.lines).clone()
    }

    fn note(&self, line: String) {
        lock(&self.lines).push(line);
    }
}

impl Journal for Notes {
    fn queued(&self, _: &Job) -> Option<i64> {
        let mut records = lock(&self.records);
        *records += 1;
        self.note(format!("queued {records}"));
        Some(*records)
    }

    fn started(&self, record: i64, counts: &Counts) {
        let Counts {
            files_done,
            files_total,
            ..
        } = counts;
        self.note(format!("started {record} {files_done}/{files_total}"));
    }

    fn writing(&self, record: i64, writing: &Writing<'_>, counts: &Counts) {
        let part = writing.part.map_or_else(String::new, name_of);
        let Counts {
            files_done,
            files_total,
            ..
        } = counts;
        self.note(format!(
            "writing {record} {part} {files_done}/{files_total}"
        ));
    }

    fn part_gone(&self, record: i64) {
        self.note(format!("part gone {record}"));
    }

    fn ended(&self, record: i64) {
        self.note(format!("ended {record}"));
    }
}

struct Scene {
    fake: Arc<Fake>,
    journal: Arc<Notes>,
    queue: Queue,
    notices: Arc<Mutex<Vec<Notice>>>,
}

const FAST: Timing = Timing {
    progress: PROGRESS_EVERY,
    watch: Duration::from_millis(5),
};

fn scene(parallel: bool) -> Scene {
    let fake = Arc::new(Fake::default());
    let journal = Arc::new(Notes::default());
    let notices = Arc::new(Mutex::new(Vec::new()));
    let sink = notices.clone();
    let queue = Queue::with(
        fake.clone(),
        parallel,
        FAST,
        journal.clone(),
        Arc::new(NoShell),
        move |notice| {
            lock(&sink).push(notice);
        },
    );
    Scene {
        fake,
        journal,
        queue,
        notices,
    }
}

/// A job from volume C: to job `name` on disk `disk`.
fn job(disk: u32, name: &str) -> Job {
    Job {
        kind: Kind::Copy,
        sources: vec![PathBuf::from(r"\\?\C:\from\a.txt")],
        destination: PathBuf::from(format!(r"\\?\C:\disk{disk}\{name}")),
    }
}

impl Scene {
    fn script(&self, name: &str, script: Script) {
        lock(&self.fake.scripts).insert(name.to_owned(), script);
    }

    /// A job that runs until the returned flag is set.
    fn hold(&self, name: &str) -> Arc<AtomicBool> {
        let release = Arc::new(AtomicBool::new(false));
        self.script(name, Script::Hold(release.clone()));
        release
    }

    fn log(&self) -> Vec<String> {
        lock(&self.fake.log).clone()
    }

    fn state(&self, id: u32) -> TransferState {
        self.job_view(id).state
    }

    fn job_view(&self, id: u32) -> TransferJob {
        self.queue
            .list()
            .jobs
            .into_iter()
            .find(|job| job.id == id)
            .expect("the job is listed")
    }

    fn wait_for(&self, what: &str, done: impl Fn(&Self) -> bool) {
        let start = Instant::now();
        while !done(self) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "waited for {what}: {:?}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn wait_state(&self, id: u32, state: &TransferState) {
        self.wait_for(&format!("{id} to be {state:?}"), |scene| {
            scene.state(id) == *state
        });
    }

    fn wait_log(&self, line: &str) {
        self.wait_for(line, |scene| scene.log().iter().any(|each| each == line));
    }

    fn ended(&self, id: u32) -> Box<TransferEnded> {
        lock(&self.notices)
            .iter()
            .find_map(|notice| match notice {
                Notice::Ended(ended) if ended.id == id => Some(ended.clone()),
                _ => None,
            })
            .expect("it ended")
    }
}

fn gone() -> JerryError {
    JerryError::new(ErrorCode::DeviceGone)
}

#[test]
fn jobs_on_two_disks_run_at_the_same_time() {
    let scene = scene(true);
    let (one, two) = (scene.hold("a"), scene.hold("b"));
    let a = scene.queue.enqueue(job(1, "a"));
    let b = scene.queue.enqueue(job(2, "b"));

    // Both started while neither has ended.
    scene.wait_log("run a");
    scene.wait_log("run b");
    assert_eq!(scene.state(a), TransferState::Running);
    assert_eq!(scene.state(b), TransferState::Running);

    one.store(true, Ordering::SeqCst);
    two.store(true, Ordering::SeqCst);
    scene.wait_state(a, &TransferState::Done);
    scene.wait_state(b, &TransferState::Done);
}

#[test]
fn jobs_on_one_disk_run_one_after_the_other() {
    let scene = scene(true);
    let first = scene.hold("a");
    let a = scene.queue.enqueue(job(1, "a"));
    let b = scene.queue.enqueue(job(1, "b"));

    scene.wait_log("run a");
    assert_eq!(scene.state(b), TransferState::Queued);
    assert!(!scene.log().contains(&"run b".to_owned()));

    first.store(true, Ordering::SeqCst);
    scene.wait_state(b, &TransferState::Done);
    assert_eq!(scene.log(), ["run a", "end a", "run b", "end b"]);
    assert_eq!(scene.state(a), TransferState::Done);
}

#[test]
fn with_the_setting_off_only_one_job_runs_at_all() {
    let scene = scene(false);
    let first = scene.hold("a");
    scene.queue.enqueue(job(1, "a"));
    let b = scene.queue.enqueue(job(2, "b"));
    scene.wait_log("run a");
    assert_eq!(scene.state(b), TransferState::Queued);

    // Turning it on lets the other disk go, without waiting for the first job.
    scene.queue.set_parallel(true);
    scene.wait_state(b, &TransferState::Done);
    first.store(true, Ordering::SeqCst);
}

#[test]
fn turning_the_setting_off_leaves_running_jobs_alone() {
    let scene = scene(true);
    let (one, two) = (scene.hold("a"), scene.hold("b"));
    let a = scene.queue.enqueue(job(1, "a"));
    let b = scene.queue.enqueue(job(2, "b"));
    scene.wait_log("run a");
    scene.wait_log("run b");

    scene.queue.set_parallel(false);
    let c = scene.queue.enqueue(job(3, "c"));
    assert_eq!(scene.state(c), TransferState::Queued);
    assert_eq!(scene.state(a), TransferState::Running);
    assert_eq!(scene.state(b), TransferState::Running);

    one.store(true, Ordering::SeqCst);
    two.store(true, Ordering::SeqCst);
    scene.wait_state(c, &TransferState::Done);
}

#[test]
fn reordering_changes_what_runs_next() {
    let scene = scene(true);
    let first = scene.hold("a");
    let a = scene.queue.enqueue(job(1, "a"));
    let b = scene.queue.enqueue(job(1, "b"));
    let c = scene.queue.enqueue(job(1, "c"));
    scene.wait_log("run a");

    scene.queue.reorder(&[a, c, b]);
    let order: Vec<u32> = scene.queue.list().jobs.iter().map(|job| job.id).collect();
    assert_eq!(order, [a, c, b]);

    first.store(true, Ordering::SeqCst);
    scene.wait_state(b, &TransferState::Done);
    assert_eq!(
        scene.log(),
        ["run a", "end a", "run c", "end c", "run b", "end b"]
    );
}

#[test]
fn reordering_keeps_jobs_it_does_not_name_and_ignores_ones_it_does_not_know() {
    let scene = scene(true);
    let first = scene.hold("a");
    let a = scene.queue.enqueue(job(1, "a"));
    let b = scene.queue.enqueue(job(1, "b"));
    let c = scene.queue.enqueue(job(1, "c"));
    scene.wait_log("run a");

    scene.queue.reorder(&[99, c]);
    let order: Vec<u32> = scene.queue.list().jobs.iter().map(|job| job.id).collect();
    assert_eq!(order, [c, a, b]);
    first.store(true, Ordering::SeqCst);
    scene.wait_state(b, &TransferState::Done);
}

#[test]
fn pause_all_holds_what_runs_and_what_waits() {
    let scene = scene(true);
    let (one, two) = (scene.hold("a"), scene.hold("b"));
    let a = scene.queue.enqueue(job(1, "a"));
    let b = scene.queue.enqueue(job(2, "b"));
    let c = scene.queue.enqueue(job(1, "c"));
    scene.wait_log("run a");
    scene.wait_log("run b");

    scene.queue.control(Action::Pause, None).expect("pauses");
    for id in [a, b, c] {
        assert_eq!(scene.state(id), TransferState::Paused);
    }
    // The jobs finishing their work wait at their checkpoint: nothing ends, and C doesn't start.
    one.store(true, Ordering::SeqCst);
    two.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(30));
    assert!(!scene.log().iter().any(|each| each == "run c"));
    assert!(!scene.log().iter().any(|each| each.starts_with("end")));

    scene.queue.control(Action::Resume, None).expect("resumes");
    for id in [a, b, c] {
        scene.wait_state(id, &TransferState::Done);
    }
    assert_eq!(
        scene.log().iter().filter(|each| *each == "run c").count(),
        1
    );
}

#[test]
fn pausing_a_job_that_waits_lets_the_next_one_go() {
    let scene = scene(true);
    let first = scene.hold("a");
    scene.queue.enqueue(job(1, "a"));
    let b = scene.queue.enqueue(job(1, "b"));
    let c = scene.queue.enqueue(job(1, "c"));
    scene.wait_log("run a");

    scene.queue.control(Action::Pause, Some(b)).expect("pauses");
    first.store(true, Ordering::SeqCst);
    scene.wait_state(c, &TransferState::Done);
    assert_eq!(scene.state(b), TransferState::Paused);
    assert!(!scene.log().iter().any(|each| each == "run b"));

    scene
        .queue
        .control(Action::Resume, Some(b))
        .expect("resumes");
    scene.wait_state(b, &TransferState::Done);
}

#[test]
fn a_paused_running_job_keeps_its_disk() {
    let scene = scene(true);
    let first = scene.hold("a");
    let a = scene.queue.enqueue(job(1, "a"));
    let b = scene.queue.enqueue(job(1, "b"));
    scene.wait_log("run a");

    scene.queue.control(Action::Pause, Some(a)).expect("pauses");
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(scene.state(b), TransferState::Queued, "one job per disk");

    first.store(true, Ordering::SeqCst);
    scene
        .queue
        .control(Action::Resume, Some(a))
        .expect("resumes");
    scene.wait_state(b, &TransferState::Done);
}

#[test]
fn cancel_ends_one_job_or_all_of_them() {
    let scene = scene(true);
    let (_one, _two) = (scene.hold("a"), scene.hold("b"));
    let a = scene.queue.enqueue(job(1, "a"));
    let b = scene.queue.enqueue(job(2, "b"));
    let queued = scene.queue.enqueue(job(1, "c"));
    let other = scene.queue.enqueue(job(1, "d"));
    scene.wait_log("run a");
    scene.wait_log("run b");

    scene
        .queue
        .control(Action::Cancel, Some(queued))
        .expect("cancels");
    assert_eq!(scene.state(queued), TransferState::Cancelled, "at once");
    assert_eq!(scene.ended(queued).state, TransferState::Cancelled);

    // A paused job cancels too.
    scene.queue.control(Action::Pause, Some(a)).expect("pauses");
    scene.queue.control(Action::Cancel, None).expect("cancels");
    for id in [a, b, other] {
        scene.wait_state(id, &TransferState::Cancelled);
    }
    assert!(!scene.log().iter().any(|each| each == "run d"));
    assert!(!scene.log().iter().any(|each| each == "run c"));
    assert_eq!(scene.queue.on_volume(Path::new(r"\\?\C:\")), 0);
}

#[test]
fn control_of_a_job_that_is_not_there_says_so_and_of_one_that_ended_does_nothing() {
    let scene = scene(true);
    let code = scene
        .queue
        .control(Action::Pause, Some(7))
        .map_err(|e| e.code);
    assert_eq!(code, Err(ErrorCode::NotFound));

    let a = scene.queue.enqueue(job(1, "a"));
    scene.wait_state(a, &TransferState::Done);
    scene
        .queue
        .control(Action::Cancel, Some(a))
        .expect("no error");
    scene
        .queue
        .control(Action::Pause, Some(a))
        .expect("no error");
    assert_eq!(scene.state(a), TransferState::Done);
}

#[test]
fn a_volume_going_away_waits_frees_the_disk_and_resumes_when_it_is_back() {
    let scene = scene(true);
    scene.script("a", Script::GoneOnce);
    let destination = PathBuf::from(r"\\?\C:\disk1\a");
    lock(&scene.fake.missing).insert(destination.clone());

    let a = scene.queue.enqueue(job(1, "a"));
    let waiting = TransferState::Waiting {
        error: gone(),
        ready: false,
    };
    scene.wait_state(a, &waiting);

    // Waiting holds no disk.
    let b = scene.queue.enqueue(job(1, "b"));
    scene.wait_state(b, &TransferState::Done);

    // Resume means nothing until the device is back.
    scene
        .queue
        .control(Action::Resume, Some(a))
        .expect("no error");
    assert_eq!(scene.state(a), waiting);

    lock(&scene.fake.missing).remove(&destination);
    let ready = TransferState::Waiting {
        error: gone(),
        ready: true,
    };
    scene.wait_state(a, &ready);
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(scene.state(a), ready, "the user decides, not the queue");

    scene
        .queue
        .control(Action::Resume, Some(a))
        .expect("resumes");
    scene.wait_state(a, &TransferState::Done);
    assert_eq!(
        lock(&scene.fake.log)
            .iter()
            .filter(|each| each.ends_with(" a") || each.ends_with(" dest"))
            .cloned()
            .collect::<Vec<_>>(),
        ["run a", "end a", "retry dest", "end dest"]
    );
    // What the first turn did counts: 1 file, then the 3 the retry reports.
    let done = scene.job_view(a).progress;
    assert_eq!((done.files_done, done.files_total), (4, 4));
    assert_eq!(done.failed, 0);
    assert_eq!(scene.ended(a).files, 4);
}

#[test]
fn a_job_that_waits_for_its_device_can_be_cancelled() {
    let scene = scene(true);
    scene.script("a", Script::GoneOnce);
    lock(&scene.fake.missing).insert(PathBuf::from(r"\\?\C:\disk1\a"));
    let a = scene.queue.enqueue(job(1, "a"));
    scene.wait_for("waiting", |scene| {
        matches!(scene.state(a), TransferState::Waiting { .. })
    });

    scene
        .queue
        .control(Action::Cancel, Some(a))
        .expect("cancels");
    assert_eq!(scene.state(a), TransferState::Cancelled);
    let ended = scene.ended(a);
    assert_eq!(ended.files, 1);
    assert_eq!(ended.failed.len(), 1, "what it had given up on");
}

#[test]
fn a_device_gone_before_the_job_starts_waits_too() {
    let scene = scene(true);
    scene.script("a", Script::GoneAtStart);
    let a = scene.queue.enqueue(job(1, "a"));
    scene.wait_state(
        a,
        &TransferState::Waiting {
            error: gone(),
            ready: true,
        },
    );
    scene.script("a", Script::Finish);
    scene
        .queue
        .control(Action::Resume, Some(a))
        .expect("resumes");
    scene.wait_state(a, &TransferState::Done);
    assert_eq!(
        scene.log(),
        ["run a", "end a", "run a", "end a"],
        "from the start"
    );
}

#[test]
fn a_job_that_does_not_fit_fails_and_can_be_tried_again() {
    let scene = scene(true);
    scene.script("a", Script::NoSpace);
    let a = scene.queue.enqueue(job(1, "a"));
    let no_space = ErrorCode::NoSpace {
        needed: Some(10),
        available: Some(1),
    };
    scene.wait_state(
        a,
        &TransferState::Failed {
            error: JerryError::new(no_space),
        },
    );
    scene.script("a", Script::Finish);
    scene.queue.retry_failed(a).expect("tries again");
    scene.wait_state(a, &TransferState::Done);
}

#[test]
fn retry_failed_tries_only_what_failed_and_only_if_something_did() {
    let scene = scene(true);
    scene.script("a", Script::FailOne);
    let a = scene.queue.enqueue(job(1, "a"));
    let clean = scene.queue.enqueue(job(2, "b"));
    scene.wait_state(a, &TransferState::Done);
    scene.wait_state(clean, &TransferState::Done);

    assert_eq!(scene.job_view(a).progress.failed, 1);
    let ended = scene.ended(a);
    assert_eq!(ended.failed.len(), 1);
    assert_eq!(ended.failed[0].source, r"C:\a.txt", "display text");
    assert_eq!(ended.failed[0].error.code, ErrorCode::AccessDenied);

    let nothing = scene.queue.retry_failed(clean).map_err(|e| e.code);
    assert_eq!(nothing, Err(ErrorCode::NotFound));

    scene.queue.retry_failed(a).expect("tries again");
    scene.wait_for("the retry", |scene| {
        scene.log().iter().any(|each| each == "retry dest")
    });
    scene.wait_state(a, &TransferState::Done);
    let done = scene.job_view(a).progress;
    assert_eq!(done.failed, 0);
    assert_eq!(done.files_done, 5, "2, then the 3 the retry reports");
    let order: Vec<u32> = scene.queue.list().jobs.iter().map(|job| job.id).collect();
    assert_eq!(order, [clean, a], "it went to the end of the queue");
    assert_eq!(
        scene.queue.retry_failed(a).map_err(|e| e.code),
        Err(ErrorCode::NotFound)
    );
}

#[test]
fn a_panic_fails_only_its_job_and_the_disk_goes_on() {
    let scene = scene(true);
    scene.script("a", Script::Panic);
    let a = scene.queue.enqueue(job(1, "a"));
    let b = scene.queue.enqueue(job(1, "b"));
    scene.wait_state(
        a,
        &TransferState::Failed {
            error: JerryError::internal(),
        },
    );
    scene.wait_state(b, &TransferState::Done);
}

#[test]
fn progress_reaches_the_ui_at_most_ten_times_a_second() {
    let scene = scene(true);
    let span = Duration::from_millis(350);
    scene.script("a", Script::Chatter(span));
    scene.script("b", Script::Chatter(span));
    let start = Instant::now();
    let a = scene.queue.enqueue(job(1, "a"));
    let b = scene.queue.enqueue(job(2, "b"));
    scene.wait_state(a, &TransferState::Done);
    scene.wait_state(b, &TransferState::Done);
    let elapsed = start.elapsed();

    let notices = lock(&scene.notices);
    let reports: Vec<&Vec<TransferProgress>> = notices
        .iter()
        .filter_map(|notice| match notice {
            Notice::Progress(running) => Some(running),
            _ => None,
        })
        .collect();
    let most = 1 + (elapsed.as_millis() / PROGRESS_EVERY.as_millis()) as usize;
    assert!(reports.len() <= most, "{} in {elapsed:?}", reports.len());
    assert!(reports.len() >= 2, "it does report: {}", reports.len());
    // One event for both jobs, not one each.
    assert!(reports.iter().any(|running| running.len() == 2));
    // A report lists every running job, so one whose engine has said nothing
    // yet has no file. Check a job that has.
    let report = reports
        .iter()
        .flat_map(|running| running.iter())
        .find(|progress| progress.file.is_some())
        .expect("a report names a file");
    assert_eq!(
        report.file.as_deref(),
        Some(r"C:\ნახაზი.dwg"),
        "display text"
    );
    assert_eq!((report.files_total, report.bytes_total), (3, 30));
}

#[test]
fn every_change_is_reported_with_a_higher_generation() {
    let scene = scene(true);
    let first = scene.hold("a");
    let a = scene.queue.enqueue(job(1, "a"));
    scene.wait_log("run a");
    scene.queue.control(Action::Pause, Some(a)).expect("pauses");
    first.store(true, Ordering::SeqCst);
    scene
        .queue
        .control(Action::Resume, Some(a))
        .expect("resumes");
    scene.wait_state(a, &TransferState::Done);

    let notices = lock(&scene.notices);
    let lists: Vec<&TransferList> = notices
        .iter()
        .filter_map(|notice| match notice {
            Notice::Jobs(list) => Some(list),
            _ => None,
        })
        .collect();
    assert!(
        lists
            .windows(2)
            .all(|two| two[1].generation > two[0].generation)
    );
    let states: Vec<TransferState> = lists
        .iter()
        .map(|list| list.jobs[0].state.clone())
        .collect();
    assert_eq!(
        states,
        [
            TransferState::Running,
            TransferState::Paused,
            TransferState::Running,
            TransferState::Done
        ]
    );
    let shown = &lists[0].jobs[0];
    assert_eq!(shown.kind, Kind::Copy);
    assert_eq!(shown.source, r"C:\from\a.txt");
    assert_eq!(shown.sources, 1);
    assert_eq!(shown.destination, r"C:\disk1\a");
}

#[test]
fn eject_hears_of_jobs_on_a_volume_until_they_end() {
    let scene = scene(true);
    let hold = scene.hold("a");
    let a = scene.queue.enqueue(Job {
        kind: Kind::Move,
        sources: vec![PathBuf::from(r"\\?\E:\from\a.txt")],
        destination: PathBuf::from(r"\\?\C:\disk1\a"),
    });
    scene.wait_log("run a");
    let b = scene.queue.enqueue(Job {
        kind: Kind::Copy,
        sources: vec![PathBuf::from(r"\\?\C:\from\b.txt")],
        destination: PathBuf::from(r"\\?\E:\disk1\b"),
    });

    assert_eq!(scene.queue.on_volume(Path::new(r"\\?\C:\")), 2);
    assert_eq!(scene.queue.on_volume(Path::new(r"\\?\e:\")), 2, "any case");
    assert_eq!(scene.queue.on_volume(Path::new(r"\\?\F:\")), 0);

    hold.store(true, Ordering::SeqCst);
    scene.wait_state(a, &TransferState::Done);
    scene.wait_state(b, &TransferState::Done);
    assert_eq!(scene.queue.on_volume(Path::new(r"\\?\E:\")), 0);
}

#[test]
fn only_the_last_ended_jobs_stay_listed() {
    let scene = scene(true);
    let total = ENDED_KEPT as u32 + 5;
    let mut last = 0;
    for n in 0..total {
        last = scene.queue.enqueue(job(1, &format!("j{n}")));
    }
    scene.wait_state(last, &TransferState::Done);
    let ids: BTreeSet<u32> = scene.queue.list().jobs.iter().map(|job| job.id).collect();
    assert_eq!(ids.len(), ENDED_KEPT);
    assert!(!ids.contains(&1), "the oldest went first");
    assert!(ids.contains(&last));
}

/// A drive letter over a folder, as `subst` makes one, for as long as a test needs it.
struct Letter {
    letter: char,
    folder: Vec<u16>,
    defined: bool,
}

impl Letter {
    /// Takes a free letter below J: other tests look for missing drives from Z down.
    fn over(folder: &Path) -> Self {
        let letter = (b'E'..=b'I')
            .map(char::from)
            .find(|letter| !Path::new(&format!(r"{letter}:\")).exists())
            .expect("a free drive letter");
        let folder = crate::path::plain_path(folder)
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect();
        let mut made = Self {
            letter,
            folder,
            defined: false,
        };
        made.define();
        made
    }

    fn device(&self) -> Vec<u16> {
        format!("{}:", self.letter)
            .encode_utf16()
            .chain([0])
            .collect()
    }

    fn define(&mut self) {
        let device = self.device();
        // SAFETY: both strings are NUL-terminated and outlive the call.
        unsafe {
            DefineDosDeviceW(
                DEFINE_DOS_DEVICE_FLAGS(0),
                PCWSTR(device.as_ptr()),
                PCWSTR(self.folder.as_ptr()),
            )
        }
        .expect("the letter is defined");
        self.defined = true;
    }

    fn remove(&mut self) {
        let device = self.device();
        // SAFETY: both strings are NUL-terminated and outlive the call.
        unsafe {
            DefineDosDeviceW(
                DDD_REMOVE_DEFINITION,
                PCWSTR(device.as_ptr()),
                PCWSTR(self.folder.as_ptr()),
            )
        }
        .expect("the letter is removed");
        self.defined = false;
    }
}

impl Drop for Letter {
    fn drop(&mut self) {
        if self.defined {
            self.remove();
        }
    }
}

/// What real paths do when a drive goes away: the file being copied finishes, because its handles
/// stay open, and the next one is only "path not found".
#[test]
fn a_drive_that_goes_away_between_files_waits_and_goes_on_when_it_is_back() {
    use crate::path::long_path;

    const FILES: usize = 400;
    let dir = tempfile::Builder::new()
        .prefix("jerry-queue-")
        .tempdir()
        .expect("temp folder");
    let root = long_path(dir.path()).expect("long path");
    let (from, disk) = (root.join("from"), root.join("disk"));
    std::fs::create_dir(&from).expect("mkdir");
    std::fs::create_dir_all(disk.join("to")).expect("mkdir");
    let sources: Vec<PathBuf> = (0..FILES)
        .map(|n| {
            let file = from.join(format!("ნახაზი-{n:03}.dwg"));
            std::fs::write(&file, b"x").expect("write");
            file
        })
        .collect();
    let mut letter = Letter::over(&disk);

    let system = System {
        shell: Arc::new(NoShell),
    };
    let queue = Queue::with(system, true, FAST, Arc::new(()), Arc::new(NoShell), |_| {});
    let id = queue.enqueue(Job {
        kind: Kind::Copy,
        sources,
        destination: PathBuf::from(format!(r"\\?\{}:\to", letter.letter)),
    });
    let view = || {
        queue
            .list()
            .jobs
            .into_iter()
            .find(|job| job.id == id)
            .expect("listed")
    };
    let until = |what: &str, done: &dyn Fn(&TransferJob) -> bool| {
        let start = Instant::now();
        while !done(&view()) {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "{what}: {:?}",
                view()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    };

    until("a few files copied", &|job| job.progress.files_done >= 3);
    queue.control(Action::Pause, Some(id)).expect("pauses");
    // What Eject asks, on the root Windows names for a real volume. (A `subst` letter's root is
    // the real volume behind it; such a drive has no Eject.)
    let source_drive = display_path(&root).chars().next().expect("a drive letter");
    let on_source_drive = || queue.on_volume(Path::new(&format!(r"\\?\{source_drive}:\")));
    assert_eq!(on_source_drive(), 1);
    letter.remove();
    queue.control(Action::Resume, Some(id)).expect("resumes");
    until("waiting for the drive", &|job| {
        matches!(
            &job.state,
            TransferState::Waiting { error, ready: false }
                if error.code == ErrorCode::DeviceGone
        )
    });
    assert!(view().progress.files_done < FILES as u64);

    letter.define();
    until("the drive is back", &|job| {
        matches!(job.state, TransferState::Waiting { ready: true, .. })
    });
    queue.control(Action::Resume, Some(id)).expect("resumes");
    until("done", &|job| job.state == TransferState::Done);
    assert_eq!(on_source_drive(), 0, "finished, so nothing to warn of");

    let names: Vec<String> = std::fs::read_dir(disk.join("to"))
        .expect("reads")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let (parts, files): (Vec<&String>, Vec<&String>) =
        names.iter().partition(|name| name.ends_with(".jerrypart"));
    assert_eq!(files.len(), FILES, "every file, once");
    // The file being finished as the drive went couldn't be tidied away without the drive; the
    // turn after Resume removed it (data safety rule 7).
    assert!(parts.is_empty(), "{parts:?}");
    let done = view().progress;
    assert_eq!(
        (done.files_done, done.files_total),
        (FILES as u64, FILES as u64)
    );
    assert_eq!(done.failed, 0);
}

/// The real engine and real files: what the queue adds around it doesn't get in the way.
#[test]
fn a_real_job_goes_through_the_queue() {
    use crate::path::long_path;

    let dir = tempfile::Builder::new()
        .prefix("jerry-queue-")
        .tempdir()
        .expect("temp folder");
    let root = long_path(dir.path()).expect("long path");
    let (from, to) = (root.join("from"), root.join("დანიშნულება"));
    std::fs::create_dir(&from).expect("mkdir");
    std::fs::create_dir(&to).expect("mkdir");
    let file = from.join("ნახაზი.dwg");
    std::fs::write(&file, vec![b'x'; 4096]).expect("write");

    let notices = Arc::new(Mutex::new(Vec::new()));
    let sink = notices.clone();
    let queue = Queue::start(true, Arc::new(()), Arc::new(NoShell), move |notice| {
        lock(&sink).push(notice);
    });
    let id = queue.enqueue(Job {
        kind: Kind::Move,
        sources: vec![file.clone()],
        destination: to.clone(),
    });
    let start = Instant::now();
    while !matches!(queue.list().jobs[0].state, TransferState::Done) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            queue.list()
        );
        std::thread::sleep(Duration::from_millis(2));
    }

    assert!(!file.exists());
    assert!(to.join("ნახაზი.dwg").exists());
    let ended = lock(&notices)
        .iter()
        .find_map(|notice| match notice {
            Notice::Ended(ended) if ended.id == id => Some(ended.clone()),
            _ => None,
        })
        .expect("it ended");
    assert_eq!(ended.files, 1);
    assert_eq!(ended.failed, []);
}

impl Scene {
    fn wait_journal(&self, line: &str) {
        self.wait_for(line, |scene| {
            scene.journal.lines().iter().any(|each| each == line)
        });
    }
}

#[test]
fn a_job_is_written_down_before_it_starts_and_as_it_ends() {
    let scene = scene(true);
    let release = scene.hold("a");
    let id = scene.queue.enqueue(job(1, "a"));
    scene.wait_log("run a");
    assert_eq!(scene.journal.lines(), ["queued 1", "started 1 0/0"]);
    release.store(true, Ordering::SeqCst);
    scene.wait_state(id, &TransferState::Done);
    scene.wait_journal("ended 1");
    assert_eq!(
        scene.journal.lines(),
        ["queued 1", "started 1 0/0", "ended 1"]
    );
}

#[test]
fn a_file_a_job_begins_is_written_down_and_a_part_it_leaves_goes() {
    let scene = scene(true);
    let dir = tempfile::tempdir().expect("temp folder");
    let part = crate::path::long_path(dir.path())
        .expect("long path")
        .join("b.txt.jerrypart");
    // As if the engine couldn't remove it.
    std::fs::write(&part, b"half").expect("write");
    scene.script("a", Script::Writes(part.clone()));
    let id = scene.queue.enqueue(job(1, "a"));
    scene.wait_state(id, &TransferState::Done);
    scene.wait_journal("ended 1");
    assert_eq!(
        scene.journal.lines(),
        [
            "queued 1",
            "started 1 0/0",
            "writing 1 b.txt.jerrypart 1/3",
            "part gone 1",
            "ended 1",
        ]
    );
    assert!(!part.exists());
}

fn unfinished(record: i64, name: &str, started: bool) -> Unfinished {
    let counts = Counts {
        files_done: 3,
        files_total: 10,
        bytes_done: 30,
        bytes_total: 100,
    };
    Unfinished {
        record,
        job: job(1, name),
        started,
        counts: if started { counts } else { Counts::default() },
        writing: started.then(|| {
            (
                PathBuf::from(r"\\?\C:\from\a.txt"),
                PathBuf::from(format!(r"\\?\C:\disk1\{name}\a.txt")),
            )
        }),
        part: None,
    }
}

#[test]
fn a_job_a_jerry_did_not_finish_waits_for_resume_and_carries_on() {
    let scene = scene(true);
    scene
        .queue
        .recover(vec![unfinished(7, "a", true), unfinished(8, "b", false)]);
    let jobs = scene.queue.list().jobs;
    let (a, b) = (jobs[0].id, jobs[1].id);
    assert_eq!(jobs[0].state, TransferState::Unfinished);
    assert_eq!(jobs[1].state, TransferState::Unfinished);
    let progress = &jobs[0].progress;
    assert_eq!((progress.files_done, progress.files_total), (3, 10));
    assert_eq!(progress.file.as_deref(), Some(r"C:\from\a.txt"));
    std::thread::sleep(Duration::from_millis(20));
    assert!(scene.log().is_empty(), "nothing starts by itself");
    assert_eq!(
        scene.queue.on_volume(Path::new(r"\\?\C:\")),
        2,
        "Eject hears of them"
    );

    scene
        .queue
        .control(Action::Resume, Some(a))
        .expect("resumes");
    scene.wait_state(a, &TransferState::Done);
    assert_eq!(scene.log(), ["resume a at a.txt", "end a"]);
    assert_eq!(scene.ended(a).files, 3 + 3, "before and now");
    scene.wait_journal("ended 7");

    scene
        .queue
        .control(Action::Resume, Some(b))
        .expect("resumes");
    scene.wait_state(b, &TransferState::Done);
    assert_eq!(scene.log()[2], "run b", "it never began: a plain run");
    scene.wait_journal("ended 8");
    assert_eq!(
        scene.journal.lines(),
        ["started 7 3/10", "ended 7", "started 8 0/0", "ended 8"]
    );
}

#[test]
fn discarding_a_job_a_jerry_did_not_finish_ends_it_as_it_is() {
    let scene = scene(true);
    let dir = tempfile::tempdir().expect("temp folder");
    let part = crate::path::long_path(dir.path())
        .expect("long path")
        .join("a.txt.jerrypart");
    std::fs::write(&part, b"half").expect("write");
    let mut left = unfinished(7, "a", true);
    left.part = Some(part.clone());
    scene.queue.recover(vec![left, unfinished(8, "b", false)]);
    let ids: Vec<u32> = scene.queue.list().jobs.iter().map(|job| job.id).collect();
    scene
        .queue
        .control(Action::Pause, Some(ids[0]))
        .expect("does nothing");
    assert_eq!(scene.state(ids[0]), TransferState::Unfinished);
    scene
        .queue
        .control(Action::Cancel, Some(ids[0]))
        .expect("discards");
    assert_eq!(scene.state(ids[0]), TransferState::Cancelled);
    assert_eq!(scene.ended(ids[0]).files, 3, "what it did stays");
    assert!(!part.exists(), "its temporary file goes");
    assert_eq!(scene.journal.lines(), ["part gone 7", "ended 7"]);
    scene
        .queue
        .control(Action::Cancel, Some(ids[1]))
        .expect("discards");
    assert_eq!(scene.journal.lines(), ["part gone 7", "ended 7", "ended 8"]);
    assert!(scene.log().is_empty());
}

#[test]
fn the_part_a_jerry_left_goes_as_the_job_resumes() {
    let scene = scene(true);
    let dir = tempfile::tempdir().expect("temp folder");
    let part = crate::path::long_path(dir.path())
        .expect("long path")
        .join("a.txt.jerrypart");
    std::fs::write(&part, b"half").expect("write");
    let mut left = unfinished(7, "a", true);
    left.part = Some(part.clone());
    scene.queue.recover(vec![left]);
    let id = scene.queue.list().jobs[0].id;
    assert!(part.exists(), "recover itself touches no file");

    scene
        .queue
        .control(Action::Resume, Some(id))
        .expect("resumes");
    scene.wait_state(id, &TransferState::Done);
    assert!(!part.exists());
    scene.wait_journal("ended 7");
    assert_eq!(
        scene.journal.lines(),
        ["part gone 7", "started 7 3/10", "ended 7"]
    );
}

fn asks(state: &TransferState) -> bool {
    matches!(state, TransferState::Conflict { .. })
}

fn answer(choice: Choice) -> Answer {
    Answer { choice, all: false }
}

#[test]
fn a_job_that_asks_leaves_its_disk_to_the_next_one() {
    for parallel in [true, false] {
        let scene = scene(parallel);
        scene.script("a", Script::Ask);
        let a = scene.queue.enqueue(job(1, "a"));
        let b = scene.queue.enqueue(job(1, "b"));
        scene.wait_for("a to ask", |scene| asks(&scene.state(a)));
        scene.wait_state(b, &TransferState::Done);

        let TransferState::Conflict { conflict } = scene.state(a) else {
            panic!("a asks: {:?}", scene.state(a));
        };
        assert_eq!(conflict.choices, FILE_CHOICES);
        assert_eq!(conflict.target, r"C:\to\a.txt", "display text");
        assert_eq!(conflict.existing.size, 10);
        assert_eq!(conflict.existing.icon, 2, "the Shell's icon key");
        assert_eq!(
            conflict.existing.modified, None,
            "a date that can't be read"
        );
        assert!(!conflict.no_recycle_bin);
        // The files themselves, for the window's thumbnails.
        let existing = scene.queue.conflict_file(a, ConflictSide::Existing);
        assert_eq!(
            existing,
            Some(ConflictFile {
                path: PathBuf::from(r"\\?\C:\to\a.txt"),
                attributes: 0x20,
                modified: 0,
            })
        );
        let incoming = scene.queue.conflict_file(a, ConflictSide::Incoming);
        assert_eq!(
            incoming.map(|file| file.path),
            Some(PathBuf::from(r"\\?\C:\from\a.txt"))
        );
        assert_eq!(scene.queue.conflict_file(b, ConflictSide::Existing), None);

        scene
            .queue
            .resolve(a, answer(Choice::Skip))
            .expect("answered");
        assert_eq!(
            scene.queue.conflict_file(a, ConflictSide::Existing),
            None,
            "answered"
        );
        scene.wait_state(a, &TransferState::Done);
        assert!(scene.log().contains(&"answered a Skip".to_owned()));
    }
}

#[test]
fn an_answered_job_waits_for_its_disk_and_then_goes_first() {
    let scene = scene(true);
    scene.script("a", Script::Ask);
    let a = scene.queue.enqueue(job(1, "a"));
    let release = scene.hold("b");
    let b = scene.queue.enqueue(job(1, "b"));
    let c = scene.queue.enqueue(job(1, "c"));
    scene.wait_for("a to ask", |scene| asks(&scene.state(a)));
    scene.wait_state(b, &TransferState::Running);

    scene
        .queue
        .resolve(a, answer(Choice::KeepBoth))
        .expect("answered");
    assert_eq!(scene.state(a), TransferState::Queued, "b holds the disk");
    assert_eq!(scene.state(c), TransferState::Queued);
    std::thread::sleep(Duration::from_millis(20));
    assert!(!scene.log().iter().any(|line| line.starts_with("answered")));

    release.store(true, Ordering::SeqCst);
    scene.wait_state(a, &TransferState::Done);
    scene.wait_state(c, &TransferState::Done);
    let log = scene.log();
    let at = |line: &str| log.iter().position(|each| each == line).expect(line);
    assert!(at("end b") < at("answered a KeepBoth"));
    assert!(
        at("answered a KeepBoth") < at("run c"),
        "a goes before c: {log:?}"
    );
}

#[test]
fn only_a_job_that_asks_takes_an_answer_and_only_one_it_offers() {
    let scene = scene(true);
    scene.script("a", Script::Ask);
    let a = scene.queue.enqueue(job(1, "a"));
    let b = scene.queue.enqueue(job(2, "b"));
    scene.wait_for("a to ask", |scene| asks(&scene.state(a)));
    scene.wait_state(b, &TransferState::Done);
    let code = |result: Result<(), JerryError>| result.map_err(|error| error.code);

    assert_eq!(
        code(scene.queue.resolve(b, answer(Choice::Skip))),
        Err(ErrorCode::NotFound)
    );
    assert_eq!(
        code(scene.queue.resolve(99, answer(Choice::Skip))),
        Err(ErrorCode::NotFound)
    );
    assert_eq!(
        code(scene.queue.resolve(a, answer(Choice::Merge))),
        Err(ErrorCode::Internal),
        "a file can't merge"
    );
    assert!(asks(&scene.state(a)), "still asking");
    assert_eq!(
        code(scene.queue.resolve(a, answer(Choice::Replace))),
        Ok(())
    );
    assert_eq!(
        code(scene.queue.resolve(a, answer(Choice::Skip))),
        Err(ErrorCode::NotFound),
        "answered already, in another window"
    );
    scene.wait_state(a, &TransferState::Done);
}

#[test]
fn cancelling_a_job_that_asks_ends_it() {
    let scene = scene(true);
    scene.script("a", Script::Ask);
    let a = scene.queue.enqueue(job(1, "a"));
    scene.wait_for("a to ask", |scene| asks(&scene.state(a)));
    scene
        .queue
        .control(Action::Cancel, Some(a))
        .expect("cancels");
    scene.wait_state(a, &TransferState::Cancelled);
    assert_eq!(scene.ended(a).state, TransferState::Cancelled);
    assert!(!scene.log().iter().any(|line| line.starts_with("answered")));
    assert_eq!(scene.log(), ["run a", "end a"]);
}

#[test]
fn a_job_paused_while_it_asks_goes_on_at_resume() {
    let scene = scene(true);
    scene.script("a", Script::Ask);
    let a = scene.queue.enqueue(job(1, "a"));
    scene.wait_for("a to ask", |scene| asks(&scene.state(a)));
    scene.queue.control(Action::Pause, Some(a)).expect("pauses");
    assert!(asks(&scene.state(a)), "the question stays");

    scene
        .queue
        .resolve(a, answer(Choice::Skip))
        .expect("answered");
    assert_eq!(scene.state(a), TransferState::Paused);
    let b = scene.queue.enqueue(job(1, "b"));
    scene.wait_state(b, &TransferState::Done);
    assert_eq!(scene.state(a), TransferState::Paused, "it holds no disk");

    scene
        .queue
        .control(Action::Resume, Some(a))
        .expect("resumes");
    scene.wait_state(a, &TransferState::Done);
}
