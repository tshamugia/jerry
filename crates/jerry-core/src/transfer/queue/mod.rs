//! The transfer queue (Tech PRD «ტრანსფერების ძრავი», #49): which job runs when, and what the UI
//! hears about them.
//!
//! - **Lanes.** A job belongs to the disk or server it writes to ([`volume::lane`]): reading from a
//!   disk doesn't hold it, so C: to D: and D: to a server run together. A lane runs one job at a
//!   time, the first one queued that isn't paused; different lanes run in parallel, or one at a
//!   time with the setting off ([`Queue::set_parallel`]). Each running job has a thread of its own,
//!   so there is at most one per lane and none when nothing runs.
//! - **Pausing.** A running job that is paused keeps its lane and its current file: starting
//!   another job there would make two run on one disk at Resume. A paused job that hasn't started
//!   holds nothing, and the next one goes.
//! - **A volume going away.** A job the engine stops ([`Status::Interrupted`]) leaves its thread
//!   and its lane, so that one unplugged USB source doesn't hold up everything else going to C:.
//!   It waits with its [`Summary`] until its volumes answer again, then Resume carries on from
//!   there (the engine's [`retry`](super::retry)) and Cancel ends it. A full disk has nothing to
//!   wait for, and Resume can be tried at once.
//! - **Conflicts** (#51). A job whose name is taken waits for the user's answer
//!   ([`TransferState::Conflict`], [`Queue::resolve`]) on its thread, but leaves its lane: the
//!   next job on that disk starts, so one question doesn't hold up the queue. Once answered, it
//!   carries on as soon as its lane is free again, before the jobs queued after it.
//! - **Progress.** The engine reports every chunk; the UI hears at most [`PROGRESS_EVERY`], one
//!   event for all running jobs.
//! - **The journal.** Every job is written down as it is queued, as each turn starts and as each
//!   file begins ([`journal`](super::journal)), always off the queue's lock. A job that a Jerry
//!   didn't finish comes back with [`Queue::recover`], [`TransferState::Unfinished`]: Resume
//!   carries it on from where it stopped, Cancel discards it. The temporary file a job leaves (a
//!   volume went away as it was writing, a Jerry stopped) is removed as its next turn starts, or
//!   as the turn ends if it can be.

mod view;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::Deserialize;
use specta::Type;
pub use view::{
    ConflictItem, TransferConflict, TransferEnded, TransferFailure, TransferJob, TransferList,
    TransferProgress, TransferState,
};

use super::conflict::Side;
use super::journal::{Counts, Journal, Unfinished, remove_part};
use super::{Answer, Bin, Conflict, Control, Event, Failure, Job, Phase, Resume, Status, Summary};
use crate::delete::Tally;
use crate::drives::quiet_errors;
use crate::error::{ErrorCode, JerryError, display_path};
use crate::job::run_job;
use crate::listing::filetime_to_unix_ms;
use crate::volume::{self, Lane, same_root};

/// How often the UI hears about progress at most: 10 times a second (CLAUDE.md rule 3).
pub const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// How often a waiting job looks whether its volumes are back.
const WATCH_EVERY: Duration = Duration::from_secs(2);

/// How many jobs that ended stay listed, for their summary and Retry.
const ENDED_KEPT: usize = 50;

/// What the UI can ask of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[serde(rename = "TransferAction")]
pub enum Action {
    Pause,
    Resume,
    Cancel,
}

/// What the queue reports, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// The list changed: a job came, went, was reordered or changed state.
    Jobs(TransferList),
    /// How far the running jobs have come, at most every [`PROGRESS_EVERY`].
    Progress(Vec<TransferProgress>),
    /// A job ended.
    Ended(Box<TransferEnded>),
}

/// What the queue needs from Windows' Shell, which core doesn't call (CLAUDE.md): `jerry_win`
/// has it, and the app hands it over. The Recycle Bin a replaced file goes to, and icons.
pub trait Shell: Bin + 'static {
    /// The key of the type icon (`icon://type-<key>`) of an item named `name` with
    /// `attributes`, for the conflict window.
    fn icon(&self, name: &OsStr, attributes: u32) -> u32;
}

/// No Shell, for tests: no Recycle Bin takes anything, and every icon is a plain file's.
#[cfg(test)]
pub(crate) struct NoShell;

#[cfg(test)]
impl Bin for NoShell {
    fn recycle(&self, path: &Path) -> Result<(), JerryError> {
        Err(JerryError::new(ErrorCode::NoRecycleBin).at(path))
    }
}

#[cfg(test)]
impl Shell for NoShell {
    fn icon(&self, _: &OsStr, _: u32) -> u32 {
        2
    }
}

/// What does the work: [`System`], or a fake in tests.
trait Engine: Send + Sync + 'static {
    fn run(
        &self,
        job: &Job,
        control: &Control,
        observer: &mut dyn FnMut(Event<'_>),
    ) -> Result<Summary, JerryError>;

    fn retry(
        &self,
        summary: &Summary,
        control: &Control,
        observer: &mut dyn FnMut(Event<'_>),
    ) -> Result<Summary, JerryError>;

    fn resume(
        &self,
        job: &Job,
        resume: &Resume,
        control: &Control,
        observer: &mut dyn FnMut(Event<'_>),
    ) -> Result<Summary, JerryError>;

    fn lane(&self, destination: &Path) -> Lane;

    /// The root of the volume `path` is on, if it has one now.
    fn root(&self, path: &Path) -> Option<PathBuf>;

    /// Whether `path` is there. Blocks for as long as a server takes to answer.
    fn reachable(&self, path: &Path) -> bool;
}

/// The real thing.
struct System {
    shell: Arc<dyn Shell>,
}

impl Engine for System {
    fn run(
        &self,
        job: &Job,
        control: &Control,
        observer: &mut dyn FnMut(Event<'_>),
    ) -> Result<Summary, JerryError> {
        super::run(job, control, &*self.shell, observer)
    }

    fn retry(
        &self,
        summary: &Summary,
        control: &Control,
        observer: &mut dyn FnMut(Event<'_>),
    ) -> Result<Summary, JerryError> {
        super::retry(summary, control, &*self.shell, observer)
    }

    fn resume(
        &self,
        job: &Job,
        resume: &Resume,
        control: &Control,
        observer: &mut dyn FnMut(Event<'_>),
    ) -> Result<Summary, JerryError> {
        super::resume(job, resume, control, &*self.shell, observer)
    }

    fn lane(&self, destination: &Path) -> Lane {
        volume::lane(destination)
    }

    fn root(&self, path: &Path) -> Option<PathBuf> {
        volume::root(path).ok()
    }

    fn reachable(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok()
    }
}

/// The transfer queue. Clones share it.
#[derive(Clone)]
pub struct Queue(Arc<Shared>);

struct Shared {
    engine: Box<dyn Engine>,
    journal: Arc<dyn Journal>,
    shell: Arc<dyn Shell>,
    notify: Box<dyn Fn(Notice) + Send + Sync>,
    timing: Timing,
    state: Mutex<State>,
}

#[derive(Clone, Copy)]
struct Timing {
    progress: Duration,
    watch: Duration,
}

struct State {
    jobs: Vec<Entry>,
    next_id: u32,
    generation: u32,
    parallel: bool,
    /// When progress was last reported.
    reported: Option<Instant>,
}

/// One job, for as long as it is listed.
struct Entry {
    id: u32,
    job: Arc<Job>,
    lane: Lane,
    /// The volumes it touches, sources and destination.
    roots: Vec<PathBuf>,
    source: String,
    destination: String,
    control: Arc<Control>,
    stage: Stage,
    /// The user paused it. It says what to show and, before the job has started, keeps it from
    /// starting.
    paused: bool,
    next: Next,
    meter: Meter,
    /// Its record in the journal, if it could be written.
    record: Option<i64>,
    /// The temporary file of the last file it began, while that may still be there.
    part: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
enum Stage {
    Queued,
    /// On its thread, paused or not.
    Active,
    /// On its thread, which waits for the user to answer a conflict. It holds no lane.
    Asking(Box<Asked>),
    /// Answered: its thread carries on with this once its lane is free.
    Answered(Answer),
    Waiting {
        error: JerryError,
        ready: bool,
    },
    /// A Jerry before this one didn't finish it: it waits for Resume or Cancel.
    Unfinished,
    Ended(Ended),
}

/// The conflict a job waits on: what the window shows, and the files on its two sides.
#[derive(Debug, Clone, PartialEq)]
struct Asked {
    view: TransferConflict,
    incoming: ConflictFile,
    existing: ConflictFile,
}

/// One side of the conflict a job waits on, as it was on disk when the job asked. The conflict
/// window's thumbnails (#67) refer to it by job and side, never by a path from the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictFile {
    pub path: PathBuf,
    pub attributes: u32,
    /// When it was last modified (`FILETIME`), 0 if that can't be read.
    pub modified: u64,
}

/// Which side of a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictSide {
    /// What comes.
    Incoming,
    /// What is already there.
    Existing,
}

#[derive(Debug, Clone, PartialEq)]
enum Ended {
    Done,
    Cancelled,
    Failed(JerryError),
}

/// What a job does when its turn comes.
enum Next {
    Run,
    Retry(Box<Summary>),
    /// Carries on a job that a Jerry before this one had started.
    Resume(Box<Resume>),
    /// Nothing is left to do, or to try again.
    Nothing,
}

/// A job's running totals.
struct Meter {
    phase: Phase,
    file: Option<PathBuf>,
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    bytes_total: u64,
    speed: u64,
    failed: u32,
    /// What the job's earlier turns finished.
    finished: Tally,
    /// Files its turns skipped because their name was taken.
    skipped: u64,
}

impl Meter {
    fn new() -> Self {
        Self {
            phase: Phase::Counting,
            file: None,
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            speed: 0,
            failed: 0,
            finished: Tally::default(),
            skipped: 0,
        }
    }

    /// A job a Jerry before this one didn't finish, as far as its record says it came.
    fn recovered(unfinished: &Unfinished) -> Self {
        let counts = unfinished.counts;
        Self {
            phase: if unfinished.started {
                Phase::Working
            } else {
                Phase::Counting
            },
            file: unfinished
                .writing
                .as_ref()
                .map(|(source, _)| source.clone()),
            files_done: counts.files_done,
            files_total: counts.files_total,
            bytes_done: counts.bytes_done,
            bytes_total: counts.bytes_total,
            speed: 0,
            failed: 0,
            finished: Tally {
                files: counts.files_done,
                folders: 0,
                bytes: counts.bytes_done,
            },
            skipped: 0,
        }
    }

    /// What the journal keeps of it.
    fn counts(&self) -> Counts {
        Counts {
            files_done: self.files_done,
            files_total: self.files_total,
            bytes_done: self.bytes_done,
            bytes_total: self.bytes_total,
        }
    }

    /// A turn starts where the earlier ones stopped. What failed is about to be tried again.
    fn begin_turn(&mut self) {
        self.files_done = self.finished.files;
        self.bytes_done = self.finished.bytes;
        self.speed = 0;
        self.failed = 0;
    }

    fn take(&mut self, event: Event<'_>) {
        match event {
            Event::Phase(phase) => self.phase = phase,
            Event::Counted(tally) => {
                self.files_total = self.finished.files + tally.files;
                self.bytes_total = self.finished.bytes + tally.bytes;
            }
            Event::Progress(progress) => {
                self.files_done = self.finished.files + progress.files_done;
                self.files_total = self.finished.files + progress.files_total;
                self.bytes_done = self.finished.bytes + progress.bytes_done;
                self.bytes_total = self.finished.bytes + progress.bytes_total;
                self.speed = progress.speed;
                // Most reports are about the file of the last one.
                if self.file.as_deref().map(Path::as_os_str) != Some(progress.file.as_os_str()) {
                    self.file = Some(progress.file.to_owned());
                }
            }
            Event::Failed(_) => self.failed += 1,
            // The queue writes it down; the file shows with the progress that follows.
            Event::Writing(_) => {}
            // The queue asks the user.
            Event::Conflict(_) => self.speed = 0,
        }
    }

    fn take_summary(&mut self, summary: &Summary) {
        self.finished.files += summary.files;
        self.finished.folders += summary.folders;
        self.finished.bytes += summary.bytes;
        self.skipped += summary.skipped;
        self.failed = summary.failed.len() as u32;
        self.speed = 0;
    }
}

impl Entry {
    fn ended(&self) -> bool {
        matches!(self.stage, Stage::Ended(_))
    }

    fn state(&self) -> TransferState {
        match &self.stage {
            Stage::Asking(asked) => TransferState::Conflict {
                conflict: asked.view.clone(),
            },
            Stage::Queued | Stage::Active | Stage::Answered(_) if self.paused => {
                TransferState::Paused
            }
            Stage::Queued | Stage::Answered(_) => TransferState::Queued,
            Stage::Active => TransferState::Running,
            Stage::Waiting { error, ready } => TransferState::Waiting {
                error: error.clone(),
                ready: *ready,
            },
            Stage::Unfinished => TransferState::Unfinished,
            Stage::Ended(Ended::Done) => TransferState::Done,
            Stage::Ended(Ended::Cancelled) => TransferState::Cancelled,
            Stage::Ended(Ended::Failed(error)) => TransferState::Failed {
                error: error.clone(),
            },
        }
    }

    fn progress(&self) -> TransferProgress {
        let meter = &self.meter;
        TransferProgress {
            id: self.id,
            phase: meter.phase,
            file: meter.file.as_deref().map(display_path),
            files_done: meter.files_done,
            files_total: meter.files_total,
            bytes_done: meter.bytes_done,
            bytes_total: meter.bytes_total,
            speed: meter.speed,
            failed: meter.failed,
        }
    }

    fn view(&self) -> TransferJob {
        TransferJob {
            id: self.id,
            kind: self.job.kind,
            state: self.state(),
            source: self.source.clone(),
            sources: self.job.sources.len() as u32,
            destination: self.destination.clone(),
            progress: self.progress(),
        }
    }

    /// What to tell the UI as this job ends.
    fn ending(&self, failed: &[Failure]) -> Notice {
        let finished = self.meter.finished;
        Notice::Ended(Box::new(TransferEnded {
            id: self.id,
            state: self.state(),
            files: finished.files,
            folders: finished.folders,
            bytes: finished.bytes,
            skipped: self.meter.skipped,
            failed: failed
                .iter()
                .map(|failure| TransferFailure {
                    source: display_path(&failure.source),
                    target: display_path(&failure.target),
                    error: failure.error.clone(),
                    copied: failure.copied,
                })
                .collect(),
        }))
    }

    /// Does `action`, as far as it applies here. Returns true if that ended the job.
    fn apply(&mut self, action: Action) -> bool {
        match (action, &self.stage) {
            (Action::Pause, Stage::Queued) => self.paused = true,
            (Action::Pause, Stage::Active) => {
                self.control.pause();
                self.paused = true;
            }
            (Action::Resume, Stage::Queued) => self.paused = false,
            (Action::Resume, Stage::Active) => {
                self.control.resume();
                self.paused = false;
            }
            (Action::Resume, Stage::Waiting { ready: true, .. } | Stage::Unfinished) => {
                self.queue_again();
            }
            (Action::Cancel, Stage::Queued | Stage::Waiting { .. } | Stage::Unfinished) => {
                self.stage = Stage::Ended(Ended::Cancelled);
                return true;
            }
            (Action::Cancel, Stage::Active) => {
                self.control.cancel();
                self.paused = false;
            }
            // Its thread waits for the answer: a pause holds it once it goes on.
            (Action::Pause, Stage::Asking(_) | Stage::Answered(_)) => {
                self.control.pause();
                self.paused = true;
            }
            (Action::Resume, Stage::Asking(_) | Stage::Answered(_)) => {
                self.control.resume();
                self.paused = false;
            }
            // Its thread stops waiting and ends the job, tidying up on its disk as it goes.
            (Action::Cancel, Stage::Asking(_) | Stage::Answered(_)) => {
                self.control.cancel();
                self.paused = false;
                self.stage = Stage::Active;
            }
            _ => {}
        }
        false
    }

    /// Back in the queue, for what `next` says. A control that was paused or cancelled is done.
    fn queue_again(&mut self) {
        self.stage = Stage::Queued;
        self.paused = false;
        self.control = Arc::new(Control::new());
    }

    /// Stops the job until its volumes are back. Returns true if that has to be watched for: a
    /// full disk has nothing to wait for.
    fn wait(&mut self, error: JerryError, next: Next) -> bool {
        let ready = error.code != ErrorCode::DeviceGone;
        self.stage = Stage::Waiting { error, ready };
        self.next = next;
        !ready
    }

    /// The paths that must answer before a waiting job can go on.
    fn locations(&self) -> Vec<PathBuf> {
        let mut paths = self.roots.clone();
        paths.push(self.job.destination.clone());
        paths
    }
}

impl State {
    fn find(&mut self, id: u32) -> Option<&mut Entry> {
        self.jobs.iter_mut().find(|entry| entry.id == id)
    }

    fn list(&self) -> TransferList {
        TransferList {
            generation: self.generation,
            jobs: self.jobs.iter().map(Entry::view).collect(),
        }
    }

    /// Forgets the oldest of the jobs that ended, past [`ENDED_KEPT`].
    fn evict(&mut self) {
        let mut excess = self
            .jobs
            .iter()
            .filter(|entry| entry.ended())
            .count()
            .saturating_sub(ENDED_KEPT);
        self.jobs.retain(|entry| {
            let goes = excess > 0 && entry.ended();
            excess -= usize::from(goes);
            !goes
        });
    }
}

/// A job handed to its thread.
struct Turn {
    id: u32,
    job: Arc<Job>,
    control: Arc<Control>,
    next: Next,
    record: Option<i64>,
    /// Where the turn starts.
    counts: Counts,
}

impl Queue {
    /// A queue with nothing in it that writes its jobs down in `journal` and asks `shell` for the
    /// Recycle Bin and icons. `notify` hears everything that happens, in order, under the
    /// queue's lock: it must be quick and must not call back into the queue.
    pub fn start(
        parallel: bool,
        journal: Arc<dyn Journal>,
        shell: Arc<dyn Shell>,
        notify: impl Fn(Notice) + Send + Sync + 'static,
    ) -> Self {
        Self::with(
            System {
                shell: shell.clone(),
            },
            parallel,
            Timing {
                progress: PROGRESS_EVERY,
                watch: WATCH_EVERY,
            },
            journal,
            shell,
            notify,
        )
    }

    fn with(
        engine: impl Engine,
        parallel: bool,
        timing: Timing,
        journal: Arc<dyn Journal>,
        shell: Arc<dyn Shell>,
        notify: impl Fn(Notice) + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(Shared {
            engine: Box::new(engine),
            journal,
            shell,
            notify: Box::new(notify),
            timing,
            state: Mutex::new(State {
                jobs: Vec::new(),
                next_id: 1,
                generation: 0,
                parallel,
                reported: None,
            }),
        }))
    }

    /// Puts `job` at the end of the queue and returns its id. Blocking IO: it asks which disk and
    /// volumes the job is on, and writes the job down.
    pub fn enqueue(&self, job: Job) -> u32 {
        let record = self.0.journal.queued(&job);
        let mut entry = self.0.entry(job, record);
        let mut state = self.0.lock();
        entry.id = state.next_id;
        state.next_id += 1;
        let id = entry.id;
        state.jobs.push(entry);
        self.0.changed(&mut state);
        id
    }

    /// Lists the jobs a Jerry before this one didn't finish, at the end of the queue, as
    /// [`TransferState::Unfinished`]: Resume carries one on, Cancel discards it. Blocking IO, as
    /// [`enqueue`](Self::enqueue).
    pub fn recover(&self, jobs: Vec<Unfinished>) {
        if jobs.is_empty() {
            return;
        }
        let entries: Vec<Entry> = jobs
            .into_iter()
            .map(|unfinished| {
                let meter = Meter::recovered(&unfinished);
                let Unfinished {
                    record,
                    job,
                    started,
                    writing,
                    part,
                    ..
                } = unfinished;
                let mut entry = self.0.entry(job, Some(record));
                entry.stage = Stage::Unfinished;
                entry.meter = meter;
                entry.part = part;
                // One that never started has nothing of its own at the destination.
                if started {
                    entry.next = Next::Resume(Box::new(Resume { writing }));
                }
                entry
            })
            .collect();
        let mut state = self.0.lock();
        for mut entry in entries {
            entry.id = state.next_id;
            state.next_id += 1;
            state.jobs.push(entry);
        }
        self.0.changed(&mut state);
    }

    /// Does `action` on job `job`, or on every job with `None`. What doesn't apply to where a job
    /// is does nothing: a job that ended stays ended. [`ErrorCode::NotFound`] for a job that isn't
    /// listed. Blocking IO: a job that ends here is written down, and the temporary file it left
    /// is removed.
    pub fn control(&self, action: Action, job: Option<u32>) -> Result<(), JerryError> {
        let mut state = self.0.lock();
        let end = |entry: &mut Entry| {
            let summary = waiting_failures(entry);
            entry.apply(action).then(|| Ending {
                notice: entry.ending(&summary),
                record: entry.record,
                part: entry.part.take(),
            })
        };
        let ended: Vec<Ending> = match job {
            Some(id) => {
                let entry = state
                    .find(id)
                    .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
                end(entry).into_iter().collect()
            }
            None => state.jobs.iter_mut().filter_map(end).collect(),
        };
        let mut left = Vec::new();
        for ending in ended {
            (self.0.notify)(ending.notice);
            left.push((ending.record, ending.part));
        }
        self.0.changed(&mut state);
        drop(state);
        for (record, part) in left {
            let removed = part.is_some_and(|part| {
                remove_part(&part)
                    .inspect_err(|error| {
                        tracing::warn!(code = ?error.code, "a temporary file a job left stays");
                    })
                    .is_ok()
            });
            if let Some(record) = record {
                if removed {
                    self.0.journal.part_gone(record);
                }
                self.0.journal.ended(record);
            }
        }
        Ok(())
    }

    /// Answers the conflict job `id` waits on ([`TransferState::Conflict`]). It carries on once
    /// its lane is free: at once, unless another job started on its disk meanwhile.
    /// [`ErrorCode::NotFound`] if the job doesn't wait for an answer (answered already, in
    /// another window); `Internal` for a choice the conflict doesn't offer.
    pub fn resolve(&self, id: u32, answer: Answer) -> Result<(), JerryError> {
        let mut state = self.0.lock();
        let entry = state
            .find(id)
            .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
        let Stage::Asking(asked) = &entry.stage else {
            return Err(JerryError::new(ErrorCode::NotFound));
        };
        if !asked.view.choices.contains(&answer.choice) {
            tracing::error!(?answer, "an answer the conflict doesn't offer");
            return Err(JerryError::internal());
        }
        entry.stage = Stage::Answered(answer);
        self.0.changed(&mut state);
        Ok(())
    }

    /// The file on `side` of the conflict job `id` waits on, for its thumbnail. None if it waits
    /// on none.
    pub fn conflict_file(&self, id: u32, side: ConflictSide) -> Option<ConflictFile> {
        let mut state = self.0.lock();
        let Stage::Asking(asked) = &state.find(id)?.stage else {
            return None;
        };
        Some(match side {
            ConflictSide::Incoming => asked.incoming.clone(),
            ConflictSide::Existing => asked.existing.clone(),
        })
    }

    /// Puts the jobs of `order` first, in that order; the others follow as they were. Ids that
    /// aren't listed (a job that ended meanwhile) do nothing.
    pub fn reorder(&self, order: &[u32]) {
        let ranks: HashMap<u32, usize> = order
            .iter()
            .enumerate()
            .map(|(rank, id)| (*id, rank))
            .collect();
        let mut state = self.0.lock();
        state
            .jobs
            .sort_by_key(|entry| ranks.get(&entry.id).copied().unwrap_or(usize::MAX));
        self.0.changed(&mut state);
    }

    /// Queues again a job that ended with failed items (only those are tried again) or that
    /// couldn't start. [`ErrorCode::NotFound`] if there is no such job, or nothing to try again.
    pub fn retry_failed(&self, id: u32) -> Result<(), JerryError> {
        let mut state = self.0.lock();
        let position = state.jobs.iter().position(|entry| {
            entry.id == id
                && matches!(entry.stage, Stage::Ended(Ended::Done | Ended::Failed(_)))
                && !matches!(entry.next, Next::Nothing)
        });
        let Some(position) = position else {
            return Err(JerryError::new(ErrorCode::NotFound));
        };
        let mut entry = state.jobs.remove(position);
        entry.queue_again();
        state.jobs.push(entry);
        self.0.changed(&mut state);
        Ok(())
    }

    /// Every job, in queue order.
    pub fn list(&self) -> TransferList {
        self.0.lock().list()
    }

    /// How many unfinished jobs read from or write to the volume at `root`.
    pub fn on_volume(&self, root: &Path) -> u32 {
        self.0
            .lock()
            .jobs
            .iter()
            .filter(|entry| !entry.ended())
            .filter(|entry| entry.roots.iter().any(|each| same_root(each, root)))
            .count() as u32
    }

    /// Whether jobs on different disks run at the same time. Turned off, what runs goes on and
    /// nothing else starts until it is done.
    pub fn set_parallel(&self, parallel: bool) {
        let mut state = self.0.lock();
        if state.parallel != parallel {
            state.parallel = parallel;
            self.0.changed(&mut state);
        }
    }
}

/// A job that [`Queue::control`] ended: what to tell the UI, and to write down after the lock.
struct Ending {
    notice: Notice,
    record: Option<i64>,
    part: Option<PathBuf>,
}

/// The items a waiting job has given up on for now: cancelling it ends the job with these.
fn waiting_failures(entry: &Entry) -> Vec<Failure> {
    match (&entry.stage, &entry.next) {
        (Stage::Waiting { .. }, Next::Retry(summary)) => summary.failed.clone(),
        _ => Vec::new(),
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new entry for `job`, queued, its id not yet given. Blocking IO: it asks which disk and
    /// volumes the job is on.
    fn entry(&self, job: Job, record: Option<i64>) -> Entry {
        let lane = self.engine.lane(&job.destination);
        let roots = self.roots(&job);
        let source = job
            .sources
            .first()
            .map_or_else(String::new, |source| display_path(source));
        let destination = display_path(&job.destination);
        Entry {
            id: 0,
            job: Arc::new(job),
            lane,
            roots,
            source,
            destination,
            control: Arc::new(Control::new()),
            stage: Stage::Queued,
            paused: false,
            next: Next::Run,
            meter: Meter::new(),
            record,
            part: None,
        }
    }

    /// The volumes `job` touches. Most sources share a folder, so each folder is asked once.
    fn roots(&self, job: &Job) -> Vec<PathBuf> {
        let folders: HashSet<&Path> = job
            .sources
            .iter()
            .map(|source| source.parent().unwrap_or(source))
            .chain([job.destination.as_path()])
            .collect();
        let mut roots: Vec<PathBuf> = Vec::new();
        for root in folders
            .into_iter()
            .filter_map(|folder| self.engine.root(folder))
        {
            if !roots.iter().any(|each| same_root(each, &root)) {
                roots.push(root);
            }
        }
        roots
    }

    /// After anything that changes the list: starts what can start and tells the UI.
    fn changed(self: &Arc<Self>, state: &mut State) {
        self.dispatch(state);
        state.evict();
        state.generation += 1;
        (self.notify)(Notice::Jobs(state.list()));
    }

    /// Starts every job whose turn it is, and lets an answered one go on.
    fn dispatch(self: &Arc<Self>, state: &mut State) {
        let mut busy: HashSet<Lane> = state
            .jobs
            .iter()
            .filter(|entry| entry.stage == Stage::Active)
            .map(|entry| entry.lane.clone())
            .collect();
        let parallel = state.parallel;
        for entry in &mut state.jobs {
            if !parallel && !busy.is_empty() {
                break;
            }
            let waits = matches!(entry.stage, Stage::Queued | Stage::Answered(_));
            if waits && !entry.paused && !busy.contains(&entry.lane) {
                busy.insert(entry.lane.clone());
                match std::mem::replace(&mut entry.stage, Stage::Active) {
                    // Its thread is waiting for this.
                    Stage::Answered(answer) => entry.control.answer(answer),
                    _ => self.start(entry),
                }
            }
        }
    }

    fn start(self: &Arc<Self>, entry: &mut Entry) {
        entry.stage = Stage::Active;
        entry.meter.begin_turn();
        let turn = Turn {
            id: entry.id,
            job: entry.job.clone(),
            control: entry.control.clone(),
            next: std::mem::replace(&mut entry.next, Next::Nothing),
            record: entry.record,
            counts: entry.meter.counts(),
        };
        let shared = self.clone();
        let started = std::thread::Builder::new()
            .name("transfer".into())
            .spawn(move || shared.work(turn));
        if let Err(error) = started {
            tracing::error!(%error, "a transfer thread didn't start");
            entry.stage = Stage::Ended(Ended::Failed(JerryError::internal()));
        }
    }

    /// A job's thread: its turn, in `run_job` so that a panic fails only this job.
    fn work(self: Arc<Self>, turn: Turn) {
        quiet_errors();
        let Turn {
            id,
            job,
            control,
            next,
            record,
            counts,
        } = turn;
        // What an earlier turn, or an earlier Jerry, left half written.
        self.clear_part(id, record);
        if let Some(record) = record {
            self.journal.started(record, &counts);
        }
        let result = run_job("transfer", || {
            let mut observer = |event: Event<'_>| self.observe(id, event);
            match &next {
                Next::Retry(summary) => self.engine.retry(summary, &control, &mut observer),
                Next::Resume(resume) => self.engine.resume(&job, resume, &control, &mut observer),
                Next::Run | Next::Nothing => self.engine.run(&job, &control, &mut observer),
            }
        });
        // The engine removes what it doesn't finish, but not from a volume that went away.
        let gone = match &result {
            Ok(summary) => matches!(&summary.status, Status::Interrupted(error)
                if error.code == ErrorCode::DeviceGone),
            Err(error) => error.code == ErrorCode::DeviceGone,
        };
        if !gone {
            self.clear_part(id, record);
        }
        self.finish(id, next, result);
    }

    /// Removes the temporary file job `id` left, if it did, and writes down that it is gone.
    fn clear_part(&self, id: u32, record: Option<i64>) {
        let Some(part) = self.lock().find(id).and_then(|entry| entry.part.clone()) else {
            return;
        };
        if let Err(error) = remove_part(&part) {
            tracing::warn!(code = ?error.code, "a temporary file a job left stays for now");
            return;
        }
        if let Some(entry) = self.lock().find(id)
            && entry.part.as_ref() == Some(&part)
        {
            entry.part = None;
        }
        if let Some(record) = record {
            self.journal.part_gone(record);
        }
    }

    /// One report from a running job. Every one counts; the UI hears of them at most every
    /// [`PROGRESS_EVERY`], and the job's ending says how it came out. A file it begins is written
    /// down before the job goes on. A conflict is the UI's to answer, and the job leaves its
    /// lane meanwhile.
    fn observe(self: &Arc<Self>, id: u32, event: Event<'_>) {
        if let Event::Conflict(conflict) = event {
            let file = |path: &Path, side: &Side| ConflictFile {
                path: path.to_path_buf(),
                attributes: side.attributes,
                modified: side.modified,
            };
            let asked = Asked {
                view: self.conflict_view(conflict),
                incoming: file(conflict.source, &conflict.incoming),
                existing: file(conflict.target, &conflict.existing),
            };
            let mut state = self.lock();
            if let Some(entry) = state.find(id)
                && !entry.control.is_cancelled()
            {
                entry.meter.take(event);
                entry.stage = Stage::Asking(Box::new(asked));
                self.changed(&mut state);
            }
            return;
        }
        if let Event::Writing(writing) = event {
            let noted = self.lock().find(id).and_then(|entry| {
                if let Some(part) = writing.part {
                    entry.part = Some(part.to_owned());
                }
                Some((entry.record?, entry.meter.counts()))
            });
            if let Some((record, counts)) = noted {
                self.journal.writing(record, writing, &counts);
            }
            return;
        }
        let mut state = self.lock();
        if let Some(entry) = state.find(id) {
            entry.meter.take(event);
        }
        let now = Instant::now();
        let due = state
            .reported
            .is_none_or(|last| now.duration_since(last) >= self.timing.progress);
        if due {
            state.reported = Some(now);
            let running = state
                .jobs
                .iter()
                .filter(|entry| entry.stage == Stage::Active)
                .map(Entry::progress)
                .collect();
            (self.notify)(Notice::Progress(running));
        }
    }

    /// What the conflict window shows of `conflict`: display text, and icons by type.
    fn conflict_view(&self, conflict: &Conflict<'_>) -> TransferConflict {
        let item = |path: &Path, side: &Side| ConflictItem {
            folder: side.folder,
            size: side.size,
            modified: (side.modified != 0).then(|| filetime_to_unix_ms(side.modified)),
            icon: self
                .shell
                .icon(path.file_name().unwrap_or_default(), side.attributes),
        };
        TransferConflict {
            source: display_path(conflict.source),
            target: display_path(conflict.target),
            incoming: item(conflict.source, &conflict.incoming),
            existing: item(conflict.target, &conflict.existing),
            choices: conflict.choices.to_vec(),
            no_recycle_bin: conflict.no_recycle_bin,
        }
    }

    /// Takes in the end of a job's turn.
    fn finish(self: &Arc<Self>, id: u32, next: Next, result: Result<Summary, JerryError>) {
        let mut state = self.lock();
        let Some(entry) = state.find(id) else {
            return;
        };
        entry.paused = false;
        let mut watch = false;
        let mut ended = Vec::new();
        match result {
            Ok(summary) => {
                entry.meter.take_summary(&summary);
                match summary.status.clone() {
                    Status::Interrupted(error) => {
                        watch = entry.wait(error, Next::Retry(Box::new(summary)));
                    }
                    Status::Cancelled => {
                        entry.stage = Stage::Ended(Ended::Cancelled);
                        ended.push(entry.ending(&summary.failed));
                    }
                    Status::Done => {
                        entry.stage = Stage::Ended(Ended::Done);
                        ended.push(entry.ending(&summary.failed));
                        if !summary.failed.is_empty() {
                            entry.next = Next::Retry(Box::new(summary));
                        }
                    }
                }
            }
            Err(error) if error.code == ErrorCode::Cancelled => {
                entry.stage = Stage::Ended(Ended::Cancelled);
                ended.push(entry.ending(&[]));
            }
            Err(error) if can_wait(&error, &next) => watch = entry.wait(error, next),
            Err(error) => {
                entry.stage = Stage::Ended(Ended::Failed(error));
                entry.next = next;
                ended.push(entry.ending(&[]));
            }
        }
        let locations = watch.then(|| entry.locations());
        let record = entry.record.filter(|_| entry.ended());
        for notice in ended {
            (self.notify)(notice);
        }
        self.changed(&mut state);
        drop(state);
        if let Some(record) = record {
            self.journal.ended(record);
        }
        if let Some(locations) = locations {
            self.watch(id, locations);
        }
    }

    /// Looks every so often whether a waiting job's volumes are back, on a thread of its own: a
    /// server that doesn't answer must hold up nothing else.
    fn watch(self: &Arc<Self>, id: u32, locations: Vec<PathBuf>) {
        let shared = self.clone();
        let started = std::thread::Builder::new()
            .name("transfer-watch".into())
            .spawn(move || {
                quiet_errors();
                loop {
                    std::thread::sleep(shared.timing.watch);
                    if !shared.still_waiting(id) {
                        return;
                    }
                    let back = run_job("transfer watch", || {
                        Ok(locations.iter().all(|path| shared.engine.reachable(path)))
                    });
                    if back == Ok(true) {
                        shared.mark_ready(id);
                        return;
                    }
                }
            });
        if let Err(error) = started {
            tracing::error!(%error, "a transfer watch thread didn't start");
            self.mark_ready(id);
        }
    }

    fn still_waiting(&self, id: u32) -> bool {
        self.lock()
            .find(id)
            .is_some_and(|entry| matches!(entry.stage, Stage::Waiting { ready: false, .. }))
    }

    /// The job's volumes are back: Resume can do its work. The job stays stopped until then.
    fn mark_ready(self: &Arc<Self>, id: u32) {
        let mut state = self.lock();
        if let Some(Entry {
            stage: Stage::Waiting { ready, .. },
            ..
        }) = state.find(id)
        {
            *ready = true;
            self.changed(&mut state);
        }
    }
}

/// Whether a job that couldn't start can wait: for its device, or for room if it had started
/// before. A new job that doesn't fit doesn't start (PRD).
fn can_wait(error: &JerryError, next: &Next) -> bool {
    match error.code {
        ErrorCode::DeviceGone => true,
        ErrorCode::NoSpace { .. } => matches!(next, Next::Retry(_) | Next::Resume(_)),
        _ => false,
    }
}
