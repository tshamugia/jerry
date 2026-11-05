//! Volumes: which one a folder is on, so that everything Jerry holds open on a volume can be let
//! go when Windows wants to remove or lock it (Tech PRD «Watcher», #126). Windows' notices about
//! that come through a window, which core doesn't have: `jerry-win` receives them and `src-tauri`
//! hands them to the watchers ([`Watchers::release`](crate::watcher::Watchers::release)).

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW};
use windows::core::PCWSTR;

use crate::error::{ErrorCode, JerryError};

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
