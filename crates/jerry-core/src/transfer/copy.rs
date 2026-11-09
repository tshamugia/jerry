//! One file: written under a temporary name, checked, and only then given its final name (data
//! safety rules 1 and 2).

use std::any::Any;
use std::ffi::{OsString, c_void};
use std::fs::OpenOptions;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};
use std::time::Duration;

use windows::Win32::Foundation::{ERROR_CRC, FILETIME, HANDLE};
use windows::Win32::Storage::FileSystem::{
    COPY_FILE_FAIL_IF_EXISTS, COPY_FILE_NO_BUFFERING, COPYFILE_FLAGS, COPYPROGRESSROUTINE_PROGRESS,
    CopyFileExW, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_NOT_CONTENT_INDEXED, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAGS_AND_ATTRIBUTES, FILE_WRITE_ATTRIBUTES,
    GetFileAttributesExW, GetFileAttributesW, GetFileExInfoStandard, INVALID_FILE_ATTRIBUTES,
    LPPROGRESS_ROUTINE_CALLBACK_REASON, PROGRESS_CANCEL, PROGRESS_CONTINUE, SetFileAttributesW,
    SetFileTime, WIN32_FILE_ATTRIBUTE_DATA,
};
use windows::core::PCWSTR;

use super::control::Flow;
use crate::delete::{probe, remove};
use crate::error::{ErrorCode, JerryError};
use crate::rename::{is_taken, move_file};

/// From this size on, files are copied past the system cache: a big copy doesn't push everything
/// else out of memory, and buffered writes wouldn't make it faster.
const NO_BUFFERING_FROM: u64 = 256 * 1024 * 1024;

/// The longest name a file system takes, in UTF-16 units.
const MAX_NAME: usize = 255;

/// The ending of a file that is still being written.
const PART: &str = ".jerrypart";

/// How many temporary names are tried next to files of that name Jerry didn't make in this run.
const PART_NAMES: u32 = 100;

/// How long to wait between attempts on a file another program holds for a moment.
const HELD_FOR: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(300),
    Duration::from_millis(800),
];

/// What writes one file: [`copy_file`], or a fake in tests.
pub(super) type Copier = fn(&Request<'_>, &mut dyn FnMut(u64) -> Flow) -> Result<u64, JerryError>;

/// The attributes a copy takes from its source. The rest describe how the file is stored, such as
/// a cloud placeholder's, and `CopyFileExW` deals with those.
const KEPT_ATTRIBUTES: u32 = FILE_ATTRIBUTE_READONLY.0
    | FILE_ATTRIBUTE_HIDDEN.0
    | FILE_ATTRIBUTE_SYSTEM.0
    | FILE_ATTRIBUTE_ARCHIVE.0
    | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED.0;

/// What a file looks like from outside. Reading it doesn't open the file, so it works on locked
/// files and doesn't download cloud files.
#[derive(Debug, Clone, Copy)]
pub(super) struct Stamp {
    pub(super) attributes: u32,
    pub(super) size: u64,
    pub(super) created: FILETIME,
    pub(super) written: FILETIME,
}

impl Stamp {
    pub(super) fn same_content(&self, other: &Self) -> bool {
        self.size == other.size && self.written == other.written
    }
}

/// The [`Stamp`] of the file or folder at `path`, a link itself and not what it points to.
pub(super) fn stamp(path: &Path) -> Result<Stamp, JerryError> {
    let name = wide(path);
    let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
    // SAFETY: `name` is NUL-terminated and outlives the call; `data` is the structure
    // GetFileExInfoStandard fills.
    unsafe {
        GetFileAttributesExW(
            PCWSTR(name.as_ptr()),
            GetFileExInfoStandard,
            (&raw mut data).cast(),
        )
    }
    .map_err(|error| JerryError::from(error).at(path))?;
    Ok(Stamp {
        attributes: data.dwFileAttributes,
        size: (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow),
        created: data.ftCreationTime,
        written: data.ftLastWriteTime,
    })
}

/// One file to write.
pub(super) struct Request<'a> {
    pub(super) source: &'a Path,
    pub(super) target: &'a Path,
    /// A move, so the source goes next: the copy must be on the disk and the source unchanged.
    pub(super) for_move: bool,
}

/// Writes `request.source` to `request.target` and returns its size. `chunk` hears the bytes
/// written so far, starting with 0, and can stop the copy. Nothing stays behind if it fails: no
/// final name, no temporary file. An existing target is never replaced.
pub(super) fn copy_file(
    request: &Request<'_>,
    chunk: &mut dyn FnMut(u64) -> Flow,
) -> Result<u64, JerryError> {
    // Saves copying a file for nothing. The final rename is what guarantees no replacing.
    if stamp(request.target).is_ok() {
        return Err(taken(request.target));
    }
    let before = stamp(request.source)?;
    let part = write_part(request, before.size, chunk)?;
    finish(request, &part, &before).inspect_err(|_| discard(&part))?;
    Ok(before.size)
}

fn taken(target: &Path) -> JerryError {
    JerryError::new(ErrorCode::InvalidName).at(target)
}

/// Copies to a temporary name that nobody else uses, and returns the name.
fn write_part(
    request: &Request<'_>,
    size: u64,
    chunk: &mut dyn FnMut(u64) -> Flow,
) -> Result<PathBuf, JerryError> {
    let flags = if size >= NO_BUFFERING_FROM {
        COPY_FILE_FAIL_IF_EXISTS | COPY_FILE_NO_BUFFERING
    } else {
        COPY_FILE_FAIL_IF_EXISTS
    };
    for attempt in 0..PART_NAMES {
        let part = part_name(request.target, attempt);
        match copy_to(request.source, &part, flags, chunk)? {
            Written::Yes => return Ok(part),
            // Not this run's file, so it isn't this run's to touch (data safety rule 7).
            Written::NameTaken => {}
        }
    }
    Err(taken(request.target))
}

/// The temporary name next to `target`: `name.ext.jerrypart`, shorter where a long name leaves no
/// room for the ending, and with a number after the first attempt.
fn part_name(target: &Path, attempt: u32) -> PathBuf {
    let ending = if attempt == 0 {
        PART.to_owned()
    } else {
        format!(".{attempt}{PART}")
    };
    let name: Vec<u16> = target
        .file_name()
        .unwrap_or_default()
        .encode_wide()
        .collect();
    let mut keep = name.len().min(MAX_NAME.saturating_sub(ending.len()));
    // Not between the two halves of a surrogate pair.
    if name
        .get(keep)
        .is_some_and(|unit| (0xDC00..=0xDFFF).contains(unit))
    {
        keep = keep.saturating_sub(1);
    }
    let mut part = OsString::from_wide(&name[..keep]);
    part.push(ending);
    target.with_file_name(part)
}

enum Written {
    Yes,
    NameTaken,
}

/// What the progress routine works with during one `CopyFileExW` call.
struct Context<'a> {
    part: &'a [u16],
    /// The routine has run, so the temporary file exists and this call made it.
    started: bool,
    chunk: &'a mut dyn FnMut(u64) -> Flow,
    /// A panic in `chunk`, carried over the system frames to the job's thread.
    panic: Option<Box<dyn Any + Send>>,
}

impl Context<'_> {
    fn on_chunk(&mut self, transferred: i64) -> Flow {
        if !self.started {
            self.started = true;
            hide(self.part);
        }
        (self.chunk)(u64::try_from(transferred).unwrap_or(0))
    }
}

/// `LPPROGRESS_ROUTINE`. Windows calls it when the temporary file exists and after each chunk.
///
/// # Safety
/// `data` is the `Context` that `copy_to` passes to `CopyFileExW`.
unsafe extern "system" fn progress_routine(
    _total: i64,
    transferred: i64,
    _stream_total: i64,
    _stream_transferred: i64,
    _stream: u32,
    _reason: LPPROGRESS_ROUTINE_CALLBACK_REASON,
    _source: HANDLE,
    _destination: HANDLE,
    data: *const c_void,
) -> COPYPROGRESSROUTINE_PROGRESS {
    // SAFETY: `copy_to` passes a `Context` that lives through the `CopyFileExW` call, which calls
    // this routine on the same thread, one call at a time, while nothing else uses it.
    let context = unsafe { &mut *data.cast_mut().cast::<Context<'_>>() };
    // A panic can't unwind out of an `extern "system"` function: it would end the process.
    match catch_unwind(AssertUnwindSafe(|| context.on_chunk(transferred))) {
        Ok(Flow::Go | Flow::Resumed) => PROGRESS_CONTINUE,
        Ok(Flow::Cancelled) => PROGRESS_CANCEL,
        Err(payload) => {
            context.panic = Some(payload);
            PROGRESS_CANCEL
        }
    }
}

/// One `CopyFileExW` call onto `part`. If it fails, no part of the file stays.
fn copy_to(
    source: &Path,
    part: &Path,
    flags: COPYFILE_FLAGS,
    chunk: &mut dyn FnMut(u64) -> Flow,
) -> Result<Written, JerryError> {
    let from = wide(source);
    let to = wide(part);
    let mut context = Context {
        part: &to,
        started: false,
        chunk,
        panic: None,
    };
    // SAFETY: both paths are NUL-terminated and outlive the call, and so does `context`, which is
    // what the routine gets as its data.
    let result = unsafe {
        CopyFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            Some(progress_routine),
            Some((&raw const context).cast()),
            None,
            flags,
        )
    };
    let started = context.started;
    let panic = context.panic.take();
    match result {
        Ok(()) => Ok(Written::Yes),
        Err(error) => {
            if started {
                // Windows removes it when told to cancel. This is for when it doesn't.
                discard(part);
            }
            if let Some(payload) = panic {
                resume_unwind(payload);
            }
            if !started && is_taken(&error) {
                return Ok(Written::NameTaken);
            }
            Err(JerryError::from(error).at(source))
        }
    }
}

/// Sets the hidden attribute of the file being written. Without it the file is still safe under
/// its temporary name, so a file system that refuses only costs a visible temporary file.
fn hide(part: &[u16]) {
    // SAFETY: `part` is NUL-terminated and outlives the call.
    let attributes = unsafe { GetFileAttributesW(PCWSTR(part.as_ptr())) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        tracing::warn!("can't read the attributes of a file being copied");
        return;
    }
    // SAFETY: as above.
    let hidden = unsafe {
        SetFileAttributesW(
            PCWSTR(part.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(attributes | FILE_ATTRIBUTE_HIDDEN.0),
        )
    };
    if let Err(error) = hidden {
        tracing::warn!(%error, "can't hide a file being copied");
    }
}

/// Turns the finished temporary file into the final one, or fails and leaves it to the caller.
fn finish(request: &Request<'_>, part: &Path, before: &Stamp) -> Result<(), JerryError> {
    let written = stamp(part)?;
    check_size(before.size, written.size, part)?;
    if request.for_move {
        // The file is about to become the only copy.
        write_to_disk(part, &written)?;
        set_times(part, Some(&before.created), None)?;
        if !stamp(request.source)?.same_content(before) {
            // Something wrote to it meanwhile, and that isn't in the copy.
            return Err(JerryError::new(ErrorCode::InUse { app: None }).at(request.source));
        }
    }
    set_attributes(part, before.attributes)?;
    patiently(|| {
        move_file(part, request.target).map_err(|error| {
            if is_taken(&error) {
                taken(request.target)
            } else {
                JerryError::from(error).at(request.target)
            }
        })
    })
}

/// Does `attempt`, again after a moment if the file is held. Virus scanners, the indexer and
/// OneDrive open a file as soon as it appears; that shouldn't cost a finished copy.
pub(super) fn patiently(
    mut attempt: impl FnMut() -> Result<(), JerryError>,
) -> Result<(), JerryError> {
    for wait in HELD_FOR {
        match attempt() {
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::InUse { .. } | ErrorCode::AccessDenied
                ) =>
            {
                std::thread::sleep(wait);
            }
            result => return result,
        }
    }
    attempt()
}

fn check_size(expected: u64, written: u64, part: &Path) -> Result<(), JerryError> {
    if written == expected {
        Ok(())
    } else {
        tracing::error!(
            expected,
            written,
            "a copy doesn't have the size of its source"
        );
        Err(JerryError::from_win32(ERROR_CRC.0).at(part))
    }
}

/// Makes sure what was written is on the disk and not only in the cache.
fn write_to_disk(part: &Path, written: &Stamp) -> Result<(), JerryError> {
    // A read-only file can't be opened for writing. The source's attributes come back later.
    if written.attributes & FILE_ATTRIBUTE_READONLY.0 != 0 {
        set_attributes(part, written.attributes & !FILE_ATTRIBUTE_READONLY.0)?;
    }
    let flushed = OpenOptions::new()
        .write(true)
        .open(part)
        .and_then(|file| file.sync_all());
    flushed.map_err(|error| JerryError::from(error).at(part))
}

/// Sets the attributes a copy keeps of `attributes`.
pub(super) fn set_attributes(path: &Path, attributes: u32) -> Result<(), JerryError> {
    let kept = attributes & KEPT_ATTRIBUTES;
    let value = if kept == 0 {
        FILE_ATTRIBUTE_NORMAL.0
    } else {
        kept
    };
    let name = wide(path);
    // SAFETY: `name` is NUL-terminated and outlives the call.
    unsafe { SetFileAttributesW(PCWSTR(name.as_ptr()), FILE_FLAGS_AND_ATTRIBUTES(value)) }
        .map_err(|error| JerryError::from(error).at(path))
}

/// Sets the given times of a file or folder.
pub(super) fn set_times(
    path: &Path,
    created: Option<&FILETIME>,
    written: Option<&FILETIME>,
) -> Result<(), JerryError> {
    let file = OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)
        .map_err(|error| JerryError::from(error).at(path))?;
    let handle = HANDLE(file.as_raw_handle());
    // SAFETY: `handle` belongs to `file`, which outlives the call; the times are valid references.
    unsafe {
        SetFileTime(
            handle,
            created.map(std::ptr::from_ref),
            None,
            written.map(std::ptr::from_ref),
        )
    }
    .map_err(|error| JerryError::from(error).at(path))
}

/// Removes a temporary file this run made, if it is still there.
fn discard(part: &Path) {
    let Ok(item) = probe(part) else { return };
    if let Err(error) = remove(&item) {
        tracing::warn!(%error, "can't remove a partly copied file");
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(path: &Path) -> usize {
        path.file_name().unwrap().encode_wide().count()
    }

    #[test]
    fn the_temporary_name_ends_the_final_one() {
        assert_eq!(
            part_name(Path::new(r"\\?\C:\a\ნახაზი.dwg"), 0),
            PathBuf::from(r"\\?\C:\a\ნახაზი.dwg.jerrypart")
        );
        assert_eq!(
            part_name(Path::new(r"\\?\C:\a\b.txt"), 3),
            PathBuf::from(r"\\?\C:\a\b.txt.3.jerrypart")
        );
    }

    #[test]
    fn a_long_name_leaves_room_for_the_ending() {
        let name = "ა".repeat(255);
        let part = part_name(&PathBuf::from(r"\\?\C:\a").join(&name), 0);
        assert_eq!(units(&part), MAX_NAME);
        assert!(part.to_string_lossy().ends_with(PART));
        let numbered = part_name(&PathBuf::from(r"\\?\C:\a").join(&name), 12);
        assert_eq!(units(&numbered), MAX_NAME);
        assert!(numbered.to_string_lossy().ends_with(".12.jerrypart"));
    }

    #[test]
    fn a_surrogate_pair_is_not_cut_in_half() {
        // 244 units, then a pair that would straddle the 245 units the ending leaves.
        let name = format!("{}😀", "a".repeat(244));
        let part = part_name(&PathBuf::from(r"\\?\C:\a").join(&name), 0);
        assert_eq!(units(&part), 244 + PART.len());
        assert!(!part.to_string_lossy().contains('\u{FFFD}'));
    }

    #[test]
    fn a_file_held_for_a_moment_is_tried_again() {
        let mut attempts = 0;
        let result = patiently(|| {
            attempts += 1;
            if attempts < 3 {
                Err(JerryError::new(ErrorCode::InUse { app: None }))
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Ok(()));
        assert_eq!(attempts, 3);
    }

    #[test]
    fn other_failures_are_not_tried_again() {
        let mut attempts = 0;
        let result = patiently(|| {
            attempts += 1;
            Err(JerryError::new(ErrorCode::InvalidName))
        });
        assert_eq!(result, Err(JerryError::new(ErrorCode::InvalidName)));
        assert_eq!(attempts, 1, "a taken name isn't a held file");
    }

    #[test]
    fn a_file_held_for_longer_fails_in_the_end() {
        let mut attempts = 0;
        let result = patiently(|| {
            attempts += 1;
            Err(JerryError::new(ErrorCode::AccessDenied))
        });
        assert_eq!(result, Err(JerryError::new(ErrorCode::AccessDenied)));
        assert_eq!(attempts, HELD_FOR.len() + 1);
    }

    #[test]
    fn a_copy_of_the_wrong_size_is_refused() {
        let part = Path::new(r"\\?\C:\a.jerrypart");
        assert_eq!(check_size(5, 5, part), Ok(()));
        let error = check_size(5, 4, part).expect_err("short");
        assert_eq!(error.code, ErrorCode::Os { os: ERROR_CRC.0 });
        assert_eq!(error.path.as_deref(), Some(part));
    }
}
