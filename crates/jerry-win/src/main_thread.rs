//! Jerry's main STA thread: OLE, and [`run_on_main`] for the work that has to run there
//! (ADR-0002).
//!
//! Modal loops (a drag, Windows' context menu) and other main-thread COM calls (the clipboard)
//! start from an `async` command and reach the main thread through [`run_on_main`], which posts
//! them to Jerry's own message-only window. Tauri's `run_on_main_thread` would run them inside
//! tao's event callback, where tao holds back events sent from other threads (Rust → page
//! `emit`s such as transfer progress) until the loop ends.
//!
//! Dev builds also watch the main thread: whenever it was busy for longer than [`SLOW_TASK`]
//! (the window was frozen that long), a warning goes to the log.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use jerry_core::JerryError;
use jerry_core::job::run_job;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, HWND_MESSAGE, PostMessageW, RegisterClassW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WNDCLASSW,
};
use windows::core::w;

/// Main-thread work longer than this freezes the window noticeably; dev builds log it.
pub const SLOW_TASK: Duration = Duration::from_millis(50);

const WM_RUN_TASK: u32 = WM_APP + 1;

/// The message-only window, kept as an integer so other threads can post to it.
static WINDOW: OnceLock<isize> = OnceLock::new();

/// Initializes OLE on the calling thread, which must be Jerry's main (STA) thread, and creates
/// the window [`run_on_main`] posts to. Call it once, from Tauri's `setup`.
///
/// wry already made the thread an STA with `CoInitializeEx`, but OLE drag & drop, the clipboard
/// and some context menu handlers need `OleInitialize`. OLE stays initialized for the life of the
/// process.
pub fn init() -> Result<(), JerryError> {
    // SAFETY: no reserved pointer. A second call on the same thread only returns S_FALSE.
    unsafe { OleInitialize(None) }?;
    if WINDOW.get().is_none() {
        let hwnd = create_window()?;
        let _ = WINDOW.set(hwnd.0 as isize);
        #[cfg(debug_assertions)]
        watchdog::start(hwnd.0 as isize);
    }
    Ok(())
}

/// Runs `task` on the main thread, from a message to Jerry's own window: modal loops (a drag,
/// Windows' context menu) and main-thread COM calls go this way, never through Tauri's
/// `run_on_main_thread` (see the module docs).
///
/// Returns once the task is queued. A panic in the task is logged and fails only that task.
/// Fails if [`init`] hasn't run or the post fails.
pub fn run_on_main(
    name: &'static str,
    task: impl FnOnce() + Send + 'static,
) -> Result<(), JerryError> {
    let Some(&hwnd) = WINDOW.get() else {
        tracing::error!(task = name, "run_on_main before main_thread::init");
        return Err(JerryError::internal());
    };
    let task = Box::into_raw(Box::new(Task {
        name,
        run: Box::new(task),
    }));
    // SAFETY: posting to our own window; once posted, its procedure owns `task`.
    let posted = unsafe {
        PostMessageW(
            Some(window(hwnd)),
            WM_RUN_TASK,
            WPARAM(0),
            LPARAM(task as isize),
        )
    };
    if let Err(error) = posted {
        // SAFETY: the message wasn't posted, so the task is still ours to free.
        drop(unsafe { Box::from_raw(task) });
        tracing::error!(task = name, %error, "couldn't post a task to the main thread");
        return Err(error.into());
    }
    Ok(())
}

struct Task {
    name: &'static str,
    run: Box<dyn FnOnce() + Send>,
}

fn window(raw: isize) -> HWND {
    HWND(raw as *mut core::ffi::c_void)
}

fn create_window() -> Result<HWND, JerryError> {
    let class = w!("JerryMainThread");
    // SAFETY: the module handle of this process.
    let instance = unsafe { GetModuleHandleW(None) }?;
    let info = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance.into(),
        lpszClassName: class,
        ..Default::default()
    };
    // SAFETY: a valid class description. A repeated registration fails harmlessly.
    unsafe { RegisterClassW(&info) };
    // SAFETY: a message-only window of the registered class, owned by this thread.
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!(""),
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
    Ok(hwnd)
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_RUN_TASK => {
            // SAFETY: `run_on_main` posted a pointer from `Box::into_raw`, and each message is
            // delivered once, so the box is reclaimed exactly once.
            let task = unsafe { Box::from_raw(lparam.0 as *mut Task) };
            run_task(*task);
            LRESULT(0)
        }
        #[cfg(debug_assertions)]
        watchdog::WM_HEARTBEAT => {
            watchdog::beat(wparam);
            LRESULT(0)
        }
        // SAFETY: default handling for a message to our own window.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn run_task(Task { name, run }: Task) {
    let started = Instant::now();
    // Unwinding out of a window procedure would abort the process: the task fails alone.
    let _ = run_job(name, || {
        run();
        Ok(())
    });
    let ms = started.elapsed().as_millis() as u64;
    tracing::debug!(task = name, ms, "main-thread task done");
}

/// Dev builds: a thread that pings the main thread and logs every freeze over [`SLOW_TASK`].
#[cfg(debug_assertions)]
mod watchdog {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

    use super::{SLOW_TASK, window};

    pub const WM_HEARTBEAT: u32 = WM_APP + 2;
    const INTERVAL: Duration = Duration::from_millis(20);

    static EPOCH: OnceLock<Instant> = OnceLock::new();
    /// A heartbeat is posted and not handled yet: a busy main thread gets only one.
    static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

    pub fn start(hwnd: isize) {
        let spawned = std::thread::Builder::new()
            .name("jerry-main-watchdog".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(INTERVAL);
                    if IN_FLIGHT.swap(true, Ordering::AcqRel) {
                        continue;
                    }
                    let sent = WPARAM(elapsed().as_micros() as usize);
                    // SAFETY: posting a plain value to Jerry's own window.
                    let posted =
                        unsafe { PostMessageW(Some(window(hwnd)), WM_HEARTBEAT, sent, LPARAM(0)) };
                    if posted.is_err() {
                        return; // the window is gone
                    }
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "the main-thread watchdog didn't start");
        }
    }

    pub fn beat(sent: WPARAM) {
        let lag = elapsed().saturating_sub(Duration::from_micros(sent.0 as u64));
        IN_FLIGHT.store(false, Ordering::Release);
        if lag > SLOW_TASK {
            let ms = lag.as_millis() as u64;
            tracing::warn!(ms, "main thread was busy: the window was frozen");
        }
    }

    fn elapsed() -> Duration {
        EPOCH.get_or_init(Instant::now).elapsed()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use windows::Win32::UI::WindowsAndMessaging::PostQuitMessage;
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG};

    use super::*;

    // One test only: the window is process-wide, and it belongs to the thread that made it.
    #[test]
    fn tasks_run_on_the_pumping_thread_and_a_panic_fails_alone() {
        assert!(run_on_main("early", || {}).is_err(), "init hasn't run");
        init().expect("OLE and the window");

        let (sender, received) = mpsc::channel();
        let poster = std::thread::spawn(move || {
            run_on_main("panics", || panic!("deliberate panic")).expect("posted");
            run_on_main("reports", move || {
                sender
                    .send(std::thread::current().id())
                    .expect("receiver alive");
            })
            .expect("posted");
            // SAFETY: ends this thread's message loop below.
            run_on_main("quits", || unsafe { PostQuitMessage(0) }).expect("posted");
        });
        poster.join().expect("poster");

        let mut msg = MSG::default();
        // SAFETY: a plain message loop on the thread that owns the window.
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
            // SAFETY: a message just retrieved by GetMessageW.
            unsafe { DispatchMessageW(&msg) };
        }
        let ran_on = received.recv().expect("the task after the panic ran");
        assert_eq!(ran_on, std::thread::current().id());
    }
}
