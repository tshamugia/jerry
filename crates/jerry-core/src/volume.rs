//! Volumes: which one a folder is on, so that everything Jerry holds open on a volume can be let
//! go when Windows wants to remove or lock it (Tech PRD «Watcher», #126), and which disk or server
//! that is, for the transfer queue's lanes (#49). Windows' notices about volumes come through a
//! window, which core doesn't have: `jerry-win` receives them and `src-tauri` hands them to the
//! watchers ([`Watchers::release`](crate::watcher::Watchers::release)).

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDriveTypeW,
    GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
    OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::VOLUME_DISK_EXTENTS;
use windows::core::PCWSTR;

use crate::drives::{connection, quiet_errors, server_name};
use crate::error::{ErrorCode, JerryError, display_path};

/// Where a transfer's writes land. Jobs in one lane run one after another.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Lane {
    /// A physical disk, by Windows' number for it: all its volumes share it.
    Disk(u32),
    /// A network server, by its name in lower case: all its shares share it.
    Server(String),
    /// A volume Windows can't place on a disk (a virtual drive, or one that is gone), by its root
    /// in lower case, without the trailing backslash.
    Volume(String),
}

/// The lane of a transfer into `path`, a long path. Asks the local tables and the volume's device,
/// never a server, but it is blocking IO all the same.
pub fn lane(path: &Path) -> Lane {
    quiet_errors();
    if let Some(server) = server_name(&display_path(path)) {
        return Lane::Server(server.to_lowercase());
    }
    let Ok(root) = root(path) else {
        return volume_lane(&drive_prefix(path));
    };
    if let Some(server) = mapped_server(&root) {
        return Lane::Server(server.to_lowercase());
    }
    disk_number(&root).map_or_else(|| volume_lane(&root), Lane::Disk)
}

fn volume_lane(root: &Path) -> Lane {
    let text = display_path(root);
    Lane::Volume(text.trim_end_matches('\\').to_lowercase())
}

/// The first part of `path` (`\\?\E:`), for a drive that has no volume to ask.
fn drive_prefix(path: &Path) -> PathBuf {
    path.components()
        .next()
        .map_or_else(PathBuf::new, |prefix| PathBuf::from(prefix.as_os_str()))
}

/// The server of the mapped network drive at `root`, from the local connection table.
fn mapped_server(root: &Path) -> Option<String> {
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    if unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) } != DRIVE_REMOTE {
        return None;
    }
    let text = display_path(root);
    let letter = *text.as_bytes().first()?;
    server_name(&connection(letter).0?)
}

/// The number of the disk the volume at `root` starts on. A volume that spans disks counts as the
/// first one's. None where Windows has no disk to name: a network or virtual drive.
fn disk_number(root: &Path) -> Option<u32> {
    let mount: Vec<u16> = root.as_os_str().encode_wide().chain([0]).collect();
    // `\\?\Volume{guid}\`, always this long.
    let mut name = [0u16; 50];
    // SAFETY: `mount` is NUL-terminated and ends with a backslash; `name` is writable for its
    // length.
    unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(mount.as_ptr()), &mut name) }.ok()?;
    // Without the trailing backslash it is the volume itself and not its root folder.
    let end = name.iter().position(|&unit| unit == 0)?;
    *name.get_mut(end.checked_sub(1)?)? = 0;

    // SAFETY: a NUL-terminated volume name; no access is asked, so nothing is read or locked.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .ok()?;
    // Room for a volume over 16 extents, aligned for the structure. More fails, and it is a volume
    // Jerry can't place.
    let mut answer = [0u64; 50];
    let mut returned = 0u32;
    // SAFETY: an open volume handle; the answer is written within its size.
    let asked = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            None,
            0,
            Some(answer.as_mut_ptr().cast()),
            size_of_val(&answer) as u32,
            Some(&mut returned),
            None,
        )
    };
    // SAFETY: the handle came from CreateFileW above and is closed only here.
    let _ = unsafe { CloseHandle(handle) };
    if asked.is_err() || (returned as usize) < size_of::<VOLUME_DISK_EXTENTS>() {
        return None;
    }
    // SAFETY: the buffer is aligned and large enough, and Windows wrote at least one extent.
    let extents = unsafe { &*answer.as_ptr().cast::<VOLUME_DISK_EXTENTS>() };
    (extents.NumberOfDiskExtents > 0).then_some(extents.Extents[0].DiskNumber)
}

/// `error`, or [`ErrorCode::DeviceGone`] if it says `path` isn't there and that is because the
/// volume `path` is on has gone. Windows reports a device that went away only for the file it was
/// in the middle of; the next file is just "path not found".
pub fn missing_or_gone(error: JerryError, path: &Path) -> JerryError {
    if error.code == ErrorCode::NotFound
        && root(path).is_err_and(|cause| cause.code == ErrorCode::DeviceGone)
    {
        JerryError {
            code: ErrorCode::DeviceGone,
            ..error
        }
    } else {
        error
    }
}

/// `GetDriveTypeW`: a network share or mapped drive.
const DRIVE_REMOTE: u32 = 4;
/// `GetDriveTypeW`: no volume at that root.
const DRIVE_NO_ROOT_DIR: u32 = 1;
/// `GetDriveTypeW`: Windows can't tell.
const DRIVE_UNKNOWN: u32 = 0;

/// The root of the volume `path` is on, with a trailing backslash: `\\?\E:\` for a drive (its
/// letter upper case, so one volume has one root), the mount folder for a volume mounted in a
/// folder, `\\?\UNC\server\share\` for a share. `path` is a long path, as core keeps them.
///
/// Blocking IO: Windows looks for mount points along the path. Fails with
/// [`ErrorCode::DeviceGone`] on a drive letter with no volume behind it.
pub fn root(path: &Path) -> Result<PathBuf, JerryError> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // A root is never longer than the path it is the root of.
    let mut root = vec![0u16; wide.len() + 1];
    // SAFETY: `wide` is NUL-terminated and outlives the call; `root` is a writable buffer of its
    // length.
    unsafe { GetVolumePathNameW(PCWSTR(wide.as_ptr()), &mut root) }.map_err(|error| {
        let code = match ErrorCode::from_hresult(error.code()) {
            ErrorCode::NotFound | ErrorCode::InvalidName => ErrorCode::DeviceGone,
            code => code,
        };
        JerryError::new(code).at(path)
    })?;
    let len = root
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(root.len());
    root.truncate(len);
    upper_case_drive(&mut root);
    Ok(PathBuf::from(OsString::from_wide(&root)))
}

/// Upper-cases the letter of a drive root (`\\?\e:\` or `e:\`).
fn upper_case_drive(root: &mut [u16]) {
    let prefix: Vec<u16> = r"\\?\".encode_utf16().collect();
    let at = if root.starts_with(&prefix) {
        prefix.len()
    } else {
        0
    };
    if root.get(at + 1) == Some(&u16::from(b':'))
        && let Some(letter) = root.get_mut(at)
        && (u16::from(b'a')..=u16::from(b'z')).contains(letter)
    {
        *letter -= u16::from(b'a' - b'A');
    }
}

/// Whether the volume at `root` is on this computer, so Windows can say when it is about to be
/// removed or locked: not a network share or mapped drive, and not a drive that is gone.
pub fn is_local(root: &Path) -> bool {
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    let kind = unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) };
    !matches!(kind, DRIVE_REMOTE | DRIVE_NO_ROOT_DIR | DRIVE_UNKNOWN)
}

/// Whether `a` and `b` are the same root, as Windows compares paths: ordinal, ignoring case.
pub fn same_root(a: &Path, b: &Path) -> bool {
    let a: Vec<u16> = a.as_os_str().encode_wide().collect();
    let b: Vec<u16> = b.as_os_str().encode_wide().collect();
    // SAFETY: both are valid slices.
    unsafe { CompareStringOrdinal(&a, &b, true) == CSTR_EQUAL }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::long_path;

    #[test]
    fn a_folder_is_on_its_drive_root() {
        let dir = tempfile::tempdir().expect("temp folder");
        let folder = long_path(&dir.path().join("ნახაზები")).expect("long path");
        std::fs::create_dir(&folder).expect("mkdir");
        let root = root(&folder).expect("a root");
        let drive = long_path(dir.path())
            .expect("long path")
            .components()
            .next()
            .map(|prefix| {
                let mut drive = prefix.as_os_str().to_owned();
                drive.push(r"\");
                PathBuf::from(drive)
            })
            .expect("a drive");
        assert!(same_root(&root, &drive), "{root:?} on {drive:?}");
        assert!(root.to_string_lossy().ends_with(r":\"));
        assert!(is_local(&root));
    }

    #[test]
    fn drive_letters_are_upper_case() {
        let mut root: Vec<u16> = r"\\?\e:\".encode_utf16().collect();
        upper_case_drive(&mut root);
        assert_eq!(String::from_utf16_lossy(&root), r"\\?\E:\");
        let mut share: Vec<u16> = r"\\?\UNC\server\share\".encode_utf16().collect();
        upper_case_drive(&mut share);
        assert_eq!(String::from_utf16_lossy(&share), r"\\?\UNC\server\share\");
    }

    #[test]
    fn roots_compare_without_case() {
        assert!(same_root(Path::new(r"\\?\E:\"), Path::new(r"\\?\e:\")));
        assert!(!same_root(Path::new(r"\\?\E:\"), Path::new(r"\\?\F:\")));
    }

    #[test]
    fn a_local_folder_is_on_a_disk_and_a_share_on_its_server() {
        let dir = tempfile::tempdir().expect("temp folder");
        let folder = long_path(dir.path()).expect("long path");
        assert!(
            matches!(lane(&folder), Lane::Disk(_)),
            "{:?}",
            lane(&folder)
        );
        // The disk is a property of the volume: another folder there is in the same lane.
        let other = long_path(&std::env::temp_dir()).expect("long path");
        assert_eq!(lane(&folder), lane(&other));

        assert_eq!(
            lane(Path::new(r"\\?\UNC\INSTA-NAS01\Projects\2026")),
            Lane::Server("insta-nas01".into())
        );
        assert_eq!(
            lane(Path::new(r"\\?\UNC\insta-nas01\Other")),
            Lane::Server("insta-nas01".into()),
            "one server, whatever the share"
        );
    }

    #[test]
    fn a_drive_that_is_gone_is_a_lane_of_its_own() {
        let letter = (b'J'..=b'Z')
            .rev()
            .map(char::from)
            .find(|letter| !Path::new(&format!(r"{letter}:\")).exists())
            .expect("a free drive letter");
        let path = format!(r"\\?\{letter}:\Photos");
        assert_eq!(
            lane(Path::new(&path)),
            Lane::Volume(format!("{letter}:").to_lowercase())
        );
    }

    #[test]
    fn not_found_on_a_volume_that_is_gone_is_device_gone() {
        let letter = (b'J'..=b'Z')
            .rev()
            .map(char::from)
            .find(|letter| !Path::new(&format!(r"{letter}:\")).exists())
            .expect("a free drive letter");
        let gone = PathBuf::from(format!(r"\\?\{letter}:\a.txt"));
        let error = missing_or_gone(JerryError::new(ErrorCode::NotFound).at(&gone), &gone);
        assert_eq!(error.code, ErrorCode::DeviceGone);
        assert_eq!(
            error.path.as_deref(),
            Some(gone.as_path()),
            "the path stays"
        );

        let dir = tempfile::tempdir().expect("temp folder");
        let here = long_path(&dir.path().join("a.txt")).expect("long path");
        let error = missing_or_gone(JerryError::new(ErrorCode::NotFound).at(&here), &here);
        assert_eq!(error.code, ErrorCode::NotFound, "only the file is missing");
        let error = missing_or_gone(JerryError::from_win32(5).at(&gone), &gone);
        assert_eq!(error.code, ErrorCode::AccessDenied, "other errors stay");
    }

    #[test]
    fn a_drive_with_no_volume_has_no_root() {
        // A letter nobody has mapped: walk back from Z: to find one.
        let letter = (b'J'..=b'Z')
            .rev()
            .map(char::from)
            .find(|letter| !Path::new(&format!(r"{letter}:\")).exists())
            .expect("a free drive letter");
        let error = root(Path::new(&format!(r"\\?\{letter}:\Photos"))).expect_err("no volume");
        assert_eq!(error.code, ErrorCode::DeviceGone);
        assert!(!is_local(Path::new(&format!(r"\\?\{letter}:\"))));
    }
}
