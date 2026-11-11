//! Deep search for the UI (#65, #66): `search_start` searches a folder and everything in it, or
//! its whole drive, on a background job (`jerry_core::search`) and streams what it finds through a
//! `Channel` as binary messages (ADR-0001); `search_cancel` stops it. Windows, Program Files and
//! ProgramData are left out unless the query asks for them (`jerry_win::folders::search_excluded`).
//!
//! The results stay for the tab that searched (`crate::results`), under an id of the listings'
//! kind: every command on a listing's entries takes them, `listing_sort` sorts them and
//! `listing_close` forgets them. The search history is here too: the last 20 searches
//! (`search_history`).
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
//! - Done (3): `[3, found, checked, skipped]`, then the `f64` ms the search took and a `u32`, 1 if
//!   it was stopped (`search_cancel`), and 4 zero bytes. The search's last message: diffs and
//!   orders follow while the results are open.
//! - Error (4): `[4, jsonBytes, 0, 0]`, then the `JerryError` as UTF-8 JSON: the folder couldn't be
//!   searched (`NotFound`, `AccessDenied`, `DeviceGone`). The last message.
//! - Order (5): `[5, count, column, flags]`, then `u32 id[count]`: the results there are, in
//!   display order, laid out as a listing's order (column 6 is Location). One comes right before
//!   Done, one for every `listing_sort` (while the walk goes on, of the results found so far: later
//!   ones follow unsorted), and one after every diff once the search is done.
//! - Diff (6): `[6, count, removedCount, nameUnits]`, then the row as in a batch, `u32 id[count]`,
//!   `u32 removed[removedCount]`, `u32 folder[count]`, `u32 folderEnd[folderCount]`, padding to 8
//!   bytes, the `count` results as in a batch and the new folders' texts: results Jerry renamed
//!   (they keep their ids; one in a renamed folder gets the folder's new id) and removed (deleted,
//!   moved away). Removed ids never come back.
//!
//! The UI's decoder is `src/ipc/search.ts`; both sides test this layout.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use jerry_core::search::{SavedSearch, SearchProgress, SearchQuery, search};
use jerry_core::sort::Sort;
use jerry_core::{ErrorCode, JerryError};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::ipc::{Bytes, message_buffer};
use crate::jobs;
use crate::listing::{Crumb, Listings};
use crate::results::ResultSet;
use crate::storage::{Db, now_ms};

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

/// A search started: its results' id and the folder it searches, as a tab shows a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct SearchOpened {
    /// The results' id, a listing's kind of id: for every command on entries, `listing_sort`,
    /// `listing_close`.
    pub id: u32,
    /// The folder searched (a drive's root for a whole drive) as display text.
    pub path: String,
    /// Its name for a tab.
    pub name: String,
    /// Its token, for `ListingTarget::Folder`.
    pub folder: u32,
    /// The address bar's segments, from the root to it.
    pub crumbs: Vec<Crumb>,
}

/// The search commands' state: the folders a search leaves out, from the Shell, read once.
/// Clones share it.
#[derive(Clone, Default)]
pub struct Searches(Arc<OnceLock<Vec<PathBuf>>>);

impl Searches {
    /// Windows, Program Files and ProgramData. A Shell call: on a background job.
    fn excluded(&self) -> &[PathBuf] {
        self.0.get_or_init(jerry_win::folders::search_excluded)
    }
}

/// Searches `within` for `query`, for tab `tab` in place of what it showed, and returns at once,
/// before anything is read, with the results' id and the folder searched. Results, progress and
/// the end come through `on_message` as binary messages (decoded by `src/ipc/search.ts`), sorted
/// by `sort` once the search is done, and so does what is wrong with the folder (`NotFound`,
/// `AccessDenied`, `DeviceGone`). Fails only with `NotFound` for a folder token Jerry never gave.
#[tauri::command]
#[specta::specta]
pub async fn search_start(
    app: AppHandle,
    tab: u32,
    within: SearchIn,
    query: SearchQuery,
    sort: Sort,
    on_message: Channel<Bytes>,
) -> Result<SearchOpened, JerryError> {
    let (SearchIn::Folder { folder } | SearchIn::Drive { folder }) = within;
    let listings = app.state::<Listings>().inner().clone();
    let path = listings
        .folder_path(folder)
        .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
    let root = match within {
        SearchIn::Folder { .. } => path,
        SearchIn::Drive { .. } => drive_root(&path),
    };
    let entry = listings.entry_for(&root);
    let (id, results) = listings.start_results(tab, root.clone(), sort, on_message);
    let searches = app.state::<Searches>().inner().clone();
    tauri::async_runtime::spawn(async move {
        let job = results.clone();
        let outcome = jobs::background("search", move || run(&searches, &root, &query, &job)).await;
        if let Err(error) = outcome {
            results.end(error_message(&error), false);
        }
    });
    Ok(SearchOpened {
        id,
        path: entry.path,
        name: entry.name,
        folder: entry.folder,
        crumbs: entry.crumbs,
    })
}

/// Stops search `id` if it still runs. What it found stays, sorted: an order and the done message
/// follow. Stopping one that has ended does nothing.
#[tauri::command]
#[specta::specta]
pub async fn search_cancel(app: AppHandle, id: u32) {
    if let Some(results) = app.state::<Listings>().results(id) {
        results.stop();
    }
}

/// The last searches, newest first (at most 20).
#[tauri::command]
#[specta::specta]
pub async fn search_history_list(db: State<'_, Db>) -> Result<Vec<SavedSearch>, JerryError> {
    let db = Db::clone(&db);
    jobs::background("search history", move || {
        db.with(|storage| storage.search_history())
    })
    .await
}

/// Puts `search` at the top of the search history, once; past 20, the oldest goes.
#[tauri::command]
#[specta::specta]
pub async fn search_history_add(db: State<'_, Db>, search: SavedSearch) -> Result<(), JerryError> {
    let db = Db::clone(&db);
    let at = now_ms();
    jobs::background("add search", move || {
        db.with(|storage| storage.add_search(&search, at))
    })
    .await
}

/// Takes `search` out of the search history.
#[tauri::command]
#[specta::specta]
pub async fn search_history_remove(
    db: State<'_, Db>,
    search: SavedSearch,
) -> Result<(), JerryError> {
    let db = Db::clone(&db);
    jobs::background("remove search", move || {
        db.with(|storage| storage.remove_search(&search).map(|_| ()))
    })
    .await
}

/// The search job: keeps and streams results and progress, then the order and the done message,
/// also when it was stopped. Fails with what is wrong with `root`.
fn run(
    searches: &Searches,
    root: &Path,
    query: &SearchQuery,
    results: &ResultSet,
) -> Result<(), JerryError> {
    let started = Instant::now();
    let excluded = if query.system_folders {
        &[][..]
    } else {
        searches.excluded()
    };
    let cancel = results.cancel_flag();
    let mut told = SearchProgress::default();
    let outcome = search(root, query, excluded, &cancel, |batch, progress| {
        if !batch.is_empty() {
            results.add(batch);
        }
        if progress != told {
            results.send(counts_message(PROGRESS, progress, &[]));
            told = progress;
        }
    });
    let (progress, stopped) = match outcome {
        Ok(progress) => (progress, false),
        Err(error) if error.code == ErrorCode::Cancelled => (
            SearchProgress {
                found: results.count() as u64,
                ..told
            },
            true,
        ),
        Err(error) => return Err(error),
    };
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    tracing::info!(
        found = progress.found,
        checked = progress.checked,
        skipped = progress.skipped,
        stopped,
        ms,
        "search ended"
    );
    let mut body = Vec::with_capacity(16);
    body.extend_from_slice(&ms.to_le_bytes());
    body.extend_from_slice(&u32::from(stopped).to_le_bytes());
    body.extend_from_slice(&[0; 4]);
    results.end(counts_message(DONE, progress, &body), true);
    Ok(())
}

/// The root of the drive or share `folder` is on: `\\?\C:\`, `\\?\UNC\server\share\`.
fn drive_root(folder: &Path) -> PathBuf {
    folder.ancestors().last().unwrap_or(folder).to_path_buf()
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

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    use jerry_core::sort::SortColumn;

    use super::*;

    fn word(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"))
    }

    fn float(bytes: &[u8], at: usize) -> f64 {
        f64::from_le_bytes(bytes[at..at + 8].try_into().expect("8 bytes"))
    }

    fn test_channel() -> (Channel<Bytes>, Receiver<Vec<u8>>) {
        let (sender, receiver) = channel();
        let channel = Channel::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Raw(bytes) = body {
                let _ = sender.send(bytes);
            }
            Ok(())
        });
        (channel, receiver)
    }

    /// Every message until the done or error message.
    fn until_end(receiver: &Receiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut messages = Vec::new();
        loop {
            let message = receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("a message");
            let kind = word(&message, 0);
            messages.push(message);
            if kind == DONE || kind == ERROR {
                return messages;
            }
        }
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
    fn a_search_streams_its_results_then_their_order_and_done() {
        let dir = tempfile::tempdir().expect("temp folder");
        let deep = dir.path().join("ნახაზები").join("2026");
        std::fs::create_dir_all(&deep).expect("folders");
        for path in [
            dir.path().join("b.dwg"),
            deep.join("a.dwg"),
            deep.join("notes.txt"),
        ] {
            std::fs::write(path, b"x").expect("write");
        }
        let root = jerry_core::path::long_path(dir.path()).expect("long path");
        let (channel, receiver) = test_channel();
        let results = ResultSet::new(1, root.clone(), Sort::default(), channel);
        let query = SearchQuery {
            name: "*.dwg".to_owned(),
            system_folders: true,
            ..SearchQuery::default()
        };

        run(&Searches::default(), &root, &query, &results).expect("searches");
        let messages = until_end(&receiver);
        let kinds: Vec<u32> = messages.iter().map(|message| word(message, 0)).collect();
        assert!(kinds.contains(&1), "a batch");
        assert_eq!(&kinds[kinds.len() - 2..], [5, DONE], "the order, then done");
        let done = messages.last().expect("done");
        assert_eq!(word(done, 4), 2, "found");
        assert_eq!(word(done, 24), 0, "not stopped");
        let order = &messages[messages.len() - 2];
        assert_eq!(word(order, 4), 2);
        assert_eq!(word(order, 8), 0, "by name");

        let paths: Vec<PathBuf> = (0..2)
            .map(|id| results.entry(id).expect("a result").0)
            .collect();
        assert!(paths.contains(&root.join("b.dwg")));
        assert!(paths.contains(&root.join("ნახაზები").join("2026").join("a.dwg")));

        // Sorted by Location once done: the root's b.dwg, then the deeper a.dwg.
        results.sort(Sort {
            column: SortColumn::Location,
            descending: false,
            folders_first: true,
        });
        let order = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("order");
        let ids = [word(&order, 16), word(&order, 20)];
        assert_eq!(
            results.entry(ids[0]).map(|(path, ..)| path),
            Some(root.join("b.dwg"))
        );
    }

    #[test]
    fn a_stopped_search_keeps_what_it_found_sorted() {
        let dir = tempfile::tempdir().expect("temp folder");
        std::fs::write(dir.path().join("a.dwg"), b"x").expect("write");
        let root = jerry_core::path::long_path(dir.path()).expect("long path");
        let (channel, receiver) = test_channel();
        let results = ResultSet::new(1, root.clone(), Sort::default(), channel);
        results.stop();
        let query = SearchQuery {
            system_folders: true,
            ..SearchQuery::default()
        };
        run(&Searches::default(), &root, &query, &results).expect("ends");
        let messages = until_end(&receiver);
        let done = messages.last().expect("done");
        assert_eq!(word(done, 0), DONE);
        assert_eq!(word(done, 24), 1, "stopped");
        assert_eq!(word(&messages[messages.len() - 2], 0), 5, "an order first");
    }

    #[test]
    fn a_folder_that_is_gone_fails_the_search() {
        let dir = tempfile::tempdir().expect("temp folder");
        let gone = jerry_core::path::long_path(&dir.path().join("gone")).expect("long path");
        let (channel, _receiver) = test_channel();
        let results = ResultSet::new(1, gone.clone(), Sort::default(), channel);
        let error = run(
            &Searches::default(),
            &gone,
            &SearchQuery::default(),
            &results,
        )
        .expect_err("no folder");
        assert_eq!(error.code, ErrorCode::NotFound);
    }
}
