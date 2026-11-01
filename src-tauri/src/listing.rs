//! Folder listing over a Tauri `Channel` in spike #12's binary format, plus spike #11's startup
//! listing: Rust starts listing the first folder before the window exists and keeps the messages
//! until the UI attaches a channel.
//!
//! Wire format (binary, little-endian; every message starts with a `u32` kind and a 16-byte
//! header so the `f64` columns that follow stay 8-byte aligned in the `ArrayBuffer`):
//!
//! - Batch (1): `[kind, count, firstId, nameUnits]`, then `f64 size[count]` (bytes; exact up to
//!   2^53), `f64 modified[count]` (ms since the Unix epoch), `u32 attributes[count]`,
//!   `u32 nameEnd[count]` (end of each name in this batch's name block, in UTF-16 units) and the
//!   names as UTF-16 code units. Entry ids are `firstId + i`.
//! - Order (2): `[kind, count, f64 rustMs]`, then `u32 ids[count]` in display order.
//! - Done (3): `[kind, count, batches, 0]`, then `f64` stats (see [`ListingStats`]).
//! - Error (4): `[kind, code, osCode, 0]`.

use std::collections::HashMap;
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use jerry_core::error::{ErrorCode, JerryError};
use jerry_core::listing::{Entries, filetime_to_unix_ms, list_dir};
use jerry_core::sort::{SortColumn, SortIndex};
use jerry_core::synth;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody, IpcResponse};

use crate::startup;

const MSG_BATCH: u32 = 1;
const MSG_ORDER: u32 = 2;
const MSG_DONE: u32 = 3;
const MSG_ERROR: u32 = 4;

/// A binary IPC payload. It reaches JS as an `ArrayBuffer`, with no JSON in between.
pub struct Bytes(pub Vec<u8>);

impl IpcResponse for Bytes {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        Ok(InvokeResponseBody::Raw(self.0))
    }
}

impl Type for Bytes {
    fn definition(_: &mut specta::Types) -> specta::datatype::DataType {
        specta::datatype::DataType::Reference(specta_typescript::define("ArrayBuffer"))
    }
}

/// An error as the UI gets it: a code, never text.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    code: IpcErrorCode,
    path: Option<String>,
    os: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Type)]
pub enum IpcErrorCode {
    AccessDenied,
    NotFound,
    Cancelled,
    Os,
}

impl From<JerryError> for IpcError {
    fn from(error: JerryError) -> Self {
        let (code, os) = match error.code {
            ErrorCode::AccessDenied => (IpcErrorCode::AccessDenied, None),
            ErrorCode::NotFound => (IpcErrorCode::NotFound, None),
            ErrorCode::Cancelled => (IpcErrorCode::Cancelled, None),
            ErrorCode::Os(os) => (IpcErrorCode::Os, Some(os)),
        };
        IpcError {
            code,
            path: error.path.map(|path| path.display().to_string()),
            os,
        }
    }
}

impl IpcError {
    fn code(code: IpcErrorCode) -> Self {
        IpcError {
            code,
            path: None,
            os: None,
        }
    }
}

/// A column to sort by.
#[derive(Debug, Clone, Copy, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SortBy {
    Name,
    Size,
    Modified,
    Type,
}

impl From<SortBy> for SortColumn {
    fn from(by: SortBy) -> Self {
        match by {
            SortBy::Name => SortColumn::Name,
            SortBy::Size => SortColumn::Size,
            SortBy::Modified => SortColumn::Modified,
            SortBy::Type => SortColumn::Type,
        }
    }
}

/// Where the time went in one listing (Rust side), in ms from the start of the job.
#[derive(Debug, Clone, Copy, Default, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListingStats {
    pub count: u32,
    pub batches: u32,
    /// Bytes sent over the channel.
    pub bytes: f64,
    pub first_batch_ms: f64,
    pub list_ms: f64,
    /// Time the listing thread spent in batch callbacks (encode + send).
    pub sink_ms: f64,
    /// Of that, time inside `Channel::send`.
    pub send_ms: f64,
    pub keys_ms: f64,
    pub name_sort_ms: f64,
    pub ranks_ms: f64,
    pub order_ms: f64,
    pub total_ms: f64,
}

/// Listings the UI has open, by id, and the startup listing until the UI takes it.
#[derive(Default)]
pub struct Listings {
    next_id: AtomicU32,
    open: Mutex<HashMap<u32, Arc<Listed>>>,
    running: Mutex<HashMap<u32, Arc<AtomicBool>>>,
    startup: Mutex<Option<(u32, Outbox)>>,
}

/// A finished listing: its entries and everything needed to re-sort them.
struct Listed {
    entries: Entries,
    index: SortIndex,
}

impl Listings {
    fn get(&self, id: u32) -> Option<Arc<Listed>> {
        let open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        open.get(&id).cloned()
    }
}

/// Where a listing's messages go: straight into the UI's channel, or kept until the UI attaches
/// one (the startup listing runs before the window exists).
#[derive(Clone, Default)]
pub struct Outbox(Arc<Mutex<OutboxState>>);

#[derive(Default)]
struct OutboxState {
    channel: Option<Channel<Bytes>>,
    kept: Vec<Vec<u8>>,
}

impl Outbox {
    fn to_channel(channel: Channel<Bytes>) -> Self {
        Outbox(Arc::new(Mutex::new(OutboxState {
            channel: Some(channel),
            kept: Vec::new(),
        })))
    }

    fn send(&self, bytes: Vec<u8>) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match &state.channel {
            Some(channel) => {
                if let Err(error) = channel.send(Bytes(bytes)) {
                    tracing::warn!(%error, "listing channel send failed");
                }
            }
            None => state.kept.push(bytes),
        }
    }

    /// Sends everything kept so far into `channel`, then sends straight to it.
    fn attach(&self, channel: Channel<Bytes>) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for bytes in state.kept.drain(..) {
            if let Err(error) = channel.send(Bytes(bytes)) {
                tracing::warn!(%error, "listing channel send failed");
            }
        }
        state.channel = Some(channel);
    }
}

/// Columns of entries `range` as one binary batch (see the module docs).
fn encode_batch(entries: &Entries, range: Range<usize>) -> Vec<u8> {
    let count = range.len();
    let names = if count == 0 {
        0..0
    } else {
        entries.name_range(range.start).start..entries.name_range(range.end - 1).end
    };
    let units = &entries.name_units()[names.clone()];
    let mut out = Vec::with_capacity(16 + count * 24 + units.len() * 2 + 2);
    for value in [
        MSG_BATCH,
        count as u32,
        range.start as u32,
        units.len() as u32,
    ] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for id in range.clone() {
        out.extend_from_slice(&(entries.sizes[id] as f64).to_le_bytes());
    }
    for id in range.clone() {
        out.extend_from_slice(&filetime_to_unix_ms(entries.modified[id]).to_le_bytes());
    }
    for id in range.clone() {
        out.extend_from_slice(&entries.attributes[id].to_le_bytes());
    }
    for id in range {
        let end = (entries.name_range(id).end - names.start) as u32;
        out.extend_from_slice(&end.to_le_bytes());
    }
    for unit in units {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

/// A display order as `[kind, count, f64 rustMs]` + `u32` ids.
fn encode_order(order: &[u32], rust_ms: f64) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + order.len() * 4);
    out.extend_from_slice(&MSG_ORDER.to_le_bytes());
    out.extend_from_slice(&(order.len() as u32).to_le_bytes());
    out.extend_from_slice(&rust_ms.to_le_bytes());
    for id in order {
        out.extend_from_slice(&id.to_le_bytes());
    }
    out
}

fn encode_done(stats: &ListingStats) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + 10 * 8);
    for value in [MSG_DONE, stats.count, stats.batches, 0] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for value in [
        stats.bytes,
        stats.first_batch_ms,
        stats.list_ms,
        stats.sink_ms,
        stats.send_ms,
        stats.keys_ms,
        stats.name_sort_ms,
        stats.ranks_ms,
        stats.order_ms,
        stats.total_ms,
    ] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn encode_error(error: &IpcError) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    for value in [MSG_ERROR, error.code as u32, error.os.unwrap_or(0), 0] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

/// Starts listing the synthetic folder for `count` on its own IO thread; returns its id.
fn start(listings: &Arc<Listings>, count: u32, outbox: Outbox, is_startup: bool) -> u32 {
    let id = listings.next_id.fetch_add(1, Ordering::Relaxed);
    let cancel = Arc::new(AtomicBool::new(false));
    listings
        .running
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id, cancel.clone());
    let job_listings = listings.clone();
    let job_outbox = outbox.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("jerry-listing-{id}"))
        .spawn(move || {
            let job = AssertUnwindSafe(|| {
                run(&job_listings, id, count, &cancel, &job_outbox, is_startup)
            });
            if catch_unwind(job).is_err() {
                tracing::error!(id, "listing job panicked");
            }
            job_listings
                .running
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
        });
    if let Err(error) = spawned {
        let error = IpcError::from(JerryError::from_io(&error, &synth::root()));
        outbox.send(encode_error(&error));
    }
    id
}

fn run(
    listings: &Listings,
    id: u32,
    count: u32,
    cancel: &AtomicBool,
    outbox: &Outbox,
    is_startup: bool,
) {
    let started = Instant::now();
    if is_startup {
        startup::mark("listing:start");
    }
    let dir = synth::dataset_dir(count as usize);
    let mut stats = ListingStats::default();
    let listed = list_dir(&dir, cancel, |entries, range| {
        if stats.batches == 0 {
            stats.first_batch_ms = ms(started);
            if is_startup {
                startup::mark("listing:first_batch");
            }
        }
        stats.batches += 1;
        let in_sink = Instant::now();
        let bytes = encode_batch(entries, range);
        stats.bytes += bytes.len() as f64;
        let sending = Instant::now();
        outbox.send(bytes);
        stats.send_ms += ms(sending);
        stats.sink_ms += ms(in_sink);
    });
    let entries = match listed {
        Ok(entries) => entries,
        Err(error) => {
            outbox.send(encode_error(&error.into()));
            return;
        }
    };
    stats.list_ms = ms(started);
    stats.count = entries.len() as u32;

    let index = SortIndex::build(&entries);
    stats.keys_ms = index.timings.keys_ms;
    stats.name_sort_ms = index.timings.name_sort_ms;
    stats.ranks_ms = index.timings.ranks_ms;
    let sorted = Instant::now();
    let order = index.order(&entries, SortColumn::Name, false, true);
    stats.order_ms = ms(sorted);

    listings
        .open
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id, Arc::new(Listed { entries, index }));
    outbox.send(encode_order(&order, stats.order_ms));
    stats.total_ms = ms(started);
    outbox.send(encode_done(&stats));
    if is_startup {
        startup::mark("listing:done");
    }
}

/// Spike #11: starts the first folder's listing right away, before Tauri and WebView2 start.
/// The UI picks it up with `listing_startup`.
pub fn start_early(listings: &Arc<Listings>, count: u32) {
    let outbox = Outbox::default();
    let id = start(listings, count, outbox.clone(), true);
    *listings
        .startup
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some((id, outbox));
}

/// The first folder's listing. If Rust started it early, the messages so far arrive at once and
/// the rest follow; otherwise the listing starts now. Returns the listing id.
#[tauri::command]
#[specta::specta]
pub async fn listing_startup(
    listings: State<'_, Arc<Listings>>,
    on_event: Channel<Bytes>,
) -> Result<u32, IpcError> {
    startup::mark("listing:attach");
    let early = listings
        .startup
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    Ok(match early {
        Some((id, outbox)) => {
            outbox.attach(on_event);
            id
        }
        None => start(
            &listings,
            startup::options().count,
            Outbox::to_channel(on_event),
            true,
        ),
    })
}

/// Opens the synthetic folder for `count` entries (1000, 10000 or 100000) and streams it as
/// binary batches. Returns the listing id.
#[tauri::command]
#[specta::specta]
pub async fn listing_open(
    listings: State<'_, Arc<Listings>>,
    count: u32,
    on_event: Channel<Bytes>,
) -> Result<u32, IpcError> {
    Ok(start(&listings, count, Outbox::to_channel(on_event), false))
}

/// A new display order for listing `id`, as a binary order message (ids as `u32`).
#[tauri::command]
#[specta::specta]
pub async fn listing_sort(
    listings: State<'_, Arc<Listings>>,
    id: u32,
    column: SortBy,
    descending: bool,
) -> Result<Bytes, IpcError> {
    let listed = listings
        .get(id)
        .ok_or_else(|| IpcError::code(IpcErrorCode::NotFound))?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    rayon::spawn(move || {
        let started = Instant::now();
        let order = listed
            .index
            .order(&listed.entries, column.into(), descending, true);
        let _ = sender.send(encode_order(&order, ms(started)));
    });
    receiver
        .await
        .map(Bytes)
        .map_err(|_| IpcError::code(IpcErrorCode::Cancelled))
}

/// Forgets listing `id` (and stops it if it is still running).
#[tauri::command]
#[specta::specta]
pub async fn listing_close(listings: State<'_, Arc<Listings>>, id: u32) -> Result<(), IpcError> {
    if let Some(cancel) = listings
        .running
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&id)
    {
        cancel.store(true, Ordering::Relaxed);
    }
    let removed = listings
        .open
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&id);
    // Free the columns off the async runtime.
    rayon::spawn(move || drop(removed));
    Ok(())
}
