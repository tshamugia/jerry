//! One Jerry (#43): a single instance, its command line, and staying in the background
//! (ADR-0005).
//!
//! - `jerry.exe [--background] [path…]`. Each path opens in a new tab in the active pane of the
//!   last active window, read as the address bar reads text but relative to the folder Jerry was
//!   started from; a file opens its folder with the file selected.
//! - One Jerry runs at a time (`tauri-plugin-single-instance`): a second start hands its
//!   arguments and folder to the running one and exits. The running one brings the last active
//!   window to the front, shown again if it was hidden, then opens the paths.
//! - What a start opens waits here until its window's page takes it (`launch_take`): once its
//!   tabs are restored, and on every `launch:opened`. So nothing is lost while a page loads.
//! - While "Keep Jerry running in the background" is on, closing the last window hides it, and
//!   the next start shows it at once. "Exit Jerry" (`app_exit`) quits.
//! - `--background` (autostart, #85): the main window is made but stays hidden until a start
//!   asks for it.
//!
//! Debug builds are their own instance, one per build folder ([`scope_to_build`]), so the dev
//! apps of parallel worktrees and an installed Jerry don't hand starts to each other.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use jerry_core::JerryError;
use jerry_core::address::{self, Context};
use jerry_core::path::long_path;
use jerry_win::folders::by_short_name;
use serde::Serialize;
use specta::Type;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, CloseRequestApi, Manager, State, WebviewWindow, Window, Wry};
use tauri_specta::Event;

use crate::jobs;
use crate::listing::Listings;
use crate::storage::Db;
use crate::window::MAIN;

/// The argument that starts Jerry hidden (autostart, #85).
pub const BACKGROUND: &str = "--background";

/// What a start asks for: `jerry.exe [--background] [path…]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Launch {
    /// Start hidden; if Jerry runs already, stay as it is.
    pub background: bool,
    /// Folders or files to open, as given.
    pub paths: Vec<String>,
}

impl Launch {
    /// A start's arguments, the program's own path first, as `std::env::args` gives them.
    /// Unknown `--` options are left out.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut launch = Self::default();
        for arg in args.into_iter().skip(1) {
            if arg.eq_ignore_ascii_case(BACKGROUND) {
                launch.background = true;
            } else if arg.starts_with("--") {
                tracing::warn!(arg, "ignoring an unknown argument");
            } else if !arg.trim().is_empty() {
                launch.paths.push(unescaped(arg));
            }
        }
        launch
    }

    /// This process's own start.
    pub fn from_env() -> Self {
        Self::parse(std::env::args_os().map(|arg| arg.to_string_lossy().into_owned()))
    }
}

/// `"D:\"` on a command line arrives as `D:"`: the backslash escapes the closing quote, as
/// Windows splits command lines. No file name holds a quote, so a lone trailing one stood for a
/// backslash.
fn unescaped(mut arg: String) -> String {
    if arg.ends_with('"') && arg.matches('"').count() % 2 == 1 {
        arg.pop();
        arg.push('\\');
    }
    arg
}

/// What a start opens in a window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum LaunchOpen {
    /// A folder, in a new tab.
    Folder {
        /// The folder, as a token for `ListingTarget::Folder`.
        folder: u32,
        /// A file in it to select: the start named a file.
        select: Option<String>,
    },
    /// A path that leads nowhere.
    Failed {
        /// The path as the start gave it.
        path: String,
        error: JerryError,
    },
}

/// Sent to every window when a start left something to open: each page takes its own with
/// `launch_take`.
#[derive(Debug, Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "launch:opened")]
pub struct LaunchOpened;

/// The windows' part in starts and closes, managed by the app.
pub struct Instance(Mutex<Windows>);

#[derive(Debug, Default)]
struct Windows {
    /// Pages that reported ready (`app_ready`): their windows can show at once.
    ready: HashSet<String>,
    /// Windows that stay hidden until a start asks for them: closed to the background, or the
    /// main window of a `--background` start.
    hidden: HashSet<String>,
    /// The window that last had the focus: a start goes there.
    last_active: Option<String>,
    /// What each window opens, until its page takes it.
    pending: HashMap<String, Vec<LaunchOpen>>,
}

impl Instance {
    /// The state for this process's `launch`: with `--background`, the main window stays hidden.
    pub fn new(launch: &Launch) -> Self {
        let mut windows = Windows::default();
        if launch.background {
            windows.hidden.insert(MAIN.into());
        }
        Self(Mutex::new(windows))
    }

    /// The page of window `label` reported ready: whether the window shows now. A window a
    /// start keeps hidden shows on the next start instead.
    pub fn page_ready(&self, label: &str) -> bool {
        let mut windows = self.windows();
        windows.ready.insert(label.into());
        !windows.hidden.contains(label)
    }

    /// Whether window `label` should show though its page never reported ready (a script error):
    /// so Jerry isn't left running invisibly.
    pub fn shows_unready(&self, label: &str) -> bool {
        let windows = self.windows();
        !windows.ready.contains(label) && !windows.hidden.contains(label)
    }

    fn windows(&self) -> MutexGuard<'_, Windows> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The single-instance plugin: a second start's arguments come to the running Jerry.
pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_single_instance::init(|app, args, from| {
        second_start(app, Launch::parse(args), PathBuf::from(from));
    })
}

/// Whether another Jerry with this `identifier` runs: its single-instance window is there. Then
/// this start only hands its arguments over (the plugin does, in `tauri::Builder::build`), so
/// nothing else needs to start. The names are the plugin's own (2.5): should they change, this
/// says no and the start runs as a first one until the plugin finds the other Jerry itself.
pub fn another_running(identifier: &str) -> bool {
    let class = windows::core::HSTRING::from(format!("{identifier}-sic"));
    let title = windows::core::HSTRING::from(format!("{identifier}-siw"));
    // SAFETY: two valid, NUL-terminated strings that outlive the call.
    unsafe { windows::Win32::UI::WindowsAndMessaging::FindWindowW(&class, &title) }.is_ok()
}

/// Debug builds: the single instance is this build's own, named after its folder. The dev apps of
/// parallel worktrees, and an installed Jerry, would otherwise hand starts to each other. Each
/// build folder also gets its own WebView2 profile, which Tauri names after the identifier.
#[cfg(debug_assertions)]
pub fn scope_to_build(context: &mut tauri::Context<Wry>) {
    let exe = std::env::current_exe().unwrap_or_default();
    let folder = exe.parent().unwrap_or(&exe);
    let config = context.config_mut();
    config.identifier = format!(
        "{}.dev{:08x}",
        config.identifier,
        fnv1a(folder.as_os_str().as_encoded_bytes())
    );
}

/// A hash that stays the same from build to build (std's may change with the toolchain).
#[cfg(debug_assertions)]
fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

/// This process's start: its paths open in the main window, after the session's tabs.
pub fn first_start(app: &AppHandle, launch: Launch) {
    let from = std::env::current_dir().unwrap_or_default();
    open(app, MAIN, launch.paths, from);
}

/// Another `jerry.exe` started: the last active window comes to the front, unless the start
/// only asked Jerry to run in the background, and its paths open there.
fn second_start(app: &AppHandle, launch: Launch, from: PathBuf) {
    tracing::info!(
        paths = launch.paths.len(),
        background = launch.background,
        "Jerry started again"
    );
    let Some(window) = target(app) else {
        tracing::error!("no window for another start");
        return;
    };
    if !launch.background {
        bring_to_front(app, &window);
    }
    open(app, window.label(), launch.paths, from);
}

/// The window a start goes to: the one last active, else the main one, else any.
fn target(app: &AppHandle) -> Option<WebviewWindow> {
    let last = app.state::<Instance>().windows().last_active.clone();
    last.and_then(|label| app.get_webview_window(&label))
        .or_else(|| app.get_webview_window(MAIN))
        .or_else(|| app.webview_windows().into_values().next())
}

/// Shows `window` in front of other apps, restored if it was minimized; a window whose page
/// isn't ready yet shows once it is (`app_ready`).
fn bring_to_front(app: &AppHandle, window: &WebviewWindow) {
    let instance = app.state::<Instance>();
    let ready = {
        let mut windows = instance.windows();
        windows.hidden.remove(window.label());
        windows.ready.contains(window.label())
    };
    if !ready {
        return;
    }
    // The second start let this process take the foreground (`AllowSetForegroundWindow`).
    let shown = window.show().and_then(|()| {
        if window.is_minimized()? {
            window.unminimize()?;
        }
        window.set_focus()
    });
    if let Err(error) = shown {
        tracing::error!(%error, "can't bring the window to the front");
    }
}

/// Resolves `paths` off the main thread, relative to `from`, and leaves them for window
/// `label`'s page.
fn open(app: &AppHandle, label: &str, paths: Vec<String>, from: PathBuf) {
    if paths.is_empty() {
        return;
    }
    let app = app.clone();
    let label = label.to_owned();
    let listings = app.state::<Listings>().inner().clone();
    tauri::async_runtime::spawn(async move {
        let resolved = jobs::background("launch paths", move || {
            Ok(resolve_all(&listings, &paths, &from))
        })
        .await;
        // A panic is logged by the job.
        let Ok(opens) = resolved else { return };
        app.state::<Instance>()
            .windows()
            .pending
            .entry(label)
            .or_default()
            .extend(opens);
        if let Err(error) = LaunchOpened.emit(&app) {
            tracing::error!(%error, "can't send launch:opened");
        }
    });
}

/// What each of `paths` opens, as the address bar reads them, relative to `from`.
fn resolve_all(listings: &Listings, paths: &[String], from: &Path) -> Vec<LaunchOpen> {
    let base = long_path(from).ok();
    let var = |name: &str| std::env::var_os(name);
    let context = Context {
        base: base.as_deref(),
        known: &by_short_name,
        var: &var,
    };
    paths
        .iter()
        .map(|path| match address::resolve(path, &context) {
            Ok(resolved) => LaunchOpen::Folder {
                folder: listings.folder_token(&resolved.folder),
                select: resolved
                    .select
                    .map(|name| name.to_string_lossy().into_owned()),
            },
            Err(error) => {
                tracing::warn!(code = ?error.code, "a start's path leads nowhere");
                LaunchOpen::Failed {
                    path: path.clone(),
                    error,
                }
            }
        })
        .collect()
}

/// Closing `window` hides it instead when it is the last one shown and "Keep Jerry running in
/// the background" is on (ADR-0005): the next start shows it at once.
pub fn close_requested(window: &Window, api: &CloseRequestApi) {
    let app = window.app_handle();
    let keep_running = app
        .try_state::<Db>()
        .and_then(|db| db.settings_within(Duration::ZERO))
        .unwrap_or_default()
        .keep_running;
    let others_shown = app
        .webview_windows()
        .values()
        .any(|other| other.label() != window.label() && other.is_visible().unwrap_or(false));
    if !keep_running || others_shown {
        return;
    }
    api.prevent_close();
    if let Some(instance) = app.try_state::<Instance>() {
        instance.windows().hidden.insert(window.label().into());
    }
    match window.hide() {
        Ok(()) => tracing::info!("closed the last window; Jerry keeps running in the background"),
        Err(error) => tracing::error!(%error, "can't hide the window"),
    }
}

/// `window` got the focus: starts go to it from now on.
pub fn focused(window: &Window) {
    if let Some(instance) = window.app_handle().try_state::<Instance>() {
        instance.windows().last_active = Some(window.label().into());
    }
}

/// `window` is gone: forget it.
pub fn destroyed(window: &Window) {
    if let Some(instance) = window.app_handle().try_state::<Instance>() {
        let mut windows = instance.windows();
        let label = window.label();
        windows.ready.remove(label);
        windows.hidden.remove(label);
        windows.pending.remove(label);
        if windows.last_active.as_deref() == Some(label) {
            windows.last_active = None;
        }
    }
}

/// What starts left for this window to open, in order; each only once.
#[tauri::command]
#[specta::specta]
pub fn launch_take(instance: State<'_, Instance>, window: WebviewWindow) -> Vec<LaunchOpen> {
    instance
        .windows()
        .pending
        .remove(window.label())
        .unwrap_or_default()
}

/// "Exit Jerry": quits, even while "Keep Jerry running in the background" is on. The session is
/// written on the way out.
#[tauri::command]
#[specta::specta]
pub fn app_exit(app: AppHandle) {
    tracing::info!("Exit Jerry");
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use jerry_core::ErrorCode;

    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("jerry.exe")
            .chain(list.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn a_start_reads_its_paths_and_the_background_option() {
        assert_eq!(Launch::parse(args(&[])), Launch::default());
        let launch = Launch::parse(args(&[
            r"D:\Projects",
            "--background",
            r"C:\ნახაზები\a.dwg",
        ]));
        assert!(launch.background);
        assert_eq!(launch.paths, [r"D:\Projects", r"C:\ნახაზები\a.dwg"]);
        let launch = Launch::parse(args(&["--BACKGROUND", "--unknown", "  "]));
        assert_eq!(
            launch,
            Launch {
                background: true,
                paths: vec![]
            }
        );
    }

    #[test]
    fn a_quoted_root_gets_its_backslash_back() {
        // `jerry.exe "D:\"` and `jerry.exe "C:\Some folder\"`, as Windows splits them.
        assert_eq!(Launch::parse(args(&["D:\""])).paths, [r"D:\"]);
        assert_eq!(
            Launch::parse(args(&[r#"C:\Some folder""#])).paths,
            [r"C:\Some folder\"]
        );
        // Quotes that pair up are the address bar's to take off.
        assert_eq!(Launch::parse(args(&[r#""C:\x""#])).paths, [r#""C:\x""#]);
    }

    #[test]
    fn paths_open_as_folders_or_files_relative_to_where_jerry_started() {
        let dir = tempfile::tempdir().expect("temp dir");
        let drawings = dir.path().join("ნახაზები");
        std::fs::create_dir(&drawings).expect("mkdir");
        std::fs::write(drawings.join("გეგმა.dwg"), b"x").expect("write");
        let listings = Listings::default();
        let paths = [
            "ნახაზები".to_owned(),
            drawings.join("გეგმა.dwg").to_string_lossy().into_owned(),
            "nowhere".to_owned(),
        ];
        let opens = resolve_all(&listings, &paths, dir.path());
        let folder = listings.folder_token(&long_path(&drawings).expect("long path"));
        assert_eq!(
            opens[0],
            LaunchOpen::Folder {
                folder,
                select: None
            }
        );
        assert_eq!(
            opens[1],
            LaunchOpen::Folder {
                folder,
                select: Some("გეგმა.dwg".into())
            }
        );
        match &opens[2] {
            LaunchOpen::Failed { path, error } => {
                assert_eq!(path, "nowhere");
                assert_eq!(error.code, ErrorCode::NotFound);
            }
            other => panic!("a failure, not {other:?}"),
        }
    }

    #[test]
    fn a_background_start_keeps_the_main_window_hidden_until_asked() {
        let instance = Instance::new(&Launch {
            background: true,
            paths: vec![],
        });
        assert!(!instance.shows_unready(MAIN));
        assert!(!instance.page_ready(MAIN), "stays hidden");
        instance.windows().hidden.remove(MAIN);
        assert!(instance.page_ready(MAIN), "a start asked for it");

        let normal = Instance::new(&Launch::default());
        assert!(normal.shows_unready(MAIN));
        assert!(normal.page_ready(MAIN));
        assert!(!normal.shows_unready(MAIN), "ready: the page showed it");
    }

    #[test]
    #[cfg(debug_assertions)]
    fn the_build_hash_stays_the_same() {
        assert_eq!(fnv1a(b""), 0x811c_9dc5);
        assert_eq!(fnv1a(b"a"), 0xe40c_292c);
    }
}
