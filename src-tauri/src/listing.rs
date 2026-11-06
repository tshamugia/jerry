//! Folder listings for the UI. `listing_open` streams a folder through a `Channel` as binary
//! messages (ADR-0001) and keeps it live, `listing_sort` re-sorts it and `listing_close` stops
//! and forgets it. Every listing is its own background job, so a slow disk holds up only its own
//! tab. Its folder's watcher (`jerry_core::watcher`, shared by every tab on that folder) starts
//! before the folder is read, so no change is missed; changes that come while it is read are
//! applied right after.
//!
//! A listing keeps its volume in use. Every local volume listings are on gets Windows' notices
//! (`jerry_win::devices`), so that when Windows wants to remove or lock it ("Safely remove",
//! Eject), Jerry lets go first, on the main thread, before the notice returns: the watchers close
//! their folders, and a listing still reading stops and ends with `DeviceGone` (#126). If the
//! volume stays, the watchers open their folders again and list them anew; if it goes, the
//! listings on it end with `DeviceGone`.
//!
//! Messages are little-endian and start with the 16-byte header `[kind, a, b, c]`:
//! - Batch (1): `[1, count, firstId, nameUnits]`, then the entries: `f64 size[count]` (bytes),
//!   `f64 modified[count]` and `f64 created[count]` (ms since the Unix epoch),
//!   `u32 attributes[count]`,
//!   `u32 iconKey[count]` (the `icon://` key: 0 for an entry with its own icon, else its type's;
//!   see `jerry_win::icons::keys`), `u32 nameEnd[count]` (where each name ends in this message's
//!   names, in UTF-16 units) and the names as UTF-16 code units. The entries' ids are
//!   `firstId + i`.
//! - Order (2): `[2, count, column, flags]`, then `u32 id[count]`: the entries the folder has, in
//!   display order, sorted by `column` (0 name, 1 modified, 2 type, 3 size, 4 created,
//!   5 extension), descending if `flags` bit 0 is set, folders first if bit 1 is. One comes for
//!   the listing's sort once the whole folder is in, right before Done, one after every Diff, and
//!   one for every `listing_sort` once the folder is in.
//! - Done (3): `[3, count, batches, 0]`, then the `f64` ms the listing took in Rust: the whole
//!   folder is in. Diffs follow while the listing is open.
//! - Error (4): `[4, jsonBytes, 0, 0]`, then the `JerryError` as UTF-8 JSON: reading the folder
//!   failed, or the folder went away (`NotFound`, `DeviceGone`). Last message.
//! - Diff (5): `[5, count, removedCount, nameUnits]`, then `u32 id[count]`,
//!   `u32 removed[removedCount]`, 4 zero bytes if `count + removedCount` is odd (so the `f64`s
//!   stay aligned), and the `count` entries laid out exactly as in a Batch. Ids are ascending:
//!   one the listing already has is that entry changed (a rename keeps its id), the others are
//!   new and continue the listing's ids in order. Removed ids are gone and never come back.
//!   Changes collect for 100 ms; an Order follows.
//!
//! The UI's decoder is `src/ipc/listing.ts`; both sides test this layout.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, RwLock, Weak};
use std::time::{Duration, Instant};

use jerry_core::error::display_path;
use jerry_core::listing::{Diff, Entries, Listing, filetime_to_unix_ms, list};
use jerry_core::path::long_path;
use jerry_core::sort::{PlainTypeNames, Sort, SortColumn, SortIndex};
use jerry_core::volume;
use jerry_core::watcher::{Changes, Event, Subscription, Watchers, refresh};
use jerry_core::{ErrorCode, JerryError};
use jerry_win::devices::{VolumeEvent, VolumeNotices, watch_volume};
use jerry_win::icons::keys::keys as icon_keys;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::ipc::{Bytes, message_buffer};
use crate::jobs;

const BATCH: u32 = 1;
const ORDER: u32 = 2;
const DONE: u32 = 3;
const ERROR: u32 = 4;
const DIFF: u32 = 5;

/// How long letting go of a volume may hold up the main thread, at most. Closing a watched
/// folder or stopping a read takes milliseconds; past this Windows refuses the removal, as it
/// would with any program that holds the drive.
const RELEASE_WAIT: Duration = Duration::from_secs(3);

/// The folder to list.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ListingTarget {
    /// A folder the user named: typed in the address bar, a favorite, a drive. Must be absolute.
    Path { path: String },
    /// Subfolder `entry` of the open listing `listing`, opened by its exact path in core, so
    /// names that aren't valid Unicode open too.
    Entry { listing: u32, entry: u32 },
    /// The folder of the open listing `listing` again, by its exact path: to retry after an
    /// error message, or to refresh.
    Listing { listing: u32 },
    /// A folder a listing named before (`ListingOpened::folder` or `parent`), by its exact path:
    /// Back, Forward, Up, a new or reopened tab. The folder needn't be open any more.
    Folder { folder: u32 },
    /// The user's own folder (`C:\Users\name`): where a tab opens when nothing else is set.
    Home,
}

/// An opened listing: its id, and its folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ListingOpened {
    pub id: u32,
    /// The folder as display text.
    pub path: String,
    /// The folder's name for a tab: its last part, or for a root the root itself (`C:\`).
    pub name: String,
    /// The folder, as a token for `ListingTarget::Folder`: it opens again by its exact path.
    /// The same folder always has the same token.
    pub folder: u32,
    /// The folder it is in, as such a token; none at a root.
    pub parent: Option<u32>,
    /// The address bar's segments, from the root (`C:`, `\\server\share`) to the folder.
    pub crumbs: Vec<Crumb>,
}

/// A folder as the address bar shows it: its name and its token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct Crumb {
    pub name: String,
    /// A token for `ListingTarget::Folder`.
    pub folder: u32,
}

/// The open listings, by id. Clones share them. A tab has at most one: opening another folder
/// for a tab closes the tab's previous listing.
#[derive(Clone, Default)]
pub struct Listings(Arc<Shared>);

/// Called with every folder a listing read to the end after the user went there (not a refresh):
/// `recent_folders` (#31).
type Visits = Box<dyn Fn(&Path) + Send + Sync>;

#[derive(Default)]
struct Shared {
    last_id: AtomicU32,
    open: Mutex<HashMap<u32, Open>>,
    watchers: Watchers,
    /// The volumes listings are on, by root (`jerry_core::volume::root`).
    volumes: Mutex<HashMap<PathBuf, Volume>>,
    /// Folders handed out as tokens (`ListingOpened::folder`).
    folders: Mutex<FolderTokens>,
    visits: Option<Visits>,
}

/// Folders by token and tokens by folder, for the life of the process. A token is a folder's
/// index here: one per folder ever opened (and its parent), so a session holds a few hundred.
#[derive(Default)]
struct FolderTokens {
    paths: Vec<PathBuf>,
    tokens: HashMap<PathBuf, u32>,
}

impl FolderTokens {
    fn token(&mut self, folder: &Path) -> u32 {
        if let Some(&token) = self.tokens.get(folder) {
            return token;
        }
        let token = self.paths.len() as u32;
        self.paths.push(folder.to_path_buf());
        self.tokens.insert(folder.to_path_buf(), token);
        token
    }

    fn path(&self, token: u32) -> Option<&PathBuf> {
        self.paths.get(token as usize)
    }
}

impl Shared {
    fn folders(&self) -> MutexGuard<'_, FolderTokens> {
        self.folders.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn volumes(&self) -> MutexGuard<'_, HashMap<PathBuf, Volume>> {
        self.volumes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Windows has said whether the volume at `root` stays or goes: `update` it if listings are
    /// still on it, else forget it. Returns notices to drop once the lock is let go.
    fn settle_volume(
        &self,
        root: &Path,
        update: impl FnOnce(&mut Volume) -> Option<VolumeNotices>,
    ) -> Option<VolumeNotices> {
        let mut volumes = self.volumes();
        let volume = volumes.get_mut(root)?;
        if volume.has_listings() {
            update(volume)
        } else {
            volumes.remove(root).and_then(|volume| volume.notices)
        }
    }
}

/// A volume listings are on.
#[derive(Default)]
struct Volume {
    /// Its listings, while they live.
    listings: Vec<Weak<ListingState>>,
    /// Windows is removing or locking it: no listing on it starts until it stays or goes.
    released: bool,
    /// Windows' notices about it: none for a network volume, or once it is gone.
    notices: Option<VolumeNotices>,
}

impl Volume {
    /// Forgets listings that are gone; whether any is left.
    fn has_listings(&mut self) -> bool {
        self.listings.retain(|listing| listing.strong_count() > 0);
        !self.listings.is_empty()
    }
}

/// A listing's place among its volume's listings. The volume's notices stop with its last
/// listing.
struct OnVolume {
    shared: Weak<Shared>,
    root: PathBuf,
}

impl Drop for OnVolume {
    fn drop(&mut self) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        let notices = {
            let mut volumes = shared.volumes();
            // A released volume stays known until Windows says whether it stays or goes.
            let unused = volumes
                .get_mut(&self.root)
                .is_some_and(|volume| !volume.has_listings() && !volume.released);
            if unused {
                volumes.remove(&self.root).and_then(|volume| volume.notices)
            } else {
                None
            }
        };
        // Unregistering is only queued, but outside the lock all the same.
        drop(notices);
    }
}

struct Open {
    tab: u32,
    state: Arc<ListingState>,
}

/// One open listing, shared by its job, its watcher subscription and the commands.
struct ListingState {
    folder: PathBuf,
    /// The sort the listing started with: what a prestarted listing is matched by, without
    /// waiting for `live`, which a change being applied holds through its IO.
    started_sort: Sort,
    /// The user went to the folder (not a refresh or a prestart): once it is read to the end, it
    /// counts as a visit (`recent_folders`).
    visit: bool,
    listing: RwLock<Listing>,
    live: Mutex<Live>,
    cancel: AtomicBool,
    outbox: Outbox,
    /// The folder's watcher, once the job has subscribed. Its callback holds this state only
    /// weakly, so closing the listing drops both.
    watch: Mutex<Option<Subscription>>,
    /// Whether the job still runs: letting go of the volume waits for it to stop reading.
    running: Mutex<bool>,
    stopped_running: Condvar,
    /// The volume was released while the job read it: the stream ends with `DeviceGone`.
    released: AtomicBool,
    /// Its place among its volume's listings, once the job knows the volume.
    on_volume: OnceLock<OnVolume>,
}

/// What changes as a listing goes on. Every message after the batches is sent under its lock,
/// so diffs and orders go out in the order they happened.
struct Live {
    sort: Sort,
    /// Once the whole folder is in: its sort index. Until then, changes wait in `pending`.
    index: Option<SortIndex>,
    pending: Changes,
    /// After an error message: nothing more goes out.
    ended: bool,
}

impl ListingState {
    /// A listing of `folder` sorted by `sort`, its job about to run.
    fn new(folder: PathBuf, sort: Sort, channel: Option<Channel<Bytes>>) -> Self {
        Self {
            listing: RwLock::new(Listing::new(folder.clone())),
            folder,
            started_sort: sort,
            visit: false,
            live: Mutex::new(Live {
                sort,
                index: None,
                pending: Changes::default(),
                ended: false,
            }),
            cancel: AtomicBool::new(false),
            outbox: Outbox::new(channel),
            watch: Mutex::new(None),
            running: Mutex::new(true),
            stopped_running: Condvar::new(),
            released: AtomicBool::new(false),
            on_volume: OnceLock::new(),
        }
    }

    /// If the job still runs, stops what it reads, as its volume is being released: a stream
    /// that isn't through the folder yet ends with `DeviceGone`. Whether the job runs.
    fn stop_reading(&self) -> bool {
        let running = self.running();
        if *running {
            self.released.store(true, Ordering::Relaxed);
            self.cancel.store(true, Ordering::Relaxed);
        }
        *running
    }

    /// The job is done: nothing it opened is open any more.
    fn job_done(&self) {
        *self.running() = false;
        self.stopped_running.notify_all();
    }

    /// Waits until the job is done, or until `deadline`; whether it is.
    fn wait_job(&self, deadline: Instant) -> bool {
        let mut running = self.running();
        while *running {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            running = self
                .stopped_running
                .wait_timeout(running, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    /// Ends the stream as the job's `result` says.
    fn finish(&self, id: u32, result: Result<(), JerryError>) {
        let error = match result {
            Ok(()) => return,
            Err(error) if error.code == ErrorCode::Cancelled => {
                if !self.released.load(Ordering::Relaxed) {
                    return; // closed
                }
                // The volume is being removed: the folder can't be read to the end.
                JerryError::new(ErrorCode::DeviceGone).at(&self.folder)
            }
            Err(error) => error,
        };
        tracing::info!(id, code = ?error.code, "listing failed");
        // Nothing to keep live.
        self.watch().take();
        self.end(&mut self.live(), &error);
    }

    fn running(&self) -> MutexGuard<'_, bool> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn live(&self) -> MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn watch(&self) -> MutexGuard<'_, Option<Subscription>> {
        self.watch.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Listing> {
        self.listing.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Listing> {
        self.listing.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sends `error` as the last message.
    fn end(&self, live: &mut Live, error: &JerryError) {
        live.ended = true;
        self.outbox.send(error_message(error));
    }

    /// Sends `diff` and the order that follows it, with the sort index brought up to date.
    fn send_diff(&self, live: &mut Live, diff: &Diff) {
        if diff.is_empty() {
            return;
        }
        let started = Instant::now();
        let listing = self.read();
        self.outbox.send(diff_message(&listing, diff));
        let index = match live.index.take() {
            Some(_) if diff.reindex => {
                SortIndex::build_for(listing.entries(), listing.live_ids(), &PlainTypeNames)
            }
            Some(index) if !diff.removed.is_empty() => index.without(&diff.removed),
            Some(index) => index,
            None => return,
        };
        self.outbox.send(order_message(
            live.sort,
            &index.order(listing.entries(), live.sort),
        ));
        live.index = Some(index);
        tracing::debug!(
            entries = listing.len(),
            changed = diff.changed.len(),
            added = diff.added.len(),
            removed = diff.removed.len(),
            ms = started.elapsed().as_secs_f64() * 1000.0,
            "listing diff sent"
        );
    }

    /// Applies what the folder's watcher says, or keeps it for after the read.
    fn on_event(&self, event: Event<'_>) {
        let mut live = self.live();
        if live.ended {
            return;
        }
        if live.index.is_none() {
            // Still reading: look the names up again once the whole folder is in.
            match event {
                Event::Changed { changes, .. } => live.pending.merge(changes),
                Event::Gone(_) => live.pending.lose(),
            }
            return;
        }
        match event {
            Event::Changed { now, .. } => {
                let diff = now.apply_to(&mut self.write());
                self.send_diff(&mut live, &diff);
            }
            Event::Gone(error) => self.end(&mut live, error),
        }
    }
}

impl Listings {
    /// Listings that call `visits` with every folder the user went to, once it is read to the end:
    /// the app notes them in `recent_folders`. [`Listings::default`] notes nothing.
    pub fn new(visits: impl Fn(&Path) + Send + Sync + 'static) -> Self {
        Self(Arc::new(Shared {
            visits: Some(Box::new(visits)),
            ..Shared::default()
        }))
    }

    /// Starts listing `folder` for `tab`, sorted by `sort`, before the UI has a channel for it:
    /// at process start, before Tauri, for the session's folders (spike #11, session restore
    /// #42). Messages are kept until `listing_open` for the same tab, folder and sort attaches
    /// its channel.
    pub fn prestart(&self, tab: u32, folder: &Path, sort: Sort) -> Result<u32, JerryError> {
        Ok(self.start(tab, long_path(folder)?, sort, None, false))
    }

    /// The folder `target` names, as a long path.
    fn resolve(&self, target: ListingTarget) -> Result<PathBuf, JerryError> {
        match target {
            ListingTarget::Path { path } => {
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    return Err(JerryError::new(ErrorCode::InvalidName).at(path));
                }
                long_path(&path)
            }
            ListingTarget::Entry { listing, entry } => {
                let open = self.open();
                open.get(&listing)
                    .and_then(|open| open.state.read().path(entry as usize))
                    .ok_or_else(|| JerryError::new(ErrorCode::NotFound))
            }
            ListingTarget::Listing { listing } => self
                .open()
                .get(&listing)
                .map(|open| open.state.folder.clone())
                .ok_or_else(|| JerryError::new(ErrorCode::NotFound)),
            ListingTarget::Folder { folder } => self
                .folder_path(folder)
                .ok_or_else(|| JerryError::new(ErrorCode::NotFound)),
            // Blocking: `listing_open` resolves it on a background job.
            ListingTarget::Home => long_path(&jerry_win::folders::profile()?),
        }
    }

    /// The folder token `folder` stands for (`ListingOpened::folder`).
    pub fn folder_path(&self, folder: u32) -> Option<PathBuf> {
        self.0.folders().path(folder).cloned()
    }

    /// The token for `folder` (an exact long path), for `ListingTarget::Folder`.
    pub fn folder_token(&self, folder: &Path) -> u32 {
        self.0.folders().token(folder)
    }

    /// `folder` as an address bar segment.
    pub fn crumb(&self, folder: &Path) -> Crumb {
        Crumb {
            name: crumb_name(folder),
            folder: self.folder_token(folder),
        }
    }

    /// What `listing_open` answers for listing `id` of `folder`: display text and tokens.
    fn opened(&self, id: u32, folder: &Path) -> ListingOpened {
        let name = folder.file_name().map_or_else(
            || display_path(folder),
            |name| name.to_string_lossy().into_owned(),
        );
        let mut tokens = self.0.folders();
        let mut crumbs: Vec<Crumb> = folder
            .ancestors()
            .map(|each| Crumb {
                name: crumb_name(each),
                folder: tokens.token(each),
            })
            .collect();
        crumbs.reverse();
        ListingOpened {
            id,
            path: display_path(folder),
            name,
            folder: tokens.token(folder),
            parent: folder.parent().map(|parent| tokens.token(parent)),
            crumbs,
        }
    }

    /// Opens `folder` for `tab`, sorted by `sort`, with its messages going to `channel`: a
    /// listing started for it with [`Listings::prestart`] if there is one, a new one otherwise.
    fn open_folder(
        &self,
        tab: u32,
        folder: PathBuf,
        sort: Sort,
        channel: Channel<Bytes>,
        visit: bool,
    ) -> u32 {
        let prestarted = self.open().iter().find_map(|(&id, open)| {
            let waiting = open.tab == tab
                && !open.state.outbox.is_attached()
                && open.state.started_sort == sort
                && open.state.folder == folder;
            waiting.then(|| (id, open.state.outbox.clone()))
        });
        match prestarted {
            Some((id, outbox)) => {
                outbox.attach(channel);
                id
            }
            None => self.start(tab, folder, sort, Some(channel), visit),
        }
    }

    /// Starts listing `folder` for `tab` on a background job and returns the listing's id.
    fn start(
        &self,
        tab: u32,
        folder: PathBuf,
        sort: Sort,
        channel: Option<Channel<Bytes>>,
        visit: bool,
    ) -> u32 {
        self.close_tab(tab);
        let id = self.0.last_id.fetch_add(1, Ordering::Relaxed) + 1;
        let state = Arc::new(ListingState {
            visit,
            ..ListingState::new(folder, sort, channel)
        });
        self.open().insert(
            id,
            Open {
                tab,
                state: state.clone(),
            },
        );
        let listings = self.clone();
        tauri::async_runtime::spawn(async move {
            let job_state = state.clone();
            let job = move || {
                let _done = JobDone(&job_state);
                run(&job_state, &listings)
            };
            let result = jobs::background("listing", job).await;
            state.finish(id, result);
        });
        id
    }

    /// Puts `state` among its volume's listings, so it hears when Windows wants the volume. The
    /// first listing on a local volume asks Windows for its notices. Fails with `DeviceGone`
    /// while the volume is released. Blocking IO: Windows looks for the volume's root.
    fn enter_volume(&self, state: &Arc<ListingState>) -> Result<(), JerryError> {
        // Without a root (a drive that is gone), reading the folder says what is wrong.
        let Ok(root) = volume::root(&state.folder) else {
            return Ok(());
        };
        let local = volume::is_local(&root);
        let mut volumes = self.0.volumes();
        let volume = volumes.entry(root.clone()).or_default();
        if volume.released {
            return Err(JerryError::new(ErrorCode::DeviceGone).at(&state.folder));
        }
        volume.has_listings();
        volume.listings.push(Arc::downgrade(state));
        if local && volume.notices.is_none() {
            volume.notices = Some(self.notices(root.clone()));
        }
        let _ = state.on_volume.set(OnVolume {
            shared: Arc::downgrade(&self.0),
            root,
        });
        Ok(())
    }

    /// Windows' notices about the volume at `root`, handed to [`Listings::on_volume`].
    fn notices(&self, root: PathBuf) -> VolumeNotices {
        let shared = Arc::downgrade(&self.0);
        let registered = root.clone();
        watch_volume(&registered, move |event| {
            if let Some(shared) = shared.upgrade() {
                Listings(shared).on_volume(&root, event);
            }
        })
    }

    /// What Windows says about the volume at `root`. On the main thread, inside the notice: a
    /// release is done when this returns.
    fn on_volume(&self, root: &Path, event: VolumeEvent) {
        match event {
            VolumeEvent::Release => self.release(root),
            VolumeEvent::Keep => self.keep(root),
            VolumeEvent::Removed => self.removed(root),
        }
    }

    /// Lets go of everything the listings hold open on the volume at `root`, as Windows wants to
    /// remove or lock it: each watcher closes its folder, and a listing still reading stops and
    /// ends with `DeviceGone`. Until the volume stays or goes, no listing on it starts. Waits for
    /// all that, at most [`RELEASE_WAIT`].
    fn release(&self, root: &Path) {
        let started = Instant::now();
        let deadline = started + RELEASE_WAIT;
        let listings: Vec<Arc<ListingState>> = {
            let mut volumes = self.0.volumes();
            let volume = volumes.entry(root.to_path_buf()).or_default();
            volume.released = true;
            volume.listings.iter().filter_map(Weak::upgrade).collect()
        };
        let reading: Vec<Arc<ListingState>> = listings
            .into_iter()
            .filter(|state| state.stop_reading())
            .collect();
        let closed = self.0.watchers.release(root, deadline);
        let stopped = reading.iter().all(|state| state.wait_job(deadline));
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let root = display_path(root);
        if closed && stopped {
            tracing::info!(%root, reading = reading.len(), ms, "volume released");
        } else {
            tracing::warn!(%root, ms, "volume not released in time: Windows may keep it");
        }
    }

    /// The volume at `root` stays: its removal or lock failed, or the lock is over. Listings on
    /// it start again, and the watchers open their folders again and list them anew.
    fn keep(&self, root: &Path) {
        let notices = self.0.settle_volume(root, |volume| {
            volume.released = false;
            None
        });
        drop(notices);
        self.0.watchers.keep(root);
    }

    /// The volume at `root` is gone: the listings on it end with `DeviceGone` (their watchers
    /// say so), and if it comes back under the same root it gets new notices.
    fn removed(&self, root: &Path) {
        let notices = self.0.settle_volume(root, |volume| {
            volume.released = false;
            volume.notices.take()
        });
        drop(notices);
        self.0.watchers.gone(root);
    }

    /// Sorts listing `id` by `sort` from now on. Once the whole folder is in, its order goes out
    /// through the stream, after any diff before it; while the folder is still being read, the
    /// stream's own Order follows the new sort. CPU work: call it on a background thread.
    fn sort(&self, id: u32, sort: Sort) -> Result<(), JerryError> {
        let state = self
            .open()
            .get(&id)
            .map(|open| open.state.clone())
            .ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
        let mut live = state.live();
        live.sort = sort;
        let Some(index) = live.index.as_ref().filter(|_| !live.ended) else {
            return Ok(());
        };
        let started = Instant::now();
        let order = index.order(state.read().entries(), sort);
        state.outbox.send(order_message(sort, &order));
        tracing::debug!(
            id,
            entries = order.len(),
            ms = started.elapsed().as_secs_f64() * 1000.0,
            "listing sorted"
        );
        Ok(())
    }

    /// Stops listing `id` if it still runs and forgets it. No message follows.
    pub fn close(&self, id: u32) {
        let closed = self.open().remove(&id);
        if let Some(open) = closed {
            open.stop();
        }
    }

    /// The folder the open listing `listing` shows, exactly: where a new folder goes (#36).
    pub fn folder(&self, listing: u32) -> Option<PathBuf> {
        self.open()
            .get(&listing)
            .map(|open| open.state.folder.clone())
    }

    /// The exact path, attributes and last write time (FILETIME) of entry `entry` of the open
    /// listing `listing`: for the `icon://` protocol, which asks for an entry's own icon by id.
    pub fn entry(&self, listing: u32, entry: u32) -> Option<(PathBuf, u32, u64)> {
        let state = self.open().get(&listing)?.state.clone();
        let listing = state.read();
        let id = entry as usize;
        let entries = listing.entries();
        let (attributes, modified) = (*entries.attributes().get(id)?, *entries.modified().get(id)?);
        Some((listing.path(id)?, attributes, modified))
    }

    /// The entries of the open listing `listing` the folder has now whose names (exact UTF-16) are
    /// among `names`: the rows of Jerry's cut (#38). None for a listing that isn't open.
    pub fn entries_named(&self, listing: u32, names: &HashSet<Vec<u16>>) -> Vec<u32> {
        if names.is_empty() {
            return Vec::new();
        }
        let Some(state) = self.open().get(&listing).map(|open| open.state.clone()) else {
            return Vec::new();
        };
        let listing = state.read();
        let entries = listing.entries();
        (0..entries.len())
            .filter(|&id| !listing.is_removed(id) && names.contains(entries.name_utf16(id)))
            .map(|id| id as u32)
            .collect()
    }

    fn close_tab(&self, tab: u32) {
        let mut open = self.open();
        open.retain(|_, listing| {
            let keep = listing.tab != tab;
            if !keep {
                listing.stop();
            }
            keep
        });
    }

    fn open(&self) -> MutexGuard<'_, HashMap<u32, Open>> {
        self.0.open.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Open {
    fn stop(&self) {
        self.state.cancel.store(true, Ordering::Relaxed);
        self.state.outbox.close();
        // Stops the folder's watcher if no other tab watches it.
        self.state.watch().take();
    }
}

/// Marks a listing's job done when dropped, panic or not.
struct JobDone<'a>(&'a ListingState);

impl Drop for JobDone<'_> {
    fn drop(&mut self) {
        self.0.job_done();
    }
}

/// The listing job: joins its volume's listings, subscribes to the folder's watcher, reads the
/// folder and sends each batch, then builds the sort index and sends the order and the done
/// message, applies the changes that came while it read, and last notes the visit.
fn run(state: &Arc<ListingState>, listings: &Listings) -> Result<(), JerryError> {
    let started = Instant::now();
    listings.enter_volume(state)?;
    watch(state, &listings.0.watchers);
    let mut count = 0;
    let mut batches = 0;
    list(&state.folder, &state.cancel, |batch| {
        // Core has the entries before the UI does, so every id the UI gets resolves.
        state.write().append(batch);
        state.outbox.send(batch_message(count, batch));
        count += batch.len();
        batches += 1;
    })?;
    // Under the lock, so a `listing_sort` meanwhile either set the sort used here or sees the
    // index and sends its own order after this one.
    let mut live = state.live();
    {
        let listing = state.read();
        let index = SortIndex::build(listing.entries(), &PlainTypeNames);
        state.outbox.send(order_message(
            live.sort,
            &index.order(listing.entries(), live.sort),
        ));
        live.index = Some(index);
    }
    state.outbox.send(done_message(
        count,
        batches,
        started.elapsed().as_secs_f64() * 1000.0,
    ));
    let pending = std::mem::take(&mut live.pending);
    if !pending.is_empty() {
        match refresh(&state.folder, &pending, &state.cancel) {
            Ok(now) => {
                let diff = now.apply_to(&mut state.write());
                state.send_diff(&mut live, &diff);
            }
            Err(error) if error.code == ErrorCode::Cancelled => {}
            Err(error) => state.end(&mut live, &error),
        }
    }
    drop(live);
    if state.visit
        && let Some(visits) = &listings.0.visits
    {
        visits(&state.folder);
    }
    Ok(())
}

/// Subscribes `state` to its folder's watcher, unless the listing was closed meanwhile.
fn watch(state: &Arc<ListingState>, watchers: &Watchers) {
    let weak = Arc::downgrade(state);
    let subscribed = watchers.subscribe(&state.folder, move |event| {
        if let Some(state) = weak.upgrade() {
            state.on_event(event);
        }
    });
    match subscribed {
        Ok(subscription) => {
            // Checked under the lock `Open::stop` takes the subscription with.
            let mut slot = state.watch();
            if !state.cancel.load(Ordering::Relaxed) {
                *slot = Some(subscription);
            }
        }
        // Reading the folder reports what is wrong with it; unwatched, the list just isn't live.
        Err(error) => tracing::info!(code = ?error.code, "folder isn't watched"),
    }
}

/// A folder's name in the address bar: its last part; a drive's root as `C:`, a share's as
/// `\\server\share`.
pub(crate) fn crumb_name(folder: &Path) -> String {
    match folder.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => display_path(folder).trim_end_matches('\\').to_owned(),
    }
}

/// Entries `first_id..` as one batch message (see the module docs).
fn batch_message(first_id: usize, batch: &Entries) -> Bytes {
    let count = batch.len();
    let units = batch.name_units().len();
    // A batch holds at most `MAX_BATCH` entries, and a folder fewer than 2^32: these fit.
    let header = [BATCH, count as u32, first_id as u32, units as u32];
    let mut out = message_buffer(header, entries_bytes(count, units));
    push_entries(&mut out, batch);
    Bytes(out)
}

/// The changed and new entries of `listing` and its removed ids, as one diff message (see the
/// module docs).
fn diff_message(listing: &Listing, diff: &Diff) -> Bytes {
    let ids: Vec<u32> = diff
        .changed
        .iter()
        .copied()
        .chain(diff.added.clone())
        .collect();
    let entries = listing.entries().copy_of(&ids);
    let units = entries.name_units().len();
    let words = ids.len() + diff.removed.len();
    let padding = words % 2 * 4;
    let header = [
        DIFF,
        ids.len() as u32,
        diff.removed.len() as u32,
        units as u32,
    ];
    let mut out = message_buffer(
        header,
        words * 4 + padding + entries_bytes(ids.len(), units),
    );
    for &id in ids.iter().chain(&diff.removed) {
        out.extend_from_slice(&id.to_le_bytes());
    }
    out.resize(out.len() + padding, 0);
    push_entries(&mut out, &entries);
    Bytes(out)
}

/// The size of `count` entries with `units` name units, as [`push_entries`] writes them.
fn entries_bytes(count: usize, units: usize) -> usize {
    count * 36 + units * 2
}

/// Writes `entries` as batches and diffs carry them: each column in turn, then the names.
fn push_entries(out: &mut Vec<u8>, entries: &Entries) {
    for &size in entries.sizes() {
        out.extend_from_slice(&(size as f64).to_le_bytes());
    }
    for &modified in entries.modified() {
        out.extend_from_slice(&filetime_to_unix_ms(modified).to_le_bytes());
    }
    for &created in entries.created() {
        out.extend_from_slice(&filetime_to_unix_ms(created).to_le_bytes());
    }
    for &attributes in entries.attributes() {
        out.extend_from_slice(&attributes.to_le_bytes());
    }
    for key in icon_keys(entries) {
        out.extend_from_slice(&key.to_le_bytes());
    }
    for &end in entries.name_ends() {
        out.extend_from_slice(&(end as u32).to_le_bytes());
    }
    for &unit in entries.name_units() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
}

/// Entry ids in display order as one order message (see the module docs).
fn order_message(sort: Sort, order: &[u32]) -> Bytes {
    let column = match sort.column {
        SortColumn::Name => 0,
        SortColumn::Modified => 1,
        SortColumn::Type => 2,
        SortColumn::Size => 3,
        SortColumn::Created => 4,
        SortColumn::Extension => 5,
    };
    let flags = u32::from(sort.descending) | u32::from(sort.folders_first) << 1;
    let mut out = message_buffer([ORDER, order.len() as u32, column, flags], order.len() * 4);
    for &id in order {
        out.extend_from_slice(&id.to_le_bytes());
    }
    Bytes(out)
}

fn done_message(count: usize, batches: usize, ms: f64) -> Bytes {
    let mut out = message_buffer([DONE, count as u32, batches as u32, 0], 8);
    out.extend_from_slice(&ms.to_le_bytes());
    Bytes(out)
}

fn error_message(error: &JerryError) -> Bytes {
    let json = serde_json::to_vec(error).unwrap_or_else(|serialize_error| {
        tracing::error!(%serialize_error, "listing error didn't serialize");
        br#"{"code":"Internal","path":null}"#.to_vec()
    });
    let mut out = message_buffer([ERROR, json.len() as u32, 0, 0], json.len());
    out.extend_from_slice(&json);
    Bytes(out)
}

/// Where a listing's messages go: the UI's channel, or kept in order until one is attached.
/// Once closed, nothing more goes out.
#[derive(Clone)]
struct Outbox(Arc<Mutex<OutboxState>>);

struct OutboxState {
    channel: Option<Channel<Bytes>>,
    kept: Vec<Bytes>,
    closed: bool,
}

impl Outbox {
    fn new(channel: Option<Channel<Bytes>>) -> Self {
        Self(Arc::new(Mutex::new(OutboxState {
            channel,
            kept: Vec::new(),
            closed: false,
        })))
    }

    fn is_attached(&self) -> bool {
        self.state().channel.is_some()
    }

    fn send(&self, message: Bytes) {
        let mut guard = self.state();
        let state = &mut *guard;
        if state.closed {
            return;
        }
        match &state.channel {
            Some(channel) => deliver(channel, message),
            None => state.kept.push(message),
        }
    }

    /// Sends what was kept to `channel`, then everything after it.
    fn attach(&self, channel: Channel<Bytes>) {
        let mut state = self.state();
        if state.closed {
            return;
        }
        for message in std::mem::take(&mut state.kept) {
            deliver(&channel, message);
        }
        state.channel = Some(channel);
    }

    fn close(&self) {
        let mut state = self.state();
        state.closed = true;
        state.channel = None;
        state.kept = Vec::new();
    }

    fn state(&self) -> MutexGuard<'_, OutboxState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn deliver(channel: &Channel<Bytes>, message: Bytes) {
    if let Err(error) = channel.send(message) {
        tracing::warn!(%error, "listing message not delivered");
    }
}

/// Opens a folder for `tab` and streams its entries to `on_message` as binary batches, then
/// their order by `sort` and a done message, then a diff and an order for every change while the
/// listing is open, or an error message (decoded by `src/ipc/listing.ts`). Returns at once,
/// before the folder is read: errors reading it, such as `AccessDenied`, `NotFound` or
/// `DeviceGone`, come as the stream's error message, and so does the folder going away. Opening
/// a folder for a tab closes the tab's previous listing.
#[tauri::command]
#[specta::specta]
pub async fn listing_open(
    listings: State<'_, Listings>,
    tab: u32,
    target: ListingTarget,
    sort: Sort,
    on_message: Channel<Bytes>,
) -> Result<ListingOpened, JerryError> {
    // Only a folder the user went to is a visit: not a refresh or a retry, nor the folder a tab
    // starts on.
    let visit = !matches!(target, ListingTarget::Listing { .. } | ListingTarget::Home);
    let folder = match target {
        ListingTarget::Home => {
            let listings = listings.inner().clone();
            jobs::background("home folder", move || listings.resolve(ListingTarget::Home)).await?
        }
        target => listings.resolve(target)?,
    };
    let id = listings.open_folder(tab, folder.clone(), sort, on_message, visit);
    Ok(listings.opened(id, &folder))
}

/// Sorts listing `id` by `sort` from now on. The new order (only entry ids) comes through the
/// listing's stream as an order message, after any diff sent before it, so it always names the
/// entries the UI has. While the folder is still being read, the stream's own order message
/// follows the new sort. Fails with `NotFound` for a closed listing.
#[tauri::command]
#[specta::specta]
pub async fn listing_sort(app: AppHandle, id: u32, sort: Sort) -> Result<(), JerryError> {
    let listings = app.state::<Listings>().inner().clone();
    jobs::background("sort", move || listings.sort(id, sort)).await
}

/// Stops listing `id` if it still runs and forgets it: no message follows, and the folder's
/// watcher stops unless another tab has the folder open. Closing a listing that is already
/// closed does nothing.
#[tauri::command]
#[specta::specta]
pub async fn listing_close(app: AppHandle, id: u32) {
    app.state::<Listings>().close(id);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    use tauri::ipc::InvokeResponseBody;

    use super::*;

    fn word(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"))
    }

    fn float(bytes: &[u8], at: usize) -> f64 {
        f64::from_le_bytes(bytes[at..at + 8].try_into().expect("8 bytes"))
    }

    #[test]
    fn batches_are_aligned_little_endian_columns() {
        let mut batch = Entries::default();
        let names = ["file10.txt", "ნახაზი"];
        let units: Vec<Vec<u16>> = names.iter().map(|n| n.encode_utf16().collect()).collect();
        // FILETIMEs of 2026-01-01T00:00:00Z and of the Unix epoch.
        batch.push(
            &units[0],
            1 << 40,
            116_444_736_000_000_000,
            134_116_992_000_000_000,
            0x20,
        );
        batch.push(
            &units[1],
            0,
            134_116_992_000_000_000,
            116_444_736_000_000_000,
            0x10,
        );
        let txt = jerry_win::icons::keys::key(&units[0], 0x20);

        let Bytes(bytes) = batch_message(7, &batch);

        assert_eq!(
            [word(&bytes, 0), word(&bytes, 4), word(&bytes, 8)],
            [1, 2, 7]
        );
        assert_eq!(word(&bytes, 12), 16, "10 + 6 name units");
        assert_eq!(
            [float(&bytes, 16), float(&bytes, 24)],
            [(1_u64 << 40) as f64, 0.0]
        );
        assert_eq!(
            [float(&bytes, 32), float(&bytes, 40)],
            [1_767_225_600_000.0, 0.0],
            "modified"
        );
        assert_eq!(
            [float(&bytes, 48), float(&bytes, 56)],
            [0.0, 1_767_225_600_000.0],
            "created"
        );
        assert_eq!([word(&bytes, 64), word(&bytes, 68)], [0x20, 0x10]);
        assert_eq!(
            [word(&bytes, 72), word(&bytes, 76)],
            [txt, jerry_win::icons::keys::FOLDER]
        );
        assert_eq!([word(&bytes, 80), word(&bytes, 84)], [10, 16]);
        let names_at = 88;
        let (pairs, rest) = bytes[names_at..].as_chunks::<2>();
        assert!(rest.is_empty());
        let decoded: Vec<u16> = pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect();
        assert_eq!(String::from_utf16_lossy(&decoded), "file10.txtნახაზი");
        assert_eq!(bytes.len(), names_at + 16 * 2);
    }

    #[test]
    fn orders_are_ids_after_the_sort_they_follow() {
        let sort = Sort {
            column: SortColumn::Created,
            descending: true,
            folders_first: true,
        };
        let Bytes(bytes) = order_message(sort, &[2, 0, 70_000]);
        assert_eq!(
            [
                word(&bytes, 0),
                word(&bytes, 4),
                word(&bytes, 8),
                word(&bytes, 12)
            ],
            [ORDER, 3, 4, 0b11]
        );
        assert_eq!(
            [word(&bytes, 16), word(&bytes, 20), word(&bytes, 24)],
            [2, 0, 70_000]
        );
        assert_eq!(bytes.len(), 28);

        let Bytes(empty) = order_message(Sort::default(), &[]);
        assert_eq!(
            [word(&empty, 4), word(&empty, 8), word(&empty, 12)],
            [0, 0, 0b10]
        );
        assert_eq!(empty.len(), 16);
    }

    #[test]
    fn done_and_error_messages() {
        let Bytes(done) = done_message(1_000, 2, 4.5);
        assert_eq!(
            [word(&done, 0), word(&done, 4), word(&done, 8)],
            [3, 1_000, 2]
        );
        assert_eq!(float(&done, 16), 4.5);

        let error = JerryError::from_win32(5).at(r"\\?\C:\დახურული");
        let Bytes(message) = error_message(&error);
        assert_eq!(word(&message, 0), 4);
        let json = &message[16..];
        assert_eq!(word(&message, 4) as usize, json.len());
        let value: serde_json::Value = serde_json::from_slice(json).expect("JSON");
        assert_eq!(
            value,
            serde_json::json!({ "code": "AccessDenied", "path": r"C:\დახურული" })
        );
    }

    #[test]
    fn diffs_carry_ids_removals_and_entries_laid_out_as_in_batches() {
        let mut listing = Listing::new(PathBuf::from(r"\\?\C:\Work"));
        let mut batch = Entries::default();
        for name in ["a.txt", "b.txt", "c.txt"] {
            let units: Vec<u16> = name.encode_utf16().collect();
            batch.push(&units, 1, 0, 116_444_736_000_000_000, 0x20);
        }
        listing.append(&batch);
        let mut present = Entries::default();
        let units: Vec<u16> = "გეგმა.pdf".encode_utf16().collect();
        present.push(
            &units,
            1 << 33,
            134_116_992_000_000_000,
            134_116_992_000_000_000,
            0x20,
        );
        let units: Vec<u16> = "b.txt".encode_utf16().collect();
        present.push(&units, 5, 0, 116_444_736_000_000_000, 0x20);
        let diff = listing.apply(&jerry_core::listing::Observed {
            present,
            absent: vec!["a.txt".encode_utf16().collect()],
            renames: Vec::new(),
        });

        let Bytes(bytes) = diff_message(&listing, &diff);

        // b.txt changed (id 1), გეგმა.pdf is new (id 3), a.txt is gone (id 0).
        assert_eq!(
            [
                word(&bytes, 0),
                word(&bytes, 4),
                word(&bytes, 8),
                word(&bytes, 12)
            ],
            [DIFF, 2, 1, 5 + 9]
        );
        assert_eq!([word(&bytes, 16), word(&bytes, 20)], [1, 3], "ids");
        assert_eq!(word(&bytes, 24), 0, "removed");
        assert_eq!(word(&bytes, 28), 0, "padding to 8 bytes");
        let entries_at = 32;
        assert_eq!(
            [float(&bytes, entries_at), float(&bytes, entries_at + 8)],
            [5.0, (1_u64 << 33) as f64]
        );
        assert_eq!(
            float(&bytes, entries_at + 24),
            1_767_225_600_000.0,
            "modified"
        );
        assert_eq!(
            float(&bytes, entries_at + 40),
            1_767_225_600_000.0,
            "created"
        );
        let keys_at = entries_at + 48 + 8;
        let key = |name: &str| {
            let units: Vec<u16> = name.encode_utf16().collect();
            jerry_win::icons::keys::key(&units, 0x20)
        };
        assert_eq!(
            [word(&bytes, keys_at), word(&bytes, keys_at + 4)],
            [key("b.txt"), key("გეგმა.pdf")]
        );
        let ends_at = keys_at + 8;
        assert_eq!([word(&bytes, ends_at), word(&bytes, ends_at + 4)], [5, 14]);
        let (pairs, rest) = bytes[ends_at + 8..].as_chunks::<2>();
        assert!(rest.is_empty());
        let names: Vec<u16> = pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect();
        assert_eq!(String::from_utf16_lossy(&names), "b.txtგეგმა.pdf");

        // An even number of ids needs no padding.
        let Bytes(removal) = diff_message(
            &listing,
            &Diff {
                removed: vec![1, 2],
                ..Diff::default()
            },
        );
        assert_eq!([word(&removal, 4), word(&removal, 8)], [0, 2]);
        assert_eq!(removal.len(), 16 + 8);
    }

    /// A channel whose raw messages arrive on the returned receiver.
    fn test_channel() -> (Channel<Bytes>, Receiver<Vec<u8>>) {
        let (sender, receiver) = channel();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Raw(bytes) = body {
                let _ = sender.send(bytes);
            }
            Ok(())
        });
        (channel, receiver)
    }

    fn next(receiver: &Receiver<Vec<u8>>) -> Vec<u8> {
        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("a message comes")
    }

    /// The messages up to and including the done or error message.
    fn until_end(receiver: &Receiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut messages = Vec::new();
        loop {
            let message = next(receiver);
            let kind = word(&message, 0);
            messages.push(message);
            if kind == DONE || kind == ERROR {
                return messages;
            }
        }
    }

    fn folder_with(names: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp folder");
        for name in names {
            std::fs::write(dir.path().join(name), name.as_bytes()).expect("write");
        }
        dir
    }

    fn path_target(path: &Path) -> ListingTarget {
        ListingTarget::Path {
            path: path.to_string_lossy().into_owned(),
        }
    }

    fn state(listings: &Listings, id: u32) -> Arc<ListingState> {
        listings.open()[&id].state.clone()
    }

    /// The names of an order message's entries, in its order.
    fn ordered_names(listings: &Listings, id: u32, message: &[u8]) -> Vec<String> {
        assert_eq!(word(message, 0), ORDER);
        let entries = state(listings, id).read().entries().clone();
        let count = word(message, 4) as usize;
        assert_eq!(message.len(), 16 + count * 4);
        (0..count)
            .map(|i| word(message, 16 + i * 4) as usize)
            .map(|entry| entries.name(entry).to_string_lossy().into_owned())
            .collect()
    }

    const BY_SIZE: Sort = Sort {
        column: SortColumn::Size,
        descending: true,
        folders_first: true,
    };

    #[test]
    fn opens_a_folder_and_streams_it() {
        let dir = folder_with(&["file10.txt", "ნახაზი.dwg", "file2.txt"]);
        std::fs::create_dir(dir.path().join("ქვესაქაღალდე")).expect("mkdir");
        let listings = Listings::default();
        let (channel, receiver) = test_channel();

        let folder = listings.resolve(path_target(dir.path())).expect("resolves");
        let id = listings.open_folder(1, folder, Sort::default(), channel, false);
        let messages = until_end(&receiver);

        let (batches, end) = messages.split_at(messages.len() - 2);
        let [order, done] = end else {
            unreachable!("split at two from the end")
        };
        assert_eq!(word(done, 0), DONE);
        assert_eq!(word(done, 4), 4, "four entries");
        assert_eq!(word(done, 8) as usize, batches.len());
        assert!(batches.iter().all(|batch| word(batch, 0) == BATCH));
        assert_eq!([word(order, 8), word(order, 12)], [0, 0b10], "by name");
        assert_eq!(
            ordered_names(&listings, id, order),
            ["ქვესაქაღალდე", "file2.txt", "file10.txt", "ნახაზი.dwg"]
        );

        // Every id the stream gave has its exact path in core, and a subfolder opens by its id.
        let entries = state(&listings, id).read().entries().clone();
        let sub = (0..entries.len())
            .find(|&id| entries.is_dir(id))
            .expect("the subfolder") as u32;
        let target = ListingTarget::Entry {
            listing: id,
            entry: sub,
        };
        let subfolder = listings.resolve(target).expect("entry resolves");
        assert_eq!(
            subfolder,
            long_path(&dir.path().join("ქვესაქაღალდე")).expect("path")
        );
        // The icon protocol finds entries the same way.
        let (path, attributes, modified) = listings.entry(id, sub).expect("entry");
        assert_eq!((path, attributes & 0x10), (subfolder, 0x10));
        assert_eq!(modified, entries.modified()[sub as usize]);
        assert!(listings.entry(id, 4).is_none());
        assert!(listings.entry(id + 1, 0).is_none());
    }

    #[test]
    fn errors_reading_the_folder_come_through_the_stream() {
        let dir = tempfile::tempdir().expect("temp folder");
        let listings = Listings::default();
        let (channel, receiver) = test_channel();

        let missing = listings
            .resolve(path_target(&dir.path().join("აღარ არის")))
            .expect("resolves");
        let id = listings.open_folder(1, missing.clone(), Sort::default(), channel, false);
        let messages = until_end(&receiver);

        assert_eq!(messages.len(), 1);
        let json: serde_json::Value = serde_json::from_slice(&messages[0][16..]).expect("JSON");
        assert_eq!(json["code"], "NotFound");
        assert_eq!(listings.0.watchers.watched(), 0, "nothing to watch");

        // Retry: the failed listing's folder opens again by its exact path.
        let again = listings.resolve(ListingTarget::Listing { listing: id });
        assert_eq!(again.expect("the listing is still open"), missing);
    }

    #[test]
    fn opened_folders_get_names_and_tokens_that_open_them_again() {
        let dir = tempfile::tempdir().expect("temp folder");
        let sub = dir.path().join("ნახაზები");
        std::fs::create_dir(&sub).expect("mkdir");
        let listings = Listings::default();
        let folder = long_path(&sub).expect("long path");

        let opened = listings.opened(7, &folder);
        assert_eq!(opened.id, 7);
        assert_eq!(opened.name, "ნახაზები");
        assert_eq!(opened.path, display_path(&folder));
        // The same folder, the same token; it resolves to the exact path even with nothing open.
        assert_eq!(listings.opened(8, &folder).folder, opened.folder);
        let again = listings.resolve(ListingTarget::Folder {
            folder: opened.folder,
        });
        assert_eq!(again.expect("a known token"), folder);
        let parent = opened.parent.expect("a temp folder has a parent");
        let up = listings.resolve(ListingTarget::Folder { folder: parent });
        assert_eq!(
            up.expect("the parent's token"),
            long_path(dir.path()).expect("path")
        );

        let unknown = listings.resolve(ListingTarget::Folder { folder: 9_999 });
        assert_eq!(
            unknown.expect_err("no such token").code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn roots_are_named_by_themselves_and_have_no_parent() {
        let listings = Listings::default();
        let drive = listings.opened(1, Path::new(r"\\?\C:\"));
        assert_eq!((drive.name.as_str(), drive.parent), (r"C:\", None));
        let share = listings.opened(2, Path::new(r"\\?\UNC\server\share\"));
        assert_eq!(share.name, r"\\server\share\");
        assert_eq!(share.parent, None);
        let folder = listings.opened(3, Path::new(r"\\?\UNC\server\share\Projects"));
        assert_eq!(folder.name, "Projects");
        assert_eq!(folder.parent, Some(share.folder));
    }

    #[test]
    fn crumbs_run_from_the_root_with_tokens_that_open_each_folder() {
        let listings = Listings::default();
        let crumbs = |path: &str| -> Vec<(String, PathBuf)> {
            listings
                .opened(1, Path::new(path))
                .crumbs
                .into_iter()
                .map(|crumb| {
                    let path = listings.folder_path(crumb.folder).expect("a known token");
                    (crumb.name, path)
                })
                .collect()
        };
        assert_eq!(
            crumbs(r"\\?\D:\Projects\ნახაზები"),
            [
                ("D:".to_owned(), PathBuf::from(r"\\?\D:\")),
                ("Projects".to_owned(), PathBuf::from(r"\\?\D:\Projects")),
                (
                    "ნახაზები".to_owned(),
                    PathBuf::from(r"\\?\D:\Projects\ნახაზები")
                ),
            ]
        );
        let share = crumbs(r"\\?\UNC\server\share\Docs");
        let names: Vec<&str> = share.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, [r"\\server\share", "Docs"]);
    }

    #[test]
    fn home_is_the_users_own_folder() {
        let home = Listings::default()
            .resolve(ListingTarget::Home)
            .expect("the profile folder");
        assert_eq!(
            home,
            long_path(&jerry_win::folders::profile().expect("profile")).expect("path")
        );
    }

    #[test]
    fn folders_read_to_the_end_count_as_visits() {
        let dir = folder_with(&["a.txt"]);
        let (visited, visits) = channel();
        let listings = Listings::new(move |folder| {
            let _ = visited.send(folder.to_path_buf());
        });
        let folder = long_path(dir.path()).expect("long path");

        let (channel, receiver) = test_channel();
        listings.open_folder(1, folder.clone(), Sort::default(), channel, true);
        until_end(&receiver);
        let noted = visits.recv_timeout(Duration::from_secs(10));
        assert_eq!(noted.expect("a visit"), folder);

        // A refresh isn't a visit, nor is a folder that can't be read.
        let (channel, receiver) = test_channel();
        listings.open_folder(1, folder, Sort::default(), channel, false);
        until_end(&receiver);
        let (channel, receiver) = test_channel();
        let missing = long_path(&dir.path().join("აღარ არის")).expect("path");
        listings.open_folder(2, missing, Sort::default(), channel, true);
        until_end(&receiver);
        assert!(visits.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn targets_must_be_absolute_and_entries_must_exist() {
        let listings = Listings::default();
        let relative = ListingTarget::Path {
            path: r"Work\plans".into(),
        };
        let error = listings.resolve(relative).expect_err("relative");
        assert_eq!(error.code, ErrorCode::InvalidName);
        let stale = ListingTarget::Entry {
            listing: 99,
            entry: 0,
        };
        let error = listings.resolve(stale).expect_err("no such listing");
        assert_eq!(error.code, ErrorCode::NotFound);
        let closed = ListingTarget::Listing { listing: 99 };
        let error = listings.resolve(closed).expect_err("no such listing");
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn a_tab_has_one_listing_and_closed_ones_are_gone() {
        let dir = folder_with(&["a.txt"]);
        let listings = Listings::default();
        let (first_channel, first) = test_channel();
        let (second_channel, second) = test_channel();
        let folder = long_path(dir.path()).expect("long path");

        let old = listings.open_folder(1, folder.clone(), Sort::default(), first_channel, false);
        until_end(&first);
        let new = listings.open_folder(1, folder.clone(), Sort::default(), second_channel, false);
        until_end(&second);

        assert_ne!(old, new);
        let entry = |listing| ListingTarget::Entry { listing, entry: 0 };
        assert!(listings.resolve(entry(old)).is_err(), "replaced");
        assert!(listings.resolve(entry(new)).is_ok());
        assert_eq!(listings.0.watchers.watched(), 1);
        listings.close(new);
        listings.close(new);
        assert!(listings.resolve(entry(new)).is_err(), "closed");
        assert_eq!(listings.0.watchers.watched(), 0, "the watcher stopped");
        let error = listings.sort(new, BY_SIZE).expect_err("closed");
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn sorting_a_listed_folder_sends_only_the_new_order() {
        // Each file holds its own name, so the longer name is the bigger file.
        let dir = folder_with(&["a.txt", "ccc.txt", "bb.txt"]);
        std::fs::create_dir(dir.path().join("ფოლდერი")).expect("mkdir");
        let listings = Listings::default();
        let (channel, receiver) = test_channel();
        let folder = long_path(dir.path()).expect("long path");
        let id = listings.open_folder(1, folder, Sort::default(), channel, false);
        until_end(&receiver);

        listings.sort(id, BY_SIZE).expect("sorts");

        let order = next(&receiver);
        assert_eq!([word(&order, 8), word(&order, 12)], [3, 0b11]);
        assert_eq!(
            ordered_names(&listings, id, &order),
            ["ფოლდერი", "ccc.txt", "bb.txt", "a.txt"]
        );
        assert!(receiver.try_recv().is_err(), "nothing more on the stream");
    }

    #[test]
    fn a_sort_while_reading_goes_into_the_streams_order() {
        let dir = folder_with(&["a.txt", "ccc.txt", "bb.txt"]);
        let folder = long_path(dir.path()).expect("long path");
        // A listing whose job hasn't run yet, as `start` makes it.
        let listings = Listings::default();
        let (channel, receiver) = test_channel();
        let state = Arc::new(ListingState::new(folder, Sort::default(), Some(channel)));
        listings.open().insert(
            7,
            Open {
                tab: 1,
                state: state.clone(),
            },
        );

        listings.sort(7, BY_SIZE).expect("sorts");
        assert!(
            receiver.try_recv().is_err(),
            "no order before the folder is in"
        );
        run(&state, &listings).expect("lists");
        let messages = until_end(&receiver);

        let order = &messages[messages.len() - 2];
        assert_eq!([word(order, 8), word(order, 12)], [3, 0b11]);
        assert_eq!(
            ordered_names(&listings, 7, order),
            ["ccc.txt", "bb.txt", "a.txt"]
        );
    }

    #[test]
    fn a_prestarted_listing_keeps_its_messages_for_the_ui() {
        let dir = folder_with(&["ნახაზი 1.dwg", "ნახაზი 2.dwg"]);
        let listings = Listings::default();
        let id = listings
            .prestart(3, dir.path(), Sort::default())
            .expect("starts");

        // Let it finish with nobody listening.
        let wait = Instant::now();
        while state(&listings, id).live().index.is_none() {
            assert!(wait.elapsed() < Duration::from_secs(10), "the listing ends");
            std::thread::sleep(Duration::from_millis(5));
        }
        let (channel, receiver) = test_channel();
        let folder = long_path(dir.path()).expect("long path");
        let attached = listings.open_folder(3, folder.clone(), Sort::default(), channel, false);
        let messages = until_end(&receiver);

        assert_eq!(
            attached, id,
            "the UI gets the listing that was already running"
        );
        assert_eq!(word(&messages[0], 0), BATCH);
        assert_eq!(word(&messages[messages.len() - 2], 0), ORDER);
        assert_eq!(word(messages.last().expect("done"), 4), 2);

        // A tab sorted otherwise gets a listing of its own.
        let again = listings
            .prestart(4, dir.path(), Sort::default())
            .expect("starts");
        let other = listings.open_folder(4, folder, BY_SIZE, test_channel().0, false);
        assert_ne!(other, again);
    }

    /// The ids and removed ids of a diff message.
    fn diff_ids(message: &[u8]) -> (Vec<u32>, Vec<u32>) {
        assert_eq!(word(message, 0), DIFF);
        let (count, removed) = (word(message, 4) as usize, word(message, 8) as usize);
        let ids = (0..count).map(|i| word(message, 16 + i * 4)).collect();
        let gone = (0..removed)
            .map(|i| word(message, 16 + (count + i) * 4))
            .collect();
        (ids, gone)
    }

    #[test]
    fn changes_in_the_folder_come_as_a_diff_and_an_order() {
        let dir = folder_with(&["b.txt", "plan.dwg"]);
        let listings = Listings::default();
        let (channel, receiver) = test_channel();
        let folder = long_path(dir.path()).expect("long path");
        let id = listings.open_folder(1, folder, Sort::default(), channel, false);
        until_end(&receiver);
        let plan = state(&listings, id)
            .read()
            .live_ids()
            .into_iter()
            .find(|&entry| state(&listings, id).read().entries().name(entry as usize) == "plan.dwg")
            .expect("plan.dwg");

        std::fs::write(dir.path().join("a.txt"), b"new").expect("write");
        std::fs::rename(dir.path().join("plan.dwg"), dir.path().join("გეგმა.dwg")).expect("rename");
        std::fs::remove_file(dir.path().join("b.txt")).expect("delete");

        // Until the stream shows all three: they may come in one diff or several.
        let mut changed = BTreeSet::new();
        let mut gone = BTreeSet::new();
        let mut last_order = Vec::new();
        while gone.is_empty() || changed.len() < 2 {
            let diff = next(&receiver);
            let (ids, removed) = diff_ids(&diff);
            assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "ascending");
            changed.extend(ids);
            gone.extend(removed);
            last_order = next(&receiver);
            assert_eq!(word(&last_order, 0), ORDER, "an order follows every diff");
        }
        assert!(changed.contains(&plan), "the renamed file kept its id");
        assert_eq!(gone.len(), 1);
        assert_eq!(
            ordered_names(&listings, id, &last_order),
            ["a.txt", "გეგმა.dwg"]
        );
        // The removed entry no longer opens and has no icon.
        let gone = *gone.first().expect("one");
        let removed = ListingTarget::Entry {
            listing: id,
            entry: gone,
        };
        assert!(listings.resolve(removed).is_err());
        assert!(listings.entry(id, gone).is_none());
    }

    #[test]
    fn a_deleted_folder_ends_the_stream_with_not_found() {
        let dir = tempfile::tempdir().expect("temp folder");
        let watched = dir.path().join("ნახაზები");
        std::fs::create_dir(&watched).expect("mkdir");
        let listings = Listings::default();
        let (channel, receiver) = test_channel();
        listings.open_folder(
            1,
            long_path(&watched).expect("long path"),
            Sort::default(),
            channel,
            false,
        );
        until_end(&receiver);

        std::fs::remove_dir(&watched).expect("a watched folder can be deleted");

        let messages = until_end(&receiver);
        let error = messages.last().expect("an error");
        let json: serde_json::Value = serde_json::from_slice(&error[16..]).expect("JSON");
        assert_eq!(json["code"], "NotFound");
    }

    /// The error code of an error message.
    fn error_code(message: &[u8]) -> String {
        assert_eq!(word(message, 0), ERROR);
        let json: serde_json::Value = serde_json::from_slice(&message[16..]).expect("JSON");
        json["code"].as_str().expect("a code").to_owned()
    }

    fn root_of(folder: &Path) -> PathBuf {
        volume::root(&long_path(folder).expect("long path")).expect("a root")
    }

    #[test]
    fn a_released_volume_holds_nothing_and_a_kept_one_is_live_again() {
        let dir = tempfile::tempdir().expect("temp folder");
        let parent = dir.path().join("USB");
        let watched = parent.join("ნახაზები");
        std::fs::create_dir_all(&watched).expect("mkdir");
        std::fs::write(watched.join("a.dwg"), b"a").expect("write");
        let listings = Listings::default();
        let (channel, receiver) = test_channel();
        let folder = long_path(&watched).expect("long path");
        listings.open_folder(1, folder, Sort::default(), channel, false);
        until_end(&receiver);
        let root = root_of(&watched);
        let moved = dir.path().join("USB (moved)");
        assert!(std::fs::rename(&parent, &moved).is_err(), "held while live");

        // Windows asks for the volume: once `release` returns, Jerry holds nothing on it.
        listings.release(&root);
        std::fs::rename(&parent, &moved).expect("nothing holds the folder");
        std::fs::rename(&moved, &parent).expect("and back");
        // No listing on it starts meanwhile.
        let (refused_channel, refused) = test_channel();
        let elsewhere = long_path(dir.path()).expect("long path");
        listings.open_folder(
            2,
            elsewhere.clone(),
            Sort::default(),
            refused_channel,
            false,
        );
        let messages = until_end(&refused);
        assert_eq!(messages.len(), 1);
        assert_eq!(error_code(&messages[0]), "DeviceGone");

        // The removal failed: the list is live again, with what changed meanwhile.
        std::fs::write(watched.join("b.dwg"), b"b").expect("write");
        listings.keep(&root);
        let (ids, removed) = diff_ids(&next(&receiver));
        assert_eq!((ids.len(), removed.len()), (1, 0), "b.dwg is new");
        assert_eq!(word(&next(&receiver), 0), ORDER);
        assert!(std::fs::rename(&parent, &moved).is_err(), "held again");
        let (channel, again) = test_channel();
        listings.open_folder(2, elsewhere, Sort::default(), channel, false);
        assert_eq!(word(until_end(&again).last().expect("done"), 0), DONE);
    }

    #[test]
    fn a_volume_that_goes_ends_its_listings_with_device_gone() {
        let dir = folder_with(&["ნახაზი.dwg"]);
        let listings = Listings::default();
        let (channel, receiver) = test_channel();
        let folder = long_path(dir.path()).expect("long path");
        listings.open_folder(1, folder, Sort::default(), channel, false);
        until_end(&receiver);
        let root = root_of(dir.path());

        listings.release(&root);
        listings.removed(&root);

        let messages = until_end(&receiver);
        assert_eq!(messages.len(), 1);
        assert_eq!(error_code(&messages[0]), "DeviceGone");
        let volumes = listings.0.volumes();
        let volume = volumes.get(&root).expect("its listing is still open");
        assert!(!volume.released && volume.notices.is_none());
    }

    #[test]
    fn a_listing_still_reading_when_its_volume_is_released_ends_with_device_gone() {
        let dir = folder_with(&["a.txt"]);
        let listings = Listings::default();
        let (channel, receiver) = test_channel();
        let folder = long_path(dir.path()).expect("long path");
        let state = Arc::new(ListingState::new(folder, Sort::default(), Some(channel)));

        // Windows asks for the volume as the job is about to read.
        assert!(state.stop_reading(), "the job runs");
        let result = run(&state, &listings);
        drop(JobDone(&state));
        assert_eq!(
            result.as_ref().map_err(|error| error.code.clone()),
            Err(ErrorCode::Cancelled)
        );
        state.finish(1, result);

        let messages = until_end(&receiver);
        assert_eq!(messages.len(), 1, "no batch: the folder wasn't opened");
        assert_eq!(error_code(&messages[0]), "DeviceGone");
        assert!(state.wait_job(Instant::now()));
        assert!(!state.stop_reading(), "done: nothing to stop");
    }

    #[test]
    fn a_volume_is_forgotten_with_its_last_listing() {
        let dir = folder_with(&["a.txt"]);
        let listings = Listings::default();
        let (channel, receiver) = test_channel();
        let folder = long_path(dir.path()).expect("long path");
        let id = listings.open_folder(1, folder, Sort::default(), channel, false);
        until_end(&receiver);
        assert_eq!(listings.0.volumes().len(), 1);

        listings.close(id);
        let wait = Instant::now();
        while !listings.0.volumes().is_empty() {
            assert!(wait.elapsed() < Duration::from_secs(10), "forgotten");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn nothing_is_sent_after_close() {
        let (channel, receiver) = test_channel();
        let outbox = Outbox::new(Some(channel));
        outbox.send(done_message(0, 0, 0.0));
        outbox.close();
        outbox.send(done_message(0, 0, 0.0));
        outbox.attach(test_channel().0);
        assert_eq!(receiver.try_iter().count(), 1);
        assert!(!outbox.is_attached());
    }
}
