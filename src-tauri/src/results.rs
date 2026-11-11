//! Search results kept for a results tab (#66). A search's results live among the listings
//! (`Listings`), under an id of the same kind, so every command that takes a listing's entries
//! takes them too: open, copy, cut, delete, rename, drag out, icons, Show in Explorer. Unlike a
//! listing, results come from many folders: each has its folder's id, and the folders go to the
//! UI as display text for the Location column.
//!
//! Results don't watch their folders (a whole drive's worth of them): they follow what Jerry
//! itself does to them. A delete or a move removes them, a rename renames them, and inside a
//! renamed folder they get the folder's new path.
//!
//! Messages go through the search's stream (`src-tauri/src/search.rs` has the layout): batches as
//! the walk finds them, the order once it ends or the sort changes, and a diff and an order for
//! every change. Everything is sent with the results locked, so an order only ever names results
//! the UI already has.

use std::collections::{HashMap, HashSet};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use jerry_core::error::display_path;
use jerry_core::listing::Entries;
use jerry_core::search::SearchBatch;
use jerry_core::sort::{PlainTypeNames, Sort, SortColumn, SortIndex, natural_ranks};
use tauri::ipc::Channel;

use crate::ipc::{Bytes, message_buffer};
use crate::listing::{entries_bytes, order_message_of, push_entries};

/// Message kinds (see `src-tauri/src/search.rs`).
pub const BATCH: u32 = 1;
pub const ORDER: u32 = 5;
pub const DIFF: u32 = 6;

/// One search's results, for the tab that shows them.
pub struct ResultSet {
    tab: u32,
    /// The folder searched.
    root: PathBuf,
    /// Set to stop the walk: the Stop button, the tab closing or searching again.
    cancel: Arc<AtomicBool>,
    found: Mutex<Found>,
}

/// The results and where their messages go.
struct Found {
    entries: Entries,
    /// `folder_of[id]`: the id of result `id`'s folder in `folders`.
    folder_of: Vec<u32>,
    /// Every folder a result is in, by id, as `\\?\` long paths. A renamed folder gets a new id;
    /// its old one stays, unused.
    folders: Vec<PathBuf>,
    /// The folders the UI has been told about: `folders[..told]`.
    told: usize,
    /// `removed[id]`: result `id` was deleted or moved away. Shorter than the results when the
    /// last ones weren't.
    removed: Vec<bool>,
    sort: Sort,
    /// Sort keys of the live results, built when an order is needed; `None` once names change.
    index: Option<SortIndex>,
    /// Each folder's rank in natural order, for the Location column; `None` once folders change.
    folder_ranks: Option<Vec<u32>>,
    /// The search has ended (done, stopped or failed): from now on every change sends an order.
    ended: bool,
    /// The UI's stream; `None` once the results are closed.
    channel: Option<Channel<Bytes>>,
}

impl ResultSet {
    /// Results for `tab`, sorted by `sort`, with their messages going to `channel`.
    pub fn new(tab: u32, root: PathBuf, sort: Sort, channel: Channel<Bytes>) -> Self {
        Self {
            tab,
            root,
            cancel: Arc::new(AtomicBool::new(false)),
            found: Mutex::new(Found {
                entries: Entries::default(),
                folder_of: Vec::new(),
                folders: Vec::new(),
                told: 0,
                removed: Vec::new(),
                sort,
                index: None,
                folder_ranks: None,
                ended: false,
                channel: Some(channel),
            }),
        }
    }

    pub fn tab(&self) -> u32 {
        self.tab
    }

    /// The folder searched.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The flag that stops the walk.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    /// Stops the walk; the results stay.
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Stops the walk and sends nothing more: the tab closed, or shows something else.
    pub fn close(&self) {
        self.stop();
        self.found().channel = None;
    }

    /// Keeps a batch the walk found and sends it on.
    pub fn add(&self, batch: &SearchBatch) {
        let mut found = self.found();
        found.entries.append(&batch.entries);
        found.folder_of.extend_from_slice(&batch.folders_of);
        found.folders.extend(batch.folders.iter().cloned());
        found.index = None;
        if !batch.folders.is_empty() {
            found.folder_ranks = None;
        }
        let message = batch_message(batch.first, &batch.entries, &batch.folders_of, &mut found);
        found.send(message);
    }

    /// Sends `message` (progress, an error) in turn with the results'.
    pub fn send(&self, message: Bytes) {
        self.found().send(message);
    }

    /// The search ended: an error message, or the results' order and then `last`, the done
    /// message. Changes send orders from now on.
    pub fn end(&self, last: Bytes, ordered: bool) {
        let mut found = self.found();
        found.ended = true;
        if ordered {
            found.send_order();
        }
        found.send(last);
    }

    /// How many results the search found, removed ones included.
    pub fn count(&self) -> usize {
        self.found().entries.len()
    }

    /// Sorts the results by `sort` from now on: the order of those the UI has goes out now. While
    /// the walk goes on, later results follow it unsorted, until the order at its end.
    pub fn sort(&self, sort: Sort) {
        let mut found = self.found();
        found.sort = sort;
        found.send_order();
    }

    /// Result `id`'s exact path, attributes and last write time (FILETIME), unless it is gone.
    pub fn entry(&self, id: u32) -> Option<(PathBuf, u32, u64)> {
        let found = self.found();
        let id = id as usize;
        let attributes = *found.entries.attributes().get(id)?;
        let modified = *found.entries.modified().get(id)?;
        Some((found.path(id)?, attributes, modified))
    }

    /// The results at `paths` (exact), for the rows Jerry cut (#38).
    pub fn entries_at(&self, paths: &HashSet<PathBuf>) -> Vec<u32> {
        let found = self.found();
        (0..found.entries.len())
            .filter(|&id| found.path(id).is_some_and(|path| paths.contains(&path)))
            .map(|id| id as u32)
            .collect()
    }

    /// Jerry deleted or moved `paths` away: results at them, or in a folder among them, go.
    pub fn removed(&self, paths: &[PathBuf]) {
        let mut found = self.found();
        let gone: Vec<u32> = (0..found.entries.len())
            .filter(|&id| !found.is_removed(id))
            .filter(|&id| {
                let folder = &found.folders[found.folder_of[id] as usize];
                let name = found.entries.name(id);
                paths.iter().any(|path| {
                    folder.starts_with(path)
                        || (path.parent() == Some(folder.as_path())
                            && path.file_name() == Some(name.as_os_str()))
                })
            })
            .map(|id| id as u32)
            .collect();
        if gone.is_empty() {
            return;
        }
        let len = found.entries.len();
        found.removed.resize(len, false);
        for &id in &gone {
            found.removed[id as usize] = true;
        }
        found.index = found.index.take().map(|index| index.without(&gone));
        let message = found.diff_message(&[], &gone);
        found.send(message);
        found.send_order_if_ended();
        tracing::debug!(removed = gone.len(), "search results removed");
    }

    /// Jerry renamed `from` to `to` (same folder): a result at `from` takes the new name, and
    /// results in it, or further in, are now in the folder's new path.
    pub fn renamed(&self, from: &Path, to: &Path) {
        let mut found = self.found();
        let Some(new_name) = to.file_name() else {
            return;
        };
        let new_name: Vec<u16> = new_name.encode_wide().collect();
        let mut changed: Vec<u32> = Vec::new();

        // The result itself.
        let renamed: Vec<usize> = (0..found.entries.len())
            .filter(|&id| !found.is_removed(id))
            .filter(|&id| {
                let folder = &found.folders[found.folder_of[id] as usize];
                from.parent() == Some(folder.as_path())
                    && from.file_name() == Some(found.entries.name(id).as_os_str())
            })
            .collect();
        if !renamed.is_empty() {
            let names: HashMap<usize, &[u16]> = renamed
                .iter()
                .map(|&id| (id, new_name.as_slice()))
                .collect();
            found.entries.rename(&names);
            found.index = None;
            changed.extend(renamed.iter().map(|&id| id as u32));
        }

        // Results inside it: their folders get new ids with the new paths.
        let moved: Vec<(u32, PathBuf)> = found
            .folders
            .iter()
            .enumerate()
            .filter_map(|(folder, path)| {
                let rest = path.strip_prefix(from).ok()?;
                let new = if rest.as_os_str().is_empty() {
                    to.to_path_buf()
                } else {
                    to.join(rest)
                };
                Some((folder as u32, new))
            })
            .collect();
        if !moved.is_empty() {
            let mut new_ids = HashMap::new();
            for (old, path) in moved {
                new_ids.insert(old, found.folders.len() as u32);
                found.folders.push(path);
            }
            found.folder_ranks = None;
            for id in 0..found.entries.len() {
                if found.is_removed(id) {
                    continue;
                }
                if let Some(&new) = new_ids.get(&found.folder_of[id]) {
                    found.folder_of[id] = new;
                    changed.push(id as u32);
                }
            }
        }

        if changed.is_empty() {
            return;
        }
        changed.sort_unstable();
        changed.dedup();
        let message = found.diff_message(&changed, &[]);
        found.send(message);
        found.send_order_if_ended();
        tracing::debug!(changed = changed.len(), "search results renamed");
    }

    fn found(&self) -> MutexGuard<'_, Found> {
        self.found.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Found {
    fn is_removed(&self, id: usize) -> bool {
        self.removed.get(id).copied().unwrap_or(false)
    }

    fn path(&self, id: usize) -> Option<PathBuf> {
        if id >= self.entries.len() || self.is_removed(id) {
            return None;
        }
        let folder = self.folders.get(*self.folder_of.get(id)? as usize)?;
        Some(folder.join(self.entries.name(id)))
    }

    fn send(&self, message: Bytes) {
        if let Some(channel) = &self.channel
            && let Err(error) = channel.send(message)
        {
            tracing::warn!(%error, "search message not delivered");
        }
    }

    fn send_order_if_ended(&mut self) {
        if self.ended {
            self.send_order();
        }
    }

    /// Sends the live results' order by the current sort.
    fn send_order(&mut self) {
        if self.channel.is_none() {
            return;
        }
        let live: Vec<u32> = (0..self.entries.len())
            .filter(|&id| !self.is_removed(id))
            .map(|id| id as u32)
            .collect();
        if self
            .index
            .as_ref()
            .is_none_or(|index| index.len() != live.len())
        {
            self.index = Some(SortIndex::build_for(&self.entries, live, &PlainTypeNames));
        }
        let locations = if self.sort.column == SortColumn::Location {
            let ranks = self.folder_ranks.get_or_insert_with(|| {
                let texts: Vec<Vec<u16>> = self
                    .folders
                    .iter()
                    .map(|folder| display_path(folder).encode_utf16().collect())
                    .collect();
                natural_ranks(&texts)
            });
            self.folder_of
                .iter()
                .map(|&folder| ranks.get(folder as usize).copied().unwrap_or(0))
                .collect()
        } else {
            Vec::new()
        };
        let Some(index) = &self.index else {
            return;
        };
        let order = index.order_with(&self.entries, self.sort, &locations);
        self.send(order_message_of(ORDER, self.sort, &order));
    }

    /// The folders the UI hasn't been told about yet, as display text; they are told now.
    fn new_folders(&mut self) -> (usize, Vec<Vec<u16>>) {
        let first = self.told;
        let texts = self.folders[first..]
            .iter()
            .map(|folder| display_path(folder).encode_utf16().collect())
            .collect();
        self.told = self.folders.len();
        (first, texts)
    }

    /// Results `changed` as they are now and `removed`, as one diff message.
    fn diff_message(&mut self, changed: &[u32], removed: &[u32]) -> Bytes {
        let entries = self.entries.copy_of(changed);
        let folders_of: Vec<u32> = changed
            .iter()
            .map(|&id| self.folder_of[id as usize])
            .collect();
        let (first_folder, folders) = self.new_folders();
        results_message(
            [DIFF, changed.len() as u32, removed.len() as u32, 0],
            &[changed, removed, &folders_of],
            &entries,
            first_folder,
            &folders,
        )
    }
}

/// A batch of results `first..` (see `src-tauri/src/search.rs`).
fn batch_message(first: usize, entries: &Entries, folders_of: &[u32], found: &mut Found) -> Bytes {
    let (first_folder, folders) = found.new_folders();
    // A batch holds at most `MAX_BATCH` results, and a search fewer than 2^32: these fit.
    results_message(
        [BATCH, entries.len() as u32, first as u32, 0],
        &[folders_of],
        entries,
        first_folder,
        &folders,
    )
}

/// A batch or a diff: `header` (its last word becomes the name units), the folder row, `words`
/// one list after another, the new folders' ends, padding to 8 bytes, the entries and last the new
/// folders' texts.
fn results_message(
    mut header: [u32; 4],
    words: &[&[u32]],
    entries: &Entries,
    first_folder: usize,
    folders: &[Vec<u16>],
) -> Bytes {
    let count = entries.len();
    let units = entries.name_units().len();
    header[3] = units as u32;
    let folder_units: usize = folders.iter().map(Vec::len).sum();
    let word_count = words.iter().map(|list| list.len()).sum::<usize>() + folders.len();
    let padding = word_count % 2 * 4;
    let mut out = message_buffer(
        header,
        16 + word_count * 4 + padding + entries_bytes(count, units) + folder_units * 2,
    );
    let row = [
        folders.len() as u32,
        first_folder as u32,
        folder_units as u32,
        0,
    ];
    for word in row
        .into_iter()
        .chain(words.iter().flat_map(|list| list.iter().copied()))
    {
        out.extend_from_slice(&word.to_le_bytes());
    }
    let mut end = 0;
    for folder in folders {
        end += folder.len();
        out.extend_from_slice(&(end as u32).to_le_bytes());
    }
    out.resize(out.len() + padding, 0);
    push_entries(&mut out, entries);
    for unit in folders.iter().flatten() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    Bytes(out)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, channel};

    use super::*;

    fn units(name: &std::ffi::OsStr) -> Vec<u16> {
        name.encode_wide().collect()
    }

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

    fn entries(names: &[&str]) -> Entries {
        let mut entries = Entries::default();
        for (at, name) in names.iter().enumerate() {
            let (name, attributes) = match name.strip_suffix('\\') {
                Some(folder) => (folder, 0x10),
                None => (*name, 0x20),
            };
            let name: Vec<u16> = name.encode_utf16().collect();
            entries.push(&name, at as u64, 0, 116_444_736_000_000_000, attributes);
        }
        entries
    }

    fn batch(first: usize, names: &[&str], folders_of: &[u32], new: &[&str]) -> SearchBatch {
        SearchBatch {
            first,
            entries: entries(names),
            folders_of: folders_of.to_vec(),
            first_folder: 0,
            folders: new.iter().map(PathBuf::from).collect(),
        }
    }

    fn header(message: &[u8]) -> [u32; 4] {
        [0, 1, 2, 3].map(|i| word(message, i * 4))
    }

    fn order_ids(message: &[u8]) -> Vec<u32> {
        assert_eq!(word(message, 0), ORDER);
        (0..word(message, 4) as usize)
            .map(|i| word(message, 16 + i * 4))
            .collect()
    }

    /// Results in two folders: ნახაზი.dwg and B in `\\?\D:\Work`, a.pdf in `\\?\D:\Work\B`.
    fn work() -> (ResultSet, Receiver<Vec<u8>>) {
        let (channel, receiver) = test_channel();
        let results = ResultSet::new(1, PathBuf::from(r"\\?\D:\"), Sort::default(), channel);
        results.add(&batch(
            0,
            &["ნახაზი.dwg", "B\\", "a.pdf"],
            &[0, 0, 1],
            &[r"\\?\D:\Work", r"\\?\D:\Work\B"],
        ));
        let message = receiver.try_recv().expect("the batch");
        assert_eq!(header(&message)[..3], [BATCH, 3, 0]);
        (results, receiver)
    }

    #[test]
    fn batches_carry_results_their_folder_ids_and_new_folders() {
        let (channel, receiver) = test_channel();
        let results = ResultSet::new(1, PathBuf::from(r"\\?\D:\"), Sort::default(), channel);
        results.add(&batch(
            10,
            &["ნახაზი.dwg", "a.pdf", "b"],
            &[0, 1, 0],
            &[r"\\?\C:\Work", r"\\?\C:\Work\ახალი"],
        ));
        let bytes = receiver.try_recv().expect("a batch");
        assert_eq!(header(&bytes), [BATCH, 3, 10, 10 + 5 + 1]);
        let first = r"C:\Work".encode_utf16().count() as u32;
        let units = first + r"C:\Work\ახალი".encode_utf16().count() as u32;
        assert_eq!([4, 5, 6, 7].map(|i| word(&bytes, i * 4)), [2, 0, units, 0]);
        assert_eq!([32, 36, 40].map(|at| word(&bytes, at)), [0, 1, 0]);
        assert_eq!([44, 48].map(|at| word(&bytes, at)), [first, units]);
        // 3 + 2 words: 4 bytes of padding, so the sizes start at 56, on 8 bytes.
        assert_eq!(word(&bytes, 52), 0);
        assert_eq!([float(&bytes, 56), float(&bytes, 64)], [0.0, 1.0]);
        let names_at = 56 + 3 * 24 + 3 * 4 * 3;
        assert_eq!(utf16(&bytes[names_at..names_at + 32]), "ნახაზი.dwga.pdfb");
        assert_eq!(utf16(&bytes[names_at + 32..]), r"C:\WorkC:\Work\ახალი");

        // The next batch names only new folders.
        results.add(&batch(13, &["c"], &[1], &[]));
        let bytes = receiver.try_recv().expect("a batch");
        assert_eq!([4, 5, 6].map(|i| word(&bytes, i * 4)), [0, 2, 0]);
        assert_eq!(results.count(), 4);
    }

    #[test]
    fn results_resolve_to_their_paths() {
        let (results, _receiver) = work();
        let (path, attributes, _) = results.entry(2).expect("a.pdf");
        assert_eq!(path, PathBuf::from(r"\\?\D:\Work\B\a.pdf"));
        assert_eq!(attributes, 0x20);
        assert_eq!(results.entry(1).map(|(_, a, _)| a), Some(0x10));
        assert!(results.entry(3).is_none());
        let cut: HashSet<PathBuf> = [PathBuf::from(r"\\?\D:\Work\ნახაზი.dwg")].into();
        assert_eq!(results.entries_at(&cut), [0]);
    }

    #[test]
    fn the_order_comes_at_the_end_and_with_every_sort() {
        let (results, receiver) = work();
        results.end(Bytes(vec![3, 0, 0, 0]), true);
        assert_eq!(order_ids(&receiver.try_recv().expect("order")), [1, 2, 0]);
        assert_eq!(receiver.try_recv().expect("done")[0], 3);

        results.sort(Sort {
            column: SortColumn::Location,
            descending: false,
            folders_first: false,
        });
        let message = receiver.try_recv().expect("order");
        assert_eq!(header(&message)[2], 6, "the Location column");
        // D:\Work (ნახაზი.dwg, B), then D:\Work\B (a.pdf).
        assert_eq!(order_ids(&message), [1, 0, 2]);
    }

    #[test]
    fn deleted_results_and_those_in_deleted_folders_go() {
        let (results, receiver) = work();
        results.end(Bytes(vec![3, 0, 0, 0]), false);
        let _ = receiver.try_recv();
        results.removed(&[PathBuf::from(r"\\?\D:\Work\B")]);
        let diff = receiver.try_recv().expect("a diff");
        assert_eq!(header(&diff)[..3], [DIFF, 0, 2]);
        assert_eq!([32, 36].map(|at| word(&diff, at)), [1, 2], "B and a.pdf");
        assert_eq!(order_ids(&receiver.try_recv().expect("order")), [0]);
        assert!(results.entry(2).is_none());
        // Nothing else matches: nothing is sent.
        results.removed(&[PathBuf::from(r"\\?\D:\Other")]);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn renamed_results_keep_their_ids_and_results_inside_move_along() {
        let (results, receiver) = work();
        results.renamed(Path::new(r"\\?\D:\Work\B"), Path::new(r"\\?\D:\Work\ბ"));
        let diff = receiver.try_recv().expect("a diff");
        assert_eq!(header(&diff)[..3], [DIFF, 2, 0]);
        // ids 1, 2; no removed; their folders 0 and the new folder 2.
        assert_eq!([32, 36, 40, 44].map(|at| word(&diff, at)), [1, 2, 0, 2]);
        assert_eq!(
            [4, 5].map(|i| word(&diff, i * 4)),
            [1, 2],
            "one new folder, id 2"
        );
        assert!(utf16(&diff[diff.len() - 18..]).ends_with(r"D:\Work\ბ"));
        assert!(
            receiver.try_recv().is_err(),
            "no order while the walk goes on"
        );
        assert_eq!(
            results.entry(2).map(|(path, ..)| path),
            Some(PathBuf::from(r"\\?\D:\Work\ბ\a.pdf"))
        );
        assert_eq!(
            results.entry(1).map(|(path, ..)| path),
            Some(PathBuf::from(r"\\?\D:\Work\ბ"))
        );

        results.renamed(
            Path::new(r"\\?\D:\Work\ნახაზი.dwg"),
            Path::new(r"\\?\D:\Work\ნახაზი v2.dwg"),
        );
        let diff = receiver.try_recv().expect("a diff");
        assert_eq!(header(&diff)[..3], [DIFF, 1, 0]);
        assert_eq!(
            units(
                results
                    .entry(0)
                    .expect("renamed")
                    .0
                    .file_name()
                    .expect("a name")
            ),
            "ნახაზი v2.dwg".encode_utf16().collect::<Vec<_>>()
        );
    }

    #[test]
    fn closed_results_send_nothing() {
        let (results, receiver) = work();
        results.close();
        assert!(results.cancel_flag().load(Ordering::Relaxed));
        results.sort(Sort::default());
        results.removed(&[PathBuf::from(r"\\?\D:\Work\B")]);
        assert!(receiver.try_recv().is_err());
    }
}
