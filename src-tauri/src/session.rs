//! The session (#42): the main window's place, its panes, their tabs and each tab's history, so
//! the next start opens them again, after a crash too. The stored form is
//! `jerry_core::session::Session`, with exact paths; the UI sees folders only as tokens.
//!
//! - At process start, before Tauri, the `jerry-session` thread waits for the database and reads
//!   the session, so the main window takes its place from it before it shows
//!   ([`Sessions::placement_within`], [`place`]). Then it decides what to open (the previous
//!   session, or the start folder from the settings; the user's own folder at a first start) and
//!   starts listing each pane's shown folder (`Listings::prestart`), in parallel with WebView2.
//! - The page asks what to open with `session_restore`: the tabs with their ids (the prestarted
//!   listings' tabs), their histories as folder tokens, and the look each shows its folder with.
//!   A restored tab whose folder went away opens the nearest folder that still exists
//!   (`ListingTarget::Nearest`); the page says so.
//! - The page sends its panes and tabs with `session_save` whenever they change; the window's
//!   place comes from its move and size events ([`Sessions::placed`]). The same thread writes the
//!   session one second after the last change, and [`Sessions::flush`] writes it at exit.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use jerry_core::JerryError;
use jerry_core::address::{self, Context};
use jerry_core::job::run_job;
use jerry_core::path::long_path;
use jerry_core::session::{
    ColumnLayout, ExactPath, FolderView, PaneSide, Placement, Session, SessionFolder, SessionPane,
    SessionTab, SessionWindow,
};
use jerry_core::settings::{Settings, Startup};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, WebviewWindow, Window};

use crate::jobs;
use crate::listing::{Crumb, Listings};
use crate::storage::{Db, now_ms};
use crate::views::{self, Look};
use crate::window::MAIN;

/// How long after the last change the session is written.
const DEBOUNCE: Duration = Duration::from_secs(1);

/// What a window opens: its panes and their tabs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionRestore {
    /// The pane the keyboard acts on.
    pub active_pane: PaneSide,
    pub left: PaneRestore,
    pub right: PaneRestore,
}

/// A pane's tabs, pinned ones first; at least one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct PaneRestore {
    pub tabs: Vec<TabRestore>,
    /// The tab the pane shows: its place in `tabs`.
    pub active: u32,
}

/// A tab to open again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct TabRestore {
    /// The tab's id. Each pane's shown tab may have its listing started already, for this id,
    /// its folder and its look's sort.
    pub id: u32,
    pub pinned: bool,
    /// Oldest first; at least one folder.
    pub history: Vec<HistoryFolder>,
    /// The folder the tab shows: its place in `history`.
    pub index: u32,
    /// How the tab shows its folder. The tab takes it before it opens the folder.
    pub look: Look,
}

/// A folder in a restored tab's history: as `ListingOpened` names it, and its focused entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct HistoryFolder {
    /// The folder as display text.
    pub path: String,
    /// The folder's name for a tab.
    pub name: String,
    /// The folder, as a token for `ListingTarget::Folder`.
    pub folder: u32,
    /// The folder it is in, as such a token; none at a root.
    pub parent: Option<u32>,
    /// The address bar's segments, from the root to the folder.
    pub crumbs: Vec<Crumb>,
    /// The entry that had the focus when the tab left the folder, by name.
    pub focus: Option<String>,
}

/// A window's panes and tabs, as the page sends them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionSave {
    pub active_pane: PaneSide,
    pub left: PaneSave,
    pub right: PaneSave,
}

/// A pane's tabs, pinned ones first.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
pub struct PaneSave {
    pub tabs: Vec<TabSave>,
    /// The tab the pane shows: its place in `tabs`.
    pub active: u32,
}

/// A tab: its history and its own view.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
pub struct TabSave {
    pub pinned: bool,
    /// Oldest first.
    pub history: Vec<FolderSave>,
    /// The folder the tab shows: its place in `history`.
    pub index: u32,
    /// The view, sort and columns the tab shows its folder with.
    pub view: FolderView,
}

/// A folder in a tab's history.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
pub struct FolderSave {
    /// The folder's token (`ListingOpened::folder`).
    pub folder: u32,
    /// The entry that had the focus when the tab left the folder, by name.
    pub focus: Option<String>,
}

/// The session in the app: what the main window opens and the latest state, written a second
/// after it changes. Clones share it.
#[derive(Clone)]
pub struct Sessions(Arc<Inner>);

struct Inner {
    db: Db,
    listings: Listings,
    kept: Mutex<Kept>,
    /// Signalled on every change to `kept`.
    changed: Condvar,
    /// Held across a write, so an older session never lands after a newer one.
    writing: Mutex<()>,
}

#[derive(Default)]
struct Kept {
    /// Set once the stored session is read: the main window's place, if it had one.
    placement: Option<Option<Placement>>,
    /// The latest session: chosen at start, then changed by every save and window move.
    session: Option<Session>,
    /// What the main window opens first, with its listings started; its first `session_restore`
    /// takes it.
    first: Option<SessionRestore>,
    /// Set once `session` and `first` are.
    ready: bool,
    /// Counts changes; `saved` is the one last written.
    changes: u64,
    saved: u64,
    last_change: Option<Instant>,
    exiting: bool,
}

impl Sessions {
    /// Starts the `jerry-session` thread: it reads the session once the database is open, starts
    /// the listings of the folders it shows, then writes the session a second after each change.
    pub fn start(db: Db, listings: Listings) -> Self {
        let sessions = Self(Arc::new(Inner {
            db,
            listings,
            kept: Mutex::new(Kept::default()),
            changed: Condvar::new(),
            writing: Mutex::new(()),
        }));
        let thread = sessions.clone();
        let started = std::thread::Builder::new()
            .name("jerry-session".into())
            .spawn(move || {
                thread.prepare();
                thread.write_until_exit();
            });
        if let Err(error) = started {
            tracing::error!(%error, "can't start the session thread; opening the user's folder");
            sessions.fall_back();
        }
        sessions
    }

    /// The main window's place from the stored session, if the session is read within `timeout`.
    pub fn placement_within(&self, timeout: Duration) -> Option<Placement> {
        let kept = self.wait_for(timeout, |kept| kept.placement.is_some());
        kept.placement.flatten()
    }

    /// What window `main` (the main one) or another window opens. The main window gets the
    /// session, at first with its listings started; another window gets the user's own folder
    /// until windows have sessions of their own (#89). Blocks until the session is read: call it
    /// from a background job.
    pub fn restore(&self, main: bool) -> SessionRestore {
        let mut kept = self.wait_for(Duration::MAX, |kept| kept.ready);
        if !main {
            drop(kept);
            let session = Session::at(&home(), self.settings().default_view);
            return self.restore_window(&session.windows[0], &self.settings(), false);
        }
        if let Some(first) = kept.first.take() {
            return first;
        }
        // The page loaded again (a reload while developing): the latest state.
        let window = kept
            .session
            .as_ref()
            .and_then(|session| session.windows.first().cloned());
        drop(kept);
        let settings = self.settings();
        let window = window.unwrap_or_else(|| {
            let mut session = Session::at(&home(), settings.default_view);
            session.windows.swap_remove(0)
        });
        self.restore_window(&window, &settings, false)
    }

    /// Takes the main window's panes and tabs from the page. Folders the core doesn't know are
    /// left out; a tab left without folders goes.
    pub fn save(&self, save: SessionSave) {
        let listings = &self.0.listings;
        let pane = |pane: PaneSave| {
            let shown = pane.active as usize;
            let mut active = 0;
            let mut tabs = Vec::with_capacity(pane.tabs.len());
            for (at, tab) in pane.tabs.into_iter().enumerate() {
                if at == shown {
                    active = tabs.len();
                }
                if let Some(tab) = stored_tab(listings, tab) {
                    tabs.push(tab);
                }
            }
            SessionPane {
                active: active.min(tabs.len().saturating_sub(1)),
                tabs,
            }
        };
        let (left, right) = (pane(save.left), pane(save.right));
        self.change(|session| {
            let window = main_window(session);
            let next = SessionWindow {
                placement: window.placement,
                active_pane: save.active_pane,
                left,
                right,
            };
            let changed = *window != next;
            *window = next;
            changed
        });
    }

    /// Notes where the main window is now: it moved or changed size.
    pub fn placed(&self, window: &Window) {
        let placement = window
            .hwnd()
            .map_err(|error| tracing::warn!(%error, "no window handle"))
            .ok()
            .and_then(|hwnd| {
                jerry_win::placement::get(hwnd.0 as isize)
                    .map_err(|error| tracing::warn!(%error, "can't read the window's place"))
                    .ok()
            });
        let Some(placement) = placement else {
            return;
        };
        self.change(|session| {
            let window = main_window(session);
            let changed = window.placement != Some(placement);
            window.placement = Some(placement);
            changed
        });
    }

    /// Writes the session now if it changed since it was last written, and stops the writer: at
    /// exit.
    pub fn flush(&self) {
        self.kept().exiting = true;
        self.0.changed.notify_all();
        self.write();
    }

    /// Reads the session and starts its listings; on failure, the user's own folder.
    fn prepare(&self) {
        let prepared = run_job("session", || {
            let (settings, stored) = self.0.db.with(|storage| {
                let stored = storage.session().unwrap_or_else(|error| {
                    tracing::warn!(code = ?error.code, "can't read the session");
                    None
                });
                Ok((storage.settings().clone(), stored))
            })?;
            {
                let mut kept = self.kept();
                kept.placement = Some(
                    stored
                        .as_ref()
                        .and_then(|session| session.windows.first())
                        .and_then(|window| window.placement),
                );
            }
            self.0.changed.notify_all();
            let started = Instant::now();
            let session = chosen(&settings, stored);
            let first = self.restore_window(&session.windows[0], &settings, true);
            tracing::info!(
                ms = started.elapsed().as_secs_f64() * 1000.0,
                tabs = first.left.tabs.len() + first.right.tabs.len(),
                "session ready"
            );
            Ok((session, first))
        });
        match prepared {
            Ok((session, first)) => self.ready(session, first),
            Err(_) => self.fall_back(),
        }
    }

    /// The user's own folder in both panes, when the session couldn't be prepared.
    fn fall_back(&self) {
        let session = Session::at(&home(), Settings::default().default_view);
        let first = self.restore_window(&session.windows[0], &Settings::default(), false);
        self.ready(session, first);
    }

    fn ready(&self, session: Session, first: SessionRestore) {
        let mut kept = self.kept();
        kept.placement.get_or_insert(None);
        kept.session = Some(session);
        kept.first = Some(first);
        kept.ready = true;
        drop(kept);
        self.0.changed.notify_all();
    }

    /// `window` as the page opens it: tab ids from 1, each tab's history as tokens, and the look
    /// each tab shows its folder with. With `prestart`, each pane's shown folder starts listing
    /// now, sorted by its look, for the page to attach to.
    fn restore_window(
        &self,
        window: &SessionWindow,
        settings: &Settings,
        prestart: bool,
    ) -> SessionRestore {
        let Inner { db, listings, .. } = &*self.0;
        let folders_first = settings.folders_first;
        let mut next_id = 0;
        let mut pane = |pane: &SessionPane| {
            let tabs = pane
                .tabs
                .iter()
                .enumerate()
                .map(|(at, tab)| {
                    next_id += 1;
                    let shown = &tab.history[tab.index].path.0;
                    // The folder's view while views are remembered per folder; the tab's own
                    // otherwise, or if the folder's can't be read (the listing must start with
                    // the sort the page opens it with).
                    let look = match settings
                        .remember_folder_views
                        .then(|| views::look(db, shown, folders_first))
                    {
                        Some(Look::Tab) | None => Look::saved(tab.view.clone(), folders_first),
                        Some(look) => look,
                    };
                    if prestart && at == pane.active {
                        let sort = look.sort(tab.view.sort);
                        if let Err(error) = listings.prestart(next_id, shown, sort) {
                            tracing::warn!(code = ?error.code, "can't start a restored listing");
                        }
                    }
                    TabRestore {
                        id: next_id,
                        pinned: tab.pinned,
                        history: tab
                            .history
                            .iter()
                            .map(|folder| history_folder(listings, folder))
                            .collect(),
                        index: tab.index as u32,
                        look,
                    }
                })
                .collect();
            PaneRestore {
                tabs,
                active: pane.active as u32,
            }
        };
        let left = pane(&window.left);
        let right = pane(&window.right);
        SessionRestore {
            active_pane: window.active_pane,
            left,
            right,
        }
    }

    /// Changes the session with `change`, which says whether anything changed; if it did, the
    /// session is written a second later. Before the session is read there is nothing to change.
    fn change(&self, change: impl FnOnce(&mut Session) -> bool) {
        let mut kept = self.kept();
        let Some(session) = kept.session.as_mut() else {
            return;
        };
        if change(session) {
            kept.changes += 1;
            kept.last_change = Some(Instant::now());
            drop(kept);
            self.0.changed.notify_all();
        }
    }

    /// Writes the session a second after the last change, until exit.
    fn write_until_exit(&self) {
        loop {
            {
                let mut kept = self.kept();
                loop {
                    if kept.exiting {
                        return;
                    }
                    if kept.changes != kept.saved {
                        let due = kept.last_change.map(|at| at + DEBOUNCE);
                        match due.and_then(|due| due.checked_duration_since(Instant::now())) {
                            Some(left) if !left.is_zero() => {
                                kept = self.wait(kept, left);
                            }
                            _ => break,
                        }
                    } else {
                        kept = self.wait(kept, Duration::MAX);
                    }
                }
            }
            self.write();
        }
    }

    /// Writes the session if it changed since it was last written.
    fn write(&self) {
        let _writing = self
            .0
            .writing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (session, change) = {
            let kept = self.kept();
            match &kept.session {
                Some(session) if kept.changes != kept.saved => (session.clone(), kept.changes),
                _ => return,
            }
        };
        let started = Instant::now();
        let saved = self
            .0
            .db
            .with(|storage| storage.save_session(&session, now_ms()));
        match saved {
            Ok(()) => tracing::debug!(
                ms = started.elapsed().as_secs_f64() * 1000.0,
                "session saved"
            ),
            // The next change tries again.
            Err(error) => tracing::warn!(code = ?error.code, "can't save the session"),
        }
        let mut kept = self.kept();
        kept.saved = kept.saved.max(change);
    }

    fn settings(&self) -> Settings {
        self.0
            .db
            .with(|storage| Ok(storage.settings().clone()))
            .unwrap_or_default()
    }

    fn kept(&self) -> MutexGuard<'_, Kept> {
        self.0.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, kept: MutexGuard<'a, Kept>, timeout: Duration) -> MutexGuard<'a, Kept> {
        if timeout == Duration::MAX {
            return self
                .0
                .changed
                .wait(kept)
                .unwrap_or_else(PoisonError::into_inner);
        }
        self.0
            .changed
            .wait_timeout(kept, timeout)
            .unwrap_or_else(PoisonError::into_inner)
            .0
    }

    /// The kept state once `done` holds for it, or when `timeout` passes.
    fn wait_for(&self, timeout: Duration, done: impl Fn(&Kept) -> bool) -> MutexGuard<'_, Kept> {
        let deadline = Instant::now().checked_add(timeout);
        let mut kept = self.kept();
        while !done(&kept) {
            let left = match deadline {
                Some(deadline) => match deadline.checked_duration_since(Instant::now()) {
                    Some(left) if !left.is_zero() => left,
                    _ => break,
                },
                None => Duration::MAX,
            };
            kept = self.wait(kept, left);
        }
        kept
    }
}

/// What this start opens: the previous session, or the start folder from the settings (keeping
/// the window's place); the user's own folder at a first start or for a pane left without tabs.
fn chosen(settings: &Settings, stored: Option<Session>) -> Session {
    let view = settings.default_view;
    let mut session = match &settings.startup {
        Startup::PreviousSession => stored
            .filter(|session| !session.windows.is_empty())
            .unwrap_or_else(|| Session::at(&home(), view)),
        Startup::Folder { path } => {
            let folder = start_folder(path).unwrap_or_else(|| {
                tracing::warn!("the start folder from the settings isn't a path; using home");
                home()
            });
            let mut session = Session::at(&folder, view);
            session.windows[0].placement = stored
                .and_then(|stored| stored.windows.into_iter().next())
                .and_then(|window| window.placement);
            session
        }
    };
    // v0.1 has one window (#89 brings more).
    session.windows.truncate(1);
    let window = &mut session.windows[0];
    for pane in [&mut window.left, &mut window.right] {
        if pane.tabs.is_empty() {
            *pane = Session::at(&home(), view).windows.swap_remove(0).left;
        }
    }
    session
}

/// The folder the start folder setting names, as the address bar reads text (`%VARIABLES%`, a
/// known folder's short name such as `Downloads`), without looking at the disk: a folder that
/// went away opens its nearest parent like any restored tab.
fn start_folder(text: &str) -> Option<PathBuf> {
    let text = address::clean(text);
    let var = |name: &str| std::env::var_os(name);
    let context = Context {
        base: None,
        known: &jerry_win::folders::by_short_name,
        var: &var,
    };
    let candidate = address::candidates(&text, &context).into_iter().next()?;
    candidate
        .is_absolute()
        .then(|| long_path(&candidate).ok())
        .flatten()
}

/// The user's own folder (`C:\Users\name`), as a long path.
fn home() -> PathBuf {
    jerry_win::folders::profile()
        .or_else(|_| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .ok_or_else(JerryError::internal)
        })
        .and_then(|home| long_path(&home))
        .unwrap_or_else(|_| PathBuf::from(r"\\?\C:\"))
}

/// The session's main window, made if there is none.
fn main_window(session: &mut Session) -> &mut SessionWindow {
    if session.windows.is_empty() {
        let home = Session::at(&home(), Settings::default().default_view);
        session.windows = home.windows;
    }
    &mut session.windows[0]
}

/// `tab` as the session keeps it: tokens as exact paths. `None` for a tab left without folders.
fn stored_tab(listings: &Listings, tab: TabSave) -> Option<SessionTab> {
    let shown = tab.index as usize;
    let mut index = 0;
    let mut history = Vec::with_capacity(tab.history.len());
    for (at, folder) in tab.history.into_iter().enumerate() {
        let Some(path) = listings.folder_path(folder.folder) else {
            tracing::warn!("a tab's history names a folder the core doesn't know");
            continue;
        };
        if at <= shown {
            index = history.len();
        }
        history.push(SessionFolder {
            path: ExactPath(path),
            focus: folder.focus,
        });
    }
    if history.is_empty() {
        return None;
    }
    let mut view = tab.view;
    if !view.columns.is_sane() {
        view.columns = ColumnLayout::default();
    }
    Some(SessionTab {
        pinned: tab.pinned,
        history,
        index,
        view,
    })
}

fn history_folder(listings: &Listings, folder: &SessionFolder) -> HistoryFolder {
    let entry = listings.entry_for(&folder.path.0);
    HistoryFolder {
        path: entry.path,
        name: entry.name,
        folder: entry.folder,
        parent: entry.parent,
        crumbs: entry.crumbs,
        focus: folder.focus.clone(),
    }
}

/// Puts the main window, still hidden, where the session had it: its restored rectangle, then
/// maximized if it was (Tauri shows it maximized). Call before the window shows.
pub fn place(window: &WebviewWindow, placement: &Placement) {
    let hwnd = match window.hwnd() {
        Ok(hwnd) => hwnd.0 as isize,
        Err(error) => {
            tracing::warn!(%error, "no window handle; the window opens where Tauri puts it");
            return;
        }
    };
    match jerry_win::placement::set_hidden(hwnd, placement) {
        Ok(true) => {
            if placement.maximized
                && let Err(error) = window.maximize()
            {
                tracing::warn!(%error, "can't maximize the window");
            }
        }
        Ok(false) => tracing::warn!(?placement, "ignoring a window place too small to be one"),
        Err(error) => tracing::warn!(%error, "can't put the window where it was"),
    }
}

/// What this window opens: for the main window, the session (the tabs of the previous start, or
/// the start folder from the settings); for another, the user's own folder. The main window's
/// first call gets the tabs whose listings started at process start.
#[tauri::command]
#[specta::specta]
pub async fn session_restore(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<SessionRestore, JerryError> {
    let sessions = app.state::<Sessions>().inner().clone();
    let main = window.label() == MAIN;
    jobs::background("session restore", move || Ok(sessions.restore(main))).await
}

/// Keeps this window's panes, tabs and their histories for the next start: written a second
/// after the last change, and at exit. Only the main window's are kept until windows have
/// sessions of their own (#89).
#[tauri::command]
#[specta::specta]
pub async fn session_save(app: AppHandle, window: WebviewWindow, session: SessionSave) {
    if window.label() == MAIN {
        app.state::<Sessions>().save(session);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use jerry_core::settings::View;
    use jerry_core::sort::SortColumn;
    use serde_json::json;

    use super::*;

    fn folder(path: &Path) -> PathBuf {
        long_path(path).expect("long path")
    }

    fn sessions_in(dir: &Path) -> (Sessions, Db, Listings) {
        let db = Db::open(Ok(dir.join("jerry.db")));
        let listings = Listings::default();
        (Sessions::start(db.clone(), listings.clone()), db, listings)
    }

    fn tab_save(listings: &Listings, paths: &[&Path], index: u32) -> TabSave {
        TabSave {
            pinned: false,
            history: paths
                .iter()
                .map(|path| FolderSave {
                    folder: listings.folder_token(path),
                    focus: None,
                })
                .collect(),
            index,
            view: FolderView::new(View::Details),
        }
    }

    #[test]
    fn a_first_start_opens_the_users_folder_in_both_panes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (sessions, _db, listings) = sessions_in(dir.path());
        let restore = sessions.restore(true);
        let home = listings.folder_token(&home());
        for pane in [&restore.left, &restore.right] {
            assert_eq!(pane.tabs.len(), 1);
            assert_eq!(pane.tabs[0].history[0].folder, home);
        }
        assert_eq!(restore.left.tabs[0].id, 1);
        assert_eq!(restore.right.tabs[0].id, 2);
        assert_eq!(restore.active_pane, PaneSide::Left);
        assert_eq!(sessions.placement_within(Duration::ZERO), None);
    }

    #[test]
    fn saved_tabs_come_back_after_a_restart_with_the_same_folders() {
        let dir = tempfile::tempdir().expect("temp dir");
        let work = tempfile::tempdir().expect("work dir");
        let drawings = folder(&work.path().join("ნახაზები"));
        let issued = folder(&work.path().join("ნახაზები").join("04_Issued"));
        std::fs::create_dir_all(&issued).expect("mkdir");
        {
            let (sessions, db, listings) = sessions_in(dir.path());
            db.with(|storage| storage.set_setting("rememberFolderViews", json!(false)))
                .expect("per-tab views");
            sessions.restore(true);
            let mut left = tab_save(&listings, &[&drawings, &issued], 0);
            left.history[1].focus = Some("plan.dwg".into());
            left.view.sort.column = SortColumn::Modified;
            let mut pinned = tab_save(&listings, &[&issued], 0);
            pinned.pinned = true;
            sessions.save(SessionSave {
                active_pane: PaneSide::Right,
                left: PaneSave {
                    tabs: vec![left],
                    active: 0,
                },
                right: PaneSave {
                    tabs: vec![pinned, tab_save(&listings, &[&drawings], 0)],
                    active: 1,
                },
            });
            sessions.flush();
        }

        let (sessions, _db, listings) = sessions_in(dir.path());
        let restore = sessions.restore(true);
        assert_eq!(restore.active_pane, PaneSide::Right);
        let left = &restore.left.tabs[0];
        assert_eq!(left.index, 0);
        assert_eq!(left.history.len(), 2);
        assert_eq!(left.history[0].folder, listings.folder_token(&drawings));
        assert_eq!(left.history[0].name, "ნახაზები");
        assert_eq!(left.history[1].focus.as_deref(), Some("plan.dwg"));
        match &left.look {
            Look::Saved { view } => assert_eq!(view.sort.column, SortColumn::Modified),
            other => panic!("the tab's own view, not {other:?}"),
        }
        assert!(restore.right.tabs[0].pinned);
        assert_eq!(restore.right.active, 1);
        let ids: Vec<u32> = [&restore.left, &restore.right]
            .iter()
            .flat_map(|pane| pane.tabs.iter().map(|tab| tab.id))
            .collect();
        assert_eq!(ids, [1, 2, 3]);
    }

    #[test]
    fn the_session_is_written_a_second_after_the_last_change() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (sessions, db, listings) = sessions_in(dir.path());
        sessions.restore(true);
        let somewhere = folder(dir.path());
        let save = SessionSave {
            active_pane: PaneSide::Left,
            left: PaneSave {
                tabs: vec![tab_save(&listings, &[&somewhere], 0)],
                active: 0,
            },
            right: PaneSave {
                tabs: vec![tab_save(&listings, &[&somewhere], 0)],
                active: 0,
            },
        };
        let saved = || {
            db.with(|storage| storage.session())
                .expect("reads")
                .map(|session| session.windows[0].left.tabs[0].history[0].path.0.clone())
        };
        let started = Instant::now();
        sessions.save(save);
        assert_ne!(saved(), Some(somewhere.clone()), "not at once");
        while saved() != Some(somewhere.clone()) {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "written in the end"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(started.elapsed() >= DEBOUNCE);
    }

    #[test]
    fn unknown_folders_drop_out_of_a_saved_history() {
        let dir = tempfile::tempdir().expect("temp dir");
        let listings = Listings::default();
        let known = folder(dir.path());
        let mut tab = tab_save(&listings, &[&known, &known], 1);
        tab.history.insert(
            0,
            FolderSave {
                folder: 999_999,
                focus: None,
            },
        );
        tab.index = 2;
        let stored = stored_tab(&listings, tab).expect("one folder is left");
        assert_eq!(stored.history.len(), 2);
        assert_eq!(stored.index, 1);

        let nothing = tab_save(&listings, &[], 0);
        assert_eq!(stored_tab(&listings, nothing), None);
    }

    #[test]
    fn the_start_folder_setting_opens_that_folder_and_keeps_the_windows_place() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut stored = Session::at(&folder(dir.path()), View::Details);
        stored.windows[0].placement = Some(Placement {
            left: 100,
            top: 100,
            right: 1100,
            bottom: 800,
            maximized: false,
        });
        let settings = Settings {
            startup: Startup::Folder {
                path: r"%SystemRoot%\Temp".into(),
            },
            ..Settings::default()
        };
        let session = chosen(&settings, Some(stored.clone()));
        let windows = std::env::var_os("SystemRoot").expect("SystemRoot");
        let temp = folder(&PathBuf::from(windows).join("Temp"));
        assert_eq!(session.windows[0].left.tabs[0].history[0].path.0, temp);
        assert_eq!(session.windows[0].right.tabs[0].history[0].path.0, temp);
        assert_eq!(session.windows[0].placement, stored.windows[0].placement);

        let previous = chosen(&Settings::default(), Some(stored.clone()));
        assert_eq!(previous, stored);
        assert_eq!(start_folder("relative\\path"), None);
    }

    #[test]
    fn a_restored_pane_starts_listing_its_shown_folder_for_the_page() {
        let dir = tempfile::tempdir().expect("temp dir");
        let work = tempfile::tempdir().expect("work dir");
        std::fs::write(work.path().join("a.txt"), b"a").expect("write");
        let shown = folder(work.path());
        let mut stored = Session::at(&shown, View::Details);
        let second = stored.windows[0].right.tabs[0].clone();
        stored.windows[0].right.tabs.insert(0, second);
        stored.windows[0].right.active = 1;
        let db = Db::open(Ok(dir.path().join("jerry.db")));
        db.with(|storage| storage.save_session(&stored, 1))
            .expect("saves");
        drop(db);

        let (sessions, _db, listings) = sessions_in(dir.path());
        let restore = sessions.restore(true);
        assert_eq!(restore.right.active, 1);
        // Tabs 1 (left) and 3 (the right pane's shown one) are listing; tab 2 waits.
        let prestarted = listings.prestarted_tabs();
        assert_eq!(prestarted, [1, 3]);
    }
}
