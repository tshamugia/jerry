//! Jerry's Tauri shell: windows, IPC commands, events and protocols. Thin glue only: file work
//! lives in `jerry-core`, Shell and COM work in `jerry-win`.

mod listing;
pub mod startup;

use std::sync::Arc;
use std::time::Duration;

use specta_typescript::Typescript;
use tauri::webview::PageLoadEvent;
use tauri::window::Color;
use tauri::{AppHandle, RunEvent, WebviewWindow, WebviewWindowBuilder};
use tauri_specta::{Builder, collect_commands};

/// Where `pnpm bindings` writes the generated TypeScript.
const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/ipc/bindings.ts");

/// How long to wait for the UI's `app_ready` before showing the window anyway.
const SHOW_FALLBACK: Duration = Duration::from_secs(5);

/// `--jerry-mica` from design/tokens.css: the window background before any web content paints.
const MICA_LIGHT: Color = Color(0xed, 0xef, 0xf2, 0xff);
const MICA_DARK: Color = Color(0x20, 0x20, 0x20, 0xff);

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
    startup::mark("app_ready");
    if let Some(after) = startup::options().resident_ms {
        // Spike #11: a Jerry kept in the background shows its ready window on a launch.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(after));
            startup::mark("resident:show");
            show(&window);
            startup::mark("shown");
            startup::notify_shown();
        });
        return;
    }
    show(&window);
    startup::mark("shown");
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
        listing::listing_startup,
        listing::listing_open,
        listing::listing_sort,
        listing::listing_close,
        startup::startup_report,
        startup::startup_tabs,
        startup::startup_wait_shown,
    ])
}

/// Writes the IPC types to `src/ipc/bindings.ts` (`pnpm bindings`).
pub fn export_bindings() -> Result<(), specta_typescript::Error> {
    specta_builder().export(Typescript::default(), BINDINGS_PATH)
}

/// Creates the main window from its config (`create: false`), hidden until the UI is ready.
fn create_main_window(app: &tauri::App) -> tauri::Result<WebviewWindow> {
    let options = startup::options();
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .cloned()
        .unwrap_or_default();
    let mut builder = WebviewWindowBuilder::from_config(app.handle(), &config)?;
    if options.background {
        let dark = jerry_win::apps_use_dark_theme();
        builder = builder.background_color(if dark { MICA_DARK } else { MICA_LIGHT });
    }
    if options.topmost {
        builder = builder.always_on_top(true);
    }
    if options.show_early {
        builder = builder.visible(true);
    }
    startup::mark("window:build");
    let window = builder.build()?;
    startup::mark("window:built");
    Ok(window)
}

/// Builds and runs the app until its last window closes.
pub fn run() -> tauri::Result<()> {
    startup::mark("run");
    let options = startup::options();
    let listings = Arc::new(listing::Listings::default());
    if options.early_listing {
        // The first folder lists while Tauri and WebView2 start; the UI takes the result.
        listing::start_early(&listings, options.count);
    }
    let builder = specta_builder();

    // Keep the bindings fresh while developing; CI checks the committed file.
    #[cfg(debug_assertions)]
    if let Err(error) = export_bindings() {
        tracing::error!(%error, "failed to export IPC bindings");
    }

    startup::mark("builder");
    let app = tauri::Builder::default()
        .manage(listings)
        .invoke_handler(builder.invoke_handler())
        .on_page_load(|_, payload| match payload.event() {
            PageLoadEvent::Started => startup::mark("page:started"),
            PageLoadEvent::Finished => startup::mark("page:finished"),
        })
        .setup(move |app| {
            startup::mark("setup");
            builder.mount_events(app);
            tracing::info!(
                core = jerry_core::VERSION,
                os_build = jerry_win::os_build(),
                "Jerry starting"
            );
            let window = create_main_window(app)?;

            // If the UI never reports ready (a script error, say), show the window anyway so
            // Jerry isn't left running invisibly.
            let fallback =
                SHOW_FALLBACK + Duration::from_millis(startup::options().resident_ms.unwrap_or(0));
            std::thread::spawn(move || {
                std::thread::sleep(fallback);
                if !window.is_visible().unwrap_or(true) {
                    tracing::warn!("UI did not report ready; showing the window anyway");
                    show(&window);
                }
            });
            startup::mark("setup:end");
            Ok(())
        })
        .build(tauri::generate_context!())?;
    startup::mark("built");
    app.run(|_, event| {
        if let RunEvent::Ready = event {
            startup::mark("ready");
        }
    });
    Ok(())
}
