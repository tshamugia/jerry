//! The parallel walker behind [`search`](super::search): one task per folder on the search's own
//! rayon pool, and the calling thread collecting what the tasks find into batches.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::num::NonZero;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

use rayon::{Scope, ThreadPool, ThreadPoolBuilder};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, HANDLE, WIN32_ERROR};
use windows::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO,
    FIND_FIRST_EX_LARGE_FETCH, FileIdInfo, FindExInfoBasic, FindExSearchNameMatch,
    FindFirstFileExW, FindNextFileW, GetFileInformationByHandle, GetFileInformationByHandleEx,
    WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

use super::query::Matcher;
use super::{BATCH_INTERVAL, SearchBatch, SearchProgress, SearchQuery};
use crate::delete::{NAME_SURROGATE, is_within};
use crate::error::{ErrorCode, JerryError};
use crate::listing::{
    Entries, FindHandle, MAX_BATCH, find_data_name, folder_error, is_dot_entry, list_error,
};
use crate::path::long_path;

/// Results a folder's task gathers before it hands them on, at most.
const PIECE: usize = 256;

/// A folder's task looks at the clock once per this many entries: to hand on what it has and
/// count what it checked, for a big folder on a slow share.
const CLOCK_EVERY: u64 = 32;

/// `FILE_READ_ATTRIBUTES`: enough to ask a folder for its file ID.
const FILE_READ_ATTRIBUTES: u32 = 0x80;

/// `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`: asking for an ID gets in no one's
/// way.
const SHARE_ALL: u32 = 0x7;

const BACKSLASH: u16 = b'\\' as u16;

/// A folder by its volume and file ID: the same folder, whatever path leads to it.
type FolderKey = (u64, u128);

/// How a folder's task came to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reached {
    /// The folder the search starts in.
    Root,
    /// A folder in a folder.
    Folder,
    /// A link (followed only when the query says so).
    Link,
}

/// Results of one folder, from its task to the collector.
struct Piece {
    /// The folder task that found them: a big folder hands on several pieces.
    task: u64,
    folder: Arc<[u16]>,
    entries: Entries,
}

/// What every folder task shares.
struct Walker<'a> {
    matcher: Matcher,
    hidden: bool,
    follow_links: bool,
    /// Folders not to enter, as wide long paths.
    excluded: Vec<Vec<u16>>,
    cancel: &'a AtomicBool,
    /// With links followed: every folder entered so far.
    seen: Mutex<HashSet<FolderKey>>,
    /// Folder tasks that read their folder, to number them.
    tasks: AtomicU64,
    checked: AtomicU64,
    skipped: AtomicU64,
    /// Why the folder the search starts in couldn't be read.
    failed: Mutex<Option<JerryError>>,
}

pub(super) fn run(
    root: &Path,
    query: &SearchQuery,
    excluded: &[PathBuf],
    cancel: &AtomicBool,
    mut on_batch: impl FnMut(&SearchBatch, SearchProgress),
) -> Result<SearchProgress, JerryError> {
    let started = Instant::now();
    let root = long_path(root)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(JerryError::new(ErrorCode::Cancelled).at(&root));
    }
    if let Some(error) = folder_error(&root) {
        return Err(error);
    }
    let walker = Walker {
        matcher: Matcher::new(query),
        hidden: query.hidden,
        follow_links: query.follow_links,
        excluded: excluded
            .iter()
            .filter_map(|folder| long_path(folder).ok())
            // The user chose to search in it.
            .filter(|folder| !is_within(&root, folder))
            .map(|folder| folder.as_os_str().encode_wide().collect())
            .collect(),
        cancel,
        seen: Mutex::new(HashSet::new()),
        tasks: AtomicU64::new(0),
        checked: AtomicU64::new(0),
        skipped: AtomicU64::new(0),
        failed: Mutex::new(None),
    };
    if walker.follow_links {
        // Seen already, so no link leads into one either.
        let mut seen = lock(&walker.seen);
        seen.extend(
            walker
                .excluded
                .iter()
                .filter_map(|folder| folder_key(folder)),
        );
    }

    let (sender, receiver) = mpsc::channel();
    let start: Arc<[u16]> = root.as_os_str().encode_wide().collect();
    let walker = &walker;
    let progress = in_scope(|scope| {
        scope.spawn(move |scope| walker.walk(scope, start, &sender, Reached::Root));
        collect(walker, &receiver, &root, &mut on_batch)
    })?;
    tracing::debug!(
        found = progress.found,
        checked = progress.checked,
        skipped = progress.skipped,
        ms = started.elapsed().as_secs_f64() * 1000.0,
        "search done"
    );
    Ok(progress)
}

/// Runs `op` with a scope whose tasks go to the search's pool, `op` itself on this thread.
fn in_scope<'s, R>(op: impl FnOnce(&Scope<'s>) -> R) -> R {
    match pool() {
        Some(pool) => pool.in_place_scope(op),
        None => rayon::in_place_scope(op),
    }
}

/// The search's own pool, started with the first search: a search on a slow share holds up only
/// other searches, never the sort's pool. `None` if Windows wouldn't give it threads; searches
/// share rayon's global pool then.
fn pool() -> Option<&'static ThreadPool> {
    static POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism().map_or(4, NonZero::get);
        ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|index| format!("jerry-search-{index}"))
            .build()
            .inspect_err(|error| {
                tracing::error!(%error, "no search pool: searches run on the shared one");
            })
            .ok()
    })
    .as_ref()
}

/// Gathers the folder tasks' pieces into batches for `on_batch` until every task is done (all
/// senders are gone), and returns the final progress.
fn collect(
    walker: &Walker<'_>,
    receiver: &Receiver<Piece>,
    root: &Path,
    on_batch: &mut impl FnMut(&SearchBatch, SearchProgress),
) -> Result<SearchProgress, JerryError> {
    let mut batch = SearchBatch::default();
    // Folder ids by the task that found results in the folder.
    let mut folder_ids: HashMap<u64, u32> = HashMap::new();
    let (mut found, mut folders) = (0_usize, 0_usize);
    let mut results_sent = false;
    let mut sent = Instant::now();
    loop {
        let wait = BATCH_INTERVAL.saturating_sub(sent.elapsed());
        let done = match receiver.recv_timeout(wait) {
            Ok(piece) => {
                let folder = *folder_ids.entry(piece.task).or_insert_with(|| {
                    batch
                        .folders
                        .push(PathBuf::from(OsString::from_wide(&piece.folder)));
                    folders += 1;
                    folders as u32 - 1
                });
                batch
                    .folders_of
                    .extend(std::iter::repeat_n(folder, piece.entries.len()));
                batch.entries.append(&piece.entries);
                found += piece.entries.len();
                false
            }
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => true,
        };
        if walker.cancelled() {
            return Err(JerryError::new(ErrorCode::Cancelled).at(root));
        }
        if done {
            break;
        }
        let due = sent.elapsed() >= BATCH_INTERVAL
            || (!results_sent && !batch.is_empty())
            || batch.entries.len() >= MAX_BATCH;
        if due {
            on_batch(&batch, walker.progress(found));
            results_sent |= !batch.is_empty();
            batch = SearchBatch {
                first: found,
                first_folder: folders,
                ..SearchBatch::default()
            };
            sent = Instant::now();
        }
    }
    if let Some(error) = lock(&walker.failed).take() {
        return Err(error);
    }
    let progress = walker.progress(found);
    if !batch.is_empty() {
        on_batch(&batch, progress);
    }
    Ok(progress)
}

impl Walker<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn progress(&self, found: usize) -> SearchProgress {
        SearchProgress {
            found: found as u64,
            checked: self.checked.load(Ordering::Relaxed),
            skipped: self.skipped.load(Ordering::Relaxed),
        }
    }

    /// Reads `folder`: hands its matching entries on through `sender`, and spawns a task for
    /// every folder in it that is to be searched.
    fn walk<'s>(
        &'s self,
        scope: &Scope<'s>,
        folder: Arc<[u16]>,
        sender: &Sender<Piece>,
        reached: Reached,
    ) {
        if self.cancelled() {
            return;
        }
        if self.follow_links {
            match folder_key(&folder) {
                Some(key) => {
                    // A link back up the tree, or a second way into a folder.
                    if !lock(&self.seen).insert(key) {
                        return;
                    }
                }
                // Without an ID a loop through it couldn't be seen.
                None if reached == Reached::Link => {
                    self.skipped.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                None => {}
            }
        }

        let mut pattern = Vec::with_capacity(folder.len() + 3);
        pattern.extend_from_slice(&folder);
        if pattern.last() != Some(&BACKSLASH) {
            pattern.push(BACKSLASH);
        }
        pattern.extend([u16::from(b'*'), 0]);
        let mut data = WIN32_FIND_DATAW::default();
        // SAFETY: `pattern` is NUL-terminated and outlives the call; `data` is the
        // WIN32_FIND_DATAW that FindExInfoBasic fills.
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
            Err(error) if WIN32_ERROR::from_error(&error) == Some(ERROR_FILE_NOT_FOUND) => return,
            Err(error) => return self.fail(&folder, &error, reached),
        };

        let task = self.tasks.fetch_add(1, Ordering::Relaxed);
        let mut piece = Entries::default();
        let mut folded = Vec::new();
        let (mut checked, mut counted) = (0_u64, 0_u64);
        let mut clock = Instant::now();
        loop {
            if self.cancelled() {
                return;
            }
            if !is_dot_entry(&data) {
                checked += 1;
                self.consider(scope, &folder, &data, sender, &mut piece, &mut folded);
                let late = checked % CLOCK_EVERY == 0 && clock.elapsed() >= BATCH_INTERVAL;
                if piece.len() >= PIECE || late {
                    self.hand_on(task, &folder, &mut piece, sender);
                }
                if late {
                    self.checked.fetch_add(checked - counted, Ordering::Relaxed);
                    counted = checked;
                    clock = Instant::now();
                }
            }
            // SAFETY: `handle` is an open find handle and `data` a valid WIN32_FIND_DATAW.
            if let Err(error) = unsafe { FindNextFileW(handle.0, &mut data) } {
                if WIN32_ERROR::from_error(&error) != Some(ERROR_NO_MORE_FILES) {
                    // The folder or its drive went away while it was read; what was read stays.
                    self.fail(&folder, &error, reached);
                }
                break;
            }
        }
        self.checked.fetch_add(checked - counted, Ordering::Relaxed);
        self.hand_on(task, &folder, &mut piece, sender);
    }

    /// One entry of `folder`: a result if it matches, and searched if it is a folder to search.
    fn consider<'s>(
        &'s self,
        scope: &Scope<'s>,
        folder: &[u16],
        data: &WIN32_FIND_DATAW,
        sender: &Sender<Piece>,
        piece: &mut Entries,
        folded: &mut Vec<char>,
    ) {
        let attributes = data.dwFileAttributes;
        if !self.hidden && attributes & FILE_ATTRIBUTE_HIDDEN.0 != 0 {
            return;
        }
        let name = find_data_name(data);
        if attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
            let link = is_link(data);
            if !link || self.follow_links {
                let inner = join(folder, name);
                if !self.is_excluded(&inner) {
                    let sender = sender.clone();
                    let reached = if link { Reached::Link } else { Reached::Folder };
                    scope.spawn(move |scope| self.walk(scope, inner, &sender, reached));
                }
            }
        }
        let size = (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow);
        let modified = (u64::from(data.ftLastWriteTime.dwHighDateTime) << 32)
            | u64::from(data.ftLastWriteTime.dwLowDateTime);
        if self
            .matcher
            .matches(name, attributes, size, modified, folded)
        {
            piece.push_find_data(data);
        }
    }

    /// Hands what `piece` has on to the collector.
    fn hand_on(&self, task: u64, folder: &Arc<[u16]>, piece: &mut Entries, sender: &Sender<Piece>) {
        if piece.is_empty() {
            return;
        }
        // The collector stops listening only once the search is cancelled.
        let _ = sender.send(Piece {
            task,
            folder: folder.clone(),
            entries: std::mem::take(piece),
        });
    }

    /// `folder` couldn't be read: the search fails if it is the one it starts in, else the
    /// folder is skipped.
    fn fail(&self, folder: &[u16], error: &windows::core::Error, reached: Reached) {
        if reached == Reached::Root {
            let folder = PathBuf::from(OsString::from_wide(folder));
            lock(&self.failed).get_or_insert_with(|| list_error(error, &folder));
        } else {
            self.skipped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn is_excluded(&self, folder: &[u16]) -> bool {
        self.excluded
            .iter()
            .any(|excluded| excluded.len() == folder.len() && same_text(excluded, folder))
    }
}

/// Whether an entry is a link: a reparse point that stands for another name (a symbolic link, a
/// junction, a mount point). OneDrive's files and folders are reparse points that aren't.
fn is_link(data: &WIN32_FIND_DATAW) -> bool {
    // With the reparse point attribute, `dwReserved0` is the reparse tag.
    data.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        && data.dwReserved0 & NAME_SURROGATE != 0
}

/// `name` in `folder`.
fn join(folder: &[u16], name: &[u16]) -> Arc<[u16]> {
    let mut path = Vec::with_capacity(folder.len() + 1 + name.len());
    path.extend_from_slice(folder);
    if path.last() != Some(&BACKSLASH) {
        path.push(BACKSLASH);
    }
    path.extend_from_slice(name);
    path.into()
}

/// Whether two paths are the same text as Windows compares names: ordinal, ignoring case.
fn same_text(a: &[u16], b: &[u16]) -> bool {
    // SAFETY: both are valid slices.
    unsafe { CompareStringOrdinal(a, b, true) == CSTR_EQUAL }
}

/// The volume and file ID of `folder` (a wide long path), what a link leads to rather than the
/// link. The folder is opened for its attributes only: nothing is read from it, and a OneDrive
/// folder stays as it is. `None` if it can't be opened.
fn folder_key(folder: &[u16]) -> Option<FolderKey> {
    let path = PathBuf::from(OsString::from_wide(folder));
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)
        .ok()?;
    let handle = HANDLE(file.as_raw_handle());
    let mut id = FILE_ID_INFO::default();
    // SAFETY: `handle` stays open while `file` lives; `id` is a FILE_ID_INFO of the size given.
    let by_id = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&raw mut id).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if by_id.is_ok() {
        return Some((
            id.VolumeSerialNumber,
            u128::from_le_bytes(id.FileId.Identifier),
        ));
    }
    // FAT and some shares have no 128-bit IDs.
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: as above; `info` is the structure the call fills.
    unsafe { GetFileInformationByHandle(handle, &mut info) }.ok()?;
    Some((
        u64::from(info.dwVolumeSerialNumber),
        (u128::from(info.nFileIndexHigh) << 32) | u128::from(info.nFileIndexLow),
    ))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(name: &str, attributes: u32, tag: u32) -> WIN32_FIND_DATAW {
        let mut data = WIN32_FIND_DATAW {
            dwFileAttributes: attributes,
            dwReserved0: tag,
            ..WIN32_FIND_DATAW::default()
        };
        for (slot, unit) in data.cFileName.iter_mut().zip(name.encode_utf16()) {
            *slot = unit;
        }
        data
    }

    #[test]
    fn links_are_name_surrogates_and_cloud_folders_are_not() {
        const FOLDER: u32 = 0x10;
        const REPARSE: u32 = 0x400;
        let junction = found("j", FOLDER | REPARSE, 0xA000_0003);
        let symlink = found("s", FOLDER | REPARSE, 0xA000_000C);
        // OneDrive: IO_REPARSE_TAG_CLOUD_6, and a cloud-only folder's recall attribute.
        let cloud = found("OneDrive", FOLDER | REPARSE | 0x40_0000, 0x9000_601A);
        // A tag without the reparse point attribute is no tag.
        let plain = found("p", FOLDER, 0xA000_0003);
        assert!(is_link(&junction));
        assert!(is_link(&symlink));
        assert!(!is_link(&cloud));
        assert!(!is_link(&plain));
    }

    #[test]
    fn joining_adds_one_separator() {
        let wide = |text: &str| text.encode_utf16().collect::<Vec<_>>();
        let text = |path: Arc<[u16]>| String::from_utf16_lossy(&path);
        assert_eq!(text(join(&wide(r"\\?\C:\"), &wide("ა"))), r"\\?\C:\ა");
        assert_eq!(text(join(&wide(r"\\?\C:\a"), &wide("b"))), r"\\?\C:\a\b");
    }

    #[test]
    fn excluded_folders_compare_ignoring_case() {
        let wide = |text: &str| text.encode_utf16().collect::<Vec<_>>();
        assert!(same_text(
            &wide(r"\\?\C:\Windows"),
            &wide(r"\\?\c:\WINDOWS")
        ));
        assert!(!same_text(
            &wide(r"\\?\C:\Windows"),
            &wide(r"\\?\C:\Windowz")
        ));
    }
}
