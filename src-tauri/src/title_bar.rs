//! The custom title bar's IPC. React draws the tab strip and the caption buttons; `jerry-win`
//! answers the native hit-test over the buttons (Snap Layouts) and streams their state back
//! through a `Channel`, which needs no capability.

use jerry_win::title_bar as native;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::{Manager, State, WebviewWindow};

/// One of the three window buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CaptionButton {
    Minimize,
    Maximize,
    Close,
}

/// Where the UI drew a caption button, in CSS pixels from the top-right corner of the window.
#[derive(Clone, Copy, Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CaptionButtonRect {
    pub button: CaptionButton,
    /// Distance from the window's right edge to the button's right edge.
    pub right: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

/// Hover, pressed, maximized and active state of the window frame, for drawing the buttons.
#[derive(Clone, Copy, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FrameState {
    pub hovered: Option<CaptionButton>,
    pub pressed: Option<CaptionButton>,
    pub maximized: bool,
    pub active: bool,
}

/// What the window frame can do on this Windows.
#[derive(Clone, Copy, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FrameInfo {
    /// Mica is on: the UI leaves its Mica areas transparent.
    pub mica: bool,
    /// Windows 11: hovering the maximize button opens Snap Layouts.
    pub snap_layouts: bool,
}

/// Set once in `setup`.
pub struct Frame(FrameInfo);

/// Starts streaming the frame state to the UI and tells it what the frame can do.
#[tauri::command]
#[specta::specta]
pub fn title_bar_attach(
    window: WebviewWindow,
    frame: State<'_, Frame>,
    updates: Channel<FrameState>,
) -> FrameInfo {
    let listener = Box::new(move |state: native::FrameState| {
        if let Err(error) = updates.send(state.into()) {
            tracing::warn!(%error, "failed to send the frame state");
        }
    });
    if let Some(Err(error)) = hwnd(&window).map(|hwnd| native::set_listener(hwnd, listener)) {
        tracing::error!(%error, "failed to attach the title bar");
    }
    frame.0
}

/// Tells the native hit-test where the UI drew the caption buttons.
#[tauri::command]
#[specta::specta]
pub fn title_bar_set_buttons(window: WebviewWindow, buttons: Vec<CaptionButtonRect>) {
    let buttons = buttons.into_iter().map(Into::into).collect();
    if let Some(Err(error)) = hwnd(&window).map(|hwnd| native::set_buttons(hwnd, buttons)) {
        tracing::error!(%error, "failed to place the caption buttons");
    }
}

/// Installs the native title bar parts on the main window: caption-button hit-testing and Mica.
pub fn install(window: &WebviewWindow) {
    let Some(hwnd) = hwnd(window) else {
        window.manage(Frame(FrameInfo {
            mica: false,
            snap_layouts: false,
        }));
        return;
    };
    // Spike switch: JERRY_PLAIN_INPUT=1 uses a plain (non-layered) input window, to compare.
    let layered = std::env::var_os("JERRY_PLAIN_INPUT").is_none();
    if let Err(error) = native::install(hwnd, layered) {
        tracing::error!(%error, "failed to install the caption-button hit-test");
    }

    // Spike switch: JERRY_NO_MICA=1 shows the Windows 10 look (flat color) on Windows 11.
    let want_mica = std::env::var_os("JERRY_NO_MICA").is_none();
    let mica = jerry_win::backdrop::set_mica(hwnd, want_mica).unwrap_or_else(|error| {
        tracing::warn!(%error, "Mica is unavailable");
        false
    });
    let snap_layouts = jerry_win::os_build().is_some_and(|b| b >= jerry_win::WINDOWS_11_BUILD);
    tracing::info!(mica, snap_layouts, "title bar installed");
    window.manage(Frame(FrameInfo { mica, snap_layouts }));
}

/// The raw window handle, for `jerry-win`.
fn hwnd(window: &WebviewWindow) -> Option<isize> {
    window
        .hwnd()
        .map(|hwnd| hwnd.0 as isize)
        .inspect_err(|error| tracing::error!(%error, "no window handle"))
        .ok()
}

impl From<native::CaptionButton> for CaptionButton {
    fn from(button: native::CaptionButton) -> Self {
        match button {
            native::CaptionButton::Minimize => Self::Minimize,
            native::CaptionButton::Maximize => Self::Maximize,
            native::CaptionButton::Close => Self::Close,
        }
    }
}

impl From<CaptionButton> for native::CaptionButton {
    fn from(button: CaptionButton) -> Self {
        match button {
            CaptionButton::Minimize => Self::Minimize,
            CaptionButton::Maximize => Self::Maximize,
            CaptionButton::Close => Self::Close,
        }
    }
}

impl From<CaptionButtonRect> for native::ButtonRect {
    fn from(rect: CaptionButtonRect) -> Self {
        Self {
            button: rect.button.into(),
            right: rect.right,
            top: rect.top,
            width: rect.width,
            height: rect.height,
        }
    }
}

impl From<native::FrameState> for FrameState {
    fn from(state: native::FrameState) -> Self {
        Self {
            hovered: state.hovered.map(Into::into),
            pressed: state.pressed.map(Into::into),
            maximized: state.maximized,
            active: state.active,
        }
    }
}
