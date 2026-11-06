//! The session and folder views (#42).
//!
//! - The session is what Jerry's windows showed: each window's place on the screen, its two
//!   panes, their tabs and each tab's history. [`Storage`](crate::storage::Storage) keeps it in the
//!   `session` table as JSON, so the next start opens the same tabs, after a crash too.
//! - A folder view is the view, sort and columns a folder was last left in (`folder_views`); a
//!   folder Jerry has none for opens with the defaults.
//!
//! Paths are kept exactly: the session's JSON writes a path as text when it is valid Unicode and
//! as its UTF-16 code units otherwise ([`ExactPath`]).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use specta::Type;

use crate::error::{ErrorCode, JerryError};
use crate::settings::View;
use crate::sort::{Sort, SortColumn};

/// The session format this Jerry writes. A session in another format is left unread.
pub const SESSION_VERSION: u32 = 1;

/// Folders a tab's history keeps (as the UI's `MAX_HISTORY`); a longer one keeps its latest.
pub const MAX_HISTORY: usize = 100;

/// The windows and what they showed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// [`SESSION_VERSION`] when it was written.
    pub version: u32,
    /// The main window first. v0.1 has one window; tearing tabs off into windows comes in #89.
    pub windows: Vec<SessionWindow>,
}

/// A window: where it was and what its panes showed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionWindow {
    /// Where it was on the screen; none before it was first placed.
    #[serde(default)]
    pub placement: Option<Placement>,
    /// The pane the keyboard acted on.
    pub active_pane: PaneSide,
    pub left: SessionPane,
    pub right: SessionPane,
}

/// Where a window was: its restored (not maximized) rectangle, in physical pixels in Windows'
/// workspace coordinates (`GetWindowPlacement`), and whether it was maximized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub maximized: bool,
}

/// One of a window's two panes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PaneSide {
    #[default]
    Left,
    Right,
}

/// A pane's tabs, pinned ones first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPane {
    pub tabs: Vec<SessionTab>,
    /// The tab the pane showed: its place in `tabs`.
    pub active: usize,
}

/// A tab: its history of folders and its own view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTab {
    #[serde(default)]
    pub pinned: bool,
    /// Oldest first.
    pub history: Vec<SessionFolder>,
    /// The folder the tab showed: its place in `history`.
    pub index: usize,
    /// How the tab showed its folder: what it opens with when views aren't remembered per folder.
    pub view: FolderView,
}

/// A folder in a tab's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFolder {
    pub path: ExactPath,
    /// The entry that had the focus when the tab left the folder, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
}

impl Session {
    /// A session with one window whose panes each have one tab on `folder`, with the defaults:
    /// a first start, or the start folder from the settings.
    pub fn at(folder: &Path, view: View) -> Self {
        let pane = || SessionPane {
            tabs: vec![SessionTab {
                pinned: false,
                history: vec![SessionFolder {
                    path: ExactPath(folder.to_path_buf()),
                    focus: None,
                }],
                index: 0,
                view: FolderView::new(view),
            }],
            active: 0,
        };
        Self {
            version: SESSION_VERSION,
            windows: vec![SessionWindow {
                placement: None,
                active_pane: PaneSide::Left,
                left: pane(),
                right: pane(),
            }],
        }
    }

    /// The session read back from its JSON, made safe to restore: tabs without folders go, a pane
    /// without tabs is left empty (the caller gives it one), and indexes point inside their lists.
    /// `None` for JSON that isn't a session, or a session from another format.
    pub fn from_json(json: &str) -> Option<Self> {
        let mut session: Self = match serde_json::from_str(json) {
            Ok(session) => session,
            Err(error) => {
                tracing::warn!(%error, "the saved session can't be read; starting a new one");
                return None;
            }
        };
        if session.version != SESSION_VERSION {
            tracing::warn!(
                version = session.version,
                "the saved session is in another format"
            );
            return None;
        }
        for window in &mut session.windows {
            window.left.repair();
            window.right.repair();
        }
        Some(session)
    }
}

impl SessionPane {
    fn repair(&mut self) {
        let shown = self.active;
        let mut kept: Vec<(bool, SessionTab)> = std::mem::take(&mut self.tabs)
            .into_iter()
            .enumerate()
            .filter(|(_, tab)| !tab.history.is_empty())
            .map(|(at, mut tab)| {
                if tab.history.len() > MAX_HISTORY {
                    let extra = tab.history.len() - MAX_HISTORY;
                    tab.history.drain(..extra);
                    tab.index = tab.index.saturating_sub(extra);
                }
                tab.index = tab.index.min(tab.history.len() - 1);
                if !tab.view.columns.is_sane() {
                    tab.view.columns = ColumnLayout::default();
                }
                (at == shown, tab)
            })
            .collect();
        // Pinned tabs first, as the strip keeps them.
        kept.sort_by_key(|(_, tab)| !tab.pinned);
        self.active = kept.iter().position(|(shown, _)| *shown).unwrap_or(0);
        self.tabs = kept.into_iter().map(|(_, tab)| tab).collect();
    }
}

/// The view, sort and columns a folder shows with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderView {
    /// Details, List or Grid. v0.1 has only Details (List and Grid come in v0.3).
    pub view: View,
    pub sort: Sort,
    pub columns: ColumnLayout,
}

impl FolderView {
    /// `view` with the default sort and columns.
    pub fn new(view: View) -> Self {
        Self {
            view,
            sort: Sort::default(),
            columns: ColumnLayout::default(),
        }
    }
}

/// The Details view's columns, as the UI's `ColumnLayout` (`src/stores/columns.ts`): their order,
/// the ones that don't show and the widths the user dragged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ColumnLayout {
    /// Every column once, in display order.
    pub order: Vec<SortColumn>,
    /// The columns that don't show; never Name.
    pub hidden: Vec<SortColumn>,
    /// Widths the user dragged, in CSS px.
    pub widths: BTreeMap<SortColumn, u32>,
}

/// Every column, in the default order.
const COLUMNS: [SortColumn; 6] = [
    SortColumn::Name,
    SortColumn::Modified,
    SortColumn::Type,
    SortColumn::Size,
    SortColumn::Created,
    SortColumn::Extension,
];

/// Widths outside this range are a damaged row, not a drag (the UI's minimum is 40 px).
const WIDTHS: std::ops::RangeInclusive<u32> = 16..=4096;

impl Default for ColumnLayout {
    /// The UI's `DEFAULT_COLUMNS`: Name, Date modified, Type, Size; Date created and Extension
    /// hidden.
    fn default() -> Self {
        Self {
            order: COLUMNS.to_vec(),
            hidden: vec![SortColumn::Created, SortColumn::Extension],
            widths: BTreeMap::new(),
        }
    }
}

impl ColumnLayout {
    /// Whether the UI can show it: every column once in `order`, Name not hidden, widths in
    /// reason. A stored layout that isn't is replaced by the defaults.
    pub fn is_sane(&self) -> bool {
        let every_column_once = self.order.len() == COLUMNS.len()
            && COLUMNS.iter().all(|column| self.order.contains(column));
        every_column_once
            && !self.hidden.contains(&SortColumn::Name)
            && self.widths.values().all(|width| WIDTHS.contains(width))
    }
}

/// A path as the session's JSON keeps it: text when it is valid Unicode, its UTF-16 code units
/// (`{"utf16": [...]}`) otherwise, so a name that isn't valid Unicode survives too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactPath(pub PathBuf);

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum PathJson {
    Text(String),
    Units { utf16: Vec<u16> },
}

impl Serialize for ExactPath {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0.to_str() {
            Some(text) => PathJson::Text(text.to_owned()),
            None => PathJson::Units {
                utf16: self.0.as_os_str().encode_wide().collect(),
            },
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ExactPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(match PathJson::deserialize(deserializer)? {
            PathJson::Text(text) => PathBuf::from(text),
            PathJson::Units { utf16 } => PathBuf::from(OsString::from_wide(&utf16)),
        }))
    }
}

/// The nearest folder that still exists: `folder` itself, or else the closest folder it is in. A
/// restored tab whose folder went away opens there. Fails with the root's error (`DeviceGone`,
/// say) when not even that is there. Blocking IO.
pub fn nearest_folder(folder: &Path) -> Result<PathBuf, JerryError> {
    let mut last = JerryError::new(ErrorCode::NotFound).at(folder);
    for each in folder.ancestors() {
        match std::fs::metadata(each) {
            Ok(meta) if meta.is_dir() => return Ok(each.to_path_buf()),
            Ok(_) => last = JerryError::new(ErrorCode::NotFound).at(each),
            Err(error) => last = JerryError::from(error).at(each),
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::path::long_path;

    fn tab(paths: &[&str], index: usize) -> SessionTab {
        SessionTab {
            pinned: false,
            history: paths
                .iter()
                .map(|path| SessionFolder {
                    path: ExactPath(PathBuf::from(path)),
                    focus: None,
                })
                .collect(),
            index,
            view: FolderView::new(View::Details),
        }
    }

    #[test]
    fn paths_survive_exactly_georgian_and_invalid_unicode_alike() {
        let georgian = ExactPath(PathBuf::from(r"\\?\D:\პროექტები\ნახაზები"));
        let json = serde_json::to_value(&georgian).expect("serializes");
        assert_eq!(json, json!(r"\\?\D:\პროექტები\ნახაზები"));
        assert_eq!(
            serde_json::from_value::<ExactPath>(json).expect("reads"),
            georgian
        );

        // A lone surrogate: a name Windows allows that isn't valid Unicode.
        let mut units: Vec<u16> = r"\\?\C:\a".encode_utf16().collect();
        units.push(0xD800);
        let odd = ExactPath(PathBuf::from(OsString::from_wide(&units)));
        let json = serde_json::to_value(&odd).expect("serializes");
        assert_eq!(json["utf16"].as_array().map(Vec::len), Some(units.len()));
        assert_eq!(
            serde_json::from_value::<ExactPath>(json).expect("reads"),
            odd
        );
    }

    #[test]
    fn a_session_round_trips_through_json() {
        let mut session = Session::at(Path::new(r"\\?\C:\Users\თენგო"), View::Details);
        let window = &mut session.windows[0];
        window.placement = Some(Placement {
            left: -1200,
            top: 40,
            right: -100,
            bottom: 900,
            maximized: true,
        });
        window.active_pane = PaneSide::Right;
        window.right.tabs.push(SessionTab {
            pinned: true,
            ..tab(&[r"\\?\D:\a", r"\\?\D:\a\b"], 0)
        });
        window.right.active = 1;
        window.right.tabs[1].history[0].focus = Some("b".into());
        window.right.tabs[1].view.sort.descending = true;
        window.right.tabs[1]
            .view
            .columns
            .widths
            .insert(SortColumn::Size, 90);

        let window = window.clone();
        let json = serde_json::to_string(&session).expect("serializes");
        let back = Session::from_json(&json).expect("reads");
        // Repair puts the pinned tab first and keeps the shown one shown.
        assert!(back.windows[0].right.tabs[0].pinned);
        assert_eq!(back.windows[0].right.active, 0);
        assert_eq!(back.windows[0].right.tabs[0], window.right.tabs[1]);
        assert_eq!(back.windows[0].placement, window.placement);
        assert_eq!(back.windows[0].active_pane, PaneSide::Right);
        assert_eq!(back.windows[0].left, window.left);
    }

    #[test]
    fn repair_drops_empty_tabs_and_keeps_indexes_inside() {
        let mut pane = SessionPane {
            tabs: vec![
                tab(&[], 0),
                tab(&[r"\\?\C:\a", r"\\?\C:\b"], 7),
                tab(&[r"\\?\C:\c"], 0),
            ],
            active: 2,
        };
        pane.tabs[1].view.columns.hidden.push(SortColumn::Name);
        pane.repair();
        assert_eq!(pane.tabs.len(), 2);
        assert_eq!(pane.tabs[0].index, 1, "past the end: the last folder");
        assert_eq!(
            pane.tabs[0].view.columns,
            ColumnLayout::default(),
            "Name can't hide"
        );
        assert_eq!(pane.active, 1, "the shown tab stays shown");

        let long: Vec<String> = (0..MAX_HISTORY + 20).map(|i| format!(r"C:\{i}")).collect();
        let long: Vec<&str> = long.iter().map(String::as_str).collect();
        let mut pane = SessionPane {
            tabs: vec![tab(&long, MAX_HISTORY + 10)],
            active: 9,
        };
        pane.repair();
        assert_eq!(pane.tabs[0].history.len(), MAX_HISTORY);
        assert_eq!(pane.tabs[0].index, MAX_HISTORY - 10);
        assert_eq!(pane.active, 0);
    }

    #[test]
    fn other_formats_and_bad_json_are_left_unread() {
        assert_eq!(Session::from_json("not json"), None);
        assert_eq!(
            Session::from_json(r#"{"version": 99, "windows": []}"#),
            None
        );
        assert!(Session::from_json(r#"{"version": 1, "windows": []}"#).is_some());
    }

    #[test]
    fn column_layouts_are_checked() {
        let sane = ColumnLayout::default();
        assert!(sane.is_sane());
        let missing = ColumnLayout {
            order: sane.order[1..].to_vec(),
            ..sane.clone()
        };
        assert!(!missing.is_sane());
        let twice = ColumnLayout {
            order: vec![SortColumn::Name; 6],
            ..sane.clone()
        };
        assert!(!twice.is_sane());
        let mut wide = sane;
        wide.widths.insert(SortColumn::Type, 1_000_000);
        assert!(!wide.is_sane());
    }

    #[test]
    fn column_widths_are_keyed_by_column() {
        let mut layout = ColumnLayout::default();
        layout.widths.insert(SortColumn::Modified, 140);
        assert_eq!(
            serde_json::to_value(&layout).expect("serializes"),
            json!({
                "order": ["name", "modified", "type", "size", "created", "extension"],
                "hidden": ["created", "extension"],
                "widths": { "modified": 140 },
            })
        );
    }

    #[test]
    fn the_nearest_folder_is_the_closest_one_left() {
        let dir = tempfile::tempdir().expect("temp dir");
        let kept = dir.path().join("პროექტი");
        std::fs::create_dir(&kept).expect("mkdir");
        let gone = long_path(&kept.join("ნახაზები").join("2026")).expect("long path");
        assert_eq!(
            nearest_folder(&gone).expect("a parent is left"),
            long_path(&kept).expect("long path")
        );
        let here = long_path(&kept).expect("long path");
        assert_eq!(nearest_folder(&here).expect("still there"), here);

        // A file where the folder was: its folder.
        let file = kept.join("plan.dwg");
        std::fs::write(&file, b"x").expect("write");
        assert_eq!(
            nearest_folder(&long_path(&file).expect("long path")).expect("its folder"),
            here
        );

        // Not even the drive.
        let error = nearest_folder(Path::new(r"\\?\Q:\nothing\here")).expect_err("no drive Q:");
        assert_ne!(error.code, ErrorCode::Internal);
    }
}
