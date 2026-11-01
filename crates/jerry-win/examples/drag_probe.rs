//! Spike #14 dev tool: drags into a running Jerry with real mouse input, from a small source
//! window of its own, and reports what came back.
//!
//! ```text
//! cargo run -p jerry-win --example drag_probe -- SOURCE WAYPOINT... [right] [esc] [shot=DIR]
//! ```
//!
//! SOURCE is what the probe drags:
//! - `files`: real files (Georgian names) through the Shell's data object, like Explorer.
//! - `stream`, `hglobal`, `storage`: virtual files (`FileGroupDescriptorW` + `FileContents`)
//!   in that medium, like Outlook attachments (`storage` is how Outlook drags a whole email).
//! - `async`: virtual files as streams from a data object with `IDataObjectAsyncCapability`.
//!
//! A WAYPOINT is `KEY[@MS][+ctrl][+shift][+alt]`: the center of the drop target KEY from Jerry's
//! layout log line (`tab:1`, `side:3`, `crumb:0`, `row:5`, `pane`, `side:network`), how long
//! to rest there (default 300 ms) and the modifiers to hold. The drop happens at the last one,
//! and only if it is over Jerry (otherwise the probe presses Esc). `shot=DIR` saves a
//! screenshot of Jerry's window at every waypoint.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::undocumented_unsafe_blocks
)]
// A throwaway dev tool: unwraps and terse unsafe blocks are fine here, not in Jerry.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    DV_E_FORMATETC, E_NOTIMPL, HGLOBAL, HWND, LPARAM, LRESULT, OLE_E_ADVISENOTSUPPORTED, POINT,
    RECT, S_OK, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, ClientToScreen, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::System::Com::StructuredStorage::{
    CreateILockBytesOnHGlobal, IStorage, StgCreateDocfileOnILockBytes,
};
use windows::Win32::System::Com::{
    DATADIR_GET, DVASPECT_CONTENT, FORMATETC, IAdviseSink, IBindCtx, IDataObject, IDataObject_Impl,
    IEnumFORMATETC, IEnumSTATDATA, ISequentialStream, STGC_DEFAULT, STGM_CREATE, STGM_READWRITE,
    STGM_SHARE_EXCLUSIVE, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL, TYMED_ISTORAGE, TYMED_ISTREAM,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE, OleInitialize,
    ReleaseStgMedium,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEINPUT, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::Shell::{
    CFSTR_FILECONTENTS, CFSTR_FILEDESCRIPTORW, CFSTR_PERFORMEDDROPEFFECT, FD_FILESIZE,
    FD_PROGRESSUI, FILEDESCRIPTORW, IDataObjectAsyncCapability, IDataObjectAsyncCapability_Impl,
    SHCreateMemStream, SHCreateStdEnumFmtEtc,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GA_ROOT, GetAncestor, GetCursorPos, GetMessageW, GetSystemMetrics, GetWindowRect,
    HWND_NOTOPMOST, HWND_TOPMOST, KillTimer, MSG, PostMessageW, PostQuitMessage, RegisterClassW,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetCursorPos, SetTimer, SetWindowPos, TranslateMessage,
    WM_APP, WM_DESTROY, WM_LBUTTONDOWN, WM_RBUTTONDOWN, WM_TIMER, WNDCLASSW, WS_CAPTION,
    WS_EX_TOPMOST, WS_VISIBLE, WindowFromPoint,
};
use windows::core::{BOOL, HRESULT, PCWSTR, Ref, implement, w};

const WM_CLOSE_LATER: u32 = WM_APP + 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Medium {
    Stream,
    HGlobal,
    Storage,
}

#[derive(Clone, Debug)]
struct Waypoint {
    key: String,
    rest: Duration,
    ctrl: bool,
    shift: bool,
    alt: bool,
}

fn parse_waypoint(text: &str) -> Waypoint {
    let mut parts = text.split('+');
    let head = parts.next().unwrap();
    let (key, rest) = match head.split_once('@') {
        Some((key, ms)) => (key, Duration::from_millis(ms.parse().unwrap())),
        None => (head, Duration::from_millis(300)),
    };
    let mut point = Waypoint {
        key: key.to_owned(),
        rest,
        ctrl: false,
        shift: false,
        alt: false,
    };
    for modifier in parts {
        match modifier {
            "ctrl" => point.ctrl = true,
            "shift" => point.shift = true,
            "alt" => point.alt = true,
            other => panic!("unknown modifier {other}"),
        }
    }
    point
}

/// Jerry's last `layout` log line: element key → client rect (CSS px), and the DPR.
fn read_layout() -> (f64, HashMap<String, [f64; 4]>) {
    let log = fs::read_to_string(std::env::temp_dir().join("jerry-spike-14.log")).unwrap();
    let line = log
        .lines()
        .rev()
        .find(|l| l.contains(" layout dpr="))
        .expect("no layout line");
    let rest = &line[line.find("layout dpr=").unwrap() + "layout dpr=".len()..];
    let (dpr, rects) = rest.split_once(' ').unwrap();
    let rects = rects
        .split(';')
        .filter_map(|entry| {
            let (key, value) = entry.split_once('=')?;
            let v: Vec<f64> = value
                .split(',')
                .map(|n| n.trim().parse().unwrap())
                .collect();
            Some((key.to_owned(), [v[0], v[1], v[2], v[3]]))
        })
        .collect();
    (dpr.parse().unwrap(), rects)
}

fn format_id(name: PCWSTR) -> u16 {
    unsafe { RegisterClipboardFormatW(name) as u16 }
}

fn hglobal(bytes: &[u8]) -> HGLOBAL {
    unsafe {
        let global = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).unwrap();
        let ptr = GlobalLock(global).cast::<u8>();
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(global);
        global
    }
}

fn hglobal_medium(bytes: &[u8]) -> STGMEDIUM {
    STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 {
            hGlobal: hglobal(bytes),
        },
        pUnkForRelease: std::mem::ManuallyDrop::new(None),
    }
}

/// An Outlook-like data object: virtual files only, copy only.
#[implement(IDataObject, IDataObjectAsyncCapability)]
struct VirtualData {
    files: Vec<(String, Vec<u8>)>,
    medium: Medium,
    async_mode: Cell<bool>,
    in_operation: Cell<bool>,
    /// Formats the drag image helper and the target set (drag image, drop description, ...).
    stored: RefCell<Vec<(u16, Vec<u8>)>>,
    storages: Vec<IStorage>,
    log: Rc<RefCell<Vec<String>>>,
    started: Instant,
}

impl VirtualData {
    fn new(medium: Medium, async_mode: bool, log: Rc<RefCell<Vec<String>>>) -> Self {
        let big: Vec<u8> = (0..5_000_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();
        let files = vec![
            (
                "ინვოისი №12.pdf".to_owned(),
                b"%PDF-1.4 jerry spike 14\n".repeat(400),
            ),
            (
                "Re: offer? \"final\".txt".to_owned(),
                "ქართული ტექსტი\r\n".as_bytes().to_vec(),
            ),
            (
                "..\\escape.txt".to_owned(),
                b"must not leave the folder".to_vec(),
            ),
            ("დიდი ფაილი.bin".to_owned(), big),
        ];
        let storages = if medium == Medium::Storage {
            files.iter().map(|(_, bytes)| storage_with(bytes)).collect()
        } else {
            Vec::new()
        };
        Self {
            files,
            medium,
            async_mode: Cell::new(async_mode),
            in_operation: Cell::new(false),
            stored: RefCell::default(),
            storages,
            log,
            started: Instant::now(),
        }
    }

    fn note(&self, line: String) {
        self.log.borrow_mut().push(format!(
            "+{:>5} ms {line}",
            self.started.elapsed().as_millis()
        ));
    }

    fn descriptor(&self) -> Vec<u8> {
        let mut bytes = (self.files.len() as u32).to_le_bytes().to_vec();
        for (name, content) in &self.files {
            let mut fd = FILEDESCRIPTORW {
                dwFlags: (FD_FILESIZE.0 | FD_PROGRESSUI.0) as u32,
                nFileSizeLow: content.len() as u32,
                ..Default::default()
            };
            let mut wide = [0u16; 260];
            for (slot, unit) in wide.iter_mut().zip(name.encode_utf16()) {
                *slot = unit;
            }
            fd.cFileName = wide;
            let raw = unsafe {
                std::slice::from_raw_parts(
                    (&fd as *const FILEDESCRIPTORW).cast::<u8>(),
                    size_of::<FILEDESCRIPTORW>(),
                )
            };
            bytes.extend_from_slice(raw);
        }
        bytes
    }

    fn formats(&self) -> Vec<FORMATETC> {
        let tymed = match self.medium {
            Medium::Stream => TYMED_ISTREAM,
            Medium::HGlobal => TYMED_HGLOBAL,
            Medium::Storage => TYMED_ISTORAGE,
        };
        let mut list = vec![
            etc(format_id(CFSTR_FILEDESCRIPTORW), TYMED_HGLOBAL.0 as u32, -1),
            etc(format_id(CFSTR_FILECONTENTS), tymed.0 as u32, -1),
        ];
        for (format, _) in self.stored.borrow().iter() {
            list.push(etc(*format, TYMED_HGLOBAL.0 as u32, -1));
        }
        list
    }
}

fn etc(format: u16, tymed: u32, index: i32) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: index,
        tymed,
    }
}

/// An in-memory compound file with one stream, standing in for an Outlook .msg.
fn storage_with(bytes: &[u8]) -> IStorage {
    unsafe {
        let lock = CreateILockBytesOnHGlobal(None, true).unwrap();
        let mode = windows::Win32::System::Com::STGM(
            STGM_CREATE.0 | STGM_READWRITE.0 | STGM_SHARE_EXCLUSIVE.0,
        );
        let storage = StgCreateDocfileOnILockBytes(&lock, mode, 0).unwrap();
        let stream = storage.CreateStream(w!("Contents"), mode, 0, 0).unwrap();
        let mut written = 0;
        let _ = ISequentialStream::Write(
            &stream,
            bytes.as_ptr().cast(),
            bytes.len() as u32,
            Some(&mut written),
        );
        drop(stream);
        storage.Commit(STGC_DEFAULT.0 as u32).unwrap();
        storage
    }
}

impl IDataObject_Impl for VirtualData_Impl {
    fn GetData(&self, format: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        let format = unsafe { &*format };
        if format.cfFormat == format_id(CFSTR_FILEDESCRIPTORW) {
            return Ok(hglobal_medium(&self.descriptor()));
        }
        if format.cfFormat == format_id(CFSTR_FILECONTENTS) {
            let index = format.lindex;
            self.note(format!(
                "GetData FileContents lindex={index} tymed={:#x}",
                format.tymed
            ));
            let Some((_, bytes)) = usize::try_from(index).ok().and_then(|i| self.files.get(i))
            else {
                return Err(DV_E_FORMATETC.into());
            };
            return Ok(match self.medium {
                Medium::HGlobal => hglobal_medium(bytes),
                Medium::Stream => STGMEDIUM {
                    tymed: TYMED_ISTREAM.0 as u32,
                    u: STGMEDIUM_0 {
                        pstm: std::mem::ManuallyDrop::new(unsafe {
                            SHCreateMemStream(Some(bytes))
                        }),
                    },
                    pUnkForRelease: std::mem::ManuallyDrop::new(None),
                },
                Medium::Storage => STGMEDIUM {
                    tymed: TYMED_ISTORAGE.0 as u32,
                    u: STGMEDIUM_0 {
                        pstg: std::mem::ManuallyDrop::new(Some(
                            self.storages[index as usize].clone(),
                        )),
                    },
                    pUnkForRelease: std::mem::ManuallyDrop::new(None),
                },
            });
        }
        let stored = self.stored.borrow();
        match stored.iter().find(|(f, _)| *f == format.cfFormat) {
            Some((_, bytes)) => Ok(hglobal_medium(bytes)),
            None => Err(DV_E_FORMATETC.into()),
        }
    }

    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, format: *const FORMATETC) -> HRESULT {
        let format = unsafe { &*format };
        if self.formats().iter().any(|f| f.cfFormat == format.cfFormat) {
            S_OK
        } else {
            DV_E_FORMATETC
        }
    }

    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        unsafe { (*out).ptd = std::ptr::null_mut() };
        E_NOTIMPL
    }

    fn SetData(
        &self,
        format: *const FORMATETC,
        medium: *const STGMEDIUM,
        release: BOOL,
    ) -> windows::core::Result<()> {
        let format = unsafe { &*format };
        let medium = unsafe { &mut *(medium as *mut STGMEDIUM) };
        if medium.tymed != TYMED_HGLOBAL.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        let bytes = unsafe {
            let global = medium.u.hGlobal;
            let size = GlobalSize(global);
            let ptr = GlobalLock(global).cast::<u8>();
            let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
            let _ = GlobalUnlock(global);
            bytes
        };
        if release.as_bool() {
            unsafe { ReleaseStgMedium(medium) };
        }
        let mut name = [0u16; 64];
        let len = unsafe {
            windows::Win32::System::DataExchange::GetClipboardFormatNameW(
                u32::from(format.cfFormat),
                &mut name,
            )
        };
        let name = String::from_utf16_lossy(&name[..len.max(0) as usize]);
        if name == "DropDescription" && bytes.len() >= 4 + 520 {
            let kind = i32::from_le_bytes(bytes[0..4].try_into().unwrap());
            let text = |offset: usize| {
                let units: Vec<u16> = bytes[offset..offset + 520]
                    .chunks(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .take_while(|&u| u != 0)
                    .collect();
                String::from_utf16_lossy(&units)
            };
            self.note(format!(
                "SetData DropDescription type={kind} {:?} insert={:?}",
                text(4),
                text(524)
            ));
        } else if name == "Performed DropEffect" {
            self.note(format!(
                "SetData Performed DropEffect = {:?}",
                bytes.get(..4)
            ));
        }
        let mut stored = self.stored.borrow_mut();
        stored.retain(|(f, _)| *f != format.cfFormat);
        stored.push((format.cfFormat, bytes));
        Ok(())
    }

    fn EnumFormatEtc(&self, direction: u32) -> windows::core::Result<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        unsafe { SHCreateStdEnumFmtEtc(&self.formats()) }
    }

    fn DAdvise(
        &self,
        _: *const FORMATETC,
        _: u32,
        _: Ref<IAdviseSink>,
    ) -> windows::core::Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _: u32) -> windows::core::Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

impl IDataObjectAsyncCapability_Impl for VirtualData_Impl {
    fn SetAsyncMode(&self, on: BOOL) -> windows::core::Result<()> {
        self.async_mode.set(on.as_bool());
        Ok(())
    }

    fn GetAsyncMode(&self) -> windows::core::Result<BOOL> {
        Ok(self.async_mode.get().into())
    }

    fn StartOperation(&self, _: Ref<IBindCtx>) -> windows::core::Result<()> {
        self.in_operation.set(true);
        self.note("StartOperation".into());
        Ok(())
    }

    fn InOperation(&self) -> windows::core::Result<BOOL> {
        Ok(self.in_operation.get().into())
    }

    fn EndOperation(
        &self,
        result: HRESULT,
        _: Ref<IBindCtx>,
        effects: u32,
    ) -> windows::core::Result<()> {
        self.in_operation.set(false);
        self.note(format!("EndOperation hr={:#x} effects={effects}", result.0));
        Ok(())
    }
}

thread_local! {
    static SOURCE: RefCell<Option<(IDataObject, DROPEFFECT)>> = const { RefCell::new(None) };
    static OUTCOME: RefCell<Option<String>> = const { RefCell::new(None) };
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
            if let Some((data, allowed)) = SOURCE.with(|s| s.borrow_mut().take()) {
                let started = Instant::now();
                let outcome = do_drag(hwnd, &data, allowed);
                let performed = dword(&data, CFSTR_PERFORMEDDROPEFFECT);
                OUTCOME.with(|o| {
                    *o.borrow_mut() = Some(format!(
                        "{outcome}, performed {performed:?}, DoDragDrop {} ms",
                        started.elapsed().as_millis()
                    ))
                });
                // Keep pumping so Jerry can still read virtual file streams after Drop.
                unsafe { SetTimer(Some(hwnd), 1, 4000, None) };
                SOURCE.with(|s| *s.borrow_mut() = Some((data, DROPEFFECT_NONE)));
            }
            LRESULT(0)
        }
        WM_TIMER | WM_CLOSE_LATER => {
            unsafe {
                let _ = KillTimer(Some(hwnd), 1);
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn do_drag(hwnd: HWND, data: &IDataObject, allowed: DROPEFFECT) -> String {
    windows::core::link!("shell32.dll" "system" fn SHDoDragDrop(
        hwnd: HWND,
        pdata: *mut core::ffi::c_void,
        pdsrc: *mut core::ffi::c_void,
        dweffect: DROPEFFECT,
        pdweffect: *mut DROPEFFECT,
    ) -> HRESULT);
    let mut effect = DROPEFFECT_NONE;
    let hr = unsafe {
        use windows::core::Interface;
        SHDoDragDrop(
            hwnd,
            data.as_raw(),
            std::ptr::null_mut(),
            allowed,
            &mut effect,
        )
    };
    format!("SHDoDragDrop hr={:#x} effect={}", hr.0, effect.0)
}

fn dword(data: &IDataObject, name: PCWSTR) -> Option<u32> {
    unsafe {
        let mut medium = data
            .GetData(&etc(format_id(name), TYMED_HGLOBAL.0 as u32, -1))
            .ok()?;
        let ptr = GlobalLock(medium.u.hGlobal).cast::<u32>();
        let value = (!ptr.is_null()).then(|| ptr.read_unaligned());
        let _ = GlobalUnlock(medium.u.hGlobal);
        ReleaseStgMedium(&mut medium);
        value
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let source = args.first().expect("SOURCE missing").clone();
    let mut waypoints = Vec::new();
    let mut options = Options::default();
    for arg in &args[1..] {
        match arg.as_str() {
            "right" => options.right = true,
            "esc" => options.esc = true,
            "anywhere" => options.anywhere = true,
            other if other.starts_with("then-click=") => {
                let (dx, dy) = other[11..].split_once(',').unwrap();
                options.then_click = Some((dx.parse().unwrap(), dy.parse().unwrap()));
            }
            other if other.starts_with("shot=") => options.shots = Some(other[5..].into()),
            other => waypoints.push(parse_waypoint(other)),
        }
    }
    assert!(!waypoints.is_empty(), "no waypoints");

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        OleInitialize(None).unwrap();
    }
    let jerry = find_jerry().expect("this worktree's Jerry isn't running");
    let (dpr, layout) = read_layout();
    let mut origin = POINT::default();
    let _ = unsafe { ClientToScreen(jerry, &mut origin) };
    let points: Vec<(Waypoint, POINT)> = waypoints
        .into_iter()
        .map(|wp| {
            if let Some(xy) = wp.key.strip_prefix("screen:") {
                let (x, y) = xy.split_once(',').unwrap();
                return (
                    wp.clone(),
                    POINT {
                        x: x.parse().unwrap(),
                        y: y.parse().unwrap(),
                    },
                );
            }
            let r = layout
                .get(&wp.key)
                .unwrap_or_else(|| panic!("{} isn't in the layout", wp.key));
            let at = POINT {
                x: origin.x + ((r[0] + r[2] / 2.0) * dpr).round() as i32,
                y: origin.y + ((r[1] + r[3] / 2.0) * dpr).round() as i32,
            };
            (wp, at)
        })
        .collect();

    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let (data, allowed): (IDataObject, DROPEFFECT) = match source.as_str() {
        "files" => {
            let dir = std::env::temp_dir().join("jerry-spike-14-src");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("ფოტოები")).unwrap();
            fs::write(dir.join("ნახაზი A-101.dwg"), b"AC1032 probe").unwrap();
            fs::write(dir.join("ფოტოები").join("1.jpg"), b"jpeg?").unwrap();
            let paths = vec![dir.join("ნახაზი A-101.dwg"), dir.join("ფოტოები")];
            let data = jerry_win::drag::data_object(&paths).unwrap();
            (
                data,
                DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0 | DROPEFFECT_LINK.0),
            )
        }
        other if other.starts_with("jerry:") || other.starts_with("at:") => {
            // Jerry is the source: the probe presses on one of its rows; no data object here.
            (
                jerry_win::drag::data_object(&[std::env::temp_dir()]).unwrap(),
                DROPEFFECT_NONE,
            )
        }
        medium => {
            let (medium, async_mode) = match medium {
                "stream" => (Medium::Stream, false),
                "hglobal" => (Medium::HGlobal, false),
                "storage" => (Medium::Storage, false),
                "async" => (Medium::Stream, true),
                other => panic!("unknown source {other}"),
            };
            (
                VirtualData::new(medium, async_mode, log.clone()).into(),
                DROPEFFECT_COPY,
            )
        }
    };
    SOURCE.with(|s| *s.borrow_mut() = Some((data, allowed)));

    // The source window sits at the top left of Jerry's screen area, outside Jerry.
    let mut jerry_rect = RECT::default();
    unsafe { GetWindowRect(jerry, &mut jerry_rect).unwrap() };
    let (sx, sy, sw, sh) = (
        jerry_rect.left.max(170) - 160,
        jerry_rect.top + 60,
        150,
        110,
    );
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("JerryDragProbe"),
            ..Default::default()
        });
        CreateWindowExW(
            WS_EX_TOPMOST,
            w!("JerryDragProbe"),
            w!("drag probe"),
            WS_CAPTION | WS_VISIBLE,
            sx,
            sy,
            sw,
            sh,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .unwrap()
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
    let press = match source.strip_prefix("jerry:") {
        None if source.starts_with("at:") => {
            // Another app is the source (Explorer): press at a screen point.
            SOURCE.with(|s| s.borrow_mut().take());
            let (x, y) = source[3..].split_once(',').unwrap();
            POINT {
                x: x.parse().unwrap(),
                y: y.parse().unwrap(),
            }
        }
        Some(key) => {
            SOURCE.with(|s| s.borrow_mut().take());
            let r = layout[key];
            POINT {
                x: origin.x + ((r[0] + r[2] / 2.0) * dpr).round() as i32,
                y: origin.y + ((r[1] + r[3] / 2.0) * dpr).round() as i32,
            }
        }
        None => POINT {
            x: sx + sw / 2,
            y: sy + sh / 2 + 10,
        },
    };
    let jerry_raw = jerry.0 as isize;
    let probe_raw = hwnd.0 as isize;
    let driver = thread::spawn(move || drive(points, press, options, jerry_raw, probe_raw));

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    println!("driver: {}", driver.join().unwrap());
    println!(
        "source: {}",
        OUTCOME
            .with(|o| o.borrow().clone())
            .unwrap_or_else(|| "no drag".into())
    );
    for line in log.borrow().iter() {
        println!("source: {line}");
    }
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
    // Exit hard: when a target still holds COM references to our data object, a normal exit can
    // hang in COM's shutdown.
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    unsafe {
        let _ = windows::Win32::System::Threading::TerminateProcess(
            windows::Win32::System::Threading::GetCurrentProcess(),
            0,
        );
    }
}

fn root(point: POINT) -> isize {
    let top = unsafe { GetAncestor(WindowFromPoint(point), GA_ROOT) };
    // Jerry's drop overlay is an owned popup: count it as Jerry.
    match unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindow(
            top,
            windows::Win32::UI::WindowsAndMessaging::GW_OWNER,
        )
    } {
        Ok(owner) if !owner.is_invalid() => owner.0 as isize,
        _ => top.0 as isize,
    }
}

/// Command-line flags for a drive.
#[derive(Default)]
struct Options {
    right: bool,
    esc: bool,
    anywhere: bool,
    then_click: Option<(i32, i32)>,
    shots: Option<PathBuf>,
}

fn drive(
    points: Vec<(Waypoint, POINT)>,
    press: POINT,
    options: Options,
    jerry: isize,
    probe: isize,
) -> String {
    let Options {
        right,
        esc,
        anywhere,
        then_click,
        shots,
    } = options;
    let close = || unsafe {
        let _ = PostMessageW(
            Some(HWND(probe as *mut _)),
            WM_CLOSE_LATER,
            WPARAM(0),
            LPARAM(0),
        );
    };
    thread::sleep(Duration::from_millis(400));
    let mut saved = POINT::default();
    let _ = unsafe { GetCursorPos(&mut saved) };
    // Pressing on the probe starts its own drag; anywhere else (a Jerry row, an Explorer item)
    // the app under the cursor starts it, and the probe only moves the mouse.
    let external = root(press) != probe;
    let (down, up) = if right {
        (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
    } else {
        (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
    };
    mouse(press, MOUSEEVENTF_MOVE);
    thread::sleep(Duration::from_millis(80));
    mouse(press, down);
    thread::sleep(Duration::from_millis(150));

    let mut report = Vec::new();
    let mut at = press;
    let mut held = (false, false, false);
    for (index, (wp, target)) in points.iter().enumerate() {
        set_modifiers(&mut held, (wp.ctrl, wp.shift, wp.alt));
        let steps = 20;
        for step in 1..=steps {
            let t = f64::from(step) / f64::from(steps);
            let p = POINT {
                x: at.x + ((target.x - at.x) as f64 * t) as i32,
                y: at.y + ((target.y - at.y) as f64 * t) as i32,
            };
            mouse(p, MOUSEEVENTF_MOVE);
            thread::sleep(Duration::from_millis(12));
        }
        at = *target;
        thread::sleep(wp.rest);
        if let Some(dir) = &shots {
            fs::create_dir_all(dir).unwrap();
            let name = format!("{index:02}-{}.bmp", wp.key.replace(':', "-"));
            capture(HWND(jerry as *mut _), &dir.join(name));
        }
        report.push(format!("{} at {:?}", wp.key, target));
    }
    let over_jerry = anywhere || root(at) == jerry;
    if esc || !over_jerry {
        key(VK_ESCAPE, false);
        key(VK_ESCAPE, true);
        thread::sleep(Duration::from_millis(150));
        report.push(if over_jerry {
            "Esc".into()
        } else {
            "NOT over Jerry: Esc".into()
        });
    }
    mouse(at, up);
    thread::sleep(Duration::from_millis(100));
    set_modifiers(&mut held, (false, false, false));
    thread::sleep(Duration::from_millis(300));
    if let Some((dx, dy)) = then_click {
        // Click something Jerry shows after the drop (the right-drag menu).
        thread::sleep(Duration::from_millis(400));
        if let Some(dir) = &shots {
            capture(HWND(jerry as *mut _), &dir.join("99-after-drop.bmp"));
        }
        let target = POINT {
            x: at.x + dx,
            y: at.y + dy,
        };
        mouse(target, MOUSEEVENTF_MOVE);
        thread::sleep(Duration::from_millis(120));
        mouse(target, MOUSEEVENTF_LEFTDOWN);
        thread::sleep(Duration::from_millis(60));
        mouse(target, MOUSEEVENTF_LEFTUP);
        thread::sleep(Duration::from_millis(300));
        report.push(format!("clicked {target:?}"));
    }
    unsafe {
        let _ = SetCursorPos(saved.x, saved.y);
    }
    if external {
        thread::sleep(Duration::from_millis(1500));
        close();
    }
    report.join(" → ")
}

fn set_modifiers(held: &mut (bool, bool, bool), want: (bool, bool, bool)) {
    for (is, should, vk) in [
        (&mut held.0, want.0, VK_CONTROL),
        (&mut held.1, want.1, VK_SHIFT),
        (&mut held.2, want.2, VK_MENU),
    ] {
        if *is != should {
            key(vk, !should);
            *is = should;
            thread::sleep(Duration::from_millis(30));
        }
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                ..Default::default()
            },
        },
    };
    unsafe { SendInput(&[input], size_of::<INPUT>() as i32) };
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
                dx: (at.x - vx) * 65535 / (vw - 1),
                dy: (at.y - vy) * 65535 / (vh - 1),
                dwFlags: flags | MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                ..Default::default()
            },
        },
    };
    unsafe { SendInput(&[input], size_of::<INPUT>() as i32) };
}

/// Saves a screenshot of `window`'s rectangle as a BMP, layered windows (the drag image)
/// included.
fn capture(window: HWND, path: &PathBuf) {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(window, &mut rect).unwrap() };
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
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
            rect.left,
            rect.top,
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

/// The Jerry window of this worktree's build (`target\<profile>\jerry.exe`, next to
/// `examples\`): other sessions may run their own Jerry with the same title.
fn find_jerry() -> Option<HWND> {
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowThreadProcessId};
    use windows::core::PWSTR;

    let probe = std::env::current_exe().ok()?;
    let expected = probe.parent()?.parent()?.join("jerry.exe");
    let mut after: Option<HWND> = None;
    loop {
        let hwnd = unsafe { FindWindowExW(None, after, w!("Tauri Window"), w!("Jerry")) }.ok()?;
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        let path = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .ok()
            .map(|process| {
                let mut buffer = [0u16; 1024];
                let mut len = buffer.len() as u32;
                let _ = unsafe {
                    QueryFullProcessImageNameW(
                        process,
                        PROCESS_NAME_WIN32,
                        PWSTR(buffer.as_mut_ptr()),
                        &mut len,
                    )
                };
                let _ = unsafe { windows::Win32::Foundation::CloseHandle(process) };
                PathBuf::from(String::from_utf16_lossy(&buffer[..len as usize]))
            });
        if path.is_some_and(|p| p.eq_ignore_ascii_case_path(&expected)) {
            return Some(hwnd);
        }
        after = Some(hwnd);
    }
}

trait EqIgnoreCase {
    fn eq_ignore_ascii_case_path(&self, other: &std::path::Path) -> bool;
}

impl EqIgnoreCase for PathBuf {
    fn eq_ignore_ascii_case_path(&self, other: &std::path::Path) -> bool {
        self.to_string_lossy()
            .eq_ignore_ascii_case(&other.to_string_lossy())
    }
}
