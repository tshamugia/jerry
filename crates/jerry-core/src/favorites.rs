//! Sidebar favorites (#35): folders the user pinned, at the top level or in groups the user
//! named ("Insta", "TSGroup"), in the order the user put them. They live in `jerry.db`
//! (`favorites`, `favorite_groups`): [`Storage::favorites`](crate::storage::Storage::favorites)
//! and the methods next to it. This module has their types and [`problem`], which says why a
//! favorite can't be opened, so the sidebar can grey it out.

use std::path::{Path, PathBuf};

use crate::error::{ErrorCode, JerryError};
use crate::volume;

/// A favorite folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Favorite {
    pub id: u32,
    /// The exact path, as core keeps it (`\\?\` long form).
    pub path: PathBuf,
    /// The name the user gave it; none: the folder's own name. The folder isn't renamed.
    pub name: Option<String>,
    /// Its group; none: the top level.
    pub group: Option<u32>,
}

/// A group of favorites, with the name the user gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FavoriteGroup {
    pub id: u32,
    pub name: String,
}

/// All favorites, in sidebar order: the groups, and the favorites with the top level first, then
/// group by group, each in its own order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Favorites {
    pub groups: Vec<FavoriteGroup>,
    pub items: Vec<Favorite>,
}

/// Why the favorite folder `folder` (an exact long path) can't be opened, if that is because it
/// isn't there: [`ErrorCode::NotFound`] when it was deleted, moved or renamed (or is a file now),
/// [`ErrorCode::DeviceGone`] when its drive or network location isn't connected. None when it is
/// there, or for any other reason (access, say): opening it tells.
///
/// Blocking IO, without reading the folder: a share that doesn't answer takes Windows' own
/// timeout.
pub fn problem(folder: &Path) -> Option<JerryError> {
    match std::fs::metadata(folder) {
        Ok(metadata) if metadata.is_dir() => None,
        Ok(_) => Some(JerryError::new(ErrorCode::NotFound).at(folder)),
        Err(error) => {
            let error = JerryError::from(error).at(folder);
            match error.code {
                // A drive that is gone can say "not found" too: look at its root.
                ErrorCode::NotFound if !volume_there(folder) => {
                    Some(JerryError::new(ErrorCode::DeviceGone).at(folder))
                }
                ErrorCode::NotFound | ErrorCode::DeviceGone => Some(error),
                _ => None,
            }
        }
    }
}

/// Whether the volume `folder` is on is there: a drive letter with a volume, media in the drive,
/// a share that answers.
fn volume_there(folder: &Path) -> bool {
    volume::root(folder).is_ok_and(|root| std::fs::metadata(root).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::long_path;

    #[test]
    fn a_folder_that_is_there_has_no_problem() {
        let dir = tempfile::tempdir().expect("temp folder");
        let folder = long_path(&dir.path().join("ნახაზები")).expect("long path");
        std::fs::create_dir(&folder).expect("mkdir");
        assert_eq!(problem(&folder), None);
    }

    #[test]
    fn a_deleted_folder_is_not_found() {
        let dir = tempfile::tempdir().expect("temp folder");
        let folder = long_path(&dir.path().join("BMW")).expect("long path");
        std::fs::create_dir(&folder).expect("mkdir");
        std::fs::remove_dir(&folder).expect("rmdir");
        let found = problem(&folder).expect("a problem");
        assert_eq!(found.code, ErrorCode::NotFound);
        assert_eq!(found.path, Some(folder.clone()));

        let file = long_path(&dir.path().join("BMW")).expect("long path");
        std::fs::write(&file, "now a file").expect("write");
        assert_eq!(
            problem(&file).map(|error| error.code),
            Some(ErrorCode::NotFound)
        );
    }

    #[test]
    fn a_folder_on_a_missing_drive_is_on_a_device_that_is_gone() {
        let Some(letter) = ('D'..='Z').rev().find(|letter| {
            let root = format!(r"{letter}:\");
            std::fs::metadata(&root).is_err() && volume::root(Path::new(&root)).is_err()
        }) else {
            return;
        };
        let folder = long_path(Path::new(&format!(r"{letter}:\Projects\2026"))).expect("long");
        let gone = problem(&folder).expect("a problem");
        assert_eq!(gone.code, ErrorCode::DeviceGone);
        assert_eq!(gone.path, Some(folder));
    }
}
