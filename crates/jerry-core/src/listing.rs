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
//!
//! A listing stays live through [`Listing::apply`] and [`Listing::apply_listed`], which take what
//! the watcher (`crate::watcher`) saw and return a [`Diff`]. Ids survive changes: a changed or
//! renamed entry keeps its id, a new one gets the next id, and a removed one's id is never reused.

use std::collections::hash_map::{DefaultHasher, Entry};
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, HANDLE, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FIND_FIRST_EX_FLAGS, FIND_FIRST_EX_LARGE_FETCH, FindClose,
    FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, GetDriveTypeW,
    GetFileAttributesW, INVALID_FILE_ATTRIBUTES, WIN32_FIND_DATAW,
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

    /// Copies of entries `ids`, in that order: entry `i` of the copy is entry `ids[i]` here.
    pub fn copy_of(&self, ids: &[u32]) -> Entries {
        let mut copy = Entries::default();
        for &id in ids {
            let id = id as usize;
            copy.push(
                self.name_utf16(id),
                self.sizes[id],
                self.created[id],
                self.modified[id],
                self.attributes[id],
            );
        }
        copy
    }

    /// Gives entry `id` the size, dates and attributes of entry `from` of `other`. Returns
    /// whether any of them changed, and whether the entry stopped or started being a folder.
    fn set_data(&mut self, id: usize, other: &Entries, from: usize) -> (bool, bool) {
        let was_dir = self.is_dir(id);
        let changed = (
            self.sizes[id],
            self.created[id],
            self.modified[id],
            self.attributes[id],
        ) != (
            other.sizes[from],
            other.created[from],
            other.modified[from],
            other.attributes[from],
        );
        self.sizes[id] = other.sizes[from];
        self.created[id] = other.created[from];
        self.modified[id] = other.modified[from];
        self.attributes[id] = other.attributes[from];
        (changed, was_dir != self.is_dir(id))
    }

    /// Gives every entry in `names` its new name, in one pass over all names, so the names stay
    /// back to back.
    pub fn rename(&mut self, names: &HashMap<usize, &[u16]>) {
        if names.is_empty() {
            return;
        }
        let mut units = Vec::with_capacity(self.name_units.len());
        let mut start = 0;
        for (id, end) in self.name_ends.iter_mut().enumerate() {
            match names.get(&id) {
                Some(name) => units.extend_from_slice(name),
                None => units.extend_from_slice(&self.name_units[start..*end]),
            }
            start = *end;
            *end = units.len();
        }
        self.name_units = units;
    }

    fn name_range(&self, id: usize) -> Range<usize> {
        let start = if id == 0 { 0 } else { self.name_ends[id - 1] };
        start..self.name_ends[id]
    }

    pub(crate) fn push_find_data(&mut self, data: &WIN32_FIND_DATAW) {
        let filetime = |time: windows::Win32::Foundation::FILETIME| {
            (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
        };
        self.push(
            find_data_name(data),
            (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow),
            filetime(data.ftCreationTime),
            filetime(data.ftLastWriteTime),
            data.dwFileAttributes,
        );
    }
}

/// One folder's entries and the folder's exact long path. Entries the folder no longer has stay
/// in the columns, marked removed, so every id keeps meaning the same entry.
#[derive(Debug, Clone)]
pub struct Listing {
    folder: PathBuf,
    entries: Entries,
    /// `removed[id]`: entry `id` left the folder. Shorter than the entries when the last ones
    /// weren't removed; empty until something is.
    removed: Vec<bool>,
    removed_count: usize,
    /// The live entries by name, built at the first change.
    names: Option<NameIndex>,
}

/// Same folder, same entries and the same ones removed: the name index is only a cache.
impl PartialEq for Listing {
    fn eq(&self, other: &Self) -> bool {
        self.folder == other.folder
            && self.entries == other.entries
            && (0..self.entries.len()).all(|id| self.is_removed(id) == other.is_removed(id))
    }
}

impl Eq for Listing {}

/// What a folder holds now under the names that changed (`crate::watcher::observe`).
#[derive(Debug, Default, Clone)]
pub struct Observed {
    /// The entries found, each name once, exactly as the folder has them.
    pub present: Entries,
    /// Names that changed and that the folder no longer has, exactly as the watcher saw them.
    pub absent: Vec<Vec<u16>>,
    /// The renames the watcher saw, in order: old name, new name.
    pub renames: Vec<(Vec<u16>, Vec<u16>)>,
}

/// What a change did to a listing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Diff {
    /// Entries whose name, size, dates or attributes changed, by ascending id. A renamed entry
    /// keeps its id.
    pub changed: Vec<u32>,
    /// New entries: these ids, continuing the listing's in order.
    pub added: Range<u32>,
    /// Entries that left the folder, by ascending id. Their ids are never used again.
    pub removed: Vec<u32>,
    /// Whether names changed, or which entries are folders: sort ranks need building again.
    pub reindex: bool,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed.is_empty()
    }
}

impl Listing {
    /// An empty listing of `folder`; batches go in with [`Listing::append`].
    pub fn new(folder: PathBuf) -> Self {
        Self {
            folder,
            entries: Entries::default(),
            removed: Vec::new(),
            removed_count: 0,
            names: None,
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

    /// Every entry the listing has had, removed ones included: ids `0..entries().len()`.
    pub fn entries(&self) -> &Entries {
        &self.entries
    }

    /// How many entries the folder has now.
    pub fn len(&self) -> usize {
        self.entries.len() - self.removed_count
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether entry `id` left the folder.
    pub fn is_removed(&self, id: usize) -> bool {
        self.removed.get(id).copied().unwrap_or(false)
    }

    /// The ids of the entries the folder has now, ascending.
    pub fn live_ids(&self) -> Vec<u32> {
        (0..self.entries.len())
            .filter(|&id| !self.is_removed(id))
            .map(|id| id as u32)
            .collect()
    }

    /// Appends a batch from [`list`]; its entries get the next ids.
    pub fn append(&mut self, batch: &Entries) {
        self.entries.append(batch);
        self.names = None;
    }

    /// The exact path of entry `id`, or `None` if the listing has no such entry or it was
    /// removed.
    pub fn path(&self, id: usize) -> Option<PathBuf> {
        (id < self.entries.len() && !self.is_removed(id))
            .then(|| self.folder.join(self.entries.name(id)))
    }

    /// Applies what the watcher saw under the names that changed: entries found under a name the
    /// listing has are updated, new names are added, names the folder no longer has are removed.
    /// A rename keeps the entry's id, even through several renames in a row.
    pub fn apply(&mut self, observed: &Observed) -> Diff {
        let mut names = self.take_names();
        let present = &observed.present;
        let mut changed = BTreeSet::new();
        let mut reindex = false;
        let mut new: HashMap<&[u16], usize> = HashMap::new();
        for at in 0..present.len() {
            let name = present.name_utf16(at);
            match names.find(&self.entries, name) {
                Some(id) => {
                    let (data, folder) = self.entries.set_data(id as usize, present, at);
                    if data {
                        changed.insert(id);
                        reindex |= folder;
                    }
                }
                None => {
                    new.insert(name, at);
                }
            }
        }

        // Entries whose name is gone, by the name they have now as renames move them along.
        let mut moving: HashMap<&[u16], u32> = observed
            .absent
            .iter()
            .filter_map(|name| Some((name.as_slice(), names.find(&self.entries, name)?)))
            .collect();
        let mut removed = Vec::new();
        for (old, new_name) in &observed.renames {
            let Some(id) = moving.remove(old.as_slice()) else {
                continue;
            };
            // A gone entry already had the new name: it was replaced.
            if let Some(displaced) = moving.insert(new_name, id) {
                removed.push(displaced);
            }
        }
        let mut renamed: HashMap<usize, &[u16]> = HashMap::new();
        for (name, id) in moving {
            match new.remove(name) {
                Some(at) => {
                    names.remove(&self.entries, id);
                    self.entries.set_data(id as usize, present, at);
                    renamed.insert(id as usize, present.name_utf16(at));
                    changed.insert(id);
                }
                None => removed.push(id),
            }
        }
        self.entries.rename(&renamed);
        for &id in renamed.keys() {
            names.insert(&self.entries, id as u32);
        }

        removed.sort_unstable();
        for &id in &removed {
            names.remove(&self.entries, id);
            self.mark_removed(id as usize);
        }

        let first = self.entries.len() as u32;
        let mut added: Vec<usize> = new.into_values().collect();
        added.sort_unstable();
        for at in added {
            self.entries.push(
                present.name_utf16(at),
                present.sizes[at],
                present.created[at],
                present.modified[at],
                present.attributes[at],
            );
            names.insert(&self.entries, self.entries.len() as u32 - 1);
        }
        self.names = Some(names);

        let added = first..self.entries.len() as u32;
        Diff {
            reindex: reindex || !renamed.is_empty() || !added.is_empty(),
            changed: changed.into_iter().collect(),
            added,
            removed,
        }
    }

    /// Brings the listing in line with `fresh`, the whole folder listed again: when single
    /// changes were lost. Entries are matched by name, so a rename shows as a removal and an
    /// addition.
    pub fn apply_listed(&mut self, fresh: &Entries) -> Diff {
        let mut names = self.take_names();
        let mut seen = vec![false; self.entries.len()];
        let mut changed = Vec::new();
        let mut reindex = false;
        let mut new = Vec::new();
        for at in 0..fresh.len() {
            match names.find(&self.entries, fresh.name_utf16(at)) {
                Some(id) => {
                    seen[id as usize] = true;
                    let (data, folder) = self.entries.set_data(id as usize, fresh, at);
                    if data {
                        changed.push(id);
                        reindex |= folder;
                    }
                }
                None => new.push(at),
            }
        }
        changed.sort_unstable();
        let removed: Vec<u32> = (0..seen.len())
            .filter(|&id| !seen[id] && !self.is_removed(id))
            .map(|id| id as u32)
            .collect();
        for &id in &removed {
            names.remove(&self.entries, id);
            self.mark_removed(id as usize);
        }
        let first = self.entries.len() as u32;
        for at in new {
            self.entries.push(
                fresh.name_utf16(at),
                fresh.sizes[at],
                fresh.created[at],
                fresh.modified[at],
                fresh.attributes[at],
            );
            names.insert(&self.entries, self.entries.len() as u32 - 1);
        }
        self.names = Some(names);
        let added = first..self.entries.len() as u32;
        Diff {
            reindex: reindex || !added.is_empty(),
            changed,
            added,
            removed,
        }
    }

    /// The name index, built if this is the first change.
    fn take_names(&mut self) -> NameIndex {
        self.names.take().unwrap_or_else(|| {
            let mut names = NameIndex::default();
            for id in 0..self.entries.len() {
                if !self.is_removed(id) {
                    names.insert(&self.entries, id as u32);
                }
            }
            names
        })
    }

    fn mark_removed(&mut self, id: usize) {
        if self.removed.len() <= id {
            self.removed.resize(id + 1, false);
        }
        if !self.removed[id] {
            self.removed[id] = true;
            self.removed_count += 1;
        }
    }
}

/// Live entries by exact name: one `u64` hash per entry instead of a copy of every name. Names
/// whose hash an earlier name already has (practically never) go to a short list.
#[derive(Debug, Default, Clone)]
struct NameIndex {
    by_hash: HashMap<u64, u32>,
    collided: Vec<u32>,
}

impl NameIndex {
    fn find(&self, entries: &Entries, name: &[u16]) -> Option<u32> {
        let named = |id: &u32| entries.name_utf16(*id as usize) == name;
        self.by_hash
            .get(&name_hash(name))
            .filter(|id| named(id))
            .or_else(|| self.collided.iter().find(|id| named(id)))
            .copied()
    }

    /// Adds entry `id` under its current name.
    fn insert(&mut self, entries: &Entries, id: u32) {
        let hash = name_hash(entries.name_utf16(id as usize));
        match self.by_hash.entry(hash) {
            Entry::Occupied(_) => self.collided.push(id),
            Entry::Vacant(slot) => {
                slot.insert(id);
            }
        }
    }

    /// Drops entry `id`, under the name it has now: call it before the name changes.
    fn remove(&mut self, entries: &Entries, id: u32) {
        let hash = name_hash(entries.name_utf16(id as usize));
        if self.by_hash.get(&hash) != Some(&id) {
            self.collided.retain(|&other| other != id);
            return;
        }
        self.by_hash.remove(&hash);
        let same_hash = self
            .collided
            .iter()
            .position(|&other| name_hash(entries.name_utf16(other as usize)) == hash);
        if let Some(position) = same_hash {
            self.by_hash
                .insert(hash, self.collided.swap_remove(position));
        }
    }
}

fn name_hash(name: &[u16]) -> u64 {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    hasher.finish()
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
    // Cancelled before it starts (its volume is being removed, say): don't open the folder.
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }
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

/// Looks up `name` in `folder` (a long path) and hands every entry that matches it to `found`:
/// the entry itself under its exact name, and entries it matches only by 8.3 short name or by
/// case. Nothing matching is not an error. Blocking IO.
pub(crate) fn find_name(
    folder: &Path,
    name: &[u16],
    mut found: impl FnMut(&WIN32_FIND_DATAW),
) -> Result<(), JerryError> {
    let mut pattern: Vec<u16> = folder.as_os_str().encode_wide().collect();
    if pattern.last() != Some(&u16::from(b'\\')) {
        pattern.push(u16::from(b'\\'));
    }
    pattern.extend_from_slice(name);
    pattern.push(0);
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
            FIND_FIRST_EX_FLAGS(0),
        )
    };
    let handle = match first {
        Ok(handle) => FindHandle(handle),
        Err(error) if WIN32_ERROR::from_error(&error) == Some(ERROR_FILE_NOT_FOUND) => {
            return Ok(());
        }
        Err(error) => return Err(list_error(&error, folder)),
    };
    loop {
        if !is_dot_entry(&data) {
            found(&data);
        }
        // SAFETY: `handle` is an open find handle and `data` a valid WIN32_FIND_DATAW.
        if let Err(error) = unsafe { FindNextFileW(handle.0, &mut data) } {
            if WIN32_ERROR::from_error(&error) == Some(ERROR_NO_MORE_FILES) {
                return Ok(());
            }
            return Err(list_error(&error, folder));
        }
    }
}

/// The exact name in `data`, without its NUL.
pub(crate) fn find_data_name(data: &WIN32_FIND_DATAW) -> &[u16] {
    let len = data
        .cFileName
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(data.cFileName.len());
    &data.cFileName[..len]
}

/// What is wrong with `folder` (a long path), or `None` while it is still a folder there:
/// [`ErrorCode::NotFound`] once it is deleted, [`ErrorCode::DeviceGone`] once its drive is.
pub(crate) fn folder_error(folder: &Path) -> Option<JerryError> {
    let wide: Vec<u16> = folder.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    let attributes = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        let error = windows::core::Error::from_thread();
        return Some(list_error(&error, folder));
    }
    (attributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0)
        .then(|| JerryError::new(ErrorCode::NotFound).at(folder))
}

/// Closes a find handle when dropped.
pub(crate) struct FindHandle(pub(crate) HANDLE);

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
pub(crate) fn list_error(error: &windows::core::Error, folder: &Path) -> JerryError {
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

pub(crate) fn is_dot_entry(data: &WIN32_FIND_DATAW) -> bool {
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

    fn wide(name: &str) -> Vec<u16> {
        name.encode_utf16().collect()
    }

    /// A listing of `names`: entry `i` has size `i`.
    fn listing_of(names: &[&str]) -> Listing {
        let mut listing = Listing::new(PathBuf::from(r"\\?\C:\Work"));
        listing.append(&entries(names));
        listing
    }

    /// What the watcher would see: `present` as (name, size), `absent` names, `renames`.
    fn observed(present: &[(&str, u64)], absent: &[&str], renames: &[(&str, &str)]) -> Observed {
        let mut found = Entries::default();
        for &(name, size) in present {
            found.push(&wide(name), size, 10, 99, 0x20);
        }
        Observed {
            present: found,
            absent: absent.iter().map(|name| wide(name)).collect(),
            renames: renames
                .iter()
                .map(|(old, new)| (wide(old), wide(new)))
                .collect(),
        }
    }

    fn live_names(listing: &Listing) -> Vec<String> {
        listing
            .live_ids()
            .into_iter()
            .map(|id| {
                listing
                    .entries()
                    .name(id as usize)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn changes_update_add_and_remove_by_name() {
        let mut listing = listing_of(&["a.txt", "ნახაზი.dwg", "c.txt"]);

        let diff = listing.apply(&observed(
            &[("ნახაზი.dwg", 7), ("new.txt", 3), ("c.txt", 2)],
            &["a.txt", "never listed.txt"],
            &[],
        ));

        assert_eq!(
            diff,
            Diff {
                changed: vec![1, 2],
                added: 3..4,
                removed: vec![0],
                reindex: true,
            }
        );
        assert_eq!(live_names(&listing), ["ნახაზი.dwg", "c.txt", "new.txt"]);
        assert_eq!(listing.entries().sizes()[1], 7);
        assert_eq!(listing.len(), 3);
        assert!(listing.is_removed(0));
        assert_eq!(listing.path(0), None, "a removed entry has no path");
        assert!(listing.path(3).is_some());

        // Seeing the same again changes nothing.
        let again = listing.apply(&observed(&[("ნახაზი.dwg", 7)], &["a.txt"], &[]));
        assert!(again.is_empty());
    }

    #[test]
    fn data_only_changes_keep_the_sort_ranks() {
        let mut listing = listing_of(&["a.txt", "b.txt"]);
        let diff = listing.apply(&observed(&[("b.txt", 500)], &[], &[]));
        assert_eq!(diff.changed, [1]);
        assert!(!diff.reindex);
    }

    #[test]
    fn a_rename_keeps_the_id_even_through_a_chain() {
        let mut listing = listing_of(&["a.txt", "b.txt"]);

        let diff = listing.apply(&observed(
            &[("გეგმა.pdf", 0)],
            &["a.txt", "middle.txt"],
            &[("a.txt", "middle.txt"), ("middle.txt", "გეგმა.pdf")],
        ));

        assert_eq!(diff.changed, [0]);
        assert!(diff.added.is_empty() && diff.removed.is_empty() && diff.reindex);
        assert_eq!(listing.entries().name(0), "გეგმა.pdf");
        assert_eq!(listing.entries().name(1), "b.txt", "later names stay put");
        // The name index follows the rename.
        let gone = listing.apply(&observed(&[], &["გეგმა.pdf"], &[]));
        assert_eq!(gone.removed, [0]);
    }

    #[test]
    fn a_case_only_rename_keeps_the_id() {
        let mut listing = listing_of(&["plan.dwg"]);
        // Windows finds "Plan.dwg" under the old name too, so the old name is absent exactly.
        let diff = listing.apply(&observed(
            &[("Plan.dwg", 0)],
            &["plan.dwg"],
            &[("plan.dwg", "Plan.dwg")],
        ));
        assert_eq!(diff.changed, [0]);
        assert!(diff.added.is_empty() && diff.removed.is_empty());
        assert_eq!(listing.entries().name(0), "Plan.dwg");
    }

    #[test]
    fn a_rename_onto_an_existing_name_replaces_it() {
        let mut listing = listing_of(&["draft.docx", "final.docx"]);
        let diff = listing.apply(&observed(
            &[("final.docx", 9)],
            &["draft.docx"],
            &[("draft.docx", "final.docx")],
        ));
        assert_eq!(live_names(&listing), ["final.docx"]);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(listing.len(), 1);
    }

    #[test]
    fn a_new_listing_matches_by_name() {
        let mut listing = listing_of(&["a.txt", "b.txt", "c.txt"]);
        let mut fresh = Entries::default();
        fresh.push(&wide("c.txt"), 2, 10, 20 + 2, 0x20);
        fresh.push(&wide("b.txt"), 50, 10, 21, 0x20);
        fresh.push(&wide("d.txt"), 0, 10, 0, 0x20);

        let diff = listing.apply_listed(&fresh);

        assert_eq!(
            diff,
            Diff {
                changed: vec![1],
                added: 3..4,
                removed: vec![0],
                reindex: true,
            }
        );
        assert_eq!(live_names(&listing), ["b.txt", "c.txt", "d.txt"]);
        assert!(listing.apply_listed(&fresh).is_empty());
    }

    #[test]
    fn removed_ids_are_never_reused() {
        let mut listing = listing_of(&["a.txt"]);
        listing.apply(&observed(&[], &["a.txt"], &[]));
        let diff = listing.apply(&observed(&[("a.txt", 0)], &[], &[]));
        assert_eq!(diff.added, 1..2, "the file came back as a new entry");
        assert_eq!(listing.live_ids(), [1]);
    }

    #[test]
    fn names_with_the_same_hash_are_still_told_apart() {
        let entries = entries(&["a", "b", "c"]);
        // Force every name into one hash bucket by indexing them under one key.
        let mut names = NameIndex::default();
        names.by_hash.insert(name_hash(&wide("a")), 0);
        names.collided.extend([1, 2]);
        assert_eq!(names.find(&entries, &wide("c")), Some(2));
        names.remove(&entries, 0);
        assert_eq!(names.find(&entries, &wide("a")), None);
        assert_eq!(names.find(&entries, &wide("b")), Some(1));
    }

    #[test]
    fn copies_and_renames_keep_names_back_to_back() {
        let mut all = entries(&["a.txt", "ნახაზი.dwg", "c"]);
        let copy = all.copy_of(&[2, 0]);
        assert_eq!(copy.name(0), "c");
        assert_eq!(copy.name(1), "a.txt");
        assert_eq!(copy.sizes(), [2, 0]);

        let longer = wide("a much longer name.txt");
        all.rename(&HashMap::from([(0, longer.as_slice())]));
        assert_eq!(all.name(0), "a much longer name.txt");
        assert_eq!(all.name(1), "ნახაზი.dwg");
        assert_eq!(all.name(2), "c");
        assert_eq!(
            all.name_units().len(),
            *all.name_ends().last().expect("ends")
        );
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
