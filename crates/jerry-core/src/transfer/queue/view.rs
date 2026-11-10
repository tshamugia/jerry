//! What the queue tells the UI. Paths are display text only: the UI names jobs by their id.

use serde::Serialize;
use specta::Type;

use crate::error::JerryError;
use crate::transfer::{Choice, Kind, Phase};

/// The queue as the panel shows it, numbered: a later list has a higher generation, so a window
/// that hears of the jobs two ways (`transfer_list` and `transfer:changed`) keeps the newest.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferList {
    pub generation: u32,
    /// In queue order, the jobs that ended among them.
    pub jobs: Vec<TransferJob>,
}

/// One job.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferJob {
    pub id: u32,
    pub kind: Kind,
    pub state: TransferState,
    /// The first source, as display text.
    pub source: String,
    /// How many sources the job has.
    pub sources: u32,
    /// The folder it goes into, as display text.
    pub destination: String,
    pub progress: TransferProgress,
}

/// Where a job is.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TransferState {
    /// Waiting for its disk.
    Queued,
    Running,
    /// Paused by the user: where it was running, it still holds its disk and its current file.
    Paused,
    /// A name it writes is taken: it waits for `conflict_resolve`, or Cancel. Its disk is free
    /// for other jobs meanwhile.
    Conflict {
        conflict: TransferConflict,
    },
    /// Stopped because a volume went away or filled up. `ready`: it is worth trying again (the
    /// volume is back), so Resume does; Cancel ends it either way.
    Waiting {
        error: JerryError,
        ready: bool,
    },
    /// Jerry closed before it finished (it crashed, or was ended). `progress` says how far it
    /// came and `progress.file` what it was writing. Resume carries it on from there; Cancel
    /// discards it, and what it did stays.
    Unfinished,
    /// Everything was attempted. `progress.failed` says how many items didn't make it.
    Done,
    Cancelled,
    /// It couldn't start.
    Failed {
        error: JerryError,
    },
}

/// A name a job writes is taken (#51): what the conflict window shows.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferConflict {
    /// What comes, as display text.
    pub source: String,
    /// Where it goes, where something is already, as display text.
    pub target: String,
    pub incoming: ConflictItem,
    pub existing: ConflictItem,
    /// What can be chosen, in the order to show.
    pub choices: Vec<Choice>,
    /// Replace was chosen, and the file there can't go to the Recycle Bin: Replace now deletes
    /// it for good.
    pub no_recycle_bin: bool,
}

/// One side of a conflict.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConflictItem {
    /// A folder, not a file or a link.
    pub folder: bool,
    /// In bytes; 0 for a folder.
    #[specta(type = f64)]
    pub size: u64,
    /// When it was last modified, in ms since the Unix epoch; null if that can't be read.
    pub modified: Option<f64>,
    /// Its type's icon: `icon://type-<icon>`.
    pub icon: u32,
}

/// How far a job has come. Sums over everything it has done, Resume and Retry included.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub id: u32,
    pub phase: Phase,
    /// The file being worked on, as display text. None before the first one.
    pub file: Option<String>,
    #[specta(type = f64)]
    pub files_done: u64,
    #[specta(type = f64)]
    pub files_total: u64,
    #[specta(type = f64)]
    pub bytes_done: u64,
    #[specta(type = f64)]
    pub bytes_total: u64,
    /// Bytes per second over the last few seconds; 0 until that is known.
    #[specta(type = f64)]
    pub speed: u64,
    /// Items that failed so far.
    pub failed: u32,
}

/// What a job did, once, when it ends.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferEnded {
    pub id: u32,
    pub state: TransferState,
    #[specta(type = f64)]
    pub files: u64,
    #[specta(type = f64)]
    pub folders: u64,
    #[specta(type = f64)]
    pub bytes: u64,
    /// Files and links left where they were because their name was taken (Skip).
    #[specta(type = f64)]
    pub skipped: u64,
    pub failed: Vec<TransferFailure>,
}

/// An item that couldn't be copied or moved. `transfer_retry_failed` tries these again.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferFailure {
    /// The item, as display text.
    pub source: String,
    /// Where it was going, as display text.
    pub target: String,
    pub error: JerryError,
    /// A move: the copy is complete, and only removing the source failed.
    pub copied: bool,
}
