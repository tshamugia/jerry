//! Opening files in their programs, as Explorer does on Enter or a double click: `ShellExecuteExW`
//! with the default verb.
//!
//! Every request gets its own short-lived STA thread rather than a Shell pool worker. Opening can
//! take seconds (a program that answers by DDE, AutoCAD with a DWG say, starts up first), and a
//! pool worker held that long would hold up icons and thumbnails. A hung handler hangs only its
//! own thread; nothing waits for it.
//!
//! Windows shows its own messages, owned by Jerry's window, as for Explorer: "How do you want to
//! open this file?" for a type without a program, and its error box when a file can't be opened.
//!
//! Also here: showing files in Explorer and opening a terminal in a folder, the context menu's
//! "Show in Explorer" and "Open in Terminal".

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use jerry_core::path::plain_path;
use jerry_core::{ErrorCode, JerryError};
use windows::Win32::Foundation::{CloseHandle, HWND, MAX_PATH};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize, IBindCtx,
};
use windows::Win32::System::Threading::{
    CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, CreateProcessW, PROCESS_CREATION_FLAGS,
    PROCESS_INFORMATION, STARTUPINFOW,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    FOLDERID_System, ILFree, SEE_MASK_FLAG_LOG_USAGE, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
    SHOpenFolderAndSelectItems, SHParseDisplayName, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, PWSTR};

use crate::folders::known_folder;

/// Opens `files` in their programs, one after another, on a thread of their own, with Windows'
/// messages owned by the window `owner`. Returns at once; the receiver gets the outcome when the
/// last one has started: the first error, if any. A cancelled prompt (Windows' "Open with", say)
/// isn't an error.
pub fn open_files(
    owner: Option<isize>,
    files: Vec<PathBuf>,
) -> mpsc::Receiver<Result<(), JerryError>> {
    on_own_thread("open files", move || open_all(owner, &files))
}

/// Shows `items`, entries of the folder `folder`, selected in an Explorer window on that folder
/// ("Show in Explorer"); with no items, just the folder. Explorer may take a moment to start, so
/// this runs on a thread of its own, like [`open_files`], and returns at once; the receiver gets
/// the outcome. Explorer reuses a window already showing the folder, as it does for others.
pub fn show_in_explorer(
    folder: PathBuf,
    items: Vec<PathBuf>,
) -> mpsc::Receiver<Result<(), JerryError>> {
    on_own_thread("show in Explorer", move || {
        in_sta(|| select_in_explorer(&folder, &items))
    })
}

fn select_in_explorer(folder: &Path, items: &[PathBuf]) -> Result<(), JerryError> {
    let folder_id = ItemId::parse(folder)?;
    let ids = items
        .iter()
        .map(|item| ItemId::parse(item))
        .collect::<Result<Vec<_>, _>>()?;
    let children: Vec<*const ITEMIDLIST> = ids.iter().map(|id| id.0.cast_const()).collect();
    // SAFETY: absolute item ID lists the Shell made, alive until after the call.
    unsafe { SHOpenFolderAndSelectItems(folder_id.0, Some(&children), 0) }
        .map_err(|error| JerryError::from(error).at(folder))
}

/// A Shell item's absolute ID list, freed when dropped.
struct ItemId(*mut ITEMIDLIST);

impl ItemId {
    /// The item at `path`. The Shell takes paths longer than MAX_PATH without the `\\?\` prefix,
    /// but not with it (spike #15). `NotFound` and the like, with the path, if it can't.
    fn parse(path: &Path) -> Result<Self, JerryError> {
        let name = wide(&plain_path(path));
        let mut id = std::ptr::null_mut();
        // SAFETY: `name` is NUL-terminated and outlives the call; `id` takes the result.
        unsafe { SHParseDisplayName(PCWSTR(name.as_ptr()), None::<&IBindCtx>, &mut id, 0, None) }
            .map_err(|error| JerryError::from(error).at(path))?;
        Ok(Self(id))
    }
}

impl Drop for ItemId {
    fn drop(&mut self) {
        // SAFETY: an ID list the Shell allocated for us, freed once.
        unsafe { ILFree(Some(self.0.cast_const())) };
    }
}

/// Opens a terminal in `folder` ("Open in Terminal"): Windows Terminal with the user's default
/// profile, or Windows PowerShell where Windows Terminal isn't installed (or won't start). The
/// folder is the program's working folder, so no path goes through a command line, whose
/// quoting Windows Terminal reads its own way (`;` splits commands). Returns once it has started.
pub fn open_terminal(folder: &Path) -> Result<(), JerryError> {
    let directory = shell_form(folder);
    // Never from Jerry's working folder: only a folder of `PATH`, where its alias lives.
    let terminal = std::env::var_os("PATH").and_then(|path| find_program(&path, "wt.exe"));
    if let Some(terminal) = terminal {
        // `-d .`: a new tab in the working folder (Windows Terminal's own example). Windows
        // Terminal is a window app; the flag only keeps a console from flashing if it isn't.
        match start(&terminal, "wt.exe -d .", &directory, CREATE_NO_WINDOW) {
            Ok(()) => return Ok(()),
            Err(error) => tracing::warn!(code = ?error.code, "Windows Terminal didn't start"),
        }
    }
    let powershell = known_folder(&FOLDERID_System)?.join(r"WindowsPowerShell\v1.0\powershell.exe");
    // A console of its own, even when Jerry has one (a dev build).
    start(
        &powershell,
        "powershell.exe",
        &directory,
        CREATE_NEW_CONSOLE,
    )
    .map_err(|error| error.at(folder))
}

/// The program `name` in the first folder of `path` (a `PATH` value) that has it. Relative
/// folders are skipped: they would be looked up from Jerry's working folder. An app execution
/// alias (`wt.exe` in `WindowsApps`) is a reparse point that can't be followed, so it counts as
/// long as it is there.
fn find_program(path: &OsStr, name: &str) -> Option<PathBuf> {
    std::env::split_paths(path)
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join(name))
        .find(|program| std::fs::symlink_metadata(program).is_ok())
}

/// Starts `program` with `command_line` (its own name first) in the working folder `directory`,
/// without waiting for it.
fn start(
    program: &Path,
    command_line: &str,
    directory: &Path,
    flags: PROCESS_CREATION_FLAGS,
) -> Result<(), JerryError> {
    let program = wide(program);
    // CreateProcessW may write to the command line.
    let mut command_line: Vec<u16> = command_line.encode_utf16().chain([0]).collect();
    let directory = wide(directory);
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    // SAFETY: NUL-terminated strings that outlive the call, a writable command line, and
    // structures as documented.
    unsafe {
        CreateProcessW(
            PCWSTR(program.as_ptr()),
            Some(PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            false,
            flags,
            None,
            PCWSTR(directory.as_ptr()),
            &startup,
            &mut process,
        )
    }?;
    // SAFETY: the handles CreateProcessW just gave us, closed once; Jerry doesn't wait for it.
    unsafe {
        let _ = CloseHandle(process.hThread);
        let _ = CloseHandle(process.hProcess);
    }
    Ok(())
}

/// Runs `job` on a short-lived thread of its own and returns at once; the receiver gets its
/// outcome. A panic fails only the job.
fn on_own_thread(
    name: &'static str,
    job: impl FnOnce() -> Result<(), JerryError> + Send + 'static,
) -> mpsc::Receiver<Result<(), JerryError>> {
    let (sender, outcome) = mpsc::channel();
    let started = std::thread::Builder::new()
        .name("jerry-open".into())
        .spawn(move || {
            let result = jerry_core::job::run_job(name, job);
            let _ = sender.send(result);
        });
    if let Err(error) = started {
        tracing::error!(%error, job = name, "couldn't start a thread");
        let (sender, failed) = mpsc::channel();
        let _ = sender.send(Err(JerryError::internal()));
        return failed;
    }
    outcome
}

fn open_all(owner: Option<isize>, files: &[PathBuf]) -> Result<(), JerryError> {
    in_sta(|| {
        let mut outcome = Ok(());
        for file in files {
            if let Err(error) = open_file(owner, file, 0) {
                tracing::info!(code = ?error.code, "couldn't open a file");
                outcome = outcome.and(Err(error));
            }
        }
        outcome
    })
}

/// Runs `job` with this thread in an STA of its own, for the length of the job: handlers that
/// COM activates need it (ShellExecuteEx's documentation).
pub(crate) fn in_sta<T>(job: impl FnOnce() -> T) -> T {
    // SAFETY: no reserved pointer; this thread's apartment, ended below.
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    let result = job();
    if com.is_ok() {
        // SAFETY: matches the successful CoInitializeEx above, on the same thread.
        unsafe { CoUninitialize() };
    }
    result
}

/// Opens a Shell name (`shell:RecycleBinFolder`) with its default verb, in Explorer for a folder.
pub(crate) fn open_shell_name(owner: Option<isize>, name: PCWSTR) -> Result<(), JerryError> {
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        hwnd: HWND(owner.unwrap_or(0) as *mut core::ffi::c_void),
        lpFile: name,
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` is filled in as documented; `name` is NUL-terminated and outlives the call.
    unsafe { ShellExecuteExW(&mut info) }.map_err(JerryError::from)
}

/// Opens `file` with its default verb, waiting until the program has it (`SEE_MASK_NOASYNC`, as
/// this thread ends soon after). `flags` adds `SEE_MASK_*` flags (tests turn Windows' messages off).
fn open_file(owner: Option<isize>, file: &Path, flags: u32) -> Result<(), JerryError> {
    let path = shell_form(file);
    let wide_file = wide(&path);
    let folder = path.parent().map(wide);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        // Log usage: Windows' frequent programs and Jump Lists, as when Explorer opens it.
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_LOG_USAGE | flags,
        hwnd: HWND(owner.unwrap_or(0) as *mut core::ffi::c_void),
        lpFile: PCWSTR(wide_file.as_ptr()),
        // Programs that open files next to it find them, as when opened from Explorer.
        lpDirectory: folder
            .as_ref()
            .map_or(PCWSTR::null(), |folder| PCWSTR(folder.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` is filled in as documented; its strings are NUL-terminated and outlive the
    // call.
    match unsafe { ShellExecuteExW(&mut info) } {
        Ok(()) => Ok(()),
        Err(error) => match JerryError::from(error) {
            error if error.code == ErrorCode::Cancelled => Ok(()),
            error => Err(error.at(file)),
        },
    }
}

/// The path as the Shell takes it: without the `\\?\` prefix while it is short enough, which most
/// programs need; a longer one keeps it, the only way to name it at all.
fn shell_form(file: &Path) -> PathBuf {
    let plain = plain_path(file);
    if plain.as_os_str().encode_wide().count() < MAX_PATH as usize {
        plain
    } else {
        file.to_path_buf()
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::Shell::SEE_MASK_FLAG_NO_UI;

    use super::*;

    #[test]
    fn short_paths_lose_the_prefix_and_long_ones_keep_it() {
        assert_eq!(
            shell_form(Path::new(r"\\?\C:\Work\ნახაზი.dwg")),
            PathBuf::from(r"C:\Work\ნახაზი.dwg")
        );
        let long = format!(r"\\?\C:\{}\a.txt", "x".repeat(300));
        assert_eq!(shell_form(Path::new(&long)), PathBuf::from(&long));
    }

    /// A temp folder by its long-path form, as core keeps folders.
    fn temp() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = std::fs::canonicalize(dir.path()).expect("canonical");
        (dir, path)
    }

    #[test]
    fn explorer_gets_items_by_their_long_path_too() {
        let (_dir, dir) = temp();
        let file = dir.join("ნახაზი — სართული 1.dwg");
        std::fs::write(&file, b"AC1032").expect("write");
        let mut long = dir.clone();
        while long.as_os_str().len() < 300 {
            long.push("ძალიან გრძელი საქაღალდის სახელი");
        }
        std::fs::create_dir_all(&long).expect("long folders");
        let missing = dir.join("არ არსებობს.txt");

        in_sta(|| {
            for path in [&dir, &file, &long] {
                assert!(ItemId::parse(path).is_ok(), "{}", path.display());
            }
            let error = ItemId::parse(&missing).err().expect("nothing there");
            assert_eq!(error.code, ErrorCode::NotFound);
            assert_eq!(error.path.as_deref(), Some(missing.as_path()));
        });
    }

    #[test]
    fn programs_come_from_absolute_path_folders_only() {
        let (_dir, dir) = temp();
        let bin = dir.join("ბინ");
        std::fs::create_dir(&bin).expect("folder");
        std::fs::write(bin.join("wt.exe"), b"").expect("program");
        let path = std::env::join_paths([dir.join("empty"), bin.clone()]).expect("PATH");
        assert_eq!(find_program(&path, "wt.exe"), Some(bin.join("wt.exe")));
        assert_eq!(find_program(OsStr::new(""), "wt.exe"), None);
        // Tests run in the crate's folder, which has a Cargo.toml: a relative folder isn't looked in.
        assert!(Path::new("Cargo.toml").exists());
        assert_eq!(find_program(OsStr::new("."), "Cargo.toml"), None);
    }

    #[test]
    fn a_program_starts_in_its_folder_and_a_missing_folder_fails() {
        let (_dir, dir) = temp();
        let folder = dir.join("ტერმინალი");
        std::fs::create_dir(&folder).expect("folder");
        let cmd = known_folder(&FOLDERID_System)
            .expect("System32")
            .join("cmd.exe");

        let missing = shell_form(&dir.join("არ არის"));
        assert!(start(&cmd, "cmd.exe /c exit", &missing, CREATE_NO_WINDOW).is_err());

        // A relative name: it lands in the working folder.
        let working = shell_form(&folder);
        start(
            &cmd,
            "cmd.exe /c echo jerry> here.txt",
            &working,
            CREATE_NO_WINDOW,
        )
        .expect("starts");
        let here = folder.join("here.txt");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !std::fs::read(&here).is_ok_and(|text| text.starts_with(b"jerry")) {
            assert!(std::time::Instant::now() < deadline, "cmd.exe didn't write");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn a_missing_file_fails_without_opening_anything() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing =
            jerry_core::path::long_path(&dir.path().join("ნახაზი.jerry-test")).expect("absolute");
        // Without Windows' own message, which would wait for a click.
        let error = open_file(None, &missing, SEE_MASK_FLAG_NO_UI).expect_err("nothing to open");
        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(error.path.as_deref(), Some(missing.as_path()));
    }
}
