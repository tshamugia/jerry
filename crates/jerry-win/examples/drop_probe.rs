//! Spike #13 dev tool: a drop target that reports exactly what a drag delivers, plus an
//! optional driver that performs a real mouse drag out of a running Jerry into it.
//!
//! ```text
//! cargo run -p jerry-win --example drop_probe                   # just a drop target
//! cargo run -p jerry-win --example drop_probe -- drive X Y DPR [right] [esc] [quick] [shot=PATH]
//! cargo run -p jerry-win --example drop_probe -- click X Y DPR
//! ```
//!
//! `drive` presses at CSS point (X, Y) of Jerry's page (scaled by DPR from the window's client
//! origin), moves into the probe and releases. It checks the window under the cursor before
//! pressing and before releasing, and presses Esc instead of dropping anywhere unexpected.
//! #14 can reuse the target half to see what Outlook or a browser puts on their drags.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::undocumented_unsafe_blocks
)]
// A throwaway dev tool: unwraps and terse unsafe blocks are fine here, not in Jerry.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use jerry_win::drag::{format_names, hdrop_paths};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, POINTL, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, ClientToScreen, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IDataObject};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, IDropTarget, IDropTarget_Impl, OleInitialize, RegisterDragDrop,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP, MOUSE_EVENT_FLAGS,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, SendInput,
    VK_ESCAPE,
};
use windows::Win32::UI::Shell::{CLSID_DragDropHelper, IDropTargetHelper};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW,
    GA_ROOT, GetAncestor, GetCursorPos, GetMessageW, GetSystemMetrics, GetWindowRect,
    HWND_NOTOPMOST, HWND_TOPMOST, MSG, PostMessageW, PostQuitMessage, RegisterClassW,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetCursorPos, SetWindowPos, TranslateMessage, WM_CLOSE,
    WM_DESTROY, WNDCLASSW, WS_CAPTION, WS_EX_TOPMOST, WS_SYSMENU, WS_VISIBLE, WindowFromPoint,
};
use windows::core::{Ref, implement, w};

type Log = Rc<RefCell<Vec<String>>>;

#[implement(IDropTarget)]
struct Probe {
    hwnd: HWND,
    log: Log,
    started: Instant,
    /// Shows the source's drag image over the probe, as Explorer and browsers do.
    helper: IDropTargetHelper,
}

impl Probe {
    fn note(&self, line: String) {
        let line = format!("+{:>5} ms  {line}", self.started.elapsed().as_millis());
        println!("{line}");
        self.log.borrow_mut().push(line);
    }
}

fn point(pt: &POINTL) -> POINT {
    POINT { x: pt.x, y: pt.y }
}

impl IDropTarget_Impl for Probe_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let formats = data
            .ok()
            .map(|d| format_names(d).unwrap_or_default())
            .unwrap_or_default();
        self.note(format!("DragEnter keys={:#x} formats={formats:?}", keys.0));
        unsafe {
            *effect = DROPEFFECT(effect.read().0 & DROPEFFECT_COPY.0);
            let _ = self
                .helper
                .DragEnter(self.hwnd, data.ok().ok(), &point(pt), *effect);
        }
        Ok(())
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            *effect = DROPEFFECT(effect.read().0 & DROPEFFECT_COPY.0);
            let _ = self.helper.DragOver(&point(pt), *effect);
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.note("DragLeave".into());
        let _ = unsafe { self.helper.DragLeave() };
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let paths = data.ok().and_then(hdrop_paths);
        self.note(format!("Drop keys={:#x} CF_HDROP={paths:?}", keys.0));
        unsafe {
            *effect = DROPEFFECT(effect.read().0 & DROPEFFECT_COPY.0);
            let _ = self.helper.Drop(data.ok().ok(), &point(pt), *effect);
        }
        Ok(())
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_DESTROY {
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[derive(Default)]
struct Drive {
    css: (f64, f64),
    dpr: f64,
    right: bool,
    esc: bool,
    quick: bool,
    shot: Option<PathBuf>,
}

fn parse_drive(args: &[String]) -> Option<Drive> {
    if args.first().map(String::as_str) != Some("drive") {
        return None;
    }
    let mut drive = Drive {
        css: (args[1].parse().unwrap(), args[2].parse().unwrap()),
        dpr: args[3].parse().unwrap(),
        ..Default::default()
    };
    for flag in &args[4..] {
        match flag.as_str() {
            "right" => drive.right = true,
            "esc" => drive.esc = true,
            "quick" => drive.quick = true,
            other if other.starts_with("shot=") => drive.shot = Some(other[5..].into()),
            other => panic!("unknown flag {other}"),
        }
    }
    Some(drive)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let drive = parse_drive(&args);
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        OleInitialize(None).unwrap();
    }

    // `click X Y DPR`: a plain left click on Jerry's page, to check its state after a drag.
    if args.first().map(String::as_str) == Some("click") {
        let jerry = unsafe { FindWindowW(None, w!("Jerry")) }.expect("Jerry isn't running");
        let mut at = POINT::default();
        let _ = unsafe { ClientToScreen(jerry, &mut at) };
        let dpr: f64 = args[3].parse().unwrap();
        at.x += (args[1].parse::<f64>().unwrap() * dpr).round() as i32;
        at.y += (args[2].parse::<f64>().unwrap() * dpr).round() as i32;
        if root(at) != jerry.0 as isize {
            println!("aborted: {at:?} isn't over Jerry");
            return;
        }
        let mut saved = POINT::default();
        let _ = unsafe { GetCursorPos(&mut saved) };
        mouse(at, MOUSEEVENTF_MOVE);
        thread::sleep(Duration::from_millis(60));
        mouse(at, MOUSEEVENTF_LEFTDOWN);
        thread::sleep(Duration::from_millis(60));
        mouse(at, MOUSEEVENTF_LEFTUP);
        thread::sleep(Duration::from_millis(100));
        restore(saved);
        println!("clicked {at:?}");
        return;
    }

    let jerry = unsafe { FindWindowW(None, w!("Jerry")) }.ok();
    let mut jerry_rect = RECT::default();
    if let Some(jerry) = jerry {
        unsafe { GetWindowRect(jerry, &mut jerry_rect).unwrap() };
    }

    // The probe sits over the right part of Jerry's window (or at a fixed spot without Jerry).
    let (x, y, width, height) = if jerry.is_some() {
        (jerry_rect.right - 520, jerry_rect.top + 160, 440, 320)
    } else {
        (200, 200, 440, 320)
    };
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("JerryDropProbe"),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WS_EX_TOPMOST,
            w!("JerryDropProbe"),
            w!("Jerry drop probe"),
            WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .unwrap()
    };

    let log: Log = Rc::default();
    let helper: IDropTargetHelper =
        unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER) }.unwrap();
    let target: IDropTarget = Probe {
        hwnd,
        helper,
        log: log.clone(),
        started: Instant::now(),
    }
    .into();
    unsafe { RegisterDragDrop(hwnd, &target).unwrap() };

    let driver = drive.map(|drive| {
        let jerry = jerry.expect("Jerry isn't running");
        let probe = hwnd.0 as isize;
        let jerry_raw = jerry.0 as isize;
        let drop_at = POINT {
            x: x + width / 2,
            y: y + height / 2,
        };
        unsafe {
            SetWindowPos(
                jerry,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            )
            .unwrap();
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .unwrap();
        }
        thread::spawn(move || run_driver(drive, jerry_raw, probe, drop_at))
    });

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    if let Some(driver) = driver {
        println!("driver: {}", driver.join().unwrap());
    }
    if let Some(jerry) = jerry {
        unsafe {
            let _ = SetWindowPos(
                jerry,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
    println!("events: {}", log.borrow().len());
}

fn root(point: POINT) -> isize {
    unsafe { GetAncestor(WindowFromPoint(point), GA_ROOT) }.0 as isize
}

fn run_driver(drive: Drive, jerry: isize, probe: isize, drop_at: POINT) -> String {
    let jerry_hwnd = HWND(jerry as *mut _);
    let close = || unsafe {
        let _ = PostMessageW(Some(HWND(probe as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
    };
    thread::sleep(Duration::from_millis(400));

    let mut origin = POINT::default();
    let _ = unsafe { ClientToScreen(jerry_hwnd, &mut origin) };
    let press = POINT {
        x: origin.x + (drive.css.0 * drive.dpr).round() as i32,
        y: origin.y + (drive.css.1 * drive.dpr).round() as i32,
    };
    if root(press) != jerry {
        close();
        return format!("aborted: {press:?} isn't over Jerry");
    }
    let mut saved = POINT::default();
    let _ = unsafe { GetCursorPos(&mut saved) };

    let (down, up) = if drive.right {
        (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
    } else {
        (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
    };
    mouse(press, MOUSEEVENTF_MOVE);
    thread::sleep(Duration::from_millis(80));
    mouse(press, down);
    thread::sleep(Duration::from_millis(80));

    let steps = 30;
    let mut report = String::new();
    for step in 1..=steps {
        let t = f64::from(step) / f64::from(steps);
        let at = POINT {
            x: press.x + ((drop_at.x - press.x) as f64 * t) as i32,
            y: press.y + ((drop_at.y - press.y) as f64 * t) as i32,
        };
        mouse(at, MOUSEEVENTF_MOVE);
        thread::sleep(Duration::from_millis(16));
        if drive.quick && step == 3 {
            mouse(at, up);
            thread::sleep(Duration::from_millis(800));
            close();
            restore(saved);
            return "quick release after 3 moves".into();
        }
        if step == steps / 2 {
            let image = unsafe { FindWindowW(w!("SysDragImage"), None) }.is_ok();
            report.push_str(&format!("drag image window mid-way: {image}; "));
        }
    }
    thread::sleep(Duration::from_millis(300));
    if let Some(path) = &drive.shot {
        // Just before the drop, over the probe: the image has had time to render.
        capture(drop_at, path);
    }
    let over_probe = root(drop_at) == probe;
    if drive.esc || !over_probe {
        key_press(VK_ESCAPE.0);
        thread::sleep(Duration::from_millis(150));
        report.push_str(if over_probe {
            "cancelled with Esc; "
        } else {
            "NOT over probe, Esc; "
        });
    }
    mouse(drop_at, up);
    thread::sleep(Duration::from_millis(800));
    close();
    restore(saved);
    report
}

fn restore(saved: POINT) {
    let _ = unsafe { SetCursorPos(saved.x, saved.y) };
}

fn mouse(at: POINT, flags: MOUSE_EVENT_FLAGS) {
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: ((at.x - vx) * 65535 / (vw - 1)),
                dy: ((at.y - vy) * 65535 / (vh - 1)),
                dwFlags: flags | MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                ..Default::default()
            },
        },
    };
    unsafe { SendInput(&[input], size_of::<INPUT>() as i32) };
}

fn key_press(vk: u16) {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(vk),
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let inputs = [key(Default::default()), key(KEYEVENTF_KEYUP)];
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

/// Saves a 480×320 screenshot around `at` as a BMP, layered windows (the drag image) included.
fn capture(at: POINT, path: &PathBuf) {
    let (w, h) = (480, 320);
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap =
            CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
        let old = SelectObject(mem, bitmap.into());
        let _ = BitBlt(
            mem,
            0,
            0,
            w,
            h,
            Some(screen),
            at.x - w / 2,
            at.y - h / 2,
            SRCCOPY | CAPTUREBLT,
        );
        let size = (w * h * 4) as usize;
        let pixels = std::slice::from_raw_parts(bits as *const u8, size);
        let mut file = Vec::with_capacity(54 + size);
        file.extend_from_slice(b"BM");
        file.extend_from_slice(&((54 + size) as u32).to_le_bytes());
        file.extend_from_slice(&[0; 4]);
        file.extend_from_slice(&54u32.to_le_bytes());
        file.extend_from_slice(&40u32.to_le_bytes());
        file.extend_from_slice(&w.to_le_bytes());
        file.extend_from_slice(&(-h).to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&32u16.to_le_bytes());
        file.extend_from_slice(&[0; 24]);
        file.extend_from_slice(pixels);
        fs::write(path, file).unwrap();
        SelectObject(mem, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }
}
