//! Deep search for the UI (#65): `search_start` searches a folder and everything in it, or its
//! whole drive, on a background job (`jerry_core::search`) and streams what it finds through a
//! `Channel` as binary messages (ADR-0001); `search_cancel` stops it. Windows, Program Files and
//! ProgramData are left out unless the query asks for them (`jerry_win::folders::search_excluded`).
//! The results tab is #66.
//!
//! Messages are little-endian and start with the 16-byte header `[kind, a, b, c]`:
//! - Batch (1): `[1, count, firstId, nameUnits]`, then the row `[folderCount, firstFolder,
//!   folderUnits, 0]`, then `u32 folder[count]` (each result's folder id), `u32
//!   folderEnd[folderCount]` (where each new folder's text ends in this message's folder texts,
//!   in UTF-16 units), 4 zero bytes if `count + folderCount` is odd (so the `f64`s stay aligned),
//!   then the results laid out exactly as a listing's batch (`src-tauri/src/listing.rs`: sizes,
//!   modified, created, attributes, icon keys, name ends, names), and last the new folders as
//!   display text (no `\\?\`), UTF-16. Result ids are `firstId + i` and continue the last batch's;
//!   new folder ids are `firstFolder + i`. A result's folder came in this batch or an earlier one.
//! - Progress (2): `[2, found, checked, skipped]`: results so far, files and folders read, and
//!   folders that couldn't be read. After each batch, and every 50 ms while it changes.
//! - Done (3): `[3, found, checked, skipped]`, then the `f64` ms the search took: the last
//!   message.
//! - Error (4): `[4, jsonBytes, 0, 0]`, then the `JerryError` as UTF-8 JSON: the folder couldn't be
//!   searched (`NotFound`, `AccessDenied`, `DeviceGone`). The last message.
//!
//! After `search_cancel` nothing more comes. The UI's decoder is `src/ipc/search.ts`; both sides
//! test this layout.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

use jerry_core::error::display_path;
use jerry_core::search::{SearchBatch, SearchProgress, SearchQuery, search};
use jerry_core::{ErrorCode, JerryError};
use serde::Deserialize;
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

use crate::ipc::{Bytes, message_buffer};
use crate::jobs;
use crate::listing::{Listings, entries_bytes, push_entries};

const BATCH: u32 = 1;
const PROGRESS: u32 = 2;
const DONE: u32 = 3;
const ERROR: u32 = 4;

/// Where to search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SearchIn {
    /// Folder token `folder` (`ListingOpened::folder`) and everything in it.
    Folder { folder: u32 },
    /// The whole drive or share folder token `folder` is on, from its root.
    Drive { folder: u32 },
}

/// The searches running, by id, with their cancel flags. Clones share them.
#[derive(Clone, Default)]
pub struct Searches(Arc<Inner>);

#[derive(Default)]
struct Inner {
    last_id: AtomicU32,
    running: Mutex<HashMap<u32, Arc<AtomicBool>>>,
    /// The folders a search leaves out, from the Shell, read once.
    excluded: OnceLock<Vec<PathBuf>>,
}

impl Searches {
    /// A new search: its id and its cancel flag.
    fn start(&self) -> (u32, Arc<AtomicBool>) {
        let id = self.0.last_id.fetch_add(1, Ordering::Relaxed) + 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.running().insert(id, cancel.clone());
        (id, cancel)
    }

    fn finish(&self, id: u32) {
        self.running().remove(&id);
    }

    fn cancel(&self, id: u32) {
        if let Some(cancel) = self.running().remove(&id) {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Windows, Program Files and ProgramData. A Shell call: on a background job.
    fn excluded(&self) -> &[PathBuf] {
        self.0
            .excluded
            .get_or_init(jerry_win::folders::search_excluded)
    }

    fn running(&self) -> MutexGuard<'_, HashMap<u32, Arc<AtomicBool>>> {
        self.0
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Starts searching `within` for `query` and returns the search's id at once, before anything is
/// read. Results, progress and the end come through `on_message` as binary messages (decoded by
/// `src/ipc/search.ts`), and so does what is wrong with the folder (`NotFound`, `AccessDenied`,
/// `DeviceGone`). Fails only with `NotFound` for a folder token Jerry never gave.
#[tauri::command]
#[specta::specta]
pub async fn search_start(
    app: AppHandle,
    within: SearchIn,
    query: SearchQuery,
    on_message: Channel<Bytes>,
) -> Result<u32, JerryError> {
    let (SearchIn::Folder { folder } | SearchIn::Drive { folder }) = within;
    let path = app
        .state::<Listings>()
        .folder_path(folder)
        .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
    let root = match within {
        SearchIn::Folder { .. } => path,
        SearchIn::Drive { .. } => drive_root(&path),
    };
    let searches = app.state::<Searches>().inner().clone();
    let (id, cancel) = searches.start();
    tauri::async_runtime::spawn(async move {
        let (job, job_cancel, channel) = (searches.clone(), cancel.clone(), on_message.clone());
        let result = jobs::background("search", move || {
            run(&job, &root, &query, &job_cancel, &channel)
        })
        .await;
        searches.finish(id);
        if let Err(error) = result
            && error.code != ErrorCode::Cancelled
            && !cancel.load(Ordering::Relaxed)
        {
            deliver(&on_message, error_message(&error));
        }
    });
    Ok(id)
}

/// Stops search `id` if it still runs: nothing more comes through its stream. Stopping one that
/// has ended does nothing.
#[tauri::command]
#[specta::specta]
pub async fn search_cancel(app: AppHandle, id: u32) {
    app.state::<Searches>().cancel(id);
}

/// The search job: streams batches and progress, then the done message.
fn run(
    searches: &Searches,
    root: &Path,
    query: &SearchQuery,
    cancel: &AtomicBool,
    channel: &Channel<Bytes>,
) -> Result<(), JerryError> {
    let started = Instant::now();
    let excluded = if query.system_folders {
        &[][..]
    } else {
        searches.excluded()
    };
    let mut told = SearchProgress::default();
    let progress = search(root, query, excluded, cancel, |batch, progress| {
        if !batch.is_empty() {
            deliver(channel, batch_message(batch));
        }
        if progress != told {
            deliver(channel, counts_message(PROGRESS, progress, &[]));
            told = progress;
        }
    })?;
    if !cancel.load(Ordering::Relaxed) {
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        deliver(channel, counts_message(DONE, progress, &ms.to_le_bytes()));
    }
    Ok(())
}

/// The root of the drive or share `folder` is on: `\\?\C:\`, `\\?\UNC\server\share\`.
fn drive_root(folder: &Path) -> PathBuf {
    folder.ancestors().last().unwrap_or(folder).to_path_buf()
}

/// One batch of results (see the module docs).
fn batch_message(batch: &SearchBatch) -> Bytes {
    let count = batch.entries.len();
    let units = batch.entries.name_units().len();
    let folders: Vec<Vec<u16>> = batch
        .folders
        .iter()
        .map(|folder| display_path(folder).encode_utf16().collect())
        .collect();
    let folder_units: usize = folders.iter().map(Vec::len).sum();
    let words = count + folders.len();
    let padding = words % 2 * 4;
    // A batch holds at most `MAX_BATCH` results, and a search fewer than 2^32: these fit.
    let header = [BATCH, count as u32, batch.first as u32, units as u32];
    let mut out = message_buffer(
        header,
        16 + words * 4 + padding + entries_bytes(count, units) + folder_units * 2,
    );
    let row = [
        folders.len() as u32,
        batch.first_folder as u32,
        folder_units as u32,
        0,
    ];
    for word in row.into_iter().chain(batch.folders_of.iter().copied()) {
        out.extend_from_slice(&word.to_le_bytes());
    }
    let mut end = 0;
    for folder in &folders {
        end += folder.len();
        out.extend_from_slice(&(end as u32).to_le_bytes());
    }
    out.resize(out.len() + padding, 0);
    push_entries(&mut out, &batch.entries);
    for unit in folders.iter().flatten() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    Bytes(out)
}

/// A progress or done message: the counts in the header, then `body`.
fn counts_message(kind: u32, progress: SearchProgress, body: &[u8]) -> Bytes {
    let count = |value: u64| u32::try_from(value).unwrap_or(u32::MAX);
    let header = [
        kind,
        count(progress.found),
        count(progress.checked),
        count(progress.skipped),
    ];
    let mut out = message_buffer(header, body.len());
    out.extend_from_slice(body);
    Bytes(out)
}

fn error_message(error: &JerryError) -> Bytes {
    let json = serde_json::to_vec(error).unwrap_or_else(|serialize_error| {
        tracing::error!(%serialize_error, "search error didn't serialize");
        br#"{"code":"Internal","path":null}"#.to_vec()
    });
    let mut out = message_buffer([ERROR, json.len() as u32, 0, 0], json.len());
    out.extend_from_slice(&json);
    Bytes(out)
}

fn deliver(channel: &Channel<Bytes>, message: Bytes) {
    if let Err(error) = channel.send(message) {
        tracing::warn!(%error, "search message not delivered");
    }
}

#[cfg(test)]
mod tests {
    use jerry_core::listing::Entries;

    use super::*;

    fn word(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"))
    }

    fn float(bytes: &[u8], at: usize) -> f64 {
        f64::from_le_bytes(bytes[at..at + 8].try_into().expect("8 bytes"))
    }

    fn utf16(bytes: &[u8]) -> String {
        let (pairs, rest) = bytes.as_chunks::<2>();
        assert!(rest.is_empty());
        String::from_utf16_lossy(
            &pairs
                .iter()
                .map(|&pair| u16::from_le_bytes(pair))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn batches_carry_results_their_folder_ids_and_new_folders() {
        let mut entries = Entries::default();
        for (name, size) in [("ნახაზი.dwg", 7_u64), ("a.pdf", 1 << 40), ("b", 0)] {
            let units: Vec<u16> = name.encode_utf16().collect();
            entries.push(
                &units,
                size,
                116_444_736_000_000_000,
                134_116_992_000_000_000,
                0x20,
            );
        }
        let batch = SearchBatch {
            first: 10,
            entries,
            folders_of: vec![3, 4, 3],
            first_folder: 4,
            folders: vec![PathBuf::from(r"\\?\C:\Work\ახალი")],
        };

        let Bytes(bytes) = batch_message(&batch);

        assert_eq!(
            (0..4).map(|i| word(&bytes, i * 4)).collect::<Vec<_>>(),
            [BATCH, 3, 10, 10 + 5 + 1]
        );
        let folder_units = r"C:\Work\ახალი".encode_utf16().count() as u32;
        assert_eq!(
            (4..8).map(|i| word(&bytes, i * 4)).collect::<Vec<_>>(),
            [1, 4, folder_units, 0]
        );
        assert_eq!(
            [word(&bytes, 32), word(&bytes, 36), word(&bytes, 40)],
            [3, 4, 3]
        );
        assert_eq!(word(&bytes, 44), folder_units, "the new folder's end");
        // 3 + 1 words: no padding, so the entries start at 48, on 8 bytes.
        let entries_at = 48;
        assert_eq!(
            [float(&bytes, entries_at), float(&bytes, entries_at + 8)],
            [7.0, (1_u64 << 40) as f64]
        );
        assert_eq!(
            float(&bytes, entries_at + 24),
            1_767_225_600_000.0,
            "modified"
        );
        let ends_at = entries_at + 3 * 24 + 3 * 4 * 2;
        assert_eq!(
            [
                word(&bytes, ends_at),
                word(&bytes, ends_at + 4),
                word(&bytes, ends_at + 8)
            ],
            [10, 15, 16]
        );
        let names_at = ends_at + 12;
        assert_eq!(utf16(&bytes[names_at..names_at + 32]), "ნახაზი.dwga.pdfb");
        assert_eq!(utf16(&bytes[names_at + 32..]), r"C:\Work\ახალი");
    }

    #[test]
    fn batches_pad_after_an_odd_number_of_words() {
        let mut entries = Entries::default();
        entries.push(&[u16::from(b'x')], 1, 0, 0, 0x20);
        entries.push(&[u16::from(b'y')], 2, 0, 0, 0x20);
        let batch = SearchBatch {
            first: 0,
            entries,
            folders_of: vec![0, 0],
            first_folder: 0,
            folders: vec![PathBuf::from(r"\\?\UNC\server\share")],
        };
        let Bytes(bytes) = batch_message(&batch);
        // 2 + 1 words, then 4 bytes of padding: the sizes start at 48.
        assert_eq!(word(&bytes, 44), 0);
        assert_eq!([float(&bytes, 48), float(&bytes, 56)], [1.0, 2.0]);
        assert!(utf16(&bytes[bytes.len() - 2 * 14..]).ends_with(r"\\server\share"));
    }

    #[test]
    fn progress_and_done_carry_the_counts() {
        let progress = SearchProgress {
            found: 48,
            checked: 5_000_000_000,
            skipped: 2,
        };
        let Bytes(bytes) = counts_message(DONE, progress, &12.5_f64.to_le_bytes());
        assert_eq!(
            (0..4).map(|i| word(&bytes, i * 4)).collect::<Vec<_>>(),
            [DONE, 48, u32::MAX, 2],
            "counts past u32 saturate"
        );
        assert_eq!(float(&bytes, 16), 12.5);
    }

    #[test]
    fn errors_are_json() {
        let error = JerryError::new(ErrorCode::AccessDenied).at(Path::new(r"\\?\C:\Secret"));
        let Bytes(bytes) = error_message(&error);
        assert_eq!(word(&bytes, 0), ERROR);
        let json: serde_json::Value =
            serde_json::from_slice(&bytes[16..]).expect("JSON after the header");
        assert_eq!(json["code"], "AccessDenied");
        assert_eq!(json["path"], r"C:\Secret");
    }

    #[test]
    fn drives_search_from_their_root() {
        assert_eq!(
            drive_root(Path::new(r"\\?\C:\Work\ნახაზები")),
            PathBuf::from(r"\\?\C:\")
        );
        assert_eq!(drive_root(Path::new(r"\\?\C:\")), PathBuf::from(r"\\?\C:\"));
        assert_eq!(
            drive_root(Path::new(r"\\?\UNC\server\share\a\b")),
            PathBuf::from(r"\\?\UNC\server\share\")
        );
    }

    #[test]
    fn cancelling_stops_and_forgets_a_search() {
        let searches = Searches::default();
        let (first, cancel) = searches.start();
        let (second, _) = searches.start();
        assert_ne!(first, second);
        searches.cancel(first);
        assert!(cancel.load(Ordering::Relaxed));
        searches.cancel(first);
        searches.finish(second);
        assert!(searches.running().is_empty());
    }
}
