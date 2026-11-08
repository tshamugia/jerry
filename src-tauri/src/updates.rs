//! New versions of Jerry (#46): a check at start and once a day, and an install only when the user
//! asks for one. This is the only network Jerry uses; the `checkUpdates` setting turns it off.
//!
//! All of it runs here, through `tauri-plugin-updater`'s Rust API, so the page gets no updater
//! permission. It reads the offer with `update_offer`, hears `update:changed` and answers with
//! `update_install`.
//!
//! On Windows the plugin starts the downloaded NSIS installer and ends Jerry with
//! `std::process::exit`; the installer then starts the new Jerry, which restores the session.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use jerry_core::JerryError;
use jerry_core::job::run_job;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, Updater, UpdaterExt};
use tauri_specta::Event;

use crate::session::Sessions;
use crate::storage::Db;
use crate::{clipboard, jobs};

/// The first check waits this long after the start, so nothing competes with the first paint.
const FIRST_CHECK: Duration = Duration::from_secs(10);
/// Time between checks.
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the clock is read: a sleeping timer stops while the computer sleeps, the clock doesn't.
const TICK: Duration = Duration::from_secs(60 * 60);
/// How long a check may take. The download has no limit: it can be slow and still arrive.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the check waits for the settings before leaving it to the next tick.
const SETTINGS_WAIT: Duration = Duration::from_secs(5);
/// Progress goes to the page at most 10 times a second (CLAUDE.md, architecture rule 3).
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// How far the offered version got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum UpdateStage {
    /// Waiting for the user.
    Offered,
    /// Downloading.
    Downloading {
        /// Percent done; `None` while the size isn't known.
        percent: Option<u8>,
    },
    /// Downloaded and verified: the installer is starting, and Jerry exits for it.
    Installing,
    /// The download or the install failed. The offer stands, so the user can try again.
    Failed,
}

/// A newer version of Jerry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOffer {
    /// The version, for example `0.2.0`.
    pub version: String,
    /// The release notes, as Markdown.
    pub notes: String,
    pub stage: UpdateStage,
}

/// Sent to every window when the offer changes; `None` when there is none any more.
#[derive(Debug, Clone, Serialize, Type, Event)]
#[tauri_specta(event_name = "update:changed")]
pub struct UpdateChanged(pub Option<UpdateOffer>);

/// The offer, managed by the app.
#[derive(Default)]
pub struct Updates(Mutex<Current<Update>>);

/// The offer and the plugin's handle for installing it. Generic so tests need no real update.
#[derive(Debug)]
struct Current<U> {
    offer: Option<UpdateOffer>,
    handle: Option<U>,
}

impl<U> Default for Current<U> {
    fn default() -> Self {
        Self {
            offer: None,
            handle: None,
        }
    }
}

impl<U: Clone> Current<U> {
    /// Whether an install is under way: a check then changes nothing.
    fn busy(&self) -> bool {
        self.offer.as_ref().is_some_and(|offer| {
            matches!(
                offer.stage,
                UpdateStage::Downloading { .. } | UpdateStage::Installing
            )
        })
    }

    /// A check found a newer version. Whether the offer changed.
    fn found(&mut self, version: String, notes: String, handle: U) -> bool {
        if self.busy() {
            return false;
        }
        self.offer = Some(UpdateOffer {
            version,
            notes,
            stage: UpdateStage::Offered,
        });
        self.handle = Some(handle);
        true
    }

    /// A check found nothing newer: a release that was taken back is no longer offered. Whether
    /// the offer changed.
    fn nothing_new(&mut self) -> bool {
        if self.busy() || self.offer.is_none() {
            return false;
        }
        self.offer = None;
        self.handle = None;
        true
    }

    /// The user asked for the install: the handle to install from, unless one is under way
    /// already or there is nothing to install.
    fn begin(&mut self) -> Option<U> {
        if self.busy() {
            return None;
        }
        let handle = self.handle.clone()?;
        self.offer.as_mut()?.stage = UpdateStage::Downloading { percent: None };
        Some(handle)
    }
}

impl Updates {
    fn current(&self) -> MutexGuard<'_, Current<Update>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `change` and tells the windows if it changed the offer. Still locked while it
    /// sends, so windows hear the changes in order.
    fn change(&self, app: &AppHandle, change: impl FnOnce(&mut Current<Update>) -> bool) {
        let mut current = self.current();
        if change(&mut current)
            && let Err(error) = UpdateChanged(current.offer.clone()).emit(app)
        {
            tracing::error!(%error, "can't send update:changed");
        }
    }

    fn set_stage(&self, app: &AppHandle, stage: UpdateStage) {
        self.change(app, |current| match current.offer.as_mut() {
            Some(offer) => {
                offer.stage = stage;
                true
            }
            None => false,
        });
    }
}

/// Turns the download's chunks into at most 10 messages a second, and only for a new percent.
#[derive(Debug, Default)]
struct Progress {
    received: u64,
    percent: Option<u8>,
    sent_at: Option<Instant>,
}

impl Progress {
    /// A chunk of `bytes` arrived at `now`, of `total`: the stage to report, if it's time.
    fn chunk(&mut self, bytes: usize, total: Option<u64>, now: Instant) -> Option<UpdateStage> {
        self.received = self.received.saturating_add(bytes as u64);
        let percent = total
            .filter(|&total| total > 0)
            .map(|total| (u128::from(self.received) * 100 / u128::from(total)).min(100) as u8);
        let too_soon = self
            .sent_at
            .is_some_and(|sent| now.saturating_duration_since(sent) < PROGRESS_EVERY);
        if percent == self.percent || too_soon {
            return None;
        }
        self.percent = percent;
        self.sent_at = Some(now);
        Some(UpdateStage::Downloading { percent })
    }
}

/// Whether a check is due at `now`, the last one having been at `last`. A clock that went
/// backwards counts as due.
fn due(last: Option<SystemTime>, now: SystemTime) -> bool {
    last.is_none_or(|last| {
        now.duration_since(last)
            .map_or(true, |elapsed| elapsed >= CHECK_EVERY)
    })
}

/// Starts the checks on their own thread. Dev builds don't check, since they aren't releases and
/// developing shouldn't touch the network, unless `JERRY_CHECK_UPDATES` asks for it.
pub fn start(app: &AppHandle) {
    #[cfg(debug_assertions)]
    if std::env::var_os("JERRY_CHECK_UPDATES").is_none() {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("jerry-updates".into())
        .spawn(move || check_forever(&app));
    if let Err(error) = spawned {
        tracing::error!(%error, "can't start the update checks");
    }
}

fn check_forever(app: &AppHandle) {
    std::thread::sleep(FIRST_CHECK);
    let mut last = None;
    loop {
        let now = SystemTime::now();
        if due(last, now) && enabled(app) {
            last = Some(now);
            // A panic fails only this check; it's logged by the job.
            let _ = run_job("update check", || {
                tauri::async_runtime::block_on(check(app));
                Ok(())
            });
        }
        std::thread::sleep(TICK);
    }
}

/// Whether the user lets Jerry check. Unknown settings (the database isn't open yet) mean no:
/// the next tick asks again.
fn enabled(app: &AppHandle) -> bool {
    app.state::<Db>()
        .settings_within(SETTINGS_WAIT)
        .is_some_and(|settings| settings.check_updates)
}

async fn check(app: &AppHandle) {
    let found = match updater(app) {
        Ok(updater) => updater.check().await,
        Err(error) => Err(error),
    };
    let updates = app.state::<Updates>();
    match found {
        Ok(Some(update)) => {
            tracing::info!(version = %update.version, "a new version is available");
            let version = update.version.clone();
            let notes = update.body.clone().unwrap_or_default();
            updates.change(app, |current| current.found(version, notes, update));
        }
        Ok(None) => updates.change(app, Current::nothing_new),
        // Being offline is ordinary, and so is a repository with no stable release yet.
        Err(error) => tracing::info!(%error, "the update check didn't finish"),
    }
}

fn updater(app: &AppHandle) -> tauri_plugin_updater::Result<Updater> {
    let finishing = app.clone();
    app.updater_builder()
        .timeout(CHECK_TIMEOUT)
        // `/R` on its own restarts Jerry without this start's arguments. The plugin's restart
        // would repeat them: `--background` would hide the Jerry the user just asked to update,
        // and a path would open a second tab next to the restored one.
        .restart_after_install(false)
        .installer_arg("/R")
        .on_before_exit(move || before_exit(&finishing))
        .build()
}

/// The plugin is about to end Jerry with `std::process::exit`, so `RunEvent::Exit` never comes.
/// The session goes to disk and the clipboard stops depending on Jerry now. The writers keep
/// going, and the log stays open: if the installer can't start, Jerry goes on.
fn before_exit(app: &AppHandle) {
    app.state::<Sessions>().save_now();
    let flushed = jobs::on_main("flush the clipboard for the update", || {
        clipboard::flush_on_exit();
        Ok(())
    });
    if let Err(error) = tauri::async_runtime::block_on(flushed) {
        tracing::error!(code = ?error.code, "can't flush the clipboard before the update");
    }
}

/// The offer, for a page that just loaded; `update:changed` tells about later ones.
#[tauri::command]
#[specta::specta]
pub fn update_offer(app: AppHandle) -> Option<UpdateOffer> {
    app.state::<Updates>().current().offer.clone()
}

/// Downloads and installs the offered version: the user pressed Update. How it goes comes through
/// `update:changed`; when it works, Jerry exits and the new one starts.
#[tauri::command]
#[specta::specta]
pub async fn update_install(app: AppHandle) {
    let updates = app.state::<Updates>();
    let mut update = None;
    updates.change(&app, |current| {
        update = current.begin();
        update.is_some()
    });
    let Some(update) = update else { return };
    tracing::info!(version = %update.version, "installing an update");
    if install(&app, update).await.is_err() {
        updates.set_stage(&app, UpdateStage::Failed);
    }
}

async fn install(app: &AppHandle, update: Update) -> Result<(), JerryError> {
    let updates = app.state::<Updates>();
    let mut progress = Progress::default();
    // Checks the package against Jerry's public key before anything uses it.
    let bytes = update
        .download(
            |bytes, total| {
                if let Some(stage) = progress.chunk(bytes, total, Instant::now()) {
                    updates.set_stage(app, stage);
                }
            },
            || {},
        )
        .await
        .map_err(failed)?;
    updates.set_stage(app, UpdateStage::Installing);
    // Returns only if the installer can't start.
    jobs::background("install the update", move || {
        update.install(bytes).map_err(failed)
    })
    .await
}

/// The cause goes to the log; the page offers Retry whatever it was.
fn failed(error: tauri_plugin_updater::Error) -> JerryError {
    tracing::error!(%error, "the update failed");
    JerryError::internal()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Duration = CHECK_EVERY;

    fn current_with_offer() -> Current<u32> {
        let mut current = Current::default();
        assert!(current.found("0.2.0".into(), "- A fix".into(), 7));
        current
    }

    fn stage(current: &Current<u32>) -> Option<UpdateStage> {
        current.offer.as_ref().map(|offer| offer.stage)
    }

    #[test]
    fn a_check_is_due_at_start_and_once_a_day() {
        let start = SystemTime::UNIX_EPOCH + DAY;
        assert!(due(None, start), "the first one runs at once");
        assert!(!due(Some(start), start));
        assert!(!due(Some(start), start + DAY - Duration::from_secs(1)));
        assert!(due(Some(start), start + DAY));
    }

    #[test]
    fn a_clock_that_went_backwards_is_due() {
        let start = SystemTime::UNIX_EPOCH + DAY;
        assert!(due(Some(start), start - Duration::from_secs(1)));
    }

    #[test]
    fn an_install_starts_once() {
        let mut current = current_with_offer();
        assert_eq!(current.begin(), Some(7));
        assert_eq!(
            stage(&current),
            Some(UpdateStage::Downloading { percent: None })
        );
        assert_eq!(current.begin(), None, "a second press does nothing");
    }

    #[test]
    fn a_failed_install_can_be_tried_again() {
        let mut current = current_with_offer();
        current.begin();
        current.offer.as_mut().expect("offered").stage = UpdateStage::Failed;
        assert_eq!(current.begin(), Some(7));
    }

    #[test]
    fn nothing_to_install_without_an_offer() {
        assert_eq!(Current::<u32>::default().begin(), None);
    }

    #[test]
    fn a_check_leaves_a_running_install_alone() {
        let mut current = current_with_offer();
        current.begin();
        assert!(!current.found("0.3.0".into(), String::new(), 8));
        assert!(!current.nothing_new());
        let offer = current.offer.as_ref().expect("still offered");
        assert_eq!(offer.version, "0.2.0");
        assert_eq!(current.begin(), None);
    }

    #[test]
    fn a_newer_check_replaces_the_offer_and_a_taken_back_release_goes() {
        let mut current = current_with_offer();
        assert!(current.found("0.3.0".into(), String::new(), 8));
        assert_eq!(current.begin(), Some(8));
        current.offer.as_mut().expect("offered").stage = UpdateStage::Failed;
        assert!(current.nothing_new());
        assert_eq!(current.offer, None);
        assert!(!current.nothing_new(), "nothing changes the second time");
        assert_eq!(current.begin(), None);
    }

    #[test]
    fn progress_is_at_most_ten_messages_a_second() {
        let start = Instant::now();
        let mut progress = Progress::default();
        let percent = |percent| {
            Some(UpdateStage::Downloading {
                percent: Some(percent),
            })
        };
        assert_eq!(progress.chunk(10, Some(100), start), percent(10));
        let soon = start + PROGRESS_EVERY - Duration::from_millis(1);
        assert_eq!(progress.chunk(10, Some(100), soon), None, "too soon");
        // The bytes held back still count.
        assert_eq!(
            progress.chunk(10, Some(100), start + PROGRESS_EVERY),
            percent(30)
        );
    }

    #[test]
    fn progress_sends_only_new_percents_and_never_more_than_100() {
        let start = Instant::now();
        let mut progress = Progress::default();
        assert!(progress.chunk(50, Some(100), start).is_some());
        let later = start + PROGRESS_EVERY * 2;
        assert_eq!(progress.chunk(0, Some(100), later), None, "still 50 %");
        assert_eq!(
            progress.chunk(500, Some(100), later),
            Some(UpdateStage::Downloading { percent: Some(100) })
        );
    }

    #[test]
    fn progress_without_a_size_has_no_percent() {
        let mut progress = Progress::default();
        assert_eq!(progress.chunk(10, None, Instant::now()), None);
        assert_eq!(progress.chunk(10, Some(0), Instant::now()), None);
    }
}
