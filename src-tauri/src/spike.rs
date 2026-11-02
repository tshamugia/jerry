//! Spike #16: Windows' own context menu ("Windows options…") in the real app. Throwaway glue
//! around a lab folder in `%TEMP%`; what v1.0 keeps is `jerry_win::context_menu` and the
//! threading here.
//!
//! Threading:
//! - `windows_menu` is `async`, so Tauri runs it on the tokio pool. A sync command would run
//!   the menu's modal loop on the main thread inside WebView2's IPC callback.
//! - From the pool, `jerry_win::main_thread::run_on_main` posts the menu to a Jerry-owned
//!   window: it runs on the main STA thread outside any tao callback, so Rust → page events keep
//!   flowing while the menu is up. The spike checks that with `MenuOpened` (emitted from a
//!   worker thread) → `menu_ack`.
//!   `JERRY_SPIKE_TAO=1` uses Tauri's `run_on_main_thread` instead, to show the difference.
//! - The command resolves once the menu closed and the Shell has the chosen command.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jerry_win::context_menu::{self, MenuError, MenuOptions, MenuOutcome};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, WebviewWindow};
use tauri_specta::Event;

/// The lab folder and every path the UI knows by id (paths stay in Rust).
pub struct Lab {
    root: PathBuf,
    items: Mutex<Vec<PathBuf>>,
    seq: AtomicU32,
    /// The last `menu_ack`: its menu and when it arrived.
    ack: Mutex<Option<(u32, Instant)>>,
    /// How long the startup prewarm took, once it's done.
    prewarm: Arc<Mutex<Option<f64>>>,
}

impl Lab {
    pub fn new() -> Self {
        Self {
            root: std::env::temp_dir().join("jerry-spike-16"),
            items: Mutex::default(),
            seq: AtomicU32::new(1),
            ack: Mutex::default(),
            prewarm: Arc::default(),
        }
    }

    /// Loads the Shell's menu handlers on a background STA thread, so the first Windows menu
    /// is fast and complete. `JERRY_SPIKE_NO_PREWARM=1` skips it, to measure without.
    pub fn prewarm(&self) {
        if std::env::var_os("JERRY_SPIKE_NO_PREWARM").is_some() {
            return;
        }
        let prewarm = Arc::clone(&self.prewarm);
        context_menu::spawn_prewarm(move |result| match result {
            Ok(elapsed) => {
                tracing::info!(ms = ms(elapsed), "Windows menu prewarmed");
                if let Ok(mut prewarm) = prewarm.lock() {
                    *prewarm = Some(ms(elapsed));
                }
            }
            Err(error) => tracing::warn!(%error, "Windows menu prewarm failed"),
        });
    }
}

/// Error codes for the spike's commands; the UI turns them into text.
#[derive(Debug, Serialize, specta::Type)]
#[serde(tag = "code")]
pub enum SpikeError {
    /// The id isn't one the UI got from `lab_items`.
    UnknownId,
    /// Items of different folders (search results) can't share one Windows menu.
    MixedFolders,
    /// A Windows menu is already open.
    Busy,
    /// The Shell can't resolve a file (gone since the listing).
    NotFound,
    Io,
    Os,
}

impl From<MenuError> for SpikeError {
    fn from(error: MenuError) -> Self {
        match error {
            MenuError::Empty => Self::UnknownId,
            MenuError::MixedFolders => Self::MixedFolders,
            MenuError::Path { .. } => Self::NotFound,
            MenuError::Busy => Self::Busy,
            MenuError::Os(error) => {
                tracing::error!(%error, "Windows menu failed");
                Self::Os
            }
        }
    }
}

/// What a lab item is for.
#[derive(Clone, Copy, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum LabKind {
    File,
    Folder,
    /// Deep enough that its path is longer than MAX_PATH.
    Long,
    /// In another folder, for the "mixed folders" case.
    Elsewhere,
}

/// One item of the lab. The UI refers to it by `id` only.
#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LabItem {
    id: u32,
    /// Display text only, never used to build a path.
    name: String,
    kind: LabKind,
}

/// Creates the lab (what's missing of it) and lists its items.
#[tauri::command]
#[specta::specta]
pub async fn lab_items(lab: State<'_, Lab>) -> Result<Vec<LabItem>, SpikeError> {
    let root = lab.root.clone();
    let seeded = tauri::async_runtime::spawn_blocking(move || seed(&root))
        .await
        .map_err(|_| SpikeError::Io)?
        .map_err(|error| {
            tracing::error!(%error, "seeding the lab failed");
            SpikeError::Io
        })?;
    let mut items = lab.items.lock().map_err(|_| SpikeError::Io)?;
    *items = seeded.iter().map(|(path, _)| path.clone()).collect();
    Ok(seeded
        .into_iter()
        .enumerate()
        .map(|(id, (path, kind))| LabItem {
            id: id as u32,
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            kind,
        })
        .collect())
}

/// The lab's items, created if missing. Never deletes anything: archives the menu commands
/// made stay until the tester removes them.
fn seed(root: &Path) -> std::io::Result<Vec<(PathBuf, LabKind)>> {
    let project = root.join("პროექტი");
    let other = root.join("other");
    let mut long = root.to_path_buf();
    for level in 1..=6 {
        long.push(format!("ნახაზები-{level}-{}", "x".repeat(40)));
    }
    let items = vec![
        (root.join("ნახაზი — სართული 1.dwg"), LabKind::File),
        (root.join("ხარჯთაღრიცხვა.pdf"), LabKind::File),
        (root.join("plain.txt"), LabKind::File),
        (project.clone(), LabKind::Folder),
        (long.join("გრძელი გზა.txt"), LabKind::Long),
        (other.join("elsewhere.txt"), LabKind::Elsewhere),
    ];
    // Core-style verbatim paths, so the long one can be created whatever LongPathsEnabled is.
    let verbatim = |path: &Path| PathBuf::from(format!(r"\\?\{}", path.display()));
    fs::create_dir_all(verbatim(&project))?;
    fs::create_dir_all(verbatim(&other))?;
    fs::create_dir_all(verbatim(&long))?;
    let inner = project.join("inner.txt");
    for path in items
        .iter()
        .filter(|(_, kind)| !matches!(kind, LabKind::Folder))
        .map(|(path, _)| path)
        .chain(Some(&inner))
    {
        if !path.exists() && !verbatim(path).exists() {
            fs::write(verbatim(path), b"jerry spike 16")?;
        }
    }
    // The menu gets core's form of the long path; jerry-win strips the prefix for the Shell.
    Ok(items
        .into_iter()
        .map(|(path, kind)| match kind {
            LabKind::Long => (verbatim(&path), kind),
            _ => (path, kind),
        })
        .collect())
}

/// Sent while a Windows menu is up (from inside its modal loop); the page answers with
/// `menu_ack`.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct MenuOpened {
    seq: u32,
}

/// The page got `MenuOpened`.
#[tauri::command]
#[specta::specta]
pub fn menu_ack(lab: State<'_, Lab>, seq: u32) {
    if let Ok(mut ack) = lab.ack.lock() {
        *ack = Some((seq, Instant::now()));
    }
}

/// How a Windows menu went, with the spike's measurements.
#[derive(Clone, Debug, Default, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MenuReport {
    seq: u32,
    /// `run_on_main` ("jerry") or Tauri's `run_on_main_thread` ("tao").
    via: String,
    chosen: Option<String>,
    verb: Option<String>,
    /// False for verbs Jerry runs itself (rename, delete, cut, copy, paste).
    invoked: bool,
    /// Command → menu task running on the main thread.
    queued_ms: f64,
    resolve_ms: f64,
    ui_object_ms: f64,
    query_ms: f64,
    menu_ms: f64,
    invoke_ms: Option<f64>,
    popups: Vec<PopupReport>,
    init_popup: u32,
    measure_item: u32,
    draw_item: u32,
    menu_char: u32,
    /// The page's answer to `MenuOpened` reached Rust before the menu closed.
    ack_during_menu: bool,
    /// Menu open → page's answer, if it came.
    ack_ms: Option<f64>,
    /// The startup prewarm's time, if it had finished when the menu was asked for.
    prewarm_ms: Option<f64>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PopupReport {
    text: String,
    ms: f64,
    items_before: i32,
    items_after: i32,
}

type MenuResult = (
    Result<MenuOutcome, MenuError>,
    Instant,
    Option<Instant>,
    Instant,
);

/// Shows Windows' context menu for `ids` (one folder's items) at the page point `(x, y)`
/// (whole CSS pixels in the window's client area). Shift held = extended verbs.
#[tauri::command]
#[specta::specta]
pub async fn windows_menu(
    app: AppHandle,
    window: WebviewWindow,
    lab: State<'_, Lab>,
    ids: Vec<u32>,
    x: i32,
    y: i32,
    extended: bool,
) -> Result<MenuReport, SpikeError> {
    let received = Instant::now();
    let paths = {
        let items = lab.items.lock().map_err(|_| SpikeError::Io)?;
        ids.iter()
            .map(|&id| items.get(id as usize).cloned().ok_or(SpikeError::UnknownId))
            .collect::<Result<Vec<_>, _>>()?
    };
    let owner = window.hwnd().map_err(|_| SpikeError::Os)?.0 as isize;
    let scale = window.scale_factor().map_err(|_| SpikeError::Os)?;
    let (screen_x, screen_y) = context_menu::client_to_screen(
        owner,
        (f64::from(x) * scale).round() as i32,
        (f64::from(y) * scale).round() as i32,
    );
    let seq = lab.seq.fetch_add(1, Ordering::Relaxed);
    let prewarm_ms = lab.prewarm.lock().ok().and_then(|prewarm| *prewarm);
    let via_tao = std::env::var_os("JERRY_SPIKE_TAO").is_some();

    let (sender, mut receiver) = tauri::async_runtime::channel::<MenuResult>(1);
    let task = move || {
        let started = Instant::now();
        let opened = Arc::new(Mutex::new(None));
        let opened_at = Arc::clone(&opened);
        let options = MenuOptions {
            extended,
            on_open: Some(Box::new(move || {
                if let Ok(mut opened) = opened_at.lock() {
                    *opened = Some(Instant::now());
                }
                // From a worker thread, as transfer progress would come: those events go through
                // tao's event loop (an emit on the main thread runs the script at once).
                std::thread::spawn(move || {
                    if let Err(error) = (MenuOpened { seq }).emit(&app) {
                        tracing::error!(%error, "emitting MenuOpened failed");
                    }
                });
            })),
            ..Default::default()
        };
        let result = context_menu::show(owner, &paths, screen_x, screen_y, options);
        let closed = Instant::now();
        let opened = opened.lock().ok().and_then(|opened| *opened);
        // Capacity 1 and nothing else sends, so this can't fail for want of room.
        let _ = sender.try_send((result, started, opened, closed));
    };
    if via_tao {
        window
            .run_on_main_thread(task)
            .map_err(|_| SpikeError::Os)?;
    } else if jerry_win::main_thread::run_on_main(Box::new(task)).is_err() {
        return Err(SpikeError::Os);
    }
    let (result, started, opened, closed) = receiver.recv().await.ok_or(SpikeError::Os)?;
    let outcome = result?;

    let ack = lab
        .ack
        .lock()
        .ok()
        .and_then(|ack| *ack)
        .filter(|&(acked, _)| acked == seq)
        .map(|(_, at)| at);
    let t = &outcome.timings;
    let report = MenuReport {
        seq,
        via: if via_tao { "tao" } else { "jerry" }.to_owned(),
        chosen: outcome.chosen.as_ref().map(|chosen| chosen.text.clone()),
        verb: outcome
            .chosen
            .as_ref()
            .and_then(|chosen| chosen.verb.clone()),
        invoked: outcome.chosen.as_ref().is_some_and(|chosen| chosen.invoked),
        queued_ms: ms(started.duration_since(received)),
        resolve_ms: ms(t.resolve),
        ui_object_ms: ms(t.ui_object),
        query_ms: ms(t.query),
        menu_ms: ms(t.menu),
        invoke_ms: t.invoke.map(ms),
        popups: t
            .popups
            .iter()
            .map(|popup| PopupReport {
                text: popup.text.clone(),
                ms: ms(popup.elapsed),
                items_before: popup.items_before,
                items_after: popup.items_after,
            })
            .collect(),
        init_popup: outcome.forwarded.init_popup,
        measure_item: outcome.forwarded.measure_item,
        draw_item: outcome.forwarded.draw_item,
        menu_char: outcome.forwarded.menu_char,
        ack_during_menu: ack.is_some_and(|at| at < closed),
        ack_ms: match (opened, ack) {
            (Some(opened), Some(at)) if at >= opened => Some(ms(at - opened)),
            _ => None,
        },
        prewarm_ms,
    };
    write_log(&report);
    Ok(report)
}

/// One line per menu in `%TEMP%\jerry-spike-16.log`, for the E2E script.
fn write_log(report: &MenuReport) {
    let path = std::env::temp_dir().join("jerry-spike-16.log");
    let line = format!("{report:?}\n");
    let written = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(line.as_bytes()));
    if let Err(error) = written {
        tracing::error!(%error, "writing the spike log failed");
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
