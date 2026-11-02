//! Jerry's main STA thread: OLE initialization and a way to run modal loops on it.
//!
//! Copied from spike #14 (`jerry_win::drag` on `spike/14-ole-drag-in`), where it was found
//! that a modal loop started through Tauri's `run_on_main_thread` runs inside tao's event
//! callback, and tao holds back its own events (Rust → page `emit`s) until the callback ends.
//! [`run_on_main`] posts to a Jerry-owned window instead, so the loop runs outside any tao or
//! WebView2 callback.

use std::sync::OnceLock;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, HWND_MESSAGE, PostMessageW, RegisterClassW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WNDCLASSW,
};

/// Initializes OLE on the calling thread, which must be Jerry's main (STA) thread, and creates
/// the window [`run_on_main`] posts to.
///
/// wry already made the thread an STA with `CoInitializeEx`, but OLE drag & drop, the
/// clipboard and some context menu handlers need `OleInitialize`. Never uninitialized: OLE
/// lives as long as the process.
pub fn init() -> windows::core::Result<()> {
    // SAFETY: no reserved pointer; a second call on the same thread just returns S_FALSE.
    unsafe { OleInitialize(None) }?;
    create_task_window()
}

/// A task for [`run_on_main`].
pub type Task = Box<dyn FnOnce() + Send>;

/// The message-only window that runs [`run_on_main`]'s tasks (an HWND, kept as an integer so
/// it can be shared with other threads).
static TASKS: OnceLock<isize> = OnceLock::new();
const WM_RUN_TASK: u32 = WM_APP + 0x16;

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
/// `run_on_main_thread` (see the module docs).
///
/// Gives the task back if [`init`] hasn't run or the post fails.
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
