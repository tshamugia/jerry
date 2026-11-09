//! Renaming a file or folder where it is (Tech PRD «გადარქმევა», #36). The new name passes
//! [`check_name`] first; a name another item in the folder has is refused, never replaced. A
//! change of case only (`file.txt` → `File.txt`) works too, through a temporary name where the
//! file system won't do it in one step.
//!
//! Blocking IO: run it on a background thread.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, WIN32_ERROR};
use windows::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
use windows::Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW};
use windows::core::PCWSTR;

use crate::error::{ErrorCode, JerryError};
use crate::name::check_name;

/// Renames the file or folder at `path` (a long path, as core keeps them) to `name` in the same
/// folder, and returns its new path. Fails with:
/// - [`ErrorCode::InvalidName`] when `name` breaks a naming rule (no path), or another item in the
///   folder has it (that item's path);
/// - [`ErrorCode::InUse`] or [`ErrorCode::AccessDenied`] when Windows refuses, on `path`;
/// - [`ErrorCode::NotFound`] when the item is gone.
///
/// The same name exactly changes nothing.
pub fn rename(path: &Path, name: &str) -> Result<PathBuf, JerryError> {
    check_name(name).map_err(|_| JerryError::new(ErrorCode::InvalidName))?;
    let (Some(folder), Some(current)) = (path.parent(), path.file_name()) else {
        // A drive or share root has no name to change.
        return Err(JerryError::new(ErrorCode::InvalidName).at(path));
    };
    if current == OsStr::new(name) {
        return Ok(path.to_path_buf());
    }
    let target = folder.join(name);
    match move_file(path, &target) {
        Ok(()) => Ok(target),
        Err(error) if is_taken(&error) && same_ignoring_case(current, OsStr::new(name)) => {
            through_temporary(path, &target)
        }
        Err(error) if is_taken(&error) => Err(JerryError::new(ErrorCode::InvalidName).at(&target)),
        Err(error) => Err(JerryError::from(error).at(path)),
    }
}

/// A change of case the file system refused as a rename onto an existing name (some network
/// shares): through a temporary name next to it. If the second step fails, the item gets its
/// old name back.
fn through_temporary(path: &Path, target: &Path) -> Result<PathBuf, JerryError> {
    let mut temporary = OsString::from(target.as_os_str());
    temporary.push(format!(".{}~jerry-rename", std::process::id()));
    let temporary = PathBuf::from(temporary);
    move_file(path, &temporary).map_err(|error| JerryError::from(error).at(path))?;
    match move_file(&temporary, target) {
        Ok(()) => Ok(target.to_path_buf()),
        Err(error) => {
            if let Err(back) = move_file(&temporary, path) {
                tracing::error!(%back, "a case-only rename couldn't restore the old name");
            }
            Err(JerryError::from(error).at(path))
        }
    }
}

/// `MoveFileExW` without replacing: a name that is taken fails with `ERROR_ALREADY_EXISTS`.
pub(crate) fn move_file(from: &Path, to: &Path) -> windows::core::Result<()> {
    let from = wide(from.as_os_str());
    let to = wide(to.as_os_str());
    // SAFETY: both point into NUL-terminated buffers that outlive the call.
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVE_FILE_FLAGS(0),
        )
    }
}

pub(crate) fn is_taken(error: &windows::core::Error) -> bool {
    matches!(
        WIN32_ERROR::from_error(error),
        Some(ERROR_ALREADY_EXISTS | ERROR_FILE_EXISTS)
    )
}

/// Whether two names are the same name to Windows: equal by its own case table.
fn same_ignoring_case(a: &OsStr, b: &OsStr) -> bool {
    let a: Vec<u16> = a.encode_wide().collect();
    let b: Vec<u16> = b.encode_wide().collect();
    // SAFETY: plain comparison of two slices that outlive the call.
    unsafe { CompareStringOrdinal(&a, &b, true) == CSTR_EQUAL }
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain([0]).collect()
}
