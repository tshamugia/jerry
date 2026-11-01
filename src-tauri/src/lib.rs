//! Jerry's Tauri shell: windows, IPC commands, events and protocols. Thin glue only: file work
//! lives in `jerry-core`, Shell and COM work in `jerry-win`.

mod spike;

use std::time::Duration;

use specta_typescript::Typescript;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_specta::{Builder, collect_commands};

/// Where `pnpm bindings` writes the generated TypeScript.
const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/ipc/bindings.ts");

/// How long to wait for the UI's `app_ready` before showing the window anyway.
const SHOW_FALLBACK: Duration = Duration::from_secs(5);

/// The app version, from the workspace `Cargo.toml`.
#[tauri::command]
#[specta::specta]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Called by the UI once its first frame is painted. The window is created hidden and shown
/// only now, so it never flashes white.
#[tauri::command]
#[specta::specta]
fn app_ready(window: WebviewWindow) {
    show(&window);
}

fn show(window: &WebviewWindow) {
    if let Err(error) = window.show() {
        tracing::error!(%error, "failed to show the window");
    }
}

/// Every command the UI may call. The same list feeds the invoke handler and the bindings.
fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![
        app_version,
        app_ready,
        spike::sample_list,
        spike::drag_start,
        spike::spike_window,
    ])
}

/// Writes the IPC types to `src/ipc/bindings.ts` (`pnpm bindings`).
pub fn export_bindings() -> Result<(), specta_typescript::Error> {
    specta_builder().export(Typescript::default(), BINDINGS_PATH)
}

/// Builds and runs the app until its last window closes.
pub fn run() -> tauri::Result<()> {
    spike::init_logging();
    let builder = specta_builder();

    // Keep the bindings fresh while developing; CI checks the committed file.
    #[cfg(debug_assertions)]
    if let Err(error) = export_bindings() {
        tracing::error!(%error, "failed to export IPC bindings");
    }

    tauri::Builder::default()
        .manage(spike::Samples::new())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            tracing::info!(
                core = jerry_core::VERSION,
                os_build = jerry_win::os_build(),
                "Jerry starting"
            );

            // setup runs on the main thread, where OLE drag & drop and the clipboard live.
            if let Err(error) = jerry_win::drag::init_ole() {
                tracing::error!(%error, "OleInitialize failed; drag & drop won't work");
            }

            // If the UI never reports ready (a script error, say), show the window anyway so
            // Jerry isn't left running invisibly.
            if let Some(window) = app.get_webview_window("main") {
                std::thread::spawn(move || {
                    std::thread::sleep(SHOW_FALLBACK);
                    if !window.is_visible().unwrap_or(true) {
                        tracing::warn!("UI did not report ready; showing the window anyway");
                        show(&window);
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
}
