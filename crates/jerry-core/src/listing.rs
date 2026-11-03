//! Folder listing with `FindFirstFileExW` (`FindExInfoBasic`, `FIND_FIRST_EX_LARGE_FETCH`): name,
//! size, dates and attributes come with the directory read itself, no file is opened.
//!
//! Entries are columns, and an entry's id is its index in the listing: ids are stable for the
//! listing's lifetime. Names stay UTF-16 exactly as Windows returns them, so a name that isn't
//! valid Unicode still gives back the same `OsString` and path for file operations.
//!
//! [`list`] streams batches (the first at [`FIRST_BATCH`] entries, then one every
//! [`BATCH_INTERVAL`], the rest at the end) and can be cancelled; [`Listing::read`] collects a
//! whole folder. Both block on IO: run them on a background thread.

use std::ffi::OsString;
use std::ops::Range;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, HANDLE, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic,
    FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, GetDriveTypeW, WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

use crate::error::{ErrorCode, JerryError};
use crate::path::long_path;

/// Entries in the first batch: more than a screen of rows, few enough to send at once.
pub const FIRST_BATCH: usize = 256;

/// Time between later batches (Tech PRD «მონაცემთა ნაკადი»).
pub const BATCH_INTERVAL: Duration = Duration::from_millis(50);

/// Most entries in one batch, so a batch's counts and name offsets always fit in a `u32`.
pub const MAX_BATCH: usize = 65_536;

/// The clock is read once per this many entries: often enough for [`BATCH_INTERVAL`], cheap.
const CLOCK_EVERY: usize = 32;

/// `GetDriveTypeW`: the root path is invalid, no volume is mounted there.
const DRIVE_NO_ROOT_DIR: u32 = 1;

/// Entries as columns. Entry `i` is index `i` in every column.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Entries {
    /// Every name as UTF-16, back to back. Name `i` ends at `name_ends[i]`.
    name_units: Vec<u16>,
    name_ends: Vec<usize>,
    sizes: Vec<u64>,
    created: Vec<u64>,
    modified: Vec<u64>,
    attributes: Vec<u32>,
}

impl Entries {
    pub fn len(&self) -> usize {
        self.sizes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sizes.is_empty()
    }

    /// Name `id` as UTF-16 code units, exactly as Windows returned it.
    pub fn name_utf16(&self, id: usize) -> &[u16] {
        &self.name_units[self.name_range(id)]
    }

    /// Name `id` as the exact `OsString`, invalid UTF-16 included.
    pub fn name(&self, id: usize) -> OsString {
        OsString::from_wide(self.name_utf16(id))
    }

    /// All names back to back; name `i` ends at `name_ends()[i]`.
    pub fn name_units(&self) -> &[u16] {
        &self.name_units
    }

    /// Where each name ends in [`Self::name_units`], in UTF-16 units.
    pub fn name_ends(&self) -> &[usize] {
        &self.name_ends
    }

    /// Sizes in bytes (0 for folders).
    pub fn sizes(&self) -> &[u64] {
        &self.sizes
    }

    /// Creation times as FILETIMEs: 100 ns ticks since 1601-01-01 UTC.
    pub fn created(&self) -> &[u64] {
        &self.created
    }

    /// Last write times as FILETIMEs: 100 ns ticks since 1601-01-01 UTC.
    pub fn modified(&self) -> &[u64] {
        &self.modified
    }

    /// `FILE_ATTRIBUTE_*` flags.
    pub fn attributes(&self) -> &[u32] {
        &self.attributes
    }

    pub fn is_dir(&self, id: usize) -> bool {
        self.attributes[id] & FILE_ATTRIBUTE_DIRECTORY.0 != 0
    }

    /// Appends one entry; it gets the next id.
    pub fn push(&mut self, name: &[u16], size: u64, created: u64, modified: u64, attributes: u32) {
        self.name_units.extend_from_slice(name);
        self.name_ends.push(self.name_units.len());
        self.sizes.push(size);
        self.created.push(created);
        self.modified.push(modified);
        self.attributes.push(attributes);
    }

    /// Appends all of `other`: its entry `i` gets id `self.len() + i`.
    pub fn append(&mut self, other: &Entries) {
        let offset = self.name_units.len();
        self.name_units.extend_from_slice(&other.name_units);
        self.name_ends
            .extend(other.name_ends.iter().map(|end| end + offset));
        self.sizes.extend_from_slice(&other.sizes);
        self.created.extend_from_slice(&other.created);
        self.modified.extend_from_slice(&other.modified);
        self.attributes.extend_from_slice(&other.attributes);
    }

    /// Removes every entry, keeping the memory for reuse.
    pub fn clear(&mut self) {
        self.name_units.clear();
        self.name_ends.clear();
        self.sizes.clear();
        self.created.clear();
        self.modified.clear();
        self.attributes.clear();
    }

    fn name_range(&self, id: usize) -> Range<usize> {
        let start = if id == 0 { 0 } else { self.name_ends[id - 1] };
        start..self.name_ends[id]
    }

    fn push_find_data(&mut self, data: &WIN32_FIND_DATAW) {
        let len = data
            .cFileName
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(data.cFileName.len());
        let filetime = |time: windows::Win32::Foundation::FILETIME| {
            (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
        };
        self.push(
            &data.cFileName[..len],
            (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow),
            filetime(data.ftCreationTime),
            filetime(data.ftLastWriteTime),
            data.dwFileAttributes,
        );
    }
}

/// One folder's entries and the folder's exact long path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    folder: PathBuf,
    entries: Entries,
}

impl Listing {
    /// An empty listing of `folder`; batches go in with [`Listing::append`].
    pub fn new(folder: PathBuf) -> Self {
        Self {
            folder,
            entries: Entries::default(),
        }
    }

    /// Reads all of `folder` (see [`list`]).
    pub fn read(folder: &Path, cancel: &AtomicBool) -> Result<Self, JerryError> {
        let mut listing = Self::new(long_path(folder)?);
        list(folder, cancel, |batch| listing.append(batch))?;
        Ok(listing)
    }

    /// The folder, as the `\\?\` long path core works with.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn entries(&self) -> &Entries {
        &self.entries
    }

    /// Appends a batch from [`list`]; its entries get the next ids.
    pub fn append(&mut self, batch: &Entries) {
        self.entries.append(batch);
    }

    /// The exact path of entry `id`, or `None` if the listing has no such entry.
    pub fn path(&self, id: usize) -> Option<PathBuf> {
        (id < self.entries.len()).then(|| self.folder.join(self.entries.name(id)))
    }
}

/// Converts a FILETIME (100 ns ticks since 1601) to milliseconds since the Unix epoch.
pub fn filetime_to_unix_ms(filetime: u64) -> f64 {
    const UNIX_EPOCH_MS: f64 = 11_644_473_600_000.0;
    filetime as f64 / 10_000.0 - UNIX_EPOCH_MS
}

/// Reads `folder` ("." and ".." left out) and hands its entries to `on_batch` in batches: the
/// first once [`FIRST_BATCH`] entries are in (or [`BATCH_INTERVAL`] has passed, on a slow disk),
/// then one at most every [`BATCH_INTERVAL`], and the rest at the end. Batch entries continue the
/// ids: the first entry of a batch has the id after the last one of the previous batch. An empty
/// folder gives no batch.
///
/// Setting `cancel` stops the listing with [`ErrorCode::Cancelled`]; no batch follows. A Windows
/// error ends it with that error, on `folder`: [`ErrorCode::NotFound`] for a missing folder,
/// [`ErrorCode::AccessDenied`], [`ErrorCode::DeviceGone`] for a drive or share that is gone.
///
/// Blocking IO: call it on a background thread, never on the UI thread or the async runtime.
pub fn list(
    folder: &Path,
    cancel: &AtomicBool,
    mut on_batch: impl FnMut(&Entries),
) -> Result<(), JerryError> {
    let long = long_path(folder)?;
    let cancelled = || JerryError::new(ErrorCode::Cancelled).at(&long);
    let pattern = search_pattern(&long);
    let mut data = WIN32_FIND_DATAW::default();
    // SAFETY: `pattern` is NUL-terminated and outlives the call; `data` is the WIN32_FIND_DATAW
    // that FindExInfoBasic fills.
    let first = unsafe {
        FindFirstFileExW(
            PCWSTR(pattern.as_ptr()),
            FindExInfoBasic,
            (&raw mut data).cast(),
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let handle = match first {
        Ok(handle) => FindHandle(handle),
        // Nothing matched `*`: an empty drive root, which has no "." entry.
        Err(error)
            if WIN32_ERROR::from_error(&error) == Some(ERROR_FILE_NOT_FOUND)
                && std::fs::metadata(&long).is_ok_and(|metadata| metadata.is_dir()) =>
        {
            return Ok(());
        }
        Err(error) => return Err(list_error(&error, &long)),
    };

    let started = Instant::now();
    let mut batch = Entries::default();
    let mut listed = 0;
    let mut sent_any = false;
    let mut last_sent = started;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        if !is_dot_entry(&data) {
            batch.push_find_data(&data);
            listed += 1;
            let due = batch.len() >= MAX_BATCH
                || (!sent_any && batch.len() >= FIRST_BATCH)
                || (listed % CLOCK_EVERY == 0 && last_sent.elapsed() >= BATCH_INTERVAL);
            if due {
                on_batch(&batch);
                batch.clear();
                sent_any = true;
                last_sent = Instant::now();
            }
        }
        // SAFETY: `handle` is an open find handle and `data` a valid WIN32_FIND_DATAW.
        if let Err(error) = unsafe { FindNextFileW(handle.0, &mut data) } {
            if WIN32_ERROR::from_error(&error) == Some(ERROR_NO_MORE_FILES) {
                break;
            }
            return Err(list_error(&error, &long));
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }
    if !batch.is_empty() {
        on_batch(&batch);
    }
    tracing::debug!(
        entries = listed,
        ms = started.elapsed().as_secs_f64() * 1000.0,
        "folder listed"
    );
    Ok(())
}

/// Closes a find handle when dropped.
struct FindHandle(HANDLE);

impl Drop for FindHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful FindFirstFileExW and is closed only here.
        if let Err(error) = unsafe { FindClose(self.0) } {
            tracing::warn!(%error, "FindClose failed");
        }
    }
}

/// The error for a failed directory read of `folder`. A path on a drive letter with no volume
/// behind it (an unplugged USB drive) is [`ErrorCode::DeviceGone`], not [`ErrorCode::NotFound`].
fn list_error(error: &windows::core::Error, folder: &Path) -> JerryError {
    let mut error = JerryError::new(ErrorCode::from_hresult(error.code())).at(folder);
    if error.code == ErrorCode::NotFound && drive_is_gone(folder) {
        error.code = ErrorCode::DeviceGone;
    }
    error
}

/// Whether `path` is on a drive letter that has no volume.
fn drive_is_gone(path: &Path) -> bool {
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return false;
    };
    let (Prefix::Disk(letter) | Prefix::VerbatimDisk(letter)) = prefix.kind() else {
        return false;
    };
    let root: Vec<u16> = format!(r"{}:\", char::from(letter))
        .encode_utf16()
        .chain([0])
        .collect();
    // SAFETY: `root` is NUL-terminated and outlives the call.
    unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) == DRIVE_NO_ROOT_DIR }
}

fn is_dot_entry(data: &WIN32_FIND_DATAW) -> bool {
    const DOT: u16 = b'.' as u16;
    matches!(data.cFileName, [DOT, 0, ..] | [DOT, DOT, 0, ..])
}

/// `folder\*`, NUL-terminated.
fn search_pattern(folder: &Path) -> Vec<u16> {
    let mut wide: Vec<u16> = folder.as_os_str().encode_wide().collect();
    if wide.last() != Some(&u16::from(b'\\')) {
        wide.push(u16::from(b'\\'));
    }
    wide.extend([u16::from(b'*'), 0]);
    wide
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(names: &[&str]) -> Entries {
        let mut entries = Entries::default();
        for (i, name) in names.iter().enumerate() {
            let units: Vec<u16> = name.encode_utf16().collect();
            entries.push(&units, i as u64, 10, 20 + i as u64, 0x20);
        }
        entries
    }

    #[test]
    fn append_continues_ids_and_name_offsets() {
        let mut all = entries(&["a.txt", "ნახაზი.dwg"]);
        all.append(&entries(&["file10", "b"]));
        assert_eq!(all.len(), 4);
        assert_eq!(all.name(1), "ნახაზი.dwg");
        assert_eq!(all.name(2), "file10");
        assert_eq!(all.name(3), "b");
        assert_eq!(all.sizes(), [0, 1, 0, 1]);
        assert_eq!(all.modified(), [20, 21, 20, 21]);
        assert_eq!(all.name_ends(), [5, 15, 21, 22]);
    }

    #[test]
    fn clear_empties_every_column() {
        let mut batch = entries(&["a", "b"]);
        batch.clear();
        assert_eq!(batch, Entries::default());
    }

    #[test]
    fn listing_paths_join_the_exact_name() {
        let mut listing = Listing::new(PathBuf::from(r"\\?\C:\Work"));
        let mut batch = Entries::default();
        // An unpaired surrogate: not valid Unicode, still a legal NTFS name.
        let odd = [0xD800, u16::from(b'x')];
        batch.push(&odd, 0, 0, 0, 0x20);
        listing.append(&batch);
        let path = listing.path(0).expect("entry 0");
        let mut expected: Vec<u16> = r"\\?\C:\Work\".encode_utf16().collect();
        expected.extend(odd);
        assert_eq!(path.as_os_str().encode_wide().collect::<Vec<_>>(), expected);
        assert_eq!(listing.path(1), None);
    }

    #[test]
    fn folders_are_told_by_their_attribute() {
        let mut batch = Entries::default();
        batch.push(
            &[u16::from(b'a')],
            0,
            0,
            0,
            FILE_ATTRIBUTE_DIRECTORY.0 | 0x2,
        );
        batch.push(&[u16::from(b'b')], 0, 0, 0, 0x20);
        assert!(batch.is_dir(0));
        assert!(!batch.is_dir(1));
    }

    #[test]
    fn filetimes_convert_to_unix_ms() {
        // 2026-01-01T00:00:00Z.
        assert_eq!(
            filetime_to_unix_ms(134_116_992_000_000_000),
            1_767_225_600_000.0
        );
        assert_eq!(filetime_to_unix_ms(116_444_736_000_000_000), 0.0);
    }

    #[test]
    fn search_patterns_add_one_separator() {
        let text = |pattern: Vec<u16>| String::from_utf16_lossy(&pattern);
        assert_eq!(
            text(search_pattern(Path::new(r"\\?\C:\"))),
            "\\\\?\\C:\\*\0"
        );
        assert_eq!(
            text(search_pattern(Path::new(r"\\?\C:\a"))),
            "\\\\?\\C:\\a\\*\0"
        );
    }

    #[test]
    fn only_dot_and_dot_dot_are_skipped() {
        let data = |name: &str| {
            let mut data = WIN32_FIND_DATAW::default();
            for (slot, unit) in data.cFileName.iter_mut().zip(name.encode_utf16()) {
                *slot = unit;
            }
            data
        };
        assert!(is_dot_entry(&data(".")));
        assert!(is_dot_entry(&data("..")));
        assert!(!is_dot_entry(&data(".config")));
        assert!(!is_dot_entry(&data("...")));
    }
}
