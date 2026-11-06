//! Paste in v0.1: the files go with the Shell's `IFileOperation`, with Windows' own progress
//! window, as from Explorer. Jerry's transfer engine takes this over in v0.2.
//!
//! A name that is taken never gets replaced: the pasted item gets a new name instead
//! (`FOF_RENAMEONCOLLISION`, "name (2).ext"). The Shell's Replace overwrites the old file for good
//! whatever the flags (`FOFX_RECYCLEONDELETE` recycles only what it deletes, not what it
//! overwrites, and the progress sink hears no delete to stop: #38's probe), and a replaced file
//! must go to the Recycle Bin (data safety rule 3). Pasting into the folder the items came from
//! makes copies there.
//!
//! Each paste gets its own short-lived STA thread, like opening files ([`open`](crate::open)): a
//! big copy takes minutes, and the Shell's dialogs wait for the user. Windows' messages
//! (progress, an error, the elevation prompt) are owned by Jerry's window.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use jerry_core::path::plain_path;
use jerry_core::{ErrorCode, JerryError};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::Shell::{
    COPYENGINE_E_USER_CANCELLED, FILEOPERATION_FLAGS, FOF_NO_UI, FOF_NOCONFIRMMKDIR,
    FOF_RENAMEONCOLLISION, FOFX_PRESERVEFILEEXTENSIONS, FileOperation, IFileOperation, IShellItem,
    SHCreateItemFromParsingName,
};
use windows::core::PCWSTR;

use crate::clipboard::Op;

/// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`: the user cancelled.
const ERROR_CANCELLED: i32 = 0x8007_04C7_u32 as i32;

/// How a paste ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pasted {
    /// The user cancelled, or skipped an item in one of Windows' messages: not everything went.
    pub aborted: bool,
}

/// Copies or moves (`op`) `sources` into the folder `destination`, on a thread of its own, with
/// Windows' progress window and messages owned by the window `owner`. Returns at once; the
/// receiver gets the outcome once the Shell is done. Sources that are gone are left out; if none
/// is left, it fails with `NotFound` for the first.
pub fn paste(
    owner: Option<isize>,
    sources: Vec<PathBuf>,
    destination: PathBuf,
    op: Op,
) -> mpsc::Receiver<Result<Pasted, JerryError>> {
    let (sender, outcome) = mpsc::channel();
    let started = std::thread::Builder::new()
        .name("jerry-paste".into())
        .spawn(move || {
            let result = jerry_core::job::run_job("paste", || {
                in_apartment(|| perform(owner, &sources, &destination, op, true))
            });
            let _ = sender.send(result);
        });
    if let Err(error) = started {
        tracing::error!(%error, "couldn't start a thread to paste");
        let (sender, failed) = mpsc::channel();
        let _ = sender.send(Err(JerryError::internal()));
        return failed;
    }
    outcome
}

/// Runs `work` in an STA of its own on this thread, as the Shell's copy engine needs.
fn in_apartment<T>(work: impl FnOnce() -> T) -> T {
    // SAFETY: no reserved pointer; this thread's apartment, ended below.
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    let result = work();
    if com.is_ok() {
        // SAFETY: matches the successful CoInitializeEx above, on the same thread.
        unsafe { CoUninitialize() };
    }
    result
}

/// One Shell operation for the whole paste. `ui` false (tests) keeps Windows' windows away.
fn perform(
    owner: Option<isize>,
    sources: &[PathBuf],
    destination: &Path,
    op: Op,
    ui: bool,
) -> Result<Pasted, JerryError> {
    // SAFETY: COM is initialized on this thread; FileOperation is the Shell's.
    let operation: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }?;
    let mut flags = FOF_NOCONFIRMMKDIR.0 | FOF_RENAMEONCOLLISION.0 | FOFX_PRESERVEFILEEXTENSIONS.0;
    if !ui {
        flags |= FOF_NO_UI.0;
    }
    // SAFETY: a valid operation and flags.
    unsafe { operation.SetOperationFlags(FILEOPERATION_FLAGS(flags)) }?;
    if let Some(owner) = owner {
        // SAFETY: Jerry's own window, alive while Jerry runs; a stale one only fails the call.
        unsafe { operation.SetOwnerWindow(HWND(owner as *mut core::ffi::c_void)) }?;
    }
    let into = shell_item(destination)?;

    let mut first_missing = None;
    let mut queued = 0;
    for source in sources {
        let item = match shell_item(source) {
            Ok(item) => item,
            Err(error) => {
                tracing::info!(path = %source.display(), "a source to paste is gone; left out");
                first_missing.get_or_insert(error);
                continue;
            }
        };
        match op {
            // SAFETY: valid items; the same name; no per-item sink.
            Op::Copy => unsafe { operation.CopyItem(&item, &into, PCWSTR::null(), None) },
            // SAFETY: as above.
            Op::Cut => unsafe { operation.MoveItem(&item, &into, PCWSTR::null(), None) },
        }?;
        queued += 1;
    }
    if queued == 0 {
        return Err(first_missing.unwrap_or_else(|| JerryError::new(ErrorCode::NotFound)));
    }

    // SAFETY: a valid operation; Windows' windows (when shown) are owned by `owner`.
    let performed = unsafe { operation.PerformOperations() };
    // SAFETY: a valid operation that has run.
    let aborted = unsafe { operation.GetAnyOperationsAborted() }
        .map(|aborted| aborted.as_bool())
        .unwrap_or(true);
    match performed {
        Ok(()) => Ok(Pasted { aborted }),
        // The user cancelled in Windows' window: they know.
        Err(error)
            if [ERROR_CANCELLED, COPYENGINE_E_USER_CANCELLED.0].contains(&error.code().0) =>
        {
            Ok(Pasted { aborted: true })
        }
        Err(error) => Err(error.into()),
    }
}

/// The Shell item for `path`, which the Shell takes without core's `\\?\` prefix. `NotFound` with
/// the path when it is gone.
fn shell_item(path: &Path) -> Result<IShellItem, JerryError> {
    let wide: Vec<u16> = plain_path(path)
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call; no bind context.
    unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }
        .map_err(|error| JerryError::from(error).at(path))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use jerry_core::path::long_path;

    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("jerry-paste-")
            .tempdir()
            .expect("temp folder")
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(folder)
            .expect("listable")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    fn quietly(sources: &[PathBuf], destination: &Path, op: Op) -> Result<Pasted, JerryError> {
        in_apartment(|| perform(None, sources, destination, op, false))
    }

    #[test]
    fn a_taken_name_keeps_the_old_file_and_the_paste_gets_a_new_name() {
        let dir = temp();
        let (from, to) = (dir.path().join("წყარო"), dir.path().join("სამიზნე"));
        fs::create_dir_all(&from).expect("source folder");
        fs::create_dir_all(to.join("საქაღალდე")).expect("destination folder");
        fs::write(from.join("ნახაზი.dwg"), "new").expect("source");
        fs::write(to.join("ნახაზი.dwg"), "old").expect("taken");
        fs::create_dir_all(from.join("საქაღალდე")).expect("source subfolder");
        fs::write(from.join("საქაღალდე").join("a.txt"), "new").expect("source inside");
        fs::write(to.join("საქაღალდე").join("a.txt"), "old").expect("taken inside");

        for op in [Op::Copy, Op::Cut] {
            let sources = [
                long_path(&from.join("ნახაზი.dwg")).expect("long"),
                long_path(&from.join("საქაღალდე")).expect("long"),
            ];
            let pasted = quietly(&sources, &long_path(&to).expect("long"), op).expect("pasted");
            assert!(!pasted.aborted, "{op:?}");
            // Nothing was overwritten, in the folder or inside the folder of the same name.
            assert_eq!(
                fs::read_to_string(to.join("ნახაზი.dwg")).expect("kept"),
                "old"
            );
            assert_eq!(
                fs::read_to_string(to.join("საქაღალდე").join("a.txt")).expect("kept"),
                "old"
            );
            if op == Op::Copy {
                assert!(from.join("ნახაზი.dwg").exists(), "a copy leaves the source");
            }
        }
        assert_eq!(
            names(&from),
            Vec::<String>::new(),
            "the cut moved everything"
        );
        let pasted = names(&to);
        assert_eq!(pasted.len(), 6, "{pasted:?}");
        assert!(
            pasted
                .iter()
                .all(|name| name.starts_with("ნახაზი") == name.ends_with(".dwg")),
            "extensions kept: {pasted:?}"
        );
        let copies = pasted
            .iter()
            .filter(|name| name.ends_with(".dwg"))
            .map(|name| fs::read_to_string(to.join(name)).expect("readable"))
            .filter(|content| content == "new")
            .count();
        assert_eq!(copies, 2, "{pasted:?}");
    }

    #[test]
    fn pasting_into_the_same_folder_makes_a_copy() {
        let dir = temp();
        let file = dir.path().join("ხარჯთაღრიცხვა.pdf");
        fs::write(&file, "pdf").expect("file");
        let pasted = quietly(std::slice::from_ref(&file), dir.path(), Op::Copy).expect("pasted");
        assert!(!pasted.aborted);
        let names = names(dir.path());
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.iter().all(|name| name.ends_with(".pdf")), "{names:?}");
        assert_eq!(fs::read_to_string(&file).expect("kept"), "pdf");
    }

    #[test]
    fn gone_sources_are_left_out_and_none_left_is_not_found() {
        let dir = temp();
        let (here, gone) = (dir.path().join("a.txt"), dir.path().join("გამქრალი.txt"));
        fs::write(&here, "a").expect("file");
        let into = dir.path().join("into");
        fs::create_dir(&into).expect("folder");

        let pasted = quietly(&[gone.clone(), here], &into, Op::Copy).expect("the rest pasted");
        assert!(!pasted.aborted);
        assert_eq!(names(&into), ["a.txt"]);

        let error = quietly(std::slice::from_ref(&gone), &into, Op::Copy).expect_err("nothing");
        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(error.path.as_deref(), Some(gone.as_path()));
    }
}
