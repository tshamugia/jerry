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
use std::ffi::{OsStr, OsString};
use std::mem::ManuallyDrop;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use jerry_core::job::run_job;
use jerry_core::path::plain_path;
use jerry_core::{ErrorCode, JerryError};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, LRESULT, S_OK};
use windows::Win32::System::Com::{
    DVASPECT_CONTENT, FORMATETC, IBindCtx, IDataObject, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardSequenceNumber,
    OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, CF_UNICODETEXT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, OleFlushClipboard,
    OleGetClipboard, OleSetClipboard, ReleaseStgMedium,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, CFSTR_PASTESUCCEEDED, CFSTR_PREFERREDDROPEFFECT, DROPFILES, DragQueryFileW,
    HDROP, ILCreateFromPathW, ILFree, SHCreateDataObject, SHCreateShellItemArrayFromIDLists,
};
use windows::core::{HRESULT, Interface, PCWSTR};

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

/// The data object for `paths`: the Shell's own, or `CF_HDROP` alone for existing files the Shell
/// can't parse. Core's `\\?\` paths go to the Shell and into `CF_HDROP` without the prefix, which
/// no other program understands; the Shell parses plain paths longer than `MAX_PATH` fine.
fn files_data_object(paths: &[PathBuf]) -> Result<IDataObject, JerryError> {
    let plain: Vec<PathBuf> = paths.iter().map(|path| plain_path(path)).collect();
    match shell_data_object(&plain) {
        Err(unparsed) if unparsed.code == ErrorCode::NotFound => {
            let exists = unparsed.path.as_ref().is_some_and(|path| path.exists());
            if !exists {
                return Err(unparsed);
            }
            tracing::warn!(path = ?unparsed.path, "the Shell can't parse it; CF_HDROP only");
            hdrop_data_object(&plain)
        }
        result => result,
    }
}

/// The Shell data object for `paths`, the same one Explorer copies and drags: the items' parent
/// folder makes it (`IShellItemArray::BindToHandler(BHID_DataObject)` ends in its
/// `GetUIObjectOf`). `SHCreateDataObject` with absolute PIDLs offers nothing (spike #13).
/// `NotFound` with the path the Shell can't parse.
fn shell_data_object(paths: &[PathBuf]) -> Result<IDataObject, JerryError> {
    if paths.is_empty() {
        return Err(JerryError::new(ErrorCode::NotFound));
    }
    let pidls = paths
        .iter()
        .map(|path| {
            Pidl::from_path(path).ok_or_else(|| JerryError::new(ErrorCode::NotFound).at(path))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let raw: Vec<*const ITEMIDLIST> = pidls.iter().map(Pidl::as_ptr).collect();
    // SAFETY: every pointer is a valid absolute ID list that `pidls` keeps alive for the call;
    // the array copies what it needs.
    let items = unsafe { SHCreateShellItemArrayFromIDLists(&raw) }?;
    // SAFETY: a plain COM call on a live object; no bind context is needed.
    let data =
        unsafe { items.BindToHandler::<_, IDataObject>(None::<&IBindCtx>, &BHID_DataObject) }?;
    Ok(data)
}

/// A data object holding only `CF_HDROP` with `paths`: the Shell's generic object, which keeps
/// any format set on it.
fn hdrop_data_object(paths: &[PathBuf]) -> Result<IDataObject, JerryError> {
    // SAFETY: no folder, no items, no inner object: an empty data object.
    let data: IDataObject = unsafe { SHCreateDataObject(None, None, None::<&IDataObject>) }?;
    set_hglobal(&data, CF_HDROP.0, &dropfiles(paths))?;
    Ok(data)
}

/// The file paths in `data`'s `CF_HDROP`.
fn hdrop_paths(data: &IDataObject) -> Result<Vec<PathBuf>, JerryError> {
    // SAFETY: a plain COM call; we release the medium below.
    let mut medium = unsafe { data.GetData(&hglobal_format(CF_HDROP.0)) }?;
    // SAFETY: we asked for TYMED_HGLOBAL, so `hGlobal` is the active union field, and an
    // HGLOBAL holding a DROPFILES is exactly what HDROP is.
    let hdrop = HDROP(unsafe { medium.u.hGlobal }.0);
    // SAFETY: `hdrop` stays valid until ReleaseStgMedium; index u32::MAX asks for the count.
    let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        // SAFETY: as above; a None buffer asks for the length without the NUL.
        let len = unsafe { DragQueryFileW(hdrop, index, None) } as usize;
        let mut buffer = vec![0u16; len + 1];
        // SAFETY: the buffer has room for the name and its NUL.
        let copied = unsafe { DragQueryFileW(hdrop, index, Some(&mut buffer)) } as usize;
        paths.push(PathBuf::from(OsString::from_wide(&buffer[..copied])));
    }
    // SAFETY: we own the medium GetData returned and release it once.
    unsafe { ReleaseStgMedium(&mut medium) };
    Ok(paths)
}

/// Reads a registered DWORD format such as "Preferred DropEffect", if `data` has it.
fn dword(data: &IDataObject, name: PCWSTR) -> Option<u32> {
    let format = registered_format(name)?;
    // SAFETY: a plain COM call; we release the medium below.
    let mut medium = unsafe { data.GetData(&hglobal_format(format)) }.ok()?;
    // SAFETY: TYMED_HGLOBAL was requested, so `hGlobal` is the active field.
    let global = unsafe { medium.u.hGlobal };
    // SAFETY: locking a valid HGLOBAL; null means it has no memory.
    let ptr = unsafe { GlobalLock(global) }.cast::<u32>();
    let value = (!ptr.is_null()).then(|| {
        // SAFETY: the format is a DWORD, so the block holds at least four bytes.
        unsafe { ptr.read_unaligned() }
    });
    // SAFETY: balanced with the lock above. It "fails" with NO_ERROR once the lock count
    // reaches zero, which is the normal case, so the result is ignored.
    let _ = unsafe { GlobalUnlock(global) };
    // SAFETY: we own the medium GetData returned and release it once.
    unsafe { ReleaseStgMedium(&mut medium) };
    value
}

/// Writes a registered DWORD format such as "Preferred DropEffect" into `data`.
fn set_dword(data: &IDataObject, name: PCWSTR, value: u32) -> Result<(), JerryError> {
    let Some(format) = registered_format(name) else {
        tracing::error!("RegisterClipboardFormatW failed");
        return Err(JerryError::internal());
    };
    set_hglobal(data, format, &value.to_le_bytes())
}

/// An absolute item ID list we own, freed with `ILFree`.
struct Pidl(*mut ITEMIDLIST);

impl Pidl {
    fn from_path(path: &Path) -> Option<Self> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        let pidl = unsafe { ILCreateFromPathW(PCWSTR(wide.as_ptr())) };
        (!pidl.is_null()).then_some(Self(pidl))
    }

    fn as_ptr(&self) -> *const ITEMIDLIST {
        self.0
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        // SAFETY: ILCreateFromPathW allocated the list, and it's freed only here, once.
        unsafe { ILFree(Some(self.0)) };
    }
}

/// A DROPFILES block with wide `paths`, as `CF_HDROP` holds it.
fn dropfiles(paths: &[PathBuf]) -> Vec<u8> {
    let header = DROPFILES {
        pFiles: size_of::<DROPFILES>() as u32,
        fWide: true.into(),
        ..Default::default()
    };
    // SAFETY: DROPFILES is plain old data; we copy its bytes.
    let mut bytes = unsafe {
        std::slice::from_raw_parts((&raw const header).cast::<u8>(), size_of::<DROPFILES>())
    }
    .to_vec();
    for path in paths {
        for unit in path.as_os_str().encode_wide().chain(Some(0)) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

/// Copies `bytes` into a new HGLOBAL and hands it to `data` as `format`.
fn set_hglobal(data: &IDataObject, format: u16, bytes: &[u8]) -> Result<(), JerryError> {
    let global = global_copy(bytes)?;
    let medium = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 { hGlobal: global },
        pUnkForRelease: ManuallyDrop::new(None),
    };
    // SAFETY: valid FORMATETC and STGMEDIUM; with fRelease the object owns the block on success.
    let result = unsafe { data.SetData(&hglobal_format(format), &medium, true) };
    if result.is_err() {
        free(global);
    }
    Ok(result?)
}

/// `bytes` in movable global memory, as the clipboard and data objects take it.
fn global_copy(bytes: &[u8]) -> Result<HGLOBAL, JerryError> {
    // SAFETY: a plain allocation; freed below on failure, else owned by whoever takes it.
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }?;
    // SAFETY: `memory` was just allocated and is locked once, here.
    let target = unsafe { GlobalLock(memory) }.cast::<u8>();
    if target.is_null() {
        free(memory);
        return Err(JerryError::internal());
    }
    // SAFETY: `target` points to `bytes.len()` writable bytes, which `bytes` fills exactly; the
    // two don't overlap.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len()) };
    // SAFETY: unlocks the lock taken above. It "fails" with no error once the count is 0.
    let _ = unsafe { GlobalUnlock(memory) };
    Ok(memory)
}

/// `units` as the little-endian bytes Windows keeps them in.
fn bytes_of(units: &[u16]) -> &[u8] {
    // SAFETY: u16 has no padding and a stricter alignment than u8; the slice covers exactly the
    // same memory, borrowed as long as `units`.
    unsafe { std::slice::from_raw_parts(units.as_ptr().cast::<u8>(), size_of_val(units)) }
}

fn free(memory: HGLOBAL) {
    // SAFETY: `memory` is ours: nobody took it.
    if let Err(error) = unsafe { GlobalFree(Some(memory)) } {
        tracing::warn!(%error, "couldn't free clipboard memory");
    }
}

fn registered_format(name: PCWSTR) -> Option<u16> {
    // SAFETY: `name` is a static NUL-terminated string.
    u16::try_from(unsafe { RegisterClipboardFormatW(name) })
        .ok()
        .filter(|&f| f != 0)
}

fn hglobal_format(format: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        ptd: core::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, DATADIR_GET,
    };
    use windows::Win32::System::DataExchange::GetClipboardFormatNameW;

    use super::*;

    // These tests build data objects but never put them on the clipboard: that is the user's.

    fn sta() {
        // SAFETY: test threads are fresh; making them an STA is what the Shell expects.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    }

    /// A temp folder, and its path by its long names. Where `TEMP` is an 8.3 short path (GitHub's
    /// runner: `C:\Users\RUNNER~1\…`), the Shell hands files back by their long names, so paths
    /// that went through it are compared in that form.
    fn temp() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let long = jerry_core::path::plain_path(&fs::canonicalize(dir.path()).unwrap());
        (dir, long)
    }

    fn files(dir: &Path, names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|name| {
                let path = dir.join(name);
                fs::write(&path, b"jerry").unwrap();
                path
            })
            .collect()
    }

    /// A file whose full path is longer than MAX_PATH, written through `\\?\`. Returns the plain
    /// path (what `CF_HDROP` carries).
    fn long_file(dir: &Path) -> PathBuf {
        let mut path = dir.to_path_buf();
        while path.as_os_str().len() < 280 {
            path.push("ძალიან გრძელი საქაღალდის სახელი");
        }
        let verbatim = PathBuf::from(format!(r"\\?\{}", path.display()));
        fs::create_dir_all(&verbatim).unwrap();
        fs::write(verbatim.join("ნახაზი.dwg"), b"AC1032").unwrap();
        path.join("ნახაზი.dwg")
    }

    /// The names of the formats `data` offers, in its order.
    fn format_names(data: &IDataObject) -> Vec<String> {
        // SAFETY: a plain COM call on a live object.
        let formats = unsafe { data.EnumFormatEtc(DATADIR_GET.0 as u32) }.unwrap();
        let mut names = Vec::new();
        loop {
            let mut format = [FORMATETC::default()];
            let mut fetched = 0;
            // SAFETY: a one-element buffer and a valid count pointer.
            let hr = unsafe { formats.Next(&mut format, Some(&mut fetched)) };
            if hr != S_OK || fetched == 0 {
                break;
            }
            let mut name = [0u16; 128];
            // SAFETY: a writable buffer; predefined formats have no name and return 0.
            let len = unsafe { GetClipboardFormatNameW(u32::from(format[0].cfFormat), &mut name) };
            names.push(match (len, format[0].cfFormat) {
                (0, 15) => "CF_HDROP".to_string(),
                (0, other) => format!("CF_{other}"),
                (len, _) => String::from_utf16_lossy(&name[..len as usize]),
            });
            if !format[0].ptd.is_null() {
                // SAFETY: the enumerator allocated `ptd` with the COM allocator and handed it over.
                unsafe { CoTaskMemFree(Some(format[0].ptd.cast_const().cast())) };
            }
        }
        names
    }

    fn has(formats: &[String], name: &str) -> bool {
        formats.iter().any(|format| format == name)
    }

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
            // Without the `\\?\` prefix: other programs don't understand it.
            assert_eq!(hdrop_paths(&data).unwrap(), paths);
        }
    }

    /// Spike #15: the Shell parses plain paths longer than MAX_PATH (even with
    /// LongPathsEnabled = 0), but not `\\?\` ones, so the prefix comes off first.
    #[test]
    fn long_paths_get_the_shell_object() {
        sta();
        let (_dir, dir) = temp();
        let long = long_file(&dir);
        let short = files(&dir, &["მოკლე.txt"]).remove(0);
        let verbatim = PathBuf::from(format!(r"\\?\{}", long.display()));

        assert!(shell_data_object(std::slice::from_ref(&verbatim)).is_err());

        let data = files_data_object(&[short.clone(), verbatim]).unwrap();
        let formats = format_names(&data);
        assert!(has(&formats, "Shell IDList Array"), "{formats:?}");
        assert_eq!(hdrop_paths(&data).unwrap(), [short, long]);
    }

    /// The fallback for files the Shell can't parse: `CF_HDROP` alone, and the effect.
    #[test]
    fn hdrop_only_object_keeps_paths_and_effect() {
        sta();
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![long_file(dir.path()), dir.path().join("არ არსებობს.txt")];

        let data = hdrop_data_object(&paths).unwrap();
        set_dword(&data, CFSTR_PREFERREDDROPEFFECT, Op::Cut.preferred_effect()).unwrap();

        let formats = format_names(&data);
        assert!(has(&formats, "CF_HDROP"), "{formats:?}");
        assert!(has(&formats, "Preferred DropEffect"), "{formats:?}");
        assert_eq!(hdrop_paths(&data).unwrap(), paths);
        assert_eq!(dword(&data, CFSTR_PREFERREDDROPEFFECT), Some(2));
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

    #[test]
    fn a_missing_file_is_not_found_with_its_path() {
        sta();
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone.txt");
        let error = files_data_object(std::slice::from_ref(&gone)).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(error.path.as_deref(), Some(gone.as_path()));
        assert_eq!(
            files_data_object(&[]).unwrap_err().code,
            ErrorCode::NotFound
        );
    }
}
