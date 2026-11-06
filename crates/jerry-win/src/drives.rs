//! The Shell's side of the sidebar (#34): its standard places, ejecting a drive, and opening
//! Recycle Bin in Explorer. Which drives there are and their free space come from
//! `jerry_core::drives`; that a drive arrived or went comes from [`devices`](crate::devices).

use std::path::PathBuf;

use jerry_core::{ErrorCode, JerryError};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    SEE_MASK_INVOKEIDLIST, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, w};

use crate::icons::Place;
use crate::open::{in_sta, open_shell_name};

/// The standard places the sidebar shows, in its order (PRD «Sidebar»), with their folders:
/// Desktop, Downloads, Documents, Pictures, OneDrive. A place this user doesn't have (OneDrive,
/// before it is set up) is left out.
pub fn places() -> Vec<(Place, PathBuf)> {
    [
        Place::Desktop,
        Place::Downloads,
        Place::Documents,
        Place::Pictures,
        Place::OneDrive,
    ]
    .into_iter()
    .filter_map(|place| place.path().map(|path| (place, path)))
    .collect()
}

/// Ejects drive `letter` as Explorer's Eject does: Windows lets go of a USB drive ("Safe to
/// remove hardware") or opens an optical drive's tray. If something holds the drive, Windows says
/// so in its own message, owned by `owner`.
///
/// Runs on a Shell pool thread (an STA). It can take seconds: Windows first asks every program,
/// Jerry among them (`devices`), to let go of the volume.
pub fn eject(owner: Option<isize>, letter: u8) -> Result<(), JerryError> {
    invoke_verb(owner, letter, 0)
}

fn invoke_verb(owner: Option<isize>, letter: u8, flags: u32) -> Result<(), JerryError> {
    let letter = letter.to_ascii_uppercase();
    if !letter.is_ascii_uppercase() {
        return Err(JerryError::new(ErrorCode::NotFound));
    }
    let root: Vec<u16> = format!("{}:\\", char::from(letter))
        .encode_utf16()
        .chain([0])
        .collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        // The drive's own context menu verb, not one from the registry; wait for it.
        fMask: SEE_MASK_INVOKEIDLIST | SEE_MASK_NOASYNC | flags,
        hwnd: HWND(owner.unwrap_or(0) as *mut core::ffi::c_void),
        lpVerb: w!("eject"),
        lpFile: PCWSTR(root.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` is filled in as documented; its strings are NUL-terminated and outlive the
    // call.
    match unsafe { ShellExecuteExW(&mut info) } {
        Ok(()) => Ok(()),
        Err(error) => match JerryError::from(error) {
            error if error.code == ErrorCode::Cancelled => Ok(()),
            error => Err(error),
        },
    }
}

/// Opens Recycle Bin in Explorer: Jerry's own view of it comes in v1.0 (#88). Runs on a thread
/// of its own, like [`open_files`](crate::open::open_files); returns at once.
pub fn open_recycle_bin(owner: Option<isize>) {
    let started = std::thread::Builder::new()
        .name("jerry-open".into())
        .spawn(move || {
            let _ = jerry_core::job::run_job("open Recycle Bin", || {
                in_sta(|| open_shell_name(owner, w!("shell:RecycleBinFolder")))
            })
            .inspect_err(|error| tracing::warn!(code = ?error.code, "Recycle Bin didn't open"));
        });
    if let Err(error) = started {
        tracing::error!(%error, "couldn't start a thread to open Recycle Bin");
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::Shell::SEE_MASK_FLAG_NO_UI;

    use super::*;

    /// No Windows message waits for a click.
    const NO_UI: u32 = SEE_MASK_FLAG_NO_UI;

    #[test]
    fn places_are_the_user_folders_in_sidebar_order() {
        let places = places();
        let order: Vec<Place> = places.iter().map(|(place, _)| *place).collect();
        // OneDrive may not be set up on a build machine; the others always are.
        assert_eq!(
            order[..4],
            [
                Place::Desktop,
                Place::Downloads,
                Place::Documents,
                Place::Pictures
            ]
        );
        for (place, path) in &places {
            assert!(path.is_absolute(), "{place:?}: {}", path.display());
        }
    }

    #[test]
    fn a_drive_that_isnt_there_doesnt_eject() {
        // SAFETY: no arguments; returns the bitmask of drive letters.
        let drives = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        let missing = (0..26u8)
            .rev()
            .find(|bit| drives & (1 << bit) == 0)
            .expect("a free drive letter");
        assert!(in_sta(|| invoke_verb(None, b'A' + missing, NO_UI)).is_err());
        assert_eq!(
            invoke_verb(None, b'1', NO_UI).map_err(|error| error.code),
            Err(ErrorCode::NotFound)
        );
    }
}
