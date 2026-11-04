//! Jerry's Tauri shell: windows, IPC commands, events and protocols. Thin glue only: file work
//! lives in `jerry-core`, Shell and COM work in `jerry-win`.

#[cfg(debug_assertions)]
mod dev;
pub mod icons;
pub mod ipc;
pub mod jobs;
pub mod listing;
pub mod logging;
pub mod storage;
pub mod window;

use std::time::{Duration, Instant};

use specta_typescript::{Typescript, semantic};
use tauri::{AppHandle, Manager, RunEvent, WebviewWindow};
use tauri_specta::{Builder, collect_commands, collect_events};

/// Where `pnpm bindings` writes the generated TypeScript.
const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/ipc/bindings.ts");

/// How long to wait for the UI's `app_ready` before showing the window anyway.
const SHOW_FALLBACK: Duration = Duration::from_secs(5);

/// The bindings' `typedError`: a command's rejection becomes `{ status: "error", error }` only
/// when it is a `JerryError`. Anything else (an unknown command, a bad argument) is a bug and
/// stays thrown, so the error type in the bindings never lies.
///
/// tauri-specta follows it with `_assertTypedErrorFollowsContract`, a constant nothing reads, which
/// fails our `noUnusedLocals`; the trailing `@ts-ignore` covers that next line.
const TYPED_ERROR: &str = r#"async function typedError<T, E>(result: Promise<T>): Promise<{ status: "ok"; data: T } | { status: "error"; error: E }> {
    try {
        return { status: "ok", data: await result };
    } catch (e) {
        if (typeof e === "object" && e !== null && "code" in e) return { status: "error", error: e as E };
        throw e instanceof Error ? e : new Error(String(e));
    }
}

// @ts-ignore: tauri-specta's unused contract check below fails noUnusedLocals."#;

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
fn app_ready(app: AppHandle, window: WebviewWindow) {
    window::before_show(&app);
    show(&window);
}

fn show(window: &WebviewWindow) {
    if let Err(error) = window.show() {
        tracing::error!(%error, "failed to show the window");
    }
}

/// Every command the UI may call; dev builds add the dev panel's (`dev.rs`). The same list
/// feeds the invoke handler and the bindings.
macro_rules! commands {
    ($($dev:tt)*) => {
        collect_commands![
            app_version,
            app_ready,
            listing::listing_open,
            listing::listing_sort,
            listing::listing_close,
            storage::settings_get,
            storage::settings_set,
            storage::keybindings_get,
            window::appearance_get,
            $($dev)*
        ]
    };
}

fn specta_builder() -> Builder<tauri::Wry> {
    #[cfg(debug_assertions)]
    let commands = commands![
        dev::dev_sample_error,
        dev::dev_panic_in_job,
        dev::dev_block_main_thread,
        dev::dev_long_task,
        dev::dev_stream,
        dev::dev_synth_folder,
        dev::dev_icon_stats,
        dev::dev_open_window,
    ];
    #[cfg(not(debug_assertions))]
    let commands = commands![];

    Builder::<tauri::Wry>::new()
        .commands(commands)
        .events(collect_events![
            storage::SettingsChanged,
            window::AppearanceChanged
        ])
        // One TypeScript type per Rust type. With serde phases a `Channel<T>` argument got T's
        // deserialize shape (`T_Deserialize`), though Rust only ever serializes T.
        .disable_serde_phases()
        // `f64` as `number`, not `number | null`: Jerry never sends NaN or infinity.
        .semantic_types(semantic::Configuration::empty().enable_lossless_floats())
        .typed_error_impl(TYPED_ERROR)
}

/// Writes the IPC types to `src/ipc/bindings.ts` (`pnpm bindings`).
pub fn export_bindings() -> Result<(), specta_typescript::Error> {
    specta_builder().export(Typescript::default(), BINDINGS_PATH)
}

/// Builds and runs the app until it exits.
pub fn run() -> tauri::Result<()> {
    logging::init();
    // Before the Shell pool starts below: this thread must be the process's main STA, not a pool
    // worker (`enter_sta` says why).
    if let Err(error) = jerry_win::main_thread::enter_sta() {
        tracing::error!(%error, "the main thread couldn't enter its STA");
    }
    // Opens on its own thread while Tauri and WebView2 start; commands wait for it.
    let db = storage::Db::open(storage::db_path());
    let builder = specta_builder();

    // Keep the bindings fresh while developing; CI checks the committed file.
    #[cfg(debug_assertions)]
    if let Err(error) = export_bindings() {
        tracing::error!(%error, "failed to export IPC bindings");
    }

    let app = tauri::Builder::default()
        .manage(db.clone())
        .manage(listing::Listings::default())
        .manage(jerry_win::icons::Icons::new())
        .register_asynchronous_uri_scheme_protocol(icons::SCHEME, icons::protocol)
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            tracing::info!(
                version = %app.package_info().version,
                os_build = jerry_win::os_build(),
                "Jerry starting"
            );
            if let Err(error) = jerry_win::main_thread::init() {
                tracing::error!(%error, "main thread setup failed: OLE and run_on_main won't work");
            }

            // The window gets its theme and Mica at creation, so it needs the settings now.
            let waited = Instant::now();
            let settings = db.settings_within(window::SETTINGS_WAIT);
            tracing::info!(
                ms = waited.elapsed().as_millis(),
                "appearance settings for the main window"
            );
            app.manage(window::Look::start(app.handle()));
            let window = window::create_main(app, &settings.unwrap_or_default())?;

            // If the UI never reports ready (a script error, say), show the window anyway so
            // Jerry isn't left running invisibly.
            std::thread::spawn(move || {
                std::thread::sleep(SHOW_FALLBACK);
                if !window.is_visible().unwrap_or(true) {
                    tracing::warn!("UI did not report ready; showing the window anyway");
                    show(&window);
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())?;

    app.run(|_, event| {
        if let RunEvent::Exit = event {
            tracing::info!("Jerry exiting");
            logging::flush();
        }
    });
    Ok(())
}
