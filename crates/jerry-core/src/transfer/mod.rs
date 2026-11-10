//! Copying and moving files: one job, from counting to its summary (Tech PRD «ტრანსფერების
//! ძრავი», #48). [`queue`] runs jobs one per disk and reports them to the UI (#49).
//!
//! A job counts what it will write, checks the destination has room, does the work, and returns a
//! [`Summary`]. What keeps the user's files safe (data safety rules 1, 2 and 7):
//!
//! - A file is written as `name.ext.jerrypart` (hidden) and gets its final name only once it is
//!   complete and its size checked, so a half-written file never has the final name.
//! - The final name never replaces anything. A name that is taken is a [`conflict`]: the job asks
//!   ([`Event::Conflict`]) and waits for the answer, unless "Apply to all" answered it already.
//!   Replace sends the file there to the Recycle Bin ([`Bin`]) once the copy is complete, and only
//!   then the copy takes the name (rule 3).
//! - A move across volumes deletes each source file only after its copy is complete, flushed to
//!   disk and checked, and only if the source didn't change meanwhile. A cancelled move leaves
//!   every file in exactly one place.
//! - Links (symbolic links, junctions, mount points) are never entered, so a link loop can't catch
//!   a job: a link is copied as a link.
//!
//! One file's failure doesn't stop the job: it goes on the [`Summary`]'s list and [`retry`] does
//! only those. A destination that is gone or full ends the job early instead of failing every file
//! left ([`Status::Interrupted`]); [`retry`] then carries on from there. A job that Jerry didn't
//! finish ([`journal`]) carries on with [`resume`].
//!
//! Everything here is blocking IO: run it on a background thread, in `job::run_job`.

pub mod conflict;
mod control;
mod copy;
mod execute;
pub mod journal;
mod link;
mod plan;
pub mod queue;
mod speed;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use conflict::Policy;
pub use conflict::{Answer, Bin, Choice, Conflict};
pub use control::Control;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::delete::{Item, Tally};
use crate::error::JerryError;

/// What a job does with its sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[serde(rename = "TransferKind")]
pub enum Kind {
    Copy,
    Move,
}

/// A copy or move of some items into one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub kind: Kind,
    /// The files and folders to copy or move, as long paths (as core keeps them).
    pub sources: Vec<PathBuf>,
    /// The folder they go into, as a long path. It must exist.
    pub destination: PathBuf,
}

/// The stages of a job, in order. The [`Summary`] is the last one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
#[serde(rename = "TransferPhase")]
pub enum Phase {
    Counting,
    CheckingSpace,
    Working,
}

/// How far a job has come. Failed and skipped items count as done, so it always reaches the total.
#[derive(Debug, Clone, Copy)]
pub struct Progress<'a> {
    /// The file being worked on.
    pub file: &'a Path,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// Bytes per second over the last few seconds of copying; 0 until that is known.
    pub speed: u64,
}

/// What a running job reports. It comes on the job's thread, as often as Windows reports progress:
/// whoever sends it on to the UI limits the rate.
#[derive(Debug, Clone, Copy)]
pub enum Event<'a> {
    /// The job entered a phase.
    Phase(Phase),
    /// What counting has found so far, every [`MEASURE_PROGRESS`](crate::delete::MEASURE_PROGRESS).
    Counted(&'a Tally),
    /// The job advanced.
    Progress(&'a Progress<'a>),
    /// An item failed; the job goes on.
    Failed(&'a Failure),
    /// The job begins to write a file: its temporary file is there and empty, or a link is about
    /// to be made. The copy goes on once this returns, so what hears it can write it down first.
    Writing(&'a journal::Writing<'a>),
    /// A name the job writes is taken, and its policy has no answer. The job waits for
    /// [`Control::answer`], which may come while this is heard, or for a cancel.
    Conflict(&'a Conflict<'a>),
}

/// What a job that Jerry didn't finish knew as it stopped, for [`resume`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resume {
    /// The last file it began to write: `(source, target)`. A move may have finished its copy
    /// and not yet removed its source.
    pub writing: Option<(PathBuf, PathBuf)>,
}

/// An item that couldn't be copied or moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub source: PathBuf,
    /// Where it was going.
    pub target: PathBuf,
    pub error: JerryError,
    /// A move: the copy is complete at `target`, and only removing the source failed.
    pub copied: bool,
}

/// How a job ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Everything was attempted. Some items may have failed: see [`Summary::failed`].
    Done,
    /// The user cancelled. What was finished stays.
    Cancelled,
    /// The job stopped early: a volume went away or filled up. [`retry`] carries on from there.
    Interrupted(JerryError),
}

/// What a job did.
#[derive(Debug, Clone)]
pub struct Summary {
    pub status: Status,
    /// Files and links fully copied or moved.
    pub files: u64,
    /// Folders created at the destination, or moved whole.
    pub folders: u64,
    /// Bytes of the files copied or moved by copying.
    pub bytes: u64,
    /// Files and links left where they were because their name was taken (Skip).
    pub skipped: u64,
    pub failed: Vec<Failure>,
    kind: Kind,
    destination: PathBuf,
    options: Options,
    /// What "Apply to all" answered: [`retry`] keeps it.
    policy: Policy,
    /// Items an interrupted job never got to: `(source, target)`.
    pending: Vec<(PathBuf, PathBuf)>,
    /// A move: source folders that stayed because something in them did, deepest first.
    leftovers: Vec<Item>,
    /// A resumed job: [`retry`] carries on as [`resume`] does.
    resuming: Option<Resume>,
}

/// How a job goes about its work. Only tests change this, because a test machine may have only
/// one volume.
#[derive(Debug, Clone, Copy)]
struct Options {
    /// Move within a volume by renaming. Off, a move copies and deletes as across volumes.
    rename: bool,
    /// What renames.
    mover: fn(&Path, &Path) -> windows::core::Result<()>,
    /// What copies a file.
    copier: copy::Copier,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            rename: true,
            mover: crate::rename::move_file,
            copier: copy::copy_file,
        }
    }
}

/// Runs `job` to its end on this thread and returns what it did. `observer` hears about it as it
/// goes; `control` pauses, cancels and answers its conflicts from another thread. A file that
/// Replace replaces goes to `bin`.
///
/// Fails, with nothing changed, when the job can't start: [`IntoItself`], [`NoSpace`] (with what
/// it needs and what is free), a destination that doesn't exist, or [`Cancelled`] while counting.
/// Once it has started it returns a [`Summary`].
///
/// [`IntoItself`]: crate::ErrorCode::IntoItself
/// [`NoSpace`]: crate::ErrorCode::NoSpace
/// [`Cancelled`]: crate::ErrorCode::Cancelled
pub fn run(
    job: &Job,
    control: &Control,
    bin: &dyn Bin,
    mut observer: impl FnMut(Event<'_>),
) -> Result<Summary, JerryError> {
    run_with(job, Options::default(), control, bin, &mut observer)
}

fn run_with(
    job: &Job,
    options: Options,
    control: &Control,
    bin: &dyn Bin,
    observer: &mut dyn FnMut(Event<'_>),
) -> Result<Summary, JerryError> {
    observer(Event::Phase(Phase::Counting));
    let plan = plan::build(job, options, None, control, observer)?;
    let run = Turn {
        destination: &job.destination,
        options,
        policy: Policy::default(),
        control,
        bin,
    };
    start(plan, &run, observer)
}

/// Carries on `job`, which a Jerry started and didn't finish, as [`run`] would but from where it
/// stopped. What is already at the destination stays and isn't counted again:
///
/// - a folder there is filled, not made again;
/// - a copy whose file is there with its source's size and date is done;
/// - a move's sources that are gone were moved; the file it was writing as it stopped
///   (`resume.writing`), if its copy is complete, only has its source removed, and only if that
///   source is unchanged.
///
/// Anything else already there is a conflict, as with [`run`]; a copy's file that is there under
/// a name Keep both gave it counts as done too. The policy of "Apply to all" isn't known to a
/// Jerry that stopped, so the resumed job asks again. The temporary file the job left isn't this
/// function's: remove it first ([`journal::remove_part`]).
pub fn resume(
    job: &Job,
    resume: &Resume,
    control: &Control,
    bin: &dyn Bin,
    mut observer: impl FnMut(Event<'_>),
) -> Result<Summary, JerryError> {
    resume_with(job, resume, Options::default(), control, bin, &mut observer)
}

fn resume_with(
    job: &Job,
    resume: &Resume,
    options: Options,
    control: &Control,
    bin: &dyn Bin,
    observer: &mut dyn FnMut(Event<'_>),
) -> Result<Summary, JerryError> {
    observer(Event::Phase(Phase::Counting));
    let plan = plan::build(job, options, Some(resume), control, observer)?;
    let run = Turn {
        destination: &job.destination,
        options,
        policy: Policy::default(),
        control,
        bin,
    };
    start(plan, &run, observer)
}

/// Tries again what `summary` lists as failed and, after an interruption, what the job never got
/// to, with what "Apply to all" answered before. A move's copies that are already complete only
/// have their sources removed. Fails like [`run`] if it can't start.
pub fn retry(
    summary: &Summary,
    control: &Control,
    bin: &dyn Bin,
    mut observer: impl FnMut(Event<'_>),
) -> Result<Summary, JerryError> {
    let observer: &mut dyn FnMut(Event<'_>) = &mut observer;
    observer(Event::Phase(Phase::Counting));
    let plan = plan::for_retry(summary, control, observer)?;
    let run = Turn {
        destination: &summary.destination,
        options: summary.options,
        policy: summary.policy,
        control,
        bin,
    };
    start(plan, &run, observer)
}

/// What one turn of a job works with besides its plan.
struct Turn<'a> {
    destination: &'a Path,
    options: Options,
    /// What "Apply to all" answered so far.
    policy: Policy,
    control: &'a Control,
    bin: &'a dyn Bin,
}

fn start(
    plan: plan::Plan,
    turn: &Turn<'_>,
    observer: &mut dyn FnMut(Event<'_>),
) -> Result<Summary, JerryError> {
    // What counting found, also when it was too quick to report as it went: the journal writes
    // the totals down with the first file.
    observer(Event::Counted(&plan.tally));
    observer(Event::Phase(Phase::CheckingSpace));
    plan::check_space(turn.destination, plan.tally.bytes)?;
    observer(Event::Phase(Phase::Working));
    Ok(execute::execute(plan, turn, observer))
}
