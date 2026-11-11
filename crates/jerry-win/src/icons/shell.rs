//! Shell lookups: which system image list icon a file type, a file, a drive or a standard folder
//! shows, as Explorer would. They may load shell extensions and touch the disk: run them on the
//! Shell pool (STA, with a timeout), never on the main thread or the tokio runtime.
//!
//! Two rules shape them. A cloud-only file's content is never read (it would download), and an
//! icon never comes from the network (CLAUDE.md: no requests but the update check). A shortcut
//! whose icon lives on a share or a website, or in a cloud-only file, shows its type's icon.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::Once;

use jerry_core::{ErrorCode, JerryError};
use windows::Win32::Foundation::{ERROR_CONNECTION_UNAVAIL, MAX_PATH};
use windows::Win32::NetworkManagement::WNet::WNetGetConnectionW;
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES, GetDriveTypeW,
    GetFileAttributesW, GetLogicalDrives, INVALID_FILE_ATTRIBUTES,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ,
};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Registry::{RegCloseKey, RegQueryValueExW};
use windows::Win32::System::WindowsProgramming::GetPrivateProfileStringW;
use windows::Win32::UI::Controls::IImageList;
use windows::Win32::UI::Shell::{
    ASSOCF_NONE, ASSOCKEY_CLASS, AssocQueryKeyW, FOLDERID_Desktop, FOLDERID_Documents,
    FOLDERID_Downloads, FOLDERID_Pictures, FOLDERID_RecycleBinFolder, FOLDERID_SkyDrive, ILFree,
    IShellLinkW, KF_FLAG_DEFAULT, SHFILEINFOW, SHGFI_FLAGS, SHGFI_PIDL, SHGFI_SYSICONINDEX,
    SHGFI_USEFILEATTRIBUTES, SHGSI_SYSICONINDEX, SHGetFileInfoW, SHGetImageList,
    SHGetKnownFolderIDList, SHGetStockIconInfo, SHIL_EXTRALARGE, SHIL_JUMBO, SHIL_LARGE,
    SHIL_SMALL, SHSTOCKICONID, SHSTOCKICONINFO, SIID_DRIVENET, SIID_DRIVENETDISABLED,
    SIID_SERVERSHARE, SLGP_RAWPATH, ShellLink,
};
use windows::core::{GUID, Interface, PCWSTR, w};

use super::keys::{CLOUD_ONLY, IconType, ascii_lower, extension, has_own_icon, icon_type_of};
use crate::folders::known_folder;

/// `GetDriveTypeW`: a network drive.
const DRIVE_REMOTE: u32 = 4;

/// An icon in the system image list, and whether the shortcut arrow goes over it. The same
/// index is valid in every size of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Glyph {
    pub index: i32,
    pub link: bool,
}

/// A standard place for the sidebar (PRD «Sidebar»).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Place {
    Desktop,
    Documents,
    Downloads,
    Pictures,
    OneDrive,
    RecycleBin,
    /// A network location the user added (`\\server\share`). Its icon needs no network access.
    NetworkShare,
}

impl Place {
    pub const ALL: [Place; 7] = [
        Place::Desktop,
        Place::Documents,
        Place::Downloads,
        Place::Pictures,
        Place::OneDrive,
        Place::RecycleBin,
        Place::NetworkShare,
    ];

    /// The place's name in `icon://` URLs.
    pub fn name(self) -> &'static str {
        match self {
            Place::Desktop => "desktop",
            Place::Documents => "documents",
            Place::Downloads => "downloads",
            Place::Pictures => "pictures",
            Place::OneDrive => "onedrive",
            Place::RecycleBin => "recyclebin",
            Place::NetworkShare => "share",
        }
    }

    pub fn from_name(name: &str) -> Option<Place> {
        Place::ALL.into_iter().find(|place| place.name() == name)
    }

    /// Whether the place exists for the current user: OneDrive only once it is set up. The
    /// sidebar shows only the places that exist; the icon of one that doesn't is `NotFound`.
    pub fn exists(self) -> bool {
        self.folder()
            .is_none_or(|folder| known_folder(folder).is_ok())
    }

    /// The place's folder: None for Recycle Bin and a share, which have none, and for a place
    /// this user doesn't have.
    pub fn path(self) -> Option<std::path::PathBuf> {
        self.folder().and_then(|folder| known_folder(folder).ok())
    }

    /// The known folder of a place in the file system; Recycle Bin and a share have none.
    fn folder(self) -> Option<&'static GUID> {
        match self {
            Place::Desktop => Some(&FOLDERID_Desktop),
            Place::Documents => Some(&FOLDERID_Documents),
            Place::Downloads => Some(&FOLDERID_Downloads),
            Place::Pictures => Some(&FOLDERID_Pictures),
            Place::OneDrive => Some(&FOLDERID_SkyDrive),
            Place::RecycleBin | Place::NetworkShare => None,
        }
    }
}

/// Sets up the Shell's system image lists once, on one thread, before any lookup: when several
/// workers make a process's first lookups at once, they race inside the Shell, and drives,
/// Recycle Bin and stock icons fail. Every lookup below calls it first; the first call blocks
/// the others until the lists exist.
pub fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        for which in [SHIL_SMALL, SHIL_LARGE, SHIL_EXTRALARGE, SHIL_JUMBO] {
            // SAFETY: a valid image list id; COM is initialized on this thread (a Shell pool
            // worker).
            if let Err(error) = unsafe { SHGetImageList::<IImageList>(which as i32) } {
                tracing::warn!(%error, which, "a system image list couldn't be set up");
            }
        }
    });
}

/// The icon of a file type, by name and attributes only: no disk access.
pub fn type_glyph(icon: &IconType) -> Result<Glyph, JerryError> {
    let (mut name, attributes, extension) = match icon {
        IconType::Folder => (wide("folder"), FILE_ATTRIBUTE_DIRECTORY.0, None),
        IconType::File => (wide("file"), FILE_ATTRIBUTE_NORMAL.0, None),
        IconType::Extension(extension) => {
            let mut name = wide("file.");
            name.extend_from_slice(extension);
            (name, FILE_ATTRIBUTE_NORMAL.0, Some(extension))
        }
    };
    name.push(0);
    let flags = SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES;
    let index = system_index(PCWSTR(name.as_ptr()), attributes, flags)
        .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
    Ok(Glyph {
        index,
        link: extension.is_some_and(|extension| is_shortcut_type(extension)),
    })
}

/// The icon a file shows: its own for .exe, .lnk, .ico and .url (this reads the file), its
/// type's for every other file, and for those four when their icon would have to be read from a
/// cloud-only file or over the network.
pub fn file_glyph(path: &Path, attributes: u32) -> Result<Glyph, JerryError> {
    let name: Vec<u16> = path
        .file_name()
        .map(|name| name.encode_wide().collect())
        .unwrap_or_default();
    let by_type = || type_glyph(&icon_type_of(&name, attributes));
    if !has_own_icon(&name, attributes) {
        return by_type();
    }
    let Some(shell) = shell_path(path) else {
        return by_type();
    };
    let lower: Vec<u16> = extension(&name)
        .unwrap_or_default()
        .iter()
        .map(|&unit| ascii_lower(unit))
        .collect();
    let source = match String::from_utf16_lossy(&lower).as_str() {
        "lnk" => link_icon_source(&shell),
        "url" => url_icon_source(&shell),
        _ => Source::Itself,
    };
    let safe = match source {
        Source::Itself => true,
        Source::File(file) => is_local_and_present(&file),
        Source::Unreadable => false,
    };
    if !safe {
        return by_type();
    }
    match system_index(PCWSTR(shell.as_ptr()), 0, SHGFI_SYSICONINDEX) {
        Some(index) => Ok(Glyph {
            index,
            link: is_shortcut_type(&lower),
        }),
        None => by_type(),
    }
}

/// A drive's icon, as Explorer shows it under This PC. A network drive gets the stock network
/// drive icon (crossed out when disconnected), so the server is never contacted.
pub fn drive_glyph(letter: u8) -> Result<Glyph, JerryError> {
    let letter = letter.to_ascii_uppercase();
    // SAFETY: no arguments; returns the bitmask of drives.
    let drives = unsafe { GetLogicalDrives() };
    if !letter.is_ascii_uppercase() || drives & (1 << (letter - b'A')) == 0 {
        return Err(JerryError::new(ErrorCode::NotFound));
    }
    let device = [u16::from(letter), u16::from(b':'), 0];
    let root = [u16::from(letter), u16::from(b':'), u16::from(b'\\'), 0];
    // SAFETY: a NUL-terminated root path.
    if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } == DRIVE_REMOTE {
        let mut length = 0;
        // SAFETY: a NUL-terminated device name; no buffer, so only the status comes back. This
        // reads the local connection table, with no network access.
        let status = unsafe { WNetGetConnectionW(PCWSTR(device.as_ptr()), None, &mut length) };
        return stock_glyph(if status == ERROR_CONNECTION_UNAVAIL {
            SIID_DRIVENETDISABLED
        } else {
            SIID_DRIVENET
        });
    }
    system_index(PCWSTR(root.as_ptr()), 0, SHGFI_SYSICONINDEX)
        .map(|index| Glyph { index, link: false })
        .ok_or_else(|| JerryError::new(ErrorCode::DeviceGone))
}

/// A standard place's icon. Recycle Bin's shows whether it is empty or full. `NotFound` for a
/// place that doesn't exist ([`Place::exists`]).
pub fn place_glyph(place: Place) -> Result<Glyph, JerryError> {
    let folder = match place {
        Place::NetworkShare => return stock_glyph(SIID_SERVERSHARE),
        Place::RecycleBin => return pidl_glyph(&FOLDERID_RecycleBinFolder),
        _ => place
            .folder()
            .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?,
    };
    // The folder's icon comes from its desktop.ini. Inside OneDrive that can be cloud-only; then
    // the plain folder icon is shown rather than download it.
    let path = known_folder(folder)?;
    let ini = path.join("desktop.ini");
    if is_cloud_only(&path) || is_cloud_only(&ini) {
        return type_glyph(&IconType::Folder);
    }
    pidl_glyph(folder)
}

fn pidl_glyph(folder: &GUID) -> Result<Glyph, JerryError> {
    // SAFETY: a valid known folder id; no token means the current user.
    let pidl = unsafe { SHGetKnownFolderIDList(folder, KF_FLAG_DEFAULT.0 as u32, None) }?;
    let index = system_index(
        PCWSTR(pidl.cast_const().cast()),
        0,
        SHGFI_PIDL | SHGFI_SYSICONINDEX,
    );
    // SAFETY: the ID list came from the Shell above and is freed once, here.
    unsafe { ILFree(Some(pidl.cast_const())) };
    index
        .map(|index| Glyph { index, link: false })
        .ok_or_else(|| JerryError::new(ErrorCode::NotFound))
}

fn stock_glyph(icon: SHSTOCKICONID) -> Result<Glyph, JerryError> {
    let mut info = SHSTOCKICONINFO {
        cbSize: size_of::<SHSTOCKICONINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a writable SHSTOCKICONINFO with its size set.
    unsafe { SHGetStockIconInfo(icon, SHGSI_SYSICONINDEX, &mut info) }?;
    Ok(Glyph {
        index: info.iSysImageIndex,
        link: false,
    })
}

/// `SHGetFileInfoW` for a system image list index. `path` is a NUL-terminated path, or an
/// `ITEMIDLIST` with `SHGFI_PIDL`.
fn system_index(path: PCWSTR, attributes: u32, flags: SHGFI_FLAGS) -> Option<i32> {
    let mut info = SHFILEINFOW::default();
    // SAFETY: `path` is valid for `flags`, and `info` is a writable SHFILEINFOW of the size given.
    let found = unsafe {
        SHGetFileInfoW(
            path,
            FILE_FLAGS_AND_ATTRIBUTES(attributes),
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    (found != 0).then_some(info.iIcon)
}

/// Whether files of this type get the shortcut arrow: their class has `IsShortcut`, as .lnk and
/// .url do. That is how Explorer decides.
fn is_shortcut_type(extension: &[u16]) -> bool {
    let mut dotted = vec![u16::from(b'.')];
    dotted.extend_from_slice(extension);
    dotted.push(0);
    // SAFETY: a NUL-terminated extension and no extra string.
    let Ok(class) =
        (unsafe { AssocQueryKeyW(ASSOCF_NONE, ASSOCKEY_CLASS, PCWSTR(dotted.as_ptr()), None) })
    else {
        return false;
    };
    // SAFETY: an open key; only whether the value exists is asked.
    let found =
        unsafe { RegQueryValueExW(class, w!("IsShortcut"), None, None, None, None) }.is_ok();
    // SAFETY: the key from AssocQueryKeyW, closed once, here.
    let _ = unsafe { RegCloseKey(class) };
    found
}

/// Where a shortcut's icon comes from.
enum Source {
    /// The file itself, or a default icon: nothing outside it is read.
    Itself,
    /// Another file (an icon file, a program, the shortcut's target).
    File(Vec<u16>),
    /// The shortcut couldn't be read.
    Unreadable,
}

/// A .lnk's icon: its icon location, or else its target.
fn link_icon_source(path: &[u16]) -> Source {
    let read = || -> windows::core::Result<Source> {
        // SAFETY: COM is initialized on this thread (a Shell pool worker).
        let link: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }?;
        let file: IPersistFile = link.cast()?;
        // SAFETY: a NUL-terminated path; the shortcut is only read.
        unsafe { file.Load(PCWSTR(path.as_ptr()), STGM_READ) }?;
        let mut buffer = vec![0_u16; 32_768];
        let mut index = 0;
        // SAFETY: a writable buffer and index.
        unsafe { link.GetIconLocation(&mut buffer, &mut index) }?;
        let icon = until_nul(&buffer);
        if !icon.is_empty() {
            return Ok(Source::File(expand(icon)));
        }
        // SAFETY: a writable buffer; no find data wanted. Reads the stored path, never resolves
        // the link (which could search the network).
        unsafe { link.GetPath(&mut buffer, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32) }?;
        let target = until_nul(&buffer);
        Ok(if target.is_empty() {
            Source::Itself // a Shell item with no file, such as Control Panel
        } else {
            Source::File(expand(target))
        })
    };
    read().unwrap_or(Source::Unreadable)
}

/// A .url's icon: its `IconFile`, if it names one; the browser's icon otherwise.
fn url_icon_source(path: &[u16]) -> Source {
    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: NUL-terminated strings and a writable buffer.
    let length = unsafe {
        GetPrivateProfileStringW(
            w!("InternetShortcut"),
            w!("IconFile"),
            w!(""),
            Some(&mut buffer),
            PCWSTR(path.as_ptr()),
        )
    } as usize;
    let icon = &buffer[..length.min(buffer.len())];
    if icon.is_empty() {
        Source::Itself
    } else {
        Source::File(expand(icon))
    }
}

/// True if `file` is on this device and its content is here: not a URL, not on a share or a
/// network drive, not cloud-only. A file that doesn't exist counts as local: the Shell then uses
/// a default icon.
fn is_local_and_present(file: &[u16]) -> bool {
    let text = String::from_utf16_lossy(file);
    if text.contains("://") {
        return false;
    }
    let local = text.strip_prefix(r"\\?\").unwrap_or(&text);
    if local.starts_with(r"\\")
        || local
            .get(..4)
            .is_some_and(|unc| unc.eq_ignore_ascii_case("UNC\\"))
    {
        return false;
    }
    let bytes = local.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let root = [u16::from(bytes[0]), u16::from(b':'), u16::from(b'\\'), 0];
        // SAFETY: a NUL-terminated root path. For a mapped drive this reads the local mapping.
        if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } == DRIVE_REMOTE {
            return false;
        }
    }
    let mut path = file.to_vec();
    path.push(0);
    !attributes_are_cloud_only(&path)
}

pub(crate) fn is_cloud_only(path: &Path) -> bool {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    attributes_are_cloud_only(&wide)
}

/// Whether the file at `path` (NUL-terminated) is cloud-only. Reads metadata only, which never
/// downloads.
fn attributes_are_cloud_only(path: &[u16]) -> bool {
    // SAFETY: a NUL-terminated path.
    let attributes = unsafe { GetFileAttributesW(PCWSTR(path.as_ptr())) };
    attributes != INVALID_FILE_ATTRIBUTES && attributes & CLOUD_ONLY != 0
}

/// `path` as the Shell takes it, NUL-terminated: without the `\\?\` prefix core uses (`\\?\UNC\`
/// back to `\\`). `None` if it is too long for the Shell (`MAX_PATH`): Shell APIs often fail on
/// longer paths, so such files show their type's icon (and have no thumbnail).
pub(crate) fn shell_path(path: &Path) -> Option<Vec<u16>> {
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    let starts = |prefix: &str| units.starts_with(&wide(prefix));
    let mut shell = if starts(r"\\?\UNC\") {
        let mut shell = wide(r"\\");
        shell.extend_from_slice(&units[8..]);
        shell
    } else if starts(r"\\?\") {
        units[4..].to_vec()
    } else {
        units
    };
    if shell.len() >= MAX_PATH as usize {
        return None;
    }
    shell.push(0);
    Some(shell)
}

/// Expands `%SystemRoot%`-style variables.
fn expand(text: &[u16]) -> Vec<u16> {
    let mut source = text.to_vec();
    source.push(0);
    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: a NUL-terminated source and a writable buffer.
    let length = unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), Some(&mut buffer)) };
    // The length counts the NUL; 0 is a failure, more than the buffer didn't fit.
    if length == 0 || length as usize > buffer.len() {
        return text.to_vec();
    }
    until_nul(&buffer).to_vec()
}

fn until_nul(buffer: &[u16]) -> &[u16] {
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    &buffer[..end]
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}
