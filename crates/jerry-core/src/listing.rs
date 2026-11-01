//! Folder listing with `FindFirstFileExW` (`FindExInfoBasic`, `FIND_FIRST_EX_LARGE_FETCH`): name,
//! size, dates and attributes in one call per entry, without opening any file.
//!
//! Entries are kept as columns. Names stay UTF-16, exactly as Windows returns them, so a name that
//! isn't valid Unicode still round-trips to the same `OsString` for file operations.

use std::ffi::OsString;
use std::ops::Range;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, HANDLE, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{
    FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    FindNextFileW, WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

use crate::error::{ErrorCode, JerryError};

/// Entries in the first batch: more than a screen of rows, small enough to send at once.
pub const FIRST_BATCH: usize = 256;

/// Time between later batches (Tech PRD: the first at once, then one every 50 ms).
pub const BATCH_INTERVAL: Duration = Duration::from_millis(50);

/// `FILE_ATTRIBUTE_DIRECTORY`.
pub const ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// One folder's entries as columns. An entry's id is its index.
#[derive(Debug, Default, Clone)]
pub struct Entries {
    /// Every name as UTF-16, back to back. Name `i` ends at `name_ends[i]`.
    name_units: Vec<u16>,
    name_ends: Vec<u32>,
    /// Size in bytes (0 for folders).
    pub sizes: Vec<u64>,
    /// Last write time as a FILETIME: 100 ns ticks since 1601-01-01 UTC.
    pub modified: Vec<u64>,
    /// `FILE_ATTRIBUTE_*` flags.
    pub attributes: Vec<u32>,
}

impl Entries {
    pub fn len(&self) -> usize {
        self.sizes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sizes.is_empty()
    }

    /// Where name `id` sits in [`Self::name_units`].
    pub fn name_range(&self, id: usize) -> Range<usize> {
        let start = if id == 0 {
            0
        } else {
            self.name_ends[id - 1] as usize
        };
        start..self.name_ends[id] as usize
    }

    /// Name `id` as UTF-16 code units, exactly as Windows returned it.
    pub fn name_utf16(&self, id: usize) -> &[u16] {
        &self.name_units[self.name_range(id)]
    }

    /// Name `id` as the exact `OsString`.
    pub fn name(&self, id: usize) -> OsString {
        OsString::from_wide(self.name_utf16(id))
    }

    /// All names back to back; see [`Self::name_range`].
    pub fn name_units(&self) -> &[u16] {
        &self.name_units
    }

    pub fn is_dir(&self, id: usize) -> bool {
        self.attributes[id] & ATTRIBUTE_DIRECTORY != 0
    }

    /// Appends one entry (used by the listing and by tests).
    pub fn push(&mut self, name: &[u16], size: u64, modified: u64, attributes: u32) {
        self.name_units.extend_from_slice(name);
        // Names are at most 255 units and a folder holds far fewer than 16 million of them.
        self.name_ends.push(self.name_units.len() as u32);
        self.sizes.push(size);
        self.modified.push(modified);
        self.attributes.push(attributes);
    }

    fn push_find_data(&mut self, data: &WIN32_FIND_DATAW) {
        let len = data
            .cFileName
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(data.cFileName.len());
        self.push(
            &data.cFileName[..len],
            (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow),
            (u64::from(data.ftLastWriteTime.dwHighDateTime) << 32)
                | u64::from(data.ftLastWriteTime.dwLowDateTime),
            data.dwFileAttributes,
        );
    }
}

/// Converts a FILETIME (100 ns ticks since 1601) to milliseconds since the Unix epoch.
pub fn filetime_to_unix_ms(filetime: u64) -> f64 {
    const UNIX_EPOCH_MS: f64 = 11_644_473_600_000.0;
    filetime as f64 / 10_000.0 - UNIX_EPOCH_MS
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

/// Reads `dir` and returns its entries ("." and ".." left out). `on_batch` gets each new range of
/// entries: the first once [`FIRST_BATCH`] entries are in, then at most every [`BATCH_INTERVAL`],
/// and the rest at the end. Setting `cancel` stops the listing with [`ErrorCode::Cancelled`].
///
/// Blocking IO: call it on an IO thread, never on the UI thread or the async runtime.
pub fn list_dir(
    dir: &Path,
    cancel: &AtomicBool,
    mut on_batch: impl FnMut(&Entries, Range<usize>),
) -> Result<Entries, JerryError> {
    let pattern = search_pattern(dir);
    let mut data = WIN32_FIND_DATAW::default();
    // SAFETY: `pattern` is NUL-terminated and outlives the call; `data` is a WIN32_FIND_DATAW, the
    // struct FindExInfoBasic fills.
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
        // An empty drive root has no "." entry, so nothing matches.
        Err(error) if WIN32_ERROR::from_error(&error) == Some(ERROR_FILE_NOT_FOUND) => {
            return Ok(Entries::default());
        }
        Err(error) => return Err(win32_error(&error, dir)),
    };

    let mut entries = Entries::default();
    let mut sent = 0;
    let mut last_sent = Instant::now();
    loop {
        if !is_dot_entry(&data) {
            entries.push_find_data(&data);
            let len = entries.len();
            let due = if sent == 0 {
                len >= FIRST_BATCH
            } else {
                len % 64 == 0 && last_sent.elapsed() >= BATCH_INTERVAL
            };
            if due {
                if cancel.load(Ordering::Relaxed) {
                    return Err(JerryError::new(ErrorCode::Cancelled, dir));
                }
                on_batch(&entries, sent..len);
                sent = len;
                last_sent = Instant::now();
            }
        }
        // SAFETY: `handle` is an open find handle and `data` a valid WIN32_FIND_DATAW.
        if let Err(error) = unsafe { FindNextFileW(handle.0, &mut data) } {
            if WIN32_ERROR::from_error(&error) == Some(ERROR_NO_MORE_FILES) {
                break;
            }
            return Err(win32_error(&error, dir));
        }
    }
    if entries.len() > sent {
        on_batch(&entries, sent..entries.len());
    }
    Ok(entries)
}

fn win32_error(error: &windows::core::Error, path: &Path) -> JerryError {
    match WIN32_ERROR::from_error(error) {
        Some(code) => JerryError::from_win32(code.0, path),
        None => JerryError::new(ErrorCode::Os(error.code().0 as u32), path),
    }
}

fn is_dot_entry(data: &WIN32_FIND_DATAW) -> bool {
    const DOT: u16 = b'.' as u16;
    matches!(data.cFileName, [DOT, 0, ..] | [DOT, DOT, 0, ..])
}

/// `dir\*` as a NUL-terminated `\\?\` long path.
fn search_pattern(dir: &Path) -> Vec<u16> {
    let mut wide: Vec<u16> = long_path(dir).as_os_str().encode_wide().collect();
    if wide.last() != Some(&u16::from(b'\\')) {
        wide.push(u16::from(b'\\'));
    }
    wide.extend([u16::from(b'*'), 0]);
    wide
}

/// Adds the `\\?\` prefix so paths over 260 characters work.
pub fn long_path(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let text = absolute.as_os_str();
    let wide: Vec<u16> = text.encode_wide().collect();
    let starts = |prefix: &str| {
        let prefix: Vec<u16> = prefix.encode_utf16().collect();
        wide.starts_with(&prefix)
    };
    if starts(r"\\?\") || starts(r"\\.\") {
        absolute
    } else if starts(r"\\") {
        let mut long = OsString::from(r"\\?\UNC\");
        long.push(OsString::from_wide(&wide[2..]));
        PathBuf::from(long)
    } else {
        let mut long = OsString::from(r"\\?\");
        long.push(text);
        PathBuf::from(long)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_names_sizes_and_folders() {
        let dir = tempfile_dir("listing");
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        std::fs::write(dir.join("ნახაზი 2.dwg"), b"").unwrap();
        std::fs::create_dir(dir.join("Folder 1")).unwrap();

        let mut batches = Vec::new();
        let entries = list_dir(&dir, &AtomicBool::new(false), |_, range| {
            batches.push(range)
        })
        .unwrap();

        let mut names: Vec<String> = (0..entries.len())
            .map(|id| entries.name(id).into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["Folder 1", "a.txt", "ნახაზი 2.dwg"]);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0], 0..3);
        let a = (0..3).find(|&id| entries.name(id) == "a.txt").unwrap();
        assert_eq!(entries.sizes[a], 5);
        let folder = (0..3).find(|&id| entries.name(id) == "Folder 1").unwrap();
        assert!(entries.is_dir(folder));
        assert!(!entries.is_dir(a));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_folder_is_not_found() {
        let dir = std::env::temp_dir().join("jerry-test-missing-folder-xyz");
        let error = list_dir(&dir, &AtomicBool::new(false), |_, _| {}).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn long_path_prefixes() {
        assert_eq!(long_path(Path::new(r"C:\a")), PathBuf::from(r"\\?\C:\a"));
        assert_eq!(
            long_path(Path::new(r"\\srv\share\a")),
            PathBuf::from(r"\\?\UNC\srv\share\a")
        );
        assert_eq!(
            long_path(Path::new(r"\\?\C:\a")),
            PathBuf::from(r"\\?\C:\a")
        );
    }

    pub(crate) fn tempfile_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "jerry-test-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
