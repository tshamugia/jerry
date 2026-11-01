//! The Windows clipboard, used the way Explorer uses it (spike #15).
//!
//! Copy and Cut put the Shell's own data object on the clipboard: the one the files' parent
//! folder makes for Explorer (`BHID_DataObject`, spike #13). Paste targets (Explorer, the
//! desktop, Outlook, Telegram) therefore get what Explorer's Copy gives them: Shell IDList Array,
//! CF_HDROP, FileNameW, FileContents and the rest, plus "Preferred DropEffect", which tells Copy
//! (COPY | LINK, as Explorer writes it) from Cut (MOVE).
//!
//! Paste reads CF_HDROP and "Preferred DropEffect" from whatever is on the clipboard. After a
//! cut is pasted, the target tells the source with "Paste Succeeded" (the delete-on-paste
//! protocol) and empties the clipboard, as Explorer does. Jerry moves the files itself, so it
//! never asks the source to delete anything, and as a source it never deletes on what a target
//! writes back.
//!
//! Without `OleFlushClipboard` at exit the clipboard is empty once Jerry is gone ([`flush`]).
//!
//! OLE clipboard calls belong on one OLE-initialized STA thread that pumps messages: Jerry's
//! main thread. Call [`init_ole`] there once at startup and everything else only there.

use std::ffi::OsString;
use std::mem::ManuallyDrop;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Once;

use windows::Win32::Foundation::{GlobalFree, HGLOBAL, HWND, LPARAM, LRESULT, S_OK, WPARAM};
pub use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Com::{
    CoTaskMemFree, DATADIR_GET, DVASPECT_CONTENT, FORMATETC, IBindCtx, STGMEDIUM, STGMEDIUM_0,
    TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, GetClipboardFormatNameW, GetClipboardOwner,
    GetClipboardSequenceNumber, RegisterClipboardFormatW, RemoveClipboardFormatListener,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, OleFlushClipboard,
    OleGetClipboard, OleInitialize, OleSetClipboard, ReleaseStgMedium,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, CFSTR_LOGICALPERFORMEDDROPEFFECT, CFSTR_PASTESUCCEEDED,
    CFSTR_PERFORMEDDROPEFFECT, CFSTR_PREFERREDDROPEFFECT, DROPFILES, DragQueryFileW, HDROP,
    ILCreateFromPathW, ILFree, SHCreateDataObject, SHCreateShellItemArrayFromIDLists,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW,
    GetWindowThreadProcessId, HWND_MESSAGE, RegisterClassW, SetWindowLongPtrW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLIPBOARDUPDATE, WNDCLASSW,
};
use windows::core::{HRESULT, Interface, PCWSTR, w};

/// Why the clipboard couldn't take the files.
#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("nothing to put on the clipboard")]
    Empty,
    /// The Shell can't parse the path: the file is gone, or the path isn't one the Shell
    /// namespace understands.
    #[error("the Shell can't resolve {}", path.display())]
    Path { path: PathBuf },
    #[error(transparent)]
    Os(#[from] windows::core::Error),
}

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

    /// A cut is MOVE without COPY. Anything else, a missing format included, is a copy: apps
    /// that put only CF_HDROP on the clipboard mean Copy.
    fn from_preferred_effect(effect: Option<u32>) -> Self {
        match effect {
            Some(e) if e & DROPEFFECT_MOVE.0 != 0 && e & DROPEFFECT_COPY.0 == 0 => Self::Cut,
            _ => Self::Copy,
        }
    }
}

/// Files on the clipboard, as a paste target sees them.
pub struct ClipboardFiles {
    /// CF_HDROP's paths, exactly as the source wrote them.
    pub paths: Vec<PathBuf>,
    pub op: Op,
    /// The raw "Preferred DropEffect", if the source wrote one (diagnostics).
    pub preferred_effect: Option<u32>,
    /// The clipboard's data object, to report a pasted cut back ([`finish_cut`]).
    pub data: IDataObject,
    /// The clipboard's sequence number when it was read, to tell whether it changed since.
    sequence: u32,
}

/// What a paste target wrote back into a data object Jerry put on the clipboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PasteFeedback {
    /// "Paste Succeeded": the target finished pasting (MOVE after a cut).
    pub paste_succeeded: Option<u32>,
    /// "Performed DropEffect": MOVE asks the source to delete the files (unoptimized move).
    pub performed: Option<u32>,
    /// "Logical Performed DropEffect": what the user asked for, whoever did the work.
    pub logical_performed: Option<u32>,
}

/// Initializes OLE on the calling thread, which must be Jerry's main (STA) thread.
///
/// wry already made the thread an STA with `CoInitializeEx`, but the OLE clipboard and drag &
/// drop need `OleInitialize`, which tao calls only when its own drop handler is on, and Jerry
/// turns that off (`dragDropEnabled: false`). Never uninitialized: OLE lives as long as the
/// process.
pub fn init_ole() -> windows::core::Result<()> {
    // SAFETY: no reserved pointer; a second call on the same thread just returns S_FALSE.
    unsafe { OleInitialize(None) }
}

/// Puts `paths` on the clipboard for Copy or Cut, as Explorer does, and returns the data object.
///
/// Keep it: [`is_current`] tells whether the clipboard still holds it, [`feedback`] how a paste
/// target used it, and [`flush`] keeps it on the clipboard after Jerry exits.
pub fn set_files(paths: &[PathBuf], op: Op) -> Result<IDataObject, ClipboardError> {
    let data = files_data_object(paths)?;
    put(&data, op)?;
    Ok(data)
}

/// The second half of [`set_files`]: marks `data` (from [`files_data_object`]) as Copy or Cut
/// and puts it on the clipboard.
pub fn put(data: &IDataObject, op: Op) -> windows::core::Result<()> {
    set_dword(data, CFSTR_PREFERREDDROPEFFECT, op.preferred_effect())?;
    // SAFETY: a live data object; OLE takes its own reference.
    unsafe { OleSetClipboard(data) }
}

/// The formats on the clipboard now, whoever put them there (diagnostics).
pub fn current_formats() -> windows::core::Result<Vec<String>> {
    // SAFETY: plain call; the clipboard's object comes back with a reference we own.
    format_names(&unsafe { OleGetClipboard() }?)
}

/// The data object for `paths`: the Shell's own, or CF_HDROP alone for existing files the Shell
/// can't parse. Core's `\\?\` paths go to the Shell and into CF_HDROP without the prefix, which
/// no other app understands; the Shell parses plain paths longer than MAX_PATH fine.
pub fn files_data_object(paths: &[PathBuf]) -> Result<IDataObject, ClipboardError> {
    let plain: Vec<PathBuf> = paths
        .iter()
        .map(|path| without_verbatim_prefix(path))
        .collect();
    match shell_data_object(&plain) {
        Err(ClipboardError::Path { path }) if path.exists() => {
            tracing::warn!(path = %path.display(), "the Shell can't parse it; CF_HDROP only");
            hdrop_data_object(&plain)
        }
        result => result,
    }
}

/// The Shell data object for `paths`, the same one Explorer copies and drags: the items' parent
/// folder makes it (`IShellItemArray::BindToHandler(BHID_DataObject)` ends in its
/// `GetUIObjectOf`). `SHCreateDataObject` with absolute PIDLs offers nothing (spike #13).
pub fn shell_data_object(paths: &[PathBuf]) -> Result<IDataObject, ClipboardError> {
    if paths.is_empty() {
        return Err(ClipboardError::Empty);
    }
    let pidls = paths
        .iter()
        .map(|path| {
            Pidl::from_path(path).ok_or_else(|| ClipboardError::Path { path: path.clone() })
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

/// A data object holding only CF_HDROP with `paths`: the Shell's generic object, which keeps
/// any format set on it.
pub fn hdrop_data_object(paths: &[PathBuf]) -> Result<IDataObject, ClipboardError> {
    if paths.is_empty() {
        return Err(ClipboardError::Empty);
    }
    // SAFETY: no folder, no items, no inner object: an empty data object.
    let data: IDataObject = unsafe { SHCreateDataObject(None, None, None::<&IDataObject>) }?;
    set_hglobal(&data, CF_HDROP.0, &dropfiles(paths))?;
    Ok(data)
}

/// Reads the files on the clipboard. `None` when it holds no CF_HDROP (text, an image, nothing).
pub fn get_files() -> windows::core::Result<Option<ClipboardFiles>> {
    // SAFETY: no arguments.
    let sequence = unsafe { GetClipboardSequenceNumber() };
    // SAFETY: plain call; the clipboard's object comes back with a reference we own.
    let data = unsafe { OleGetClipboard() }?;
    // SAFETY: a plain COM call with a valid FORMATETC.
    if unsafe { data.QueryGetData(&hglobal_format(CF_HDROP.0)) } != S_OK {
        return Ok(None);
    }
    let paths = hdrop_paths(&data)?;
    let preferred_effect = dword(&data, CFSTR_PREFERREDDROPEFFECT);
    Ok(Some(ClipboardFiles {
        paths,
        op: Op::from_preferred_effect(preferred_effect),
        preferred_effect,
        data,
        sequence,
    }))
}

/// Call once Jerry has moved the files of a pasted cut. Tells the source ("Paste Succeeded" =
/// MOVE without "Performed DropEffect": the files are moved, delete nothing; MSDN, "Handling
/// Shell Data Transfer Scenarios", delete-on-paste), then empties the clipboard if it still
/// holds that cut, as Explorer does after pasting one, so a second Paste can't try to move the
/// same files again. Explorer as the source doesn't empty it itself (spike #15).
pub fn finish_cut(files: &ClipboardFiles) -> windows::core::Result<()> {
    if let Err(error) = set_dword(&files.data, CFSTR_PASTESUCCEEDED, DROPEFFECT_MOVE.0) {
        // The source may be gone (it exited and flushed): nobody left to tell.
        tracing::debug!(%error, "couldn't report the paste to the source");
    }
    // SAFETY: no arguments.
    if unsafe { GetClipboardSequenceNumber() } == files.sequence {
        // SAFETY: a null object empties the clipboard.
        unsafe { OleSetClipboard(None) }?;
    }
    Ok(())
}

/// What a paste target wrote back into `data` (one of ours) so far.
pub fn feedback(data: &IDataObject) -> PasteFeedback {
    PasteFeedback {
        paste_succeeded: dword(data, CFSTR_PASTESUCCEEDED),
        performed: dword(data, CFSTR_PERFORMEDDROPEFFECT),
        logical_performed: dword(data, CFSTR_LOGICALPERFORMEDDROPEFFECT),
    }
}

/// The clipboard still holds `data`: nobody copied anything since.
pub fn is_current(data: &IDataObject) -> bool {
    // The `windows` wrapper folds S_FALSE ("not current") into Ok, so call the export directly.
    windows::core::link!("ole32.dll" "system" fn OleIsCurrentClipboard(
        pdataobj: *mut core::ffi::c_void,
    ) -> HRESULT);
    // SAFETY: a live data object borrowed for the call.
    let hr = unsafe { OleIsCurrentClipboard(data.as_raw()) };
    hr == S_OK
}

/// If the clipboard still holds `data`, renders it into the clipboard (`OleFlushClipboard`), so
/// the files can still be pasted after Jerry exits. Returns whether it did. Call at exit, on the
/// thread that called [`set_files`].
pub fn flush(data: &IDataObject) -> windows::core::Result<bool> {
    if !is_current(data) {
        return Ok(false);
    }
    // SAFETY: plain call on the thread that owns the clipboard object.
    unsafe { OleFlushClipboard() }?;
    Ok(true)
}

/// Empties the clipboard if it still holds `data`.
pub fn clear(data: &IDataObject) -> windows::core::Result<()> {
    if is_current(data) {
        // SAFETY: a null object empties the clipboard.
        unsafe { OleSetClipboard(None) }?;
    }
    Ok(())
}

/// The process that owns the clipboard (the window that last set it), if any (diagnostics).
pub fn owner_process() -> Option<u32> {
    // SAFETY: no arguments.
    let owner = unsafe { GetClipboardOwner() }.ok()?;
    let mut process_id = 0;
    // SAFETY: a window handle (possibly stale, which only fails the call) and an out pointer.
    unsafe { GetWindowThreadProcessId(owner, Some(&mut process_id)) };
    (process_id != 0).then_some(process_id)
}

/// Jerry's own process id, to compare with [`owner_process`].
pub fn current_process() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetCurrentProcessId() }
}

/// The names of the clipboard formats `data` offers, in its order (diagnostics).
pub fn format_names(data: &IDataObject) -> windows::core::Result<Vec<String>> {
    // SAFETY: a plain COM call on a live object.
    let formats = unsafe { data.EnumFormatEtc(DATADIR_GET.0 as u32) }?;
    let mut names = Vec::new();
    loop {
        let mut format = [FORMATETC::default()];
        let mut fetched = 0;
        // SAFETY: a one-element buffer and a valid count pointer.
        let hr = unsafe { formats.Next(&mut format, Some(&mut fetched)) };
        if hr != S_OK || fetched == 0 {
            break;
        }
        names.push(format_name(format[0].cfFormat));
        if !format[0].ptd.is_null() {
            // SAFETY: the enumerator allocated `ptd` with the COM allocator and handed it to us.
            unsafe { CoTaskMemFree(Some(format[0].ptd.cast_const().cast())) };
        }
    }
    Ok(names)
}

/// The file paths in `data`'s CF_HDROP.
pub fn hdrop_paths(data: &IDataObject) -> windows::core::Result<Vec<PathBuf>> {
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
pub fn dword(data: &IDataObject, name: PCWSTR) -> Option<u32> {
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
pub fn set_dword(data: &IDataObject, name: PCWSTR, value: u32) -> windows::core::Result<()> {
    let format = registered_format(name).ok_or_else(windows::core::Error::from_thread)?;
    set_hglobal(data, format, &value.to_le_bytes())
}

/// Calls a function on every clipboard change (`WM_CLIPBOARDUPDATE`), Jerry's own changes
/// included. Lives on the thread that created it, which must pump messages (the main thread);
/// stops when dropped.
pub struct ClipboardListener {
    hwnd: HWND,
    /// Owned through the window's GWLP_USERDATA; freed in Drop after the window is gone.
    callback: *mut Callback,
}

type Callback = Box<dyn Fn()>;

impl ClipboardListener {
    /// Starts listening with a message-only window. `on_change` runs inside the window
    /// procedure: keep it short, and don't touch the clipboard's content from it.
    pub fn new(on_change: impl Fn() + 'static) -> windows::core::Result<Self> {
        let class = w!("JerryClipboardListener");
        // SAFETY: a null name asks for the executable's module, which outlives the window.
        let instance = unsafe { GetModuleHandleW(None) }?;
        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(listener_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            };
            // SAFETY: a fully initialized class with a static name; failure shows up as a
            // CreateWindowExW error below.
            unsafe { RegisterClassW(&wc) };
        });
        // SAFETY: the class is registered above; HWND_MESSAGE makes a message-only window.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!("Jerry clipboard"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance.into()),
                None,
            )
        }?;
        let callback: *mut Callback = Box::into_raw(Box::new(Box::new(on_change)));
        // SAFETY: our own window; the pointer stays valid until Drop resets it and frees it.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, callback as isize) };
        let listener = Self { hwnd, callback };
        // SAFETY: our own window, on its own thread.
        unsafe { AddClipboardFormatListener(hwnd) }?;
        Ok(listener)
    }
}

impl Drop for ClipboardListener {
    fn drop(&mut self) {
        // SAFETY: our own window; a failure only means it's already gone.
        unsafe {
            let _ = RemoveClipboardFormatListener(self.hwnd);
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
        }
        // SAFETY: made by Box::into_raw in `new`, no longer reachable from the window, freed once.
        drop(unsafe { Box::from_raw(self.callback) });
    }
}

unsafe extern "system" fn listener_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CLIPBOARDUPDATE {
        // SAFETY: our window's user data is null or a live Callback (see ClipboardListener).
        let callback = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Callback;
        if !callback.is_null() {
            // A panic must not unwind into Windows; log it and keep listening.
            // SAFETY: as above.
            let callback = unsafe { &*callback };
            if catch_unwind(AssertUnwindSafe(callback)).is_err() {
                tracing::error!("clipboard listener callback panicked");
            }
        }
        return LRESULT(0);
    }
    // SAFETY: default handling for everything else.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
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

/// `\\?\C:\x` → `C:\x`, `\\?\UNC\server\share\x` → `\\server\share\x`; anything else unchanged.
fn without_verbatim_prefix(path: &Path) -> PathBuf {
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let starts = |prefix: &str| {
        let prefix: Vec<u16> = prefix.encode_utf16().collect();
        wide.starts_with(&prefix)
    };
    let rest = if starts(r"\\?\UNC\") {
        [r"\\".encode_utf16().collect(), wide[8..].to_vec()].concat()
    } else if starts(r"\\?\") && wide.get(5) == Some(&u16::from(b':')) {
        wide[4..].to_vec()
    } else {
        return path.to_path_buf();
    };
    PathBuf::from(OsString::from_wide(&rest))
}

/// A DROPFILES block with wide `paths`, as CF_HDROP holds it.
fn dropfiles(paths: &[PathBuf]) -> Vec<u8> {
    let header = DROPFILES {
        pFiles: size_of::<DROPFILES>() as u32,
        fWide: true.into(),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    // SAFETY: DROPFILES is plain old data; we copy its bytes.
    bytes.extend_from_slice(unsafe {
        std::slice::from_raw_parts((&raw const header).cast::<u8>(), size_of::<DROPFILES>())
    });
    for path in paths {
        for unit in path.as_os_str().encode_wide().chain(Some(0)) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

/// Copies `bytes` into a new HGLOBAL and hands it to `data` as `format`.
fn set_hglobal(data: &IDataObject, format: u16, bytes: &[u8]) -> windows::core::Result<()> {
    // SAFETY: a plain allocation; freed below on failure, owned by `data` on success.
    let global = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }?;
    // SAFETY: locking the block we just allocated.
    let ptr = unsafe { GlobalLock(global) }.cast::<u8>();
    if ptr.is_null() {
        free(global);
        return Err(windows::core::Error::from_thread());
    }
    // SAFETY: the block is at least `bytes.len()` long and doesn't overlap `bytes`.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
    // SAFETY: balanced with the lock above; the NO_ERROR "failure" is the normal case.
    let _ = unsafe { GlobalUnlock(global) };
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
    result
}

fn free(global: HGLOBAL) {
    // SAFETY: a block we allocated and nobody else owns.
    let _ = unsafe { GlobalFree(Some(global)) };
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

fn format_name(format: u16) -> String {
    let mut name = [0u16; 128];
    // SAFETY: a writable buffer; predefined formats have no name and return 0.
    let len = unsafe { GetClipboardFormatNameW(u32::from(format), &mut name) };
    if len > 0 {
        return String::from_utf16_lossy(&name[..len as usize]);
    }
    match format {
        1 => "CF_TEXT".into(),
        2 => "CF_BITMAP".into(),
        7 => "CF_OEMTEXT".into(),
        8 => "CF_DIB".into(),
        13 => "CF_UNICODETEXT".into(),
        15 => "CF_HDROP".into(),
        16 => "CF_LOCALE".into(),
        17 => "CF_DIBV5".into(),
        other => format!("CF_{other}"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

    use super::*;

    fn sta() {
        // SAFETY: test threads are fresh; making them an STA is what the Shell expects.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
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

    /// A file whose full path is longer than MAX_PATH, written through `\\?\`. Returns the
    /// plain path (what CF_HDROP carries).
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

    #[test]
    fn copy_and_cut_objects_carry_explorers_formats_and_the_effect() {
        sta();
        let dir = tempfile::tempdir().unwrap();
        let paths = files(dir.path(), &["ნახაზი — სართული 1.dwg", "ხარჯთაღრიცხვა.pdf"]);

        for (op, effect) in [(Op::Copy, 5), (Op::Cut, 2)] {
            let data = files_data_object(&paths).unwrap();
            set_dword(&data, CFSTR_PREFERREDDROPEFFECT, op.preferred_effect()).unwrap();
            let formats = format_names(&data).unwrap();
            println!("{op:?}: {formats:?}");
            // OleFlushClipboard renders only what EnumFormatEtc lists, so the effect must be
            // listed too, or a cut would turn into a copy once Jerry exits.
            for expected in [
                "Shell IDList Array",
                "CF_HDROP",
                "FileNameW",
                "Preferred DropEffect",
            ] {
                assert!(
                    formats.iter().any(|f| f == expected),
                    "{expected}: {formats:?}"
                );
            }
            assert_eq!(dword(&data, CFSTR_PREFERREDDROPEFFECT), Some(effect));
            assert_eq!(Op::from_preferred_effect(Some(effect)), op);
            assert_eq!(hdrop_paths(&data).unwrap(), paths);
        }
    }

    /// Spike #15: the Shell parses plain paths longer than MAX_PATH (even with
    /// LongPathsEnabled = 0), but not `\\?\` ones, so the prefix comes off first.
    #[test]
    fn long_paths_get_the_shell_object_without_the_verbatim_prefix() {
        sta();
        let dir = tempfile::tempdir().unwrap();
        let long = long_file(dir.path());
        let short = files(dir.path(), &["მოკლე.txt"]).remove(0);
        let verbatim = PathBuf::from(format!(r"\\?\{}", long.display()));

        let raw = shell_data_object(std::slice::from_ref(&verbatim));
        println!(
            "Shell object for the \\\\?\\ path: {:?}",
            raw.as_ref().map(|_| ())
        );
        assert!(matches!(raw, Err(ClipboardError::Path { .. })));

        let data = files_data_object(&[short.clone(), verbatim]).unwrap();
        let formats = format_names(&data).unwrap();
        println!("{}-char path: {formats:?}", long.as_os_str().len());
        assert!(
            formats.iter().any(|f| f == "Shell IDList Array"),
            "{formats:?}"
        );
        assert_eq!(hdrop_paths(&data).unwrap(), [short, long]);
    }

    /// The fallback for files the Shell can't parse: CF_HDROP alone, and the effect.
    #[test]
    fn hdrop_only_object_keeps_paths_and_effect() {
        sta();
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![long_file(dir.path()), dir.path().join("არ არსებობს.txt")];

        let data = hdrop_data_object(&paths).unwrap();
        set_dword(&data, CFSTR_PREFERREDDROPEFFECT, Op::Cut.preferred_effect()).unwrap();

        let formats = format_names(&data).unwrap();
        println!("CF_HDROP only: {formats:?}");
        assert!(formats.iter().any(|f| f == "CF_HDROP"), "{formats:?}");
        assert!(
            formats.iter().any(|f| f == "Preferred DropEffect"),
            "{formats:?}"
        );
        assert_eq!(hdrop_paths(&data).unwrap(), paths);
        assert_eq!(dword(&data, CFSTR_PREFERREDDROPEFFECT), Some(2));
    }

    #[test]
    fn verbatim_prefix_comes_off() {
        let cases = [
            (r"\\?\C:\ა\ბ.txt", r"C:\ა\ბ.txt"),
            (r"\\?\UNC\server\share\ა.txt", r"\\server\share\ა.txt"),
            (r"C:\ა.txt", r"C:\ა.txt"),
            (r"\\server\share\ა.txt", r"\\server\share\ა.txt"),
            (r"\\?\Volume{1234}\ა.txt", r"\\?\Volume{1234}\ა.txt"),
        ];
        for (path, expected) in cases {
            assert_eq!(
                without_verbatim_prefix(Path::new(path)),
                Path::new(expected)
            );
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

    #[test]
    fn missing_file_is_a_path_error() {
        sta();
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone.txt");
        assert!(matches!(
            files_data_object(&[gone]),
            Err(ClipboardError::Path { .. })
        ));
        assert!(matches!(files_data_object(&[]), Err(ClipboardError::Empty)));
    }
}
