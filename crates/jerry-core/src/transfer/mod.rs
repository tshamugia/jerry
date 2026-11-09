//! Copying and moving files: one job, from counting to its summary (Tech PRD «ტრანსფერების
//! ძრავი», #48). [`queue`] runs jobs one per disk and reports them to the UI (#49).
//!
//! A job counts what it will write, checks the destination has room, does the work, and returns a
//! [`Summary`]. What keeps the user's files safe (data safety rules 1, 2 and 7):
//!
//! - A file is written as `name.ext.jerrypart` (hidden) and gets its final name only once it is
//!   complete and its size checked, so a half-written file never has the final name.
//! - The final name never replaces anything: an existing item at the destination fails that item.
//!   Choosing Replace, Skip or Keep both comes with the conflict window (#51).
//! - A move across volumes deletes each source file only after its copy is complete, flushed to
//!   disk and checked, and only if the source didn't change meanwhile. A cancelled move leaves
//!   every file in exactly one place.
//! - Links (symbolic links, junctions, mount points) are never entered, so a link loop can't catch
//!   a job: a link is copied as a link.
//!
//! One file's failure doesn't stop the job: it goes on the [`Summary`]'s list and [`retry`] does
//! only those. A destination that is gone or full ends the job early instead of failing every file
//! left ([`Status::Interrupted`]); [`retry`] then carries on from there.
//!
//! Everything here is blocking IO: run it on a background thread, in `job::run_job`.

mod control;
mod copy;
mod execute;
mod link;
mod plan;
pub mod queue;
mod speed;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

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
#[derive(Debug, Clone)]
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
    pub failed: Vec<Failure>,
    kind: Kind,
    destination: PathBuf,
    options: Options,
    /// Items an interrupted job never got to: `(source, target)`.
    pending: Vec<(PathBuf, PathBuf)>,
    /// A move: source folders that stayed because something in them did, deepest first.
    leftovers: Vec<Item>,
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
/// goes; `control` pauses and cancels it from another thread.
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
    mut observer: impl FnMut(Event<'_>),
) -> Result<Summary, JerryError> {
    run_with(job, Options::default(), control, &mut observer)
}

fn run_with(
    job: &Job,
    options: Options,
    control: &Control,
    observer: &mut dyn FnMut(Event<'_>),
) -> Result<Summary, JerryError> {
    observer(Event::Phase(Phase::Counting));
    let plan = plan::build(job, options, control, observer)?;
    start(plan, &job.destination, options, control, observer)
}

/// Tries again what `summary` lists as failed and, after an interruption, what the job never got
/// to. A move's copies that are already complete only have their sources removed. Fails like
/// [`run`] if it can't start.
pub fn retry(
    summary: &Summary,
    control: &Control,
    mut observer: impl FnMut(Event<'_>),
) -> Result<Summary, JerryError> {
    let observer: &mut dyn FnMut(Event<'_>) = &mut observer;
    observer(Event::Phase(Phase::Counting));
    let plan = plan::for_retry(summary, control, observer)?;
    start(
        plan,
        &summary.destination,
        summary.options,
        control,
        observer,
    )
}

fn start(
    plan: plan::Plan,
    destination: &Path,
    options: Options,
    control: &Control,
    observer: &mut dyn FnMut(Event<'_>),
) -> Result<Summary, JerryError> {
    observer(Event::Phase(Phase::CheckingSpace));
    plan::check_space(destination, plan.tally.bytes)?;
    observer(Event::Phase(Phase::Working));
    Ok(execute::execute(
        plan,
        destination,
        options,
        control,
        observer,
    ))
}
