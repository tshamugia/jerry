//! Spike #15: the clipboard between Jerry and Explorer. Throwaway glue around a fixed sample
//! folder; what v0.1 keeps is `jerry_win::clipboard` and the threading here.
//!
//! - Clipboard calls run on the main thread, the OLE STA. The commands are `async`, so Tauri
//!   runs them on the tokio pool, and `run_on_main_thread` posts the clipboard work to the event
//!   loop, outside WebView2's IPC callback (a sync command would run inside it, spike #13).
//! - The data object Jerry put on the clipboard is a COM object of the main thread's apartment,
//!   so it lives in a main-thread `thread_local`, not in Tauri state.
//! - Paste reads the clipboard on the main thread, copies or moves on a blocking thread, and
//!   goes back to the main thread to finish a pasted cut.
//! - A message-only window hears every clipboard change (`AddClipboardFormatListener`); the UI
//!   gets the new state on a Channel and drops the cut style once the clipboard isn't Jerry's.
//! - At exit, `OleFlushClipboard` keeps Jerry's files on the clipboard.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use jerry_win::clipboard::{self, ClipboardFiles, ClipboardListener, IDataObject, Op};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State, WebviewWindow};

thread_local! {
    /// The data object Jerry last put on the clipboard (main thread only).
    static OURS: RefCell<Option<IDataObject>> = const { RefCell::new(None) };
    /// The clipboard being pasted, between reading it and finishing the paste (main thread).
    static PASTING: RefCell<Option<ClipboardFiles>> = const { RefCell::new(None) };
    /// Keeps the clipboard listener alive (main thread).
    static LISTENER: RefCell<Option<ClipboardListener>> = const { RefCell::new(None) };
}

/// The sample folder, the paths behind the ids the UI holds (paths stay in Rust), and what Jerry
/// cut.
pub struct Spike {
    root: PathBuf,
    paths: Mutex<Vec<PathBuf>>,
    /// Paths of Jerry's cut while the clipboard holds it: the UI shows them dimmed.
    cut: Mutex<Vec<PathBuf>>,
    watcher: Mutex<Option<Channel<ClipboardState>>>,
    pasting: AtomicBool,
}

impl Spike {
    pub fn new() -> Self {
        Self {
            root: std::env::temp_dir().join("jerry-spike-15"),
            paths: Mutex::default(),
            cut: Mutex::default(),
            watcher: Mutex::default(),
            pasting: AtomicBool::new(false),
        }
    }
}

/// Starts the clipboard listener. Call from `setup`, on the main thread, after OLE is up.
pub fn listen(app: &AppHandle) {
    let handle = app.clone();
    let listener = ClipboardListener::new(move || {
        let state = state_on_main(&handle);
        tracing::debug!(?state, "clipboard changed");
        let spike = handle.state::<Spike>();
        let watcher = spike.watcher.lock().ok().and_then(|w| w.clone());
        if let Some(watcher) = watcher
            && let Err(error) = watcher.send(state)
        {
            tracing::warn!(%error, "couldn't send the clipboard state");
        }
    });
    match listener {
        Ok(listener) => LISTENER.with_borrow_mut(|slot| *slot = Some(listener)),
        Err(error) => tracing::error!(%error, "clipboard listener failed"),
    }
}

/// At exit, on the main thread: keeps Jerry's files on the clipboard after the process is gone.
pub fn flush_on_exit() {
    OURS.with_borrow(|ours| {
        if let Some(data) = ours {
            match clipboard::flush(data) {
                Ok(flushed) => tracing::info!(flushed, "clipboard at exit"),
                Err(error) => tracing::error!(%error, "OleFlushClipboard failed"),
            }
        }
    });
}

/// Window actions for the spike's bare title bar. The real one is #17's; these exist only so
/// the undecorated window can be moved next to Explorer and closed (closing tests the flush).
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
    /// A paste is already running.
    Busy,
    /// The clipboard holds no files.
    NothingToPaste,
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
pub async fn sample_list(spike: State<'_, Spike>) -> Result<Vec<SampleEntry>, SpikeError> {
    let root = spike.root.clone();
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

    let mut paths = spike.paths.lock().map_err(|_| SpikeError::Io)?;
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

#[derive(Clone, Copy, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ClipOp {
    Copy,
    Cut,
}

impl From<Op> for ClipOp {
    fn from(op: Op) -> Self {
        match op {
            Op::Copy => Self::Copy,
            Op::Cut => Self::Cut,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Owner {
    /// Nothing on the clipboard, or no owner window.
    None,
    Jerry,
    Other,
}

/// What the clipboard holds now.
#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardState {
    owner: Owner,
    owner_pid: Option<u32>,
    /// Files in CF_HDROP; 0 when it holds something else.
    files: u32,
    op: Option<ClipOp>,
    formats: Vec<String>,
    /// Rows of Jerry's cut, while the clipboard still holds it.
    cut_ids: Vec<u32>,
}

/// The clipboard state now; the same arrives on `clipboard_watch`'s channel on every change.
#[tauri::command]
#[specta::specta]
pub async fn clipboard_state(app: AppHandle) -> Result<ClipboardState, SpikeError> {
    let handle = app.clone();
    on_main(&app, move || state_on_main(&handle)).await
}

/// Sends the clipboard state to `on_change` on every clipboard change.
#[tauri::command]
#[specta::specta]
pub fn clipboard_watch(spike: State<'_, Spike>, on_change: Channel<ClipboardState>) {
    if let Ok(mut watcher) = spike.watcher.lock() {
        *watcher = Some(on_change);
    }
}

/// How Copy or Cut went, with the spike's measurements.
#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CopyReport {
    files: u32,
    op: ClipOp,
    /// Command → closure running on the main thread.
    queued_ms: f64,
    /// Building the Shell data object.
    prepare_ms: f64,
    /// Marking it and `OleSetClipboard`.
    set_ms: f64,
    formats: Vec<String>,
}

/// Copy (Ctrl+C) or Cut (Ctrl+X): puts the files behind `ids` on the clipboard, as Explorer does.
#[tauri::command]
#[specta::specta]
pub async fn clipboard_copy(
    app: AppHandle,
    spike: State<'_, Spike>,
    ids: Vec<u32>,
    cut: bool,
) -> Result<CopyReport, SpikeError> {
    let paths = {
        let all = spike.paths.lock().map_err(|_| SpikeError::Io)?;
        ids.iter()
            .map(|&id| all.get(id as usize).cloned().ok_or(SpikeError::UnknownId))
            .collect::<Result<Vec<_>, _>>()?
    };
    let op = if cut { Op::Cut } else { Op::Copy };
    let received = Instant::now();
    let handle = app.clone();
    on_main(&app, move || copy_on_main(&handle, paths, op, received)).await?
}

fn copy_on_main(
    app: &AppHandle,
    paths: Vec<PathBuf>,
    op: Op,
    received: Instant,
) -> Result<CopyReport, SpikeError> {
    let queued_ms = ms(received);
    let start = Instant::now();
    let data = clipboard::files_data_object(&paths).map_err(|error| {
        tracing::error!(%error, "data object failed");
        match error {
            clipboard::ClipboardError::Path { .. } => SpikeError::NotFound,
            clipboard::ClipboardError::Empty => SpikeError::UnknownId,
            clipboard::ClipboardError::Os(_) => SpikeError::Os,
        }
    })?;
    let prepare_ms = ms(start);
    // Before OleSetClipboard: the listener reads it when the change arrives.
    if let Ok(mut cut) = app.state::<Spike>().cut.lock() {
        *cut = if op == Op::Cut {
            paths.clone()
        } else {
            Vec::new()
        };
    }
    let start = Instant::now();
    clipboard::put(&data, op).map_err(|error| {
        tracing::error!(%error, "OleSetClipboard failed");
        SpikeError::Os
    })?;
    let set_ms = ms(start);
    let formats = clipboard::format_names(&data).unwrap_or_default();
    OURS.with_borrow_mut(|ours| *ours = Some(data));
    tracing::info!(
        files = paths.len(),
        ?op,
        queued_ms,
        prepare_ms,
        set_ms,
        ?formats,
        "copied"
    );
    Ok(CopyReport {
        files: paths.len() as u32,
        op: op.into(),
        queued_ms,
        prepare_ms,
        set_ms,
        formats,
    })
}

/// How Paste went.
#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PasteReport {
    op: ClipOp,
    /// The source's raw "Preferred DropEffect", if it wrote one.
    preferred_effect: Option<u32>,
    /// The clipboard was Jerry's own (a paste inside Jerry).
    from_jerry: bool,
    /// Names as they are now in the sample folder.
    pasted: Vec<String>,
    /// Names left alone, with why.
    skipped: Vec<String>,
    /// Reading CF_HDROP and the effect on the main thread.
    read_ms: f64,
    /// Copying or moving on a blocking thread.
    paste_ms: f64,
}

/// Paste (Ctrl+V): copies or moves the clipboard's files into the sample folder.
#[tauri::command]
#[specta::specta]
pub async fn clipboard_paste(
    app: AppHandle,
    spike: State<'_, Spike>,
) -> Result<PasteReport, SpikeError> {
    if spike.pasting.swap(true, Ordering::AcqRel) {
        return Err(SpikeError::Busy);
    }
    let _flag = Flag(&spike.pasting);

    let (paths, op, preferred_effect, from_jerry, read_ms) = on_main(&app, || {
        let start = Instant::now();
        let files = clipboard::get_files();
        let read_ms = ms(start);
        let from_jerry = clipboard::owner_process() == Some(clipboard::current_process());
        match files {
            Ok(Some(files)) => {
                let read = (
                    files.paths.clone(),
                    files.op,
                    files.preferred_effect,
                    from_jerry,
                    read_ms,
                );
                PASTING.with_borrow_mut(|pasting| *pasting = Some(files));
                Ok(read)
            }
            Ok(None) => Err(SpikeError::NothingToPaste),
            Err(error) => {
                tracing::error!(%error, "reading the clipboard failed");
                Err(SpikeError::Os)
            }
        }
    })
    .await??;
    tracing::info!(
        ?paths,
        ?op,
        ?preferred_effect,
        from_jerry,
        read_ms,
        "pasting"
    );

    let root = spike.root.clone();
    let start = Instant::now();
    let moving = op == Op::Cut;
    let pasted = tauri::async_runtime::spawn_blocking(move || {
        jerry_core::sample::paste(&paths, &root, moving)
    })
    .await
    .map_err(|_| SpikeError::Io);
    let paste_ms = ms(start);

    // Back on the main thread: tell the source and empty the clipboard after a cut (only if
    // something moved), and let go of the clipboard's object either way.
    let moved = matches!(&pasted, Ok(Ok(p)) if !p.done.is_empty());
    on_main(&app, move || {
        let files = PASTING.with_borrow_mut(Option::take);
        if let Some(files) = files
            && moving
            && moved
            && let Err(error) = clipboard::finish_cut(&files)
        {
            tracing::warn!(%error, "finishing the cut failed");
        }
    })
    .await?;

    let pasted = pasted?.map_err(|error| {
        tracing::error!(%error, "paste failed");
        SpikeError::Io
    })?;
    tracing::info!(?pasted, paste_ms, "pasted");
    Ok(PasteReport {
        op: op.into(),
        preferred_effect,
        from_jerry,
        pasted: pasted
            .done
            .iter()
            .map(|name| name.to_string_lossy().into_owned())
            .collect(),
        skipped: pasted
            .skipped
            .iter()
            .map(|(name, why)| format!("{} ({why})", name.to_string_lossy()))
            .collect(),
        read_ms,
        paste_ms,
    })
}

fn state_on_main(app: &AppHandle) -> ClipboardState {
    let spike = app.state::<Spike>();
    let jerry_owns = OURS.with_borrow(|ours| ours.as_ref().is_some_and(clipboard::is_current));
    let cut_ids = match (spike.cut.lock(), spike.paths.lock()) {
        (Ok(mut cut), Ok(paths)) => {
            if !jerry_owns {
                cut.clear();
            }
            paths
                .iter()
                .enumerate()
                .filter(|(_, path)| cut.contains(path))
                .map(|(id, _)| id as u32)
                .collect()
        }
        _ => Vec::new(),
    };
    let (files, op) = match clipboard::get_files() {
        Ok(Some(files)) => (files.paths.len() as u32, Some(files.op.into())),
        _ => (0, None),
    };
    let owner_pid = clipboard::owner_process();
    ClipboardState {
        owner: match (jerry_owns, owner_pid) {
            (true, _) => Owner::Jerry,
            (false, Some(_)) => Owner::Other,
            (false, None) => Owner::None,
        },
        owner_pid,
        files,
        op,
        formats: clipboard::current_formats().unwrap_or_default(),
        cut_ids,
    }
}

/// Runs `work` on the main thread (from the event loop, outside WebView2 callbacks) and waits
/// for its result.
async fn on_main<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, SpikeError> {
    let (sender, mut receiver) = tauri::async_runtime::channel(1);
    app.run_on_main_thread(move || {
        // Capacity 1 and nothing else sends, so this can't fail for want of room.
        let _ = sender.try_send(work());
    })
    .map_err(|_| SpikeError::Os)?;
    receiver.recv().await.ok_or(SpikeError::Os)
}

/// Clears the paste flag however the command ends.
struct Flag<'a>(&'a AtomicBool);

impl Drop for Flag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

/// Logs to `%TEMP%\jerry-spike-15.log`, so manual test runs can be read afterwards. The real
/// log setup (`%LOCALAPPDATA%\Jerry\logs`, rotation) is its own issue.
pub fn init_logging() {
    let path = std::env::temp_dir().join("jerry-spike-15.log");
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
