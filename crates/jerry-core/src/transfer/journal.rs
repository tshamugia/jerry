//! The transfer journal (Tech PRD «Crash recovery», #50): every job written down in `jerry.db`
//! before it starts, so that one Jerry didn't finish (a crash, a kill, Windows shutting down) is
//! found at the next start, and the temporary file it was writing is removed.
//!
//! - A job is written when it is queued, and again when each turn of it starts.
//! - Each file it copies is written as its temporary file appears, before a byte of it is copied:
//!   the file being written, where it goes, its temporary file and how far the job has come. A
//!   job writes one file at a time, so this is the one file it can leave half written. The record
//!   is on its way to the disk before the copy goes on: SQLite in WAL mode with `synchronous =
//!   NORMAL` doesn't wait for the disk, and a process that dies loses nothing it committed.
//! - A job that ends is marked so, and its record goes after [`ENDED_KEPT_FOR`].
//!
//! At the next start ([`StorageJournal::recover`]) a job that isn't ended is offered again:
//! Resume carries on (the engine's [`resume`](super::resume)), Discard leaves what was done. Its
//! temporary file is removed either way, without asking: it is Jerry's own (data safety rule 7,
//! [`remove_part`]). Not before the answer, so that it can't be a resumed job's new one.
//!
//! The journal never stops a transfer: a record that can't be written is logged, and the job goes
//! on without it.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::Job;
use super::copy::{PART, patiently};
use super::queue::Queue;
use crate::delete::{probe, remove};
use crate::error::{ErrorCode, JerryError};
use crate::storage::Storage;
use crate::volume::missing_or_gone;

/// How long the record of a job that ended is kept.
pub const ENDED_KEPT_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// How far a job had come, as its record says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// A file a job begins to write.
#[derive(Debug, Clone, Copy)]
pub struct Writing<'a> {
    pub source: &'a Path,
    pub target: &'a Path,
    /// Its temporary file. A link has none.
    pub part: Option<&'a Path>,
}

/// A job the journal has that the Jerry running it didn't finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unfinished {
    /// Its record in the journal.
    pub record: i64,
    pub job: Job,
    /// A turn of it had started, so some of it may be done; else it never began.
    pub started: bool,
    pub counts: Counts,
    /// The last file it began to write: `(source, target)`.
    pub writing: Option<(PathBuf, PathBuf)>,
    /// That file's temporary file, while it may still be there.
    pub part: Option<PathBuf>,
}

/// Where the transfer queue writes its jobs down. Every call is blocking IO: the queue makes them
/// on background threads, outside its lock.
pub trait Journal: Send + Sync + 'static {
    /// A job was queued: its record, or `None` if it couldn't be written.
    fn queued(&self, job: &Job) -> Option<i64>;

    /// A turn of the job starts, from `counts`.
    fn started(&self, record: i64, counts: &Counts);

    /// The job begins to write a file. Returns once the record is written.
    fn writing(&self, record: i64, writing: &Writing<'_>, counts: &Counts);

    /// The temporary file the record names isn't there any more.
    fn part_gone(&self, record: i64);

    /// The job ended: done, cancelled, discarded or failed.
    fn ended(&self, record: i64);
}

/// No journal: nothing is written down.
impl Journal for () {
    fn queued(&self, _: &Job) -> Option<i64> {
        None
    }

    fn started(&self, _: i64, _: &Counts) {}

    fn writing(&self, _: i64, _: &Writing<'_>, _: &Counts) {}

    fn part_gone(&self, _: i64) {}

    fn ended(&self, _: i64) {}
}

/// Where a [`StorageJournal`] finds the database: the app's, which opens on its own thread, or a
/// test's.
pub trait Store: Send + Sync + 'static {
    /// Runs `job` with the database, waiting for it to open.
    fn with_storage(
        &self,
        job: &mut dyn FnMut(&mut Storage) -> Result<(), JerryError>,
    ) -> Result<(), JerryError>;
}

impl Store for Mutex<Storage> {
    fn with_storage(
        &self,
        job: &mut dyn FnMut(&mut Storage) -> Result<(), JerryError>,
    ) -> Result<(), JerryError> {
        job(&mut self.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// The journal in `jerry.db` (the `transfer_journal` table).
pub struct StorageJournal<S> {
    store: S,
    /// This Jerry: its start time, in ms since the Unix epoch.
    run: i64,
}

impl<S: Store> StorageJournal<S> {
    /// The journal of the Jerry started at `run` (ms since the Unix epoch), in `store`.
    pub fn new(store: S, run: i64) -> Self {
        Self { store, run }
    }

    /// The database it writes to.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// What a Jerry does with the journal as it starts, on a background thread: lists the jobs
    /// a Jerry before it didn't finish in `queue`, for Resume or Discard (their temporary files
    /// go with that answer); removes the temporary files that jobs which ended left (data safety
    /// rule 7), and forgets the jobs that ended over [`ENDED_KEPT_FOR`] ago. Returns how many
    /// jobs it listed.
    pub fn recover(&self, queue: &Queue) -> usize {
        let mut unfinished = Vec::new();
        self.write("unfinished", |storage| {
            unfinished = storage.journal_unfinished(self.run)?;
            Ok(())
        });
        let listed = unfinished.len();
        if listed > 0 {
            tracing::info!(listed, "jobs a Jerry before this one didn't finish");
        }
        queue.recover(unfinished);
        let mut parts = Vec::new();
        self.write("parts", |storage| {
            parts = storage.journal_parts(self.run)?;
            Ok(())
        });
        for (record, part) in parts {
            match remove_part(&part) {
                Ok(()) => self.part_gone(record),
                Err(error) => {
                    tracing::info!(code = ?error.code, "a temporary file a job left stays for now");
                }
            }
        }
        let before = self.run.saturating_sub(ENDED_KEPT_FOR.as_millis() as i64);
        self.write("forget", |storage| storage.journal_forget(before).map(drop));
        listed
    }

    fn write(&self, what: &str, mut job: impl FnMut(&mut Storage) -> Result<(), JerryError>) {
        if let Err(error) = self.store.with_storage(&mut job) {
            tracing::warn!(code = ?error.code, what, "the transfer journal failed");
        }
    }
}

impl<S: Store> Journal for StorageJournal<S> {
    fn queued(&self, job: &Job) -> Option<i64> {
        let mut record = None;
        self.write("queued", |storage| {
            record = Some(storage.journal_queue(self.run, job, now_ms())?);
            Ok(())
        });
        record
    }

    fn started(&self, record: i64, counts: &Counts) {
        self.write("started", |storage| {
            storage.journal_start(record, counts, now_ms())
        });
    }

    fn writing(&self, record: i64, writing: &Writing<'_>, counts: &Counts) {
        self.write("writing", |storage| {
            storage.journal_write(record, writing, counts, now_ms())
        });
    }

    fn part_gone(&self, record: i64) {
        self.write("part gone", |storage| storage.journal_part_gone(record));
    }

    fn ended(&self, record: i64) {
        self.write("ended", |storage| storage.journal_end(record, now_ms()));
    }
}

/// Now, in ms since the Unix epoch: how the tables keep times.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64)
}

/// Removes a temporary file a job left, as the journal names it: the one file Jerry removes
/// without asking (data safety rule 7). Only a file whose name ends with `.jerrypart` goes, never
/// a folder or a link; one that is gone already is fine. Fails with [`ErrorCode::DeviceGone`]
/// while its volume is away, so that it is tried again later.
pub fn remove_part(part: &Path) -> Result<(), JerryError> {
    let named = part
        .file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(PART));
    if !named {
        tracing::error!(path = %part.display(), "the journal names a part that isn't one");
        return Err(JerryError::new(ErrorCode::InvalidName).at(part));
    }
    match probe(part) {
        Ok(item) if !item.is_dir() && !item.link => patiently(|| remove(&item)),
        Ok(_) => {
            tracing::warn!("a folder or a link has a part's name; leaving it");
            Err(JerryError::new(ErrorCode::InvalidName).at(part))
        }
        Err(error) => match missing_or_gone(error, part) {
            error if error.code == ErrorCode::NotFound => Ok(()),
            error => Err(error),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use super::*;
    use crate::path::long_path;
    use crate::transfer::Kind;
    use crate::transfer::queue::{NoShell, TransferState};

    #[test]
    fn a_jerry_starting_lists_what_the_one_before_left_and_tidies_up() {
        const BEFORE: i64 = 1_000;
        const NOW: i64 = BEFORE + ENDED_KEPT_FOR.as_millis() as i64 + 5;
        let dir = tempfile::tempdir().unwrap();
        let root = long_path(dir.path()).unwrap();
        let job = Job {
            kind: Kind::Copy,
            sources: vec![root.join("a.txt")],
            destination: root.clone(),
        };
        let part = |name: &str| {
            let part = root.join(name);
            fs::write(&part, b"half").unwrap();
            part
        };
        let (unfinished_part, ended_part) = (part("u.txt.jerrypart"), part("e.txt.jerrypart"));
        fn writing(part: &Path) -> Writing<'_> {
            Writing {
                source: Path::new(r"\\?\C:\x.txt"),
                target: Path::new(r"\\?\C:\y.txt"),
                part: Some(part),
            }
        }
        let mut storage = Storage::in_memory().unwrap();
        let unfinished = storage.journal_queue(BEFORE, &job, NOW).unwrap();
        storage
            .journal_write(
                unfinished,
                &writing(&unfinished_part),
                &Counts::default(),
                NOW,
            )
            .unwrap();
        let ended = storage.journal_queue(BEFORE, &job, NOW).unwrap();
        storage
            .journal_write(ended, &writing(&ended_part), &Counts::default(), NOW)
            .unwrap();
        storage.journal_end(ended, NOW).unwrap();
        let old = storage.journal_queue(BEFORE, &job, BEFORE).unwrap();
        storage.journal_end(old, BEFORE).unwrap();
        let own = storage.journal_queue(NOW, &job, NOW).unwrap();

        let journal = StorageJournal::new(Mutex::new(storage), NOW);
        let queue = Queue::start(true, Arc::new(()), Arc::new(NoShell), |_| {});
        assert_eq!(journal.recover(&queue), 1);
        let jobs = queue.list().jobs;
        assert_eq!(jobs.len(), 1, "not this Jerry's own job");
        assert_eq!(jobs[0].state, TransferState::Unfinished);
        assert!(unfinished_part.exists(), "it waits for Resume or Discard");
        assert!(!ended_part.exists(), "a job that ended left it");

        let mut storage = journal.store().lock().unwrap();
        assert_eq!(storage.journal_parts(NOW + 1).unwrap(), vec![]);
        let records: Vec<i64> = storage
            .journal_unfinished(NOW + 1)
            .unwrap()
            .iter()
            .map(|left| left.record)
            .collect();
        assert_eq!(records, [unfinished, own]);
        // The old one is forgotten, the one that ended now is kept a week.
        assert_eq!(
            storage.journal_forget(NOW + 1).unwrap(),
            1,
            "only `ended` was left to forget"
        );
    }

    #[test]
    fn a_part_is_removed_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let root = long_path(dir.path()).unwrap();
        let part = root.join("ნახაზი.dwg.jerrypart");
        fs::write(&part, b"half").unwrap();
        let mut permissions = fs::metadata(&part).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&part, permissions).unwrap();
        assert_eq!(remove_part(&part), Ok(()));
        assert!(!part.exists());
        assert_eq!(remove_part(&part), Ok(()), "gone already");

        let file = root.join("ნახაზი.dwg");
        fs::write(&file, b"whole").unwrap();
        assert_eq!(
            remove_part(&file).map_err(|error| error.code),
            Err(ErrorCode::InvalidName)
        );
        assert!(file.exists());

        let folder = root.join("x.jerrypart");
        fs::create_dir(&folder).unwrap();
        assert_eq!(
            remove_part(&folder).map_err(|error| error.code),
            Err(ErrorCode::InvalidName)
        );
        assert!(folder.exists());
    }

    #[test]
    fn a_part_in_a_folder_that_is_gone_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let part = long_path(dir.path())
            .unwrap()
            .join("gone")
            .join("a.txt.jerrypart");
        assert_eq!(remove_part(&part), Ok(()));
    }
}
