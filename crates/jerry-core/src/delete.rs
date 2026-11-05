//! Deleting: Jerry's own permanent delete, what a delete would remove, and which deletes take a
//! system folder with them (Tech PRD «Recycle Bin», data safety rules 5–7).
//!
//! The Recycle Bin is the Shell's (`jerry_win::recycle`). What it can't take is deleted here:
//! Shift+Del, and items the Shell has no Recycle Bin for, once the user has confirmed.
//! [`delete_permanently`] uses plain Win32 calls on long paths, so it works past 260 characters
//! where the Shell doesn't. It goes through a folder depth first, files before their folder, and
//! clears the read-only attribute where it is in the way. It never enters a link (symbolic link,
//! junction, mount point): it deletes the link itself, never what the link points to. A file it
//! can't delete (open in another program, no permission) doesn't stop the rest; the folders above
//! it stay, and the item's first failure comes back with that file's path.
//!
//! Everything here is blocking IO: run it on a background thread.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use specta::Type;
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_DIR_NOT_EMPTY, WIN32_ERROR};
use windows::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
use windows::Win32::Storage::FileSystem::{
    DeleteFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_READONLY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAGS_AND_ATTRIBUTES, RemoveDirectoryW, SetFileAttributesW,
    WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

use crate::error::{ErrorCode, JerryError};
use crate::listing::{find_data_name, find_name};
use crate::path::long_path;

/// How often [`measure`] reports what it has counted so far.
pub const MEASURE_PROGRESS: Duration = Duration::from_millis(100);

/// `IsReparseTagNameSurrogate`: the reparse point stands for another name (a symbolic link, a
/// junction, a mount point). Cloud files (OneDrive) are reparse points too, but not surrogates:
/// they are the files themselves.
const NAME_SURROGATE: u32 = 0x2000_0000;

/// How often removing a folder is tried while Windows still finishes deleting what was in it
/// (another program, an indexer or a virus scanner, had a file open with delete sharing).
const DIR_ATTEMPTS: u32 = 10;
const DIR_RETRY: Duration = Duration::from_millis(20);

/// What deleting some items would remove.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Tally {
    /// Files, links to files included.
    #[specta(type = f64)]
    pub files: u64,
    /// Folders, the items themselves and links to folders included.
    #[specta(type = f64)]
    pub folders: u64,
    /// The files' size, in bytes.
    #[specta(type = f64)]
    pub bytes: u64,
}

/// Counts what deleting `paths` (long paths, as core keeps them) would remove, through every
/// subfolder but not through links. `progress` gets the count so far every
/// [`MEASURE_PROGRESS`]. A folder that can't be read counts as itself; an item that is gone
/// counts as nothing. Fails only with [`ErrorCode::Cancelled`], once `cancel` is set.
pub fn measure(
    paths: &[PathBuf],
    cancel: &AtomicBool,
    mut progress: impl FnMut(&Tally),
) -> Result<Tally, JerryError> {
    let mut tally = Tally::default();
    let mut reported = Instant::now();
    let mut folders: Vec<PathBuf> = Vec::new();
    for path in paths {
        if let Ok(item) = probe(path) {
            count(&item, &mut tally, &mut folders);
        }
    }
    while let Some(folder) = folders.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err(JerryError::new(ErrorCode::Cancelled));
        }
        if let Ok(children) = children(&folder) {
            for child in &children {
                count(child, &mut tally, &mut folders);
            }
        }
        if reported.elapsed() >= MEASURE_PROGRESS {
            reported = Instant::now();
            progress(&tally);
        }
    }
    Ok(tally)
}

fn count(item: &Item, tally: &mut Tally, folders: &mut Vec<PathBuf>) {
    if item.is_dir() {
        tally.folders += 1;
        if !item.link {
            folders.push(item.path.clone());
        }
    } else {
        tally.files += 1;
        tally.bytes += item.size;
    }
}

/// Deletes `paths` (long paths, as core keeps them) for good, folders with everything in them.
/// Returns one result per path, in order: `Ok` once it is gone, else the first thing that
/// couldn't be deleted under it (with that file's path). What could be deleted is gone either
/// way. Never call it without the user's confirmation (data safety rule 7).
pub fn delete_permanently(paths: &[PathBuf]) -> Vec<Result<(), JerryError>> {
    paths.iter().map(|path| delete_item(path)).collect()
}

fn delete_item(path: &Path) -> Result<(), JerryError> {
    let item = probe(path)?;
    if item.is_dir() && !item.link {
        remove_tree(item)
    } else {
        remove(&item)
    }
}

/// A folder being emptied: what is left in it, and whether something in it stays.
struct Frame {
    folder: Item,
    left: Vec<Item>,
    kept: bool,
}

/// Empties `folder` depth first and removes it. Iterative, so a deep tree can't overflow the
/// stack.
fn remove_tree(folder: Item) -> Result<(), JerryError> {
    let mut first: Option<JerryError> = None;
    let mut stack = vec![Frame {
        left: children(&folder.path)?,
        folder,
        kept: false,
    }];
    while let Some(frame) = stack.last_mut() {
        let Some(child) = frame.left.pop() else {
            // Empty now, unless something in it stays.
            let Some(done) = stack.pop() else { break };
            let kept = done.kept
                || remove(&done.folder)
                    .map_err(|e| first.get_or_insert(e))
                    .is_err();
            if kept && let Some(parent) = stack.last_mut() {
                parent.kept = true;
            }
            continue;
        };
        if child.is_dir() && !child.link {
            match children(&child.path) {
                Ok(left) => stack.push(Frame {
                    folder: child,
                    left,
                    kept: false,
                }),
                Err(error) => {
                    first.get_or_insert(error);
                    frame.kept = true;
                }
            }
        } else if let Err(error) = remove(&child) {
            first.get_or_insert(error);
            frame.kept = true;
        }
    }
    first.map_or(Ok(()), Err)
}

/// Removes one file, link or empty folder. A read-only one loses the attribute first; if it still
/// can't be removed, it gets the attribute back.
fn remove(item: &Item) -> Result<(), JerryError> {
    let wide: Vec<u16> = item.path.as_os_str().encode_wide().chain([0]).collect();
    let path = PCWSTR(wide.as_ptr());
    let attempt = || {
        if item.is_dir() {
            remove_folder(path)
        } else {
            // SAFETY: `path` points into `wide`, which is NUL-terminated and outlives the call.
            unsafe { DeleteFileW(path) }
        }
    };
    let result = match attempt() {
        Err(error)
            if item.attributes & FILE_ATTRIBUTE_READONLY.0 != 0
                && win32(&error) == Some(ERROR_ACCESS_DENIED) =>
        {
            let writable = item.attributes & !FILE_ATTRIBUTE_READONLY.0;
            let writable = if writable == 0 {
                FILE_ATTRIBUTE_NORMAL.0
            } else {
                writable
            };
            // SAFETY: as above. If even the attribute can't change (the file is open in another
            // program, say), that is the reason it stays.
            unsafe { SetFileAttributesW(path, FILE_FLAGS_AND_ATTRIBUTES(writable)) }.and_then(
                |()| {
                    attempt().inspect_err(|_| {
                        // SAFETY: as above. Best effort: the item stays as it was.
                        let restored = unsafe {
                            SetFileAttributesW(path, FILE_FLAGS_AND_ATTRIBUTES(item.attributes))
                        };
                        if let Err(error) = restored {
                            tracing::warn!(%error, "couldn't make a kept item read-only again");
                        }
                    })
                },
            )
        }
        result => result,
    };
    result.map_err(|error| JerryError::from(error).at(&item.path))
}

/// `RemoveDirectoryW`, tried again for a moment while the folder still has entries Windows is
/// deleting.
fn remove_folder(path: PCWSTR) -> windows::core::Result<()> {
    let mut attempt = 1;
    loop {
        // SAFETY: the caller's `path` is NUL-terminated and outlives the call.
        match unsafe { RemoveDirectoryW(path) } {
            Err(error) if win32(&error) == Some(ERROR_DIR_NOT_EMPTY) && attempt < DIR_ATTEMPTS => {
                std::thread::sleep(DIR_RETRY * attempt);
                attempt += 1;
            }
            result => return result,
        }
    }
}

fn win32(error: &windows::core::Error) -> Option<WIN32_ERROR> {
    WIN32_ERROR::from_error(error)
}

/// A file, folder or link, as its folder lists it.
struct Item {
    path: PathBuf,
    attributes: u32,
    size: u64,
    /// A link (symbolic link, junction, mount point): deleting removes only the link.
    link: bool,
}

impl Item {
    fn found(folder: &Path, data: &WIN32_FIND_DATAW) -> Self {
        let name = OsString::from_wide(find_data_name(data));
        let attributes = data.dwFileAttributes;
        let reparse = attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0;
        Self {
            path: folder.join(name),
            attributes,
            size: (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow),
            // With the reparse point attribute, `dwReserved0` is the reparse tag.
            link: reparse && data.dwReserved0 & NAME_SURROGATE != 0,
        }
    }

    fn is_dir(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0
    }
}

/// The item at `path` as its folder lists it. A drive root is no item: [`ErrorCode::InvalidName`].
fn probe(path: &Path) -> Result<Item, JerryError> {
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(JerryError::new(ErrorCode::InvalidName).at(path));
    };
    let name: Vec<u16> = name.encode_wide().collect();
    let mut item = None;
    // The exact name only, not an entry it matches by 8.3 short name or by case.
    find_name(folder, &name, |data| {
        if find_data_name(data) == name.as_slice() {
            item = Some(Item::found(folder, data));
        }
    })?;
    item.ok_or_else(|| JerryError::new(ErrorCode::NotFound).at(path))
}

/// Everything in `folder`.
fn children(folder: &Path) -> Result<Vec<Item>, JerryError> {
    let mut items = Vec::new();
    find_name(folder, &[u16::from(b'*')], |data| {
        items.push(Item::found(folder, data));
    })?;
    Ok(items)
}

/// Whether deleting or moving `path` takes one of the `system` folders with it (data safety
/// rule 6): `path` is one of them, or holds one. Paths compare as Windows compares them: whole
/// names, ordinal, ignoring case, with or without the long-path prefix.
pub fn takes_system_folder(path: &Path, system: &[PathBuf]) -> bool {
    system.iter().any(|folder| is_within(folder, path))
}

/// Whether `inner` is `outer` or inside it.
fn is_within(inner: &Path, outer: &Path) -> bool {
    let (Ok(inner), Ok(outer)) = (long_path(inner), long_path(outer)) else {
        return false;
    };
    let mut inner = inner.components();
    outer.components().all(|part| {
        inner
            .next()
            .is_some_and(|other| same_name(part.as_os_str(), other.as_os_str()))
    })
}

fn same_name(a: &OsStr, b: &OsStr) -> bool {
    let a: Vec<u16> = a.encode_wide().collect();
    let b: Vec<u16> = b.encode_wide().collect();
    // SAFETY: both are valid slices.
    unsafe { CompareStringOrdinal(&a, &b, true) == CSTR_EQUAL }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_folders_and_what_holds_them() {
        let system = [
            PathBuf::from(r"C:\Windows"),
            PathBuf::from(r"C:\Program Files"),
            PathBuf::from(r"C:\Users"),
            PathBuf::from(r"D:\OS\ვინდოუსი"),
        ];
        let takes = |path: &str| takes_system_folder(Path::new(path), &system);
        assert!(takes(r"\\?\C:\Windows"));
        assert!(takes(r"\\?\c:\WINDOWS"));
        assert!(takes(r"C:\Users\"));
        assert!(takes(r"\\?\D:\OS"), "holds a system folder");
        assert!(takes(r"\\?\d:\os\ვინდოუსი"));
        assert!(
            !takes(r"\\?\C:\Windows\Temp\x.txt"),
            "inside one is not one"
        );
        assert!(!takes(r"\\?\C:\Users\TS"));
        assert!(!takes(r"\\?\C:\Win"), "whole names only");
        assert!(!takes(r"\\?\C:\Windows.old"));
        assert!(!takes(r"\\?\E:\Windows"));
    }
}
