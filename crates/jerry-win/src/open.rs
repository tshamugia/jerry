//! Opening files in their programs, as Explorer does on Enter or a double click: `ShellExecuteExW`
//! with the default verb.
//!
//! Every request gets its own short-lived STA thread rather than a Shell pool worker. Opening can
//! take seconds (a program that answers by DDE, AutoCAD with a DWG say, starts up first), and a
//! pool worker held that long would hold up icons and thumbnails. A hung handler hangs only its
//! own thread; nothing waits for it.
//!
//! Windows shows its own messages, owned by Jerry's window, as for Explorer: "How do you want to
//! open this file?" for a type without a program, and its error box when a file can't be opened.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use jerry_core::path::plain_path;
use jerry_core::{ErrorCode, JerryError};
use windows::Win32::Foundation::{HWND, MAX_PATH};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    SEE_MASK_FLAG_LOG_USAGE, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::PCWSTR;

/// Opens `files` in their programs, one after another, on a thread of their own, with Windows'
/// messages owned by the window `owner`. Returns at once; the receiver gets the outcome when the
/// last one has started: the first error, if any. A cancelled prompt (Windows' "Open with", say)
/// isn't an error.
pub fn open_files(
    owner: Option<isize>,
    files: Vec<PathBuf>,
) -> mpsc::Receiver<Result<(), JerryError>> {
    let (sender, outcome) = mpsc::channel();
    let started = std::thread::Builder::new()
        .name("jerry-open".into())
        .spawn(move || {
            let result = jerry_core::job::run_job("open files", || open_all(owner, &files));
            let _ = sender.send(result);
        });
    if let Err(error) = started {
        tracing::error!(%error, "couldn't start a thread to open files");
        let (sender, failed) = mpsc::channel();
        let _ = sender.send(Err(JerryError::internal()));
        return failed;
    }
    outcome
}

fn open_all(owner: Option<isize>, files: &[PathBuf]) -> Result<(), JerryError> {
    in_sta(|| {
        let mut outcome = Ok(());
        for file in files {
            if let Err(error) = open_file(owner, file, 0) {
                tracing::info!(code = ?error.code, "couldn't open a file");
                outcome = outcome.and(Err(error));
            }
        }
        outcome
    })
}

/// Runs `job` with this thread in an STA of its own, for the length of the job: handlers that
/// COM activates need it (ShellExecuteEx's documentation).
pub(crate) fn in_sta<T>(job: impl FnOnce() -> T) -> T {
    // SAFETY: no reserved pointer; this thread's apartment, ended below.
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    let result = job();
    if com.is_ok() {
        // SAFETY: matches the successful CoInitializeEx above, on the same thread.
        unsafe { CoUninitialize() };
    }
    result
}

/// Opens a Shell name (`shell:RecycleBinFolder`) with its default verb, in Explorer for a folder.
pub(crate) fn open_shell_name(owner: Option<isize>, name: PCWSTR) -> Result<(), JerryError> {
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        hwnd: HWND(owner.unwrap_or(0) as *mut core::ffi::c_void),
        lpFile: name,
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` is filled in as documented; `name` is NUL-terminated and outlives the call.
    unsafe { ShellExecuteExW(&mut info) }.map_err(JerryError::from)
}

/// Opens `file` with its default verb, waiting until the program has it (`SEE_MASK_NOASYNC`, as
/// this thread ends soon after). `flags` adds `SEE_MASK_*` flags (tests turn Windows' messages off).
fn open_file(owner: Option<isize>, file: &Path, flags: u32) -> Result<(), JerryError> {
    let path = shell_form(file);
    let wide_file = wide(&path);
    let folder = path.parent().map(wide);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        // Log usage: Windows' frequent programs and Jump Lists, as when Explorer opens it.
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_LOG_USAGE | flags,
        hwnd: HWND(owner.unwrap_or(0) as *mut core::ffi::c_void),
        lpFile: PCWSTR(wide_file.as_ptr()),
        // Programs that open files next to it find them, as when opened from Explorer.
        lpDirectory: folder
            .as_ref()
            .map_or(PCWSTR::null(), |folder| PCWSTR(folder.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` is filled in as documented; its strings are NUL-terminated and outlive the
    // call.
    match unsafe { ShellExecuteExW(&mut info) } {
        Ok(()) => Ok(()),
        Err(error) => match JerryError::from(error) {
            error if error.code == ErrorCode::Cancelled => Ok(()),
            error => Err(error.at(file)),
        },
    }
}

/// The path as the Shell takes it: without the `\\?\` prefix while it is short enough, which most
/// programs need; a longer one keeps it, the only way to name it at all.
fn shell_form(file: &Path) -> PathBuf {
    let plain = plain_path(file);
    if plain.as_os_str().encode_wide().count() < MAX_PATH as usize {
        plain
    } else {
        file.to_path_buf()
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::Shell::SEE_MASK_FLAG_NO_UI;

    use super::*;

    #[test]
    fn short_paths_lose_the_prefix_and_long_ones_keep_it() {
        assert_eq!(
            shell_form(Path::new(r"\\?\C:\Work\ნახაზი.dwg")),
            PathBuf::from(r"C:\Work\ნახაზი.dwg")
        );
        let long = format!(r"\\?\C:\{}\a.txt", "x".repeat(300));
        assert_eq!(shell_form(Path::new(&long)), PathBuf::from(&long));
    }

    #[test]
    fn a_missing_file_fails_without_opening_anything() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing =
            jerry_core::path::long_path(&dir.path().join("ნახაზი.jerry-test")).expect("absolute");
        // Without Windows' own message, which would wait for a click.
        let error = open_file(None, &missing, SEE_MASK_FLAG_NO_UI).expect_err("nothing to open");
        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(error.path.as_deref(), Some(missing.as_path()));
    }
}
