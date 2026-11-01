//! Spike #12: folder listing over a Tauri `Channel` and sort orders as binary.
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
//!
//! `listing_open_json` sends the same data as JSON for comparison.

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
use tauri::ipc::{Channel, InvokeResponseBody, IpcResponse};
use tauri::{AppHandle, Manager, State};

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
    /// Bytes sent over the channel (binary format only).
    pub bytes: f64,
    pub first_batch_ms: f64,
    pub list_ms: f64,
    pub keys_ms: f64,
    pub name_sort_ms: f64,
    pub ranks_ms: f64,
    pub order_ms: f64,
    pub total_ms: f64,
}

/// Listings the UI has open, by id.
#[derive(Default)]
pub struct Listings {
    next_id: AtomicU32,
    open: Mutex<HashMap<u32, Arc<Listed>>>,
    running: Mutex<HashMap<u32, Arc<AtomicBool>>>,
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

/// Where a listing job sends its results.
trait Sink: Send + 'static {
    /// Sends entries `range`; returns the bytes sent (0 if unknown).
    fn batch(&mut self, entries: &Entries, range: Range<usize>) -> usize;
    fn order(&mut self, order: &[u32], rust_ms: f64);
    fn done(&mut self, stats: &ListingStats);
    fn error(&mut self, error: IpcError);
}

struct BinarySink(Channel<Bytes>);

impl BinarySink {
    fn send(&self, bytes: Vec<u8>) {
        if let Err(error) = self.0.send(Bytes(bytes)) {
            tracing::warn!(%error, "listing channel send failed");
        }
    }
}

impl Sink for BinarySink {
    fn batch(&mut self, entries: &Entries, range: Range<usize>) -> usize {
        let bytes = encode_batch(entries, range);
        let len = bytes.len();
        self.send(bytes);
        len
    }

    fn order(&mut self, order: &[u32], rust_ms: f64) {
        self.send(encode_order(order, rust_ms));
    }

    fn done(&mut self, stats: &ListingStats) {
        let mut out = Vec::with_capacity(16 + 8 * 8);
        for value in [MSG_DONE, stats.count, stats.batches, 0] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in [
            stats.bytes,
            stats.first_batch_ms,
            stats.list_ms,
            stats.keys_ms,
            stats.name_sort_ms,
            stats.ranks_ms,
            stats.order_ms,
            stats.total_ms,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        self.send(out);
    }

    fn error(&mut self, error: IpcError) {
        let mut out = Vec::with_capacity(16);
        for value in [MSG_ERROR, error.code as u32, error.os.unwrap_or(0), 0] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        self.send(out);
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

/// The JSON format, for comparison with the binary one.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JsonEvent {
    Batch(JsonBatch),
    Order(JsonOrder),
    Done(ListingStats),
    Error(IpcError),
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JsonBatch {
    first_id: u32,
    names: Vec<String>,
    sizes: Vec<f64>,
    modified: Vec<f64>,
    attributes: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JsonOrder {
    ids: Vec<u32>,
    rust_ms: f64,
}

struct JsonSink(Channel<JsonEvent>);

impl JsonSink {
    fn send(&self, event: JsonEvent) {
        if let Err(error) = self.0.send(event) {
            tracing::warn!(%error, "listing channel send failed");
        }
    }
}

impl Sink for JsonSink {
    fn batch(&mut self, entries: &Entries, range: Range<usize>) -> usize {
        self.send(JsonEvent::Batch(JsonBatch {
            first_id: range.start as u32,
            names: range
                .clone()
                .map(|id| String::from_utf16_lossy(entries.name_utf16(id)))
                .collect(),
            sizes: range.clone().map(|id| entries.sizes[id] as f64).collect(),
            modified: range
                .clone()
                .map(|id| filetime_to_unix_ms(entries.modified[id]))
                .collect(),
            attributes: range.map(|id| entries.attributes[id]).collect(),
        }));
        0
    }

    fn order(&mut self, order: &[u32], rust_ms: f64) {
        self.send(JsonEvent::Order(JsonOrder {
            ids: order.to_vec(),
            rust_ms,
        }));
    }

    fn done(&mut self, stats: &ListingStats) {
        self.send(JsonEvent::Done(*stats));
    }

    fn error(&mut self, error: IpcError) {
        self.send(JsonEvent::Error(error));
    }
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

/// Starts a listing of the synthetic folder for `count` on its own IO thread; returns its id.
fn start(
    app: AppHandle,
    listings: &Listings,
    count: u32,
    sink: impl Sink,
) -> Result<u32, IpcError> {
    let id = listings.next_id.fetch_add(1, Ordering::Relaxed);
    let cancel = Arc::new(AtomicBool::new(false));
    listings
        .running
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id, cancel.clone());
    std::thread::Builder::new()
        .name(format!("jerry-listing-{id}"))
        .spawn(move || {
            let job = AssertUnwindSafe(|| run(&app, id, count, &cancel, sink));
            if catch_unwind(job).is_err() {
                tracing::error!(id, "listing job panicked");
            }
            let listings = app.state::<Listings>();
            let mut running = listings
                .running
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            running.remove(&id);
        })
        .map_err(|error| IpcError::from(JerryError::from_io(&error, &synth::root())))?;
    Ok(id)
}

fn run(app: &AppHandle, id: u32, count: u32, cancel: &AtomicBool, mut sink: impl Sink) {
    let started = Instant::now();
    let dir = synth::dataset_dir(count as usize);
    let mut stats = ListingStats::default();
    let listed = list_dir(&dir, cancel, |entries, range| {
        if stats.batches == 0 {
            stats.first_batch_ms = ms(started);
        }
        stats.batches += 1;
        stats.bytes += sink.batch(entries, range) as f64;
    });
    let entries = match listed {
        Ok(entries) => entries,
        Err(error) => {
            sink.error(error.into());
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

    let listings = app.state::<Listings>();
    listings
        .open
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id, Arc::new(Listed { entries, index }));
    sink.order(&order, stats.order_ms);
    stats.total_ms = ms(started);
    sink.done(&stats);
}

/// Opens the synthetic folder for `count` entries (1000, 10000 or 100000) and streams it as
/// binary batches. Returns the listing id.
#[tauri::command]
#[specta::specta]
pub async fn listing_open(
    app: AppHandle,
    listings: State<'_, Listings>,
    count: u32,
    on_event: Channel<Bytes>,
) -> Result<u32, IpcError> {
    start(app, &listings, count, BinarySink(on_event))
}

/// Same as `listing_open`, as JSON events (format comparison).
#[tauri::command]
#[specta::specta]
pub async fn listing_open_json(
    app: AppHandle,
    listings: State<'_, Listings>,
    count: u32,
    on_event: Channel<JsonEvent>,
) -> Result<u32, IpcError> {
    start(app, &listings, count, JsonSink(on_event))
}

/// A new display order for listing `id`, as a binary order message (ids as `u32`).
#[tauri::command]
#[specta::specta]
pub async fn listing_sort(
    listings: State<'_, Listings>,
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
pub async fn listing_close(listings: State<'_, Listings>, id: u32) -> Result<(), IpcError> {
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
