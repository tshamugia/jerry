//! Windows' own context menu for files: Jerry's "Windows options…" (spike #16).
//!
//! The menu is the one Explorer shows (on Windows 11 behind "Show more options"): the items'
//! parent folder hands out an `IContextMenu` (`IShellFolder::GetUIObjectOf`), and
//! `QueryContextMenu` fills a popup menu with the static verbs and every registered handler
//! (7-Zip, OneDrive, antivirus, "Send to"). Some handlers fill their submenus only when they
//! open ("Send to", "Open with") or draw their own items, and they need the owner window's menu
//! messages: while the menu is up the owner is subclassed, and `WM_INITMENUPOPUP`,
//! `WM_MEASUREITEM`, `WM_DRAWITEM` and `WM_MENUCHAR` go to `IContextMenu3::HandleMenuMsg2` (or
//! `IContextMenu2::HandleMenuMsg`).
//!
//! Everything here runs on Jerry's main STA thread: `TrackPopupMenuEx` must run on the thread
//! that owns the owner window, and its modal loop must not run inside a tao or WebView2
//! callback (start it with [`crate::main_thread::run_on_main`]).

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CMF_CANRENAME, CMF_EXTENDEDVERBS, CMF_ITEMMENU, CMF_NORMAL, CMIC_MASK_CONTROL_DOWN,
    CMIC_MASK_PTINVOKE, CMIC_MASK_SHIFT_DOWN, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX,
    DefSubclassProc, GCS_VERBW, IContextMenu, IContextMenu2, IContextMenu3, ILClone,
    ILCreateFromPathW, ILFindLastID, ILFree, ILIsParent, ILRemoveLastID, IShellFolder,
    RemoveWindowSubclass, SEE_MASK_ASYNCOK, SEE_MASK_UNICODE, SHBindToParent, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetMenuItemCount, GetMenuItemInfoW, HMENU, MENUITEMINFOW,
    MFS_DEFAULT, MFS_DISABLED, MFT_OWNERDRAW, MFT_SEPARATOR, MIIM_BITMAP, MIIM_FTYPE, MIIM_ID,
    MIIM_STATE, MIIM_STRING, MIIM_SUBMENU, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenuEx, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
};
use windows::core::{BOOL, Interface, PCSTR, PCWSTR, PSTR};

/// The command ids handed to the handlers. 0 is "nothing chosen" for `TrackPopupMenuEx`.
const FIRST_ID: u32 = 1;
const LAST_ID: u32 = 0x7FFF;
/// Our id among the owner window's subclasses (tao has its own).
const SUBCLASS_ID: usize = 0x4A16;

/// Verbs Jerry runs itself, with its own undo, journal and cut style, instead of the Shell's.
/// "rename" must be Jerry's anyway: the Shell's starts in-place editing in Explorer's view
/// (through the menu's site), which Jerry doesn't have.
pub const JERRY_VERBS: &[&str] = &["rename", "delete", "cut", "copy", "paste"];

/// Why the menu couldn't be shown.
#[derive(Debug, thiserror::Error)]
pub enum MenuError {
    #[error("nothing selected")]
    Empty,
    /// One menu covers items of one folder only (search results can mix folders).
    #[error("the items are in different folders")]
    MixedFolders,
    /// The Shell can't parse the path: the file is gone, or the namespace doesn't know it.
    #[error("the Shell can't resolve {}", path.display())]
    Path { path: PathBuf },
    /// A Windows menu is already open (its modal loop dispatches Jerry's own posted tasks, so
    /// a second request can arrive while it runs).
    #[error("a Windows menu is already open")]
    Busy,
    #[error(transparent)]
    Os(#[from] windows::core::Error),
}

/// How [`show`] runs the menu.
pub struct MenuOptions {
    /// Shift was held: extended verbs too, as in Explorer's Shift+right-click.
    pub extended: bool,
    /// Verbs that are returned to the caller instead of being invoked ([`JERRY_VERBS`]).
    pub own_verbs: Vec<String>,
    /// Forward the menu messages to the handlers. Always on in Jerry; off only to show what
    /// breaks without it.
    pub forward: bool,
    /// Runs on the main thread once the menu is about to appear (its modal loop is running).
    pub on_open: Option<Box<dyn FnOnce()>>,
}

impl Default for MenuOptions {
    fn default() -> Self {
        Self {
            extended: false,
            own_verbs: JERRY_VERBS.iter().map(|&verb| verb.to_owned()).collect(),
            forward: true,
            on_open: None,
        }
    }
}

/// What the user chose and how long each step took.
#[derive(Clone, Debug, Default)]
pub struct MenuOutcome {
    pub chosen: Option<Chosen>,
    pub timings: Timings,
    pub forwarded: Forwarded,
}

/// The command the user picked.
#[derive(Clone, Debug)]
pub struct Chosen {
    /// The command's offset from the first id (what `InvokeCommand` takes).
    pub offset: u32,
    /// The canonical verb ("open", "properties"), if the handler reports one.
    pub verb: Option<String>,
    /// The item's text without `&` and accelerators, for logs.
    pub text: String,
    /// False when the verb is one of `own_verbs`: Jerry runs it, the Shell didn't.
    pub invoked: bool,
}

/// Step times. Everything but `menu` blocks the main thread.
#[derive(Clone, Debug, Default)]
pub struct Timings {
    /// Paths → ID lists → parent folder.
    pub resolve: Duration,
    /// `GetUIObjectOf`: the folder creates the menu object and its handlers' list.
    pub ui_object: Duration,
    /// `QueryContextMenu`: every handler loads and adds its items. The slow part.
    pub query: Duration,
    /// Submenus the handlers filled when they opened (`WM_INITMENUPOPUP`), in order.
    pub popups: Vec<Popup>,
    /// `TrackPopupMenuEx`, from show to choice (the user's time).
    pub menu: Duration,
    /// `InvokeCommand`, when the Shell ran the choice.
    pub invoke: Option<Duration>,
}

/// One forwarded `WM_INITMENUPOPUP`.
#[derive(Clone, Debug)]
pub struct Popup {
    /// The submenu's item text; empty for the root menu.
    pub text: String,
    pub elapsed: Duration,
    pub items_before: i32,
    pub items_after: i32,
}

/// How many menu messages went to the handlers.
#[derive(Clone, Copy, Debug, Default)]
pub struct Forwarded {
    pub init_popup: u32,
    pub measure_item: u32,
    pub draw_item: u32,
    pub menu_char: u32,
}

/// One item of a menu, for diagnostics and tests.
#[derive(Clone, Debug, Default)]
pub struct MenuItem {
    pub text: String,
    /// The command's offset, for items that are commands (not separators or submenus).
    pub offset: Option<u32>,
    pub verb: Option<String>,
    pub separator: bool,
    pub disabled: bool,
    pub default: bool,
    /// The handler draws the item itself (`WM_MEASUREITEM` / `WM_DRAWITEM`).
    pub owner_draw: bool,
    /// The submenu, if this item opens one (its handle and position for [`ShellMenu::open`]).
    pub submenu: Option<Submenu>,
}

#[derive(Clone, Debug, Default)]
pub struct Submenu {
    pub handle: isize,
    /// Position of the item in its parent menu.
    pub position: u32,
    pub items: Vec<MenuItem>,
}

static OPEN: AtomicBool = AtomicBool::new(false);

/// Clears [`OPEN`] however [`show`] ends.
struct OpenFlag;

impl Drop for OpenFlag {
    fn drop(&mut self) {
        OPEN.store(false, Ordering::Release);
    }
}

/// Shows Windows' context menu for `paths` (items of one folder) at screen point `(x, y)`,
/// owned by `owner`, and runs the chosen command unless it's one of `own_verbs`.
///
/// Main thread only, outside tao and WebView2 callbacks; returns when the menu closes and the
/// command was handed to its handler (handlers that work long, like "Send to → Compressed
/// folder", finish on their own threads).
pub fn show(
    owner: isize,
    paths: &[PathBuf],
    x: i32,
    y: i32,
    options: MenuOptions,
) -> Result<MenuOutcome, MenuError> {
    if OPEN.swap(true, Ordering::AcqRel) {
        return Err(MenuError::Busy);
    }
    let _open = OpenFlag;
    let menu = ShellMenu::new(owner, paths, options.extended)?;
    let mut timings = menu.timings.clone();
    let tracked = menu.track(owner, x, y, options.forward, options.on_open);
    timings.menu = tracked.elapsed;
    timings.popups = tracked.popups;
    let mut outcome = MenuOutcome {
        forwarded: tracked.forwarded,
        ..Default::default()
    };
    if let Some(offset) = tracked.offset {
        let verb = menu.verb(offset);
        let text = menu.text_of(offset).unwrap_or_default();
        let own = verb.as_ref().is_some_and(|verb| {
            options
                .own_verbs
                .iter()
                .any(|own| own.eq_ignore_ascii_case(verb))
        });
        if !own {
            let start = Instant::now();
            menu.invoke(owner, offset, x, y)?;
            timings.invoke = Some(start.elapsed());
        }
        outcome.chosen = Some(Chosen {
            offset,
            verb,
            text,
            invoked: !own,
        });
    }
    outcome.timings = timings;
    Ok(outcome)
}

/// Loads the Shell's menu machinery and the handlers registered for every file and folder, by
/// building and dropping the menus of Jerry's own exe and its folder. Run it once on a Shell
/// pool (STA) thread after startup, before the user asks for a Windows menu: the main thread's
/// first menu then takes ~0.1 s instead of ~0.5 s, and it includes the packaged handlers
/// (WinRAR, Notepad, Terminal), which the first menu of a process can miss (spike #16). No
/// user file is touched, nothing is invoked.
pub fn prewarm() -> Result<Duration, MenuError> {
    let start = Instant::now();
    let exe = std::env::current_exe().map_err(|_| MenuError::Empty)?;
    let folder = exe
        .parent()
        .map(Path::to_path_buf)
        .ok_or(MenuError::Empty)?;
    for path in [exe, folder] {
        drop(ShellMenu::new(0, &[path], false)?);
    }
    Ok(start.elapsed())
}

/// Runs [`prewarm`] on a new STA thread (a stand-in for the Shell pool) and hands the result
/// to `done` there.
pub fn spawn_prewarm(done: impl FnOnce(Result<Duration, MenuError>) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("jerry-menu-prewarm".into())
        .spawn(move || {
            // SAFETY: a fresh thread; the Shell's objects need an STA. Never uninitialized, so
            // COM keeps the handler DLLs loaded for the main thread's menus.
            let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
                .ok()
                .map_err(MenuError::Os)
                .and_then(|()| prewarm());
            done(result);
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "couldn't start the menu prewarm thread");
    }
}

/// Lets Win32 menus (Windows' context menu included) follow Windows' dark mode, as Explorer's
/// do; without it they're always light. uxtheme exports this only by ordinal (undocumented,
/// stable since Windows 10 1903, used by Explorer and Notepad): SetPreferredAppMode(AllowDark)
/// #135, AllowDarkModeForWindow #133 for the menus' owner, FlushMenuThemes #136. Main thread,
/// once, with Jerry's window.
pub fn follow_system_theme(owner: isize) -> windows::core::Result<()> {
    // SAFETY: a system DLL by name; it stays loaded (WebView2 and comctl32 use it anyway).
    let uxtheme = unsafe { LoadLibraryW(windows::core::w!("uxtheme.dll")) }?;
    let ordinal = |n: usize| {
        // SAFETY: a valid module; an ordinal is passed as MAKEINTRESOURCE in the name.
        unsafe { GetProcAddress(uxtheme, PCSTR(n as *const u8)) }
            .ok_or_else(windows::core::Error::from_thread)
    };
    let (set_mode, allow, flush) = (ordinal(135)?, ordinal(133)?, ordinal(136)?);
    // SAFETY: the ordinals' signatures since Windows 10 1903: SetPreferredAppMode(int) -> int,
    // AllowDarkModeForWindow(HWND, BOOL) -> BOOL, FlushMenuThemes().
    unsafe {
        let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
        let allow: unsafe extern "system" fn(HWND, BOOL) -> BOOL = std::mem::transmute(allow);
        let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
        // 1 = AllowDark: dark when Windows' app mode is dark, light otherwise.
        set_mode(1);
        let _ = allow(hwnd(owner), BOOL(1));
        flush();
    }
    Ok(())
}

/// Converts a point in `owner`'s client area (physical pixels) to screen coordinates.
pub fn client_to_screen(owner: isize, x: i32, y: i32) -> (i32, i32) {
    let mut point = POINT { x, y };
    // SAFETY: a valid out pointer; a stale window only fails the call, leaving the point.
    let _ = unsafe { ClientToScreen(hwnd(owner), &mut point) };
    (point.x, point.y)
}

/// The path the Shell can parse: without core's `\\?\` prefix. The Shell handles plain paths
/// longer than MAX_PATH, but not verbatim ones (spike #15).
pub fn shell_path(path: &Path) -> PathBuf {
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let unc: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
    let verbatim: Vec<u16> = r"\\?\".encode_utf16().collect();
    let plain: Vec<u16> = if wide.starts_with(&unc) {
        r"\\"
            .encode_utf16()
            .chain(wide[unc.len()..].iter().copied())
            .collect()
    } else if wide.starts_with(&verbatim) {
        wide[verbatim.len()..].to_vec()
    } else {
        return path.to_path_buf();
    };
    PathBuf::from(OsString::from_wide(&plain))
}

/// A filled Shell context menu: the handlers are loaded and their items are in `hmenu`.
pub struct ShellMenu {
    menu: IContextMenu,
    menu2: Option<IContextMenu2>,
    menu3: Option<IContextMenu3>,
    hmenu: HMENU,
    /// The items' folder, NUL-terminated, for `InvokeCommand`'s working directory.
    directory: Vec<u16>,
    /// `resolve`, `ui_object` and `query`.
    pub timings: Timings,
}

impl ShellMenu {
    /// Builds the menu for `paths` (items of one folder): the slow part of showing it, since
    /// every handler loads here.
    pub fn new(owner: isize, paths: &[PathBuf], extended: bool) -> Result<Self, MenuError> {
        let mut timings = Timings::default();
        let start = Instant::now();
        let items = Items::resolve(paths)?;
        timings.resolve = start.elapsed();

        let start = Instant::now();
        // SAFETY: the children are single-level IDs of `folder`, kept alive by `items`.
        let menu: IContextMenu = unsafe {
            items
                .folder
                .GetUIObjectOf(hwnd(owner), &items.children, None)
        }?;
        timings.ui_object = start.elapsed();

        // SAFETY: no arguments; destroyed in Drop.
        let hmenu = unsafe { CreatePopupMenu() }?;
        let mut flags = CMF_NORMAL | CMF_CANRENAME | CMF_ITEMMENU;
        if extended {
            flags |= CMF_EXTENDEDVERBS;
        }
        let start = Instant::now();
        // SAFETY: a live menu object and an empty popup menu we own.
        let filled = unsafe { menu.QueryContextMenu(hmenu, 0, FIRST_ID, LAST_ID, flags) };
        timings.query = start.elapsed();
        let built = Self {
            menu2: menu.cast().ok(),
            menu3: menu.cast().ok(),
            menu,
            hmenu,
            directory: items.directory,
            timings,
        };
        filled.ok()?;
        Ok(built)
    }

    /// The menu's items, with every submenu as it is now (lazy ones are empty until opened).
    pub fn items(&self) -> Vec<MenuItem> {
        self.read(self.hmenu, 0)
    }

    /// Opens a submenu the way the menu loop does when the user reaches it: sends the
    /// handlers `WM_INITMENUPOPUP` (what the owner's subclass forwards), so lazy submenus fill.
    /// Returns its items afterwards.
    pub fn open(&self, submenu: &Submenu) -> Vec<MenuItem> {
        let handle = HMENU(submenu.handle as *mut core::ffi::c_void);
        let _ = self.forward(
            WM_INITMENUPOPUP,
            WPARAM(submenu.handle as usize),
            LPARAM(submenu.position as isize),
        );
        self.read(handle, 1)
    }

    /// The canonical verb of the command at `offset`, if its handler reports one.
    pub fn verb(&self, offset: u32) -> Option<String> {
        let mut buffer = [0u16; 256];
        // SAFETY: with GCS_VERBW the handler writes at most `cchmax` UTF-16 units into the
        // buffer, which is that long; handlers without verbs just fail.
        unsafe {
            self.menu.GetCommandString(
                offset as usize,
                GCS_VERBW,
                None,
                PSTR(buffer.as_mut_ptr().cast()),
                buffer.len() as u32,
            )
        }
        .ok()?;
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        (len > 0).then(|| String::from_utf16_lossy(&buffer[..len]))
    }

    /// Runs the command at `offset`, as Explorer does: Unicode, at the menu's point, with the
    /// Shift and Ctrl state (Shift+Delete is a permanent delete), and allowed to finish on its
    /// own thread (`ASYNCOK`).
    pub fn invoke(&self, owner: isize, offset: u32, x: i32, y: i32) -> windows::core::Result<()> {
        // CMIC_MASK_UNICODE and CMIC_MASK_ASYNCOK are the SEE_MASK_ values of the same name.
        let mut mask = SEE_MASK_UNICODE | SEE_MASK_ASYNCOK | CMIC_MASK_PTINVOKE;
        if key_down(VK_SHIFT.0) {
            mask |= CMIC_MASK_SHIFT_DOWN;
        }
        if key_down(VK_CONTROL.0) {
            mask |= CMIC_MASK_CONTROL_DOWN;
        }
        let info = CMINVOKECOMMANDINFOEX {
            cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            fMask: mask,
            hwnd: hwnd(owner),
            // MAKEINTRESOURCE(offset): a command by offset, not by verb.
            lpVerb: PCSTR(offset as usize as *const u8),
            lpVerbW: PCWSTR(offset as usize as *const u16),
            lpDirectoryW: PCWSTR(self.directory.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ptInvoke: POINT { x, y },
            ..Default::default()
        };
        // SAFETY: CMINVOKECOMMANDINFOEX starts with CMINVOKECOMMANDINFO and `cbSize` tells the
        // handler which one it got; `directory` outlives the call.
        unsafe {
            self.menu
                .InvokeCommand((&raw const info).cast::<CMINVOKECOMMANDINFO>())
        }
    }

    /// Hands a menu message to the handlers; `Some` when one handled it.
    fn forward(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        if let Some(menu3) = &self.menu3 {
            let mut result = LRESULT(0);
            // SAFETY: a live object and a valid out pointer.
            let handled = unsafe { menu3.HandleMenuMsg2(msg, wparam, lparam, Some(&mut result)) };
            return handled.is_ok().then_some(result);
        }
        if let Some(menu2) = &self.menu2 {
            // IContextMenu2 predates WM_MENUCHAR.
            if msg == WM_MENUCHAR {
                return None;
            }
            // SAFETY: a live object.
            let handled = unsafe { menu2.HandleMenuMsg(msg, wparam, lparam) };
            return handled.is_ok().then_some(LRESULT(0));
        }
        None
    }

    /// Runs the menu's modal loop with the owner subclassed.
    fn track(
        &self,
        owner: isize,
        x: i32,
        y: i32,
        forward: bool,
        on_open: Option<Box<dyn FnOnce()>>,
    ) -> Tracked {
        let tracking = Tracking {
            menu: self,
            forward,
            on_open: Cell::new(on_open),
            popups: RefCell::default(),
            forwarded: Cell::default(),
        };
        let owner = hwnd(owner);
        // SAFETY: `tracking` outlives the subclass, which is removed below before it drops.
        let subclassed = unsafe {
            SetWindowSubclass(
                owner,
                Some(subclass_proc),
                SUBCLASS_ID,
                &raw const tracking as usize,
            )
        }
        .as_bool();
        if !subclassed {
            tracing::warn!("SetWindowSubclass failed; lazy submenus will stay empty");
        }
        let start = Instant::now();
        // SAFETY: our menu, on the thread that owns `owner`; TPM_RETURNCMD returns the chosen
        // id (0 for none) instead of posting WM_COMMAND.
        let id = unsafe {
            TrackPopupMenuEx(
                self.hmenu,
                (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
                x,
                y,
                owner,
                None,
            )
        }
        .0 as u32;
        let elapsed = start.elapsed();
        if subclassed {
            // SAFETY: removes the subclass installed above.
            let _ = unsafe { RemoveWindowSubclass(owner, Some(subclass_proc), SUBCLASS_ID) };
        }
        let items = self.items();
        let popups = tracking
            .popups
            .take()
            .into_iter()
            .map(|(mut popup, handle)| {
                popup.text = submenu_text(&items, handle).unwrap_or_default();
                popup
            })
            .collect();
        Tracked {
            offset: (FIRST_ID..=LAST_ID).contains(&id).then(|| id - FIRST_ID),
            elapsed,
            popups,
            forwarded: tracking.forwarded.get(),
        }
    }

    fn text_of(&self, offset: u32) -> Option<String> {
        fn find(items: &[MenuItem], offset: u32) -> Option<String> {
            items.iter().find_map(|item| {
                if item.offset == Some(offset) {
                    return Some(item.text.clone());
                }
                item.submenu
                    .as_ref()
                    .and_then(|submenu| find(&submenu.items, offset))
            })
        }
        find(&self.items(), offset)
    }

    fn read(&self, hmenu: HMENU, depth: u32) -> Vec<MenuItem> {
        // SAFETY: a menu handle; a destroyed one returns -1.
        let count = unsafe { GetMenuItemCount(Some(hmenu)) }.max(0) as u32;
        (0..count)
            .map(|position| self.read_item(hmenu, position, depth))
            .collect()
    }

    fn read_item(&self, hmenu: HMENU, position: u32, depth: u32) -> MenuItem {
        let mut info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_SUBMENU | MIIM_STRING | MIIM_BITMAP,
            ..Default::default()
        };
        // SAFETY: a valid MENUITEMINFOW with a null buffer, which asks for the text length.
        if unsafe { GetMenuItemInfoW(hmenu, position, true, &mut info) }.is_err() {
            return MenuItem::default();
        }
        let mut text = vec![0u16; info.cch as usize + 1];
        if info.cch > 0 {
            info.dwTypeData = windows::core::PWSTR(text.as_mut_ptr());
            info.cch += 1;
            // SAFETY: the buffer holds `cch` units, the text and its NUL.
            let _ = unsafe { GetMenuItemInfoW(hmenu, position, true, &mut info) };
        }
        let len = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        let mut item = MenuItem {
            text: clean_text(&String::from_utf16_lossy(&text[..len])),
            separator: info.fType.0 & MFT_SEPARATOR.0 != 0,
            disabled: info.fState.0 & MFS_DISABLED.0 != 0,
            default: info.fState.0 & MFS_DEFAULT.0 != 0,
            owner_draw: info.fType.0 & MFT_OWNERDRAW.0 != 0,
            ..Default::default()
        };
        if !info.hSubMenu.is_invalid() {
            item.submenu = Some(Submenu {
                handle: info.hSubMenu.0 as isize,
                position,
                // Deep enough for every handler seen; guards against a menu that loops.
                items: if depth < 4 {
                    self.read(info.hSubMenu, depth + 1)
                } else {
                    Vec::new()
                },
            });
        } else if !item.separator && (FIRST_ID..=LAST_ID).contains(&info.wID) {
            let offset = info.wID - FIRST_ID;
            item.offset = Some(offset);
            item.verb = self.verb(offset);
        }
        item
    }
}

/// The text of the item that opens the submenu `handle`.
fn submenu_text(items: &[MenuItem], handle: isize) -> Option<String> {
    items.iter().find_map(|item| {
        let submenu = item.submenu.as_ref()?;
        if submenu.handle == handle {
            return Some(item.text.clone());
        }
        submenu_text(&submenu.items, handle)
    })
}

impl Drop for ShellMenu {
    fn drop(&mut self) {
        // SAFETY: we created the menu and destroy it once; submenus go with it.
        let _ = unsafe { DestroyMenu(self.hmenu) };
    }
}

/// What `track` saw.
struct Tracked {
    offset: Option<u32>,
    elapsed: Duration,
    popups: Vec<Popup>,
    forwarded: Forwarded,
}

/// The subclass's state, on `track`'s stack while the menu is up.
struct Tracking<'a> {
    menu: &'a ShellMenu,
    forward: bool,
    on_open: Cell<Option<Box<dyn FnOnce()>>>,
    popups: RefCell<Vec<(Popup, isize)>>,
    forwarded: Cell<Forwarded>,
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: `track` passed a pointer to its `Tracking`, which outlives the subclass.
    let tracking = unsafe { &*(data as *const Tracking) };
    // Menus send WM_MEASUREITEM and WM_DRAWITEM with wParam 0; anything else is a control's.
    let menu_message = match msg {
        WM_INITMENUPOPUP | WM_MENUCHAR => true,
        WM_MEASUREITEM | WM_DRAWITEM => wparam.0 == 0,
        _ => false,
    };
    if menu_message && tracking.forward {
        // A panic must not unwind into the menu loop (it would abort the process).
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            handle_menu_message(tracking, msg, wparam, lparam)
        }));
        if let Ok(Some(result)) = result {
            return result;
        }
    } else if msg == WM_INITMENUPOPUP {
        notify_open(tracking, wparam);
    }
    // SAFETY: passes the message on to the next subclass and the window's procedure.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

fn handle_menu_message(
    tracking: &Tracking,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let mut forwarded = tracking.forwarded.get();
    let result = match msg {
        WM_INITMENUPOPUP => {
            forwarded.init_popup += 1;
            let popup = HMENU(wparam.0 as *mut core::ffi::c_void);
            // SAFETY: the popup being opened; a bad handle returns -1.
            let before = unsafe { GetMenuItemCount(Some(popup)) };
            let start = Instant::now();
            let result = tracking.menu.forward(msg, wparam, lparam);
            let elapsed = start.elapsed();
            // SAFETY: as above.
            let after = unsafe { GetMenuItemCount(Some(popup)) };
            tracking.popups.borrow_mut().push((
                Popup {
                    text: String::new(),
                    elapsed,
                    items_before: before,
                    items_after: after,
                },
                wparam.0 as isize,
            ));
            notify_open(tracking, wparam);
            result
        }
        WM_MEASUREITEM => {
            forwarded.measure_item += 1;
            tracking.menu.forward(msg, wparam, lparam)
        }
        WM_DRAWITEM => {
            forwarded.draw_item += 1;
            tracking.menu.forward(msg, wparam, lparam)
        }
        _ => {
            forwarded.menu_char += 1;
            tracking.menu.forward(msg, wparam, lparam)
        }
    };
    tracking.forwarded.set(forwarded);
    result
}

/// Runs `on_open` when the root menu initializes, i.e. once its modal loop is running.
fn notify_open(tracking: &Tracking, wparam: WPARAM) {
    if wparam.0 as isize != tracking.menu.hmenu.0 as isize {
        return;
    }
    if let Some(on_open) = tracking.on_open.take() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(on_open));
    }
}

/// The items' folder and their IDs relative to it.
struct Items {
    folder: IShellFolder,
    /// Absolute ID lists; `children` point into them.
    _pidls: Vec<Pidl>,
    children: Vec<*const ITEMIDLIST>,
    directory: Vec<u16>,
}

impl Items {
    fn resolve(paths: &[PathBuf]) -> Result<Self, MenuError> {
        let plain: Vec<PathBuf> = paths.iter().map(|path| shell_path(path)).collect();
        let first = plain.first().ok_or(MenuError::Empty)?;
        let pidls = plain
            .iter()
            .map(|path| Pidl::from_path(path).ok_or_else(|| MenuError::Path { path: path.clone() }))
            .collect::<Result<Vec<_>, _>>()?;
        let parent = pidls[0].parent().ok_or_else(|| MenuError::Path {
            path: first.clone(),
        })?;
        for pidl in &pidls[1..] {
            // SAFETY: two valid absolute ID lists.
            if !unsafe { ILIsParent(parent.as_ptr(), pidl.as_ptr(), true) }.as_bool() {
                return Err(MenuError::MixedFolders);
            }
        }
        // SAFETY: a valid absolute ID list; the last-ID out pointer is optional.
        let folder: IShellFolder = unsafe { SHBindToParent(pidls[0].as_ptr(), None) }?;
        let children = pidls
            .iter()
            // SAFETY: valid ID lists; the result points into them.
            .map(|pidl| unsafe { ILFindLastID(pidl.as_ptr()) }.cast_const())
            .collect();
        let directory = first
            .parent()
            .map(|dir| dir.as_os_str().encode_wide().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .chain(Some(0))
            .collect();
        Ok(Self {
            folder,
            _pidls: pidls,
            children,
            directory,
        })
    }
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

    /// A copy without the last ID: the parent folder.
    fn parent(&self) -> Option<Self> {
        // SAFETY: a valid ID list; the copy is ours.
        let copy = unsafe { ILClone(self.0) };
        if copy.is_null() {
            return None;
        }
        // SAFETY: our own copy; removing its last ID edits it in place.
        let _ = unsafe { ILRemoveLastID(Some(copy)) };
        Some(Self(copy))
    }

    fn as_ptr(&self) -> *const ITEMIDLIST {
        self.0
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        // SAFETY: allocated by the Shell for us and freed only here, once.
        unsafe { ILFree(Some(self.0)) };
    }
}

fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut core::ffi::c_void)
}

fn key_down(key: u16) -> bool {
    // SAFETY: a plain query of the thread's input state.
    let state = unsafe { GetKeyState(i32::from(key)) };
    state < 0
}

/// A menu item's text without the `&` mnemonic marks and the tab-separated accelerator.
pub fn clean_text(text: &str) -> String {
    let label = text.split('\t').next().unwrap_or_default();
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            // "&&" is a literal ampersand.
            if chars.peek() == Some(&'&') {
                out.push('&');
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// An OLE STA, as Jerry's main thread is.
    fn sta() {
        // OLE and a window (so the thread has a message queue), like Jerry's main thread.
        crate::main_thread::init().unwrap();
    }

    /// A lab with a Georgian file, a plain one, a folder and a file in another folder.
    fn lab() -> (tempfile::TempDir, Vec<PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        let drawing = dir.path().join("ნახაზი — სართული 1.dwg");
        let plain = dir.path().join("plain.txt");
        let folder = dir.path().join("პროექტი");
        let other = dir.path().join("other").join("elsewhere.txt");
        fs::write(&drawing, b"jerry").unwrap();
        fs::write(&plain, b"jerry").unwrap();
        fs::create_dir(&folder).unwrap();
        fs::create_dir(other.parent().unwrap()).unwrap();
        fs::write(&other, b"jerry").unwrap();
        (dir, vec![drawing, plain, folder, other])
    }

    fn verbs(items: &[MenuItem]) -> Vec<String> {
        items
            .iter()
            .flat_map(|item| {
                let own = item.verb.clone().into_iter();
                let nested = item
                    .submenu
                    .as_ref()
                    .map(|submenu| verbs(&submenu.items))
                    .unwrap_or_default();
                own.chain(nested)
            })
            .collect()
    }

    #[test]
    fn shell_path_drops_the_verbatim_prefix() {
        assert_eq!(
            shell_path(Path::new(r"\\?\C:\ნახაზები\a.dwg")),
            PathBuf::from(r"C:\ნახაზები\a.dwg")
        );
        assert_eq!(
            shell_path(Path::new(r"\\?\UNC\server\share\a.dwg")),
            PathBuf::from(r"\\server\share\a.dwg")
        );
        assert_eq!(
            shell_path(Path::new(r"C:\a.dwg")),
            PathBuf::from(r"C:\a.dwg")
        );
    }

    #[test]
    fn clean_text_drops_mnemonics_and_accelerators() {
        assert_eq!(clean_text("Se&nd to"), "Send to");
        assert_eq!(clean_text("&Save && close\tCtrl+S"), "Save & close");
        assert_eq!(clean_text("Add to \"ნახაზი.rar\""), "Add to \"ნახაზი.rar\"");
    }

    #[test]
    fn file_folder_and_several_files_get_explorers_verbs() {
        sta();
        let (_dir, paths) = lab();
        for selection in [&paths[0..1], &paths[2..3], &paths[0..3]] {
            let menu = ShellMenu::new(0, selection, false).unwrap();
            let verbs = verbs(&menu.items());
            for expected in ["copy", "delete", "rename", "properties"] {
                assert!(
                    verbs.iter().any(|verb| verb.eq_ignore_ascii_case(expected)),
                    "{expected} missing for {selection:?}: {verbs:?}"
                );
            }
        }
    }

    #[test]
    fn long_and_verbatim_paths_get_a_menu() {
        sta();
        let dir = tempfile::tempdir().unwrap();
        let mut deep = dir.path().to_path_buf();
        for level in 0..6 {
            deep.push(format!("ნახაზები-{level}-{}", "x".repeat(40)));
        }
        // std creates paths past MAX_PATH itself; the menu gets core's verbatim form.
        fs::create_dir_all(&deep).unwrap();
        let file = deep.join("გრძელი.txt");
        fs::write(&file, b"jerry").unwrap();
        assert!(file.as_os_str().len() > 300);
        let verbatim = PathBuf::from(format!(r"\\?\{}", file.display()));

        let menu = ShellMenu::new(0, &[verbatim], false).unwrap();
        assert!(verbs(&menu.items()).iter().any(|verb| verb == "properties"));
    }

    #[test]
    fn refuses_mixed_folders_missing_files_and_nothing() {
        sta();
        let (dir, paths) = lab();
        assert!(matches!(
            ShellMenu::new(0, &[paths[0].clone(), paths[3].clone()], false),
            Err(MenuError::MixedFolders)
        ));
        assert!(matches!(
            ShellMenu::new(0, &[dir.path().join("gone.txt")], false),
            Err(MenuError::Path { .. })
        ));
        assert!(matches!(
            ShellMenu::new(0, &[], false),
            Err(MenuError::Empty)
        ));
    }

    #[test]
    fn a_second_menu_while_one_is_open_is_busy() {
        let _open = OPEN.swap(true, Ordering::AcqRel);
        let result = show(0, &[], 0, 0, MenuOptions::default());
        OPEN.store(false, Ordering::Release);
        assert!(matches!(result, Err(MenuError::Busy)));
    }

    /// The forwarding the owner's subclass does: lazy submenus ("Open with", "Give access to",
    /// WinRAR) have one placeholder until WM_INITMENUPOPUP reaches their handler. Ignored in
    /// CI: which lazy submenus exist depends on the machine.
    #[test]
    #[ignore = "needs a desktop shell with lazy submenus; run with --ignored"]
    fn opening_a_submenu_fills_it() {
        sta();
        let (_dir, paths) = lab();
        let menu = ShellMenu::new(0, &paths[0..1], false).unwrap();
        let lazy: Vec<(String, usize, usize)> = menu
            .items()
            .iter()
            .filter_map(|item| {
                let submenu = item.submenu.as_ref()?;
                let before = submenu.items.len();
                let after = menu.open(submenu).len();
                Some((item.text.clone(), before, after))
            })
            .collect();
        println!("{lazy:?}");
        assert!(
            lazy.iter().any(|(_, before, after)| after > before),
            "no submenu filled on WM_INITMENUPOPUP: {lazy:?}"
        );
    }

    /// Runs "Send to → Compressed (zipped) folder" without showing the menu and waits for the
    /// archive. Ignored in CI: it needs the zip handler and an English Send To.
    #[test]
    #[ignore = "needs the desktop shell's Send To; run with --ignored"]
    fn send_to_compressed_folder_zips_a_georgian_file() {
        sta();
        let (dir, paths) = lab();
        let menu = ShellMenu::new(0, &paths[0..1], false).unwrap();
        // Send To fills its submenu from the thread's message loop (the menu loop, in Jerry).
        pump(Duration::from_millis(500));
        let items = menu.items();
        let send_to = items
            .iter()
            .find_map(|item| item.submenu.as_ref().filter(|_| item.text == "Send to"))
            .expect("no Send to");
        let zip = menu
            .open(send_to)
            .into_iter()
            .find(|item| item.text.starts_with("Compressed"))
            .and_then(|item| item.offset)
            .unwrap_or_else(|| panic!("no Compressed (zipped) folder in {:?}", menu.open(send_to)));
        menu.invoke(0, zip, 0, 0).unwrap();

        let archive = dir.path().join("ნახაზი — სართული 1.zip");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !archive.exists() && Instant::now() < deadline {
            pump(Duration::from_millis(100));
        }
        assert!(archive.exists(), "no {}", archive.display());
    }

    fn pump(duration: Duration) {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
        };
        let end = Instant::now() + duration;
        while Instant::now() < end {
            let mut msg = MSG::default();
            // SAFETY: a valid MSG out pointer; dispatching to this thread's own windows.
            while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                // SAFETY: as above.
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
