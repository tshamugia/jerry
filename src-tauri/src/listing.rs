//! Folder listings for the UI. `listing_open` streams a folder through a `Channel` as binary
//! messages (ADR-0001) and keeps it live, `listing_sort` re-sorts it and `listing_close` stops
//! and forgets it. Every listing is its own background job, so a slow disk holds up only its own
//! tab. Its folder's watcher (`jerry_core::watcher`, shared by every tab on that folder) starts
//! before the folder is read, so no change is missed; changes that come while it is read are
//! applied right after.
//!
//! Messages are little-endian and start with the 16-byte header `[kind, a, b, c]`:
//! - Batch (1): `[1, count, firstId, nameUnits]`, then the entries: `f64 size[count]` (bytes),
//!   `f64 modified[count]` (ms since the Unix epoch), `u32 attributes[count]`,
//!   `u32 nameEnd[count]` (where each name ends in this message's names, in UTF-16 units) and the
//!   names as UTF-16 code units. The entries' ids are `firstId + i`.
//! - Order (2): `[2, count, column, flags]`, then `u32 id[count]`: the entries the folder has, in
//!   display order, sorted by `column` (0 name, 1 modified, 2 type, 3 size, 4 created,
//!   5 extension), descending if `flags` bit 0 is set, folders first if bit 1 is. One comes for
//!   the listing's sort once the whole folder is in, right before Done, one after every Diff, and
//!   one for every `listing_sort` once the folder is in.
//! - Done (3): `[3, count, batches, 0]`, then the `f64` ms the listing took in Rust: the whole
//!   folder is in. Diffs follow while the listing is open.
//! - Error (4): `[4, jsonBytes, 0, 0]`, then the `JerryError` as UTF-8 JSON: reading the folder
//!   failed, or the folder went away (`NotFound`, `DeviceGone`). Last message.
//! - Diff (5): `[5, count, removedCount, nameUnits]`, then `u32 id[count]`,
//!   `u32 removed[removedCount]`, 4 zero bytes if `count + removedCount` is odd (so the `f64`s
//!   stay aligned), and the `count` entries laid out exactly as in a Batch. Ids are ascending:
//!   one the listing already has is that entry changed (a rename keeps its id), the others are
//!   new and continue the listing's ids in order. Removed ids are gone and never come back.
//!   Changes collect for 100 ms; an Order follows.
//!
//! The UI's decoder is `src/ipc/listing.ts`; both sides test this layout.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Instant;

use jerry_core::error::display_path;
use jerry_core::listing::{Diff, Entries, Listing, filetime_to_unix_ms, list};
use jerry_core::path::long_path;
use jerry_core::sort::{PlainTypeNames, Sort, SortColumn, SortIndex};
use jerry_core::watcher::{Changes, Event, Subscription, Watchers, refresh};
use jerry_core::{ErrorCode, JerryError};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::ipc::{Bytes, message_buffer};
use crate::jobs;

const BATCH: u32 = 1;
const ORDER: u32 = 2;
const DONE: u32 = 3;
const ERROR: u32 = 4;
const DIFF: u32 = 5;

/// The folder to list.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ListingTarget {
    /// A folder the user named: typed in the address bar, a favorite, a drive. Must be absolute.
    Path { path: String },
    /// Subfolder `entry` of the open listing `listing`, opened by its exact path in core, so
    /// names that aren't valid Unicode open too.
    Entry { listing: u32, entry: u32 },
}

/// An opened listing: its id, and its folder as display text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ListingOpened {
    pub id: u32,
    pub path: String,
}

/// The open listings, by id. Clones share them. A tab has at most one: opening another folder
/// for a tab closes the tab's previous listing.
#[derive(Clone, Default)]
pub struct Listings(Arc<Shared>);

#[derive(Default)]
struct Shared {
    last_id: AtomicU32,
    open: Mutex<HashMap<u32, Open>>,
    watchers: Watchers,
}

struct Open {
    tab: u32,
    state: Arc<ListingState>,
}

/// One open listing, shared by its job, its watcher subscription and the commands.
struct ListingState {
    folder: PathBuf,
    listing: RwLock<Listing>,
    live: Mutex<Live>,
    cancel: AtomicBool,
    outbox: Outbox,
    /// The folder's watcher, once the job has subscribed. Its callback holds this state only
    /// weakly, so closing the listing drops both.
    watch: Mutex<Option<Subscription>>,
}

/// What changes as a listing goes on. Every message after the batches is sent under its lock,
/// so diffs and orders go out in the order they happened.
struct Live {
    sort: Sort,
    /// Once the whole folder is in: its sort index. Until then, changes wait in `pending`.
    index: Option<SortIndex>,
    pending: Changes,
    /// After an error message: nothing more goes out.
    ended: bool,
}

impl ListingState {
    fn live(&self) -> MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn watch(&self) -> MutexGuard<'_, Option<Subscription>> {
        self.watch.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Listing> {
        self.listing.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Listing> {
        self.listing.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sends `error` as the last message.
    fn end(&self, live: &mut Live, error: &JerryError) {
        live.ended = true;
        self.outbox.send(error_message(error));
    }

    /// Sends `diff` and the order that follows it, with the sort index brought up to date.
    fn send_diff(&self, live: &mut Live, diff: &Diff) {
        if diff.is_empty() {
            return;
        }
        let started = Instant::now();
        let listing = self.read();
        self.outbox.send(diff_message(&listing, diff));
        let index = match live.index.take() {
            Some(_) if diff.reindex => {
                SortIndex::build_for(listing.entries(), listing.live_ids(), &PlainTypeNames)
            }
            Some(index) if !diff.removed.is_empty() => index.without(&diff.removed),
            Some(index) => index,
            None => return,
        };
        self.outbox.send(order_message(
            live.sort,
            &index.order(listing.entries(), live.sort),
        ));
        live.index = Some(index);
        tracing::debug!(
            entries = listing.len(),
            changed = diff.changed.len(),
            added = diff.added.len(),
            removed = diff.removed.len(),
            ms = started.elapsed().as_secs_f64() * 1000.0,
            "listing diff sent"
        );
    }

    /// Applies what the folder's watcher says, or keeps it for after the read.
    fn on_event(&self, event: Event<'_>) {
        let mut live = self.live();
        if live.ended {
            return;
        }
        if live.index.is_none() {
            // Still reading: look the names up again once the whole folder is in.
            match event {
                Event::Changed { changes, .. } => live.pending.merge(changes),
                Event::Gone(_) => live.pending.lose(),
            }
            return;
        }
        match event {
            Event::Changed { now, .. } => {
                let diff = now.apply_to(&mut self.write());
                self.send_diff(&mut live, &diff);
            }
            Event::Gone(error) => self.end(&mut live, error),
        }
    }
}

impl Listings {
    /// Starts listing `folder` for `tab`, sorted by `sort`, before the UI has a channel for it:
    /// at process start, before Tauri, for the session's folders (spike #11, session restore
    /// #42). Messages are kept until `listing_open` for the same tab, folder and sort attaches
    /// its channel.
    pub fn prestart(&self, tab: u32, folder: &Path, sort: Sort) -> Result<u32, JerryError> {
        Ok(self.start(tab, long_path(folder)?, sort, None))
    }

    /// The folder `target` names, as a long path.
    fn resolve(&self, target: ListingTarget) -> Result<PathBuf, JerryError> {
        match target {
            ListingTarget::Path { path } => {
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    return Err(JerryError::new(ErrorCode::InvalidName).at(path));
                }
                long_path(&path)
            }
            ListingTarget::Entry { listing, entry } => {
                let open = self.open();
                open.get(&listing)
                    .and_then(|open| open.state.read().path(entry as usize))
                    .ok_or_else(|| JerryError::new(ErrorCode::NotFound))
            }
        }
    }

    /// Opens `folder` for `tab`, sorted by `sort`, with its messages going to `channel`: a
    /// listing started for it with [`Listings::prestart`] if there is one, a new one otherwise.
    fn open_folder(&self, tab: u32, folder: PathBuf, sort: Sort, channel: Channel<Bytes>) -> u32 {
        let prestarted = self.open().iter().find_map(|(&id, open)| {
            let waiting = open.tab == tab
                && !open.state.outbox.is_attached()
                && open.state.live().sort == sort
                && open.state.folder == folder;
            waiting.then(|| (id, open.state.outbox.clone()))
        });
        match prestarted {
            Some((id, outbox)) => {
                outbox.attach(channel);
                id
            }
            None => self.start(tab, folder, sort, Some(channel)),
        }
    }

    /// Starts listing `folder` for `tab` on a background job and returns the listing's id.
    fn start(&self, tab: u32, folder: PathBuf, sort: Sort, channel: Option<Channel<Bytes>>) -> u32 {
        self.close_tab(tab);
        let id = self.0.last_id.fetch_add(1, Ordering::Relaxed) + 1;
        let state = Arc::new(ListingState {
            listing: RwLock::new(Listing::new(folder.clone())),
            folder,
            live: Mutex::new(Live {
                sort,
                index: None,
                pending: Changes::default(),
                ended: false,
            }),
            cancel: AtomicBool::new(false),
            outbox: Outbox::new(channel),
            watch: Mutex::new(None),
        });
        self.open().insert(
            id,
            Open {
                tab,
                state: state.clone(),
            },
        );
        let watchers = self.0.watchers.clone();
        tauri::async_runtime::spawn(async move {
            let job_state = state.clone();
            let job = move || run(&job_state, &watchers);
            match jobs::background("listing", job).await {
                Ok(()) => {}
                Err(error) if error.code == ErrorCode::Cancelled => {}
                Err(error) => {
                    tracing::info!(id, code = ?error.code, "listing failed");
                    // Nothing to keep live.
                    state.watch().take();
                    state.end(&mut state.live(), &error);
                }
            }
        });
        id
    }

    /// Sorts listing `id` by `sort` from now on. Once the whole folder is in, its order goes out
    /// through the stream, after any diff before it; while the folder is still being read, the
    /// stream's own Order follows the new sort. CPU work: call it on a background thread.
    fn sort(&self, id: u32, sort: Sort) -> Result<(), JerryError> {
        let state = self
            .open()
            .get(&id)
            .map(|open| open.state.clone())
            .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
        let mut live = state.live();
        live.sort = sort;
        let Some(index) = live.index.as_ref().filter(|_| !live.ended) else {
            return Ok(());
        };
        let started = Instant::now();
        let order = index.order(state.read().entries(), sort);
        state.outbox.send(order_message(sort, &order));
        tracing::debug!(
            id,
            entries = order.len(),
            ms = started.elapsed().as_secs_f64() * 1000.0,
            "listing sorted"
        );
        Ok(())
    }

    /// Stops listing `id` if it still runs and forgets it. No message follows.
    pub fn close(&self, id: u32) {
        let closed = self.open().remove(&id);
        if let Some(open) = closed {
            open.stop();
        }
    }

    fn close_tab(&self, tab: u32) {
        let mut open = self.open();
        open.retain(|_, listing| {
            let keep = listing.tab != tab;
            if !keep {
                listing.stop();
            }
            keep
        });
    }

    fn open(&self) -> MutexGuard<'_, HashMap<u32, Open>> {
        self.0.open.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Open {
    fn stop(&self) {
        self.state.cancel.store(true, Ordering::Relaxed);
        self.state.outbox.close();
        // Stops the folder's watcher if no other tab watches it.
        self.state.watch().take();
    }
}

/// The listing job: subscribes to the folder's watcher, reads the folder and sends each batch,
/// then builds the sort index and sends the order and the done message, and last applies the
/// changes that came while it read.
fn run(state: &Arc<ListingState>, watchers: &Watchers) -> Result<(), JerryError> {
    let started = Instant::now();
    watch(state, watchers);
    let mut count = 0;
    let mut batches = 0;
    list(&state.folder, &state.cancel, |batch| {
        // Core has the entries before the UI does, so every id the UI gets resolves.
        state.write().append(batch);
        state.outbox.send(batch_message(count, batch));
        count += batch.len();
        batches += 1;
    })?;
    // Under the lock, so a `listing_sort` meanwhile either set the sort used here or sees the
    // index and sends its own order after this one.
    let mut live = state.live();
    {
        let listing = state.read();
        let index = SortIndex::build(listing.entries(), &PlainTypeNames);
        state.outbox.send(order_message(
            live.sort,
            &index.order(listing.entries(), live.sort),
        ));
        live.index = Some(index);
    }
    state.outbox.send(done_message(
        count,
        batches,
        started.elapsed().as_secs_f64() * 1000.0,
    ));
    let pending = std::mem::take(&mut live.pending);
    if !pending.is_empty() {
        match refresh(&state.folder, &pending, &state.cancel) {
            Ok(now) => {
                let diff = now.apply_to(&mut state.write());
                state.send_diff(&mut live, &diff);
            }
            Err(error) if error.code == ErrorCode::Cancelled => {}
            Err(error) => state.end(&mut live, &error),
        }
    }
    Ok(())
}

/// Subscribes `state` to its folder's watcher, unless the listing was closed meanwhile.
fn watch(state: &Arc<ListingState>, watchers: &Watchers) {
    let weak = Arc::downgrade(state);
    let subscribed = watchers.subscribe(&state.folder, move |event| {
        if let Some(state) = weak.upgrade() {
            state.on_event(event);
        }
    });
    match subscribed {
        Ok(subscription) => {
            // Checked under the lock `Open::stop` takes the subscription with.
            let mut slot = state.watch();
            if !state.cancel.load(Ordering::Relaxed) {
                *slot = Some(subscription);
            }
        }
        // Reading the folder reports what is wrong with it; unwatched, the list just isn't live.
        Err(error) => tracing::info!(code = ?error.code, "folder isn't watched"),
    }
}

/// Entries `first_id..` as one batch message (see the module docs).
fn batch_message(first_id: usize, batch: &Entries) -> Bytes {
    let count = batch.len();
    let units = batch.name_units().len();
    // A batch holds at most `MAX_BATCH` entries, and a folder fewer than 2^32: these fit.
    let header = [BATCH, count as u32, first_id as u32, units as u32];
    let mut out = message_buffer(header, entries_bytes(count, units));
    push_entries(&mut out, batch);
    Bytes(out)
}

/// The changed and new entries of `listing` and its removed ids, as one diff message (see the
/// module docs).
fn diff_message(listing: &Listing, diff: &Diff) -> Bytes {
    let ids: Vec<u32> = diff
        .changed
        .iter()
        .copied()
        .chain(diff.added.clone())
        .collect();
    let entries = listing.entries().copy_of(&ids);
    let units = entries.name_units().len();
    let words = ids.len() + diff.removed.len();
    let padding = words % 2 * 4;
    let header = [
        DIFF,
        ids.len() as u32,
        diff.removed.len() as u32,
        units as u32,
    ];
    let mut out = message_buffer(
        header,
        words * 4 + padding + entries_bytes(ids.len(), units),
    );
    for &id in ids.iter().chain(&diff.removed) {
        out.extend_from_slice(&id.to_le_bytes());
    }
    out.resize(out.len() + padding, 0);
    push_entries(&mut out, &entries);
    Bytes(out)
}

/// The size of `count` entries with `units` name units, as [`push_entries`] writes them.
fn entries_bytes(count: usize, units: usize) -> usize {
    count * 24 + units * 2
}

/// Writes `entries` as batches and diffs carry them: each column in turn, then the names.
fn push_entries(out: &mut Vec<u8>, entries: &Entries) {
    for &size in entries.sizes() {
        out.extend_from_slice(&(size as f64).to_le_bytes());
    }
    for &modified in entries.modified() {
        out.extend_from_slice(&filetime_to_unix_ms(modified).to_le_bytes());
    }
    for &attributes in entries.attributes() {
        out.extend_from_slice(&attributes.to_le_bytes());
    }
    for &end in entries.name_ends() {
        out.extend_from_slice(&(end as u32).to_le_bytes());
    }
    for &unit in entries.name_units() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
}

/// Entry ids in display order as one order message (see the module docs).
fn order_message(sort: Sort, order: &[u32]) -> Bytes {
    let column = match sort.column {
        SortColumn::Name => 0,
        SortColumn::Modified => 1,
        SortColumn::Type => 2,
        SortColumn::Size => 3,
        SortColumn::Created => 4,
        SortColumn::Extension => 5,
    };
    let flags = u32::from(sort.descending) | u32::from(sort.folders_first) << 1;
    let mut out = message_buffer([ORDER, order.len() as u32, column, flags], order.len() * 4);
    for &id in order {
        out.extend_from_slice(&id.to_le_bytes());
    }
    Bytes(out)
}

fn done_message(count: usize, batches: usize, ms: f64) -> Bytes {
    let mut out = message_buffer([DONE, count as u32, batches as u32, 0], 8);
    out.extend_from_slice(&ms.to_le_bytes());
    Bytes(out)
}

fn error_message(error: &JerryError) -> Bytes {
    let json = serde_json::to_vec(error).unwrap_or_else(|serialize_error| {
        tracing::error!(%serialize_error, "listing error didn't serialize");
        br#"{"code":"Internal","path":null}"#.to_vec()
    });
    let mut out = message_buffer([ERROR, json.len() as u32, 0, 0], json.len());
    out.extend_from_slice(&json);
    Bytes(out)
}

/// Where a listing's messages go: the UI's channel, or kept in order until one is attached.
/// Once closed, nothing more goes out.
#[derive(Clone)]
struct Outbox(Arc<Mutex<OutboxState>>);

struct OutboxState {
    channel: Option<Channel<Bytes>>,
    kept: Vec<Bytes>,
    closed: bool,
}

impl Outbox {
    fn new(channel: Option<Channel<Bytes>>) -> Self {
        Self(Arc::new(Mutex::new(OutboxState {
            channel,
            kept: Vec::new(),
            closed: false,
        })))
    }

    fn is_attached(&self) -> bool {
        self.state().channel.is_some()
    }

    fn send(&self, message: Bytes) {
        let mut guard = self.state();
        let state = &mut *guard;
        if state.closed {
            return;
        }
        match &state.channel {
            Some(channel) => deliver(channel, message),
            None => state.kept.push(message),
        }
    }

    /// Sends what was kept to `channel`, then everything after it.
    fn attach(&self, channel: Channel<Bytes>) {
        let mut state = self.state();
        if state.closed {
            return;
        }
        for message in std::mem::take(&mut state.kept) {
            deliver(&channel, message);
        }
        state.channel = Some(channel);
    }

    fn close(&self) {
        let mut state = self.state();
        state.closed = true;
        state.channel = None;
        state.kept = Vec::new();
    }

    fn state(&self) -> MutexGuard<'_, OutboxState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn deliver(channel: &Channel<Bytes>, message: Bytes) {
    if let Err(error) = channel.send(message) {
        tracing::warn!(%error, "listing message not delivered");
    }
}

/// Opens a folder for `tab` and streams its entries to `on_message` as binary batches, then
/// their order by `sort` and a done message, then a diff and an order for every change while the
/// listing is open, or an error message (decoded by `src/ipc/listing.ts`). Returns at once,
/// before the folder is read: errors reading it, such as `AccessDenied`, `NotFound` or
/// `DeviceGone`, come as the stream's error message, and so does the folder going away. Opening
/// a folder for a tab closes the tab's previous listing.
#[tauri::command]
#[specta::specta]
pub async fn listing_open(
    listings: State<'_, Listings>,
    tab: u32,
    target: ListingTarget,
    sort: Sort,
    on_message: Channel<Bytes>,
) -> Result<ListingOpened, JerryError> {
    let folder = listings.resolve(target)?;
    let path = display_path(&folder);
    let id = listings.open_folder(tab, folder, sort, on_message);
    Ok(ListingOpened { id, path })
}

/// Sorts listing `id` by `sort` from now on. The new order (only entry ids) comes through the
/// listing's stream as an order message, after any diff sent before it, so it always names the
/// entries the UI has. While the folder is still being read, the stream's own order message
/// follows the new sort. Fails with `NotFound` for a closed listing.
#[tauri::command]
#[specta::specta]
pub async fn listing_sort(app: AppHandle, id: u32, sort: Sort) -> Result<(), JerryError> {
    let listings = app.state::<Listings>().inner().clone();
    jobs::background("sort", move || listings.sort(id, sort)).await
}

/// Stops listing `id` if it still runs and forgets it: no message follows, and the folder's
/// watcher stops unless another tab has the folder open. Closing a listing that is already
/// closed does nothing.
#[tauri::command]
#[specta::specta]
pub async fn listing_close(app: AppHandle, id: u32) {
    app.state::<Listings>().close(id);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    use tauri::ipc::InvokeResponseBody;

    use super::*;

    fn word(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"))
    }

    fn float(bytes: &[u8], at: usize) -> f64 {
        f64::from_le_bytes(bytes[at..at + 8].try_into().expect("8 bytes"))
    }

    #[test]
    fn batches_are_aligned_little_endian_columns() {
        let mut batch = Entries::default();
        let names = ["file10.txt", "ნახაზი"];
        let units: Vec<Vec<u16>> = names.iter().map(|n| n.encode_utf16().collect()).collect();
        // FILETIME of 2026-01-01T00:00:00Z.
        batch.push(&units[0], 1 << 40, 0, 134_116_992_000_000_000, 0x20);
        batch.push(&units[1], 0, 0, 116_444_736_000_000_000, 0x10);

        let Bytes(bytes) = batch_message(7, &batch);

        assert_eq!(
            [word(&bytes, 0), word(&bytes, 4), word(&bytes, 8)],
            [1, 2, 7]
        );
        assert_eq!(word(&bytes, 12), 16, "10 + 6 name units");
        assert_eq!(
            [float(&bytes, 16), float(&bytes, 24)],
            [(1_u64 << 40) as f64, 0.0]
        );
        assert_eq!(
            [float(&bytes, 32), float(&bytes, 40)],
            [1_767_225_600_000.0, 0.0]
        );
        assert_eq!([word(&bytes, 48), word(&bytes, 52)], [0x20, 0x10]);
        assert_eq!([word(&bytes, 56), word(&bytes, 60)], [10, 16]);
        let names_at = 64;
        let (pairs, rest) = bytes[names_at..].as_chunks::<2>();
        assert!(rest.is_empty());
        let decoded: Vec<u16> = pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect();
        assert_eq!(String::from_utf16_lossy(&decoded), "file10.txtნახაზი");
        assert_eq!(bytes.len(), names_at + 16 * 2);
    }

    #[test]
    fn orders_are_ids_after_the_sort_they_follow() {
        let sort = Sort {
            column: SortColumn::Created,
            descending: true,
            folders_first: true,
        };
        let Bytes(bytes) = order_message(sort, &[2, 0, 70_000]);
        assert_eq!(
            [
                word(&bytes, 0),
                word(&bytes, 4),
                word(&bytes, 8),
                word(&bytes, 12)
            ],
            [ORDER, 3, 4, 0b11]
        );
        assert_eq!(
            [word(&bytes, 16), word(&bytes, 20), word(&bytes, 24)],
            [2, 0, 70_000]
        );
        assert_eq!(bytes.len(), 28);

        let Bytes(empty) = order_message(Sort::default(), &[]);
        assert_eq!(
            [word(&empty, 4), word(&empty, 8), word(&empty, 12)],
            [0, 0, 0b10]
        );
        assert_eq!(empty.len(), 16);
    }

    #[test]
    fn done_and_error_messages() {
        let Bytes(done) = done_message(1_000, 2, 4.5);
        assert_eq!(
            [word(&done, 0), word(&done, 4), word(&done, 8)],
            [3, 1_000, 2]
        );
        assert_eq!(float(&done, 16), 4.5);

        let error = JerryError::from_win32(5).at(r"\\?\C:\დახურული");
        let Bytes(message) = error_message(&error);
        assert_eq!(word(&message, 0), 4);
        let json = &message[16..];
        assert_eq!(word(&message, 4) as usize, json.len());
        let value: serde_json::Value = serde_json::from_slice(json).expect("JSON");
        assert_eq!(
            value,
            serde_json::json!({ "code": "AccessDenied", "path": r"C:\დახურული" })
        );
    }

    #[test]
    fn diffs_carry_ids_removals_and_entries_laid_out_as_in_batches() {
        let mut listing = Listing::new(PathBuf::from(r"\\?\C:\Work"));
        let mut batch = Entries::default();
        for name in ["a.txt", "b.txt", "c.txt"] {
            let units: Vec<u16> = name.encode_utf16().collect();
            batch.push(&units, 1, 0, 116_444_736_000_000_000, 0x20);
        }
        listing.append(&batch);
        let mut present = Entries::default();
        let units: Vec<u16> = "გეგმა.pdf".encode_utf16().collect();
        present.push(&units, 1 << 33, 0, 134_116_992_000_000_000, 0x20);
        let units: Vec<u16> = "b.txt".encode_utf16().collect();
        present.push(&units, 5, 0, 116_444_736_000_000_000, 0x20);
        let diff = listing.apply(&jerry_core::listing::Observed {
            present,
            absent: vec!["a.txt".encode_utf16().collect()],
            renames: Vec::new(),
        });

        let Bytes(bytes) = diff_message(&listing, &diff);

        // b.txt changed (id 1), გეგმა.pdf is new (id 3), a.txt is gone (id 0).
        assert_eq!(
            [
                word(&bytes, 0),
                word(&bytes, 4),
                word(&bytes, 8),
                word(&bytes, 12)
            ],
            [DIFF, 2, 1, 5 + 9]
        );
        assert_eq!([word(&bytes, 16), word(&bytes, 20)], [1, 3], "ids");
        assert_eq!(word(&bytes, 24), 0, "removed");
        assert_eq!(word(&bytes, 28), 0, "padding to 8 bytes");
        let entries_at = 32;
        assert_eq!(
            [float(&bytes, entries_at), float(&bytes, entries_at + 8)],
            [5.0, (1_u64 << 33) as f64]
        );
        assert_eq!(float(&bytes, entries_at + 24), 1_767_225_600_000.0);
        let ends_at = entries_at + 32 + 8;
        assert_eq!([word(&bytes, ends_at), word(&bytes, ends_at + 4)], [5, 14]);
        let (pairs, rest) = bytes[ends_at + 8..].as_chunks::<2>();
        assert!(rest.is_empty());
        let names: Vec<u16> = pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect();
        assert_eq!(String::from_utf16_lossy(&names), "b.txtგეგმა.pdf");

        // An even number of ids needs no padding.
        let Bytes(removal) = diff_message(
            &listing,
            &Diff {
                removed: vec![1, 2],
                ..Diff::default()
            },
        );
        assert_eq!([word(&removal, 4), word(&removal, 8)], [0, 2]);
        assert_eq!(removal.len(), 16 + 8);
    }

    /// A channel whose raw messages arrive on the returned receiver.
    fn test_channel() -> (Channel<Bytes>, Receiver<Vec<u8>>) {
        let (sender, receiver) = channel();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Raw(bytes) = body {
                let _ = sender.send(bytes);
            }
            Ok(())
        });
        (channel, receiver)
    }

    fn next(receiver: &Receiver<Vec<u8>>) -> Vec<u8> {
        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("a message comes")
    }

    /// The messages up to and including the done or error message.
    fn until_end(receiver: &Receiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut messages = Vec::new();
        loop {
            let message = next(receiver);
            let kind = word(&message, 0);
            messages.push(message);
            if kind == DONE || kind == ERROR {
                return messages;
            }
        }
    }

    fn folder_with(names: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp folder");
        for name in names {
            std::fs::write(dir.path().join(name), name.as_bytes()).expect("write");
        }
        dir
    }

    fn path_target(path: &Path) -> ListingTarget {
        ListingTarget::Path {
            path: path.to_string_lossy().into_owned(),
        }
    }

    fn state(listings: &Listings, id: u32) -> Arc<ListingState> {
        listings.open()[&id].state.clone()
    }

    /// The names of an order message's entries, in its order.
    fn ordered_names(listings: &Listings, id: u32, message: &[u8]) -> Vec<String> {
        assert_eq!(word(message, 0), ORDER);
        let entries = state(listings, id).read().entries().clone();
        let count = word(message, 4) as usize;
        assert_eq!(message.len(), 16 + count * 4);
        (0..count)
            .map(|i| word(message, 16 + i * 4) as usize)
            .map(|entry| entries.name(entry).to_string_lossy().into_owned())
            .collect()
    }

    const BY_SIZE: Sort = Sort {
        column: SortColumn::Size,
        descending: true,
        folders_first: true,
    };

    #[test]
    fn opens_a_folder_and_streams_it() {
        let dir = folder_with(&["file10.txt", "ნახაზი.dwg", "file2.txt"]);
        std::fs::create_dir(dir.path().join("ქვესაქაღალდე")).expect("mkdir");
        let listings = Listings::default();
        let (channel, receiver) = test_channel();

        let folder = listings.resolve(path_target(dir.path())).expect("resolves");
        let id = listings.open_folder(1, folder, Sort::default(), channel);
        let messages = until_end(&receiver);

        let (batches, end) = messages.split_at(messages.len() - 2);
        let [order, done] = end else {
            unreachable!("split at two from the end")
        };
        assert_eq!(word(done, 0), DONE);
        assert_eq!(word(done, 4), 4, "four entries");
        assert_eq!(word(done, 8) as usize, batches.len());
        assert!(batches.iter().all(|batch| word(batch, 0) == BATCH));
        assert_eq!([word(order, 8), word(order, 12)], [0, 0b10], "by name");
        assert_eq!(
            ordered_names(&listings, id, order),
            ["ქვესაქაღალდე", "file2.txt", "file10.txt", "ნახაზი.dwg"]
        );

        // Every id the stream gave has its exact path in core, and a subfolder opens by its id.
        let entries = state(&listings, id).read().entries().clone();
        let sub = (0..entries.len())
            .find(|&id| entries.is_dir(id))
            .expect("the subfolder") as u32;
        let target = ListingTarget::Entry {
            listing: id,
            entry: sub,
        };
        let subfolder = listings.resolve(target).expect("entry resolves");
        assert_eq!(
            subfolder,
            long_path(&dir.path().join("ქვესაქაღალდე")).expect("path")
        );
    }

    #[test]
    fn errors_reading_the_folder_come_through_the_stream() {
        let dir = tempfile::tempdir().expect("temp folder");
        let listings = Listings::default();
        let (channel, receiver) = test_channel();

        let missing = listings
            .resolve(path_target(&dir.path().join("აღარ არის")))
            .expect("resolves");
        listings.open_folder(1, missing, Sort::default(), channel);
        let messages = until_end(&receiver);

        assert_eq!(messages.len(), 1);
        let json: serde_json::Value = serde_json::from_slice(&messages[0][16..]).expect("JSON");
        assert_eq!(json["code"], "NotFound");
        assert_eq!(listings.0.watchers.watched(), 0, "nothing to watch");
    }

    #[test]
    fn targets_must_be_absolute_and_entries_must_exist() {
        let listings = Listings::default();
        let relative = ListingTarget::Path {
            path: r"Work\plans".into(),
        };
        let error = listings.resolve(relative).expect_err("relative");
        assert_eq!(error.code, ErrorCode::InvalidName);
        let stale = ListingTarget::Entry {
            listing: 99,
            entry: 0,
        };
        let error = listings.resolve(stale).expect_err("no such listing");
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn a_tab_has_one_listing_and_closed_ones_are_gone() {
        let dir = folder_with(&["a.txt"]);
        let listings = Listings::default();
        let (first_channel, first) = test_channel();
        let (second_channel, second) = test_channel();
        let folder = long_path(dir.path()).expect("long path");

        let old = listings.open_folder(1, folder.clone(), Sort::default(), first_channel);
        until_end(&first);
        let new = listings.open_folder(1, folder.clone(), Sort::default(), second_channel);
        until_end(&second);

        assert_ne!(old, new);
        let entry = |listing| ListingTarget::Entry { listing, entry: 0 };
        assert!(listings.resolve(entry(old)).is_err(), "replaced");
        assert!(listings.resolve(entry(new)).is_ok());
        assert_eq!(listings.0.watchers.watched(), 1);
        listings.close(new);
        listings.close(new);
        assert!(listings.resolve(entry(new)).is_err(), "closed");
        assert_eq!(listings.0.watchers.watched(), 0, "the watcher stopped");
        let error = listings.sort(new, BY_SIZE).expect_err("closed");
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn sorting_a_listed_folder_sends_only_the_new_order() {
        // Each file holds its own name, so the longer name is the bigger file.
        let dir = folder_with(&["a.txt", "ccc.txt", "bb.txt"]);
        std::fs::create_dir(dir.path().join("ფოლდერი")).expect("mkdir");
        let listings = Listings::default();
        let (channel, receiver) = test_channel();
        let folder = long_path(dir.path()).expect("long path");
        let id = listings.open_folder(1, folder, Sort::default(), channel);
        until_end(&receiver);

        listings.sort(id, BY_SIZE).expect("sorts");

        let order = next(&receiver);
        assert_eq!([word(&order, 8), word(&order, 12)], [3, 0b11]);
        assert_eq!(
            ordered_names(&listings, id, &order),
            ["ფოლდერი", "ccc.txt", "bb.txt", "a.txt"]
        );
        assert!(receiver.try_recv().is_err(), "nothing more on the stream");
    }

    #[test]
    fn a_sort_while_reading_goes_into_the_streams_order() {
        let dir = folder_with(&["a.txt", "ccc.txt", "bb.txt"]);
        let folder = long_path(dir.path()).expect("long path");
        // A listing whose job hasn't run yet, as `start` makes it.
        let listings = Listings::default();
        let (channel, receiver) = test_channel();
        let state = Arc::new(ListingState {
            listing: RwLock::new(Listing::new(folder.clone())),
            folder,
            live: Mutex::new(Live {
                sort: Sort::default(),
                index: None,
                pending: Changes::default(),
                ended: false,
            }),
            cancel: AtomicBool::new(false),
            outbox: Outbox::new(Some(channel)),
            watch: Mutex::new(None),
        });
        listings.open().insert(
            7,
            Open {
                tab: 1,
                state: state.clone(),
            },
        );

        listings.sort(7, BY_SIZE).expect("sorts");
        assert!(
            receiver.try_recv().is_err(),
            "no order before the folder is in"
        );
        run(&state, &listings.0.watchers).expect("lists");
        let messages = until_end(&receiver);

        let order = &messages[messages.len() - 2];
        assert_eq!([word(order, 8), word(order, 12)], [3, 0b11]);
        assert_eq!(
            ordered_names(&listings, 7, order),
            ["ccc.txt", "bb.txt", "a.txt"]
        );
    }

    #[test]
    fn a_prestarted_listing_keeps_its_messages_for_the_ui() {
        let dir = folder_with(&["ნახაზი 1.dwg", "ნახაზი 2.dwg"]);
        let listings = Listings::default();
        let id = listings
            .prestart(3, dir.path(), Sort::default())
            .expect("starts");

        // Let it finish with nobody listening.
        let wait = Instant::now();
        while state(&listings, id).live().index.is_none() {
            assert!(wait.elapsed() < Duration::from_secs(10), "the listing ends");
            std::thread::sleep(Duration::from_millis(5));
        }
        let (channel, receiver) = test_channel();
        let folder = long_path(dir.path()).expect("long path");
        let attached = listings.open_folder(3, folder.clone(), Sort::default(), channel);
        let messages = until_end(&receiver);

        assert_eq!(
            attached, id,
            "the UI gets the listing that was already running"
        );
        assert_eq!(word(&messages[0], 0), BATCH);
        assert_eq!(word(&messages[messages.len() - 2], 0), ORDER);
        assert_eq!(word(messages.last().expect("done"), 4), 2);

        // A tab sorted otherwise gets a listing of its own.
        let again = listings
            .prestart(4, dir.path(), Sort::default())
            .expect("starts");
        let other = listings.open_folder(4, folder, BY_SIZE, test_channel().0);
        assert_ne!(other, again);
    }

    /// The ids and removed ids of a diff message.
    fn diff_ids(message: &[u8]) -> (Vec<u32>, Vec<u32>) {
        assert_eq!(word(message, 0), DIFF);
        let (count, removed) = (word(message, 4) as usize, word(message, 8) as usize);
        let ids = (0..count).map(|i| word(message, 16 + i * 4)).collect();
        let gone = (0..removed)
            .map(|i| word(message, 16 + (count + i) * 4))
            .collect();
        (ids, gone)
    }

    #[test]
    fn changes_in_the_folder_come_as_a_diff_and_an_order() {
        let dir = folder_with(&["b.txt", "plan.dwg"]);
        let listings = Listings::default();
        let (channel, receiver) = test_channel();
        let folder = long_path(dir.path()).expect("long path");
        let id = listings.open_folder(1, folder, Sort::default(), channel);
        until_end(&receiver);
        let plan = state(&listings, id)
            .read()
            .live_ids()
            .into_iter()
            .find(|&entry| state(&listings, id).read().entries().name(entry as usize) == "plan.dwg")
            .expect("plan.dwg");

        std::fs::write(dir.path().join("a.txt"), b"new").expect("write");
        std::fs::rename(dir.path().join("plan.dwg"), dir.path().join("გეგმა.dwg")).expect("rename");
        std::fs::remove_file(dir.path().join("b.txt")).expect("delete");

        // Until the stream shows all three: they may come in one diff or several.
        let mut changed = BTreeSet::new();
        let mut gone = BTreeSet::new();
        let mut last_order = Vec::new();
        while gone.is_empty() || changed.len() < 2 {
            let diff = next(&receiver);
            let (ids, removed) = diff_ids(&diff);
            assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "ascending");
            changed.extend(ids);
            gone.extend(removed);
            last_order = next(&receiver);
            assert_eq!(word(&last_order, 0), ORDER, "an order follows every diff");
        }
        assert!(changed.contains(&plan), "the renamed file kept its id");
        assert_eq!(gone.len(), 1);
        assert_eq!(
            ordered_names(&listings, id, &last_order),
            ["a.txt", "გეგმა.dwg"]
        );
        // The removed entry no longer opens.
        let removed = ListingTarget::Entry {
            listing: id,
            entry: *gone.first().expect("one"),
        };
        assert!(listings.resolve(removed).is_err());
    }

    #[test]
    fn a_deleted_folder_ends_the_stream_with_not_found() {
        let dir = tempfile::tempdir().expect("temp folder");
        let watched = dir.path().join("ნახაზები");
        std::fs::create_dir(&watched).expect("mkdir");
        let listings = Listings::default();
        let (channel, receiver) = test_channel();
        listings.open_folder(
            1,
            long_path(&watched).expect("long path"),
            Sort::default(),
            channel,
        );
        until_end(&receiver);

        std::fs::remove_dir(&watched).expect("a watched folder can be deleted");

        let messages = until_end(&receiver);
        let error = messages.last().expect("an error");
        let json: serde_json::Value = serde_json::from_slice(&error[16..]).expect("JSON");
        assert_eq!(json["code"], "NotFound");
    }

    #[test]
    fn nothing_is_sent_after_close() {
        let (channel, receiver) = test_channel();
        let outbox = Outbox::new(Some(channel));
        outbox.send(done_message(0, 0, 0.0));
        outbox.close();
        outbox.send(done_message(0, 0, 0.0));
        outbox.attach(test_channel().0);
        assert_eq!(receiver.try_iter().count(), 1);
        assert!(!outbox.is_attached());
    }
}
