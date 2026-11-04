//! The native side of Jerry's title bar: hit-testing for the caption buttons (ADR-0004).
//!
//! React draws the tab strip and the caption buttons, but the page lives in a WebView2 window
//! owned by the browser process, so the top-level window never sees `WM_NCHITTEST` for points
//! over it. Windows 11 shows Snap Layouts only when the window under the cursor answers
//! `HTMAXBUTTON`. So, like Windows Terminal, Jerry puts an invisible input window (layered, no
//! redirection bitmap) over the buttons React drew. It answers `HTMINBUTTON`, `HTMAXBUTTON` and
//! `HTCLOSE`, runs the button actions through `WM_SYSCOMMAND`, and reports hover and pressed
//! state so React can draw them.
//!
//! The empty parts of the tab strip don't need this: they use the CSS `app-region: drag`, which
//! WebView2 turns into a native caption (move, double-click to maximize, Aero Snap).
//!
//! Everything here runs on the window's UI thread (Jerry's main thread): [`install`] must be
//! called there. The other functions may be called from any thread; they reach the input window
//! through `SendMessageW`, which blocks until the UI thread has handled the message.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use jerry_core::JerryError;
use windows::Win32::Foundation::{
    COLORREF, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetActiveWindow, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, GWLP_USERDATA, GetClientRect,
    GetWindowLongPtrW, HTCAPTION, HTCLOSE, HTMAXBUTTON, HTMINBUTTON, HWND_TOP, IDC_ARROW, IsZoomed,
    LWA_ALPHA, LoadCursorW, PostMessageW, RegisterClassExW, SC_CLOSE, SC_MAXIMIZE, SC_MINIMIZE,
    SC_RESTORE, SM_CYFRAME, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER,
    SWP_NOSIZE, SWP_SHOWWINDOW, SendMessageW, SetLayeredWindowAttributes, SetWindowLongPtrW,
    SetWindowPos, WA_INACTIVE, WINDOW_EX_STYLE, WM_ACTIVATE, WM_APP, WM_DPICHANGED, WM_ERASEBKGND,
    WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
    WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_NCRBUTTONDBLCLK, WM_NCRBUTTONDOWN, WM_NCRBUTTONUP, WM_SIZE,
    WM_SYSCOMMAND, WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_LAYERED,
    WS_EX_NOREDIRECTIONBITMAP,
};
use windows::core::{PCWSTR, w};

const CLASS_NAME: PCWSTR = w!("JerryCaptionButtons");
/// `lparam` points to a `Vec<ButtonRect>` owned by the sender.
const WM_SET_BUTTONS: u32 = WM_APP + 1;
/// `lparam` points to an `Option<Listener>` owned by the sender; the handler takes it.
const WM_SET_LISTENER: u32 = WM_APP + 2;
/// Posted to itself so the listener runs from the message loop, never nested inside another
/// window procedure (Tauri's runtime isn't re-entrant).
const WM_NOTIFY_STATE: u32 = WM_APP + 3;
const SUBCLASS_ID: usize = 0x4A45_5252; // "JERR"

/// One of the three window buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionButton {
    Minimize,
    Maximize,
    Close,
}

impl CaptionButton {
    fn hit_code(self) -> u32 {
        match self {
            Self::Minimize => HTMINBUTTON,
            Self::Maximize => HTMAXBUTTON,
            Self::Close => HTCLOSE,
        }
    }

    fn from_hit_code(code: usize) -> Option<Self> {
        match u32::try_from(code).ok()? {
            HTMINBUTTON => Some(Self::Minimize),
            HTMAXBUTTON => Some(Self::Maximize),
            HTCLOSE => Some(Self::Close),
            _ => None,
        }
    }
}

/// Where React drew a caption button, in CSS pixels, measured from the top-right corner of the
/// client area. Caption buttons are right-anchored, so a live resize keeps them in place without
/// a new report, and a DPI change only rescales them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonRect {
    pub button: CaptionButton,
    /// Distance from the right edge of the client area to the button's right edge.
    pub right: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

/// What React needs to draw the caption buttons like native ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameState {
    pub hovered: Option<CaptionButton>,
    pub pressed: Option<CaptionButton>,
    pub maximized: bool,
    /// The window is the active (foreground) window; inactive windows dim their glyphs.
    pub active: bool,
}

/// Receives every change of [`FrameState`], on the UI thread.
pub type Listener = Box<dyn Fn(FrameState) + Send + 'static>;

#[derive(Default)]
struct Inner {
    buttons: Vec<ButtonRect>,
    /// `buttons` in client pixels of the parent, from the last layout.
    placed: Vec<(CaptionButton, RECT)>,
    state: FrameState,
    /// The last state the listener saw; `None` sends the next one unconditionally.
    sent: Option<FrameState>,
    notify_pending: bool,
    tracking_mouse: bool,
    listener: Option<Rc<dyn Fn(FrameState) + Send>>,
}

/// Per-window data, owned by the input window and freed in its `WM_NCDESTROY`.
struct Input {
    parent: HWND,
    window: Cell<HWND>,
    inner: RefCell<Inner>,
}

/// Puts the caption-button input window on top of `hwnd` (a top-level window, raw `HWND`) and
/// starts following its size, DPI and activation. Call it on the window's UI thread. Calling it
/// again for the same window does nothing.
pub fn install(hwnd: isize) -> Result<(), JerryError> {
    let parent = to_hwnd(hwnd);
    if find(parent).is_some() {
        return Ok(());
    }

    // SAFETY: GetModuleHandleW(None) returns this executable's module; nothing is borrowed.
    let instance = unsafe { GetModuleHandleW(None) }?;
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(input_proc),
        hInstance: instance.into(),
        // SAFETY: loading a predefined system cursor has no preconditions.
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }?,
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    // SAFETY: `class` is fully initialized and its strings are static.
    if unsafe { RegisterClassExW(&class) } == 0 {
        let error = windows::core::Error::from_thread();
        if error.code() != ERROR_CLASS_ALREADY_EXISTS.to_hresult() {
            return Err(error.into());
        }
    }

    let create = |ex_style: WINDOW_EX_STYLE| {
        // SAFETY: the class is registered and `parent` is a live window owned by this thread.
        // The window is sized and shown by the first layout.
        unsafe {
            CreateWindowExW(
                ex_style,
                CLASS_NAME,
                PCWSTR::null(),
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                Some(parent),
                None,
                Some(instance.into()),
                None,
            )
        }
    };
    // Layered with no redirection bitmap: the window takes mouse input but draws nothing, so the
    // buttons React painted underneath stay visible. A layered window counts as shown (and gets
    // the mouse) only once it has its attributes; full alpha over no bitmap still draws nothing.
    // A layered *child* needs a manifest that declares Windows 8 or later: jerry.exe has one,
    // test binaries don't, and they get a plain child that never paints.
    let window = match create(WS_EX_LAYERED | WS_EX_NOREDIRECTIONBITMAP) {
        Ok(window) => {
            // SAFETY: `window` was just created as a layered window.
            let shown = unsafe { SetLayeredWindowAttributes(window, COLORREF(0), 255, LWA_ALPHA) };
            if let Err(error) = shown {
                tracing::error!(%error, "can't make the caption-button input window visible");
            }
            window
        }
        Err(error) => {
            tracing::warn!(%error, "layered child window unavailable; using a plain one");
            create(WS_EX_NOREDIRECTIONBITMAP)?
        }
    };

    // SAFETY: plain queries on a live window.
    let (maximized, active) = unsafe { (IsZoomed(parent).as_bool(), GetActiveWindow() == parent) };
    let input = Box::into_raw(Box::new(Input {
        parent,
        window: Cell::new(window),
        inner: RefCell::new(Inner {
            state: FrameState {
                maximized,
                active,
                ..Default::default()
            },
            ..Default::default()
        }),
    }));
    // SAFETY: `window` was created above; the pointer stays valid until its WM_NCDESTROY, which
    // removes the parent subclass before freeing it.
    unsafe {
        SetWindowLongPtrW(window, GWLP_USERDATA, input as isize);
        if !SetWindowSubclass(parent, Some(parent_proc), SUBCLASS_ID, input as usize).as_bool() {
            let error = windows::core::Error::from_thread();
            // Its WM_NCDESTROY frees `input`.
            let _ = DestroyWindow(window);
            return Err(error.into());
        }
    }
    tracing::debug!("caption-button input window installed");
    Ok(())
}

/// Replaces the state listener and sends it the current state.
pub fn set_listener(hwnd: isize, listener: Listener) -> Result<(), JerryError> {
    let window = installed(hwnd)?;
    let mut slot = Some(listener);
    // SAFETY: SendMessageW returns only after the UI thread has handled the message, so `slot`
    // outlives its use there.
    unsafe {
        SendMessageW(
            window,
            WM_SET_LISTENER,
            None,
            Some(LPARAM(&raw mut slot as isize)),
        )
    };
    Ok(())
}

/// Tells the input window where React drew the caption buttons.
pub fn set_buttons(hwnd: isize, buttons: Vec<ButtonRect>) -> Result<(), JerryError> {
    let window = installed(hwnd)?;
    // SAFETY: as in `set_listener`, `buttons` outlives the synchronous SendMessageW.
    unsafe {
        SendMessageW(
            window,
            WM_SET_BUTTONS,
            None,
            Some(LPARAM(&raw const buttons as isize)),
        )
    };
    Ok(())
}

/// Does what a click on `button` does, for the keyboard and screen readers (their clicks reach
/// the page's buttons, not the input window). Works without [`install`].
pub fn press(hwnd: isize, button: CaptionButton) -> Result<(), JerryError> {
    let parent = to_hwnd(hwnd);
    // SAFETY: a plain query on a window handle; a stale one only answers false.
    let maximized = unsafe { IsZoomed(parent) }.as_bool();
    system_command(parent, button, maximized, LPARAM(0))
}

/// Sends the window the system command a native caption button sends. `WM_SYSCOMMAND` keeps the
/// window animations, and `SC_CLOSE` goes through `WM_CLOSE`, so Tauri's `CloseRequested` still
/// sees it.
fn system_command(
    parent: HWND,
    button: CaptionButton,
    maximized: bool,
    lparam: LPARAM,
) -> Result<(), JerryError> {
    let command = match button {
        CaptionButton::Minimize => SC_MINIMIZE,
        CaptionButton::Maximize if maximized => SC_RESTORE,
        CaptionButton::Maximize => SC_MAXIMIZE,
        CaptionButton::Close => SC_CLOSE,
    };
    // SAFETY: posting to a window handle; a stale one fails and is reported.
    unsafe {
        PostMessageW(
            Some(parent),
            WM_SYSCOMMAND,
            WPARAM(command as usize),
            lparam,
        )
    }?;
    Ok(())
}

fn to_hwnd(hwnd: isize) -> HWND {
    HWND(hwnd as *mut c_void)
}

fn find(parent: HWND) -> Option<HWND> {
    // SAFETY: a lookup by class name among `parent`'s children.
    unsafe { FindWindowExW(Some(parent), None, CLASS_NAME, PCWSTR::null()) }.ok()
}

fn installed(hwnd: isize) -> Result<HWND, JerryError> {
    find(to_hwnd(hwnd)).ok_or_else(|| {
        tracing::error!("the title bar's input window isn't installed");
        JerryError::internal()
    })
}

impl Input {
    /// Places the input window over the buttons, scaled to the parent's current DPI.
    fn layout(&self) {
        let parent = self.parent;
        let mut client = RECT::default();
        // SAFETY: plain queries on the live parent window.
        let (dpi, maximized) = unsafe {
            if GetClientRect(parent, &mut client).is_err() {
                return;
            }
            (GetDpiForWindow(parent), IsZoomed(parent).as_bool())
        };
        // While restored, the top few pixels are Tauri's resize border (its own child window);
        // stay below them. Maximized windows can't be resized, so the buttons reach the top.
        let resize_border = if maximized {
            0
        } else {
            // SAFETY: a metrics query with no preconditions.
            unsafe { GetSystemMetricsForDpi(SM_CYFRAME, dpi) }
        };

        let placed = place(&self.inner.borrow().buttons, client.right, dpi);
        let bounds = bounds(&placed);
        {
            let mut inner = self.inner.borrow_mut();
            inner.placed = placed;
            inner.state.maximized = maximized;
        }
        self.state_changed();

        let window = self.window.get();
        // SAFETY: `window` is our live child window. HWND_TOP keeps it above the WebView2
        // window, which may reorder itself.
        let result = unsafe {
            match bounds {
                Some(b) if b.right > b.left && b.bottom > b.top.max(resize_border) => {
                    let top = b.top.max(resize_border);
                    SetWindowPos(
                        window,
                        Some(HWND_TOP),
                        b.left,
                        top,
                        b.right - b.left,
                        b.bottom - top,
                        SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
                    )
                }
                _ => SetWindowPos(
                    window,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_HIDEWINDOW,
                ),
            }
        };
        if let Err(error) = result {
            tracing::warn!(%error, "can't place the caption-button input window");
        }
    }

    /// The hit-test code for a point in screen coordinates.
    fn hit_test(&self, lparam: LPARAM) -> u32 {
        let mut point = point_from_lparam(lparam);
        // SAFETY: converts in place against the live parent window.
        if !unsafe { ScreenToClient(self.parent, &mut point) }.as_bool() {
            return HTCAPTION;
        }
        let inner = self.inner.borrow();
        inner
            .placed
            .iter()
            .find(|(_, r)| {
                point.x >= r.left && point.x < r.right && point.y >= r.top && point.y < r.bottom
            })
            .map_or(HTCAPTION, |(button, _)| button.hit_code())
    }

    fn update(&self, change: impl FnOnce(&mut FrameState)) {
        change(&mut self.inner.borrow_mut().state);
        self.state_changed();
    }

    /// Schedules a listener call for after the current message.
    fn state_changed(&self) {
        let mut inner = self.inner.borrow_mut();
        if inner.notify_pending || inner.sent == Some(inner.state) {
            return;
        }
        inner.notify_pending = true;
        drop(inner);
        // SAFETY: posting to our own live window.
        let posted = unsafe {
            PostMessageW(
                Some(self.window.get()),
                WM_NOTIFY_STATE,
                WPARAM(0),
                LPARAM(0),
            )
        };
        if posted.is_err() {
            self.inner.borrow_mut().notify_pending = false;
        }
    }

    fn notify(&self) {
        let (listener, state) = {
            let mut inner = self.inner.borrow_mut();
            inner.notify_pending = false;
            if inner.sent == Some(inner.state) {
                return;
            }
            let Some(listener) = inner.listener.clone() else {
                return;
            };
            inner.sent = Some(inner.state);
            (listener, inner.state)
        };
        listener(state);
    }

    fn track_mouse_leave(&self) {
        if self.inner.borrow().tracking_mouse {
            return;
        }
        let mut event = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            // TME_NONCLIENT: the input window only ever sees non-client mouse messages.
            dwFlags: TME_LEAVE | TME_NONCLIENT,
            hwndTrack: self.window.get(),
            dwHoverTime: 0,
        };
        // SAFETY: `event` is a valid TRACKMOUSEEVENT for our live window.
        if unsafe { TrackMouseEvent(&mut event) }.is_ok() {
            self.inner.borrow_mut().tracking_mouse = true;
        }
    }

    fn click(&self, button: CaptionButton, lparam: LPARAM) {
        let maximized = self.inner.borrow().state.maximized;
        if let Err(error) = system_command(self.parent, button, maximized, lparam) {
            tracing::warn!(%error, ?button, "caption button command failed");
        }
    }
}

/// Button rects in client pixels. Edges are computed from the right edge, so neighbours share
/// them exactly and no pixel falls between two buttons.
fn place(buttons: &[ButtonRect], client_right: i32, dpi: u32) -> Vec<(CaptionButton, RECT)> {
    let scale = f64::from(dpi) / 96.0;
    let px = |css: f64| (css * scale).round() as i32;
    buttons
        .iter()
        .map(|b| {
            let rect = RECT {
                left: client_right - px(b.right + b.width),
                top: px(b.top),
                right: client_right - px(b.right),
                bottom: px(b.top + b.height),
            };
            (b.button, rect)
        })
        .collect()
}

/// The smallest rect around all buttons; `None` without buttons.
fn bounds(placed: &[(CaptionButton, RECT)]) -> Option<RECT> {
    placed.iter().map(|(_, r)| *r).reduce(|a, r| RECT {
        left: a.left.min(r.left),
        top: a.top.min(r.top),
        right: a.right.max(r.right),
        bottom: a.bottom.max(r.bottom),
    })
}

fn point_from_lparam(lparam: LPARAM) -> POINT {
    // GET_X_LPARAM / GET_Y_LPARAM: signed, so monitors left of or above the primary one work.
    POINT {
        x: i32::from((lparam.0 & 0xFFFF) as u16 as i16),
        y: i32::from(((lparam.0 >> 16) & 0xFFFF) as u16 as i16),
    }
}

/// Runs `body`, turning a panic into a logged default instead of aborting the process (a panic
/// can't unwind out of a window procedure).
fn guarded(body: impl FnOnce() -> LRESULT, fallback: impl FnOnce() -> LRESULT) -> LRESULT {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(result) => result,
        Err(_) => {
            tracing::error!("panic in the title bar's window procedure");
            fallback()
        }
    }
}

unsafe extern "system" fn input_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: called by Windows with a live `window`.
    let default = || unsafe { DefWindowProcW(window, message, wparam, lparam) };
    // SAFETY: GWLP_USERDATA is either 0 or the `Input` set in `install`, freed only below.
    let input = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const Input;
    if input.is_null() {
        return default();
    }

    if message == WM_NCDESTROY {
        // SAFETY: the last message for this window. Remove the parent subclass first, so nothing
        // can reach `input` after it is freed.
        unsafe {
            let parent = (*input).parent;
            let _ = RemoveWindowSubclass(parent, Some(parent_proc), SUBCLASS_ID);
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            drop(Box::from_raw(input.cast_mut()));
        }
        return default();
    }

    // SAFETY: non-null and alive until WM_NCDESTROY (handled above).
    let input = unsafe { &*input };
    guarded(
        || match message {
            WM_NCHITTEST => LRESULT(input.hit_test(lparam) as isize),

            WM_NCMOUSEMOVE => {
                let hovered = CaptionButton::from_hit_code(wparam.0);
                input.update(|s| s.hovered = hovered);
                if hovered.is_some() {
                    input.track_mouse_leave();
                }
                LRESULT(0)
            }

            WM_NCMOUSELEAVE => {
                input.inner.borrow_mut().tracking_mouse = false;
                input.update(|s| {
                    s.hovered = None;
                    s.pressed = None;
                });
                LRESULT(0)
            }

            // A fast second click arrives as a double-click; for a button it's just a press.
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => match CaptionButton::from_hit_code(wparam.0) {
                Some(button) => {
                    // Not passed to DefWindowProc: it would run its own tracking loop and draw
                    // classic caption buttons over React's.
                    input.update(|s| {
                        s.hovered = Some(button);
                        s.pressed = Some(button);
                    });
                    LRESULT(0)
                }
                None => forward(input.parent, message, wparam, lparam),
            },

            WM_NCLBUTTONUP => {
                let released = CaptionButton::from_hit_code(wparam.0);
                let pressed = input.inner.borrow().state.pressed;
                input.update(|s| s.pressed = None);
                if let Some(button) = released.filter(|b| Some(*b) == pressed) {
                    input.click(button, lparam);
                }
                LRESULT(0)
            }

            WM_NCRBUTTONDOWN | WM_NCRBUTTONUP | WM_NCRBUTTONDBLCLK => {
                if CaptionButton::from_hit_code(wparam.0).is_some() {
                    LRESULT(0)
                } else {
                    forward(input.parent, message, wparam, lparam)
                }
            }

            WM_SET_BUTTONS => {
                // SAFETY: `set_buttons` sent a pointer to a Vec that outlives this call.
                let buttons = unsafe { &*(lparam.0 as *const Vec<ButtonRect>) };
                input.inner.borrow_mut().buttons.clone_from(buttons);
                input.layout();
                LRESULT(0)
            }

            WM_SET_LISTENER => {
                // SAFETY: `set_listener` sent a pointer to an Option that outlives this call.
                let slot = unsafe { &mut *(lparam.0 as *mut Option<Listener>) };
                if let Some(listener) = slot.take() {
                    let mut inner = input.inner.borrow_mut();
                    inner.listener = Some(Rc::from(listener));
                    inner.sent = None;
                }
                input.state_changed();
                LRESULT(0)
            }

            WM_NOTIFY_STATE => {
                input.notify();
                LRESULT(0)
            }

            WM_ERASEBKGND => LRESULT(1),

            _ => default(),
        },
        default,
    )
}

/// Hands a caption-area mouse message to the top-level window, whose default handling starts the
/// move loop (with Aero Snap), maximizes on double-click or shows the system menu.
fn forward(parent: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: a synchronous message to the live parent on the same thread.
    unsafe { SendMessageW(parent, message, Some(wparam), Some(lparam)) }
}

unsafe extern "system" fn parent_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: the subclass chain is live while this procedure is installed.
    let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };
    // SAFETY: `data` is the `Input` from `install`; its WM_NCDESTROY removes this subclass
    // before freeing it, so it is alive here.
    let input = unsafe { &*(data as *const Input) };
    guarded(
        || {
            match message {
                // WM_DPICHANGED resizes the window (WM_SIZE follows), but a move between
                // monitors can change the scale without changing the size in pixels.
                WM_SIZE | WM_DPICHANGED => input.layout(),
                WM_ACTIVATE => {
                    let active = (wparam.0 & 0xFFFF) as u32 != WA_INACTIVE;
                    input.update(|s| s.active = active);
                }
                _ => {}
            }
            result
        },
        || result,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(button: CaptionButton, right: f64) -> ButtonRect {
        ButtonRect {
            button,
            right,
            top: 0.0,
            width: 46.0,
            height: 40.0,
        }
    }

    fn three() -> Vec<ButtonRect> {
        vec![
            rect(CaptionButton::Minimize, 92.0),
            rect(CaptionButton::Maximize, 46.0),
            rect(CaptionButton::Close, 0.0),
        ]
    }

    #[test]
    fn buttons_share_edges_at_fractional_scale() {
        // 125 % and 175 %: 46 CSS px are 57.5 and 80.5 device pixels.
        for dpi in [120, 168] {
            let placed = place(&three(), 1000, dpi);
            assert_eq!(placed[2].1.right, 1000);
            assert_eq!(placed[0].1.right, placed[1].1.left);
            assert_eq!(placed[1].1.right, placed[2].1.left);
        }
    }

    #[test]
    fn places_buttons_from_the_right_edge() {
        let placed = place(&[rect(CaptionButton::Close, 0.0)], 1280, 144);
        assert_eq!(
            placed[0].1,
            RECT {
                left: 1211,
                top: 0,
                right: 1280,
                bottom: 60
            }
        );
    }

    #[test]
    fn the_input_window_covers_all_buttons() {
        let placed = place(&three(), 1000, 96);
        assert_eq!(
            bounds(&placed),
            Some(RECT {
                left: 862,
                top: 0,
                right: 1000,
                bottom: 40
            })
        );
        assert_eq!(bounds(&[]), None);
    }

    #[test]
    fn lparam_points_can_be_negative() {
        // A monitor left of and above the primary one.
        let lparam = LPARAM(((-20i16 as u16 as isize) << 16) | (-1500i16 as u16 as isize));
        let point = point_from_lparam(lparam);
        assert_eq!((point.x, point.y), (-1500, -20));
    }

    #[test]
    fn hit_codes_round_trip() {
        for button in [
            CaptionButton::Minimize,
            CaptionButton::Maximize,
            CaptionButton::Close,
        ] {
            assert_eq!(
                CaptionButton::from_hit_code(button.hit_code() as usize),
                Some(button)
            );
        }
        assert_eq!(CaptionButton::from_hit_code(HTCAPTION as usize), None);
    }

    mod window {
        use std::cell::RefCell;
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        use windows::Win32::Graphics::Gdi::ClientToScreen;
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage, WS_OVERLAPPEDWINDOW,
        };

        use super::super::*;
        use super::three;

        thread_local! {
            /// The `WM_SYSCOMMAND`s the test window got, instead of running them: a hidden
            /// window would show itself to maximize.
            static COMMANDS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
        }

        /// A hidden top-level window with the styles Tauri's has (caption, sizebox, maxbox).
        fn top_level() -> HWND {
            unsafe extern "system" fn proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
                if m == WM_SYSCOMMAND {
                    COMMANDS.with_borrow_mut(|commands| commands.push(w.0 as u32 & 0xFFF0));
                    return LRESULT(0);
                }
                // SAFETY: forwarding the arguments Windows gave us.
                unsafe { DefWindowProcW(h, m, w, l) }
            }
            // SAFETY: test setup; the class name is static and the window is destroyed by the
            // test.
            unsafe {
                let instance = GetModuleHandleW(None).expect("module handle");
                let class = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(proc),
                    hInstance: instance.into(),
                    lpszClassName: w!("JerryTitleBarTest"),
                    ..Default::default()
                };
                RegisterClassExW(&class);
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("JerryTitleBarTest"),
                    w!("title bar test"),
                    WS_OVERLAPPEDWINDOW,
                    100,
                    100,
                    800,
                    600,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )
                .expect("test window")
            }
        }

        /// Runs this thread's pending messages.
        fn pump() {
            let mut msg = MSG::default();
            // SAFETY: a standard message pump on this thread's windows.
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }

        /// Pumps this thread's messages until the listener reports a state that matches `wanted`.
        /// Earlier states are skipped: the test window is off screen, so the mouse "leaves" the
        /// buttons right after every hover.
        fn wait_for(
            rx: &mpsc::Receiver<FrameState>,
            wanted: impl Fn(&FrameState) -> bool,
        ) -> FrameState {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                while let Ok(state) = rx.try_recv() {
                    if wanted(&state) {
                        return state;
                    }
                }
                assert!(Instant::now() < deadline, "no such state");
                pump();
            }
        }

        /// The system commands the test window got since the last call.
        fn commands() -> Vec<u32> {
            pump();
            COMMANDS.with_borrow_mut(std::mem::take)
        }

        fn pack(x: i32, y: i32) -> LPARAM {
            LPARAM((((y as u16) as isize) << 16) | (x as u16) as isize)
        }

        #[test]
        fn answers_hit_tests_reports_hover_and_clicks() {
            let parent = top_level();
            let hwnd = parent.0 as isize;
            install(hwnd).expect("install");
            install(hwnd).expect("a second install does nothing");
            set_buttons(hwnd, three()).expect("set buttons");
            let input = find(parent).expect("input window");

            let mut client = RECT::default();
            let mut origin = POINT::default();
            // SAFETY: queries on the live test window.
            let scale = unsafe {
                GetClientRect(parent, &mut client).expect("client rect");
                origin.x = client.right;
                assert!(ClientToScreen(parent, &mut origin).as_bool());
                f64::from(GetDpiForWindow(parent)) / 96.0
            };
            let at = |right_css: f64| {
                let x = origin.x - (right_css * scale) as i32;
                let y = origin.y + (20.0 * scale) as i32;
                pack(x, y)
            };
            let send = |message: u32, button: CaptionButton, centre: f64| {
                // SAFETY: a synchronous message to our live input window.
                unsafe {
                    SendMessageW(
                        input,
                        message,
                        Some(WPARAM(button.hit_code() as usize)),
                        Some(at(centre)),
                    )
                }
            };
            for (button, centre) in [
                (CaptionButton::Close, 23.0),
                (CaptionButton::Maximize, 69.0),
                (CaptionButton::Minimize, 115.0),
            ] {
                // SAFETY: as above.
                let code = unsafe { SendMessageW(input, WM_NCHITTEST, None, Some(at(centre))) };
                assert_eq!(code.0 as u32, button.hit_code(), "{button:?}");
            }
            // Left of the buttons is caption.
            // SAFETY: as above.
            let code = unsafe { SendMessageW(input, WM_NCHITTEST, None, Some(at(200.0))) };
            assert_eq!(code.0 as u32, HTCAPTION);

            let (tx, rx) = mpsc::channel();
            set_listener(
                hwnd,
                Box::new(move |state| {
                    let _ = tx.send(state);
                }),
            )
            .expect("listener");
            let first = wait_for(&rx, |_| true);
            assert_eq!(
                (first.hovered, first.pressed, first.maximized),
                (None, None, false)
            );

            send(WM_NCMOUSEMOVE, CaptionButton::Maximize, 69.0);
            wait_for(&rx, |s| s.hovered == Some(CaptionButton::Maximize));

            // Press and release on maximize: the window gets SC_MAXIMIZE, as from its own button.
            send(WM_NCLBUTTONDOWN, CaptionButton::Maximize, 69.0);
            wait_for(&rx, |s| s.pressed == Some(CaptionButton::Maximize));
            send(WM_NCLBUTTONUP, CaptionButton::Maximize, 69.0);
            wait_for(&rx, |s| s.pressed.is_none());
            assert_eq!(commands(), [SC_MAXIMIZE]);

            // Press on close, release on minimize: nothing happens.
            send(WM_NCLBUTTONDOWN, CaptionButton::Close, 23.0);
            send(WM_NCLBUTTONUP, CaptionButton::Minimize, 115.0);
            assert_eq!(commands(), Vec::<u32>::new());

            // The keyboard and screen readers press the buttons through `press`.
            press(hwnd, CaptionButton::Close).expect("press");
            press(hwnd, CaptionButton::Minimize).expect("press");
            assert_eq!(commands(), [SC_CLOSE, SC_MINIMIZE]);

            // SAFETY: destroying our own test window (and with it the input window).
            unsafe { DestroyWindow(parent) }.expect("destroy");
            assert!(find(parent).is_none());
        }

        #[test]
        fn a_window_without_the_input_window_reports_it() {
            let parent = top_level();
            let hwnd = parent.0 as isize;
            assert!(set_buttons(hwnd, three()).is_err());
            // SAFETY: destroying our own test window.
            unsafe { DestroyWindow(parent) }.expect("destroy");
        }
    }
}
