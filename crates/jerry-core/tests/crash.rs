//! Crash recovery (#50, Tech PRD «Crash recovery»): a Jerry killed in the middle of a copy or a
//! move, and the next one finding the job in the journal, removing the half-written file and
//! finishing the job. The Jerry that is killed is this test binary, run again as a child process
//! that copies with the real queue, engine and journal and holds still once it is writing the big
//! file; the test kills it there (`TerminateProcess`).

#![allow(
    clippy::expect_used,
    clippy::print_stdout,
    reason = "test helpers: a failed setup fails the test; the child tells the test it is ready"
)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use jerry_core::path::long_path;
use jerry_core::storage::Storage;
use jerry_core::transfer::journal::{Counts, Journal, StorageJournal, Writing};
use jerry_core::transfer::queue::{Action, Queue, TransferState};
use jerry_core::transfer::{Job, Kind};
use jerry_core::volume;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN;

/// Set in the child: what it is to do.
const CHILD: &str = "JERRY_CRASH_CHILD";
const DB: &str = "JERRY_CRASH_DB";
const FROM: &str = "JERRY_CRASH_FROM";
const TO: &str = "JERRY_CRASH_TO";

/// What the child prints once it is writing the big file.
const READY: &str = "JERRY-CRASH-READY";

/// The Jerry that is killed, and the one after it.
const KILLED_RUN: i64 = 1;
const NEXT_RUN: i64 = 2;

const BIG: &str = "ნახაზი.bin";
const BIG_SIZE: usize = 8 * 1024 * 1024;

#[test]
fn a_copy_killed_halfway_is_finished_by_the_next_jerry() {
    if std::env::var_os(CHILD).is_some() {
        child(Kind::Copy);
    }
    let from = temp("jerry-crash-from-");
    let to = temp("jerry-crash-to-");
    killed_and_finished(Kind::Copy, &from, &to);
}

#[test]
fn a_move_killed_halfway_is_finished_by_the_next_jerry() {
    if std::env::var_os(CHILD).is_some() {
        child(Kind::Move);
    }
    // A move within a volume renames and writes nothing: the destination is on another one.
    let from = temp("jerry-crash-from-");
    let to = tempfile::Builder::new()
        .prefix("jerry-crash-to-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .expect("temp folder");
    let (from_root, to_root) = (
        volume::root(&long(from.path())).expect("a volume"),
        volume::root(&long(to.path())).expect("a volume"),
    );
    if volume::same_root(&from_root, &to_root) {
        eprintln!("skipped: the temp folder and the target folder are on one volume");
        return;
    }
    killed_and_finished(Kind::Move, &from, &to);
}

fn temp(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("temp folder")
}

fn long(path: &Path) -> PathBuf {
    long_path(path).expect("long path")
}

/// The three sources: a small file the job finishes, the big one it is killed in, and one it
/// never gets to.
fn sources(from: &Path) -> Vec<PathBuf> {
    vec![from.join("a.txt"), from.join(BIG), from.join("c.txt")]
}

fn killed_and_finished(kind: Kind, from: &tempfile::TempDir, to: &tempfile::TempDir) {
    let (from, to) = (long(from.path()), long(to.path()));
    let db_dir = temp("jerry-crash-db-");
    let db = db_dir.path().join("jerry.db");
    fs::write(from.join("a.txt"), b"a").expect("write");
    fs::write(from.join(BIG), vec![b'x'; BIG_SIZE]).expect("write");
    fs::write(from.join("c.txt"), b"cc").expect("write");

    kill_while_writing(kind, &db, &from, &to);

    // What the killed Jerry left: a.txt done, the big file half written, its record.
    let target = to.join(BIG);
    let part = to.join(format!("{BIG}.jerrypart"));
    let attributes = fs::metadata(&part)
        .expect("the half-written file is there")
        .file_attributes();
    assert_ne!(attributes & FILE_ATTRIBUTE_HIDDEN.0, 0, "hidden");
    assert!(!target.exists(), "no final name for half a file");
    assert!(to.join("a.txt").exists());
    let storage = Storage::open(&db).expect("opens after the kill");
    let unfinished = storage
        .journal_unfinished(NEXT_RUN)
        .expect("reads the journal");
    assert_eq!(unfinished.len(), 1, "{unfinished:?}");
    let job = &unfinished[0];
    assert!(job.started);
    assert_eq!(job.job.kind, kind);
    assert_eq!(job.job.sources, sources(&from));
    assert_eq!(job.writing, Some((from.join(BIG), target.clone())));
    assert_eq!(job.part.as_deref(), Some(part.as_path()));
    assert_eq!((job.counts.files_done, job.counts.files_total), (1, 3));

    // What the next Jerry does as it starts: the job is listed for Resume or Discard.
    let journal = Arc::new(StorageJournal::new(Mutex::new(storage), NEXT_RUN));
    let queue = Queue::start(true, journal.clone(), |_| {});
    assert_eq!(journal.recover(&queue), 1);
    let id = queue.list().jobs[0].id;
    assert_eq!(queue.list().jobs[0].state, TransferState::Unfinished);
    let progress = &queue.list().jobs[0].progress;
    assert_eq!((progress.files_done, progress.files_total), (1, 3));

    // Resume removes the half-written file (data safety rule 7) and finishes the job.
    queue.control(Action::Resume, Some(id)).expect("resumes");
    let start = Instant::now();
    while queue.list().jobs[0].state != TransferState::Done {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "{:?}",
            queue.list()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let progress = &queue.list().jobs[0].progress;
    assert_eq!((progress.files_done, progress.files_total), (3, 3));
    assert_eq!(progress.failed, 0);

    assert_eq!(names(&to), ["a.txt", "c.txt", BIG]);
    assert_eq!(fs::metadata(&target).expect("there").len(), BIG_SIZE as u64);
    match kind {
        Kind::Copy => assert_eq!(names(&from), ["a.txt", "c.txt", BIG], "a copy leaves them"),
        Kind::Move => assert_eq!(names(&from), Vec::<String>::new(), "moved"),
    }
    // The job's record ends too, just after the queue says it is done.
    let start = Instant::now();
    loop {
        let left = journal
            .store()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .journal_unfinished(NEXT_RUN + 1)
            .expect("reads");
        if left.is_empty() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "{left:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Runs the job in a child process and kills it once it is writing the big file.
fn kill_while_writing(kind: Kind, db: &Path, from: &Path, to: &Path) {
    let test = match kind {
        Kind::Copy => "a_copy_killed_halfway_is_finished_by_the_next_jerry",
        Kind::Move => "a_move_killed_halfway_is_finished_by_the_next_jerry",
    };
    let mut child = Command::new(std::env::current_exe().expect("this test"))
        .args(["--exact", test, "--nocapture"])
        .env(CHILD, "1")
        .env(DB, db)
        .env(FROM, from)
        .env(TO, to)
        .stdout(Stdio::piped())
        .spawn()
        .expect("starts the child");
    let stdout = child.stdout.take().expect("its output");
    let (lines, ready) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            if line.trim() == READY {
                let _ = lines.send(());
                return;
            }
        }
    });
    let reached = ready.recv_timeout(Duration::from_secs(60));
    child.kill().expect("kills it");
    child.wait().expect("it ends");
    reached.expect("the child got to the big file");
}

/// The killed Jerry: copies or moves the three sources with the real queue and a journal in
/// `DB`, and holds still once the big file's temporary file is there and written down.
fn child(kind: Kind) -> ! {
    let var = |name: &str| -> PathBuf { std::env::var_os(name).map(PathBuf::from).expect(name) };
    let (db, from, to) = (var(DB), var(FROM), var(TO));
    let storage = Storage::open(&db).expect("opens");
    let (reached, ready) = mpsc::channel();
    let journal = Arc::new(Holding {
        journal: StorageJournal::new(Mutex::new(storage), KILLED_RUN),
        reached: Mutex::new(Some(reached)),
    });
    let queue = Queue::start(true, journal, |_| {});
    queue.enqueue(Job {
        kind,
        sources: sources(&from),
        destination: to,
    });
    ready.recv().expect("reaches the big file");
    let mut out = std::io::stdout();
    writeln!(out, "{READY}").expect("prints");
    out.flush().expect("flushes");
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

/// The child's journal: the real one, and the job holds still once it writes the big file down.
struct Holding {
    journal: StorageJournal<Mutex<Storage>>,
    reached: Mutex<Option<mpsc::Sender<()>>>,
}

impl Journal for Holding {
    fn queued(&self, job: &Job) -> Option<i64> {
        self.journal.queued(job)
    }

    fn started(&self, record: i64, counts: &Counts) {
        self.journal.started(record, counts);
    }

    fn writing(&self, record: i64, writing: &Writing<'_>, counts: &Counts) {
        self.journal.writing(record, writing, counts);
        if writing.source.ends_with(BIG) {
            let reached = self
                .reached
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(reached) = reached {
                let _ = reached.send(());
            }
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        }
    }

    fn part_gone(&self, record: i64) {
        self.journal.part_gone(record);
    }

    fn ended(&self, record: i64) {
        self.journal.ended(record);
    }
}

/// The names in `folder`, hidden ones too, sorted.
fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("reads")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}
