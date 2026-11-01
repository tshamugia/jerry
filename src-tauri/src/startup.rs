//! Spike #11: startup timing.
//!
//! Rust and the UI stamp each startup stage in ms since the Unix epoch (both read the system
//! clock), and the report goes to `JERRY_SPIKE_REPORT` as JSON with the start times and memory of
//! Jerry's process tree. `scripts/spike-11-summary.mjs` turns reports into the stage table.
//!
//! Switches (environment, read once; "1" turns a switch on, "0" off):
//! - `JERRY_SPIKE_REPORT=<file>`: write the report there once the first row is painted.
//! - `JERRY_SPIKE_EXIT=<ms>`: exit that long after the report.
//! - `JERRY_SPIKE_TOPMOST=1`: keep the window above others (the probe samples the screen).
//! - `JERRY_SPIKE_LISTING=early|ui`: start the first folder's listing at process start (default)
//!   or only when the UI asks for it.
//! - `JERRY_SPIKE_BG=1|0`: give the window and WebView2 the theme background (default on).
//! - `JERRY_SPIKE_SHOW=ready|early`: show the window after the first render (default) or right
//!   after it is created.
//! - `JERRY_SPIKE_COUNT=<n>`: the synthetic folder to open (default 1000).
//! - `JERRY_SPIKE_RESIDENT=<ms>`: stay hidden after the first render and show the window that
//!   much later, as a Jerry kept running in the background would on a launch or hotkey.
//! - `JERRY_SPIKE_TABS=<n,n,...>`: after startup, open more tabs' listings (synthetic folder
//!   sizes) and report memory before and after.

use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use specta::Type;
use tauri::AppHandle;
use tokio::sync::Notify;

/// The spike's switches.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub report: Option<PathBuf>,
    pub exit_after_ms: Option<u64>,
    pub topmost: bool,
    pub early_listing: bool,
    pub background: bool,
    pub show_early: bool,
    pub count: u32,
    pub tabs: Vec<u32>,
    pub resident_ms: Option<u64>,
}

/// The switches from the environment.
pub fn options() -> &'static Options {
    static OPTIONS: OnceLock<Options> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        let var = |name: &str| std::env::var(name).ok();
        let flag = |name: &str, default: bool| var(name).map_or(default, |value| value == "1");
        Options {
            report: var("JERRY_SPIKE_REPORT").map(PathBuf::from),
            exit_after_ms: var("JERRY_SPIKE_EXIT").and_then(|value| value.parse().ok()),
            topmost: flag("JERRY_SPIKE_TOPMOST", false),
            early_listing: var("JERRY_SPIKE_LISTING").is_none_or(|value| value != "ui"),
            background: flag("JERRY_SPIKE_BG", true),
            show_early: var("JERRY_SPIKE_SHOW").is_some_and(|value| value == "early"),
            count: var("JERRY_SPIKE_COUNT")
                .and_then(|value| value.parse().ok())
                .unwrap_or(1000),
            tabs: var("JERRY_SPIKE_TABS")
                .map(|value| {
                    value
                        .split(',')
                        .filter_map(|n| n.trim().parse().ok())
                        .collect()
                })
                .unwrap_or_default(),
            resident_ms: var("JERRY_SPIKE_RESIDENT").and_then(|value| value.parse().ok()),
        }
    })
}

/// The system clock in ms since the Unix epoch (`GetSystemTimePreciseAsFileTime`).
pub fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64() * 1000.0)
}

static MARKS: Mutex<Vec<(&str, f64)>> = Mutex::new(Vec::new());

/// Stamps a startup stage.
pub fn mark(name: &'static str) {
    let at = now_ms();
    MARKS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push((name, at));
}

/// A stage stamped in the UI: `at` is ms since the Unix epoch (`timeOrigin + now()`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct JsMark {
    pub name: String,
    pub at: f64,
}

/// What the UI reports: its stage stamps and loose values (bundle sizes, heap, clock check).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JsReport {
    /// "startup" after the first row is painted; "one-tab" and "tabs" in the memory test.
    pub phase: String,
    pub marks: Vec<JsMark>,
    pub values: Vec<JsMark>,
}

static PHASES: Mutex<Vec<Value>> = Mutex::new(Vec::new());

fn phase_json(report: &JsReport) -> Value {
    let processes: Vec<Value> = jerry_win::process::process_tree(std::process::id())
        .into_iter()
        .map(|p| {
            json!({
                "pid": p.pid,
                "parent": p.parent_pid,
                "name": p.name,
                "created": p.created_unix_ms,
                "privateBytes": p.private_bytes,
                "workingSet": p.working_set,
                "privateWorkingSet": p.private_working_set,
            })
        })
        .collect();
    json!({
        "phase": report.phase,
        "at": now_ms(),
        "js": report.marks,
        "values": report.values,
        "processes": processes,
    })
}

/// Takes one UI report. After the last phase, writes the whole report and exits if asked to.
pub fn take_report(app: &AppHandle, report: JsReport) {
    mark("report");
    let last = if !options().tabs.is_empty() {
        report.phase == "tabs"
    } else if options().resident_ms.is_some() {
        report.phase == "resident"
    } else {
        report.phase == "startup"
    };
    let mut phases = PHASES.lock().unwrap_or_else(PoisonError::into_inner);
    phases.push(phase_json(&report));
    if !last {
        return;
    }
    let marks: Vec<Value> = MARKS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(name, at)| json!({ "name": name, "at": at }))
        .collect();
    let whole = json!({
        "pid": std::process::id(),
        "processCreated": jerry_win::process::current_process_created_unix_ms(),
        "options": options(),
        "rust": marks,
        "phases": *phases,
    });
    if let Some(path) = &options().report
        && let Err(error) = std::fs::write(path, whole.to_string())
    {
        tracing::error!(%error, "failed to write the startup report");
    }
    if let Some(after) = options().exit_after_ms {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(after));
            app.exit(0);
        });
    }
}

/// The UI's startup stamps and values; see [`take_report`].
#[tauri::command]
#[specta::specta]
pub fn startup_report(app: AppHandle, report: JsReport) {
    take_report(&app, report);
}

static SHOWN: LazyLock<Notify> = LazyLock::new(Notify::new);

/// Resident mode: the window was shown (see `JERRY_SPIKE_RESIDENT`).
pub fn notify_shown() {
    SHOWN.notify_one();
}

/// Resident mode: resolves with true once the window is shown; false at once otherwise.
#[tauri::command]
#[specta::specta]
pub async fn startup_wait_shown() -> bool {
    if options().resident_ms.is_none() {
        return false;
    }
    SHOWN.notified().await;
    true
}

/// The folder sizes of the extra tabs the memory test opens (empty: no memory test).
#[tauri::command]
#[specta::specta]
pub fn startup_tabs() -> Vec<u32> {
    options().tabs.clone()
}
