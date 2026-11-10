//! The Windows clipboard, on the main STA thread, used the way Explorer uses it (Tech PRD
//! «Clipboard», spike #15).
//!
//! - Text: Copy path and Copy name put `CF_UNICODETEXT` there ([`set_text`]).
//! - Files: Copy and Cut put the Shell's own data object there, the one the files' parent folder
//!   makes for Explorer ([`put_files`]). Paste targets (Explorer, the desktop, Outlook, Telegram)
//!   get what Explorer's Copy gives them: Shell IDList Array, `CF_HDROP`, FileNameW, FileContents
//!   and the rest, plus "Preferred DropEffect", which tells Copy (COPY | LINK, as Explorer writes
//!   it) from Cut (MOVE).
//! - Paste reads `CF_HDROP` and "Preferred DropEffect", whoever put them there ([`read_files`]).
//!   After moving the files of a cut, the target says so with "Paste Succeeded" and empties the
//!   clipboard, as Explorer does ([`finish_cut`]). Jerry moves the files itself, so it never asks
//!   the source to delete anything, and as a source it never deletes on what a target writes back.
//! - Jerry's own window hears every change ([`watch`]). Once the clipboard no longer holds Jerry's
//!   files, Jerry's cut is over ([`cut_paths`] is empty): its rows stop showing dimmed.
//! - At exit, [`flush`] keeps Jerry's files on the clipboard: without `OleFlushClipboard` it is
//!   empty once Jerry is gone. A cut stays a cut.
//!
//! Everything here except [`cut_paths`] runs on the main thread (`run_on_main`), after
//! [`main_thread::init`](crate::main_thread::init): the data object Jerry put on the clipboard is
//! a COM object of that thread's apartment, and Jerry's own window there owns the clipboard.

use std::cell::RefCell;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use jerry_core::JerryError;
use jerry_core::job::run_job;
use windows::Win32::Foundation::{HANDLE, LRESULT, S_OK};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Ole::{
    CF_HDROP, CF_UNICODETEXT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, OleFlushClipboard,
    OleGetClipboard, OleSetClipboard,
};
use windows::Win32::UI::Shell::{CFSTR_PASTESUCCEEDED, CFSTR_PREFERREDDROPEFFECT};
use windows::core::{HRESULT, Interface};

use crate::data_object::{
    dword, files as files_data_object, free, global_copy, hdrop_paths, hglobal_format, set_dword,
};
use crate::main_thread::message_window;

/// Another program may hold the clipboard open for a moment: tries this many times.
const OPEN_TRIES: u32 = 5;
const OPEN_RETRY: Duration = Duration::from_millis(10);

/// `CLIPBRD_E_CANT_OPEN`: another program has the clipboard open.
const CANT_OPEN: HRESULT = HRESULT(0x8004_00D0_u32 as i32);

/// Copy or Cut, as "Preferred DropEffect" says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Copy,
    Cut,
}

impl Op {
    /// What Explorer writes: COPY | LINK for Copy (so Paste Shortcut works), MOVE for Cut.
    fn preferred_effect(self) -> u32 {
        match self {
            Self::Copy => DROPEFFECT_COPY.0 | DROPEFFECT_LINK.0,
            Self::Cut => DROPEFFECT_MOVE.0,
        }
    }

    /// A cut is MOVE without COPY. Anything else, a missing format included, is a copy: programs
    /// that put only `CF_HDROP` on the clipboard mean Copy, and COPY | MOVE leaves the choice to
    /// the target, which takes the safe one.
    fn from_preferred_effect(effect: Option<u32>) -> Self {
        match effect {
            Some(e) if e & DROPEFFECT_MOVE.0 != 0 && e & DROPEFFECT_COPY.0 == 0 => Self::Cut,
            _ => Self::Copy,
        }
    }
}

/// Files on the clipboard, as Paste reads them.
#[derive(Debug, Clone)]
pub struct ClipboardFiles {
    /// `CF_HDROP`'s paths, exactly as the source wrote them (no `\\?\` prefix).
    pub paths: Vec<PathBuf>,
    pub op: Op,
    /// The clipboard's sequence number when it was read: [`finish_cut`] touches the clipboard
    /// only if it still holds the same thing.
    sequence: u32,
}

thread_local! {
    /// The data object Jerry last put on the clipboard (main thread only).
    static OURS: RefCell<Option<IDataObject>> = const { RefCell::new(None) };
}

/// The paths of Jerry's cut while the clipboard holds it; empty otherwise. Read from any thread.
static CUT: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Called on every clipboard change ([`watch`]).
static ON_CHANGE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

fn cut() -> MutexGuard<'static, Vec<PathBuf>> {
    CUT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Puts `text` on the clipboard as Unicode text, its UTF-16 exactly as it is (a path with a name
/// that isn't valid Unicode keeps it). Main thread only (see the module docs).
pub fn set_text(text: &OsStr) -> Result<(), JerryError> {
    let Some(owner) = message_window() else {
        tracing::error!("clipboard used before main_thread::init");
        return Err(JerryError::internal());
    };
    let units: Vec<u16> = text.encode_wide().chain([0]).collect();
    let memory = global_copy(bytes_of(&units))?;
    let mut opened = Err(JerryError::internal());
    for _ in 0..OPEN_TRIES {
        // SAFETY: Jerry's own window, alive for the life of the process.
        opened = unsafe { OpenClipboard(Some(owner)) }.map_err(JerryError::from);
        if opened.is_ok() {
            break;
        }
        std::thread::sleep(OPEN_RETRY);
    }
    if let Err(error) = opened {
        free(memory);
        return Err(error);
    }
    // SAFETY: the clipboard is open for this thread, with Jerry's window as its owner.
    let set = unsafe { EmptyClipboard() }
        // SAFETY: `memory` holds NUL-terminated UTF-16; on success the clipboard owns it.
        .and_then(|()| unsafe { SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(memory.0))) });
    if set.is_err() {
        free(memory);
    }
    // SAFETY: closes the clipboard this thread opened above.
    if let Err(error) = unsafe { CloseClipboard() } {
        tracing::warn!(%error, "couldn't close the clipboard");
    }
    set.map(|_| ()).map_err(JerryError::from)
}

/// Copy (Ctrl+C) or Cut (Ctrl+X): puts `paths` (core's, `\\?\` long paths welcome) on the
/// clipboard as Explorer does. Main thread only. `NotFound` with its path for a file that is gone.
pub fn put_files(paths: &[PathBuf], op: Op) -> Result<(), JerryError> {
    let data = files_data_object(paths)?;
    set_dword(&data, CFSTR_PREFERREDDROPEFFECT, op.preferred_effect())?;
    // Before OleSetClipboard: the change it causes reaches `on_update`, which reads them.
    let before_cut = std::mem::replace(
        &mut *cut(),
        if op == Op::Cut {
            paths.to_vec()
        } else {
            Vec::new()
        },
    );
    let before_ours = OURS.with_borrow_mut(|ours| ours.replace(data.clone()));
    let mut set = Ok(());
    for _ in 0..OPEN_TRIES {
        // SAFETY: a live data object of this thread; OLE takes its own reference.
        set = unsafe { OleSetClipboard(&data) };
        match &set {
            Err(error) if error.code() == CANT_OPEN => std::thread::sleep(OPEN_RETRY),
            _ => break,
        }
    }
    if let Err(error) = set {
        // The clipboard still holds what it held, so Jerry's earlier cut still stands.
        *cut() = before_cut;
        OURS.with_borrow_mut(|ours| *ours = before_ours);
        return Err(error.into());
    }
    Ok(())
}

/// The paths of Jerry's cut while the clipboard still holds it, as [`put_files`] got them; empty
/// once anything else is on the clipboard. Any thread.
pub fn cut_paths() -> Vec<PathBuf> {
    cut().clone()
}

/// Whether Jerry's cut is on the clipboard ([`cut_paths`] isn't empty). Any thread.
pub fn has_cut() -> bool {
    !cut().is_empty()
}

/// Whether the clipboard holds files to paste (`CF_HDROP`), whoever put them there: the context
/// menu grays out Paste without them, as Explorer does. Any thread; the clipboard stays closed.
pub fn has_files() -> bool {
    // SAFETY: a plain query of the clipboard's formats.
    unsafe { IsClipboardFormatAvailable(u32::from(CF_HDROP.0)) }.is_ok()
}

/// Reads the files on the clipboard, whoever put them there. `None` when it holds no files (text,
/// an image, nothing). Main thread only.
pub fn read_files() -> Result<Option<ClipboardFiles>, JerryError> {
    // SAFETY: no arguments.
    let sequence = unsafe { GetClipboardSequenceNumber() };
    // SAFETY: plain call; the clipboard's object comes back with a reference we own.
    let data = unsafe { OleGetClipboard() }?;
    // SAFETY: a plain COM call with a valid FORMATETC.
    if unsafe { data.QueryGetData(&hglobal_format(CF_HDROP.0)) } != S_OK {
        return Ok(None);
    }
    let paths = hdrop_paths(&data)?;
    if paths.is_empty() {
        return Ok(None);
    }
    Ok(Some(ClipboardFiles {
        paths,
        op: Op::from_preferred_effect(dword(&data, CFSTR_PREFERREDDROPEFFECT)),
        sequence,
    }))
}

/// Call once the files of a pasted cut are moved. If the clipboard still holds that cut, tells its
/// source ("Paste Succeeded" = MOVE without "Performed DropEffect": the files are moved, delete
/// nothing; MSDN «Handling Shell Data Transfer Scenarios», delete-on-paste) and empties the
/// clipboard, as Explorer does after pasting a cut, so a second Paste doesn't try to move the
/// same files again. Explorer as the source doesn't empty it itself (spike #15). Main thread only.
pub fn finish_cut(files: &ClipboardFiles) {
    // SAFETY: no arguments.
    if unsafe { GetClipboardSequenceNumber() } != files.sequence {
        return; // Something else was copied meanwhile: leave it.
    }
    // SAFETY: plain call; the clipboard's object comes back with a reference we own.
    match unsafe { OleGetClipboard() } {
        Ok(data) => {
            if let Err(error) = set_dword(&data, CFSTR_PASTESUCCEEDED, DROPEFFECT_MOVE.0) {
                // The source may be gone (it exited and flushed): nobody left to tell.
                tracing::debug!(code = ?error.code, "couldn't tell the source about the paste");
            }
        }
        Err(error) => tracing::debug!(%error, "couldn't read the clipboard after a paste"),
    }
    // SAFETY: a null object empties the clipboard.
    if let Err(error) = unsafe { OleSetClipboard(None) } {
        tracing::warn!(%error, "couldn't empty the clipboard after pasting a cut");
    }
}

/// Calls `on_change` on every clipboard change, Jerry's own included, once Jerry has noted
/// whether its cut is over ([`cut_paths`]). It runs on the main thread, inside Jerry's window
/// procedure: keep it short, and don't touch the clipboard from it. Call once, on the main
/// thread, after [`main_thread::init`](crate::main_thread::init).
pub fn watch(on_change: impl Fn() + Send + Sync + 'static) -> Result<(), JerryError> {
    let Some(window) = message_window() else {
        tracing::error!("clipboard watched before main_thread::init");
        return Err(JerryError::internal());
    };
    if ON_CHANGE.set(Box::new(on_change)).is_err() {
        tracing::warn!("the clipboard is watched already");
        return Ok(());
    }
    // SAFETY: Jerry's own window, on this thread, alive for the life of the process.
    unsafe { AddClipboardFormatListener(window) }?;
    Ok(())
}

/// `WM_CLIPBOARDUPDATE` on Jerry's window: Jerry's cut is over once the clipboard holds anything
/// else. Jerry's own changes arrive here too, which is why [`put_files`] notes the cut first.
pub(crate) fn on_update() -> LRESULT {
    // Unwinding out of a window procedure would abort the process.
    let _ = run_job("clipboard change", || {
        let lost = OURS.with_borrow_mut(|ours| {
            let lost = ours.as_ref().is_some_and(|data| !is_current(data));
            if lost {
                *ours = None;
            }
            lost
        });
        if lost {
            cut().clear();
        }
        if let Some(on_change) = ON_CHANGE.get() {
            on_change();
        }
        Ok(())
    });
    LRESULT(0)
}

/// At exit, on the main thread: if the clipboard still holds Jerry's files, renders them into it
/// (`OleFlushClipboard`), so they can still be pasted once Jerry is gone.
pub fn flush() {
    let ours = OURS.with_borrow(|ours| ours.as_ref().is_some_and(is_current));
    if !ours {
        return;
    }
    // SAFETY: plain call on the thread that put the object on the clipboard.
    match unsafe { OleFlushClipboard() } {
        Ok(()) => tracing::info!("Jerry's files stay on the clipboard"),
        Err(error) => tracing::error!(%error, "OleFlushClipboard failed"),
    }
}

/// The clipboard still holds `data`: nothing else was copied since.
fn is_current(data: &IDataObject) -> bool {
    // The `windows` wrapper folds S_FALSE ("not current") into Ok, so call the export directly.
    windows::core::link!("ole32.dll" "system" fn OleIsCurrentClipboard(
        pdataobj: *mut core::ffi::c_void,
    ) -> HRESULT);
    // SAFETY: a live data object borrowed for the call.
    let current = unsafe { OleIsCurrentClipboard(data.as_raw()) };
    current == S_OK
}

/// `units` as the little-endian bytes Windows keeps them in.
fn bytes_of(units: &[u16]) -> &[u8] {
    // SAFETY: u16 has no padding and a stricter alignment than u8; the slice covers exactly the
    // same memory, borrowed as long as `units`.
    unsafe { std::slice::from_raw_parts(units.as_ptr().cast::<u8>(), size_of_val(units)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_object::tests::{files, format_names, has, sta, temp};

    // These tests build data objects but never put them on the clipboard: that is the user's.

    #[test]
    fn copy_and_cut_carry_explorers_formats_and_the_effect() {
        sta();
        let (_dir, dir) = temp();
        let paths = files(&dir, &["ნახაზი — სართული 1.dwg", "ხარჯთაღრიცხვა.pdf"]);
        let long: Vec<PathBuf> = paths
            .iter()
            .map(|path| jerry_core::path::long_path(path).unwrap())
            .collect();

        for (op, effect) in [(Op::Copy, 5), (Op::Cut, 2)] {
            let data = files_data_object(&long).unwrap();
            set_dword(&data, CFSTR_PREFERREDDROPEFFECT, op.preferred_effect()).unwrap();
            let formats = format_names(&data);
            // OleFlushClipboard renders only what EnumFormatEtc lists, so the effect must be
            // listed too, or a cut would turn into a copy once Jerry exits.
            for expected in [
                "Shell IDList Array",
                "CF_HDROP",
                "FileNameW",
                "Preferred DropEffect",
            ] {
                assert!(has(&formats, expected), "{expected}: {formats:?}");
            }
            assert_eq!(dword(&data, CFSTR_PREFERREDDROPEFFECT), Some(effect));
            assert_eq!(Op::from_preferred_effect(Some(effect)), op);
            // Without the `\?\` prefix: other programs don't understand it.
            assert_eq!(hdrop_paths(&data).unwrap(), paths);
        }
    }

    #[test]
    fn preferred_effect_reading() {
        assert_eq!(Op::from_preferred_effect(None), Op::Copy);
        assert_eq!(Op::from_preferred_effect(Some(1)), Op::Copy);
        assert_eq!(Op::from_preferred_effect(Some(5)), Op::Copy);
        assert_eq!(Op::from_preferred_effect(Some(2)), Op::Cut);
        // COPY | MOVE leaves the choice to the target; Jerry picks the safe one.
        assert_eq!(Op::from_preferred_effect(Some(3)), Op::Copy);
    }
}
