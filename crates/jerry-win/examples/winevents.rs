//! Spike #14 dev tool: prints the system's WinEvents that could tell Jerry an OLE drag is under
//! way in another app (mouse capture, drag & drop, the Shell's drag image window).
//!
//! ```text
//! cargo run -p jerry-win --example winevents -- [SECONDS]
//! ```

#![allow(clippy::print_stdout, clippy::undocumented_unsafe_blocks)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::OnceLock;
use std::time::Instant;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_OBJECT_HIDE, EVENT_OBJECT_SHOW, EVENT_SYSTEM_CAPTUREEND,
    EVENT_SYSTEM_CAPTURESTART, EVENT_SYSTEM_DRAGDROPEND, EVENT_SYSTEM_DRAGDROPSTART, GetClassNameW,
    GetMessageW, GetWindowThreadProcessId, KillTimer, MSG, OBJID_WINDOW, PostQuitMessage, SetTimer,
    TranslateMessage, WINEVENT_OUTOFCONTEXT, WM_TIMER,
};

static START: OnceLock<Instant> = OnceLock::new();

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let mut class = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
    let class = String::from_utf16_lossy(&class[..len]);
    let name = match event {
        e if e == EVENT_SYSTEM_CAPTURESTART => "CAPTURESTART",
        e if e == EVENT_SYSTEM_CAPTUREEND => "CAPTUREEND",
        e if e == EVENT_SYSTEM_DRAGDROPSTART => "DRAGDROPSTART",
        e if e == EVENT_SYSTEM_DRAGDROPEND => "DRAGDROPEND",
        e if e == EVENT_OBJECT_SHOW => "SHOW",
        e if e == EVENT_OBJECT_HIDE => "HIDE",
        _ => "other",
    };
    if (event == EVENT_OBJECT_SHOW || event == EVENT_OBJECT_HIDE)
        && (object != OBJID_WINDOW.0 || class != "SysDragImage")
    {
        return;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    let ms = START.get().unwrap().elapsed().as_millis();
    println!("+{ms:>6} ms {name:<14} {class} pid {pid}");
}

fn main() {
    let seconds: u32 = std::env::args().nth(1).map_or(15, |s| s.parse().unwrap());
    START.set(Instant::now()).unwrap();
    let hooks = unsafe {
        [
            SetWinEventHook(
                EVENT_SYSTEM_CAPTURESTART,
                EVENT_SYSTEM_DRAGDROPEND,
                None,
                Some(on_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            ),
            SetWinEventHook(
                EVENT_OBJECT_SHOW,
                EVENT_OBJECT_HIDE,
                None,
                Some(on_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            ),
        ]
    };
    unsafe { SetTimer(None, 1, seconds * 1000, None) };
    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        if msg.message == WM_TIMER {
            unsafe {
                let _ = KillTimer(None, msg.wParam.0);
                PostQuitMessage(0);
            }
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    for hook in hooks {
        let _ = unsafe { UnhookWinEvent(hook) };
    }
}
