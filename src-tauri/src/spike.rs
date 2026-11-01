//! Spike #13: dragging files out of Jerry with Windows OLE. Throwaway glue around a fixed
//! sample folder; what #58 keeps is `jerry_win::drag` and the threading here.
//!
//! Threading, the point of the spike:
//! - `drag_start` is `async`, so Tauri runs it on the tokio pool. A sync command would run on
//!   the main thread inside WebView2's IPC callback, and `run_on_main_thread` would then call
//!   the closure right there (tauri-runtime-wry does that on the main thread), so the drag's
//!   modal loop would run inside a WebView2 callback.
//! - From the pool, `run_on_main_thread` posts the drag to the event loop. It runs on the main
//!   STA thread, outside any WebView2 callback, while the user still holds the button.
//! - The command resolves when the drop or cancel is done, so the UI gets the outcome.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use jerry_win::drag::{self, DropEffect};
use serde::Serialize;
use tauri::{AppHandle, State, WebviewWindow};

/// The sample folder and the paths behind the ids the UI holds (paths stay in Rust).
pub struct Samples {
    root: PathBuf,
    paths: Mutex<Vec<PathBuf>>,
    dragging: AtomicBool,
}

impl Samples {
    pub fn new() -> Self {
        Self {
            root: std::env::temp_dir().join("jerry-spike-13"),
            paths: Mutex::default(),
            dragging: AtomicBool::new(false),
        }
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
    /// The id isn't in the current listing (list again).
    UnknownId,
    /// A drag is already running.
    Busy,
    /// The Shell can't resolve a file (gone since the listing).
    NotFound,
    Io,
    Os,
}

/// One row of the sample folder. The UI refers to it by `id` only.
#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SampleEntry {
    id: u32,
    /// Display text only, never used to build a path.
    name: String,
    is_dir: bool,
    size: f64,
}

/// Lists the sample folder, creating it and its seed files first.
#[tauri::command]
#[specta::specta]
pub async fn sample_list(samples: State<'_, Samples>) -> Result<Vec<SampleEntry>, SpikeError> {
    let root = samples.root.clone();
    let entries = tauri::async_runtime::spawn_blocking(move || {
        jerry_core::sample::ensure(&root)?;
        jerry_core::sample::list(&root)
    })
    .await
    .map_err(|_| SpikeError::Io)?
    .map_err(|error| {
        tracing::error!(%error, "listing the sample folder failed");
        SpikeError::Io
    })?;

    let mut paths = samples.paths.lock().map_err(|_| SpikeError::Io)?;
    *paths = entries.iter().map(|entry| entry.path.clone()).collect();
    Ok(entries
        .into_iter()
        .enumerate()
        .map(|(id, entry)| SampleEntry {
            id: id as u32,
            name: entry.name.to_string_lossy().into_owned(),
            is_dir: entry.is_dir,
            size: entry.size as f64,
        })
        .collect())
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
    /// The window that had the mouse capture when the drag started (class, and whose).
    capture: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, specta::Type)]
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
    app: AppHandle,
    window: WebviewWindow,
    samples: State<'_, Samples>,
    ids: Vec<u32>,
    ui_time: f64,
) -> Result<DragReport, SpikeError> {
    let received = Instant::now();
    let ui_to_rust_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|now| now.as_secs_f64() * 1000.0 - ui_time)
        .unwrap_or(f64::NAN);

    let paths = {
        let all = samples.paths.lock().map_err(|_| SpikeError::Io)?;
        ids.iter()
            .map(|&id| all.get(id as usize).cloned().ok_or(SpikeError::UnknownId))
            .collect::<Result<Vec<_>, _>>()?
    };
    if samples.dragging.swap(true, Ordering::AcqRel) {
        return Err(SpikeError::Busy);
    }
    let _flag = DragFlag(&samples.dragging);
    let owner = window.hwnd().map_err(|_| SpikeError::Os)?.0 as isize;

    let (sender, mut receiver) = tauri::async_runtime::channel(1);
    app.run_on_main_thread(move || {
        let report = drag_on_main_thread(owner, &paths, received, ui_to_rust_ms);
        // Capacity 1 and nothing else sends, so this can't fail for want of room.
        let _ = sender.try_send(report);
    })
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
    let capture = input.capture.as_ref().map(|window| {
        let whose = if window.ours {
            "jerry"
        } else {
            "other process"
        };
        format!("{} ({whose}, pid {})", window.class, window.process_id)
    });
    tracing::info!(
        thread = ?std::thread::current().name(),
        items = paths.len(),
        ui_to_rust_ms,
        queued_ms,
        ?input,
        "drag requested"
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
        capture,
    };
    if !(input.left_down || input.right_down) {
        // Released before we got here: a drag now would drop at once, onto Jerry itself.
        tracing::warn!("mouse button already released; drag skipped");
        return Ok(report);
    }

    let prepare = Instant::now();
    let data = drag::data_object(paths).map_err(drag_error)?;
    report.prepare_ms = ms(prepare.elapsed());
    tracing::debug!(formats = ?drag::format_names(&data), "data object ready");

    let start = Instant::now();
    let outcome = drag::drag(owner, &data).map_err(drag_error)?;
    report.drag_ms = ms(start.elapsed());
    report.started = true;
    report.dropped = outcome.dropped;
    report.effect = outcome.effect.into();
    report.performed = outcome.performed.map(Effect::from);
    tracing::info!(
        ?outcome,
        prepare_ms = report.prepare_ms,
        drag_ms = report.drag_ms,
        after = ?drag::input_state(),
        "drag finished"
    );
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

fn ms(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Logs to `%TEMP%\jerry-spike-13.log`, so manual test runs can be read afterwards. The real
/// log setup (`%LOCALAPPDATA%\Jerry\logs`, rotation) is its own issue.
pub fn init_logging() {
    let path = std::env::temp_dir().join("jerry-spike-13.log");
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_max_level(tracing::Level::DEBUG)
        .with_thread_names(true)
        .try_init();
}
