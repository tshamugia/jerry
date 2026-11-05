//! The Recycle Bin: Del deletes with the Shell's `IFileOperation`, as Explorer does, so the items
//! show in Windows' Recycle Bin and Explorer's Restore puts them back (Tech PRD «Recycle Bin»).
//! Flags `FOFX_RECYCLEONDELETE | FOF_ALLOWUNDO | FOF_NO_UI`. Run [`recycle`] on the Shell pool
//! (STA) with [`RECYCLE_TIMEOUT`].
//!
//! The Shell recycles only where it can: not on a network or removable drive, not past
//! `MAX_PATH` on a volume without 8.3 names, not what is bigger than the Recycle Bin or where it
//! is turned off. Everywhere else, with `FOF_NO_UI`, it deletes for good and says nothing: #37
//! measured it on a share (`\\localhost\C$`) and on a 337-character path on a drive without 8.3
//! names. Worse, it decides once for the whole operation: one such item among many, and it
//! deletes all of them for good, the ones it could recycle too. So:
//!
//! - Items on a drive that isn't fixed (network, removable, optical, RAM disk) have no Recycle
//!   Bin: they come back as [`Outcome::NoRecycleBin`] without asking the Shell.
//! - A progress sink watches the operation. When the Shell is about to delete an item without
//!   `TSF_DELETE_RECYCLE_IF_POSSIBLE` in `PreDeleteItem`'s flags, the sink vetoes it with
//!   `COPYENGINE_E_RECYCLE_BIN_NOT_FOUND`, which stops the operation before anything is deleted.
//!   (A success code such as `COPYENGINE_S_USER_IGNORED` doesn't skip it: the Shell then deletes
//!   everything for good.)
//! - Items a stopped operation never got to go again, each in its own operation, so a veto
//!   names the item the Shell can't recycle. That one is [`Outcome::NoRecycleBin`] and still
//!   where it was, for the UI to ask whether to delete it for good (data safety rule 5).
//!
//! Nothing the Shell decides on its own is deleted for good. `PostDeleteItem` names each recycled
//! item's place in the Recycle Bin (`$Recycle.Bin\<SID>\$R…`): Undo (#56) restores from it.

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use jerry_core::{ErrorCode, JerryError, volume};
use windows::Win32::Storage::FileSystem::GetDriveTypeW;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::{
    COPYENGINE_E_ACCESS_DENIED_SRC, COPYENGINE_E_CANCELLED, COPYENGINE_E_PATH_NOT_FOUND_SRC,
    COPYENGINE_E_RECYCLE_BIN_NOT_FOUND, COPYENGINE_E_REQUIRES_ELEVATION,
    COPYENGINE_E_SHARING_VIOLATION_SRC, COPYENGINE_E_USER_CANCELLED, FILEOPERATION_FLAGS,
    FOF_ALLOWUNDO, FOF_NO_UI, FOFX_RECYCLEONDELETE, FileOperation, IFileOperation,
    IFileOperationProgressSink, IFileOperationProgressSink_Impl, IShellItem,
    SHCreateItemFromParsingName, SIGDN, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH,
    TSF_DELETE_RECYCLE_IF_POSSIBLE,
};
use windows::core::{ComObject, HRESULT, IUnknown, Interface, PCWSTR, Ref, implement};

/// How long one recycle may take on the Shell pool. Long: a big folder takes a while, and the
/// Shell's work goes on even past it (only the answer is lost). It is there for a shell
/// extension that hangs.
pub const RECYCLE_TIMEOUT: Duration = Duration::from_secs(600);

/// What happened to one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// In the Recycle Bin. `in_bin` is where (`C:\$Recycle.Bin\<SID>\$R…`), if the Shell said.
    Recycled { in_bin: Option<PathBuf> },
    /// The Shell can't recycle it here; it is untouched.
    NoRecycleBin,
    /// It couldn't be deleted; it is untouched.
    Failed(JerryError),
}

/// `GetDriveTypeW`: a hard disk, the only kind of drive with a Recycle Bin.
const DRIVE_FIXED: u32 = 3;

/// Sends `paths` (long paths, as core keeps them) to the Recycle Bin. Returns one outcome per
/// path, in order. Needs an STA: run it on the Shell pool. Fails only if the Shell's file
/// operation can't be set up at all; what was recycled by then stays recycled.
pub fn recycle(paths: &[PathBuf]) -> Result<Vec<Outcome>, JerryError> {
    let mut outcomes: Vec<Option<Outcome>> = paths
        .iter()
        .map(|path| (!on_fixed_drive(path)).then_some(Outcome::NoRecycleBin))
        .collect();
    let pending = |outcomes: &[Option<Outcome>]| -> Vec<usize> {
        (0..outcomes.len())
            .filter(|&index| outcomes[index].is_none())
            .collect()
    };
    let all = pending(&outcomes);
    if all.len() > 1 {
        perform(paths, &all, &mut outcomes)?;
    }
    // One by one: what a stopped operation never got to, or the only item.
    for index in pending(&outcomes) {
        if let Some(error) = perform(paths, &[index], &mut outcomes)? {
            let error = JerryError::new(code_of(error.code())).at(&paths[index]);
            outcomes[index].get_or_insert(Outcome::Failed(error));
        }
    }
    Ok(outcomes
        .into_iter()
        .zip(paths)
        .map(|(outcome, path)| {
            outcome.unwrap_or_else(|| {
                tracing::error!(path = %path.display(), "the Shell said nothing about an item");
                Outcome::Failed(JerryError::internal().at(path))
            })
        })
        .collect())
}

/// Whether `path` is on a hard disk. Elsewhere there is no Recycle Bin.
fn on_fixed_drive(path: &Path) -> bool {
    let Ok(root) = volume::root(path) else {
        // No volume: let the Shell say what is wrong.
        return true;
    };
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) == DRIVE_FIXED }
}

/// Runs one operation that recycles the items at `indices` and puts what happened to each into
/// `outcomes`; an item it never got to stays `None`. Returns the operation's error, if it ended
/// with one.
fn perform(
    paths: &[PathBuf],
    indices: &[usize],
    outcomes: &mut [Option<Outcome>],
) -> Result<Option<windows::core::Error>, JerryError> {
    // SAFETY: COM is initialized on this (Shell pool) thread; FileOperation is the Shell's.
    let operation: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }?;
    let flags = FOFX_RECYCLEONDELETE.0 | FOF_ALLOWUNDO.0 | FOF_NO_UI.0;
    // SAFETY: a valid operation and flags.
    unsafe { operation.SetOperationFlags(FILEOPERATION_FLAGS(flags)) }?;

    let mut items = Items::default();
    for &index in indices {
        let path = &paths[index];
        let queued = shell_item(path).and_then(|item| {
            // SAFETY: a valid item; no per-item sink.
            unsafe { operation.DeleteItem(&item, None) }?;
            Ok(item)
        });
        match queued {
            Ok(item) => items.add(&item, index),
            Err(error) => outcomes[index] = Some(Outcome::Failed(error.at(path))),
        }
    }
    if items.is_empty() {
        return Ok(None);
    }

    let single = indices.len() == 1;
    let sink = ComObject::new(Sink {
        items,
        state: Mutex::new(SinkState::default()),
    });
    let interface: IFileOperationProgressSink = sink.to_interface();
    // SAFETY: a valid sink; the cookie is given back below.
    let cookie = unsafe { operation.Advise(&interface) }?;
    // SAFETY: a valid operation. It calls the sink on this thread.
    let performed = unsafe { operation.PerformOperations() };
    // SAFETY: the cookie from Advise above.
    if let Err(error) = unsafe { operation.Unadvise(cookie) } {
        tracing::warn!(%error, "IFileOperation::Unadvise failed");
    }

    let state = std::mem::take(&mut *sink.state());
    for (index, outcome) in state.outcomes {
        outcomes[index] = Some(match outcome {
            Outcome::Failed(error) => Outcome::Failed(error.at(&paths[index])),
            outcome => outcome,
        });
    }
    // Only an operation of one item says which item the Shell can't recycle.
    if state.vetoed
        && single
        && let Some(&index) = indices.first()
    {
        tracing::info!(path = %paths[index].display(), "the Shell can't recycle an item; kept");
        outcomes[index] = Some(Outcome::NoRecycleBin);
    }
    Ok(performed.err())
}

/// The code for a delete the Shell reports as failed.
fn code_of(hr: HRESULT) -> ErrorCode {
    match hr {
        COPYENGINE_E_RECYCLE_BIN_NOT_FOUND => ErrorCode::NoRecycleBin,
        COPYENGINE_E_ACCESS_DENIED_SRC | COPYENGINE_E_REQUIRES_ELEVATION => ErrorCode::AccessDenied,
        COPYENGINE_E_SHARING_VIOLATION_SRC => ErrorCode::InUse { app: None },
        COPYENGINE_E_PATH_NOT_FOUND_SRC => ErrorCode::NotFound,
        COPYENGINE_E_USER_CANCELLED | COPYENGINE_E_CANCELLED => ErrorCode::Cancelled,
        hr => ErrorCode::from_hresult(hr),
    }
}

/// The Shell item for `path`, which the Shell takes without core's `\\?\` prefix.
fn shell_item(path: &Path) -> Result<IShellItem, JerryError> {
    let wide = shell_path(path);
    // SAFETY: `wide` is NUL-terminated and outlives the call; no bind context.
    Ok(unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }?)
}

/// `path` as the Shell takes it, NUL-terminated: without the `\\?\` prefix core uses (`\\?\UNC\`
/// back to `\\`). Exact UTF-16, so names that aren't valid Unicode survive.
fn shell_path(path: &Path) -> Vec<u16> {
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    let prefix = |text: &str| -> Vec<u16> { text.encode_utf16().collect() };
    let mut shell = if units.starts_with(&prefix(r"\\?\UNC\")) {
        let mut shell = prefix(r"\\");
        shell.extend_from_slice(&units[8..]);
        shell
    } else if units.starts_with(&prefix(r"\\?\")) {
        units[4..].to_vec()
    } else {
        units
    };
    shell.push(0);
    shell
}

/// The display name `kind` of `item`.
fn name_of(item: &IShellItem, kind: SIGDN) -> Option<Vec<u16>> {
    // SAFETY: a valid item.
    let name = unsafe { item.GetDisplayName(kind) }.ok()?;
    // SAFETY: on success the Shell returns a NUL-terminated string.
    let units = unsafe { name.as_wide() }.to_vec();
    // SAFETY: allocated with the COM allocator and freed once, here.
    unsafe { CoTaskMemFree(Some(name.0.cast_const().cast())) };
    Some(units)
}

/// The items queued, so the sink can tell which path each callback is about.
#[derive(Default)]
struct Items {
    /// By COM identity: the Shell usually hands back the very item it was given.
    identities: HashMap<usize, usize>,
    /// By parsing name, for an item the Shell made anew.
    names: HashMap<Vec<u16>, usize>,
}

impl Items {
    fn add(&mut self, item: &IShellItem, index: usize) {
        if let Some(identity) = identity(item) {
            self.identities.insert(identity, index);
        }
        if let Some(name) = name_of(item, SIGDN_DESKTOPABSOLUTEPARSING) {
            self.names.insert(name, index);
        }
    }

    fn is_empty(&self) -> bool {
        self.identities.is_empty() && self.names.is_empty()
    }

    /// Which queued path `item` is; `None` for anything else (an item inside a folder).
    fn index_of(&self, item: &IShellItem) -> Option<usize> {
        identity(item)
            .and_then(|identity| self.identities.get(&identity))
            .or_else(|| {
                name_of(item, SIGDN_DESKTOPABSOLUTEPARSING).and_then(|name| self.names.get(&name))
            })
            .copied()
    }
}

/// A COM object's identity: its `IUnknown` pointer.
fn identity(item: &IShellItem) -> Option<usize> {
    item.cast::<IUnknown>()
        .ok()
        .map(|unknown| unknown.as_raw() as usize)
}

/// Watches an operation item by item: vetoes a delete that wouldn't recycle and records what
/// happened to each queued item. Called on the thread that runs the operation.
#[implement(IFileOperationProgressSink)]
struct Sink {
    items: Items,
    state: Mutex<SinkState>,
}

#[derive(Default)]
struct SinkState {
    /// What happened, by the queued item's index. A failure has no path yet.
    outcomes: Vec<(usize, Outcome)>,
    /// The Shell was about to delete something for good, and the sink stopped it.
    vetoed: bool,
}

impl Sink {
    fn state(&self) -> MutexGuard<'_, SinkState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl IFileOperationProgressSink_Impl for Sink_Impl {
    fn PreDeleteItem(&self, flags: u32, _item: Ref<IShellItem>) -> windows::core::Result<()> {
        if flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 != 0 {
            return Ok(());
        }
        // The Shell would delete for good: stop the operation (see the module docs). Which item
        // is to blame isn't known yet: the Shell decides for the whole operation.
        self.state().vetoed = true;
        Err(COPYENGINE_E_RECYCLE_BIN_NOT_FOUND.into())
    }

    fn PostDeleteItem(
        &self,
        _flags: u32,
        item: Ref<IShellItem>,
        result: HRESULT,
        created: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        let Some(index) = item.as_ref().and_then(|item| self.items.index_of(item)) else {
            return Ok(());
        };
        let outcome = if result.is_ok() {
            let in_bin = created
                .as_ref()
                .and_then(|created| name_of(created, SIGDN_FILESYSPATH))
                .map(|name| PathBuf::from(OsString::from_wide(&name)));
            if in_bin.is_none() {
                tracing::warn!("the Shell deleted an item without saying where it went");
            }
            Outcome::Recycled { in_bin }
        } else if result == COPYENGINE_E_RECYCLE_BIN_NOT_FOUND {
            // The veto's own report, or the Shell's: the item is untouched, and the operation
            // of this item alone says whether it can be recycled.
            return Ok(());
        } else {
            Outcome::Failed(JerryError::new(code_of(result)))
        };
        self.state().outcomes.push((index, outcome));
        Ok(())
    }

    fn StartOperations(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn FinishOperations(&self, _result: HRESULT) -> windows::core::Result<()> {
        Ok(())
    }

    fn PreRenameItem(
        &self,
        _flags: u32,
        _item: Ref<IShellItem>,
        _new_name: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn PostRenameItem(
        &self,
        _flags: u32,
        _item: Ref<IShellItem>,
        _new_name: &PCWSTR,
        _result: HRESULT,
        _created: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn PreMoveItem(
        &self,
        _flags: u32,
        _item: Ref<IShellItem>,
        _destination: Ref<IShellItem>,
        _new_name: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn PostMoveItem(
        &self,
        _flags: u32,
        _item: Ref<IShellItem>,
        _destination: Ref<IShellItem>,
        _new_name: &PCWSTR,
        _result: HRESULT,
        _created: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn PreCopyItem(
        &self,
        _flags: u32,
        _item: Ref<IShellItem>,
        _destination: Ref<IShellItem>,
        _new_name: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn PostCopyItem(
        &self,
        _flags: u32,
        _item: Ref<IShellItem>,
        _destination: Ref<IShellItem>,
        _new_name: &PCWSTR,
        _result: HRESULT,
        _created: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn PreNewItem(
        &self,
        _flags: u32,
        _destination: Ref<IShellItem>,
        _new_name: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn PostNewItem(
        &self,
        _flags: u32,
        _destination: Ref<IShellItem>,
        _new_name: &PCWSTR,
        _template: &PCWSTR,
        _attributes: u32,
        _result: HRESULT,
        _created: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn UpdateProgress(&self, _total: u32, _done: u32) -> windows::core::Result<()> {
        Ok(())
    }

    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    use jerry_core::path::long_path;

    use super::*;
    use crate::shell_pool::ShellPool;

    const LONG: Duration = Duration::from_secs(60);

    fn temp() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("jerry-recycle-")
            .tempdir()
            .expect("temp folder")
    }

    /// The temp folder with its long names (`%TEMP%` can be `C:\Users\RUNNER~1\…`), as a long
    /// path: what the Recycle Bin records.
    fn real(dir: &tempfile::TempDir) -> PathBuf {
        std::fs::canonicalize(dir.path()).expect("canonical path")
    }

    /// `path` (a long path on a drive letter) through the administrative share `\\localhost\C$`:
    /// a network path. `None` where the share is off.
    fn through_share(path: &Path) -> Option<PathBuf> {
        let local = jerry_core::error::display_path(path);
        let (drive, rest) = local.split_once(":")?;
        let share = PathBuf::from(format!(r"\\localhost\{drive}${rest}"));
        share
            .exists()
            .then(|| long_path(&share).expect("long path"))
    }

    fn recycle_on_pool(paths: Vec<PathBuf>) -> Vec<Outcome> {
        ShellPool::new()
            .run("recycle", LONG, move || recycle(&paths))
            .expect("the operation runs")
    }

    /// Takes our own item back out of the Recycle Bin: `$R…` and the `$I…` that records it.
    fn empty_from_bin(in_bin: &Path) {
        let name = in_bin
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a $R name");
        let info = in_bin.with_file_name(name.replacen("$R", "$I", 1));
        let info_bytes = std::fs::read(&info).expect("the $I record");
        let _ = std::fs::remove_file(&info);
        let removed = if in_bin.is_dir() {
            std::fs::remove_dir_all(in_bin)
        } else {
            std::fs::remove_file(in_bin)
        };
        removed.expect("our item leaves the Recycle Bin");
        assert!(info_bytes.len() > 28, "the record names the original path");
    }

    /// The original path the `$I` record next to `in_bin` keeps.
    fn recorded_original(in_bin: &Path) -> String {
        let name = in_bin
            .file_name()
            .and_then(|name| name.to_str())
            .expect("name");
        let info = in_bin.with_file_name(name.replacen("$R", "$I", 1));
        let bytes = std::fs::read(info).expect("the $I record");
        // Version 2: a header of 24 bytes, the path length in UTF-16 units, then the path.
        let units: Vec<u16> = bytes[28..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .take_while(|&unit| unit != 0)
            .collect();
        String::from_utf16_lossy(&units)
    }

    #[test]
    fn recycles_georgian_files_and_folders_and_says_where_they_went() {
        let dir = temp();
        let file = real(&dir).join("ნახაზი.dwg");
        std::fs::write(&file, b"drawing").expect("write");
        let folder = real(&dir).join("პროექტი");
        std::fs::create_dir(&folder).expect("mkdir");
        std::fs::write(folder.join("a.txt"), b"a").expect("write");

        let outcomes = recycle_on_pool(vec![file.clone(), folder.clone()]);
        assert!(
            !file.exists() && !folder.exists(),
            "both are gone from here"
        );
        for (outcome, original) in outcomes.iter().zip([&file, &folder]) {
            let Outcome::Recycled {
                in_bin: Some(in_bin),
            } = outcome
            else {
                panic!("not recycled: {outcome:?}");
            };
            assert!(in_bin.exists(), "{in_bin:?} is in the Recycle Bin");
            assert!(
                in_bin.to_string_lossy().contains("$Recycle.Bin"),
                "{in_bin:?}"
            );
            assert_eq!(
                recorded_original(in_bin),
                jerry_core::error::display_path(original),
                "Restore puts it back here"
            );
            empty_from_bin(in_bin);
        }
    }

    #[test]
    fn a_share_has_no_recycle_bin_and_its_files_stay() {
        let dir = temp();
        let kept = real(&dir).join("ქსელი.txt");
        std::fs::write(&kept, b"keep me").expect("write");
        let local = real(&dir).join("ადგილობრივი.txt");
        std::fs::write(&local, b"recycle me").expect("write");
        // The same temp folder through the administrative share is a network path, where the
        // Shell would delete for good.
        let Some(shared) = through_share(&kept) else {
            // The share is turned off by policy here: nothing to check.
            return;
        };

        // A network drive is known to have no Recycle Bin: the Shell isn't asked. The next item
        // still goes to the Recycle Bin.
        let outcomes = recycle_on_pool(vec![shared.clone(), local.clone()]);
        assert_eq!(outcomes[0], Outcome::NoRecycleBin);
        assert_eq!(std::fs::read(&kept).expect("still there"), b"keep me");
        let Outcome::Recycled {
            in_bin: Some(in_bin),
        } = &outcomes[1]
        else {
            panic!("not recycled: {:?}", outcomes[1]);
        };
        assert!(!local.exists());
        empty_from_bin(in_bin);
    }

    #[test]
    fn the_veto_stops_an_operation_that_would_delete_for_good() {
        let dir = temp();
        let kept = real(&dir).join("ქსელი.txt");
        std::fs::write(&kept, b"keep me").expect("write");
        let local = real(&dir).join("ადგილობრივი.txt");
        std::fs::write(&local, b"keep me too").expect("write");
        let Some(shared) = through_share(&kept) else {
            return;
        };
        // Straight to the Shell, past the drive check: with the share in it, the Shell would
        // delete both for good, the local file too.
        let paths = vec![shared, local.clone()];
        let (together, alone) = ShellPool::new()
            .run("perform", LONG, move || {
                let mut together = vec![None, None];
                let stopped = perform(&paths, &[0, 1], &mut together)?;
                let mut alone = vec![None, None];
                perform(&paths, &[0], &mut alone)?;
                Ok(((together, stopped.map(|error| error.code())), alone))
            })
            .expect("runs");
        assert_eq!(
            together,
            (vec![None, None], Some(COPYENGINE_E_RECYCLE_BIN_NOT_FOUND)),
            "stopped before either was touched"
        );
        assert_eq!(alone, [Some(Outcome::NoRecycleBin), None]);
        assert_eq!(std::fs::read(&kept).expect("still there"), b"keep me");
        assert_eq!(std::fs::read(&local).expect("still there"), b"keep me too");
    }

    #[test]
    fn a_locked_file_fails_and_stays() {
        let dir = temp();
        let file = dir.path().join("დაკავებული.dwg");
        std::fs::write(&file, b"open").expect("write");
        let _held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&file)
            .expect("lock");
        let missing = dir.path().join("გამქრალი.txt");
        let free = real(&dir).join("თავისუფალი.txt");
        std::fs::write(&free, b"free").expect("write");

        let outcomes = recycle_on_pool(vec![
            long_path(&file).expect("long path"),
            long_path(&missing).expect("long path"),
            free.clone(),
        ]);
        let codes: Vec<_> = outcomes[..2]
            .iter()
            .map(|outcome| match outcome {
                Outcome::Failed(error) => Some(error.code.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            codes,
            [
                Some(ErrorCode::InUse { app: None }),
                Some(ErrorCode::NotFound)
            ]
        );
        assert!(file.exists());
        let Outcome::Failed(error) = &outcomes[0] else {
            unreachable!()
        };
        assert_eq!(
            error.path.as_deref(),
            Some(long_path(&file).expect("long path").as_path())
        );
        // A failure stops the Shell's operation; the items after it go one by one.
        let Outcome::Recycled {
            in_bin: Some(in_bin),
        } = &outcomes[2]
        else {
            panic!("not recycled: {:?}", outcomes[2]);
        };
        assert!(!free.exists());
        empty_from_bin(in_bin);
    }

    #[test]
    fn shell_paths_drop_the_long_path_prefix() {
        let text = |path: &str| {
            let mut units = shell_path(Path::new(path));
            assert_eq!(units.pop(), Some(0));
            String::from_utf16(&units).expect("UTF-16")
        };
        assert_eq!(text(r"\\?\C:\ნახაზი.dwg"), r"C:\ნახაზი.dwg");
        assert_eq!(text(r"\\?\UNC\server\share\a"), r"\\server\share\a");
        assert_eq!(text(r"D:\a"), r"D:\a");
    }
}
