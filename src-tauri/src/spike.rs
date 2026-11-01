//! Spikes #13 and #14: dragging files out of Jerry and into it with Windows OLE. Throwaway glue
//! around a fixed sample folder (the "lab"); what v0.2 keeps is `jerry_win::drag`,
//! `jerry_win::drop` and the threading here and in `spike_drop`.
//!
//! Threading of a drag out (spikes #13 and #14):
//! - `drag_start` is `async`, so Tauri runs it on the tokio pool. A sync command would run on
//!   the main thread inside WebView2's IPC callback, and the drag's modal loop with it.
//! - From the pool, `jerry_win::drag::run_on_main` posts the drag to a Jerry-owned window. It
//!   runs on the main STA thread, outside any WebView2 or tao callback, while the user still
//!   holds the button. (Tauri's `run_on_main_thread` would run it inside tao's event callback,
//!   where tao holds back the Rust → page events Jerry's drop target sends during the drag.)
//! - The command resolves when the drop or cancel is done, so the UI gets the outcome.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use jerry_win::drag::{self, DropEffect};
use serde::Serialize;
use tauri::{State, WebviewWindow};

/// The lab folder and every path the UI knows by id (paths stay in Rust). Ids are handed out
/// once per path and never reused, so an id stays valid across listings.
pub struct Lab {
    pub root: PathBuf,
    registry: Mutex<Registry>,
    dragging: AtomicBool,
}

#[derive(Default)]
struct Registry {
    paths: Vec<PathBuf>,
    ids: HashMap<PathBuf, u32>,
}

impl Registry {
    fn id(&mut self, path: &Path) -> u32 {
        if let Some(&id) = self.ids.get(path) {
            return id;
        }
        let id = self.paths.len() as u32;
        self.paths.push(path.to_path_buf());
        self.ids.insert(path.to_path_buf(), id);
        id
    }
}

impl Lab {
    pub fn new() -> Self {
        Self {
            root: std::env::temp_dir().join("jerry-spike-14"),
            registry: Mutex::default(),
            dragging: AtomicBool::new(false),
        }
    }

    /// The path behind `id`, if the UI got it from us.
    pub fn path(&self, id: u32) -> Option<PathBuf> {
        let registry = self.registry.lock().ok()?;
        registry.paths.get(id as usize).cloned()
    }

    fn id(&self, path: &Path) -> Result<u32, SpikeError> {
        Ok(self.registry.lock().map_err(|_| SpikeError::Io)?.id(path))
    }
}

/// Window actions for the spike's bare title bar. The real one is #17's; these exist only so
/// the undecorated window can be moved next to a drop target and closed.
#[derive(Debug, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum WindowAction {
    Move,
    Close,
}

/// Moves (title bar drag) or closes the window.
#[tauri::command]
#[specta::specta]
pub fn spike_window(window: WebviewWindow, action: WindowAction) {
    let result = match action {
        WindowAction::Move => window.start_dragging(),
        WindowAction::Close => window.close(),
    };
    if let Err(error) = result {
        tracing::error!(%error, ?action, "window action failed");
    }
}

/// Error codes for the spike's commands; the UI turns them into text.
#[derive(Debug, Serialize, specta::Type)]
#[serde(tag = "code")]
pub enum SpikeError {
    /// The id isn't one the UI got from a listing (list again).
    UnknownId,
    /// A drag is already running.
    Busy,
    /// The Shell can't resolve a file (gone since the listing).
    NotFound,
    Io,
    Os,
}

/// A folder of the lab, for tabs, breadcrumb and sidebar.
#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LabFolder {
    id: u32,
    /// Display text only, never used to build a path.
    name: String,
    parent: Option<u32>,
    depth: u32,
}

/// Lists the lab's folders (root first, depth-first), creating the lab and its seed first.
#[tauri::command]
#[specta::specta]
pub async fn lab_folders(lab: State<'_, Lab>) -> Result<Vec<LabFolder>, SpikeError> {
    let root = lab.root.clone();
    let folders = tauri::async_runtime::spawn_blocking(move || {
        jerry_core::sample::ensure(&root)?;
        jerry_core::sample::folders(&root, 3)
    })
    .await
    .map_err(|_| SpikeError::Io)?
    .map_err(io_error)?;

    let mut out = Vec::with_capacity(folders.len());
    for (path, depth) in folders {
        let parent = match path.parent() {
            Some(parent) if depth > 0 => Some(lab.id(parent)?),
            _ => None,
        };
        let name = if depth == 0 {
            "jerry-spike-14".to_owned()
        } else {
            display_name(&path)
        };
        out.push(LabFolder {
            id: lab.id(&path)?,
            name,
            parent,
            depth: depth as u32,
        });
    }
    Ok(out)
}

/// One row of a lab folder. The UI refers to it by `id` only.
#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LabEntry {
    id: u32,
    /// Display text only, never used to build a path.
    name: String,
    is_dir: bool,
    size: f64,
}

/// Lists the folder `id`.
#[tauri::command]
#[specta::specta]
pub async fn lab_list(lab: State<'_, Lab>, folder: u32) -> Result<Vec<LabEntry>, SpikeError> {
    let dir = lab.path(folder).ok_or(SpikeError::UnknownId)?;
    let entries = tauri::async_runtime::spawn_blocking(move || jerry_core::sample::list(&dir))
        .await
        .map_err(|_| SpikeError::Io)?
        .map_err(io_error)?;
    entries
        .into_iter()
        .map(|entry| {
            Ok(LabEntry {
                id: lab.id(&entry.path)?,
                name: entry.name.to_string_lossy().into_owned(),
                is_dir: entry.is_dir,
                size: entry.size as f64,
            })
        })
        .collect()
}

pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn io_error(error: std::io::Error) -> SpikeError {
    tracing::error!(%error, "lab io failed");
    SpikeError::Io
}

/// How a drag ended, with the spike's measurements.
#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DragReport {
    /// False when no mouse button was down any more once the main thread got to the drag.
    started: bool,
    dropped: bool,
    effect: Effect,
    /// The target's "Performed DropEffect", if it wrote one.
    performed: Option<Effect>,
    /// UI threshold crossing → command running in Rust (wall clock, both sides).
    ui_to_rust_ms: f64,
    /// Command → drag closure running on the main thread.
    queued_ms: f64,
    /// Building the Shell data object.
    prepare_ms: f64,
    /// The modal drag loop, from start to drop or cancel.
    drag_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Effect {
    None,
    Copy,
    Move,
    Link,
}

impl From<DropEffect> for Effect {
    fn from(effect: DropEffect) -> Self {
        match effect {
            DropEffect::None => Self::None,
            DropEffect::Copy => Self::Copy,
            DropEffect::Move => Self::Move,
            DropEffect::Link => Self::Link,
        }
    }
}

impl From<Effect> for DropEffect {
    fn from(effect: Effect) -> Self {
        match effect {
            Effect::None => Self::None,
            Effect::Copy => Self::Copy,
            Effect::Move => Self::Move,
            Effect::Link => Self::Link,
        }
    }
}

/// Clears the drag flag however the command ends.
struct DragFlag<'a>(&'a AtomicBool);

impl Drop for DragFlag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Starts an OLE drag of `ids`. Called by the UI once the pointer moves past the drag
/// threshold with a button held; resolves when the user drops or cancels. `ui_time` is the
/// UI's wall clock (ms since the epoch) at the threshold crossing.
#[tauri::command]
#[specta::specta]
pub async fn drag_start(
    window: WebviewWindow,
    lab: State<'_, Lab>,
    ids: Vec<u32>,
    ui_time: f64,
) -> Result<DragReport, SpikeError> {
    let received = Instant::now();
    let ui_to_rust_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|now| now.as_secs_f64() * 1000.0 - ui_time)
        .unwrap_or(f64::NAN);

    let paths = ids
        .iter()
        .map(|&id| lab.path(id).ok_or(SpikeError::UnknownId))
        .collect::<Result<Vec<_>, _>>()?;
    if lab.dragging.swap(true, Ordering::AcqRel) {
        return Err(SpikeError::Busy);
    }
    let _flag = DragFlag(&lab.dragging);
    let owner = window.hwnd().map_err(|_| SpikeError::Os)?.0 as isize;

    let (sender, mut receiver) = tauri::async_runtime::channel(1);
    // Not `app.run_on_main_thread`: inside its callback tao holds back the Rust → page events
    // the drop target sends during Jerry's own drag (spike #14).
    drag::run_on_main(Box::new(move || {
        let report = drag_on_main_thread(owner, &paths, received, ui_to_rust_ms);
        // Capacity 1 and nothing else sends, so this can't fail for want of room.
        let _ = sender.try_send(report);
    }))
    .map_err(|_| SpikeError::Os)?;
    receiver.recv().await.ok_or(SpikeError::Os)?
}

fn drag_on_main_thread(
    owner: isize,
    paths: &[PathBuf],
    received: Instant,
    ui_to_rust_ms: f64,
) -> Result<DragReport, SpikeError> {
    let queued_ms = ms(received.elapsed());
    let input = drag::input_state();
    tracing::info!(
        items = paths.len(),
        ui_to_rust_ms,
        queued_ms,
        "drag out requested"
    );

    let mut report = DragReport {
        started: false,
        dropped: false,
        effect: Effect::None,
        performed: None,
        ui_to_rust_ms,
        queued_ms,
        prepare_ms: 0.0,
        drag_ms: 0.0,
    };
    if !(input.left_down || input.right_down) {
        // Released before we got here: a drag now would drop at once, onto Jerry itself.
        tracing::warn!("mouse button already released; drag skipped");
        return Ok(report);
    }

    let prepare = Instant::now();
    let data = drag::data_object(paths).map_err(drag_error)?;
    report.prepare_ms = ms(prepare.elapsed());

    let start = Instant::now();
    let outcome = drag::drag(owner, &data).map_err(drag_error)?;
    report.drag_ms = ms(start.elapsed());
    report.started = true;
    report.dropped = outcome.dropped;
    report.effect = outcome.effect.into();
    report.performed = outcome.performed.map(Effect::from);
    tracing::info!(?outcome, drag_ms = report.drag_ms, "drag out finished");
    Ok(report)
}

fn drag_error(error: drag::DragError) -> SpikeError {
    tracing::error!(%error, "drag failed");
    match error {
        drag::DragError::Path { .. } => SpikeError::NotFound,
        drag::DragError::Empty => SpikeError::UnknownId,
        drag::DragError::Os(_) => SpikeError::Os,
    }
}

pub fn ms(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Where the spike logs, so manual test runs (and the probe) can read it afterwards. The real
/// log setup (`%LOCALAPPDATA%\Jerry\logs`, rotation) is its own issue.
pub fn log_path() -> PathBuf {
    std::env::temp_dir().join("jerry-spike-14.log")
}

pub fn init_logging() {
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    else {
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_max_level(tracing::Level::DEBUG)
        .with_thread_names(true)
        .try_init();
}
