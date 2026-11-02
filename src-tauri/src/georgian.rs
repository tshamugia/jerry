//! Spike #18: the native window title with a Georgian folder name (taskbar, Alt+Tab, Task
//! Manager). The custom title bar draws its own text; this is the one Windows draws.

use tauri::WebviewWindow;

/// Sets the window's native title and returns it as Windows reads it back (`GetWindowTextW`,
/// through tao), or `None` if either call failed.
#[tauri::command]
#[specta::specta]
pub fn window_set_title(window: WebviewWindow, title: String) -> Option<String> {
    if let Err(error) = window.set_title(&title) {
        tracing::error!(%error, "failed to set the window title");
        return None;
    }
    window.title().ok()
}
