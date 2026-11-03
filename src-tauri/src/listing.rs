//! Folder listings for the UI. `listing_open` streams a folder through a `Channel` as binary
//! messages (ADR-0001) and `listing_close` stops and forgets it. Every listing is its own
//! background job, so a slow disk holds up only its own tab.
//!
//! Messages are little-endian and start with the 16-byte header `[kind, a, b, c]`:
//! - Batch (1): `[1, count, firstId, nameUnits]`, then `f64 size[count]` (bytes),
//!   `f64 modified[count]` (ms since the Unix epoch), `u32 attributes[count]`,
//!   `u32 nameEnd[count]` (where each name ends in this batch's names, in UTF-16 units) and the
//!   names as UTF-16 code units. The entries' ids are `firstId + i`.
//! - Order (2): a display order, from sorting (#26).
//! - Done (3): `[3, count, batches, 0]`, then the `f64` ms the listing took in Rust. Last message.
//! - Error (4): `[4, jsonBytes, 0, 0]`, then the `JerryError` as UTF-8 JSON. Last message.
//!
//! The UI's decoder is `src/ipc/listing.ts`; both sides test this layout.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Instant;

use jerry_core::error::display_path;
use jerry_core::listing::{Entries, Listing, filetime_to_unix_ms, list};
use jerry_core::path::long_path;
use jerry_core::{ErrorCode, JerryError};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::ipc::{Bytes, message_buffer};
use crate::jobs;

const BATCH: u32 = 1;
const DONE: u32 = 3;
const ERROR: u32 = 4;

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
}

struct Open {
    tab: u32,
    listing: Arc<RwLock<Listing>>,
    cancel: Arc<AtomicBool>,
    outbox: Outbox,
}

impl Listings {
    /// Starts listing `folder` for `tab` before the UI has a channel for it: at process start,
    /// before Tauri, for the session's folders (spike #11, session restore #42). Messages are
    /// kept until `listing_open` for the same tab and folder attaches its channel.
    pub fn prestart(&self, tab: u32, folder: &Path) -> Result<u32, JerryError> {
        Ok(self.start(tab, long_path(folder)?, None))
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
                    .and_then(|open| read(&open.listing).path(entry as usize))
                    .ok_or_else(|| JerryError::new(ErrorCode::NotFound))
            }
        }
    }

    /// Opens `folder` for `tab` with its messages going to `channel`: a listing started for it
    /// with [`Listings::prestart`] if there is one, a new one otherwise.
    fn open_folder(&self, tab: u32, folder: PathBuf, channel: Channel<Bytes>) -> u32 {
        let prestarted = self.open().iter().find_map(|(&id, open)| {
            let waiting = open.tab == tab
                && !open.outbox.is_attached()
                && read(&open.listing).folder() == folder;
            waiting.then(|| (id, open.outbox.clone()))
        });
        match prestarted {
            Some((id, outbox)) => {
                outbox.attach(channel);
                id
            }
            None => self.start(tab, folder, Some(channel)),
        }
    }

    /// Starts listing `folder` for `tab` on a background job and returns the listing's id.
    fn start(&self, tab: u32, folder: PathBuf, channel: Option<Channel<Bytes>>) -> u32 {
        self.close_tab(tab);
        let id = self.0.last_id.fetch_add(1, Ordering::Relaxed) + 1;
        let listing = Arc::new(RwLock::new(Listing::new(folder.clone())));
        let cancel = Arc::new(AtomicBool::new(false));
        let outbox = Outbox::new(channel);
        self.open().insert(
            id,
            Open {
                tab,
                listing: listing.clone(),
                cancel: cancel.clone(),
                outbox: outbox.clone(),
            },
        );
        let job_outbox = outbox.clone();
        tauri::async_runtime::spawn(async move {
            let job = move || run(&folder, &listing, &cancel, &job_outbox);
            match jobs::background("listing", job).await {
                Ok(()) => {}
                Err(error) if error.code == ErrorCode::Cancelled => {}
                Err(error) => {
                    tracing::info!(id, code = ?error.code, "listing failed");
                    outbox.send(error_message(&error));
                }
            }
        });
        id
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
        self.cancel.store(true, Ordering::Relaxed);
        self.outbox.close();
    }
}

/// The listing job: reads `folder` into `listing` and sends each batch, then the done message.
fn run(
    folder: &Path,
    listing: &RwLock<Listing>,
    cancel: &AtomicBool,
    outbox: &Outbox,
) -> Result<(), JerryError> {
    let started = Instant::now();
    let mut count = 0;
    let mut batches = 0;
    list(folder, cancel, |batch| {
        // Core has the entries before the UI does, so every id the UI gets resolves.
        write(listing).append(batch);
        outbox.send(batch_message(count, batch));
        count += batch.len();
        batches += 1;
    })?;
    outbox.send(done_message(
        count,
        batches,
        started.elapsed().as_secs_f64() * 1000.0,
    ));
    Ok(())
}

/// Entries `first_id..` as one batch message (see the module docs).
fn batch_message(first_id: usize, batch: &Entries) -> Bytes {
    let count = batch.len();
    let units = batch.name_units();
    // A batch holds at most `MAX_BATCH` entries, and a folder fewer than 2^32: these fit.
    let header = [BATCH, count as u32, first_id as u32, units.len() as u32];
    let mut out = message_buffer(header, count * 24 + units.len() * 2);
    for &size in batch.sizes() {
        out.extend_from_slice(&(size as f64).to_le_bytes());
    }
    for &modified in batch.modified() {
        out.extend_from_slice(&filetime_to_unix_ms(modified).to_le_bytes());
    }
    for &attributes in batch.attributes() {
        out.extend_from_slice(&attributes.to_le_bytes());
    }
    for &end in batch.name_ends() {
        out.extend_from_slice(&(end as u32).to_le_bytes());
    }
    for &unit in units {
        out.extend_from_slice(&unit.to_le_bytes());
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

fn read(listing: &RwLock<Listing>) -> std::sync::RwLockReadGuard<'_, Listing> {
    listing.read().unwrap_or_else(PoisonError::into_inner)
}

fn write(listing: &RwLock<Listing>) -> std::sync::RwLockWriteGuard<'_, Listing> {
    listing.write().unwrap_or_else(PoisonError::into_inner)
}

/// Opens a folder for `tab` and streams its entries to `on_message` as binary batches, then a
/// done or an error message (decoded by `src/ipc/listing.ts`). Returns at once, before the
/// folder is read: errors reading it, such as `AccessDenied`, `NotFound` or `DeviceGone`, come
/// as the stream's error message. Opening a folder for a tab closes the tab's previous listing.
#[tauri::command]
#[specta::specta]
pub async fn listing_open(
    listings: State<'_, Listings>,
    tab: u32,
    target: ListingTarget,
    on_message: Channel<Bytes>,
) -> Result<ListingOpened, JerryError> {
    let folder = listings.resolve(target)?;
    let path = display_path(&folder);
    let id = listings.open_folder(tab, folder, on_message);
    Ok(ListingOpened { id, path })
}

/// Stops listing `id` if it still runs and forgets it: no message follows. Closing a listing
/// that is already closed does nothing.
#[tauri::command]
#[specta::specta]
pub async fn listing_close(app: AppHandle, id: u32) {
    app.state::<Listings>().close(id);
}

#[cfg(test)]
mod tests {
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
        let decoded: Vec<u16> = bytes[names_at..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(String::from_utf16_lossy(&decoded), "file10.txtნახაზი");
        assert_eq!(bytes.len(), names_at + 16 * 2);
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

    /// The messages up to and including the done or error message.
    fn until_end(receiver: &Receiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut messages = Vec::new();
        loop {
            let message = receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("the listing ends");
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

    #[test]
    fn opens_a_folder_and_streams_it() {
        let dir = folder_with(&["ა.txt", "ბ.txt", "c.txt"]);
        std::fs::create_dir(dir.path().join("ქვესაქაღალდე")).expect("mkdir");
        let listings = Listings::default();
        let (channel, receiver) = test_channel();

        let folder = listings.resolve(path_target(dir.path())).expect("resolves");
        let id = listings.open_folder(1, folder, channel);
        let messages = until_end(&receiver);

        let (batches, done) = messages.split_at(messages.len() - 1);
        assert_eq!(word(&done[0], 0), DONE);
        assert_eq!(word(&done[0], 4), 4, "four entries");
        assert_eq!(word(&done[0], 8) as usize, batches.len());
        assert!(batches.iter().all(|batch| word(batch, 0) == BATCH));

        // Every id the stream gave has its exact path in core, and a subfolder opens by its id.
        let entries = read(&listings.open()[&id].listing).entries().clone();
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
        listings.open_folder(1, missing, channel);
        let messages = until_end(&receiver);

        assert_eq!(messages.len(), 1);
        let json: serde_json::Value = serde_json::from_slice(&messages[0][16..]).expect("JSON");
        assert_eq!(json["code"], "NotFound");
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

        let old = listings.open_folder(1, folder.clone(), first_channel);
        until_end(&first);
        let new = listings.open_folder(1, folder.clone(), second_channel);
        until_end(&second);

        assert_ne!(old, new);
        let entry = |listing| ListingTarget::Entry { listing, entry: 0 };
        assert!(listings.resolve(entry(old)).is_err(), "replaced");
        assert!(listings.resolve(entry(new)).is_ok());
        listings.close(new);
        listings.close(new);
        assert!(listings.resolve(entry(new)).is_err(), "closed");
    }

    #[test]
    fn a_prestarted_listing_keeps_its_messages_for_the_ui() {
        let dir = folder_with(&["ნახაზი 1.dwg", "ნახაზი 2.dwg"]);
        let listings = Listings::default();
        let id = listings.prestart(3, dir.path()).expect("starts");

        // Let it finish with nobody listening.
        let wait = Instant::now();
        while read(&listings.open()[&id].listing).entries().len() < 2 {
            assert!(wait.elapsed() < Duration::from_secs(10), "the listing ends");
            std::thread::sleep(Duration::from_millis(5));
        }
        let (channel, receiver) = test_channel();
        let folder = long_path(dir.path()).expect("long path");
        let attached = listings.open_folder(3, folder, channel);
        let messages = until_end(&receiver);

        assert_eq!(
            attached, id,
            "the UI gets the listing that was already running"
        );
        assert_eq!(word(&messages[0], 0), BATCH);
        assert_eq!(word(messages.last().expect("done"), 4), 2);
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
