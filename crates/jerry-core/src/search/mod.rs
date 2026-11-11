//! Deep search (Tech PRD «ძებნა», PRD «ღრმა ძებნა»): a folder and everything in it, by name,
//! type, size, date and kind, with results streamed while the walk goes on.
//!
//! The walker reads each folder with `FindFirstFileExW` (`FindExInfoBasic`, large fetch), as the
//! listing does: name, size, dates and attributes come with the directory read, no file is
//! opened, so a OneDrive file that lives only in the cloud is never downloaded. Every folder is
//! one task on a rayon pool of the search's own: a worker that runs out of folders steals
//! another's, so wide and deep trees both keep every worker busy, and a search on a slow share
//! never holds up the sort pool.
//!
//! - Links (symbolic links, junctions, mount points) can match but aren't entered, unless the
//!   query says so. Then every folder entered is noted by volume and file ID, and one seen
//!   before is skipped: a link back up the tree can't loop, and two ways into one folder find
//!   its files once. OneDrive's folders are reparse points too, but not links: they are entered.
//! - Hidden items are skipped, and hidden folders not entered, unless the query wants them.
//! - The folders [`search`] is told to leave out (Windows, Program Files, ProgramData: they come
//!   from the Shell, in `jerry-win`) aren't entered, unless the search starts inside one.
//! - A folder that can't be read (no permission, gone meanwhile) is counted as skipped; only the
//!   folder the search starts in failing fails the search.
//!
//! Results come in batches: the first as soon as anything is found, then one at most every
//! [`BATCH_INTERVAL`], with how many items were found and checked so far. Setting `cancel` stops
//! the search: no batch follows. Everything here is blocking: run it on a background thread.

mod query;
mod saved;
mod walk;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub use query::{DateRange, FileType, SearchItems, SearchQuery, SizeRange};
pub use saved::{DateFilter, SavedSearch};

use crate::error::JerryError;
use crate::listing::Entries;

pub use crate::listing::BATCH_INTERVAL;

/// What a search found since its last batch.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SearchBatch {
    /// The id of the first result here: results are numbered from 0 in the order found, and a
    /// batch's continue the last one's.
    pub first: usize,
    /// The results, as a listing's entries: result `first + i` is entry `i`.
    pub entries: Entries,
    /// The folder each result is in, by folder id (`folders_of[i]` for entry `i`). Folder ids
    /// are numbered from 0 in the order they first had a result.
    pub folders_of: Vec<u32>,
    /// The id of the first folder in `folders`.
    pub first_folder: usize,
    /// Folders named for the first time in this batch, as `\\?\` long paths: folder
    /// `first_folder + i` is `folders[i]`.
    pub folders: Vec<PathBuf>,
}

impl SearchBatch {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// How far a search is.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SearchProgress {
    /// Results so far.
    pub found: u64,
    /// Files and folders read so far, matching or not.
    pub checked: u64,
    /// Folders that couldn't be read: no permission, or gone meanwhile.
    pub skipped: u64,
}

/// Searches `root` (a folder) and everything in it for what `query` wants, and hands the results
/// to `on_batch` with the progress so far: the first batch as soon as anything is found, then one
/// at most every [`BATCH_INTERVAL`] (with no results, when there are none new). Returns the final
/// progress once the walk is done; the last batch came before.
///
/// `excluded` are folders not to enter (`jerry_win::folders::search_excluded`); one the search
/// starts in, or in a folder inside, doesn't count. Pass none for `query.system_folders`.
///
/// Fails with what is wrong with `root` (`NotFound`, `AccessDenied`, `DeviceGone`), or with
/// [`ErrorCode::Cancelled`](crate::ErrorCode::Cancelled) once `cancel` is set: no batch follows.
/// Blocking IO: call it on a background thread.
pub fn search(
    root: &Path,
    query: &SearchQuery,
    excluded: &[PathBuf],
    cancel: &AtomicBool,
    on_batch: impl FnMut(&SearchBatch, SearchProgress),
) -> Result<SearchProgress, JerryError> {
    walk::run(root, query, excluded, cancel, on_batch)
}
