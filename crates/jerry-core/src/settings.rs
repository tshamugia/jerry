//! Jerry's settings and their defaults, from the PRD's settings table («პარამეტრები და
//! პერსონალიზაცია»), plus the zone sizes the window remembers («ინტერფეისის სტრუქტურა»).
//! [`Storage`](crate::storage::Storage) keeps them in the `settings` table, one row per key, the
//! value as JSON; a key without a row has its default.
//!
//! On the wire the keys are camelCase (`showHidden`), the same as the rows' keys.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::error::JerryError;

/// Every setting, defaults filled in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// App theme.
    pub theme: Theme,
    /// Accent color: the Windows accent, or one of Jerry's six.
    pub accent: Accent,
    /// Row density.
    pub density: Density,
    /// Font size, in percent.
    pub font_scale: u16,
    /// Mica window background (Windows 11).
    pub mica: bool,
    /// How dates are written, for example `DD.MM.YYYY`.
    pub date_format: String,
    /// What Jerry opens at start.
    pub startup: Startup,
    /// Show hidden and system files.
    pub show_hidden: bool,
    /// Show file name extensions.
    pub show_extensions: bool,
    /// Folders before files.
    pub folders_first: bool,
    /// View for folders without a remembered one.
    pub default_view: View,
    /// Two panes.
    pub dual_pane: bool,
    /// Remember view and sort for every folder.
    pub remember_folder_views: bool,
    /// Open items with a single or a double click.
    pub click_to_open: ClickToOpen,
    /// Ask before deleting to the Recycle Bin.
    pub confirm_recycle: bool,
    /// What a drag does without a modifier key.
    pub drag_action: DragAction,
    /// How long a drag waits over a folder before opening it, in milliseconds.
    pub spring_load_delay_ms: u32,
    /// Transfers to different disks run at the same time.
    pub parallel_transfers: bool,
    /// Notify when a transfer finishes.
    pub notify_on_finish: bool,
    /// Start Jerry with Windows (in the background).
    pub start_with_windows: bool,
    /// Closing the last window hides Jerry instead of exiting (ADR-0005).
    pub keep_running: bool,
    /// The global hotkey that brings Jerry to the front.
    pub global_hotkey: String,
    /// "Open in Jerry" in Explorer's context menu.
    pub open_in_jerry: bool,
    /// Check for new versions.
    pub check_updates: bool,
    /// Sidebar width, in CSS pixels (drag its edge).
    pub sidebar_width: u16,
    /// Preview panel width, in CSS pixels (drag its edge).
    pub preview_width: u16,
    /// The left pane's share of the two panes' width, 0 to 1 (drag the divider).
    pub pane_split: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            accent: Accent::Windows,
            density: Density::Comfortable,
            font_scale: 100,
            mica: true,
            date_format: "DD.MM.YYYY".into(),
            startup: Startup::PreviousSession,
            show_hidden: false,
            show_extensions: true,
            folders_first: true,
            default_view: View::Details,
            dual_pane: true,
            remember_folder_views: true,
            click_to_open: ClickToOpen::Double,
            confirm_recycle: false,
            drag_action: DragAction::Windows,
            spring_load_delay_ms: 700,
            parallel_transfers: true,
            notify_on_finish: true,
            start_with_windows: false,
            keep_running: true,
            global_hotkey: "Ctrl+Alt+J".into(),
            open_in_jerry: false,
            check_updates: true,
            sidebar_width: 232,
            preview_width: 288,
            pane_split: 0.5,
        }
    }
}

/// App theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    /// Follow the Windows app theme.
    System,
    Light,
    Dark,
}

/// Accent color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Accent {
    /// Match Windows: the Windows accent color.
    Windows,
    Blue,
    Teal,
    Indigo,
    Graphite,
    Plum,
    Rust,
}

/// Row density: 30 px or 24 px rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Density {
    Comfortable,
    Compact,
}

/// What Jerry opens at start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Startup {
    /// The windows and tabs of the previous session.
    PreviousSession,
    /// One folder.
    Folder {
        /// The folder, as the user entered it.
        path: String,
    },
}

/// File list view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum View {
    Details,
    List,
    Grid,
}

/// Clicks that open an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ClickToOpen {
    Single,
    Double,
}

/// What a drag does without a modifier key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DragAction {
    /// The Windows rule: move on the same drive, copy to another one.
    Windows,
    Copy,
    Move,
}

impl Settings {
    /// Settings from stored `(key, JSON)` rows. A row with an unknown key (a newer Jerry's) or a
    /// value that doesn't fit its key is skipped and logged; that key keeps its default.
    pub fn from_rows(rows: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut settings = Self::default();
        for (key, json) in rows {
            let parsed = serde_json::from_str(&json)
                .map_err(|_| JerryError::internal())
                .and_then(|value| settings.with(&key, value));
            match parsed {
                Ok(next) => settings = next,
                Err(_) => tracing::warn!(key, "ignoring a stored setting; using its default"),
            }
        }
        settings
    }

    /// These settings with `key` set to `value`. Fails with `Internal` if there is no such key or
    /// the value doesn't fit it: the UI sends only typed values, so either one is a bug.
    pub fn with(&self, key: &str, value: Value) -> Result<Self, JerryError> {
        let mut object = self.to_object()?;
        let Some(slot) = object.get_mut(key) else {
            tracing::warn!(key, "no such setting");
            return Err(JerryError::internal());
        };
        *slot = value;
        serde_json::from_value(Value::Object(object)).map_err(|error| {
            tracing::warn!(key, %error, "the value doesn't fit the setting");
            JerryError::internal()
        })
    }

    /// The value of `key` as JSON, or `None` if there is no such key.
    pub fn get(&self, key: &str) -> Option<Value> {
        self.to_object().ok()?.remove(key)
    }

    fn to_object(&self) -> Result<serde_json::Map<String, Value>, JerryError> {
        match serde_json::to_value(self) {
            Ok(Value::Object(object)) => Ok(object),
            _ => Err(JerryError::internal()),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::ErrorCode;

    #[test]
    fn defaults_follow_the_prd() {
        let defaults = serde_json::to_value(Settings::default()).expect("serializes");
        assert_eq!(
            defaults,
            json!({
                "theme": "system",
                "accent": "windows",
                "density": "comfortable",
                "fontScale": 100,
                "mica": true,
                "dateFormat": "DD.MM.YYYY",
                "startup": { "kind": "previousSession" },
                "showHidden": false,
                "showExtensions": true,
                "foldersFirst": true,
                "defaultView": "details",
                "dualPane": true,
                "rememberFolderViews": true,
                "clickToOpen": "double",
                "confirmRecycle": false,
                "dragAction": "windows",
                "springLoadDelayMs": 700,
                "parallelTransfers": true,
                "notifyOnFinish": true,
                "startWithWindows": false,
                "keepRunning": true,
                "globalHotkey": "Ctrl+Alt+J",
                "openInJerry": false,
                "checkUpdates": true,
                "sidebarWidth": 232,
                "previewWidth": 288,
                "paneSplit": 0.5,
            })
        );
    }

    #[test]
    fn with_sets_one_key() {
        let settings = Settings::default()
            .with("showHidden", json!(true))
            .and_then(|settings| {
                settings.with(
                    "startup",
                    json!({ "kind": "folder", "path": r"D:\პროექტები" }),
                )
            })
            .expect("valid values");
        assert!(settings.show_hidden);
        assert_eq!(
            settings.startup,
            Startup::Folder {
                path: r"D:\პროექტები".into()
            }
        );
        assert_eq!(settings.get("showHidden"), Some(json!(true)));
        assert_eq!(
            settings.theme,
            Theme::System,
            "other keys keep their values"
        );
    }

    #[test]
    fn with_rejects_unknown_keys_and_wrong_values() {
        let settings = Settings::default();
        let code = |result: Result<Settings, JerryError>| result.map_err(|error| error.code);
        assert_eq!(
            code(settings.with("noSuchKey", json!(1))),
            Err(ErrorCode::Internal)
        );
        assert_eq!(
            code(settings.with("theme", json!("purple"))),
            Err(ErrorCode::Internal)
        );
        assert_eq!(
            code(settings.with("fontScale", json!(-5))),
            Err(ErrorCode::Internal)
        );
        assert_eq!(settings.get("noSuchKey"), None);
    }

    #[test]
    fn bad_rows_fall_back_to_defaults_one_key_at_a_time() {
        let rows = [
            ("density", r#""compact""#),
            ("theme", r#""purple""#),   // not a theme
            ("showHidden", "not json"), // not JSON
            ("aNewerJerrysKey", "1"),   // unknown here
            ("dateFormat", r#""YYYY-MM-DD""#),
        ]
        .map(|(key, value)| (key.to_owned(), value.to_owned()));
        let settings = Settings::from_rows(rows);
        assert_eq!(settings.density, Density::Compact);
        assert_eq!(settings.date_format, "YYYY-MM-DD");
        assert_eq!(settings.theme, Theme::System);
        assert!(!settings.show_hidden);
    }
}
