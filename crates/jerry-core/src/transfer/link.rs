//! Copying a link as a link. What it points to is neither read nor copied.
//!
//! A symbolic link is made the usual way, which needs Developer Mode or elevation: without either,
//! that link fails with [`AccessDenied`](crate::ErrorCode::AccessDenied) and the job goes on.
//! `CopyFileExW` refuses junctions, so a junction is an empty folder given the same reparse data;
//! that needs no privilege.

use std::fs::{File, OpenOptions};
use std::os::windows::fs::{OpenOptionsExt, symlink_dir, symlink_file};
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows::Win32::Foundation::{ERROR_NOT_SUPPORTED, HANDLE};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{FSCTL_GET_REPARSE_POINT, FSCTL_SET_REPARSE_POINT};

use super::copy::stamp;
use crate::delete::Item;
use crate::error::{ErrorCode, JerryError};

const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;

/// The most a reparse point can hold (`MAXIMUM_REPARSE_DATA_BUFFER_SIZE`).
const REPARSE_DATA_MAX: usize = 16 * 1024;

/// Makes a link at `target` like the link `item`. An existing target is never replaced.
pub(super) fn copy_link(item: &Item, target: &Path) -> Result<(), JerryError> {
    if stamp(target).is_ok() {
        return Err(JerryError::new(ErrorCode::InvalidName).at(target));
    }
    match item.tag {
        IO_REPARSE_TAG_SYMLINK => copy_symlink(item, target),
        IO_REPARSE_TAG_MOUNT_POINT => copy_junction(&item.path, target),
        // Other kinds of link have no way to be made here.
        _ => Err(JerryError::from_win32(ERROR_NOT_SUPPORTED.0).at(&item.path)),
    }
}

fn copy_symlink(item: &Item, target: &Path) -> Result<(), JerryError> {
    let points_to =
        std::fs::read_link(&item.path).map_err(|error| JerryError::from(error).at(&item.path))?;
    let made = if item.is_dir() {
        symlink_dir(points_to, target)
    } else {
        symlink_file(points_to, target)
    };
    made.map_err(|error| JerryError::from(error).at(target))
}

fn copy_junction(source: &Path, target: &Path) -> Result<(), JerryError> {
    let data = read_reparse_data(source).map_err(|error| error.at(source))?;
    std::fs::create_dir(target).map_err(|error| {
        let code = if error.kind() == std::io::ErrorKind::AlreadyExists {
            JerryError::new(ErrorCode::InvalidName)
        } else {
            JerryError::from(error)
        };
        code.at(target)
    })?;
    write_reparse_data(target, &data).map_err(|error| {
        // Still the plain empty folder this call made.
        if let Err(removing) = std::fs::remove_dir(target) {
            tracing::warn!(%removing, "can't remove the folder of a junction that failed");
        }
        error.at(target)
    })
}

/// Opens the link at `path` itself. `OpenOptions` needs the reparse point flag not to follow it.
fn open_link(options: &mut OpenOptions, path: &Path) -> Result<File, JerryError> {
    options
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .map_err(JerryError::from)
}

fn read_reparse_data(path: &Path) -> Result<Vec<u8>, JerryError> {
    let file = open_link(OpenOptions::new().access_mode(0), path)?;
    let mut data = vec![0_u8; REPARSE_DATA_MAX];
    let mut length = 0_u32;
    // SAFETY: the handle belongs to `file`, and `data` and `length` are writable and outlive the
    // call. No OVERLAPPED: `file` is synchronous.
    unsafe {
        DeviceIoControl(
            HANDLE(file.as_raw_handle()),
            FSCTL_GET_REPARSE_POINT,
            None,
            0,
            Some(data.as_mut_ptr().cast()),
            REPARSE_DATA_MAX as u32,
            Some(&raw mut length),
            None,
        )
    }?;
    data.truncate(length as usize);
    Ok(data)
}

fn write_reparse_data(path: &Path, data: &[u8]) -> Result<(), JerryError> {
    let file = open_link(OpenOptions::new().write(true), path)?;
    let mut length = 0_u32;
    // SAFETY: the handle belongs to `file`; `data` is what the same control code returned, and
    // outlives the call, as does `length`. No OVERLAPPED: `file` is synchronous.
    unsafe {
        DeviceIoControl(
            HANDLE(file.as_raw_handle()),
            FSCTL_SET_REPARSE_POINT,
            Some(data.as_ptr().cast()),
            data.len() as u32,
            None,
            0,
            Some(&raw mut length),
            None,
        )
    }?;
    Ok(())
}
