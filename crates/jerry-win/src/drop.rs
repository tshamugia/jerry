//! OLE drag & drop into Jerry (spike #14).
//!
//! Jerry gets its own `IDropTarget`, and getting OLE to call it is the hard part. Under the
//! cursor there is always one of WebView2's windows, and those live in WebView2's browser
//! process: Chromium registers its drop target there (`Chrome_WidgetWin_1`), and OLE never walks
//! up from them to Jerry's windows. A target registered on Jerry's own windows, the way wry does
//! for Tauri's drop handler, is never called while the cursor is over the page (WebView2 156;
//! Tauri's handler fails the same way). So:
//! - [`disallow_webview_drop`] turns off WebView2's `AllowExternalDrop`, which removes
//!   Chromium's target, so the page can never take (and open) a dropped file;
//! - [`register`] puts Jerry's target on a transparent popup over the page, shown only while a
//!   drag is over Jerry (see `sync_overlay`), and on Jerry's own windows for the edges.
//!
//! OLE calls the target on the main STA thread (the thread that registered it), with no mouse
//! events reaching the page. The target only reports what happens to a [`DropHandler`], which
//! answers synchronously with the effect and the drop description; finding the element under
//! the cursor is the UI's job, a frame behind (see `src-tauri`).
//!
//! On drop the target hands over what the source offered: the CF_HDROP paths, or virtual files
//! (`FileGroupDescriptorW` + `FileContents`, from Outlook attachments and browsers). Virtual
//! file contents are taken as agile references so a worker thread can save them after `Drop`
//! returns; see [`VirtualFiles`].

use std::cell::RefCell;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    COLORREF, DV_E_FORMATETC, E_ABORT, E_UNEXPECTED, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, POINT,
    POINTL, RECT, S_FALSE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, GetVolumePathNameW, SetFileAttributesW,
};
use windows::Win32::System::Com::StructuredStorage::{IStorage, StgCreateDocfile};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, DVASPECT_CONTENT,
    FORMATETC, IDataObject, ISequentialStream, IStream, STGC_DEFAULT, STGM, STGM_READWRITE,
    STGM_SHARE_EXCLUSIVE, STGMEDIUM, STGMEDIUM_0, TYMED, TYMED_HGLOBAL, TYMED_ISTORAGE,
    TYMED_ISTREAM,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE, IDropTarget,
    IDropTarget_Impl, MK_ALT, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
};
use windows::Win32::System::SystemServices::{
    MK_CONTROL, MK_LBUTTON, MK_RBUTTON, MK_SHIFT, MODIFIERKEYS_FLAGS,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::Shell::{
    CFSTR_DROPDESCRIPTION, CFSTR_FILECONTENTS, CFSTR_FILEDESCRIPTORW, CFSTR_PERFORMEDDROPEFFECT,
    CLSID_DragDropHelper, DROPDESCRIPTION, DROPIMAGE_COPY, DROPIMAGE_INVALID, DROPIMAGE_LINK,
    DROPIMAGE_MOVE, DROPIMAGE_NONE, DROPIMAGETYPE, FD_ATTRIBUTES, FD_FILESIZE, FILEDESCRIPTORW,
    IDataObjectAsyncCapability, IDropTargetHelper,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EnumChildWindows, GA_ROOT, GetAncestor,
    GetClassNameW, GetClientRect, GetCursorPos, GetParent, GetPropW, GetWindowThreadProcessId,
    HWND_TOP, LWA_ALPHA, MA_NOACTIVATE, RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, WM_MOUSEACTIVATE, WM_TIMER,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP, WindowFromPoint,
};
use windows::core::{AgileReference, BOOL, HRESULT, IUnknown, Interface, PCWSTR, Ref, implement};

use crate::drag::{DropEffect, format_names, hdrop_paths};

/// The COM interface trait, so callers can hand over interfaces from other bindings built on
/// the same `windows-core` (the WebView2 controller) without depending on `windows` themselves.
pub use windows::core::Interface as ComInterface;

/// Mouse buttons and modifier keys during a drag (OLE's `grfKeyState`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    pub left: bool,
    pub right: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Keys {
    fn from_raw(keys: MODIFIERKEYS_FLAGS) -> Self {
        let has = |flag: u32| keys.0 & flag != 0;
        Self {
            left: has(MK_LBUTTON.0),
            right: has(MK_RBUTTON.0),
            ctrl: has(MK_CONTROL.0),
            shift: has(MK_SHIFT.0),
            alt: has(MK_ALT),
        }
    }
}

/// The effects a drag source allows (`dwOKEffects` of its `DoDragDrop`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    pub copy: bool,
    pub moves: bool,
    pub link: bool,
}

impl Effects {
    fn from_raw(effect: DROPEFFECT) -> Self {
        Self {
            copy: effect.0 & DROPEFFECT_COPY.0 != 0,
            moves: effect.0 & DROPEFFECT_MOVE.0 != 0,
            link: effect.0 & DROPEFFECT_LINK.0 != 0,
        }
    }

    pub fn allows(self, effect: DropEffect) -> bool {
        match effect {
            DropEffect::None => true,
            DropEffect::Copy => self.copy,
            DropEffect::Move => self.moves,
            DropEffect::Link => self.link,
        }
    }
}

/// Windows' rule for the effect of a drop, as Explorer applies it (PRD «Drag & Drop»): Ctrl
/// copies, Shift moves, Alt (or Ctrl+Shift) links; without a modifier, the same volume moves and
/// another volume copies. A forced effect the source doesn't allow is refused; a default one
/// falls back to whatever the source allows.
pub fn choose_effect(keys: Keys, allowed: Effects, same_volume: bool) -> DropEffect {
    let forced = if keys.alt || (keys.ctrl && keys.shift) {
        Some(DropEffect::Link)
    } else if keys.ctrl {
        Some(DropEffect::Copy)
    } else if keys.shift {
        Some(DropEffect::Move)
    } else {
        None
    };
    if let Some(effect) = forced {
        return if allowed.allows(effect) {
            effect
        } else {
            DropEffect::None
        };
    }
    let default = if same_volume {
        DropEffect::Move
    } else {
        DropEffect::Copy
    };
    [
        default,
        DropEffect::Copy,
        DropEffect::Move,
        DropEffect::Link,
    ]
    .into_iter()
    .find(|&effect| allowed.allows(effect))
    .unwrap_or(DropEffect::None)
}

/// The volume a path is on (`C:\`, `\\server\share\`, or a mount point), for the same-volume
/// rule. `None` when Windows can't tell.
pub fn volume_of(path: &Path) -> Option<PathBuf> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut volume = [0u16; 1024];
    // SAFETY: `wide` is NUL-terminated and the buffer is writable.
    unsafe { GetVolumePathNameW(PCWSTR(wide.as_ptr()), &mut volume) }.ok()?;
    let len = volume.iter().position(|&c| c == 0).unwrap_or(volume.len());
    let text = String::from_utf16_lossy(&volume[..len]).to_lowercase();
    Some(PathBuf::from(text))
}

/// A point in the target window's client area, in physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientPoint {
    pub x: i32,
    pub y: i32,
}

/// One virtual file as its `FILEDESCRIPTORW` describes it.
#[derive(Clone, Debug)]
pub struct VirtualFileInfo {
    /// The name the source gives, possibly a relative path (`folder\file.txt`). Untrusted: the
    /// saver must reject `..` and absolute paths, like ZIP extraction.
    pub name: String,
    pub size: Option<u64>,
    pub is_dir: bool,
}

/// What a drag carries, read once when it enters Jerry.
#[derive(Clone, Debug, Default)]
pub struct DragInfo {
    pub formats: Vec<String>,
    /// CF_HDROP: real files and folders.
    pub files: Vec<PathBuf>,
    /// `FileGroupDescriptorW`: virtual files (Outlook attachments, browser images and links).
    pub virtual_files: Vec<VirtualFileInfo>,
    pub allowed: Effects,
    /// The data object lets the target extract asynchronously (`IDataObjectAsyncCapability`).
    pub async_capable: bool,
    /// The drag started in Jerry itself (`drag::drag` is running on this thread).
    pub from_self: bool,
}

/// The text Windows shows under the drag image while it's over a target ("Move to %1").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Description {
    /// The icon next to the text.
    pub icon: DropEffect,
    /// The message, with `%1` where `insert` goes.
    pub message: String,
    /// Shown in the accent style in place of `%1` (the target folder's name).
    pub insert: String,
}

/// The handler's answer for the current cursor position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feedback {
    pub effect: DropEffect,
    /// `None` leaves Windows' default text ("Copy", "Move").
    pub description: Option<Description>,
}

impl Feedback {
    pub const NONE: Self = Self {
        effect: DropEffect::None,
        description: None,
    };
}

/// What a drop delivers to the handler.
pub struct Dropped {
    pub files: Vec<PathBuf>,
    pub virtual_files: Option<VirtualFiles>,
    /// The drag was made with the right button (show the Copy / Move menu).
    pub right_button: bool,
    /// Time spent in `Drop` before the handler was called (reading the data object).
    pub read_time: Duration,
}

/// What the handler did with a drop, reported back to the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropResult {
    /// Returned from `Drop`. Anything but `Move` tells the source not to delete: the target
    /// either copied or moved the files itself (an optimized move).
    pub effect: DropEffect,
    /// Written as "Performed DropEffect", the reliable signal Explorer reads. `None` (the format
    /// value DROPEFFECT_NONE) after an optimized move.
    pub performed: DropEffect,
}

/// Jerry's side of a drag over its window. Called on the main thread.
pub trait DropHandler {
    fn enter(&self, info: &DragInfo, at: ClientPoint, keys: Keys) -> Feedback;
    fn over(&self, at: ClientPoint, keys: Keys) -> Feedback;
    fn leave(&self);
    fn dropped(&self, dropped: Dropped, at: ClientPoint, keys: Keys) -> DropResult;
    /// Every OLE call as it arrives, before anything else (diagnostics).
    fn trace(&self, _call: &str) {}
}

/// A drop target registered on a window tree; revoked when dropped.
pub struct Registration {
    windows: Vec<HWND>,
    overlay: Option<HWND>,
    _target: IDropTarget,
}

impl Drop for Registration {
    fn drop(&mut self) {
        for &hwnd in self.windows.iter().chain(&self.overlay) {
            // SAFETY: a window we registered; a destroyed window just fails the call.
            let _ = unsafe { RevokeDragDrop(hwnd) };
        }
        if let Some(overlay) = self.overlay {
            OVERLAY.with(|state| state.borrow_mut().take());
            // SAFETY: our own window, destroyed on the thread that created it.
            let _ = unsafe { DestroyWindow(overlay) };
        }
    }
}

impl Registration {
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    /// The drag overlay exists (see [`register`]).
    pub fn has_overlay(&self) -> bool {
        self.overlay.is_some()
    }
}

// The drag overlay (spike #14). Under WebView2 (156) the window under the cursor is always one
// of WebView2's browser-process windows, and OLE never walks up from them to Jerry's own
// windows: a drop target registered on any Jerry window is never called while the cursor is
// over the page (Tauri's own drop handler fails the same way). So Jerry keeps a transparent
// popup over its client area, hidden until a drag comes: a timer shows it while a mouse
// button that was pressed outside Jerry is held over Jerry (a drag from another app), or while
// Jerry's own drag runs. OLE then finds the popup, which carries Jerry's drop target. The page
// gets no mouse input during an OLE drag anyway, so covering it costs nothing.

/// How often the overlay checks for a drag: rarely while no button is down, every frame while
/// one is.
const OVERLAY_IDLE: u32 = 50;
const OVERLAY_ACTIVE: u32 = 16;
const OVERLAY_TIMER: usize = 1;

struct OverlayState {
    top: HWND,
    handler: Rc<dyn DropHandler>,
    hwnd: HWND,
    was_down: bool,
    pressed_outside: bool,
    visible: bool,
    interval: u32,
}

thread_local! {
    /// The overlay of the window registered on this (the main) thread.
    static OVERLAY: RefCell<Option<OverlayState>> = const { RefCell::new(None) };
}

fn create_overlay(top: HWND, handler: Rc<dyn DropHandler>) -> windows::core::Result<HWND> {
    let class = windows::core::w!("JerryDropOverlay");
    // SAFETY: the module handle of this process.
    let instance = unsafe { GetModuleHandleW(None) }?;
    let info = WNDCLASSW {
        lpfnWndProc: Some(overlay_proc),
        hInstance: instance.into(),
        lpszClassName: class,
        ..Default::default()
    };
    // SAFETY: a valid class description; registering it twice just fails, which is fine.
    unsafe { RegisterClassW(&info) };
    // SAFETY: an owned, never-activated popup of our window; hidden until a drag comes.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            windows::core::w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            Some(top),
            None,
            Some(instance.into()),
            None,
        )
    }?;
    // Alpha 1 of 255: invisible, but still hit-tested (alpha 0 would let clicks through, and
    // OLE would miss it too).
    // SAFETY: our own layered window.
    unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA) }?;
    OVERLAY.with(|state| {
        *state.borrow_mut() = Some(OverlayState {
            top,
            handler,
            hwnd,
            was_down: false,
            pressed_outside: false,
            visible: false,
            interval: OVERLAY_IDLE,
        })
    });
    // SAFETY: a timer on our own window.
    unsafe { SetTimer(Some(hwnd), OVERLAY_TIMER, OVERLAY_IDLE, None) };
    Ok(hwnd)
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER => {
            sync_overlay();
            LRESULT(0)
        }
        // Never take focus from Jerry's page.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // SAFETY: default handling for our own window.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Shows the overlay over Jerry while a drag is over it, hides it otherwise. Called by the
/// overlay's timer, and by [`crate::drag::drag`] right before Jerry's own drag starts (its modal
/// loop may not run the timer).
pub(crate) fn sync_overlay() {
    OVERLAY.with(|cell| {
        let mut cell = cell.borrow_mut();
        let Some(state) = cell.as_mut() else {
            return;
        };
        let down = buttons_down();
        if !down && !state.was_down && !state.visible {
            // Idle: no button held, nothing to do (the common tick, kept to two key reads).
            return;
        }
        let mut cursor = POINT::default();
        // SAFETY: a valid out pointer.
        let _ = unsafe { GetCursorPos(&mut cursor) };
        // SAFETY: plain window queries.
        let under = unsafe { GetAncestor(WindowFromPoint(cursor), GA_ROOT) };
        let over_jerry = under == state.top || under == state.hwnd;
        if down && !state.was_down {
            state.pressed_outside = !over_jerry;
        }
        state.was_down = down;
        let want = down && over_jerry && (state.pressed_outside || crate::drag::is_dragging());
        if want != state.visible {
            state.visible = want;
            let mut class = [0u16; 64];
            // SAFETY: a writable buffer.
            let len = unsafe { GetClassNameW(under, &mut class) }.max(0) as usize;
            state.handler.trace(&format!(
                "overlay {} (down {down}, over Jerry {over_jerry}, pressed outside {}, under {})",
                if want { "shown" } else { "hidden" },
                state.pressed_outside,
                String::from_utf16_lossy(&class[..len]),
            ));
            if want {
                let mut client = RECT::default();
                let mut origin = POINT::default();
                // SAFETY: our own windows and valid out pointers.
                unsafe {
                    let _ = GetClientRect(state.top, &mut client);
                    let _ = ClientToScreen(state.top, &mut origin);
                    let _ = SetWindowPos(
                        state.hwnd,
                        Some(HWND_TOP),
                        origin.x,
                        origin.y,
                        client.right,
                        client.bottom,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    );
                }
            } else {
                // SAFETY: our own window.
                let _ = unsafe { ShowWindow(state.hwnd, SW_HIDE) };
            }
        }
        let interval = if down { OVERLAY_ACTIVE } else { OVERLAY_IDLE };
        if interval != state.interval {
            state.interval = interval;
            // SAFETY: replaces the timer on our own window.
            unsafe { SetTimer(Some(state.hwnd), OVERLAY_TIMER, interval, None) };
        }
    });
}

/// The overlay is showing (diagnostics).
pub fn overlay_visible() -> bool {
    OVERLAY.with(|state| state.borrow().as_ref().is_some_and(|s| s.visible))
}

fn buttons_down() -> bool {
    // SAFETY: plain input queries; physical keys, so swapped buttons don't matter here.
    unsafe {
        GetAsyncKeyState(i32::from(VK_LBUTTON.0)) < 0
            || GetAsyncKeyState(i32::from(VK_RBUTTON.0)) < 0
    }
}

/// One window of a tree, for diagnostics.
#[derive(Clone, Debug)]
pub struct WindowNode {
    pub depth: usize,
    pub class: String,
    pub process_id: u32,
    pub ours: bool,
    /// A drop target is registered on it (OLE's window property).
    pub drop_target: bool,
}

/// The window `top` and all its descendants, with their drop targets (diagnostics).
pub fn window_tree(top: isize) -> Vec<WindowNode> {
    let top = HWND(top as *mut core::ffi::c_void);
    let mut all = vec![top];
    all.extend(children(top));
    all.into_iter()
        .map(|hwnd| {
            let mut depth = 0;
            let mut parent = hwnd;
            // SAFETY: plain window queries; a stale handle only ends the walk.
            while let Ok(up) = unsafe { GetParent(parent) } {
                if up == top || up.is_invalid() {
                    if up == top {
                        depth += 1;
                    }
                    break;
                }
                depth += 1;
                parent = up;
            }
            let mut class = [0u16; 128];
            // SAFETY: a writable buffer.
            let len = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
            let mut process_id = 0;
            // SAFETY: a valid out pointer.
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
            WindowNode {
                depth,
                class: String::from_utf16_lossy(&class[..len]),
                process_id,
                // SAFETY: no arguments.
                ours: process_id == unsafe { GetCurrentProcessId() },
                drop_target: has_drop_target(hwnd),
            }
        })
        .collect()
}

fn has_drop_target(hwnd: HWND) -> bool {
    // SAFETY: reading a window property by a static name; a missing one is a null handle.
    let prop: HANDLE = unsafe { GetPropW(hwnd, windows::core::w!("OleDropTargetInterface")) };
    !prop.is_invalid()
}

fn children(top: HWND) -> Vec<HWND> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `&mut Vec<HWND>` passed below, alive for the whole enumeration.
        let list = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        list.push(hwnd);
        true.into()
    }
    let mut list: Vec<HWND> = Vec::new();
    // SAFETY: the callback only pushes into `list`, which outlives the synchronous call.
    let _ = unsafe {
        EnumChildWindows(
            Some(top),
            Some(collect),
            LPARAM(&mut list as *mut Vec<HWND> as isize),
        )
    };
    list
}

/// Registers Jerry's drop target on the drag overlay over `top` (what OLE finds over the page)
/// and on `top` and its descendant windows in Jerry's process (what it finds at the edges),
/// replacing whatever was registered there. Main thread only, after [`crate::drag::init_ole`].
pub fn register(top: isize, handler: Rc<dyn DropHandler>) -> windows::core::Result<Registration> {
    let top = HWND(top as *mut core::ffi::c_void);
    // SAFETY: creating the Shell's in-process drag image helper on an STA thread.
    let helper: Option<IDropTargetHelper> =
        unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER) }.ok();
    let target: IDropTarget = Target {
        hwnd: top,
        handler: handler.clone(),
        helper,
        session: RefCell::new(None),
    }
    .into();

    // SAFETY: no arguments.
    let pid = unsafe { GetCurrentProcessId() };
    let mut windows = Vec::new();
    for hwnd in std::iter::once(top).chain(children(top)) {
        let mut owner = 0;
        // SAFETY: a valid out pointer.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner != pid {
            continue;
        }
        // SAFETY: a window of this process; failing (nothing registered) is fine.
        let _ = unsafe { RevokeDragDrop(hwnd) };
        // SAFETY: as above; OLE keeps its own reference to the target.
        unsafe { RegisterDragDrop(hwnd, &target) }?;
        windows.push(hwnd);
    }
    // The overlay is what OLE actually finds over the page (see `sync_overlay`).
    let overlay = create_overlay(top, handler)?;
    // SAFETY: our own window; OLE keeps its own reference to the target.
    unsafe { RegisterDragDrop(overlay, &target) }?;
    Ok(Registration {
        windows,
        overlay: Some(overlay),
        _target: target,
    })
}

/// Turns off WebView2's own external drop handling (`ICoreWebView2Controller4::
/// put_AllowExternalDrop(FALSE)`), as wry does when Tauri's handler is on. `controller` is the
/// `ICoreWebView2Controller` Tauri's `with_webview` hands out.
pub fn disallow_webview_drop<T: Interface>(controller: &T) -> windows::core::Result<()> {
    // ICoreWebView2Controller4 (WebView2 1.0.1185+). Declared by hand: the WebView2 bindings
    // aren't a dependency of Jerry, and only this one slot is needed.
    const IID_CONTROLLER4: windows::core::GUID =
        windows::core::GUID::from_u128(0x97d418d5_a426_4e49_a151_e1a10f327d9e);
    /// IUnknown (3) + ICoreWebView2Controller (23) + Controller2 (2) + Controller3 (8), then
    /// `get_AllowExternalDrop` and `put_AllowExternalDrop`.
    #[repr(C)]
    struct Vtbl {
        base: [usize; 36],
        get_allow_external_drop:
            unsafe extern "system" fn(*mut core::ffi::c_void, *mut BOOL) -> HRESULT,
        put_allow_external_drop: unsafe extern "system" fn(*mut core::ffi::c_void, BOOL) -> HRESULT,
    }

    let this = controller.as_raw();
    // SAFETY: `controller` is a live COM object; borrowing it as IUnknown doesn't change the
    // reference count.
    let unknown = unsafe { IUnknown::from_raw_borrowed(&this) }
        .ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?;
    let mut raw = core::ptr::null_mut();
    // SAFETY: a plain QueryInterface; on success `raw` holds a reference we release below.
    unsafe { unknown.query(&IID_CONTROLLER4, &mut raw) }.ok()?;
    let mut allowed = BOOL(1);
    // SAFETY: `raw` is an ICoreWebView2Controller4, whose vtable starts with the layout above;
    // `from_raw` takes over the QueryInterface reference and releases it.
    let hr = unsafe {
        let vtbl = &**(raw as *const *const Vtbl);
        let hr = (vtbl.put_allow_external_drop)(raw, false.into());
        let _ = (vtbl.get_allow_external_drop)(raw, &mut allowed);
        drop(IUnknown::from_raw(raw));
        hr
    };
    hr.ok()?;
    if allowed.as_bool() {
        Err(E_UNEXPECTED.into())
    } else {
        Ok(())
    }
}

/// Per-drag state on the target.
struct Session {
    data: IDataObject,
    info: DragInfo,
    right_button: bool,
    description: Option<Description>,
}

#[implement(IDropTarget)]
struct Target {
    hwnd: HWND,
    handler: Rc<dyn DropHandler>,
    helper: Option<IDropTargetHelper>,
    session: RefCell<Option<Session>>,
}

impl Target {
    fn client(&self, pt: &POINTL) -> (ClientPoint, POINT) {
        let screen = POINT { x: pt.x, y: pt.y };
        let mut client = screen;
        // SAFETY: our own window and a valid point.
        let _ = unsafe { ScreenToClient(self.hwnd, &mut client) };
        (
            ClientPoint {
                x: client.x,
                y: client.y,
            },
            screen,
        )
    }

    /// Writes the drop description into the data object when it changed. The Shell's drag image
    /// window reads it on the next `IDropTargetHelper` call.
    fn describe(&self, session: &mut Session, description: Option<&Description>) {
        if session.description.as_ref() == description {
            return;
        }
        if let Err(error) = set_description(&session.data, description) {
            self.handler
                .trace(&format!("SetData(DropDescription) refused: {error}"));
        }
        session.description = description.cloned();
    }
}

/// The description to show: the handler's, or the "no drop" icon without text where Jerry refuses
/// the drop. Without one, Windows falls back to its own text for the drag's default effect ("+ Copy")
/// even though the drop would be refused.
fn shown(feedback: &Feedback) -> Option<Description> {
    feedback.description.clone().or_else(|| {
        (feedback.effect == DropEffect::None).then(|| Description {
            icon: DropEffect::None,
            message: String::new(),
            insert: String::new(),
        })
    })
}

fn raw_effect(effect: DropEffect) -> DROPEFFECT {
    match effect {
        DropEffect::None => DROPEFFECT_NONE,
        DropEffect::Copy => DROPEFFECT_COPY,
        DropEffect::Move => DROPEFFECT_MOVE,
        DropEffect::Link => DROPEFFECT_LINK,
    }
}

/// Never report more than the source allows.
fn allowed_effect(effect: DropEffect, allowed: Effects) -> DropEffect {
    if allowed.allows(effect) {
        effect
    } else {
        DropEffect::None
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.handler.trace("DragEnter");
        let data = data.ok()?.clone();
        // SAFETY: OLE passes a valid in/out pointer holding the source's allowed effects.
        let allowed = Effects::from_raw(unsafe { effect.read() });
        let info = read_info(&data, allowed);
        let keys = Keys::from_raw(keys);
        let (at, screen) = self.client(pt);
        let feedback = self.handler.enter(&info, at, keys);
        let chosen = raw_effect(allowed_effect(feedback.effect, allowed));
        let mut session = Session {
            data,
            info,
            right_button: keys.right,
            description: None,
        };
        self.describe(&mut session, shown(&feedback).as_ref());
        // SAFETY: as above.
        unsafe { effect.write(chosen) };
        if let Some(helper) = &self.helper {
            // SAFETY: our window, a live data object and a valid point.
            let _ = unsafe { helper.DragEnter(self.hwnd, &session.data, &screen, chosen) };
        }
        *self.session.borrow_mut() = Some(session);
        Ok(())
    }

    fn DragOver(
        &self,
        keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let keys = Keys::from_raw(keys);
        let (at, screen) = self.client(pt);
        let feedback = self.handler.over(at, keys);
        let mut chosen = DROPEFFECT_NONE;
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.right_button |= keys.right;
            chosen = raw_effect(allowed_effect(feedback.effect, session.info.allowed));
            self.describe(session, shown(&feedback).as_ref());
        }
        // SAFETY: OLE passes a valid in/out pointer.
        unsafe { effect.write(chosen) };
        if let Some(helper) = &self.helper {
            // SAFETY: a valid point.
            let _ = unsafe { helper.DragOver(&screen, chosen) };
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.handler.trace("DragLeave");
        if let Some(mut session) = self.session.borrow_mut().take() {
            // Clear our text, so the next target doesn't show "Move to Drawings".
            self.describe(&mut session, None);
        }
        if let Some(helper) = &self.helper {
            // SAFETY: a plain call on the helper.
            let _ = unsafe { helper.DragLeave() };
        }
        self.handler.leave();
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.handler.trace("Drop");
        let start = Instant::now();
        let data = data.ok()?.clone();
        let mut session = self.session.borrow_mut().take();
        // SAFETY: OLE passes a valid in/out pointer.
        let allowed = Effects::from_raw(unsafe { effect.read() });
        let info = match &session {
            Some(session) => session.info.clone(),
            None => read_info(&data, allowed),
        };
        let keys = Keys::from_raw(keys);
        let (at, screen) = self.client(pt);
        let virtual_files = (info.files.is_empty() && !info.virtual_files.is_empty())
            .then(|| VirtualFiles::take(&data, &info))
            .flatten();
        let dropped = Dropped {
            files: info.files.clone(),
            virtual_files,
            right_button: session.as_ref().is_some_and(|s| s.right_button),
            read_time: start.elapsed(),
        };
        let result = self.handler.dropped(dropped, at, keys);
        let chosen = raw_effect(allowed_effect(result.effect, info.allowed));
        if let Some(session) = session.as_mut() {
            self.describe(session, None);
        }
        set_dword(
            &data,
            CFSTR_PERFORMEDDROPEFFECT,
            raw_effect(result.performed).0,
        );
        // SAFETY: as above.
        unsafe { effect.write(chosen) };
        if let Some(helper) = &self.helper {
            // SAFETY: a live data object and a valid point.
            let _ = unsafe { helper.Drop(&data, &screen, chosen) };
        }
        Ok(())
    }
}

fn read_info(data: &IDataObject, allowed: Effects) -> DragInfo {
    DragInfo {
        formats: format_names(data).unwrap_or_default(),
        files: hdrop_paths(data).unwrap_or_default(),
        virtual_files: file_descriptors(data).unwrap_or_default(),
        allowed,
        async_capable: data
            .cast::<IDataObjectAsyncCapability>()
            .ok()
            // SAFETY: a plain COM call.
            .and_then(|op| unsafe { op.GetAsyncMode() }.ok())
            .is_some_and(|mode| mode.as_bool()),
        from_self: crate::drag::is_dragging(),
    }
}

fn format_id(name: PCWSTR) -> u16 {
    // SAFETY: a static NUL-terminated name; registering an existing format returns its id.
    u16::try_from(unsafe { RegisterClipboardFormatW(name) }).unwrap_or(0)
}

fn format_etc(format: u16, tymed: TYMED, index: i32) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        ptd: core::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: index,
        tymed: tymed.0 as u32,
    }
}

/// Copies `bytes` into a movable HGLOBAL (released by whoever takes the medium).
fn hglobal_from(bytes: &[u8]) -> windows::core::Result<HGLOBAL> {
    // SAFETY: a plain allocation.
    let global = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) }?;
    // SAFETY: a fresh, valid block.
    let ptr = unsafe { GlobalLock(global) }.cast::<u8>();
    if !ptr.is_null() {
        // SAFETY: the block holds at least `bytes.len()` bytes.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
        // SAFETY: balanced with the lock; "fails" with NO_ERROR when the count reaches zero.
        let _ = unsafe { GlobalUnlock(global) };
    }
    Ok(global)
}

/// Hands `bytes` to the data object under `format` (it takes ownership of the memory).
fn set_hglobal(data: &IDataObject, format: u16, bytes: &[u8]) -> windows::core::Result<()> {
    let global = hglobal_from(bytes)?;
    let medium = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 { hGlobal: global },
        pUnkForRelease: core::mem::ManuallyDrop::new(None),
    };
    // SAFETY: fRelease = true hands the HGLOBAL to the data object, which frees it.
    unsafe { data.SetData(&format_etc(format, TYMED_HGLOBAL, -1), &medium, true) }
}

fn set_dword(data: &IDataObject, name: PCWSTR, value: u32) {
    let _ = set_hglobal(data, format_id(name), &value.to_le_bytes());
}

fn icon(effect: DropEffect) -> DROPIMAGETYPE {
    match effect {
        DropEffect::None => DROPIMAGE_NONE,
        DropEffect::Copy => DROPIMAGE_COPY,
        DropEffect::Move => DROPIMAGE_MOVE,
        DropEffect::Link => DROPIMAGE_LINK,
    }
}

fn set_description(
    data: &IDataObject,
    description: Option<&Description>,
) -> windows::core::Result<()> {
    let mut raw = DROPDESCRIPTION {
        r#type: DROPIMAGE_INVALID,
        ..Default::default()
    };
    if let Some(description) = description {
        raw.r#type = icon(description.icon);
        // DROPDESCRIPTION is packed: fill local arrays, then copy them in.
        raw.szMessage = wide_260(&description.message);
        raw.szInsert = wide_260(&description.insert);
    }
    // SAFETY: DROPDESCRIPTION is plain data; viewing it as bytes is sound.
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&raw as *const DROPDESCRIPTION).cast::<u8>(),
            size_of::<DROPDESCRIPTION>(),
        )
    };
    // Sources without drag images (some apps) refuse SetData; the description just isn't shown.
    set_hglobal(data, format_id(CFSTR_DROPDESCRIPTION), bytes)
}

fn wide_260(text: &str) -> [u16; 260] {
    let mut wide = [0u16; 260];
    for (slot, unit) in wide.iter_mut().zip(text.encode_utf16().take(259)) {
        *slot = unit;
    }
    wide
}

/// Reads `FileGroupDescriptorW`, if the data object has it.
fn file_descriptors(data: &IDataObject) -> windows::core::Result<Vec<VirtualFileInfo>> {
    let format = format_etc(format_id(CFSTR_FILEDESCRIPTORW), TYMED_HGLOBAL, -1);
    // SAFETY: a plain COM call; the medium is released below.
    let mut medium = unsafe { data.GetData(&format) }?;
    // SAFETY: TYMED_HGLOBAL was requested, so `hGlobal` is the active field.
    let global = unsafe { medium.u.hGlobal };
    // SAFETY: the block's real size, for bounds checks.
    let size = unsafe { GlobalSize(global) };
    // SAFETY: locking a valid HGLOBAL; null means it has no memory.
    let ptr = unsafe { GlobalLock(global) }.cast::<u8>().cast_const();
    let mut files = Vec::new();
    if !ptr.is_null() && size >= 4 {
        // SAFETY: the block starts with the descriptor count (FILEGROUPDESCRIPTORW is packed).
        let count = unsafe { ptr.cast::<u32>().read_unaligned() } as usize;
        let entry = size_of::<FILEDESCRIPTORW>();
        for index in 0..count {
            let offset = 4 + index * entry;
            if offset + entry > size {
                break;
            }
            // SAFETY: in bounds (checked above); FILEDESCRIPTORW is plain packed data.
            let descriptor = unsafe { ptr.add(offset).cast::<FILEDESCRIPTORW>().read_unaligned() };
            let name = descriptor.cFileName;
            let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            let flags = descriptor.dwFlags;
            let attributes = descriptor.dwFileAttributes;
            files.push(VirtualFileInfo {
                name: String::from_utf16_lossy(&name[..len]),
                size: (flags & FD_FILESIZE.0 as u32 != 0).then(|| {
                    (u64::from(descriptor.nFileSizeHigh) << 32) | u64::from(descriptor.nFileSizeLow)
                }),
                is_dir: flags & FD_ATTRIBUTES.0 as u32 != 0
                    && attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
            });
        }
        // SAFETY: balanced with the lock above.
        let _ = unsafe { GlobalUnlock(global) };
    }
    // SAFETY: we own the medium GetData returned and release it once.
    unsafe { ReleaseStgMedium(&mut medium) };
    Ok(files)
}

/// The content of one virtual file, ready to be saved from any thread.
enum Content {
    Stream(AgileReference<IStream>),
    Storage(AgileReference<IStorage>),
    Bytes(Vec<u8>),
    Folder,
    /// The source refused this item.
    Missing(HRESULT),
}

/// Virtual files taken from a drop. Save them on a worker thread (see [`init_worker`]).
pub struct VirtualFiles {
    items: Vec<(VirtualFileInfo, Content)>,
    /// When the source supports async extraction, the operation to end once saving is done.
    operation: Option<AgileReference<IDataObjectAsyncCapability>>,
    /// Time spent taking the contents in `Drop`.
    pub take_time: Duration,
}

impl VirtualFiles {
    /// Takes every `FileContents` medium from `data` during `Drop`. Streams and storages are
    /// only references into the source, so this is fast; their bytes are read later.
    fn take(data: &IDataObject, info: &DragInfo) -> Option<Self> {
        let start = Instant::now();
        let contents = format_id(CFSTR_FILECONTENTS);
        let items = info
            .virtual_files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                let content = if file.is_dir {
                    Content::Folder
                } else {
                    take_content(data, contents, index as i32)
                };
                (file.clone(), content)
            })
            .collect();
        let operation = if info.async_capable {
            data.cast::<IDataObjectAsyncCapability>()
                .ok()
                .filter(|op| {
                    // SAFETY: a plain COM call; no bind context.
                    unsafe { op.StartOperation(None) }.is_ok()
                })
                .and_then(|op| AgileReference::new(&op).ok())
        } else {
            None
        };
        Some(Self {
            items,
            operation,
            take_time: start.elapsed(),
        })
    }

    pub fn infos(&self) -> impl Iterator<Item = &VirtualFileInfo> {
        self.items.iter().map(|(info, _)| info)
    }

    /// Saves every item with `save`, which gets the item's descriptor and a writer that puts its
    /// content at a path the caller chose. Call on a thread set up with [`init_worker`].
    pub fn save_each(
        self,
        mut save: impl FnMut(&VirtualFileInfo, &mut dyn FnMut(&Path) -> io::Result<u64>),
    ) {
        for (info, content) in &self.items {
            let mut write = |path: &Path| write_content(content, path);
            save(info, &mut write);
        }
        self.end(HRESULT(0), DROPEFFECT_COPY);
    }

    /// Drops the items unsaved (a refused drop, or Cancel in the right-drag menu).
    pub fn abandon(self) {
        self.end(E_ABORT, DROPEFFECT_NONE);
    }

    /// Tells an async-capable source that extraction is over.
    fn end(&self, result: HRESULT, effect: DROPEFFECT) {
        if let Some(operation) = &self.operation
            && let Ok(operation) = operation.resolve()
        {
            // SAFETY: ends the operation StartOperation began, with what actually happened.
            let _ = unsafe { operation.EndOperation(result, None, effect.0) };
        }
    }
}

fn take_content(data: &IDataObject, format: u16, index: i32) -> Content {
    let tymed = TYMED(TYMED_ISTREAM.0 | TYMED_ISTORAGE.0 | TYMED_HGLOBAL.0);
    // SAFETY: a plain COM call; the medium is released below.
    let mut medium = match unsafe { data.GetData(&format_etc(format, tymed, index)) } {
        Ok(medium) => medium,
        Err(error) => return Content::Missing(error.code()),
    };
    let content = match TYMED(medium.tymed as i32) {
        // SAFETY: `tymed` says which union field is active; cloning adds our own reference.
        TYMED_ISTREAM => unsafe { (*medium.u.pstm).clone() }
            .and_then(|stream| AgileReference::new(&stream).ok())
            .map_or(Content::Missing(E_UNEXPECTED), Content::Stream),
        // SAFETY: as above.
        TYMED_ISTORAGE => unsafe { (*medium.u.pstg).clone() }
            .and_then(|storage| AgileReference::new(&storage).ok())
            .map_or(Content::Missing(E_UNEXPECTED), Content::Storage),
        TYMED_HGLOBAL => {
            // SAFETY: as above.
            let global = unsafe { medium.u.hGlobal };
            Content::Bytes(hglobal_bytes(global))
        }
        _ => Content::Missing(DV_E_FORMATETC),
    };
    // SAFETY: we own the medium GetData returned and release it once.
    unsafe { ReleaseStgMedium(&mut medium) };
    content
}

fn hglobal_bytes(global: HGLOBAL) -> Vec<u8> {
    // SAFETY: a valid HGLOBAL from the medium.
    let size = unsafe { GlobalSize(global) };
    // SAFETY: as above; null means no memory.
    let ptr = unsafe { GlobalLock(global) }.cast::<u8>().cast_const();
    if ptr.is_null() {
        return Vec::new();
    }
    // SAFETY: the locked block holds `size` bytes.
    let bytes = unsafe { core::slice::from_raw_parts(ptr, size) }.to_vec();
    // SAFETY: balanced with the lock.
    let _ = unsafe { GlobalUnlock(global) };
    bytes
}

/// Makes the calling worker thread a COM MTA thread, so it can resolve agile references.
pub fn init_worker() -> windows::core::Result<()> {
    // SAFETY: no reserved pointer; a second call just returns S_FALSE.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()
}

/// Writes one item to `path`, which must not exist yet. The new file is hidden (a
/// `.jerrypart`); the caller renames it and clears the attribute once it's complete.
fn write_content(content: &Content, path: &Path) -> io::Result<u64> {
    match content {
        Content::Stream(stream) => {
            let stream = stream.resolve().map_err(io::Error::other)?;
            let mut file = new_hidden(path)?;
            let mut buffer = vec![0u8; 256 * 1024];
            let mut total = 0u64;
            loop {
                let mut read = 0u32;
                // SAFETY: a writable buffer of the given size and a valid count pointer.
                let hr = unsafe {
                    ISequentialStream::Read(
                        &stream,
                        buffer.as_mut_ptr().cast(),
                        buffer.len() as u32,
                        Some(&mut read),
                    )
                };
                hr.ok().map_err(io::Error::other)?;
                if read == 0 {
                    break;
                }
                file.write_all(&buffer[..read as usize])?;
                total += u64::from(read);
                if hr == S_FALSE {
                    break;
                }
            }
            file.sync_all()?;
            Ok(total)
        }
        Content::Bytes(bytes) => {
            let mut file = new_hidden(path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            Ok(bytes.len() as u64)
        }
        Content::Storage(storage) => {
            let source = storage.resolve().map_err(io::Error::other)?;
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            // A new compound file (an Outlook .msg); without STGM_CREATE it fails if one exists.
            let mode = STGM(STGM_READWRITE.0 | STGM_SHARE_EXCLUSIVE.0);
            // SAFETY: a NUL-terminated path.
            let target = unsafe { StgCreateDocfile(PCWSTR(wide.as_ptr()), mode, None) }
                .map_err(io::Error::other)?;
            // SAFETY: plain COM calls on live storages.
            unsafe { source.CopyTo(None, None, &target) }.map_err(io::Error::other)?;
            // SAFETY: as above.
            unsafe { target.Commit(STGC_DEFAULT.0 as u32) }.map_err(io::Error::other)?;
            drop(target);
            // SAFETY: a NUL-terminated path.
            unsafe { SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_ATTRIBUTE_HIDDEN) }
                .map_err(io::Error::other)?;
            Ok(std::fs::metadata(path)?.len())
        }
        Content::Folder => Ok(0),
        Content::Missing(code) => Err(io::Error::other(windows::core::Error::from(*code))),
    }
}

fn new_hidden(path: &Path) -> io::Result<std::fs::File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .attributes(FILE_ATTRIBUTE_HIDDEN.0)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Effects = Effects {
        copy: true,
        moves: true,
        link: true,
    };
    const COPY_ONLY: Effects = Effects {
        copy: true,
        moves: false,
        link: false,
    };

    fn keys(ctrl: bool, shift: bool, alt: bool) -> Keys {
        Keys {
            left: true,
            ctrl,
            shift,
            alt,
            ..Keys::default()
        }
    }

    #[test]
    fn effect_follows_windows_rule() {
        let none = keys(false, false, false);
        assert_eq!(choose_effect(none, ALL, true), DropEffect::Move);
        assert_eq!(choose_effect(none, ALL, false), DropEffect::Copy);
        assert_eq!(
            choose_effect(keys(true, false, false), ALL, true),
            DropEffect::Copy
        );
        assert_eq!(
            choose_effect(keys(false, true, false), ALL, false),
            DropEffect::Move
        );
        assert_eq!(
            choose_effect(keys(false, false, true), ALL, true),
            DropEffect::Link
        );
        assert_eq!(
            choose_effect(keys(true, true, false), ALL, true),
            DropEffect::Link
        );
    }

    #[test]
    fn effect_respects_what_the_source_allows() {
        // Outlook and browsers only allow copy: the same volume doesn't make it a move.
        assert_eq!(
            choose_effect(keys(false, false, false), COPY_ONLY, true),
            DropEffect::Copy
        );
        // A forced move the source can't do is refused, not silently turned into a copy.
        assert_eq!(
            choose_effect(keys(false, true, false), COPY_ONLY, true),
            DropEffect::None
        );
        assert_eq!(
            choose_effect(keys(false, false, false), Effects::default(), true),
            DropEffect::None
        );
    }

    #[test]
    fn volume_of_temp_is_a_drive_root() {
        let temp = std::env::temp_dir();
        let volume = volume_of(&temp).unwrap();
        assert!(volume.to_string_lossy().ends_with('\\'), "{volume:?}");
        assert!(
            temp.to_string_lossy()
                .to_lowercase()
                .starts_with(&*volume.to_string_lossy())
        );
    }
}
