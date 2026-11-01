//! Spike #11: launches jerry.exe again and again and measures its startup from the outside.
//!
//! ```text
//! cargo run -p jerry-win --release --example startup_probe -- <jerry.exe> <out.jsonl> [options]
//!   --runs N        recorded runs (default 5)
//!   --warmup N      unrecorded runs first (default 1; 0 for cold runs)
//!   --label L       label stored with each run
//!   --profile P     WebView2 user data folder: "warm" (kept between runs) or "fresh" (new each run)
//!   --env K=V       extra environment for jerry.exe (repeatable)
//!   --idle S        before each run, wait until the user has been idle S seconds (default 3)
//!   --gap S         at least S seconds between runs (default 0); 20 lets a hybrid laptop's
//!                   discrete GPU fall asleep again, as it is when a user starts Jerry
//!   --cpu P         before each run, wait until system CPU use is below P % (default 20)
//!   --high-qos      opt jerry.exe and each WebView2 process out of power throttling (EcoQoS) as
//!                   soon as it appears: what Windows gives a foreground app
//!   --no-screen     don't sample the screen (GDI reads of the desktop slow DWM down a little)
//!   --dgpu ID       device instance id of a discrete GPU: its power state (D0 awake, D3 asleep)
//!                   is recorded right before each launch
//!   --purge DIR     before each run, have the elevated helper (scripts/spike-11-purge.ps1)
//!                   empty the standby list: the file cache goes cold, as after a reboot
//! ```
//!
//! Each run appends one JSON line: launch time, when the window appeared and was shown, what the
//! screen showed at the window (sampled about every millisecond) and the app's own report
//! (`JERRY_SPIKE_REPORT`). All times are ms since the Unix epoch. `scripts/spike-11-summary.mjs`
//! turns the file into tables.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CR_SUCCESS,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_PowerData, DEVPROPTYPE};
use windows::Win32::Foundation::{FILETIME, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    GetSystemTimes, OpenProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    PROCESS_SET_INFORMATION, ProcessPowerThrottling, SetProcessInformation,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible,
};
use windows::core::{BOOL, PCWSTR};

type Error = Box<dyn std::error::Error>;

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64() * 1000.0)
}

struct Args {
    exe: PathBuf,
    out: PathBuf,
    runs: u32,
    warmup: u32,
    label: String,
    fresh_profile: bool,
    env: Vec<(String, String)>,
    idle_s: u32,
    cpu_pct: f64,
    purge: Option<PathBuf>,
    screen: bool,
    high_qos: bool,
    gap_s: u64,
    dgpu: Option<String>,
}

fn parse_args() -> Result<Args, Error> {
    let mut args = std::env::args().skip(1);
    let exe = PathBuf::from(
        args.next()
            .ok_or("usage: startup_probe <jerry.exe> <out.jsonl>")?,
    );
    let out = PathBuf::from(args.next().ok_or("missing <out.jsonl>")?);
    let mut parsed = Args {
        exe,
        out,
        runs: 5,
        warmup: 1,
        label: "run".into(),
        fresh_profile: false,
        env: Vec::new(),
        idle_s: 3,
        cpu_pct: 20.0,
        purge: None,
        screen: true,
        high_qos: false,
        gap_s: 0,
        dgpu: None,
    };
    while let Some(flag) = args.next() {
        if flag == "--no-screen" {
            parsed.screen = false;
            continue;
        }
        if flag == "--high-qos" {
            parsed.high_qos = true;
            continue;
        }
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--runs" => parsed.runs = value.parse()?,
            "--warmup" => parsed.warmup = value.parse()?,
            "--label" => parsed.label = value,
            "--profile" => parsed.fresh_profile = value == "fresh",
            "--env" => {
                let (key, val) = value.split_once('=').ok_or("--env needs K=V")?;
                parsed.env.push((key.into(), val.into()));
            }
            "--idle" => parsed.idle_s = value.parse()?,
            "--cpu" => parsed.cpu_pct = value.parse()?,
            "--gap" => parsed.gap_s = value.parse()?,
            "--dgpu" => parsed.dgpu = Some(value),
            "--purge" => parsed.purge = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown option {flag}").into()),
        }
    }
    Ok(parsed)
}

/// Seconds since the last keyboard or mouse input.
fn user_idle_s() -> f64 {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `info` is a LASTINPUTINFO with cbSize set.
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return f64::MAX;
    }
    // SAFETY: no arguments.
    let now = unsafe { GetTickCount() };
    f64::from(now.wrapping_sub(info.dwTime)) / 1000.0
}

fn ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// System-wide CPU use over `window`, in percent.
fn cpu_busy_pct(window: Duration) -> f64 {
    let sample = || {
        let (mut idle, mut kernel, mut user) = Default::default();
        // SAFETY: three valid FILETIME out-pointers.
        let ok = unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) };
        ok.is_ok()
            .then(|| (ticks(idle), ticks(kernel) + ticks(user)))
    };
    let Some((idle0, total0)) = sample() else {
        return 0.0;
    };
    std::thread::sleep(window);
    let Some((idle1, total1)) = sample() else {
        return 0.0;
    };
    let total = total1.saturating_sub(total0).max(1) as f64;
    100.0 * (1.0 - idle1.saturating_sub(idle0) as f64 / total)
}

/// Waits (up to 3 minutes) until the user is idle and the CPU is calm. Returns the CPU use seen
/// last and how long it waited.
fn wait_for_quiet(args: &Args) -> (f64, f64) {
    let started = Instant::now();
    loop {
        let idle_ok = user_idle_s() >= f64::from(args.idle_s);
        let cpu = cpu_busy_pct(Duration::from_millis(700));
        if (idle_ok && cpu < args.cpu_pct) || started.elapsed() > Duration::from_secs(180) {
            return (cpu, started.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

/// Pids of processes whose parent is in `roots`, transitively (the roots may be gone already).
fn descendants(roots: &[u32]) -> Vec<u32> {
    // SAFETY: plain snapshot; the handle is closed below.
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: `entry` has dwSize set.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while more {
        pairs.push((entry.th32ProcessID, entry.th32ParentProcessID));
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    // SAFETY: the snapshot handle is ours and closed once.
    let _ = unsafe { windows::Win32::Foundation::CloseHandle(snapshot) };
    let mut found: Vec<u32> = roots.to_vec();
    let mut next = 0;
    while next < found.len() {
        let parent = found[next];
        for &(pid, ppid) in &pairs {
            if ppid == parent && pid != parent && !found.contains(&pid) {
                found.push(pid);
            }
        }
        next += 1;
    }
    found.retain(|pid| !roots.contains(pid));
    found
}

/// A device's current power state: 0 for D0 (on) up to 3 for D3 (off), from CM_POWER_DATA.
fn device_power_state(instance: &str) -> Option<i32> {
    let id: Vec<u16> = instance.encode_utf16().chain(Some(0)).collect();
    let mut node = 0u32;
    // SAFETY: `id` is NUL-terminated; `node` is a valid out-pointer.
    if unsafe { CM_Locate_DevNodeW(&mut node, PCWSTR(id.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) }
        != CR_SUCCESS
    {
        return None;
    }
    let mut kind = DEVPROPTYPE(0);
    let mut data = [0u8; 64];
    let mut size = data.len() as u32;
    // SAFETY: `data` holds `size` writable bytes; the other pointers are valid.
    let status = unsafe {
        CM_Get_DevNode_PropertyW(
            node,
            &DEVPKEY_Device_PowerData,
            &mut kind,
            Some(data.as_mut_ptr()),
            &mut size,
            0,
        )
    };
    (status == CR_SUCCESS && size >= 8)
        .then(|| i32::from_le_bytes([data[4], data[5], data[6], data[7]]) - 1)
}

/// Turns power throttling (EcoQoS) off for `pid`, as Windows does for a foreground app.
fn high_qos(pid: u32) -> bool {
    // SAFETY: plain open by pid; the handle is closed below.
    let Ok(process) = (unsafe { OpenProcess(PROCESS_SET_INFORMATION, false, pid) }) else {
        return false;
    };
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    // SAFETY: `state` is a PROCESS_POWER_THROTTLING_STATE and the size matches it.
    let ok = unsafe {
        SetProcessInformation(
            process,
            ProcessPowerThrottling,
            (&raw const state).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .is_ok();
    // SAFETY: the handle is ours and closed once.
    let _ = unsafe { windows::Win32::Foundation::CloseHandle(process) };
    ok
}

/// Waits until every process the last Jerry started (WebView2) is gone, so the next run starts
/// its own browser process instead of reusing a dying one.
fn wait_for_tree_exit(pid: u32) {
    let started = Instant::now();
    while !descendants(&[pid]).is_empty() && started.elapsed() < Duration::from_secs(15) {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
}

struct Find {
    pid: u32,
    hwnd: Option<HWND>,
}

unsafe extern "system" fn find_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` is the `Find` passed to EnumWindows below, alive for the whole call.
    let find = unsafe { &mut *(lparam.0 as *mut Find) };
    let mut pid = 0;
    // SAFETY: `hwnd` comes from EnumWindows; `pid` is a valid out-pointer.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == find.pid {
        let mut title = [0u16; 16];
        // SAFETY: `title` is a writable buffer of the given length.
        let len = unsafe { GetWindowTextW(hwnd, &mut title) };
        if String::from_utf16_lossy(&title[..len.max(0) as usize]) == "Jerry" {
            find.hwnd = Some(hwnd);
            return BOOL(0);
        }
    }
    BOOL(1)
}

fn jerry_window(pid: u32) -> Option<HWND> {
    let mut find = Find { pid, hwnd: None };
    // SAFETY: `find` outlives the call; the callback only reads it through `lparam`. EnumWindows
    // reports an error when the callback stops early, which is how a match ends it.
    let _ = unsafe { EnumWindows(Some(find_window), LPARAM((&raw mut find) as isize)) };
    find.hwnd
}

/// Reads screen pixels (as `0xRRGGBB`) with GDI from the composed desktop.
struct Screen {
    screen: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    width: i32,
    height: i32,
}

impl Screen {
    fn new(width: i32, height: i32) -> Self {
        // SAFETY: plain GDI object creation; everything is released in Drop.
        unsafe {
            let screen = GetDC(None);
            let memory = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            SelectObject(memory, bitmap.into());
            Screen {
                screen,
                memory,
                bitmap,
                width,
                height,
            }
        }
    }

    fn capture(&self, x: i32, y: i32, width: i32, height: i32) -> Vec<u32> {
        let (width, height) = (width.min(self.width), height.min(self.height));
        let mut pixels = vec![0u32; (width * height) as usize];
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: the DCs and bitmap are valid; `pixels` holds width * height 32-bit pixels,
        // matching the top-down 32 bpp BITMAPINFO.
        unsafe {
            if BitBlt(
                self.memory,
                0,
                0,
                width,
                height,
                Some(self.screen),
                x,
                y,
                SRCCOPY,
            )
            .is_err()
            {
                return Vec::new();
            }
            GetDIBits(
                self.memory,
                self.bitmap,
                0,
                height as u32,
                Some(pixels.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            );
        }
        for pixel in &mut pixels {
            *pixel &= 0x00ff_ffff;
        }
        pixels
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        // SAFETY: these objects were created in `new` and are released once.
        unsafe {
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.memory);
            ReleaseDC(None, self.screen);
        }
    }
}

/// Whether two `0xRRGGBB` colors differ by at most `tolerance` per channel.
fn near(a: u32, b: u32, tolerance: u32) -> bool {
    (0..3).all(|i| ((a >> (8 * i)) & 0xff).abs_diff((b >> (8 * i)) & 0xff) <= tolerance)
}

fn luma(pixel: u32) -> i32 {
    let (r, g, b) = ((pixel >> 16) & 0xff, (pixel >> 8) & 0xff, pixel & 0xff);
    ((r * 299 + g * 587 + b * 114) / 1000) as i32
}

/// Pixels that differ clearly from the most common color, if that color is the list's background
/// (`--jerry-pane`): rows drawn by Jerry, not whatever showed through before the first paint.
fn text_pixels(pixels: &[u32], pane: u32) -> usize {
    let mut counts = std::collections::HashMap::new();
    for &pixel in pixels {
        *counts.entry(pixel).or_insert(0usize) += 1;
    }
    let Some((&background, _)) = counts.iter().max_by_key(|(_, count)| **count) else {
        return 0;
    };
    if !near(background, pane, 2) {
        return 0;
    }
    let base = luma(background);
    pixels
        .iter()
        .filter(|&&pixel| (luma(pixel) - base).abs() > 48)
        .count()
}

/// What the probe saw during one run.
#[derive(Default)]
struct Seen {
    window_found: Option<f64>,
    visible: Option<f64>,
    /// When the sampled background patch changed color: (time, 0xRRGGBB).
    patch: Vec<(f64, u32)>,
    /// First time the first rows' area showed text, and how many text pixels.
    text: Option<(f64, usize)>,
    samples: u32,
    sample_gap_max_ms: f64,
    /// When the window became the foreground window, if it did.
    foreground: Option<f64>,
    /// Processes switched to high QoS (--high-qos).
    boosted: usize,
}

/// Samples the window until the process exits (or 90 s pass).
fn probe(child: &mut Child, screen: Option<&Screen>, theme: (u32, u32), boost: bool) -> Seen {
    let (mica, pane) = theme;
    let mut seen = Seen::default();
    let mut boosted: Vec<u32> = Vec::new();
    if boost && high_qos(child.id()) {
        boosted.push(child.id());
    }
    let started = Instant::now();
    let mut hwnd = None;
    let mut last_sample: Option<f64> = None;
    let mut text_candidate: Option<(f64, usize)> = None;
    loop {
        if matches!(child.try_wait(), Ok(Some(_))) || started.elapsed() > Duration::from_secs(90) {
            break;
        }
        if boost && started.elapsed() < Duration::from_secs(3) {
            for pid in descendants(&[child.id()]) {
                if !boosted.contains(&pid) && high_qos(pid) {
                    boosted.push(pid);
                }
            }
            seen.boosted = boosted.len();
        }
        if hwnd.is_none() {
            hwnd = jerry_window(child.id());
            if hwnd.is_some() {
                seen.window_found = Some(now_ms());
            }
        }
        let Some(window) = hwnd else {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        };
        // SAFETY: `window` is a window handle; a stale one just reports not visible.
        if !unsafe { IsWindowVisible(window) }.as_bool() {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        let at = now_ms();
        if seen.visible.is_none() {
            seen.visible = Some(at);
        }
        // SAFETY: no arguments.
        if seen.foreground.is_none() && unsafe { GetForegroundWindow() } == window {
            seen.foreground = Some(at);
        }
        let Some(screen) = screen else {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        };
        let mut rect = RECT::default();
        // SAFETY: `rect` is a valid out-pointer.
        if unsafe { GetWindowRect(window, &mut rect) }.is_err() {
            continue;
        }
        // SAFETY: `window` is a window handle.
        let scale = f64::from(unsafe { GetDpiForWindow(window) }.max(96)) / 96.0;
        let px = |css: f64| (css * scale).round() as i32;

        // Title bar, right side: plain mica once the UI paints.
        let patch = screen.capture(rect.right - px(200.0), rect.top + px(12.0), 4, 4);
        // First three rows' names (title bar 40 + column header 28, rows 30; CSS px).
        let strip = screen.capture(
            rect.left + px(12.0),
            rect.top + px(68.0),
            px(300.0),
            px(90.0),
        );
        let done = now_ms();
        if let Some(&color) = patch.first()
            && seen.patch.last().is_none_or(|&(_, last)| last != color)
        {
            seen.patch.push((done, color));
        }
        // Rows count as on screen once the title bar shows plain mica, the rows area the list
        // background with text, twice in a row (the open animation blends in what is behind).
        if seen.text.is_none() {
            let settled = patch.first().is_some_and(|&color| near(color, mica, 2));
            let count = if settled {
                text_pixels(&strip, pane)
            } else {
                0
            };
            if count > 100 {
                if let Some((first, _)) = text_candidate {
                    seen.text = Some((first, count));
                } else {
                    text_candidate = Some((done, count));
                }
            } else {
                text_candidate = None;
            }
        }
        seen.samples += 1;
        if let Some(last) = last_sample {
            seen.sample_gap_max_ms = seen.sample_gap_max_ms.max(done - last);
        }
        last_sample = Some(done);
        std::thread::sleep(Duration::from_millis(1));
    }
    seen
}

fn opt(value: Option<f64>) -> String {
    value.map_or("null".into(), |v| format!("{v:.3}"))
}

/// Asks the elevated helper to empty the standby list and waits for it.
fn purge(dir: &Path) -> Result<f64, Error> {
    let request = dir.join("request");
    let done = dir.join("done");
    let _ = std::fs::remove_file(&done);
    let started = Instant::now();
    std::fs::write(&request, b"purge")?;
    while !done.exists() {
        if started.elapsed() > Duration::from_secs(60) {
            return Err(
                "the purge helper did not answer (is scripts/spike-11-purge.ps1 running \
                        as administrator?)"
                    .into(),
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let answer = std::fs::read_to_string(&done)?;
    let _ = std::fs::remove_file(&done);
    if !answer.trim().starts_with("ok") {
        return Err(format!("purge failed: {answer}").into());
    }
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}

// A command-line tool: progress goes to stderr.
#[allow(clippy::print_stderr)]
fn main() -> Result<(), Error> {
    let args = parse_args()?;
    // SAFETY: plain process setting, before any window or DC is used.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let work = std::env::temp_dir().join("jerry-spike-11");
    std::fs::create_dir_all(&work)?;
    let profile = work.join(if args.fresh_profile {
        "webview-fresh"
    } else {
        "webview-warm"
    });
    let screen = args.screen.then(|| Screen::new(1200, 400));
    // --jerry-mica and --jerry-pane in design/tokens.css.
    let theme = if jerry_win::apps_use_dark_theme() {
        (0x202020, 0x1c1c1c)
    } else {
        (0xedeff2, 0xffffff)
    };
    let mut out = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&args.out)?;

    let mut last_exit: Option<Instant> = None;
    for run in 0..args.warmup + args.runs {
        let recorded = run >= args.warmup;
        if let Some(exit) = last_exit {
            let gap = Duration::from_secs(args.gap_s);
            if let Some(left) = gap.checked_sub(exit.elapsed()) {
                std::thread::sleep(left);
            }
        }
        if args.fresh_profile && profile.exists() {
            std::fs::remove_dir_all(&profile)?;
        }
        let (cpu, waited_ms) = wait_for_quiet(&args);
        let purge_ms = match &args.purge {
            Some(dir) => Some(purge(dir)?),
            None => None,
        };
        let dgpu = args.dgpu.as_deref().and_then(device_power_state);
        let report = work.join(format!("report-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&report);

        let mut command = Command::new(&args.exe);
        command
            .env("JERRY_SPIKE_REPORT", &report)
            .env("JERRY_SPIKE_EXIT", "400")
            .env("JERRY_SPIKE_TOPMOST", "1")
            .env("WEBVIEW2_USER_DATA_FOLDER", &profile);
        for (key, value) in &args.env {
            command.env(key, value);
        }
        let launched = now_ms();
        let mut child = command.spawn()?;
        let spawned = now_ms();
        let seen = probe(&mut child, screen.as_ref(), theme, args.high_qos);
        let exit = child.wait()?;
        let exited = now_ms();
        let pid = child.id();
        wait_for_tree_exit(pid);
        last_exit = Some(Instant::now());
        let app_report = std::fs::read_to_string(&report).unwrap_or_else(|_| "null".into());

        let patch: Vec<String> = seen
            .patch
            .iter()
            .map(|(at, color)| format!("[{at:.3},\"#{color:06x}\"]"))
            .collect();
        let line = format!(
            "{{\"label\":{label:?},\"run\":{run},\"recorded\":{recorded},\"profile\":{profile:?},\
             \"screen\":{screen},\"highQos\":{qos},\"boosted\":{boosted},\"foreground\":{fg},\"gapS\":{gap_s},\"dgpuState\":{dgpu},\"cpuBefore\":{cpu:.1},\"waitedMs\":{waited_ms:.0},\"purgeMs\":{purge},\
             \"launched\":{launched:.3},\"spawned\":{spawned:.3},\"windowFound\":{found},\
             \"visible\":{visible},\"patch\":[{patch}],\"text\":{text},\"textPixels\":{pixels},\
             \"samples\":{samples},\"sampleGapMaxMs\":{gap:.1},\"exited\":{exited:.3},\
             \"exitCode\":{code},\"report\":{app_report}}}",
            label = args.label,
            screen = args.screen,
            gap_s = args.gap_s,
            dgpu = dgpu.map_or("null".into(), |d| format!("\"D{d}\"")),
            qos = args.high_qos,
            boosted = seen.boosted,
            fg = opt(seen.foreground),
            profile = if args.fresh_profile { "fresh" } else { "warm" },
            purge = opt(purge_ms),
            found = opt(seen.window_found),
            visible = opt(seen.visible),
            patch = patch.join(","),
            text = opt(seen.text.map(|(at, _)| at)),
            pixels = seen.text.map_or(0, |(_, count)| count),
            samples = seen.samples,
            gap = seen.sample_gap_max_ms,
            code = exit.code().unwrap_or(-1),
        );
        writeln!(out, "{line}")?;
        let rel = |at: Option<f64>| at.map_or("-".into(), |at| format!("{:.0}", at - launched));
        eprintln!(
            "{} run {run}{}: window {} visible {} text {} exit {} (cpu {cpu:.0}%, waited {:.1} s)",
            args.label,
            if recorded { "" } else { " (warm-up)" },
            rel(seen.window_found),
            rel(seen.visible),
            rel(seen.text.map(|(at, _)| at)),
            rel(Some(exited)),
            waited_ms / 1000.0,
        );
    }
    Ok(())
}
