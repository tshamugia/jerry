//! OLE drag & drop out of Jerry (spike #13).
//!
//! Files leave Jerry as the Shell's own data object: the one the items' parent folder hands
//! Explorer for its drags. Every target (Explorer, the desktop, Outlook, browsers, Telegram,
//! AutoCAD) therefore gets exactly what a drag from Explorer gives it: CF_HDROP, Shell IDList
//! Array, FileNameW and the rest. The drag loop, drag image and count badge are Windows' own
//! (`SHDoDragDrop` with its default drop source).
//!
//! OLE drag & drop needs an OLE-initialized STA thread that pumps messages. In Jerry that is the
//! main thread: call [`init_ole`] there once at startup, and [`drag_files`] only there.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{DRAGDROP_S_DROP, HWND, LPARAM, LRESULT, S_OK, WPARAM};
use windows::Win32::System::Com::{
    CoTaskMemFree, DATADIR_GET, DVASPECT_CONTENT, FORMATETC, IBindCtx, IDataObject, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::{GetClipboardFormatNameW, RegisterClipboardFormatW};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE,
    OleInitialize, ReleaseStgMedium,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetCapture, GetKeyState, VK_LBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, CFSTR_LOGICALPERFORMEDDROPEFFECT, CFSTR_PERFORMEDDROPEFFECT, DragQueryFileW,
    HDROP, ILCreateFromPathW, ILFree, SHCreateShellItemArrayFromIDLists,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClassNameW, GetSystemMetrics, GetWindowThreadProcessId,
    HWND_MESSAGE, PostMessageW, RegisterClassW, SM_SWAPBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_APP, WNDCLASSW,
};
use windows::core::{HRESULT, Interface, PCWSTR};

/// Why a drag couldn't start.
#[derive(Debug, thiserror::Error)]
pub enum DragError {
    #[error("nothing to drag")]
    Empty,
    /// The Shell can't parse the path: the file is gone, or the path isn't one the Shell
    /// namespace understands (for example a `\\?\` long path).
    #[error("the Shell can't resolve {}", path.display())]
    Path { path: PathBuf },
    #[error(transparent)]
    Os(#[from] windows::core::Error),
}

/// What a drop target did with the files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropEffect {
    None,
    Copy,
    Move,
    Link,
}

impl DropEffect {
    fn from_raw(effect: DROPEFFECT) -> Self {
        // A target reports one effect; MOVE wins if it ever reports several.
        if effect.0 & DROPEFFECT_MOVE.0 != 0 {
            Self::Move
        } else if effect.0 & DROPEFFECT_COPY.0 != 0 {
            Self::Copy
        } else if effect.0 & DROPEFFECT_LINK.0 != 0 {
            Self::Link
        } else {
            Self::None
        }
    }
}

/// How a drag ended.
#[derive(Clone, Copy, Debug)]
pub struct DragOutcome {
    /// The drag ended with a drop (`DRAGDROP_S_DROP`), not with Esc or over nothing that
    /// accepted it.
    pub dropped: bool,
    /// The effect the target returned from `IDropTarget::Drop`.
    pub effect: DropEffect,
    /// "Performed DropEffect" the target wrote back into the data object, if any. After an
    /// optimized move (the target moved the files itself) it is `None`, after an unoptimized
    /// move (the target copied, the source should delete) it is `Move`.
    pub performed: Option<DropEffect>,
    /// "Logical Performed DropEffect": what the user asked for, whoever did the work.
    pub logical_performed: Option<DropEffect>,
}

/// Initializes OLE on the calling thread, which must be Jerry's main (STA) thread.
///
/// wry already made the thread an STA with `CoInitializeEx`, but OLE drag & drop and the
/// clipboard need `OleInitialize`, which tao calls only when its own drop handler is on, and
/// Jerry turns that off (`dragDropEnabled: false`). Never uninitialized: OLE lives as long as
/// the process.
pub fn init_ole() -> windows::core::Result<()> {
    // SAFETY: no reserved pointer; a second call on the same thread just returns S_FALSE.
    unsafe { OleInitialize(None) }?;
    create_task_window()
}

/// A task for [`run_on_main`].
pub type Task = Box<dyn FnOnce() + Send>;

/// The message-only window that runs [`run_on_main`]'s tasks (an HWND, kept as an integer so
/// it can be shared with other threads).
static TASKS: OnceLock<isize> = OnceLock::new();
const WM_RUN_TASK: u32 = WM_APP + 0x14;

fn create_task_window() -> windows::core::Result<()> {
    if TASKS.get().is_some() {
        return Ok(());
    }
    let class = windows::core::w!("JerryMainTasks");
    // SAFETY: the module handle of this process.
    let instance = unsafe { GetModuleHandleW(None) }?;
    let info = WNDCLASSW {
        lpfnWndProc: Some(task_proc),
        hInstance: instance.into(),
        lpszClassName: class,
        ..Default::default()
    };
    // SAFETY: a valid class description.
    unsafe { RegisterClassW(&info) };
    // SAFETY: a message-only window on this (the main) thread.
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            windows::core::w!(""),
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
    let _ = TASKS.set(hwnd.0 as isize);
    Ok(())
}

unsafe extern "system" fn task_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_RUN_TASK {
        // SAFETY: `run_on_main` posted a pointer from `Box::into_raw`, and each message is
        // delivered once, so the box is reclaimed exactly once.
        let task = unsafe { Box::from_raw(lparam.0 as *mut Task) };
        // A panicking task fails alone (CLAUDE.md: panics don't kill the app).
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
        return LRESULT(0);
    }
    // SAFETY: default handling for our own window.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Runs `task` on the main thread, from a message to Jerry's own window. Use it for anything
/// that runs a modal loop (a drag, Windows' context menu) instead of Tauri's
/// `run_on_main_thread`: that one runs the task inside tao's event callback, and tao holds back
/// its own events while a callback runs, so Rust → page events (`emit`, which evaluates script
/// through tao) would wait until the modal loop ends (spike #14). From here they don't.
///
/// Gives the task back if [`init_ole`] hasn't run or the post fails.
pub fn run_on_main(task: Task) -> Result<(), Task> {
    let Some(&hwnd) = TASKS.get() else {
        return Err(task);
    };
    let raw = Box::into_raw(Box::new(task));
    // SAFETY: posting to our own window; on success its procedure takes ownership of `raw`.
    let posted = unsafe {
        PostMessageW(
            Some(HWND(hwnd as *mut core::ffi::c_void)),
            WM_RUN_TASK,
            WPARAM(0),
            LPARAM(raw as isize),
        )
    };
    match posted {
        Ok(()) => Ok(()),
        // SAFETY: the message wasn't posted, so the box is still ours.
        Err(_) => Err(*unsafe { Box::from_raw(raw) }),
    }
}

/// Drags `paths` out of Jerry and returns once the user drops or cancels.
///
/// Must run on the main thread (see [`init_ole`]) while a mouse button is down, and not from
/// inside a WebView2 callback: `SHDoDragDrop` runs a modal message loop until the drop. `owner`
/// is the source window; Windows asks it for a drag image and, when it has none, draws its own
/// (icons or thumbnails with the count badge).
pub fn drag_files(owner: isize, paths: &[PathBuf]) -> Result<DragOutcome, DragError> {
    let data = data_object(paths)?;
    drag(owner, &data)
}

/// The Shell data object for `paths`, the same one Explorer drags and copies: the items'
/// parent folder makes it (`IShellItemArray::BindToHandler(BHID_DataObject)` ends in its
/// `GetUIObjectOf`), so CF_HDROP, Shell IDList Array, FileNameW and the rest are all there.
pub fn data_object(paths: &[PathBuf]) -> Result<IDataObject, DragError> {
    if paths.is_empty() {
        return Err(DragError::Empty);
    }
    let pidls = paths
        .iter()
        .map(|path| Pidl::from_path(path).ok_or_else(|| DragError::Path { path: path.clone() }))
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

/// A drag out of Jerry is running (its modal loop is on the main thread). Jerry's own drop
/// target reads it to tell an internal drag from one that comes from another app.
static DRAGGING: AtomicBool = AtomicBool::new(false);

/// Whether a drag started by [`drag`] is running.
pub fn is_dragging() -> bool {
    DRAGGING.load(Ordering::Acquire)
}

/// Clears [`DRAGGING`] however the drag ends.
struct Dragging;

impl Drop for Dragging {
    fn drop(&mut self) {
        DRAGGING.store(false, Ordering::Release);
    }
}

/// Runs the modal drag loop for `data` and reports how it ended.
pub fn drag(owner: isize, data: &IDataObject) -> Result<DragOutcome, DragError> {
    // The `windows` wrapper folds DRAGDROP_S_DROP and DRAGDROP_S_CANCEL into Ok, so call the
    // export directly to tell a drop from a cancel.
    windows::core::link!("shell32.dll" "system" fn SHDoDragDrop(
        hwnd: HWND,
        pdata: *mut core::ffi::c_void,
        pdsrc: *mut core::ffi::c_void,
        dweffect: DROPEFFECT,
        pdweffect: *mut DROPEFFECT,
    ) -> HRESULT);

    let allowed = DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0 | DROPEFFECT_LINK.0);
    let mut effect = DROPEFFECT_NONE;
    DRAGGING.store(true, Ordering::Release);
    let _dragging = Dragging;
    // Show the drop overlay now: the modal drag loop may not run its timer.
    crate::drop::sync_overlay();
    // SAFETY: `data` is a live IDataObject borrowed for the whole call; a null drop source makes
    // the Shell create its default one; `effect` is a valid out pointer.
    let hr = unsafe {
        SHDoDragDrop(
            HWND(owner as *mut core::ffi::c_void),
            data.as_raw(),
            core::ptr::null_mut(),
            allowed,
            &mut effect,
        )
    };
    hr.ok()?;
    Ok(DragOutcome {
        dropped: hr == DRAGDROP_S_DROP,
        effect: DropEffect::from_raw(effect),
        performed: dword_format(data, CFSTR_PERFORMEDDROPEFFECT)
            .map(|effect| DropEffect::from_raw(DROPEFFECT(effect))),
        logical_performed: dword_format(data, CFSTR_LOGICALPERFORMEDDROPEFFECT)
            .map(|effect| DropEffect::from_raw(DROPEFFECT(effect))),
    })
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

/// The file paths in `data`'s CF_HDROP, the format most drop targets read.
pub fn hdrop_paths(data: &IDataObject) -> windows::core::Result<Vec<PathBuf>> {
    let format = hglobal_format(CF_HDROP.0);
    // SAFETY: a plain COM call; we release the medium below.
    let mut medium = unsafe { data.GetData(&format) }?;
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

/// Mouse state as the main thread sees it when a drag is about to start (spike #13).
#[derive(Clone, Debug)]
pub struct InputState {
    /// The logical left / right button is physically down now (`GetAsyncKeyState`).
    pub left_down: bool,
    pub right_down: bool,
    /// The same as the main thread's input queue last saw it (`GetKeyState`). This is what
    /// the drag loop starts from.
    pub left_down_queue: bool,
    pub right_down_queue: bool,
    /// The window holding the mouse capture, if the main thread's input queue knows one.
    pub capture: Option<WindowInfo>,
}

/// A window, for diagnostics.
#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub class: String,
    pub process_id: u32,
    /// The window belongs to Jerry's process (not WebView2's browser process).
    pub ours: bool,
}

/// Reads the mouse buttons and the capture window (diagnostics).
pub fn input_state() -> InputState {
    // SAFETY: plain Win32 queries without pointers.
    let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
    let (left, right) = if swapped {
        (VK_RBUTTON, VK_LBUTTON)
    } else {
        (VK_LBUTTON, VK_RBUTTON)
    };
    // SAFETY: as above.
    let physical = |key: i32| unsafe { GetAsyncKeyState(key) } < 0;
    // SAFETY: as above; GetKeyState already maps swapped buttons.
    let queued = |key: i32| unsafe { GetKeyState(key) } < 0;
    InputState {
        left_down: physical(i32::from(left.0)),
        right_down: physical(i32::from(right.0)),
        left_down_queue: queued(i32::from(VK_LBUTTON.0)),
        right_down_queue: queued(i32::from(VK_RBUTTON.0)),
        capture: window_info(
            // SAFETY: no arguments; returns null when no window has the capture.
            unsafe { GetCapture() },
        ),
    }
}

fn window_info(hwnd: HWND) -> Option<WindowInfo> {
    if hwnd.is_invalid() {
        return None;
    }
    let mut class = [0u16; 256];
    // SAFETY: `hwnd` is a window handle (possibly stale, which only fails the call) and the
    // buffer is writable.
    let len = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
    let mut process_id = 0;
    // SAFETY: as above; `process_id` is a valid out pointer.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    Some(WindowInfo {
        class: String::from_utf16_lossy(&class[..len]),
        process_id,
        // SAFETY: no arguments.
        ours: process_id == unsafe { GetCurrentProcessId() },
    })
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

fn hglobal_format(format: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        ptd: core::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

/// Reads a registered DWORD format such as "Performed DropEffect", if `data` has it.
fn dword_format(data: &IDataObject, name: PCWSTR) -> Option<u32> {
    // SAFETY: `name` is a static NUL-terminated string.
    let format = u16::try_from(unsafe { RegisterClipboardFormatW(name) }).ok()?;
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

fn format_name(format: u16) -> String {
    let mut name = [0u16; 128];
    // SAFETY: a writable buffer; predefined formats have no name and return 0.
    let len = unsafe { GetClipboardFormatNameW(u32::from(format), &mut name) };
    if len > 0 {
        return String::from_utf16_lossy(&name[..len as usize]);
    }
    match format {
        1 => "CF_TEXT".into(),
        13 => "CF_UNICODETEXT".into(),
        15 => "CF_HDROP".into(),
        16 => "CF_LOCALE".into(),
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

    #[test]
    fn data_object_offers_what_explorer_offers() {
        sta();
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<PathBuf> = ["ნახაზი — სართული 1.dwg", "ხარჯთაღრიცხვა.pdf", "plain.txt"]
            .iter()
            .map(|name| {
                let path = dir.path().join(name);
                fs::write(&path, b"jerry").unwrap();
                path
            })
            .collect();

        let data = data_object(&paths).unwrap();

        let formats = format_names(&data).unwrap();
        println!("formats: {formats:?}");
        for expected in ["Shell IDList Array", "CF_HDROP", "FileNameW"] {
            assert!(
                formats.iter().any(|f| f == expected),
                "{expected} missing: {formats:?}"
            );
        }
        // CF_HDROP renders the exact paths, Georgian names included.
        assert_eq!(hdrop_paths(&data).unwrap(), paths);
    }

    /// Spike #13: the three ways to get a Shell data object, side by side. The parent folder's
    /// `GetUIObjectOf` is what Explorer itself drags; `BHID_DataObject` must match it.
    #[test]
    fn bind_to_handler_matches_explorers_folder_object() {
        use windows::Win32::UI::Shell::{IShellFolder, SHBindToParent, SHCreateDataObject};

        sta();
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<PathBuf> = ["a.pdf", "ბ.dwg"]
            .iter()
            .map(|name| {
                let path = dir.path().join(name);
                fs::write(&path, b"jerry").unwrap();
                path
            })
            .collect();
        let pidls: Vec<Pidl> = paths.iter().map(|p| Pidl::from_path(p).unwrap()).collect();

        // The parent folder, and each item's last ID (a pointer into its absolute list, which
        // `pidls` keeps alive). Both files share the folder.
        let mut folder = None;
        let mut children: Vec<*const ITEMIDLIST> = Vec::new();
        for pidl in &pidls {
            let mut child = core::ptr::null_mut();
            // SAFETY: a valid absolute ID list and a valid out pointer.
            let parent: IShellFolder =
                unsafe { SHBindToParent(pidl.as_ptr(), Some(&mut child)) }.unwrap();
            folder.get_or_insert(parent);
            children.push(child.cast_const());
        }
        // SAFETY: the children are valid relative IDs of this folder.
        let explorer: IDataObject = unsafe {
            folder
                .unwrap()
                .GetUIObjectOf(HWND::default(), &children, None)
        }
        .unwrap();
        let absolute: Vec<_> = pidls.iter().map(Pidl::as_ptr).collect();
        // SAFETY: valid absolute IDs, relative to the desktop when no folder is given.
        let generic: IDataObject =
            unsafe { SHCreateDataObject(None, Some(&absolute), None) }.unwrap();
        let ours = data_object(&paths).unwrap();

        let explorer_formats = format_names(&explorer).unwrap();
        println!("GetUIObjectOf:      {explorer_formats:?}");
        println!("BHID_DataObject:    {:?}", format_names(&ours).unwrap());
        println!(
            "SHCreateDataObject: {:?}, CF_HDROP: {:?}",
            format_names(&generic).unwrap(),
            hdrop_paths(&generic).map_err(|e| e.code())
        );
        assert_eq!(format_names(&ours).unwrap(), explorer_formats);
        assert_eq!(hdrop_paths(&explorer).unwrap(), paths);
    }

    #[test]
    fn missing_file_is_a_path_error() {
        sta();
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone.txt");
        assert!(matches!(data_object(&[gone]), Err(DragError::Path { .. })));
        assert!(matches!(data_object(&[]), Err(DragError::Empty)));
    }
}
