//! Doing what a plan says, and the summary of what happened.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use windows::Win32::Foundation::{
    ERROR_DIR_NOT_EMPTY, ERROR_NOT_SAME_DEVICE, FILETIME, WIN32_ERROR,
};

use super::conflict::{self, Choice, Conflict, NO_BIN_CHOICES, Policy, Side, free_name};
use super::control::{Control, Flow};
use super::copy::{
    Copier, Request, Stamp, Step as CopyStep, Written, check_unchanged, discard, patiently,
    set_attributes, set_times, stamp, take_name,
};
use super::journal::Writing;
use super::link::copy_link;
use super::plan::{Counter, Entry, Plan, check_space};
use super::speed::Speed;
use super::{Bin, Event, Failure, Kind, Progress, Status, Summary, Turn};
use crate::delete::{Item, children, probe, remove};
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
    /// Its name was taken and the user chose Skip: it stays where it was.
    Skipped,
}

/// How one item came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Done,
    Skipped,
    /// A folder moved by renaming went into the folder of its name that was there: what it holds
    /// follows on its own.
    Merged,
}

/// How a conflict was settled.
enum Resolved {
    /// The copy takes the name, once what is there is out of the way.
    Replace,
    Skip,
    /// It goes under this free name instead.
    Rename(PathBuf),
    /// A folder fills the one that is there.
    Merge,
}

/// What making room for a copy that replaces a file came to.
enum Room {
    Made,
    /// The file couldn't go to the Recycle Bin, and the user chose to keep it.
    Skip,
    /// The same, and the copy goes under this free name instead.
    Rename(PathBuf),
}

/// What is left to do of a move by renaming.
enum Step {
    Rename {
        item: Item,
        target: PathBuf,
        /// It is in a folder that is being merged, so a folder of its name merges too.
        merging: bool,
    },
    /// A folder whose content was merged elsewhere goes, once it is empty.
    Tidy(Item),
}

/// Runs `plan` and returns its summary.
pub(super) fn execute(
    mut plan: Plan,
    turn: &Turn<'_>,
    observer: &mut dyn FnMut(Event<'_>),
) -> Summary {
    let mut run = Run {
        control: turn.control,
        observer,
        destination: turn.destination,
        mover: turn.options.mover,
        copier: turn.options.copier,
        bin: turn.bin,
        policy: turn.policy,
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
            skipped: 0,
            failed: Vec::new(),
            kind: plan.kind,
            destination: turn.destination.to_owned(),
            options: turn.options,
            policy: turn.policy,
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
            run.write_all(&mut plan.entries, &mut marks, &mut stamps)
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
    bin: &'a dyn Bin,
    /// What "Apply to all" answered so far.
    policy: Policy,
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

    /// Settles the conflict of `item` going to `target`, where `existing` is: by the job's
    /// policy, or by asking. A folder in a folder being merged (`merging`) merges too. Fails
    /// with [`ErrorCode::Cancelled`] if the job is cancelled while it asks.
    fn resolve(
        &mut self,
        item: &Item,
        target: &Path,
        existing: &Item,
        merging: bool,
    ) -> Result<Resolved, JerryError> {
        let folder = is_folder(item);
        let choices = conflict::choices(item, existing, target);
        if merging && choices.contains(&Choice::Merge) {
            return Ok(Resolved::Merge);
        }
        let choice = match self.policy.answer(folder, choices, false) {
            Some(choice) => choice,
            None => self.ask(item, target, existing, choices, false)?,
        };
        Ok(match choice {
            Choice::Replace => Resolved::Replace,
            Choice::Skip => Resolved::Skip,
            Choice::Merge => Resolved::Merge,
            Choice::KeepBoth => Resolved::Rename(free_name(target, folder)?),
            Choice::ReplaceIfNewer if newer(&item.path, target) => Resolved::Replace,
            Choice::ReplaceIfNewer => Resolved::Skip,
        })
    }

    /// Reports a conflict and waits for the user's answer, which "Apply to all" keeps in the
    /// policy. Fails with [`ErrorCode::Cancelled`] if the job is cancelled meanwhile.
    fn ask(
        &mut self,
        item: &Item,
        target: &Path,
        existing: &Item,
        choices: &'static [Choice],
        no_bin: bool,
    ) -> Result<Choice, JerryError> {
        let folder = is_folder(item);
        let conflict = Conflict {
            source: &item.path,
            target,
            incoming: side(&item.path, item),
            existing: side(target, existing),
            choices,
            no_recycle_bin: no_bin,
        };
        loop {
            self.control.forget_answer();
            (self.observer)(Event::Conflict(&conflict));
            let Some(answer) = self.control.wait_answer() else {
                return Err(JerryError::new(ErrorCode::Cancelled));
            };
            // The time it waited isn't copying time.
            self.speed.restart();
            if choices.contains(&answer.choice) {
                self.policy.learn(folder, no_bin, answer);
                return Ok(answer.choice);
            }
            tracing::warn!(
                ?answer,
                "an answer that doesn't fit the conflict; asking again"
            );
        }
    }

    /// Makes room for a copy that replaces the file at `target`: it goes to the Recycle Bin (data
    /// safety rule 3). Where it can't, the user is asked again, and Replace then deletes it for
    /// good (rule 5).
    fn make_room(&mut self, item: &Item, target: &Path) -> Result<Room, JerryError> {
        match self.bin.recycle(target) {
            Ok(()) => return Ok(Room::Made),
            Err(error) if error.code == ErrorCode::NoRecycleBin => {}
            // It went meanwhile.
            Err(error) if error.code == ErrorCode::NotFound => return Ok(Room::Made),
            Err(error) => return Err(error),
        }
        let existing = match probe(target) {
            Ok(existing) if existing.is_dir() => {
                return Err(JerryError::new(ErrorCode::InvalidName).at(target));
            }
            Ok(existing) => existing,
            Err(error) if error.code == ErrorCode::NotFound => return Ok(Room::Made),
            Err(error) => return Err(error),
        };
        let choice = match self.policy.answer(false, NO_BIN_CHOICES, true) {
            Some(choice) => choice,
            None => self.ask(item, target, &existing, NO_BIN_CHOICES, true)?,
        };
        match choice {
            Choice::Replace => {
                tracing::info!("a replaced file with no Recycle Bin to go to is deleted for good");
                patiently(|| remove(&existing))?;
                Ok(Room::Made)
            }
            Choice::Skip => Ok(Room::Skip),
            Choice::KeepBoth => Ok(Room::Rename(free_name(target, false)?)),
            Choice::ReplaceIfNewer | Choice::Merge => Err(JerryError::internal().at(target)),
        }
    }

    fn rename_all(&mut self, plan: &mut Plan) -> Option<Status> {
        let mut steps: VecDeque<Step> = std::mem::take(&mut plan.renames)
            .into_iter()
            .map(|(item, target)| Step::Rename {
                item,
                target,
                merging: false,
            })
            .collect();
        while let Some(step) = steps.pop_front() {
            let stop = match step {
                Step::Rename {
                    item,
                    target,
                    merging,
                } => self
                    .gate()
                    .or_else(|| self.rename(plan, item, target, merging, &mut steps)),
                Step::Tidy(folder) => {
                    self.remove_if_empty(folder);
                    None
                }
            };
            if let Some(stop) = stop {
                for step in steps {
                    match step {
                        Step::Rename { item, target, .. } => {
                            self.keep_for_retry(&stop, std::iter::once((item.path, target)));
                        }
                        Step::Tidy(folder) => self.remove_if_empty(folder),
                    }
                }
                return Some(stop);
            }
        }
        None
    }

    fn rename(
        &mut self,
        plan: &mut Plan,
        item: Item,
        target: PathBuf,
        merging: bool,
        steps: &mut VecDeque<Step>,
    ) -> Option<Status> {
        let result = match (self.mover)(&item.path, &target) {
            Ok(()) => Ok(Outcome::Done),
            // The paths looked like one volume, but a junction on the way leads to another.
            Err(error) if WIN32_ERROR::from_error(&error) == Some(ERROR_NOT_SAME_DEVICE) => {
                return self.fall_back_to_copying(plan, item, target);
            }
            Err(error) if is_taken(&error) => self.rename_onto(&item, &target, merging, steps),
            Err(error) => Err(JerryError::from(error).at(&item.path)),
        };
        let failure = match result {
            Ok(outcome) => {
                match outcome {
                    Outcome::Done if is_folder(&item) => self.summary.folders += 1,
                    Outcome::Done => self.summary.files += 1,
                    Outcome::Skipped => self.summary.skipped += 1,
                    Outcome::Merged => {}
                }
                self.advance(&item.path, 0);
                return None;
            }
            Err(error) => error,
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

    /// A rename whose name is taken: the conflict, and what it comes to.
    fn rename_onto(
        &mut self,
        item: &Item,
        target: &Path,
        merging: bool,
        steps: &mut VecDeque<Step>,
    ) -> Result<Outcome, JerryError> {
        let existing = match probe(target) {
            Ok(existing) => existing,
            // It went meanwhile: the name is free.
            Err(error) if error.code == ErrorCode::NotFound => {
                return self.move_to(item, target).map(|()| Outcome::Done);
            }
            Err(error) => return Err(error),
        };
        let free = match self.resolve(item, target, &existing, merging)? {
            Resolved::Skip => return Ok(Outcome::Skipped),
            Resolved::Rename(free) => free,
            Resolved::Replace => match self.make_room(item, target)? {
                Room::Made => target.to_owned(),
                Room::Rename(free) => free,
                Room::Skip => return Ok(Outcome::Skipped),
            },
            Resolved::Merge => {
                let inside = children(&item.path)?;
                self.files_total += inside.len() as u64;
                steps.push_front(Step::Tidy(item.clone()));
                for child in inside.into_iter().rev() {
                    let Some(name) = child.path.file_name() else {
                        continue;
                    };
                    let target = target.join(name);
                    steps.push_front(Step::Rename {
                        item: child,
                        target,
                        merging: true,
                    });
                }
                return Ok(Outcome::Merged);
            }
        };
        self.move_to(item, &free).map(|()| Outcome::Done)
    }

    /// Moves `item` to `target` by renaming, which never replaces anything.
    fn move_to(&self, item: &Item, target: &Path) -> Result<(), JerryError> {
        (self.mover)(&item.path, target).map_err(|error| {
            if is_taken(&error) {
                JerryError::new(ErrorCode::InvalidName).at(target)
            } else {
                JerryError::from(error).at(&item.path)
            }
        })
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
                        if is_folder(&item) {
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
        entries: &mut [Entry],
        marks: &mut [Mark],
        stamps: &mut HashMap<usize, Stamp>,
    ) -> Option<Status> {
        for index in 0..entries.len() {
            if entries[index]
                .parent
                .is_some_and(|parent| marks[parent] != Mark::Done)
            {
                // Its folder's failure covers it, and trying that again brings this along.
                marks[index] = Mark::Failed;
                self.count_entry(&entries[index]);
                continue;
            }
            if let Some(stop) = self.gate() {
                return Some(stop);
            }
            let written = if entries[index].is_folder() {
                self.make_folder(index, entries, stamps)
                    .map_err(|error| (error, false))
            } else {
                self.write_file(&entries[index])
            };
            let entry = &entries[index];
            match written {
                Ok(Outcome::Skipped) => {
                    marks[index] = Mark::Skipped;
                    self.summary.skipped += 1;
                    self.count_entry(entry);
                }
                Ok(Outcome::Done | Outcome::Merged) => {
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

    /// Creates the folder of entry `index` and notes what its source looks like before anything
    /// moves out of it. A folder of its name that is there is a conflict: merged into, it stays
    /// as it is; under a free name, everything in the entry goes there too.
    fn make_folder(
        &mut self,
        index: usize,
        entries: &mut [Entry],
        stamps: &mut HashMap<usize, Stamp>,
    ) -> Result<Outcome, JerryError> {
        let entry = &entries[index];
        if let Some(error) = &entry.unreadable {
            return Err(error.clone());
        }
        let source = stamp(&entry.item.path)?;
        // A resumed job's own.
        if entry.merge {
            stamps.insert(index, source);
            return Ok(Outcome::Done);
        }
        match std::fs::create_dir(&entry.target) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let merging = entry.parent.is_some_and(|parent| entries[parent].merge);
                let (item, target) = (entry.item.clone(), entry.target.clone());
                let existing = probe(&target)?;
                match self.resolve(&item, &target, &existing, merging)? {
                    Resolved::Merge => {
                        // Not this job's folder: its dates and attributes stay.
                        entries[index].merge = true;
                        return Ok(Outcome::Done);
                    }
                    Resolved::Rename(free) => {
                        std::fs::create_dir(&free).map_err(|error| folder_error(error, &free))?;
                        retarget(entries, index, free);
                    }
                    Resolved::Replace | Resolved::Skip => {
                        return Err(JerryError::internal().at(&target));
                    }
                }
            }
            Err(error) => return Err(folder_error(error, &entry.target)),
        }
        stamps.insert(index, source);
        self.summary.folders += 1;
        Ok(Outcome::Done)
    }

    /// Writes one file or link. A failure says what went wrong, and whether the copy is complete
    /// anyway (a move whose source couldn't be removed).
    fn write_file(&mut self, entry: &Entry) -> Result<Outcome, (JerryError, bool)> {
        let not_copied = |error| (error, false);
        let mut target = entry.target.clone();
        let mut replace = false;
        if stamp(&target).is_ok() {
            match probe(&target) {
                Ok(existing) => match self
                    .resolve(&entry.item, &target, &existing, false)
                    .map_err(not_copied)?
                {
                    Resolved::Skip => return Ok(Outcome::Skipped),
                    Resolved::Replace => replace = true,
                    Resolved::Rename(free) => target = free,
                    Resolved::Merge => return Err(not_copied(JerryError::internal().at(&target))),
                },
                // It went meanwhile: the name is free.
                Err(error) if error.code == ErrorCode::NotFound => {}
                Err(error) => return Err(not_copied(error)),
            }
        }
        let (bytes, source) = if entry.item.link {
            if replace {
                match self.make_room(&entry.item, &target).map_err(not_copied)? {
                    Room::Made => {}
                    Room::Skip => return Ok(Outcome::Skipped),
                    Room::Rename(free) => target = free,
                }
            }
            (self.observer)(Event::Writing(&Writing {
                source: &entry.item.path,
                target: &target,
                part: None,
            }));
            copy_link(&entry.item, &target).map_err(not_copied)?;
            (0, None)
        } else {
            let written = self.copy_entry(entry, &target).map_err(not_copied)?;
            if self.place(&entry.item, &written, target, replace)? == Outcome::Skipped {
                return Ok(Outcome::Skipped);
            }
            (written.source.size, Some(written.source))
        };
        if self.kind == Kind::Move {
            // Only now: the copy is complete, on the disk, and has been checked. If the user was
            // asked meanwhile, the source may have changed since.
            if let Some(source) = &source {
                check_unchanged(&entry.item.path, source).map_err(|error| (error, true))?;
            }
            patiently(|| remove(&entry.item)).map_err(|error| (error, true))?;
        }
        self.summary.files += 1;
        self.summary.bytes += bytes;
        Ok(Outcome::Done)
    }

    /// Gives a finished copy its name, `target`, making room first if it replaces what is there.
    /// The temporary file goes if it doesn't get a name.
    fn place(
        &mut self,
        item: &Item,
        written: &Written,
        mut target: PathBuf,
        replace: bool,
    ) -> Result<Outcome, (JerryError, bool)> {
        if replace {
            let room = self
                .make_room(item, &target)
                .inspect_err(|_| discard(&written.part));
            match room.map_err(|error| (error, false))? {
                Room::Made => {}
                Room::Rename(free) => target = free,
                Room::Skip => {
                    discard(&written.part);
                    return Ok(Outcome::Skipped);
                }
            }
        }
        take_name(&written.part, &target).map_err(|error| {
            discard(&written.part);
            (error, false)
        })?;
        Ok(Outcome::Done)
    }

    fn copy_entry(&mut self, entry: &Entry, target: &Path) -> Result<Written, JerryError> {
        let control = self.control;
        let size = entry.item.size;
        let file = entry.item.path.as_path();
        let request = Request {
            source: file,
            target,
            for_move: self.kind == Kind::Move,
        };
        let copier = self.copier;
        let mut last = 0_u64;
        copier(&request, &mut |step| {
            let transferred = match step {
                CopyStep::Started(part) => {
                    (self.observer)(Event::Writing(&Writing {
                        source: file,
                        target,
                        part: Some(part),
                    }));
                    return Flow::Go;
                }
                CopyStep::Copied(transferred) => transferred,
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
        self.summary.policy = self.policy;
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

    /// A source folder goes once it is empty: after a retried move, or a merge by renaming.
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

/// A folder to create and fill; a link to a folder is not one.
fn is_folder(item: &Item) -> bool {
    item.is_dir() && !item.link
}

/// One side of a conflict, for the window.
fn side(path: &Path, item: &Item) -> Side {
    Side {
        folder: is_folder(item),
        attributes: item.attributes,
        size: item.size,
        modified: stamp(path).map_or(0, |stamp| ticks(&stamp.written)),
    }
}

/// Whether the file at `source` was modified later than the one at `target`. If either can't
/// say, it isn't: what is there stays.
fn newer(source: &Path, target: &Path) -> bool {
    match (stamp(source), stamp(target)) {
        (Ok(source), Ok(target)) => ticks(&source.written) > ticks(&target.written),
        _ => false,
    }
}

fn ticks(time: &FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// A folder that couldn't be created at `target`.
fn folder_error(error: io::Error, target: &Path) -> JerryError {
    let code = if error.kind() == io::ErrorKind::AlreadyExists {
        JerryError::new(ErrorCode::InvalidName)
    } else {
        JerryError::from(error)
    };
    code.at(target)
}

/// Gives the folder of entry `index` the target `free` instead, and everything in it the same
/// place in there. The entries of a folder follow it directly, its subfolders' included, so the
/// first one after it whose folder came before it is past its end.
fn retarget(entries: &mut [Entry], index: usize, free: PathBuf) {
    let old = std::mem::replace(&mut entries[index].target, free.clone());
    for entry in &mut entries[index + 1..] {
        if entry.parent.is_none_or(|parent| parent < index) {
            break;
        }
        if let Ok(rest) = entry.target.strip_prefix(&old) {
            entry.target = free.join(rest);
        }
    }
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
    if is_folder(item) {
        if !children(&item.path)?.is_empty() {
            return Err(JerryError::from_win32(ERROR_DIR_NOT_EMPTY.0).at(&item.path));
        }
    } else if !item.link && !stamp(&item.path)?.same_content(&copy) {
        return Err(JerryError::new(ErrorCode::InUse { app: None }).at(&item.path));
    }
    remove(item)
}
