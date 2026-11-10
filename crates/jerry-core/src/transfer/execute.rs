//! Doing what a plan says, and the summary of what happened.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use windows::Win32::Foundation::{ERROR_DIR_NOT_EMPTY, ERROR_NOT_SAME_DEVICE, WIN32_ERROR};

use super::control::{Control, Flow};
use super::copy::{Copier, Request, Stamp, Step, patiently, set_attributes, set_times, stamp};
use super::journal::Writing;
use super::link::copy_link;
use super::plan::{Counter, Entry, Plan, check_space};
use super::speed::Speed;
use super::{Event, Failure, Kind, Options, Progress, Status, Summary};
use crate::delete::{Item, children, remove};
use crate::error::{ErrorCode, JerryError};
use crate::rename::is_taken;
use crate::volume::missing_or_gone;

/// What became of a plan's entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Pending,
    Done,
    /// It failed, or the folder it was going into did.
    Failed,
}

/// Runs `plan` and returns its summary.
pub(super) fn execute(
    mut plan: Plan,
    destination: &Path,
    options: Options,
    control: &Control,
    observer: &mut dyn FnMut(Event<'_>),
) -> Summary {
    let mut run = Run {
        control,
        observer,
        destination,
        mover: options.mover,
        copier: options.copier,
        kind: plan.kind,
        files_done: 0,
        files_total: plan.tally.files + (plan.renames.len() + plan.removals.len()) as u64,
        bytes_done: 0,
        bytes_total: plan.tally.bytes,
        streamed: 0,
        speed: Speed::default(),
        summary: Summary {
            status: Status::Done,
            files: 0,
            folders: 0,
            bytes: 0,
            failed: Vec::new(),
            kind: plan.kind,
            destination: destination.to_owned(),
            options,
            pending: Vec::new(),
            leftovers: Vec::new(),
            resuming: plan.resuming.clone(),
        },
    };
    for failure in std::mem::take(&mut plan.failed) {
        run.fail(failure);
    }
    let mut stamps = HashMap::new();
    let mut marks = Vec::new();
    let stop = run
        .rename_all(&mut plan)
        .or_else(|| run.remove_all(&mut plan))
        .or_else(|| {
            marks = vec![Mark::Pending; plan.entries.len()];
            run.write_all(&plan.entries, &mut marks, &mut stamps)
        });
    // A rename stage that stopped never got as far as the entries.
    marks.resize(plan.entries.len(), Mark::Pending);
    run.close(plan, &marks, &stamps, stop.unwrap_or(Status::Done))
}

struct Run<'a, 'o> {
    control: &'a Control,
    observer: &'a mut (dyn FnMut(Event<'_>) + 'o),
    destination: &'a Path,
    mover: fn(&Path, &Path) -> windows::core::Result<()>,
    copier: Copier,
    kind: Kind,
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    bytes_total: u64,
    /// Bytes actually written so far, which is what the speed is about.
    streamed: u64,
    speed: Speed,
    summary: Summary,
}

impl Run<'_, '_> {
    fn fail(&mut self, failure: Failure) {
        (self.observer)(Event::Failed(&failure));
        self.summary.failed.push(failure);
    }

    /// Takes in a failed step: the status the job ends with, if this ends it.
    fn settle(&mut self, mut failure: Failure) -> Option<Status> {
        if failure.error.code == ErrorCode::Cancelled && self.control.is_cancelled() {
            return Some(Status::Cancelled);
        }
        failure.error = missing_or_gone(failure.error, &failure.target);
        failure.error = missing_or_gone(failure.error, &failure.source);
        // Every file left would fail the same way.
        let stop = matches!(
            failure.error.code,
            ErrorCode::DeviceGone | ErrorCode::NoSpace { .. }
        )
        .then(|| Status::Interrupted(failure.error.clone()));
        self.fail(failure);
        stop
    }

    /// Waits out a pause: the status the job ends with, if it was cancelled.
    fn gate(&mut self) -> Option<Status> {
        match self.control.checkpoint() {
            Flow::Go => None,
            Flow::Resumed => {
                self.speed.restart();
                None
            }
            Flow::Cancelled => Some(Status::Cancelled),
        }
    }

    fn report(&mut self, file: &Path, current: u64) {
        let speed = self.speed.update(Instant::now(), self.streamed);
        (self.observer)(Event::Progress(&Progress {
            file,
            files_done: self.files_done,
            files_total: self.files_total,
            bytes_done: self.bytes_done + current,
            bytes_total: self.bytes_total,
            speed,
        }));
    }

    /// One more file is dealt with, however that went.
    fn advance(&mut self, file: &Path, bytes: u64) {
        self.files_done += 1;
        self.bytes_done += bytes;
        self.report(file, 0);
    }

    fn rename_all(&mut self, plan: &mut Plan) -> Option<Status> {
        let mut renames = std::mem::take(&mut plan.renames).into_iter();
        while let Some((item, target)) = renames.next() {
            let stop = self.gate().or_else(|| self.rename(plan, item, target));
            if let Some(stop) = stop {
                self.keep_for_retry(&stop, renames.map(|(item, target)| (item.path, target)));
                return Some(stop);
            }
        }
        None
    }

    fn rename(&mut self, plan: &mut Plan, item: Item, target: PathBuf) -> Option<Status> {
        let failure = match (self.mover)(&item.path, &target) {
            Ok(()) => {
                if item.is_dir() && !item.link {
                    self.summary.folders += 1;
                } else {
                    self.summary.files += 1;
                }
                self.advance(&item.path, 0);
                return None;
            }
            // The paths looked like one volume, but a junction on the way leads to another.
            Err(error) if WIN32_ERROR::from_error(&error) == Some(ERROR_NOT_SAME_DEVICE) => {
                return self.fall_back_to_copying(plan, item, target);
            }
            Err(error) if is_taken(&error) => JerryError::new(ErrorCode::InvalidName).at(&target),
            Err(error) => JerryError::from(error).at(&item.path),
        };
        let stop = self.settle(Failure {
            source: item.path.clone(),
            target,
            error: failure,
            copied: false,
        });
        if stop.is_none() {
            self.advance(&item.path, 0);
        }
        stop
    }

    /// Adds to the plan what a rename turned out not to be able to do.
    fn fall_back_to_copying(
        &mut self,
        plan: &mut Plan,
        item: Item,
        target: PathBuf,
    ) -> Option<Status> {
        let source = item.path.clone();
        let goal = target.clone();
        let mut extra = Plan::new(self.kind, plan.resuming.clone());
        let mut counter = Counter::new(self.control, &mut *self.observer);
        if extra.walk(item, target, &mut counter).is_err() {
            return Some(Status::Cancelled);
        }
        if let Err(error) = check_space(self.destination, extra.tally.bytes) {
            return self.settle(Failure {
                source,
                target: goal,
                error,
                copied: false,
            });
        }
        self.files_total = self.files_total - 1 + extra.tally.files + extra.removals.len() as u64;
        self.bytes_total += extra.tally.bytes;
        plan.absorb(extra);
        None
    }

    /// A retried move: removes the sources whose copies are complete.
    fn remove_all(&mut self, plan: &mut Plan) -> Option<Status> {
        let mut removals = std::mem::take(&mut plan.removals).into_iter();
        while let Some((item, target)) = removals.next() {
            let stop = self.gate().or_else(|| {
                let failure = match remove_copied_source(&item, &target) {
                    Ok(()) => {
                        if item.is_dir() && !item.link {
                            self.summary.folders += 1;
                        } else {
                            self.summary.files += 1;
                        }
                        self.advance(&item.path, 0);
                        return None;
                    }
                    Err(error) => error,
                };
                let stop = self.settle(Failure {
                    source: item.path.clone(),
                    target: target.clone(),
                    error: failure,
                    copied: true,
                });
                if stop.is_none() {
                    self.advance(&item.path, 0);
                }
                stop
            });
            if let Some(stop) = stop {
                self.keep_copied_for_retry(&stop, &mut removals);
                return Some(stop);
            }
        }
        None
    }

    /// An interrupted job's removals that weren't reached stay on its list of what to try again.
    fn keep_copied_for_retry(
        &mut self,
        stop: &Status,
        removals: &mut impl Iterator<Item = (Item, PathBuf)>,
    ) {
        if let Status::Interrupted(error) = stop {
            for (item, target) in removals {
                self.summary.failed.push(Failure {
                    source: item.path,
                    target,
                    error: error.clone(),
                    copied: true,
                });
            }
        }
    }

    fn keep_for_retry(&mut self, stop: &Status, items: impl Iterator<Item = (PathBuf, PathBuf)>) {
        if matches!(stop, Status::Interrupted(_)) {
            self.summary.pending.extend(items);
        }
    }

    fn write_all(
        &mut self,
        entries: &[Entry],
        marks: &mut [Mark],
        stamps: &mut HashMap<usize, Stamp>,
    ) -> Option<Status> {
        for (index, entry) in entries.iter().enumerate() {
            if entry
                .parent
                .is_some_and(|parent| marks[parent] != Mark::Done)
            {
                // Its folder's failure covers it, and trying that again brings this along.
                marks[index] = Mark::Failed;
                self.count_entry(entry);
                continue;
            }
            if let Some(stop) = self.gate() {
                return Some(stop);
            }
            match self.write_entry(index, entry, stamps) {
                Ok(()) => {
                    marks[index] = Mark::Done;
                    self.count_entry(entry);
                }
                Err((error, copied)) => {
                    let stop = self.settle(Failure {
                        source: entry.item.path.clone(),
                        target: entry.target.clone(),
                        error,
                        copied,
                    });
                    if stop == Some(Status::Cancelled) {
                        return stop;
                    }
                    marks[index] = Mark::Failed;
                    self.count_entry(entry);
                    if stop.is_some() {
                        return stop;
                    }
                }
            }
        }
        None
    }

    fn count_entry(&mut self, entry: &Entry) {
        if !entry.is_folder() {
            self.advance(&entry.item.path, entry.item.size);
        }
    }

    /// Writes one entry. A failure says what went wrong, and whether the copy is complete anyway
    /// (a move whose source couldn't be removed).
    fn write_entry(
        &mut self,
        index: usize,
        entry: &Entry,
        stamps: &mut HashMap<usize, Stamp>,
    ) -> Result<(), (JerryError, bool)> {
        let not_copied = |error| (error, false);
        if entry.is_folder() {
            let made = make_folder(entry).map_err(not_copied)?;
            stamps.insert(index, made);
            self.summary.folders += 1;
            return Ok(());
        }
        let bytes = if entry.item.link {
            (self.observer)(Event::Writing(&Writing {
                source: &entry.item.path,
                target: &entry.target,
                part: None,
            }));
            copy_link(&entry.item, &entry.target).map_err(not_copied)?;
            0
        } else {
            self.copy_entry(entry).map_err(not_copied)?
        };
        if self.kind == Kind::Move {
            // Only now: the copy is complete, on the disk, and has been checked.
            patiently(|| remove(&entry.item)).map_err(|error| (error, true))?;
        }
        self.summary.files += 1;
        self.summary.bytes += bytes;
        Ok(())
    }

    fn copy_entry(&mut self, entry: &Entry) -> Result<u64, JerryError> {
        let control = self.control;
        let size = entry.item.size;
        let file = entry.item.path.as_path();
        let request = Request {
            source: file,
            target: &entry.target,
            for_move: self.kind == Kind::Move,
        };
        let copier = self.copier;
        let mut last = 0_u64;
        copier(&request, &mut |step| {
            let transferred = match step {
                Step::Started(part) => {
                    (self.observer)(Event::Writing(&Writing {
                        source: file,
                        target: &entry.target,
                        part: Some(part),
                    }));
                    return Flow::Go;
                }
                Step::Copied(transferred) => transferred,
            };
            self.streamed += transferred.saturating_sub(last);
            last = transferred;
            // Windows' totals count alternate data streams too; the plan's sizes don't.
            self.report(file, transferred.min(size));
            let flow = control.checkpoint();
            if flow == Flow::Resumed {
                self.speed.restart();
            }
            flow
        })
    }

    /// Finishes up: folder dates, emptied source folders, and what is left to try again.
    fn close(
        mut self,
        plan: Plan,
        marks: &[Mark],
        stamps: &HashMap<usize, Stamp>,
        status: Status,
    ) -> Summary {
        let mut kept = vec![false; plan.entries.len()];
        // Backwards, so that a folder comes after everything in it.
        for (index, entry) in plan.entries.into_iter().enumerate().rev() {
            let finished = marks[index] == Mark::Done && !kept[index];
            if !finished && let Some(parent) = entry.parent {
                kept[parent] = true;
            }
            if marks[index] == Mark::Pending
                && matches!(status, Status::Interrupted(_))
                && entry
                    .parent
                    .is_none_or(|parent| marks[parent] == Mark::Done)
            {
                self.summary.pending.push((entry.item.path, entry.target));
                continue;
            }
            if entry.is_folder() && marks[index] == Mark::Done {
                self.close_folder(entry, index, stamps, finished, &status);
            }
        }
        for item in plan.leftovers {
            self.remove_if_empty(item);
        }
        self.summary.status = status;
        self.summary
    }

    /// Finishes a folder this job created at the destination, and its source if that has to go.
    fn close_folder(
        &mut self,
        entry: Entry,
        index: usize,
        stamps: &HashMap<usize, Stamp>,
        finished: bool,
        status: &Status,
    ) {
        // Nothing of a cancelled job's half-made folders should stay, unless it holds finished
        // files. Removing a folder fails if anything is in it. One that was there before this
        // turn may not be the job's own.
        if *status == Status::Cancelled
            && !finished
            && !entry.merge
            && std::fs::remove_dir(&entry.target).is_ok()
        {
            return;
        }
        if let Some(source) = stamps.get(&index) {
            restore_folder(&entry.target, source, self.kind);
        }
        if self.kind != Kind::Move {
            return;
        }
        if !finished {
            self.summary.leftovers.push(entry.item);
        } else if let Err(error) = remove(&entry.item) {
            self.fail(Failure {
                source: entry.item.path,
                target: entry.target,
                error,
                copied: true,
            });
        }
    }

    /// A retried move: a source folder goes when the retry has emptied it.
    fn remove_if_empty(&mut self, item: Item) {
        match children(&item.path) {
            Ok(inside) if inside.is_empty() => {
                if let Err(error) = remove(&item) {
                    tracing::warn!(%error, "can't remove an emptied source folder");
                    self.summary.leftovers.push(item);
                }
            }
            Ok(_) => self.summary.leftovers.push(item),
            // It has gone already, or will be there next time.
            Err(error) if error.code == ErrorCode::NotFound => {}
            Err(_) => self.summary.leftovers.push(item),
        }
    }
}

/// Creates the folder of `entry`, unless a resumed job finds it there, and returns what its
/// source looks like before anything moves out of it.
fn make_folder(entry: &Entry) -> Result<Stamp, JerryError> {
    if let Some(error) = &entry.unreadable {
        return Err(error.clone());
    }
    let source = stamp(&entry.item.path)?;
    if entry.merge {
        return Ok(source);
    }
    std::fs::create_dir(&entry.target).map_err(|error| {
        let code = if error.kind() == io::ErrorKind::AlreadyExists {
            JerryError::new(ErrorCode::InvalidName)
        } else {
            JerryError::from(error)
        };
        code.at(&entry.target)
    })?;
    Ok(source)
}

/// Gives a finished folder its source's attributes and dates. Last, because writing into a folder
/// changes its date. A move keeps the creation date too, like a move of a file. The folder is
/// fine without these, so a failure is only logged.
fn restore_folder(target: &Path, source: &Stamp, kind: Kind) {
    let created = (kind == Kind::Move).then_some(&source.created);
    if let Err(error) = set_times(target, created, Some(&source.written)) {
        tracing::warn!(%error, "can't set the dates of a copied folder");
    }
    if let Err(error) = set_attributes(target, source.attributes) {
        tracing::warn!(%error, "can't set the attributes of a copied folder");
    }
}

/// A retried move: removes a source whose copy is complete, once it is sure of that.
fn remove_copied_source(item: &Item, target: &Path) -> Result<(), JerryError> {
    let copy = stamp(target)?;
    if item.is_dir() && !item.link {
        if !children(&item.path)?.is_empty() {
            return Err(JerryError::from_win32(ERROR_DIR_NOT_EMPTY.0).at(&item.path));
        }
    } else if !item.link && !stamp(&item.path)?.same_content(&copy) {
        return Err(JerryError::new(ErrorCode::InUse { app: None }).at(&item.path));
    }
    remove(item)
}
