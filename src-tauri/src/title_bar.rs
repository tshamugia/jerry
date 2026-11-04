//! The title bar's IPC (ADR-0004). React draws the tab strip and the caption buttons;
//! `jerry-win`'s input window answers the native hit-test over the buttons (Snap Layouts), runs
//! their clicks and streams their state back through a `Channel`, which needs no capability.

use jerry_core::JerryError;
use jerry_win::main_thread::run_on_main;
use jerry_win::title_bar as native;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::WebviewWindow;
use tauri::ipc::Channel;

use crate::jobs;

/// One of the three window buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CaptionButton {
    Minimize,
    Maximize,
    Close,
}

/// Where the page drew a caption button, in CSS pixels from the top-right corner of the window.
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

/// What the page needs to draw the caption buttons like native ones: which one the mouse is over
/// or holds down (only the native hit-test sees that), whether the window is maximized (the
/// maximize button shows Restore) and whether it is the active window (inactive glyphs dim).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FrameState {
    pub hovered: Option<CaptionButton>,
    pub pressed: Option<CaptionButton>,
    pub maximized: bool,
    pub active: bool,
}

/// Starts streaming the window's [`FrameState`] to the page; the current state comes first.
#[tauri::command]
#[specta::specta]
pub async fn title_bar_attach(
    window: WebviewWindow,
    updates: Channel<FrameState>,
) -> Result<(), JerryError> {
    let hwnd = hwnd(&window)?;
    let listener = Box::new(move |state: native::FrameState| {
        if let Err(error) = updates.send(state.into()) {
            tracing::warn!(%error, "can't send the frame state");
        }
    });
    jobs::on_main("title bar attach", move || {
        native::set_listener(hwnd, listener)
    })
    .await
}

/// Tells the native hit-test where the page drew the caption buttons.
#[tauri::command]
#[specta::specta]
pub async fn title_bar_set_buttons(
    window: WebviewWindow,
    buttons: Vec<CaptionButtonRect>,
) -> Result<(), JerryError> {
    let hwnd = hwnd(&window)?;
    let buttons = buttons.into_iter().map(Into::into).collect();
    jobs::on_main("title bar buttons", move || {
        native::set_buttons(hwnd, buttons)
    })
    .await
}

/// Does what a click on the caption button does. The mouse never reaches the page's buttons
/// (the input window does that); the keyboard and screen readers press them through this.
#[tauri::command]
#[specta::specta]
pub async fn title_bar_press(
    window: WebviewWindow,
    button: CaptionButton,
) -> Result<(), JerryError> {
    native::press(hwnd(&window)?, button.into())
}

/// Puts the caption-button input window on `window`. It runs on the main thread, queued before
/// the page can ask for it.
pub fn install(window: &WebviewWindow) {
    let Ok(hwnd) = hwnd(window) else {
        return;
    };
    let queued = run_on_main("title bar install", move || {
        if let Err(error) = native::install(hwnd) {
            tracing::error!(%error, "can't install the caption buttons' hit-test");
        }
    });
    if let Err(error) = queued {
        tracing::error!(%error, "can't queue the caption buttons' hit-test");
    }
}

/// The raw window handle, for `jerry-win`.
fn hwnd(window: &WebviewWindow) -> Result<isize, JerryError> {
    window.hwnd().map(|hwnd| hwnd.0 as isize).map_err(|error| {
        tracing::error!(%error, "no window handle");
        JerryError::internal()
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_sees_buttons_in_camel_case() {
        let state = FrameState {
            hovered: Some(CaptionButton::Maximize),
            pressed: None,
            maximized: true,
            active: false,
        };
        assert_eq!(
            serde_json::to_value(state).expect("json"),
            serde_json::json!({
                "hovered": "maximize",
                "pressed": null,
                "maximized": true,
                "active": false,
            })
        );
        let rect: CaptionButtonRect = serde_json::from_value(serde_json::json!({
            "button": "close", "right": 0, "top": 0, "width": 46, "height": 40,
        }))
        .expect("rect");
        let native = native::ButtonRect::from(rect);
        assert_eq!(native.button, native::CaptionButton::Close);
        assert_eq!((native.width, native.height), (46.0, 40.0));
    }
}
