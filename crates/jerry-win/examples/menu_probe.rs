//! Spike #16 dev tool: Windows' context menu without a mouse or a keyboard.
//!
//! ```text
//! menu_probe dump [--prewarm[=MS]] [--rounds N] PATH...
//! menu_probe show [--no-forward] [--prewarm] [--dark] [--wait FILE] [CHOICE] PATH...
//! menu_probe drive --pid PID [--dwell MS] CHOICE|ESC
//! ```
//!
//! - `dump` builds the menu N times (default 2) and prints timings and top-level items;
//!   `MENU_TREE=1` also opens every submenu the way the menu loop does and prints the tree.
//! - `show` opens a small window of its own (not activated), shows the real menu with
//!   `jerry_win::context_menu::show` on its main thread and lets a driver thread pick CHOICE.
//!   `--no-forward` turns the subclass off, to see what breaks without it. `--prewarm` runs
//!   `context_menu::prewarm` on a background STA thread first, as Jerry does at startup.
//!   `--dark` lets the menu follow Windows' dark mode (`follow_system_theme`), as Jerry does.
//!   `--wait FILE` keeps the message loop running until FILE exists (handlers that finish on
//!   their own threads).
//! - `drive` picks CHOICE in a menu another process (Jerry) shows; `ESC` closes it unchosen.
//!
//! CHOICE is a `>`-separated item path such as `Send to>Compressed (zipped) folder`, matched
//! without `&` and case, by prefix. The driver never moves the mouse or presses keys: it finds
//! the menu windows (`#32768`) of the process, selects the item with `MN_SELECTITEM` and posts
//! `WM_KEYDOWN` (Right to open a submenu, Enter to choose) to the menu window, which only that
//! menu's loop reads.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::undocumented_unsafe_blocks
)]
// A throwaway dev tool: unwraps and terse unsafe blocks are fine here, not in Jerry.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use jerry_win::context_menu::{self, MenuItem, MenuOptions, ShellMenu, clean_text};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN, VK_RIGHT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, EnumWindows, GetClassNameW,
    GetMenuItemCount, GetMenuStringW, GetWindowThreadProcessId, HMENU, IsWindowVisible,
    MF_BYPOSITION, MN_GETHMENU, MSG, PM_REMOVE, PeekMessageW, PostMessageW, RegisterClassW,
    SW_SHOWNOACTIVATE, SendMessageW, ShowWindow, TranslateMessage, WM_KEYDOWN, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{BOOL, w};

/// Selects an item of a popup menu window by position (undocumented, stable since Windows 95;
/// accessibility tools use it).
const MN_SELECTITEM: u32 = 0x01E5;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        usage();
    };
    match command.as_str() {
        "dump" => dump(rest),
        "show" => show(rest),
        "drive" => drive_command(rest),
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: menu_probe dump PATH... | show [--no-forward] [--prewarm] [--dark] [--wait FILE] [CHOICE] PATH... | drive --pid PID [--dwell MS] CHOICE"
    );
    std::process::exit(2);
}

/// `dump [--prewarm[=MS]] [--rounds N] PATH...`: `--prewarm` first builds (and drops) a menu
/// for the system folder on a background STA thread, then waits MS (default 0) before the
/// main thread's rounds; every round prints its timings and top-level items.
fn dump(args: &[String]) {
    let mut prewarm = None;
    let mut rounds = 2;
    let mut paths = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if let Some(wait) = arg.strip_prefix("--prewarm") {
            prewarm = Some(wait.trim_start_matches('=').parse().unwrap_or(0));
        } else if arg == "--rounds" {
            rounds = iter.next().and_then(|n| n.parse().ok()).unwrap_or(2);
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    jerry_win::main_thread::init().unwrap();
    if let Some(wait) = prewarm {
        let start = Instant::now();
        thread::spawn(|| {
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
                );
            }
            let system = PathBuf::from(std::env::var("SystemRoot").unwrap()).join("notepad.exe");
            let menu = ShellMenu::new(0, &[system], false).unwrap();
            println!(
                "prewarm (background STA): QueryContextMenu {:.1} ms",
                ms(menu.timings.query)
            );
        })
        .join()
        .unwrap();
        pump(Duration::from_millis(wait));
        println!("prewarm done after {:.1} ms", ms(start.elapsed()));
    }
    for round in 1..=rounds {
        let start = Instant::now();
        let menu = match ShellMenu::new(0, &paths, false) {
            Ok(menu) => menu,
            Err(error) => {
                println!("round {round}: {error}");
                return;
            }
        };
        let built = start.elapsed();
        let t = &menu.timings;
        let items = menu.items();
        let top: Vec<&str> = items
            .iter()
            .filter(|item| !item.separator)
            .map(|item| item.text.as_str())
            .collect();
        println!(
            "round {round}: resolve {:.1} ms, GetUIObjectOf {:.1} ms, QueryContextMenu {:.1} ms, total {:.1} ms, {} items: {}",
            ms(t.resolve),
            ms(t.ui_object),
            ms(t.query),
            ms(built),
            top.len(),
            top.join(" | ")
        );
        if round == rounds && std::env::var_os("MENU_TREE").is_some() {
            print_items(&menu, &items, 1);
        }
    }
}

fn print_items(menu: &ShellMenu, items: &[MenuItem], depth: usize) {
    let indent = "  ".repeat(depth);
    for item in items {
        if item.separator {
            println!("{indent}----");
            continue;
        }
        let mut flags = Vec::new();
        if item.default {
            flags.push("default");
        }
        if item.disabled {
            flags.push("disabled");
        }
        if item.owner_draw {
            flags.push("owner-draw");
        }
        let verb = item
            .verb
            .as_deref()
            .map(|verb| format!(" verb={verb}"))
            .unwrap_or_default();
        let id = item
            .offset
            .map(|offset| format!(" #{offset}"))
            .unwrap_or_default();
        println!("{indent}{}{id}{verb} {flags:?}", item.text);
        if let Some(submenu) = &item.submenu {
            let before = submenu.items.len();
            let start = Instant::now();
            let opened = menu.open(submenu);
            println!(
                "{indent}  [submenu: {before} items before WM_INITMENUPOPUP, {} after, {:.1} ms]",
                opened.len(),
                ms(start.elapsed())
            );
            print_items(menu, &opened, depth + 1);
        }
    }
}

fn show(args: &[String]) {
    let mut forward = true;
    let mut prewarm = false;
    let mut dark = false;
    let mut wait = None;
    let mut rest = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--no-forward" => forward = false,
            "--prewarm" => prewarm = true,
            "--dark" => dark = true,
            "--wait" => wait = iter.next().map(PathBuf::from),
            _ => rest.push(arg.clone()),
        }
    }
    // The choice is the first argument that isn't an existing path.
    let (choice, paths): (Vec<String>, Vec<String>) =
        rest.into_iter().partition(|arg| !Path::new(arg).exists());
    let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let choice = choice.first().cloned();

    jerry_win::main_thread::init().unwrap();
    if prewarm {
        // As Jerry would at startup: on a background STA thread.
        let warm = thread::spawn(|| {
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
                );
            }
            context_menu::prewarm()
                .map(ms)
                .map_err(|error| error.to_string())
        });
        println!("prewarm: {:?} ms", warm.join().unwrap());
    }
    let owner = probe_window();
    if dark {
        context_menu::follow_system_theme(owner).unwrap();
    }
    let pid = unsafe { GetCurrentProcessId() };
    if let Some(choice) = choice.clone() {
        thread::spawn(move || {
            if let Err(error) = drive(pid, &choice, Duration::from_millis(200)) {
                println!("driver: {error}");
                cancel(pid);
            }
        });
    }
    let opened = Instant::now();
    let options = MenuOptions {
        forward,
        on_open: Some(Box::new(move || {
            println!("menu open after {:.1} ms", ms(opened.elapsed()));
        })),
        ..Default::default()
    };
    let (x, y) = context_menu::client_to_screen(owner, 40, 40);
    match context_menu::show(owner, &paths, x, y, options) {
        Ok(outcome) => {
            let t = &outcome.timings;
            println!(
                "resolve {:.1} ms, GetUIObjectOf {:.1} ms, QueryContextMenu {:.1} ms, menu {:.1} ms, invoke {:?} ms",
                ms(t.resolve),
                ms(t.ui_object),
                ms(t.query),
                ms(t.menu),
                t.invoke.map(ms)
            );
            for popup in &t.popups {
                println!(
                    "  WM_INITMENUPOPUP {:?}: {} -> {} items, {:.1} ms",
                    popup.text,
                    popup.items_before,
                    popup.items_after,
                    ms(popup.elapsed)
                );
            }
            println!("forwarded: {:?}", outcome.forwarded);
            println!("chosen: {:?}", outcome.chosen);
        }
        Err(error) => println!("error: {error}"),
    }
    if let Some(wait) = wait {
        let deadline = Instant::now() + Duration::from_secs(30);
        while found(&wait).is_none() && Instant::now() < deadline {
            pump(Duration::from_millis(50));
        }
        // Let the handler finish writing (and close its progress window).
        pump(Duration::from_millis(1500));
        println!("wait: {} -> {:?}", wait.display(), found(&wait));
    }
}

/// `wait` if it exists, or for `DIR\*.ext` the first file in DIR with that extension. std
/// handles paths past MAX_PATH itself.
fn found(wait: &Path) -> Option<PathBuf> {
    let name = wait.file_name()?.to_string_lossy().into_owned();
    let Some(extension) = name.strip_prefix("*.") else {
        return wait.exists().then(|| wait.to_path_buf());
    };
    std::fs::read_dir(wait.parent()?)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
        })
}

fn drive_command(args: &[String]) {
    let mut pid = None;
    let mut dwell = Duration::from_millis(300);
    let mut choice = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--pid" => pid = iter.next().and_then(|pid| pid.parse().ok()),
            "--dwell" => {
                dwell = Duration::from_millis(iter.next().and_then(|v| v.parse().ok()).unwrap())
            }
            _ => choice = Some(arg.clone()),
        }
    }
    let (Some(pid), Some(choice)) = (pid, choice) else {
        usage();
    };
    // `ESC`: wait for the menu, then close it without a choice.
    if choice == "ESC" {
        let deadline = Instant::now() + Duration::from_secs(15);
        while menu_windows(pid).is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        thread::sleep(dwell);
        cancel(pid);
        println!("cancelled");
        return;
    }
    match drive(pid, &choice, dwell) {
        Ok(()) => println!("drove {choice:?}"),
        Err(error) => {
            println!("driver: {error}");
            cancel(pid);
            std::process::exit(1);
        }
    }
}

/// Picks `choice` (`A>B>C`) in the menu `pid` shows, waiting up to 15 s for it to appear.
fn drive(pid: u32, choice: &str, dwell: Duration) -> Result<(), String> {
    let steps: Vec<&str> = choice.split('>').map(str::trim).collect();
    for (index, step) in steps.iter().enumerate() {
        let deadline = Instant::now() + Duration::from_secs(15);
        let (window, position) = loop {
            if let Some(found) = find_item(pid, step) {
                break found;
            }
            if Instant::now() > deadline {
                let seen: Vec<Vec<String>> = menu_windows(pid)
                    .into_iter()
                    .map(|window| item_texts(menu_of(window)))
                    .collect();
                return Err(format!("no item {step:?}; menus: {seen:?}"));
            }
            thread::sleep(Duration::from_millis(50));
        };
        thread::sleep(dwell);
        unsafe {
            SendMessageW(
                window,
                MN_SELECTITEM,
                Some(WPARAM(position as usize)),
                Some(LPARAM(0)),
            );
        }
        thread::sleep(Duration::from_millis(100));
        let key = if index + 1 == steps.len() {
            VK_RETURN
        } else {
            VK_RIGHT
        };
        unsafe { PostMessageW(Some(window), WM_KEYDOWN, WPARAM(key.0 as usize), LPARAM(0)) }
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Closes every menu `pid` shows (Esc, innermost first), so a failed drive doesn't leave the
/// process stuck in the menu loop.
fn cancel(pid: u32) {
    for _ in 0..10 {
        let Some(&window) = menu_windows(pid).first() else {
            return;
        };
        let _ = unsafe {
            PostMessageW(
                Some(window),
                WM_KEYDOWN,
                WPARAM(VK_ESCAPE.0 as usize),
                LPARAM(0),
            )
        };
        thread::sleep(Duration::from_millis(150));
    }
}

/// The visible menu window of `pid` with an item starting with `name`, and its position.
fn find_item(pid: u32, name: &str) -> Option<(HWND, u32)> {
    let wanted = name.to_lowercase();
    // The newest (topmost) menu window first.
    menu_windows(pid).into_iter().find_map(|window| {
        let texts = item_texts(menu_of(window));
        texts
            .iter()
            .position(|text| text.to_lowercase().starts_with(&wanted))
            .map(|position| (window, position as u32))
    })
}

fn menu_of(window: HWND) -> HMENU {
    let menu = unsafe { SendMessageW(window, MN_GETHMENU, None, None) };
    HMENU(menu.0 as *mut core::ffi::c_void)
}

fn item_texts(menu: HMENU) -> Vec<String> {
    let count = unsafe { GetMenuItemCount(Some(menu)) }.max(0) as u32;
    (0..count)
        .map(|position| {
            let mut buffer = [0u16; 512];
            let len =
                unsafe { GetMenuStringW(menu, position, Some(&mut buffer), MF_BYPOSITION) }.max(0);
            clean_text(&String::from_utf16_lossy(&buffer[..len as usize]))
        })
        .collect()
}

/// Visible `#32768` windows of `pid`, topmost first.
fn menu_windows(pid: u32) -> Vec<HWND> {
    struct Search {
        pid: u32,
        found: Vec<HWND>,
    }
    unsafe extern "system" fn each(window: HWND, data: LPARAM) -> BOOL {
        let search = unsafe { &mut *(data.0 as *mut Search) };
        let mut class = [0u16; 32];
        let len = unsafe { GetClassNameW(window, &mut class) }.max(0) as usize;
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(window, Some(&mut owner)) };
        if owner == search.pid
            && String::from_utf16_lossy(&class[..len]) == "#32768"
            && unsafe { IsWindowVisible(window) }.as_bool()
        {
            search.found.push(window);
        }
        BOOL(1)
    }
    let mut search = Search {
        pid,
        found: Vec::new(),
    };
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&raw mut search as isize)) };
    search.found
}

/// A small window that owns the menu, shown without taking the focus from the user's app.
fn probe_window() -> isize {
    unsafe extern "system" fn proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
    let instance = unsafe { GetModuleHandleW(None) }.unwrap();
    let class = w!("JerryMenuProbe");
    let info = WNDCLASSW {
        lpfnWndProc: Some(proc),
        hInstance: instance.into(),
        lpszClassName: class,
        ..Default::default()
    };
    unsafe { RegisterClassW(&info) };
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class,
            w!("Jerry menu probe"),
            WS_OVERLAPPEDWINDOW,
            40,
            40,
            320,
            200,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .unwrap();
    let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    pump(Duration::from_millis(100));
    hwnd.0 as isize
}

fn pump(duration: Duration) {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        let mut msg = MSG::default();
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
