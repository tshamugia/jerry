//! Files as the Shell hands them to other programs: the data object Explorer puts on the clipboard
//! and drags (Tech PRD «Clipboard», spikes #13 and #15). Copy, Cut ([`clipboard`]) and dragging out
//! of Jerry ([`drag`]) give every target the same object, so Explorer, the desktop, Outlook,
//! Telegram, browsers and AutoCAD treat Jerry's files exactly as Explorer's.
//!
//! The data object is a COM object of the thread that makes it: Jerry makes it on the main STA
//! thread, where the clipboard and the drag loop run.
//!
//! [`clipboard`]: crate::clipboard
//! [`drag`]: crate::drag

use std::ffi::OsString;
use std::mem::ManuallyDrop;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use jerry_core::path::plain_path;
use jerry_core::{ErrorCode, JerryError};
use windows::Win32::Foundation::{GlobalFree, HGLOBAL};
use windows::Win32::System::Com::{
    DVASPECT_CONTENT, FORMATETC, IBindCtx, IDataObject, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{CF_HDROP, ReleaseStgMedium};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, DROPFILES, DragQueryFileW, HDROP, ILCreateFromPathW, ILFree,
    SHCreateDataObject, SHCreateShellItemArrayFromIDLists,
};
use windows::core::PCWSTR;

/// The data object for `paths` (core's, `\\?\` long paths welcome): the Shell's own, or `CF_HDROP`
/// alone for existing files the Shell can't parse. Core's `\\?\` paths go to the Shell and into
/// `CF_HDROP` without the prefix, which no other program understands; the Shell parses plain paths
/// longer than `MAX_PATH` fine. `NotFound` with its path for a file that is gone.
pub(crate) fn files(paths: &[PathBuf]) -> Result<IDataObject, JerryError> {
    let plain: Vec<PathBuf> = paths.iter().map(|path| plain_path(path)).collect();
    match shell_data_object(&plain) {
        Err(unparsed) if unparsed.code == ErrorCode::NotFound => {
            let exists = unparsed.path.as_ref().is_some_and(|path| path.exists());
            if !exists {
                return Err(unparsed);
            }
            tracing::warn!(path = ?unparsed.path, "the Shell can't parse it; CF_HDROP only");
            hdrop_data_object(&plain)
        }
        result => result,
    }
}

/// The Shell data object for `paths`, the same one Explorer copies and drags: the items' parent
/// folder makes it (`IShellItemArray::BindToHandler(BHID_DataObject)` ends in its
/// `GetUIObjectOf`). `SHCreateDataObject` with absolute PIDLs offers nothing (spike #13).
/// `NotFound` with the path the Shell can't parse.
fn shell_data_object(paths: &[PathBuf]) -> Result<IDataObject, JerryError> {
    if paths.is_empty() {
        return Err(JerryError::new(ErrorCode::NotFound));
    }
    let pidls = paths
        .iter()
        .map(|path| {
            Pidl::from_path(path).ok_or_else(|| JerryError::new(ErrorCode::NotFound).at(path))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let raw: Vec<*const ITEMIDLIST> = pidls.iter().map(Pidl::as_ptr).collect();
    // SAFETY: every pointer is a valid absolute ID list that `pidls` keeps alive for the call;
    // the array copies what it needs.
    let items = unsafe { SHCreateShellItemArrayFromIDLists(&raw) }?;
    // SAFETY: a plain COM call on a live object; no bind context is needed.
    let data =
        unsafe { items.BindToHandler::<_, IDataObject>(None::<&IBindCtx>, &BHID_DataObject) }?;
    Ok(data)
}

/// A data object holding only `CF_HDROP` with `paths`: the Shell's generic object, which keeps
/// any format set on it.
fn hdrop_data_object(paths: &[PathBuf]) -> Result<IDataObject, JerryError> {
    // SAFETY: no folder, no items, no inner object: an empty data object.
    let data: IDataObject = unsafe { SHCreateDataObject(None, None, None::<&IDataObject>) }?;
    set_hglobal(&data, CF_HDROP.0, &dropfiles(paths))?;
    Ok(data)
}

/// The file paths in `data`'s `CF_HDROP`.
pub(crate) fn hdrop_paths(data: &IDataObject) -> Result<Vec<PathBuf>, JerryError> {
    // SAFETY: a plain COM call; we release the medium below.
    let mut medium = unsafe { data.GetData(&hglobal_format(CF_HDROP.0)) }?;
    // SAFETY: we asked for TYMED_HGLOBAL, so `hGlobal` is the active union field, and an
    // HGLOBAL holding a DROPFILES is exactly what HDROP is.
    let hdrop = HDROP(unsafe { medium.u.hGlobal }.0);
    // SAFETY: `hdrop` stays valid until ReleaseStgMedium; index u32::MAX asks for the count.
    let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        // SAFETY: as above; a None buffer asks for the length without the NUL.
        let len = unsafe { DragQueryFileW(hdrop, index, None) } as usize;
        let mut buffer = vec![0u16; len + 1];
        // SAFETY: the buffer has room for the name and its NUL.
        let copied = unsafe { DragQueryFileW(hdrop, index, Some(&mut buffer)) } as usize;
        paths.push(PathBuf::from(OsString::from_wide(&buffer[..copied])));
    }
    // SAFETY: we own the medium GetData returned and release it once.
    unsafe { ReleaseStgMedium(&mut medium) };
    Ok(paths)
}

/// Reads a registered DWORD format such as "Preferred DropEffect", if `data` has it.
pub(crate) fn dword(data: &IDataObject, name: PCWSTR) -> Option<u32> {
    let format = registered_format(name)?;
    // SAFETY: a plain COM call; we release the medium below.
    let mut medium = unsafe { data.GetData(&hglobal_format(format)) }.ok()?;
    // SAFETY: TYMED_HGLOBAL was requested, so `hGlobal` is the active field.
    let global = unsafe { medium.u.hGlobal };
    // SAFETY: locking a valid HGLOBAL; null means it has no memory.
    let ptr = unsafe { GlobalLock(global) }.cast::<u32>();
    let value = (!ptr.is_null()).then(|| {
        // SAFETY: the format is a DWORD, so the block holds at least four bytes.
        unsafe { ptr.read_unaligned() }
    });
    // SAFETY: balanced with the lock above. It "fails" with NO_ERROR once the lock count
    // reaches zero, which is the normal case, so the result is ignored.
    let _ = unsafe { GlobalUnlock(global) };
    // SAFETY: we own the medium GetData returned and release it once.
    unsafe { ReleaseStgMedium(&mut medium) };
    value
}

/// Writes a registered DWORD format such as "Preferred DropEffect" into `data`.
pub(crate) fn set_dword(data: &IDataObject, name: PCWSTR, value: u32) -> Result<(), JerryError> {
    let Some(format) = registered_format(name) else {
        tracing::error!("RegisterClipboardFormatW failed");
        return Err(JerryError::internal());
    };
    set_hglobal(data, format, &value.to_le_bytes())
}

/// `bytes` in movable global memory, as the clipboard and data objects take it.
pub(crate) fn global_copy(bytes: &[u8]) -> Result<HGLOBAL, JerryError> {
    // SAFETY: a plain allocation; freed below on failure, else owned by whoever takes it.
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }?;
    // SAFETY: `memory` was just allocated and is locked once, here.
    let target = unsafe { GlobalLock(memory) }.cast::<u8>();
    if target.is_null() {
        free(memory);
        return Err(JerryError::internal());
    }
    // SAFETY: `target` points to `bytes.len()` writable bytes, which `bytes` fills exactly; the
    // two don't overlap.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len()) };
    // SAFETY: unlocks the lock taken above. It "fails" with no error once the count is 0.
    let _ = unsafe { GlobalUnlock(memory) };
    Ok(memory)
}

/// Frees global memory nobody took.
pub(crate) fn free(memory: HGLOBAL) {
    // SAFETY: `memory` is ours: nobody took it.
    if let Err(error) = unsafe { GlobalFree(Some(memory)) } {
        tracing::warn!(%error, "couldn't free clipboard memory");
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

    fn as_ptr(&self) -> *const ITEMIDLIST {
        self.0
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        // SAFETY: ILCreateFromPathW allocated the list, and it's freed only here, once.
        unsafe { ILFree(Some(self.0)) };
    }
}

/// A DROPFILES block with wide `paths`, as `CF_HDROP` holds it.
fn dropfiles(paths: &[PathBuf]) -> Vec<u8> {
    let header = DROPFILES {
        pFiles: size_of::<DROPFILES>() as u32,
        fWide: true.into(),
        ..Default::default()
    };
    // SAFETY: DROPFILES is plain old data; we copy its bytes.
    let mut bytes = unsafe {
        std::slice::from_raw_parts((&raw const header).cast::<u8>(), size_of::<DROPFILES>())
    }
    .to_vec();
    for path in paths {
        for unit in path.as_os_str().encode_wide().chain(Some(0)) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

/// Copies `bytes` into a new HGLOBAL and hands it to `data` as `format`.
fn set_hglobal(data: &IDataObject, format: u16, bytes: &[u8]) -> Result<(), JerryError> {
    let global = global_copy(bytes)?;
    let medium = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 { hGlobal: global },
        pUnkForRelease: ManuallyDrop::new(None),
    };
    // SAFETY: valid FORMATETC and STGMEDIUM; with fRelease the object owns the block on success.
    let result = unsafe { data.SetData(&hglobal_format(format), &medium, true) };
    if result.is_err() {
        free(global);
    }
    Ok(result?)
}

fn registered_format(name: PCWSTR) -> Option<u16> {
    // SAFETY: `name` is a static NUL-terminated string.
    u16::try_from(unsafe { RegisterClipboardFormatW(name) })
        .ok()
        .filter(|&f| f != 0)
}

pub(crate) fn hglobal_format(format: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        ptd: core::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;

    use windows::Win32::Foundation::S_OK;
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, DATADIR_GET,
    };
    use windows::Win32::System::DataExchange::GetClipboardFormatNameW;

    use super::*;

    // These tests build data objects but never put them on the clipboard or drag them: the
    // clipboard and the mouse are the user's.

    pub(crate) fn sta() {
        // SAFETY: test threads are fresh; making them an STA is what the Shell expects.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    }

    /// A temp folder, and its path by its long names. Where `TEMP` is an 8.3 short path (GitHub's
    /// runner: `C:\Users\RUNNER~1\…`), the Shell hands files back by their long names, so paths
    /// that went through it are compared in that form.
    pub(crate) fn temp() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let long = jerry_core::path::plain_path(&fs::canonicalize(dir.path()).unwrap());
        (dir, long)
    }

    pub(crate) fn files(dir: &Path, names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|name| {
                let path = dir.join(name);
                fs::write(&path, b"jerry").unwrap();
                path
            })
            .collect()
    }

    /// A file whose full path is longer than MAX_PATH, written through `\\?\`. Returns the plain
    /// path (what `CF_HDROP` carries).
    fn long_file(dir: &Path) -> PathBuf {
        let mut path = dir.to_path_buf();
        while path.as_os_str().len() < 280 {
            path.push("ძალიან გრძელი საქაღალდის სახელი");
        }
        let verbatim = PathBuf::from(format!(r"\\?\{}", path.display()));
        fs::create_dir_all(&verbatim).unwrap();
        fs::write(verbatim.join("ნახაზი.dwg"), b"AC1032").unwrap();
        path.join("ნახაზი.dwg")
    }

    /// The names of the formats `data` offers, in its order.
    pub(crate) fn format_names(data: &IDataObject) -> Vec<String> {
        // SAFETY: a plain COM call on a live object.
        let formats = unsafe { data.EnumFormatEtc(DATADIR_GET.0 as u32) }.unwrap();
        let mut names = Vec::new();
        loop {
            let mut format = [FORMATETC::default()];
            let mut fetched = 0;
            // SAFETY: a one-element buffer and a valid count pointer.
            let hr = unsafe { formats.Next(&mut format, Some(&mut fetched)) };
            if hr != S_OK || fetched == 0 {
                break;
            }
            let mut name = [0u16; 128];
            // SAFETY: a writable buffer; predefined formats have no name and return 0.
            let len = unsafe { GetClipboardFormatNameW(u32::from(format[0].cfFormat), &mut name) };
            names.push(match (len, format[0].cfFormat) {
                (0, 15) => "CF_HDROP".to_string(),
                (0, other) => format!("CF_{other}"),
                (len, _) => String::from_utf16_lossy(&name[..len as usize]),
            });
            if !format[0].ptd.is_null() {
                // SAFETY: the enumerator allocated `ptd` with the COM allocator and handed it over.
                unsafe { CoTaskMemFree(Some(format[0].ptd.cast_const().cast())) };
            }
        }
        names
    }

    pub(crate) fn has(formats: &[String], name: &str) -> bool {
        formats.iter().any(|format| format == name)
    }

    #[test]
    fn files_carry_explorers_formats_without_the_long_path_prefix() {
        sta();
        let (_dir, dir) = temp();
        let paths = files(&dir, &["ნახაზი — სართული 1.dwg", "ხარჯთაღრიცხვა.pdf"]);
        let long: Vec<PathBuf> = paths
            .iter()
            .map(|path| jerry_core::path::long_path(path).unwrap())
            .collect();

        let data = super::files(&long).unwrap();
        let formats = format_names(&data);
        for expected in ["Shell IDList Array", "CF_HDROP", "FileNameW"] {
            assert!(has(&formats, expected), "{expected}: {formats:?}");
        }
        // Without the `\\?\` prefix: other programs don't understand it.
        assert_eq!(hdrop_paths(&data).unwrap(), paths);
    }

    /// Search results hold files of many folders: the Shell makes one object for all of them.
    #[test]
    fn files_of_different_folders_go_together() {
        sta();
        let (_dir, dir) = temp();
        fs::create_dir(dir.join("ა")).unwrap();
        fs::create_dir(dir.join("ბ")).unwrap();
        let mut paths = files(&dir.join("ა"), &["one.pdf"]);
        paths.extend(files(&dir.join("ბ"), &["ორი.dwg"]));
        paths.push(dir.join("ბ"));

        let data = super::files(&paths).unwrap();
        let formats = format_names(&data);
        assert!(has(&formats, "Shell IDList Array"), "{formats:?}");
        assert_eq!(hdrop_paths(&data).unwrap(), paths);
    }

    /// Spike #15: the Shell parses plain paths longer than MAX_PATH (even with
    /// LongPathsEnabled = 0), but not `\\?\` ones, so the prefix comes off first.
    #[test]
    fn long_paths_get_the_shell_object() {
        sta();
        let (_dir, dir) = temp();
        let long = long_file(&dir);
        let short = files(&dir, &["მოკლე.txt"]).remove(0);
        let verbatim = PathBuf::from(format!(r"\\?\{}", long.display()));

        assert!(shell_data_object(std::slice::from_ref(&verbatim)).is_err());

        let data = super::files(&[short.clone(), verbatim]).unwrap();
        let formats = format_names(&data);
        assert!(has(&formats, "Shell IDList Array"), "{formats:?}");
        assert_eq!(hdrop_paths(&data).unwrap(), [short, long]);
    }

    /// The fallback for files the Shell can't parse: `CF_HDROP` alone, which keeps what is set.
    #[test]
    fn hdrop_only_object_keeps_paths_and_dwords() {
        sta();
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![long_file(dir.path()), dir.path().join("არ არსებობს.txt")];
        let name = windows::Win32::UI::Shell::CFSTR_PREFERREDDROPEFFECT;

        let data = hdrop_data_object(&paths).unwrap();
        set_dword(&data, name, 2).unwrap();

        let formats = format_names(&data);
        assert!(has(&formats, "CF_HDROP"), "{formats:?}");
        assert!(has(&formats, "Preferred DropEffect"), "{formats:?}");
        assert_eq!(hdrop_paths(&data).unwrap(), paths);
        assert_eq!(dword(&data, name), Some(2));
    }

    #[test]
    fn a_missing_file_is_not_found_with_its_path() {
        sta();
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone.txt");
        let error = super::files(std::slice::from_ref(&gone)).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(error.path.as_deref(), Some(gone.as_path()));
        assert_eq!(super::files(&[]).unwrap_err().code, ErrorCode::NotFound);
    }
}
