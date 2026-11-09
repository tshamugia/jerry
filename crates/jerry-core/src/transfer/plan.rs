//! What a job will do, worked out before it touches anything: the checks that can stop it from
//! starting, and the list of what to create at the destination.

use std::path::{Path, PathBuf};
use std::time::Instant;

use super::control::{Control, Flow};
use super::{Event, Failure, Job, Kind, Options, Summary};
use crate::delete::{Item, MEASURE_PROGRESS, Tally, children, is_within, probe};
use crate::error::{ErrorCode, JerryError};
use crate::space::folder_space;
use crate::volume::{self, same_root};

/// One thing to create at the destination.
#[derive(Debug)]
pub(super) struct Entry {
    pub(super) item: Item,
    pub(super) target: PathBuf,
    /// The entry of the folder it is in, always earlier in the list. The items of a folder follow
    /// it directly, its subfolders' items included (depth first).
    pub(super) parent: Option<usize>,
    /// Why a folder couldn't be read.
    pub(super) unreadable: Option<JerryError>,
}

impl Entry {
    /// A folder to create and fill; a link to a folder is not one.
    pub(super) fn is_folder(&self) -> bool {
        self.item.is_dir() && !self.item.link
    }
}

/// Everything a job will do.
#[derive(Debug)]
pub(super) struct Plan {
    pub(super) kind: Kind,
    /// Items to move by renaming: `(item, target)`.
    pub(super) renames: Vec<(Item, PathBuf)>,
    /// A retried move: sources whose copy is complete at the target and only have to go.
    pub(super) removals: Vec<(Item, PathBuf)>,
    pub(super) entries: Vec<Entry>,
    /// Sources that couldn't even be looked at.
    pub(super) failed: Vec<Failure>,
    /// A retried move: source folders to remove once they are empty, deepest first.
    pub(super) leftovers: Vec<Item>,
    /// What `entries` will write; a folder that is renamed counts as nothing.
    pub(super) tally: Tally,
}

impl Plan {
    pub(super) fn new(kind: Kind) -> Self {
        Self {
            kind,
            renames: Vec::new(),
            removals: Vec::new(),
            entries: Vec::new(),
            failed: Vec::new(),
            leftovers: Vec::new(),
            tally: Tally::default(),
        }
    }

    /// Adds `other`'s entries after this plan's. Nothing may have started on the entries yet.
    pub(super) fn absorb(&mut self, other: Self) {
        let offset = self.entries.len();
        self.entries
            .extend(other.entries.into_iter().map(|entry| Entry {
                parent: entry.parent.map(|parent| parent + offset),
                ..entry
            }));
        self.tally.files += other.tally.files;
        self.tally.folders += other.tally.folders;
        self.tally.bytes += other.tally.bytes;
    }

    fn push_leaf(&mut self, item: Item, target: PathBuf, parent: Option<usize>) {
        self.tally.files += 1;
        self.tally.bytes += item.size;
        self.entries.push(Entry {
            item,
            target,
            parent,
            unreadable: None,
        });
    }

    /// Adds `item` and, for a folder, everything in it. Links are never entered. A folder that
    /// can't be read stays on the list, and fails when its turn comes. Fails only with
    /// [`ErrorCode::Cancelled`].
    pub(super) fn walk(
        &mut self,
        item: Item,
        target: PathBuf,
        counter: &mut Counter<'_, '_>,
    ) -> Result<(), JerryError> {
        if !item.is_dir() || item.link {
            self.push_leaf(item, target, None);
            return Ok(());
        }
        // A folder's entry goes on the list when it comes off this stack, so the items of a
        // folder are together: when a job stops, only the folders around the current item are
        // still being filled.
        let mut folders = vec![(item, target, None)];
        while let Some((folder, target, parent)) = folders.pop() {
            counter.checkpoint()?;
            let index = self.entries.len();
            let listing = children(&folder.path);
            self.tally.folders += 1;
            let unreadable = listing.as_ref().err().cloned();
            self.entries.push(Entry {
                item: folder,
                target: target.clone(),
                parent,
                unreadable,
            });
            for child in listing.unwrap_or_default() {
                let Some(name) = child.path.file_name() else {
                    continue;
                };
                let child_target = target.join(name);
                if child.is_dir() && !child.link {
                    folders.push((child, child_target, Some(index)));
                } else {
                    self.push_leaf(child, child_target, Some(index));
                }
            }
            counter.tick(&self.tally);
        }
        Ok(())
    }
}

/// Reports counting as it goes, and stops it when the job is cancelled.
pub(super) struct Counter<'a, 'o> {
    control: &'a Control,
    observer: &'a mut (dyn FnMut(Event<'_>) + 'o),
    reported: Instant,
}

impl<'a, 'o> Counter<'a, 'o> {
    pub(super) fn new(control: &'a Control, observer: &'a mut (dyn FnMut(Event<'_>) + 'o)) -> Self {
        Self {
            control,
            observer,
            reported: Instant::now(),
        }
    }

    fn checkpoint(&self) -> Result<(), JerryError> {
        match self.control.checkpoint() {
            Flow::Cancelled => Err(JerryError::new(ErrorCode::Cancelled)),
            Flow::Go | Flow::Resumed => Ok(()),
        }
    }

    fn tick(&mut self, tally: &Tally) {
        if self.reported.elapsed() >= MEASURE_PROGRESS {
            self.reported = Instant::now();
            (self.observer)(Event::Counted(tally));
        }
    }
}

/// The plan for `job`. Fails if the job can't start.
pub(super) fn build(
    job: &Job,
    options: Options,
    control: &Control,
    observer: &mut dyn FnMut(Event<'_>),
) -> Result<Plan, JerryError> {
    let destination = real_folder(&job.destination)?;
    let mut plan = Plan::new(job.kind);
    // All of them first: a job that is blocked must not have counted for nothing.
    let mut items = Vec::with_capacity(job.sources.len());
    for source in &job.sources {
        let target = source.file_name().map_or_else(
            || job.destination.clone(),
            |name| job.destination.join(name),
        );
        match probe(source) {
            Ok(item) => {
                check_not_into_itself(&item, &destination)?;
                items.push((item, target));
            }
            Err(error) => plan.failed.push(Failure {
                source: source.clone(),
                target,
                error,
                copied: false,
            }),
        }
    }
    let root = volume::root(&job.destination).ok();
    let mut counter = Counter::new(control, observer);
    for (item, target) in items {
        add(
            &mut plan,
            item,
            target,
            options,
            root.as_deref(),
            &mut counter,
        )?;
    }
    Ok(plan)
}

/// The plan to try again what `summary` didn't finish.
pub(super) fn for_retry(
    summary: &Summary,
    control: &Control,
    observer: &mut dyn FnMut(Event<'_>),
) -> Result<Plan, JerryError> {
    let mut plan = Plan::new(summary.kind);
    plan.leftovers.clone_from(&summary.leftovers);
    let root = volume::root(&summary.destination).ok();
    let mut counter = Counter::new(control, observer);
    let again = summary
        .failed
        .iter()
        .map(|failure| (&failure.source, &failure.target, failure.copied))
        .chain(
            summary
                .pending
                .iter()
                .map(|(source, target)| (source, target, false)),
        );
    for (source, target, copied) in again {
        match probe(source) {
            Ok(item) if copied => plan.removals.push((item, target.clone())),
            Ok(item) => add(
                &mut plan,
                item,
                target.clone(),
                summary.options,
                root.as_deref(),
                &mut counter,
            )?,
            // The source that only had to go has gone.
            Err(_) if copied => {}
            Err(error) => plan.failed.push(Failure {
                source: source.clone(),
                target: target.clone(),
                error,
                copied: false,
            }),
        }
    }
    Ok(plan)
}

/// Puts one source on the plan: as a rename if that is how it moves, else with everything in it.
fn add(
    plan: &mut Plan,
    item: Item,
    target: PathBuf,
    options: Options,
    root: Option<&Path>,
    counter: &mut Counter<'_, '_>,
) -> Result<(), JerryError> {
    if plan.kind == Kind::Move && options.rename && same_volume(&item.path, root) {
        // Not counted: moving within a volume is instant, whatever the folder holds.
        plan.renames.push((item, target));
        Ok(())
    } else {
        plan.walk(item, target, counter)
    }
}

fn same_volume(path: &Path, root: Option<&Path>) -> bool {
    let (Some(root), Ok(own)) = (root, volume::root(path)) else {
        return false;
    };
    same_root(&own, root)
}

/// Where `path` really is (`GetFinalPathNameByHandleW`, through `canonicalize`): junctions and
/// symbolic links resolved. Fails if it isn't an existing folder.
fn real_folder(path: &Path) -> Result<PathBuf, JerryError> {
    let metadata = std::fs::metadata(path).map_err(|error| JerryError::from(error).at(path))?;
    if !metadata.is_dir() {
        return Err(JerryError::new(ErrorCode::NotFound).at(path));
    }
    std::fs::canonicalize(path).map_err(|error| JerryError::from(error).at(path))
}

/// Blocks a folder going into itself or a folder inside it, also when `destination` gets there
/// through a junction. A link is safe: only the link is copied, never what it points to.
fn check_not_into_itself(item: &Item, destination: &Path) -> Result<(), JerryError> {
    if !item.is_dir() || item.link {
        return Ok(());
    }
    let real = std::fs::canonicalize(&item.path).unwrap_or_else(|_| item.path.clone());
    if is_within(destination, &real) {
        return Err(JerryError::new(ErrorCode::IntoItself).at(&item.path));
    }
    Ok(())
}

/// Fails with [`ErrorCode::NoSpace`] if `needed` bytes don't fit on the volume of `destination`.
/// A volume that can't say how much is free doesn't block anything.
pub(super) fn check_space(destination: &Path, needed: u64) -> Result<(), JerryError> {
    if needed == 0 {
        return Ok(());
    }
    let available = folder_space(destination).map(|space| space.free as u64);
    fits(needed, available).map_err(|code| JerryError::new(code).at(destination))
}

fn fits(needed: u64, available: Option<u64>) -> Result<(), ErrorCode> {
    match available {
        Some(available) if needed > available => Err(ErrorCode::NoSpace {
            needed: Some(needed),
            available: Some(available),
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_that_does_not_fit_says_what_it_needs() {
        assert_eq!(
            fits(10, Some(9)),
            Err(ErrorCode::NoSpace {
                needed: Some(10),
                available: Some(9),
            })
        );
        assert_eq!(fits(10, Some(10)), Ok(()));
    }

    #[test]
    fn an_unknown_volume_blocks_nothing() {
        assert_eq!(fits(u64::MAX, None), Ok(()));
    }
}
