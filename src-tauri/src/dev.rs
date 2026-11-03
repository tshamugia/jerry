//! Dev builds only: commands behind the UI's dev panel, for checking the IPC and error
//! foundation by hand in the running app. Release builds have none of them.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use jerry_core::JerryError;
use tauri::ipc::Channel;
use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};

use crate::ipc::{Bytes, message};
use crate::jobs;

/// Fails with a real Windows error: a background job reads a file that doesn't exist, so the
/// UI gets `NotFound` with the path.
#[tauri::command]
#[specta::specta]
pub async fn dev_sample_error() -> Result<(), JerryError> {
    jobs::background("dev sample error", || {
        let missing = std::env::temp_dir()
            .join("jerry-dev-missing")
            .join("ნახაზი.dwg");
        std::fs::metadata(&missing)
            .map(|_| ())
            .map_err(|error| JerryError::from(error).at(missing))
    })
    .await
}

/// Panics on purpose inside a background job: the job fails with `Internal`, Jerry keeps
/// running and the log has the panic.
#[tauri::command]
#[specta::specta]
pub async fn dev_panic_in_job() -> Result<(), JerryError> {
    jobs::background::<()>("dev panic", || {
        panic!("deliberate panic from the dev panel")
    })
    .await
}

/// Keeps the main thread busy for `ms`, so the dev watchdog logs a frozen window.
#[tauri::command]
#[specta::specta]
pub async fn dev_block_main_thread(ms: u32) -> Result<(), JerryError> {
    jobs::on_main("dev block", move || {
        std::thread::sleep(Duration::from_millis(ms.into()));
        Ok(())
    })
    .await
}

/// The page reported a long task (a `PerformanceObserver` "longtask" entry): the UI was frozen
/// for `duration_ms`.
#[tauri::command]
#[specta::specta]
pub async fn dev_long_task(duration_ms: f64) {
    let ms = duration_ms.round() as u64;
    tracing::warn!(ms, "UI long task: the page was frozen");
}

/// Streams three binary messages (`[1, index, 0, 0]` + the index as an `f64`) through
/// `on_message`.
#[tauri::command]
#[specta::specta]
pub async fn dev_stream(on_message: Channel<Bytes>) -> Result<(), JerryError> {
    for index in 0..3_u32 {
        let body = f64::from(index).to_le_bytes();
        on_message
            .send(message([1, index, 0, 0], &body))
            .map_err(|error| {
                tracing::warn!(%error, "dev stream: send failed");
                JerryError::internal()
            })?;
    }
    Ok(())
}

/// Opens another window on the same UI, so events can be checked across windows before Jerry
/// has windows of its own (v1.0). It has Windows' title bar, to move and close it with the mouse.
#[tauri::command]
#[specta::specta]
pub async fn dev_open_window(app: AppHandle) -> Result<(), JerryError> {
    static OPENED: AtomicU32 = AtomicU32::new(0);
    let label = format!("dev-{}", OPENED.fetch_add(1, Ordering::Relaxed) + 1);
    WebviewWindowBuilder::new(&app, label, WebviewUrl::default())
        .title("Jerry (dev window)")
        .inner_size(900.0, 600.0)
        .disable_drag_drop_handler()
        .visible(false) // `app_ready` shows it
        .build()
        .map(|_| ())
        .map_err(|error| {
            tracing::error!(%error, "can't open a dev window");
            JerryError::internal()
        })
}
