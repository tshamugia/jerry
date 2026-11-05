//! Live listings: one `ReadDirectoryChangesW` watcher per open folder (Tech PRD «Watcher»).
//!
//! [`Watchers`] keeps one watcher per folder, however many listings (tabs) subscribe to it. Each
//! watcher runs on a thread of its own, never on the async runtime. From the first change on it
//! collects the changed names for [`COLLECT`], looks up what the folder holds under them now
//! ([`observe`]) and hands that to every subscriber, which applies it to its own listing
//! ([`Refresh::apply_to`]). When single changes are lost (the notification buffer overflowed) the
//! watcher lists the whole folder again; a folder that sends no notifications (some network
//! shares) is listed again every [`POLL`]. A folder that is deleted, renamed or moved away, or
//! whose drive or share goes away, ends its watcher with [`Event::Gone`].
//!
//! The folder is opened with every share mode and `FILE_FLAG_BACKUP_SEMANTICS`, so Explorer can
//! still rename, move and delete it while Jerry watches.
//!
//! An open folder keeps its volume in use: Windows removes a USB drive ("Safely remove") or locks
//! a volume only once nothing on it is open (#126). When Windows asks, [`Watchers::release`] has
//! every watcher on that volume close its folder; [`Watchers::keep`] opens them again if the
//! volume stays, and [`Watchers::gone`] ends them with [`ErrorCode::DeviceGone`] if it goes.

use std::collections::{HashMap, HashSet};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_IO_INCOMPLETE,
    ERROR_NOT_SUPPORTED, ERROR_NOTIFY_ENUM_DIR, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WIN32_ERROR,
};
use windows::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ACTION, FILE_ACTION_RENAMED_NEW_NAME, FILE_ACTION_RENAMED_OLD_NAME,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE,
    FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GETFINALPATHNAMEBYHANDLE_FLAGS,
    GetFinalPathNameByHandleW, OPEN_EXISTING, ReadDirectoryChangesW,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{
    CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects,
};
use windows::core::PCWSTR;

use crate::error::{ErrorCode, JerryError};
use crate::job::run_job;
use crate::listing::{
    Diff, Entries, Listing, Observed, find_data_name, find_name, folder_error, list, list_error,
};
use crate::path::long_path;
use crate::volume::{self, same_root};

/// How long changes collect, from the first one on, before they go out together.
pub const COLLECT: Duration = Duration::from_millis(100);

/// How often a folder that sends no change notifications is listed again, at the least.
pub const POLL: Duration = Duration::from_secs(5);

/// How often the watcher checks that its folder is still where the listing thinks it is.
const CHECK_PATH: Duration = Duration::from_secs(2);

/// The notification buffer: Windows refuses more than 64 KB for a folder on the network.
pub const BUFFER_BYTES: usize = 64 * 1024;

/// Most names one collection looks up one by one; past that, listing the folder is cheaper.
const MAX_NAMES: usize = 4096;

/// What counts as a change: names, attributes, size and the dates the list shows. Not last
/// access or security.
const CHANGE_FILTER: FILE_NOTIFY_CHANGE = FILE_NOTIFY_CHANGE(
    FILE_NOTIFY_CHANGE_FILE_NAME.0
        | FILE_NOTIFY_CHANGE_DIR_NAME.0
        | FILE_NOTIFY_CHANGE_ATTRIBUTES.0
        | FILE_NOTIFY_CHANGE_SIZE.0
        | FILE_NOTIFY_CHANGE_LAST_WRITE.0
        | FILE_NOTIFY_CHANGE_CREATION.0,
);

/// The names a folder's changes touched, each once in the order first seen, and its renames.
/// Lost when single names can't be trusted (an overflow, too many names): then only listing
/// the whole folder again helps.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Changes {
    names: Vec<Vec<u16>>,
    seen: HashSet<Vec<u16>>,
    renames: Vec<(Vec<u16>, Vec<u16>)>,
    lost: bool,
}

impl Changes {
    /// Something changed under `name`: added, removed or modified.
    pub fn touch(&mut self, name: &[u16]) {
        if self.lost || self.seen.contains(name) {
            return;
        }
        if self.names.len() == MAX_NAMES {
            self.lose();
            return;
        }
        self.seen.insert(name.to_vec());
        self.names.push(name.to_vec());
    }

    /// `old` was renamed to `new`.
    pub fn rename(&mut self, old: &[u16], new: &[u16]) {
        self.touch(old);
        self.touch(new);
        if !self.lost {
            self.renames.push((old.to_vec(), new.to_vec()));
        }
    }

    /// Single changes were lost: the folder has to be listed again.
    pub fn lose(&mut self) {
        *self = Self {
            lost: true,
            ..Self::default()
        };
    }

    /// Adds `other`'s changes after these.
    pub fn merge(&mut self, other: &Changes) {
        if other.lost {
            self.lose();
            return;
        }
        for name in &other.names {
            self.touch(name);
        }
        if !self.lost {
            self.renames.extend(other.renames.iter().cloned());
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.lost && self.names.is_empty()
    }

    pub fn is_lost(&self) -> bool {
        self.lost
    }

    /// The changed names, in the order first seen.
    pub fn names(&self) -> &[Vec<u16>] {
        &self.names
    }

    /// The renames, in order: old name, new name.
    pub fn renames(&self) -> &[(Vec<u16>, Vec<u16>)] {
        &self.renames
    }
}

/// Looks up what `folder` holds now under every name in `changes`. An entry found by its 8.3
/// short name or another case is reported under its exact name; a name the folder doesn't have
/// exactly is absent. Setting `cancel` stops it with [`ErrorCode::Cancelled`]. Blocking IO: one
/// directory lookup per name.
pub fn observe(
    folder: &Path,
    changes: &Changes,
    cancel: &AtomicBool,
) -> Result<Observed, JerryError> {
    let folder = long_path(folder)?;
    let mut observed = Observed {
        renames: changes.renames.clone(),
        ..Observed::default()
    };
    let mut found: HashSet<Vec<u16>> = HashSet::new();
    for name in &changes.names {
        if cancel.load(Ordering::Relaxed) {
            return Err(JerryError::new(ErrorCode::Cancelled).at(&folder));
        }
        let mut exact = false;
        find_name(&folder, name, |data| {
            let found_name = find_data_name(data);
            exact |= found_name == name.as_slice();
            if found.insert(found_name.to_vec()) {
                observed.present.push_find_data(data);
            }
        })?;
        if !exact {
            observed.absent.push(name.clone());
        }
    }
    Ok(observed)
}

/// What a folder holds now after changes.
#[derive(Debug, Clone)]
pub enum Refresh {
    /// What it holds under the names that changed.
    Names(Observed),
    /// All of it, listed again.
    Listed(Entries),
}

impl Refresh {
    /// Applies this to `listing` and says what changed.
    pub fn apply_to(&self, listing: &mut Listing) -> Diff {
        match self {
            Self::Names(observed) => listing.apply(observed),
            Self::Listed(entries) => listing.apply_listed(entries),
        }
    }
}

/// Finds out what `folder` holds now after `changes`: the changed names one by one, or the whole
/// folder when single changes were lost, a lookup failed, or a name that went away looks like an
/// 8.3 short name (the watcher can report one; which entry it meant is gone with it). Setting
/// `cancel` stops listing the folder with [`ErrorCode::Cancelled`]. Blocking IO.
pub fn refresh(
    folder: &Path,
    changes: &Changes,
    cancel: &AtomicBool,
) -> Result<Refresh, JerryError> {
    if !changes.is_lost() {
        match observe(folder, changes, cancel) {
            Ok(observed) if !observed.absent.iter().any(|name| is_short_name(name)) => {
                return Ok(Refresh::Names(observed));
            }
            Err(error) if error.code == ErrorCode::Cancelled => return Err(error),
            Ok(_) => tracing::debug!("a short name went away; listing the folder again"),
            Err(error) => {
                tracing::debug!(code = ?error.code, "changed names didn't resolve; listing the folder again");
            }
        }
    }
    let mut all = Entries::default();
    list(folder, cancel, |batch| all.append(batch))?;
    Ok(Refresh::Listed(all))
}

/// Whether `name` has the form of a generated 8.3 short name: `PROGRA~1`, `NAHAZI~2.DWG`.
fn is_short_name(name: &[u16]) -> bool {
    let (base, extension) = match name.iter().rposition(|&unit| unit == u16::from(b'.')) {
        Some(dot) => (&name[..dot], &name[dot + 1..]),
        None => (name, &[][..]),
    };
    let Some(tilde) = base.iter().position(|&unit| unit == u16::from(b'~')) else {
        return false;
    };
    let digits = &base[tilde + 1..];
    base.len() <= 8
        && extension.len() <= 3
        && !digits.is_empty()
        && digits
            .iter()
            .all(|&unit| (u16::from(b'0')..=u16::from(b'9')).contains(&unit))
}

/// What a watcher tells its subscribers.
#[derive(Debug, Clone, Copy)]
pub enum Event<'a> {
    /// The folder changed: `changes` is what the watcher saw, `now` what the folder holds now.
    Changed {
        changes: &'a Changes,
        now: &'a Refresh,
    },
    /// The folder is gone: deleted, renamed or moved away ([`ErrorCode::NotFound`]), or its
    /// drive or share went away ([`ErrorCode::DeviceGone`]). No event follows.
    Gone(&'a JerryError),
}

type Subscriber = Arc<dyn Fn(Event<'_>) + Send + Sync>;

/// The running watchers, one per folder. Clones share them.
#[derive(Clone)]
pub struct Watchers(Arc<Registry>);

struct Registry {
    folders: Mutex<Folders>,
    buffer_bytes: usize,
}

#[derive(Default)]
struct Folders {
    watches: HashMap<PathBuf, Arc<Watch>>,
    /// The roots of volumes Windows is removing or locking: no folder on them is opened until
    /// [`Watchers::keep`] or [`Watchers::gone`].
    released: Vec<PathBuf>,
}

impl Folders {
    fn is_released(&self, root: &Path) -> bool {
        self.released
            .iter()
            .any(|released| same_root(released, root))
    }

    /// The watchers of folders on the volume at `root`.
    fn on(&self, root: &Path) -> Vec<Arc<Watch>> {
        self.watches
            .values()
            .filter(|watch| watch.is_on(root))
            .cloned()
            .collect()
    }
}

impl Default for Watchers {
    fn default() -> Self {
        Self::with_buffer(BUFFER_BYTES)
    }
}

impl Watchers {
    /// Watchers whose notification buffer has `bytes` bytes. Tests use a small one to make it
    /// overflow; everything else uses [`Watchers::default`].
    pub fn with_buffer(bytes: usize) -> Self {
        Self(Arc::new(Registry {
            folders: Mutex::new(Folders::default()),
            buffer_bytes: bytes,
        }))
    }

    /// Calls `subscriber` on the watcher's thread with every change to `folder` from now on, and
    /// once with [`Event::Gone`] if the folder goes away. A folder already watched shares its
    /// watcher. Returns once changes are being recorded, so nothing after it is missed. Fails
    /// when the folder can't be opened, with the listing's error codes, and with
    /// [`ErrorCode::DeviceGone`] while its volume is released ([`Watchers::release`]).
    /// Blocking IO.
    pub fn subscribe(
        &self,
        folder: &Path,
        subscriber: impl Fn(Event<'_>) + Send + Sync + 'static,
    ) -> Result<Subscription, JerryError> {
        let folder = long_path(folder)?;
        // Blocking IO, so before the lock. Without a root the folder is watched all the same;
        // only Windows' notices about its volume don't reach it.
        let root = volume::root(&folder).ok();
        let (watch, id) = {
            let mut folders = self.folders();
            if root
                .as_deref()
                .is_some_and(|root| folders.is_released(root))
            {
                return Err(JerryError::new(ErrorCode::DeviceGone).at(&folder));
            }
            let watch = match folders.watches.get(&folder) {
                Some(watch) => watch.clone(),
                None => {
                    let watch = Arc::new(Watch::new(folder.clone(), root)?);
                    self.start(&watch)?;
                    folders.watches.insert(folder, watch.clone());
                    watch
                }
            };
            let id = watch.add(Arc::new(subscriber));
            (watch, id)
        };
        let subscription = Subscription {
            watchers: self.clone(),
            watch,
            id,
        };
        subscription.watch.wait_started()?;
        Ok(subscription)
    }

    /// How many folders are watched.
    pub fn watched(&self) -> usize {
        self.folders().watches.len()
    }

    /// Lets go of every folder watched on the volume at `root` ([`volume::root`]): Windows wants
    /// to remove or lock the volume, and does so only once nothing on it is open. Each watcher
    /// closes its folder and waits to hear whether the volume stays ([`Watchers::keep`]) or goes
    /// ([`Watchers::gone`]). Until then no folder on it is opened, and subscribing to one fails
    /// with [`ErrorCode::DeviceGone`].
    ///
    /// Returns `true` once every one of them has closed its folder, `false` if some haven't by
    /// `deadline`. It only signals the watchers' threads and waits for them, so the main thread
    /// can call it from a device notification.
    pub fn release(&self, root: &Path, deadline: Instant) -> bool {
        let watches = {
            let mut folders = self.folders();
            if !folders.is_released(root) {
                folders.released.push(root.to_path_buf());
            }
            folders.on(root)
        };
        for watch in &watches {
            watch.release();
        }
        watches.iter().all(|watch| watch.wait_released(deadline))
    }

    /// The volume at `root` stays after all: its removal or lock failed, or the lock is over.
    /// The watchers [`Watchers::release`] closed open their folders again and list them anew, as
    /// changes made meanwhile weren't recorded. Doesn't wait.
    pub fn keep(&self, root: &Path) {
        for watch in self.unrelease(root) {
            watch.order(Hold::Reopen);
        }
    }

    /// The volume at `root` is gone: its watchers end with [`Event::Gone`] and
    /// [`ErrorCode::DeviceGone`]. Doesn't wait.
    pub fn gone(&self, root: &Path) {
        for watch in self.unrelease(root) {
            watch.order(Hold::Gone);
        }
    }

    /// Takes `root` off the released volumes; returns its watchers.
    fn unrelease(&self, root: &Path) -> Vec<Arc<Watch>> {
        let mut folders = self.folders();
        folders
            .released
            .retain(|released| !same_root(released, root));
        folders.on(root)
    }

    fn start(&self, watch: &Arc<Watch>) -> Result<(), JerryError> {
        let thread_watch = watch.clone();
        let registry = Arc::downgrade(&self.0);
        let buffer_bytes = self.0.buffer_bytes;
        std::thread::Builder::new()
            .name("watcher".into())
            .spawn(move || {
                let watch = thread_watch;
                // A panic fails only this folder's watcher; its listings just stop being live.
                let _ = run_job("watcher", || {
                    watch_folder(&watch, &registry, buffer_bytes);
                    Ok(())
                });
                watch.set_started(Err(JerryError::internal()));
                watch.end();
                forget(&registry, &watch);
            })
            .map(drop)
            .map_err(|error| {
                tracing::error!(%error, "watcher thread didn't start");
                JerryError::internal()
            })
    }

    fn folders(&self) -> MutexGuard<'_, Folders> {
        self.0
            .folders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Takes `watch` off the running watchers, if it is still the one for its folder.
fn forget(registry: &Weak<Registry>, watch: &Arc<Watch>) {
    let Some(registry) = registry.upgrade() else {
        return;
    };
    let mut folders = registry
        .folders
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if folders
        .watches
        .get(&watch.folder)
        .is_some_and(|running| Arc::ptr_eq(running, watch))
    {
        folders.watches.remove(&watch.folder);
    }
}

/// A listing's interest in its folder's changes. Dropping it unsubscribes: no call starts after
/// that, and the folder's watcher stops with its last subscription.
pub struct Subscription {
    watchers: Watchers,
    watch: Arc<Watch>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut folders = self.watchers.folders();
        if self.watch.remove(self.id) == 0 {
            self.watch.stop();
            if folders
                .watches
                .get(&self.watch.folder)
                .is_some_and(|running| Arc::ptr_eq(running, &self.watch))
            {
                folders.watches.remove(&self.watch.folder);
            }
        }
    }
}

/// Where a watcher stands with its volume (#126). [`Watchers::release`] asks for
/// [`Hold::Release`] and the thread answers [`Hold::Released`] once its folder is closed;
/// [`Watchers::keep`] and [`Watchers::gone`] then say what comes next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// The folder is open, or about to be.
    Open,
    /// Windows wants the volume: close the folder.
    Release,
    /// The folder is closed until the volume stays or goes.
    Released,
    /// The volume stays: open the folder again.
    Reopen,
    /// The volume is gone.
    Gone,
    /// The thread is done, its folder closed for good.
    Ended,
}

/// What a watcher's thread does about its volume when it wakes.
enum Order {
    /// Nothing.
    Stay,
    /// Close the folder and wait for [`Watch::wait_for_volume`].
    Release,
    /// The volume was released and kept before the folder was closed: reads were interrupted,
    /// so list the folder again.
    Resume,
    /// End with [`ErrorCode::DeviceGone`].
    Gone,
}

/// One folder's watcher, shared by its subscriptions and its thread.
struct Watch {
    folder: PathBuf,
    /// The root of the volume the folder is on, if Windows could say.
    root: Option<PathBuf>,
    subscribers: Mutex<Vec<(u64, Subscriber)>>,
    next_id: AtomicU64,
    /// Wakes the thread: to stop, or to follow its volume ([`Watch::hold`]). Manual reset: the
    /// thread resets it when it wakes, before it looks at what changed.
    wake: Signal,
    stopped: AtomicBool,
    /// Stops the folder read the thread is doing (a refresh): set to stop it or release it.
    interrupt: AtomicBool,
    hold: Mutex<Hold>,
    hold_changed: Condvar,
    started: Mutex<Option<Result<(), JerryError>>>,
    started_changed: Condvar,
}

impl Watch {
    fn new(folder: PathBuf, root: Option<PathBuf>) -> Result<Self, JerryError> {
        Ok(Self {
            folder,
            root,
            subscribers: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(0),
            wake: Signal::new()?,
            stopped: AtomicBool::new(false),
            interrupt: AtomicBool::new(false),
            hold: Mutex::new(Hold::Open),
            hold_changed: Condvar::new(),
            started: Mutex::new(None),
            started_changed: Condvar::new(),
        })
    }

    fn add(&self, subscriber: Subscriber) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.subscribers().push((id, subscriber));
        id
    }

    /// Removes subscriber `id`; returns how many are left.
    fn remove(&self, id: u64) -> usize {
        let mut subscribers = self.subscribers();
        subscribers.retain(|(other, _)| *other != id);
        subscribers.len()
    }

    fn has_subscribers(&self) -> bool {
        !self.subscribers().is_empty()
    }

    /// Calls every subscriber with `event`, outside the lock, each as its own job.
    fn deliver(&self, event: Event<'_>) {
        let subscribers: Vec<Subscriber> = self
            .subscribers()
            .iter()
            .map(|(_, subscriber)| subscriber.clone())
            .collect();
        for subscriber in subscribers {
            let _ = run_job("watcher subscriber", || {
                subscriber(event);
                Ok(())
            });
        }
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.interrupt.store(true, Ordering::Relaxed);
        self.wake.set();
        // A thread waiting for its volume waits on `hold_changed`.
        let _hold = self.hold();
        self.hold_changed.notify_all();
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    fn is_on(&self, root: &Path) -> bool {
        self.root
            .as_deref()
            .is_some_and(|mine| same_root(mine, root))
    }

    /// Asks the thread to close the folder, interrupting what it reads.
    fn release(&self) {
        let mut hold = self.hold();
        if matches!(*hold, Hold::Open | Hold::Reopen) {
            *hold = Hold::Release;
        }
        self.interrupt.store(true, Ordering::Relaxed);
        drop(hold);
        self.wake.set();
    }

    /// Tells the thread what became of its released volume: [`Hold::Reopen`] or [`Hold::Gone`].
    fn order(&self, order: Hold) {
        let mut hold = self.hold();
        match (*hold, order) {
            // Done already, or never released: nothing to reopen.
            (Hold::Ended, _) | (Hold::Open, Hold::Reopen) => return,
            _ => *hold = order,
        }
        self.hold_changed.notify_all();
        drop(hold);
        self.wake.set();
    }

    /// Waits until the thread has closed the folder, or until `deadline`; whether it has.
    fn wait_released(&self, deadline: Instant) -> bool {
        let mut hold = self.hold();
        while *hold == Hold::Release {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            hold = self
                .hold_changed
                .wait_timeout(hold, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    /// The thread's side: what its volume asks of it now. Taking a [`Hold::Reopen`] here means
    /// the folder was never closed.
    fn volume_order(&self) -> Order {
        let mut hold = self.hold();
        match *hold {
            Hold::Release => Order::Release,
            Hold::Reopen => {
                *hold = Hold::Open;
                self.resume();
                Order::Resume
            }
            Hold::Gone => Order::Gone,
            Hold::Open | Hold::Released | Hold::Ended => Order::Stay,
        }
    }

    /// The thread's side, once its folder is closed: says so to [`Watch::wait_released`] and
    /// waits for the volume to stay ([`Order::Resume`]) or go ([`Order::Gone`]). `None` once the
    /// watch stops.
    fn wait_for_volume(&self) -> Option<Order> {
        let mut hold = self.hold();
        if *hold == Hold::Release {
            *hold = Hold::Released;
            self.hold_changed.notify_all();
        }
        loop {
            if self.is_stopped() {
                return None;
            }
            match *hold {
                Hold::Reopen => {
                    *hold = Hold::Open;
                    self.resume();
                    return Some(Order::Resume);
                }
                Hold::Gone => return Some(Order::Gone),
                Hold::Open | Hold::Release | Hold::Released | Hold::Ended => {
                    hold = self
                        .hold_changed
                        .wait(hold)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            }
        }
    }

    /// Reads may run again, unless the watch is stopping.
    fn resume(&self) {
        if !self.is_stopped() {
            self.interrupt.store(false, Ordering::Relaxed);
        }
    }

    /// The thread is done and its folder closed: a release waiting for it is over.
    fn end(&self) {
        let mut hold = self.hold();
        *hold = Hold::Ended;
        self.hold_changed.notify_all();
    }

    /// Records how starting went; only the first call counts.
    fn set_started(&self, result: Result<(), JerryError>) {
        let mut started = self.started.lock().unwrap_or_else(PoisonError::into_inner);
        if started.is_none() {
            *started = Some(result);
            self.started_changed.notify_all();
        }
    }

    fn wait_started(&self) -> Result<(), JerryError> {
        let mut started = self.started.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(result) = &*started {
                return result.clone();
            }
            started = self
                .started_changed
                .wait(started)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn subscribers(&self) -> MutexGuard<'_, Vec<(u64, Subscriber)>> {
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn hold(&self) -> MutexGuard<'_, Hold> {
        self.hold.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The watcher thread: watches until the last subscription goes or the folder does.
fn watch_folder(watch: &Arc<Watch>, registry: &Weak<Registry>, buffer_bytes: usize) {
    let mut source = match Source::open(&watch.folder, buffer_bytes) {
        Ok(source) => source,
        Err(error) => {
            watch.set_started(Err(error));
            return;
        }
    };
    watch.set_started(Ok(()));
    let gone = watch_changes(watch, &mut source, buffer_bytes);
    // Close the folder before anyone hears it's gone: a folder being deleted goes once no handle
    // is open on it.
    drop(source);
    watch.end();
    forget(registry, watch);
    if let Some(error) = gone {
        tracing::info!(code = ?error.code, "watched folder is gone");
        watch.deliver(Event::Gone(&error));
    }
}

/// Collects and delivers changes until the watch stops (`None`) or the folder is gone.
fn watch_changes(watch: &Watch, source: &mut Source, buffer_bytes: usize) -> Option<JerryError> {
    let folder = &watch.folder;
    let mut changes = Changes::default();
    let mut flush_at: Option<Instant> = None;
    let mut poll_at: Option<Instant> = None;
    let mut check_at = Instant::now() + CHECK_PATH;
    let mut reopened: Option<Instant> = None;
    loop {
        if matches!(source, Source::Poll) && poll_at.is_none() {
            poll_at = Some(Instant::now() + POLL);
        }
        let deadline = [flush_at, poll_at, Some(check_at)]
            .into_iter()
            .flatten()
            .min();
        let timeout = deadline.map_or(INFINITE, |deadline| {
            let left = deadline.saturating_duration_since(Instant::now());
            u32::try_from(left.as_millis() + 1).unwrap_or(INFINITE - 1)
        });
        let mut handles = vec![watch.wake.0];
        handles.extend(source.event());
        // SAFETY: every handle is a live event: the watch's wake signal and the source's.
        let woke = unsafe { WaitForMultipleObjects(&handles, false, timeout) };
        if woke == WAIT_OBJECT_0 {
            // Reset before looking, so a wake that comes meanwhile isn't lost.
            watch.wake.reset();
        }
        if watch.is_stopped() {
            return None;
        }
        if woke == WAIT_FAILED {
            let error = windows::core::Error::from_thread();
            tracing::error!(%error, "watcher wait failed");
            return Some(JerryError::internal());
        }
        match watch.volume_order() {
            Order::Stay => {}
            Order::Release => {
                // Windows removes or locks the volume only once nothing on it is open.
                *source = Source::Closed;
                tracing::info!("watched folder closed: its volume is being removed or locked");
                match watch.wait_for_volume() {
                    None => return None,
                    Some(Order::Resume) => match Source::open(folder, buffer_bytes) {
                        Ok(open) => *source = open,
                        Err(error) => return Some(error),
                    },
                    Some(_) => return Some(JerryError::new(ErrorCode::DeviceGone).at(folder)),
                }
                tracing::info!("watched folder open again: its volume stays");
                // Changes made meanwhile weren't recorded: list the folder again right away.
                changes.lose();
                flush_at = Some(Instant::now());
                poll_at = None;
                check_at = Instant::now() + CHECK_PATH;
                continue;
            }
            Order::Resume => {
                // A refresh may have been interrupted, and what it was looking up is lost.
                changes.lose();
                flush_at = Some(Instant::now());
            }
            Order::Gone => return Some(JerryError::new(ErrorCode::DeviceGone).at(folder)),
        }
        if woke.0 == WAIT_OBJECT_0.0 + 1
            && let Err(error) = source.collect(&mut changes)
        {
            tracing::debug!(%error, "change notifications stopped");
            // Close the folder first, then see whether it is still there.
            *source = Source::Poll;
            if let Some(gone) = folder_error(folder) {
                return Some(gone);
            }
            let again = reopened.is_some_and(|at| at.elapsed() < Duration::from_secs(1));
            if !again {
                match Source::open(folder, buffer_bytes) {
                    Ok(open) => *source = open,
                    Err(error) => return Some(error),
                }
                reopened = Some(Instant::now());
            }
            // Changes made meanwhile weren't recorded.
            changes.lose();
        }

        let now = Instant::now();
        if !changes.is_empty() && flush_at.is_none() {
            flush_at = Some(now + COLLECT);
        }
        let poll_due = poll_at.is_some_and(|at| at <= now);
        if poll_due || flush_at.is_some_and(|at| at <= now) {
            if poll_due {
                changes.lose();
            }
            flush_at = None;
            let started = Instant::now();
            if let Err(error) = flush(watch, &mut changes) {
                return Some(error);
            }
            if poll_due {
                // A folder that takes long to list is listed less often.
                poll_at = Some(Instant::now() + POLL.max(started.elapsed() * 4));
            }
        }
        if check_at <= now {
            check_at = now + CHECK_PATH;
            if source.moved() {
                return Some(JerryError::new(ErrorCode::NotFound).at(folder));
            }
        }
    }
}

/// Finds out what the folder holds now after `changes` and tells the subscribers.
fn flush(watch: &Watch, changes: &mut Changes) -> Result<(), JerryError> {
    let changes = std::mem::take(changes);
    if changes.is_empty() || !watch.has_subscribers() {
        return Ok(());
    }
    let started = Instant::now();
    match refresh(&watch.folder, &changes, &watch.interrupt) {
        Ok(now) => {
            tracing::debug!(
                names = changes.names().len(),
                listed = changes.is_lost(),
                ms = started.elapsed().as_secs_f64() * 1000.0,
                "folder changes looked up"
            );
            watch.deliver(Event::Changed {
                changes: &changes,
                now: &now,
            });
            Ok(())
        }
        Err(error) if error.code == ErrorCode::Cancelled => Ok(()),
        Err(error) => Err(error),
    }
}

/// Where a watcher's changes come from.
enum Source {
    /// Notifications from the open folder, and its final path when it was opened.
    Notify { dir: Dir, path: Option<Vec<u16>> },
    /// None: the folder is listed again every [`POLL`].
    Poll,
    /// None for now: the folder is closed while its volume is released.
    Closed,
}

impl Source {
    fn open(folder: &Path, buffer_bytes: usize) -> Result<Self, JerryError> {
        let mut dir = Dir::open(folder, buffer_bytes)?;
        match dir.read() {
            Ok(()) => {
                let path = dir.final_path();
                Ok(Self::Notify { dir, path })
            }
            Err(error) if unsupported(&error) => {
                tracing::info!("folder sends no change notifications; polling it");
                Ok(Self::Poll)
            }
            Err(error) => Err(list_error(&error, folder)),
        }
    }

    /// The event a finished read signals.
    fn event(&self) -> Option<HANDLE> {
        match self {
            Self::Notify { dir, .. } => Some(dir.signal.0),
            Self::Poll | Self::Closed => None,
        }
    }

    /// Adds what the finished read reported to `changes` and starts the next read.
    fn collect(&mut self, changes: &mut Changes) -> windows::core::Result<()> {
        let Self::Notify { dir, .. } = self else {
            return Ok(());
        };
        match dir.finish() {
            Ok(None) => return Ok(()),
            // Windows had more changes than the buffer holds and dropped them.
            Ok(Some(0)) => changes.lose(),
            Ok(Some(len)) => {
                parse_records(&dir.buffer, len as usize, &mut dir.renamed_from, changes);
            }
            Err(error) if is(&error, ERROR_NOTIFY_ENUM_DIR) => changes.lose(),
            Err(error) => return Err(error),
        }
        dir.read()
    }

    /// Whether the folder is no longer at the path it had when it was opened: renamed or moved
    /// away, or one of the folders above it was.
    fn moved(&self) -> bool {
        let Self::Notify {
            dir,
            path: Some(opened),
        } = self
        else {
            return false;
        };
        dir.final_path().is_some_and(|now| {
            // SAFETY: both are valid slices.
            unsafe { CompareStringOrdinal(opened, &now, true) != CSTR_EQUAL }
        })
    }
}

fn is(error: &windows::core::Error, code: WIN32_ERROR) -> bool {
    WIN32_ERROR::from_error(error) == Some(code)
}

/// Whether `error` means the folder can't send change notifications.
fn unsupported(error: &windows::core::Error) -> bool {
    [
        ERROR_INVALID_FUNCTION,
        ERROR_NOT_SUPPORTED,
        ERROR_INVALID_PARAMETER,
    ]
    .iter()
    .any(|&code| is(error, code))
}

/// A watched folder: its handle, opened for overlapped reads of its changes, and the buffer and
/// `OVERLAPPED` a read fills. Both are heap allocations that stay put while a read is pending.
struct Dir {
    handle: HANDLE,
    signal: Signal,
    overlapped: Box<OVERLAPPED>,
    /// `u32`s: Windows wants the buffer DWORD-aligned.
    buffer: Box<[u32]>,
    reading: bool,
    /// The old name of a rename whose new name is still to come.
    renamed_from: Option<Vec<u16>>,
}

impl Dir {
    fn open(folder: &Path, buffer_bytes: usize) -> Result<Self, JerryError> {
        let signal = Signal::new()?;
        let wide: Vec<u16> = folder.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call; there are no security
        // attributes and no template. Every share mode, so others can still rename, move and
        // delete the folder.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .map_err(|error| list_error(&error, folder))?;
        let overlapped = Box::new(OVERLAPPED {
            hEvent: signal.0,
            ..OVERLAPPED::default()
        });
        Ok(Self {
            handle,
            signal,
            overlapped,
            buffer: vec![0; buffer_bytes.div_ceil(4)].into_boxed_slice(),
            reading: false,
            renamed_from: None,
        })
    }

    /// Starts reading the next changes; `signal` is set when they are in.
    fn read(&mut self) -> windows::core::Result<()> {
        self.signal.reset();
        let len = u32::try_from(self.buffer.len() * 4).unwrap_or(u32::MAX);
        // SAFETY: the handle was opened for overlapped IO. The buffer and OVERLAPPED belong to
        // `self` on the heap and stay put until the read completes or `drop` cancels it and
        // waits.
        unsafe {
            ReadDirectoryChangesW(
                self.handle,
                self.buffer.as_mut_ptr().cast(),
                len,
                false,
                CHANGE_FILTER,
                None,
                Some(&raw mut *self.overlapped),
                None,
            )
        }?;
        self.reading = true;
        Ok(())
    }

    /// The bytes the finished read wrote, or `None` if it is still running.
    fn finish(&mut self) -> windows::core::Result<Option<u32>> {
        let mut bytes = 0;
        // SAFETY: the OVERLAPPED belongs to the read started on this handle; not waiting.
        let result =
            unsafe { GetOverlappedResult(self.handle, &*self.overlapped, &mut bytes, false) };
        match result {
            Ok(()) => {
                self.reading = false;
                Ok(Some(bytes))
            }
            Err(error) if is(&error, ERROR_IO_INCOMPLETE) => Ok(None),
            Err(error) => {
                self.reading = false;
                Err(error)
            }
        }
    }

    /// The folder's path as Windows has it now, or `None` if it can't say.
    fn final_path(&self) -> Option<Vec<u16>> {
        let mut path = vec![0u16; 512];
        loop {
            // SAFETY: `path` is a valid buffer of its length; the handle is open.
            let len = unsafe {
                GetFinalPathNameByHandleW(self.handle, &mut path, GETFINALPATHNAMEBYHANDLE_FLAGS(0))
            } as usize;
            if len == 0 {
                return None;
            }
            if len < path.len() {
                path.truncate(len);
                return Some(path);
            }
            // Too small: `len` is the size needed, with the NUL.
            path.resize(len + 1, 0);
        }
    }
}

/// Adds the `FILE_NOTIFY_INFORMATION` records in the first `len` bytes of `words` to
/// `changes`. `renamed_from` carries a rename's old name over to its new name, also from one read
/// to the next. Records that don't add up lose the changes rather than guess.
fn parse_records(
    words: &[u32],
    len: usize,
    renamed_from: &mut Option<Vec<u16>>,
    changes: &mut Changes,
) {
    let len = len.min(words.len() * 4);
    let mut at = 0;
    loop {
        let word = at / 4;
        let (Some(&next), Some(&action), Some(&name_bytes)) =
            (words.get(word), words.get(word + 1), words.get(word + 2))
        else {
            changes.lose();
            return;
        };
        if at + 12 + name_bytes as usize > len {
            changes.lose();
            return;
        }
        // The name starts on a word, little-endian: a word's low half is the earlier unit.
        let name: Vec<u16> = (0..name_bytes as usize / 2)
            .map(|unit| {
                let pair = words[word + 3 + unit / 2];
                if unit % 2 == 0 {
                    pair as u16
                } else {
                    (pair >> 16) as u16
                }
            })
            .collect();
        match FILE_ACTION(action) {
            FILE_ACTION_RENAMED_OLD_NAME => {
                changes.touch(&name);
                *renamed_from = Some(name);
            }
            FILE_ACTION_RENAMED_NEW_NAME => match renamed_from.take() {
                Some(old) => changes.rename(&old, &name),
                None => changes.touch(&name),
            },
            _ => changes.touch(&name),
        }
        if next == 0 {
            return;
        }
        at += next as usize;
        if at % 4 != 0 || at >= len {
            changes.lose();
            return;
        }
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        if self.reading {
            let mut bytes = 0;
            // SAFETY: the handle and the OVERLAPPED of its pending read are live. Waiting for the
            // cancelled read to finish means Windows no longer writes to the buffer when it is
            // freed.
            unsafe {
                let _ = CancelIoEx(self.handle, Some(&*self.overlapped));
                let _ = GetOverlappedResult(self.handle, &*self.overlapped, &mut bytes, true);
            }
        }
        // SAFETY: the handle came from CreateFileW and is closed only here.
        if let Err(error) = unsafe { CloseHandle(self.handle) } {
            tracing::warn!(%error, "watched folder didn't close");
        }
    }
}

/// A manual-reset Win32 event, closed when dropped.
struct Signal(HANDLE);

// SAFETY: an event is a kernel object; any thread may wait on, set, reset or close its handle.
unsafe impl Send for Signal {}
// SAFETY: as above; the handle value itself never changes.
unsafe impl Sync for Signal {}

impl Signal {
    fn new() -> Result<Self, JerryError> {
        // SAFETY: no security attributes and no name; manual reset, not set.
        unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
            .map(Self)
            .map_err(|error| JerryError::new(ErrorCode::from_hresult(error.code())))
    }

    fn set(&self) {
        // SAFETY: the handle is a live event.
        if let Err(error) = unsafe { SetEvent(self.0) } {
            tracing::warn!(%error, "SetEvent failed");
        }
    }

    fn reset(&self) {
        // SAFETY: the handle is a live event.
        if let Err(error) = unsafe { ResetEvent(self.0) } {
            tracing::warn!(%error, "ResetEvent failed");
        }
    }
}

impl Drop for Signal {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateEventW and is closed only here.
        if let Err(error) = unsafe { CloseHandle(self.0) } {
            tracing::warn!(%error, "event didn't close");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn changes_keep_each_name_once_in_order() {
        let mut changes = Changes::default();
        changes.touch(&wide("b.txt"));
        changes.touch(&wide("ნახაზი.dwg"));
        changes.touch(&wide("b.txt"));
        changes.rename(&wide("ნახაზი.dwg"), &wide("გეგმა.dwg"));
        assert_eq!(
            changes.names(),
            [wide("b.txt"), wide("ნახაზი.dwg"), wide("გეგმა.dwg")]
        );
        assert_eq!(changes.renames(), [(wide("ნახაზი.dwg"), wide("გეგმა.dwg"))]);
        assert!(!changes.is_empty());
        assert!(Changes::default().is_empty());
    }

    #[test]
    fn lost_changes_stay_lost() {
        let mut changes = Changes::default();
        changes.touch(&wide("a"));
        changes.lose();
        changes.touch(&wide("b"));
        assert!(changes.is_lost() && !changes.is_empty());
        assert!(changes.names().is_empty());

        let mut merged = Changes::default();
        merged.touch(&wide("c"));
        merged.merge(&changes);
        assert!(merged.is_lost());
    }

    #[test]
    fn too_many_names_are_lost() {
        let mut changes = Changes::default();
        for i in 0..=MAX_NAMES {
            changes.touch(&wide(&format!("file {i}")));
        }
        assert!(changes.is_lost());
    }

    #[test]
    fn merging_appends_names_and_renames() {
        let mut first = Changes::default();
        first.touch(&wide("a"));
        let mut second = Changes::default();
        second.rename(&wide("a"), &wide("b"));
        first.merge(&second);
        assert_eq!(first.names(), [wide("a"), wide("b")]);
        assert_eq!(first.renames(), [(wide("a"), wide("b"))]);
    }

    #[test]
    fn short_names_are_told_apart() {
        for name in ["PROGRA~1", "NAHAZI~2.DWG", "A~10.TXT"] {
            assert!(is_short_name(&wide(name)), "{name}");
        }
        for name in [
            "~$report.docx",
            "~WRL0001.tmp",
            "plan~1.docx",
            "file~.txt",
            "ნახაზი",
        ] {
            assert!(!is_short_name(&wide(name)), "{name}");
        }
    }

    /// The bytes of `FILE_NOTIFY_INFORMATION` records as `u32`s, as Windows lays them out.
    fn records(records: &[(u32, &str)]) -> (Box<[u32]>, usize) {
        let mut bytes: Vec<u8> = Vec::new();
        for (i, (action, name)) in records.iter().enumerate() {
            let units: Vec<u16> = name.encode_utf16().collect();
            let size = (12 + units.len() * 2).div_ceil(4) * 4;
            let next = if i + 1 == records.len() {
                0
            } else {
                size as u32
            };
            bytes.extend(next.to_le_bytes());
            bytes.extend(action.to_le_bytes());
            bytes.extend(((units.len() * 2) as u32).to_le_bytes());
            bytes.extend(units.iter().flat_map(|unit| unit.to_le_bytes()));
            bytes.resize(bytes.len().div_ceil(4) * 4, 0);
        }
        let len = bytes.len();
        let words: Vec<u32> = bytes
            .chunks(4)
            .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();
        (words.into_boxed_slice(), len)
    }

    fn parsed(buffer: (Box<[u32]>, usize)) -> Changes {
        let (words, len) = buffer;
        let mut changes = Changes::default();
        parse_records(&words, len, &mut None, &mut changes);
        changes
    }

    #[test]
    fn notification_records_parse_into_names_and_renames() {
        let changes = parsed(records(&[
            (1, "new.txt"),
            (3, "ნახაზი.dwg"),
            (4, "old name.pdf"),
            (5, "new name.pdf"),
            (2, "gone"),
        ]));
        assert_eq!(
            changes.names(),
            [
                wide("new.txt"),
                wide("ნახაზი.dwg"),
                wide("old name.pdf"),
                wide("new name.pdf"),
                wide("gone")
            ]
        );
        assert_eq!(
            changes.renames(),
            [(wide("old name.pdf"), wide("new name.pdf"))]
        );
    }

    #[test]
    fn records_that_run_past_the_data_lose_the_changes() {
        let (words, len) = records(&[(1, "a.txt"), (1, "b.txt")]);
        assert!(parsed((words.clone(), len - 4)).is_lost());
        assert!(!parsed((words, len)).is_lost());
    }
}
