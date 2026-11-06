//! Folder views (#42): the view, sort and columns each folder was last left in
//! (`jerry_core::session::FolderView`, the `folder_views` table). With the setting "Remember the
//! view of every folder" on, a tab that opens a folder (`listing_open`) takes the folder's view,
//! or the defaults for a folder without one, and the UI remembers a folder's view when the user
//! sorts it or changes its columns (`folder_view_save`). With it off, a tab keeps its own view
//! from folder to folder.

use std::path::Path;

use jerry_core::session::FolderView;
use jerry_core::settings::View;
use jerry_core::sort::Sort;
use jerry_core::{ErrorCode, JerryError};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::jobs;
use crate::listing::Listings;
use crate::storage::{Db, now_ms};

/// How a tab shows a folder it opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Look {
    /// The tab keeps its own view, sort and columns: views aren't remembered per folder, or the
    /// tab opened the same folder again (Refresh, Retry).
    Tab,
    /// The folder has no remembered view: the default view (a setting), the default sort and the
    /// default columns.
    Default { view: View, sort: Sort },
    /// A remembered view: the folder's, or a restored tab's own.
    Saved { view: FolderView },
}

impl Look {
    /// The sort the folder's listing starts with: this look's, or else the tab's own.
    pub fn sort(&self, tab: Sort) -> Sort {
        match self {
            Self::Tab => tab,
            Self::Default { sort, .. } => *sort,
            Self::Saved { view } => view.sort,
        }
    }

    /// `view` as a look, folders first or not as the setting says: that one is global, not a
    /// folder's.
    pub fn saved(mut view: FolderView, folders_first: bool) -> Self {
        view.sort.folders_first = folders_first;
        Self::Saved { view }
    }

    /// The defaults, with the default view `view`.
    pub fn default_for(view: View, folders_first: bool) -> Self {
        Self::Default {
            view,
            sort: Sort {
                folders_first,
                ..Sort::default()
            },
        }
    }
}

/// How a tab shows `folder` (an exact path) as it opens it, folders first or not as
/// `folders_first`: the folder's remembered view or the defaults when views are remembered per
/// folder, otherwise the tab's own. Notes that the folder's view was used. Blocking: call it from
/// a background job.
pub fn look(db: &Db, folder: &Path, folders_first: bool) -> Look {
    let looked = db.with(|storage| {
        let settings = storage.settings();
        if !settings.remember_folder_views {
            return Ok(Look::Tab);
        }
        let default_view = settings.default_view;
        Ok(match storage.folder_view(folder, now_ms())? {
            Some(view) => Look::saved(view, folders_first),
            None => Look::default_for(default_view, folders_first),
        })
    });
    looked.unwrap_or_else(|error| {
        tracing::warn!(code = ?error.code, "can't read a folder view; the tab keeps its own");
        Look::Tab
    })
}

/// Remembers `view` for folder token `folder` (`ListingOpened::folder`): the user sorted it or
/// changed its columns. Does nothing while views aren't remembered per folder. Past 5 000
/// folders, the ones used longest ago are forgotten.
#[tauri::command]
#[specta::specta]
pub async fn folder_view_save(
    app: AppHandle,
    folder: u32,
    view: FolderView,
) -> Result<(), JerryError> {
    let path = app
        .state::<Listings>()
        .folder_path(folder)
        .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
    if !view.columns.is_sane() {
        tracing::warn!("the UI sent columns it can't show");
        return Err(JerryError::internal());
    }
    let db = Db::clone(&app.state::<Db>());
    jobs::background("save folder view", move || {
        db.with(|storage| {
            if storage.settings().remember_folder_views {
                storage.save_folder_view(&path, &view, now_ms())?;
            }
            Ok(())
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use jerry_core::sort::SortColumn;
    use serde_json::json;

    use super::*;

    #[test]
    fn a_folder_gets_its_own_view_or_the_defaults_while_views_are_remembered() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = Db::open(Ok(dir.path().join("jerry.db")));
        let folder = Path::new(r"\\?\D:\პროექტები");

        let defaults = look(&db, folder, true);
        assert_eq!(defaults, Look::default_for(View::Details, true));
        assert_eq!(
            defaults.sort(Sort {
                column: SortColumn::Size,
                descending: true,
                folders_first: true,
            }),
            Sort::default(),
            "the tab's sort gives way"
        );

        let mut by_date = FolderView::new(View::Details);
        by_date.sort.column = SortColumn::Modified;
        by_date.sort.folders_first = true;
        db.with(|storage| storage.save_folder_view(folder, &by_date, 1))
            .expect("saves");
        let remembered = look(&db, folder, false);
        assert_eq!(
            remembered.sort(Sort::default()).column,
            SortColumn::Modified
        );
        assert!(
            !remembered.sort(Sort::default()).folders_first,
            "the setting decides folders first"
        );

        db.with(|storage| storage.set_setting("rememberFolderViews", json!(false)))
            .expect("turns it off");
        assert_eq!(look(&db, folder, true), Look::Tab);
    }

    #[test]
    fn looks_go_to_the_ui_tagged() {
        let json = serde_json::to_value(Look::default_for(View::Details, true)).expect("json");
        assert_eq!(
            json,
            json!({
                "kind": "default",
                "view": "details",
                "sort": { "column": "name", "descending": false, "foldersFirst": true },
            })
        );
        assert_eq!(
            serde_json::to_value(Look::Tab).expect("json"),
            json!({ "kind": "tab" })
        );
    }
}
