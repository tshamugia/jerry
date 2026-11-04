//! Jerry's windows: created hidden and transparent (Mica shows through), with the theme's
//! background, with WebView2's browser keys turned off, and kept in step with the appearance
//! settings (theme, Mica) and the Windows accent.
//!
//! The window's Tauri `theme` follows the theme setting (`None` = follow Windows). WebView2's
//! `prefers-color-scheme` follows the window's theme, so the page reads the System theme from
//! that media query.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use jerry_core::settings::{Settings, Theme};
use jerry_win::appearance::{self as look, AccentColors, AccentWatch};
use jerry_win::main_thread::run_on_main;
use serde::Serialize;
use specta::Type;
use tauri::window::Color;
use tauri::{AppHandle, Manager, State, WebviewWindow, WebviewWindowBuilder, Wry};
use tauri_specta::Event;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Controller, ICoreWebView2Settings3,
};
use windows::core::Interface;

use crate::storage::{Db, SettingsChanged};

/// The main window's label (`tauri.conf.json`).
pub const MAIN: &str = "main";

/// How long the start waits for the database to learn the theme before it creates the main
/// window with the defaults. The database usually opens in 2–10 ms, in parallel with Tauri.
pub const SETTINGS_WAIT: Duration = Duration::from_millis(250);

/// `--jerry-mica` from design/tokens.css: the window's background where no Mica shows, until the
/// page paints.
const MICA_LIGHT: Color = Color(0xed, 0xef, 0xf2, 0xff);
const MICA_DARK: Color = Color(0x20, 0x20, 0x20, 0xff);

/// What the page needs from Windows to look right.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct Appearance {
    /// Mica can show behind the windows (Windows 11 22H2 and later). With the Mica setting on,
    /// the page leaves its Mica areas transparent; otherwise it paints them.
    pub mica: bool,
    /// The Windows accent, for the accent setting "Match Windows"; empty if Windows didn't tell.
    pub accent: Option<WindowsAccent>,
}

/// The Windows accent as CSS colors (`#rrggbb`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct WindowsAccent {
    /// On light surfaces.
    pub light: String,
    /// On dark surfaces.
    pub dark: String,
}

impl From<AccentColors> for WindowsAccent {
    fn from(colors: AccentColors) -> Self {
        Self {
            light: format!("#{:06x}", colors.light),
            dark: format!("#{:06x}", colors.dark),
        }
    }
}

/// Sent to every window when the Windows accent changes or Mica stops working.
#[derive(Debug, Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "appearance:changed")]
pub struct AppearanceChanged(pub Appearance);

/// The appearance, managed by the app.
pub struct Look {
    appearance: Mutex<Appearance>,
    /// The theme and Mica setting last applied to the windows, so other settings' changes skip
    /// the windows.
    applied: Mutex<Option<(Theme, bool)>>,
    _accent: Option<AccentWatch>,
}

impl Look {
    /// Reads the Windows accent and starts following it.
    pub fn start(app: &AppHandle) -> Self {
        let accent = look::accent_colors()
            .inspect_err(|error| tracing::warn!(%error, "can't read the Windows accent"))
            .ok();
        let handle = app.clone();
        let watch = AccentWatch::start(move |colors| {
            update(&handle, |appearance| {
                appearance.accent = Some(colors.into());
            });
        })
        .inspect_err(|error| tracing::warn!(%error, "can't follow the Windows accent"))
        .ok();
        Self {
            appearance: Mutex::new(Appearance {
                mica: look::mica_supported(),
                accent: accent.map(Into::into),
            }),
            applied: Mutex::new(None),
            _accent: watch,
        }
    }
}

/// Changes the appearance and tells every window, if anything changed.
fn update(app: &AppHandle, change: impl FnOnce(&mut Appearance)) {
    let Some(state) = app.try_state::<Look>() else {
        return;
    };
    let mut appearance = state
        .appearance
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let before = appearance.clone();
    change(&mut appearance);
    if *appearance != before {
        tracing::info!(?appearance, "appearance changed");
        if let Err(error) = AppearanceChanged(appearance.clone()).emit(app) {
            tracing::error!(%error, "can't send appearance:changed");
        }
    }
}

/// Whether Mica can show and the Windows accent.
#[tauri::command]
#[specta::specta]
pub fn appearance_get(look: State<'_, Look>) -> Appearance {
    look.appearance
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Creates the main window from its config (`create: false`) with `settings`' appearance, and
/// keeps every window's theme and Mica in step with the settings from now on.
pub fn create_main(app: &tauri::App, settings: &Settings) -> tauri::Result<WebviewWindow> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN)
        .cloned()
        .unwrap_or_default();
    let window = build(
        WebviewWindowBuilder::from_config(app.handle(), &config)?,
        settings,
    )?;
    if let Some(look) = app.try_state::<Look>() {
        *look.applied.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((settings.theme, settings.mica));
    }

    let handle = app.handle().clone();
    SettingsChanged::listen_any(app, move |event| apply(&handle, &event.payload.0));
    Ok(window)
}

/// Builds a Jerry window, hidden until its page reports ready (`app_ready`), with `settings`'
/// theme and Mica, and WebView2's browser behavior turned off.
pub fn build<M: Manager<Wry>>(
    builder: WebviewWindowBuilder<'_, Wry, M>,
    settings: &Settings,
) -> tauri::Result<WebviewWindow> {
    let mica = settings.mica && look::mica_supported();
    let mut builder = builder
        .visible(false)
        .transparent(true)
        .theme(window_theme(settings.theme));
    if !mica {
        // Shows until the page's first frame: without it the window flashes white (spike #1).
        builder = builder.background_color(background(settings.theme));
    }
    let window = builder.build()?;
    if mica {
        set_mica(&window, true);
    }
    lock_down(&window);
    Ok(window)
}

/// Applies the theme and Mica settings to every window, if they changed since the last time.
pub fn apply(app: &AppHandle, settings: &Settings) {
    let Some(look) = app.try_state::<Look>() else {
        return;
    };
    let wanted = (settings.theme, settings.mica);
    {
        let mut applied = look.applied.lock().unwrap_or_else(PoisonError::into_inner);
        if *applied == Some(wanted) {
            return;
        }
        *applied = Some(wanted);
    }
    for window in app.webview_windows().into_values() {
        if let Err(error) = window.set_theme(window_theme(settings.theme)) {
            tracing::error!(%error, "can't set the window theme");
        }
        if look::mica_supported() {
            set_mica(&window, settings.mica);
        }
    }
}

/// The window's Tauri theme: `None` follows Windows.
fn window_theme(theme: Theme) -> Option<tauri::Theme> {
    match theme {
        Theme::System => None,
        Theme::Light => Some(tauri::Theme::Light),
        Theme::Dark => Some(tauri::Theme::Dark),
    }
}

fn background(theme: Theme) -> Color {
    let dark = match theme {
        Theme::System => look::apps_use_dark_theme(),
        Theme::Light => false,
        Theme::Dark => true,
    };
    if dark { MICA_DARK } else { MICA_LIGHT }
}

/// Turns Mica on or off behind `window`, on the main thread. If it fails, the page stops
/// counting on it and paints the Mica areas itself.
fn set_mica(window: &WebviewWindow, enabled: bool) {
    let hwnd = match window.hwnd() {
        Ok(hwnd) => hwnd.0 as isize,
        Err(error) => {
            tracing::error!(%error, "no window handle");
            return;
        }
    };
    let app = window.app_handle().clone();
    let queued = run_on_main("set Mica", move || {
        if let Err(error) = look::set_mica(hwnd, enabled) {
            tracing::error!(%error, "can't set Mica; painting the background instead");
            update(&app, |appearance| appearance.mica = false);
        }
    });
    if let Err(error) = queued {
        tracing::error!(%error, "can't queue the Mica change");
    }
}

/// Turns off WebView2's browser behavior: its keyboard shortcuts (Ctrl+P prints, Ctrl+F finds,
/// Ctrl+R and F5 reload; those keys are Jerry's) and its default context menu, which dev builds
/// keep for "Inspect". Zoom is off already: Tauri's `zoomHotkeysEnabled` defaults to false.
fn lock_down(window: &WebviewWindow) {
    let result = window.with_webview(|webview| {
        // SAFETY: Tauri runs this on the main thread with the webview's live controller.
        if let Err(error) = unsafe { lock_down_webview(&webview.controller()) } {
            tracing::error!(%error, "can't turn off WebView2's browser keys");
        }
    });
    if let Err(error) = result {
        tracing::error!(%error, "can't reach the WebView2 settings");
    }
}

/// # Safety
/// Call on the main thread with a live controller.
unsafe fn lock_down_webview(controller: &ICoreWebView2Controller) -> windows::core::Result<()> {
    // SAFETY: plain property setters on the controller's own settings object.
    unsafe {
        let settings = controller.CoreWebView2()?.Settings()?;
        settings.SetAreDefaultContextMenusEnabled(cfg!(debug_assertions))?;
        settings
            .cast::<ICoreWebView2Settings3>()?
            .SetAreBrowserAcceleratorKeysEnabled(false)
    }
}

/// Applies the settings' appearance to the windows once more right before one shows: if the
/// database opened too late for the window's start, the window was built with the defaults.
pub fn before_show(app: &AppHandle) {
    let settings = app
        .try_state::<Db>()
        .and_then(|db| db.settings_within(Duration::ZERO));
    if let Some(settings) = settings {
        apply(app, &settings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_theme_follows_windows() {
        assert_eq!(window_theme(Theme::System), None);
        assert_eq!(window_theme(Theme::Dark), Some(tauri::Theme::Dark));
        assert_eq!(background(Theme::Light), MICA_LIGHT);
        assert_eq!(background(Theme::Dark), MICA_DARK);
    }

    #[test]
    fn accent_colors_become_css() {
        let accent = WindowsAccent::from(AccentColors {
            light: 0x005fb8,
            dark: 0x60cdff,
        });
        assert_eq!(accent.light, "#005fb8");
        assert_eq!(accent.dark, "#60cdff");
    }
}
