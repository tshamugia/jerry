//! Jerry's database in the app: opened on its own thread at start, while Tauri and WebView2 start
//! up, then shared by the commands. The settings commands and `settings:changed` live here; the
//! storage itself is `jerry_core::storage`.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use jerry_core::JerryError;
use jerry_core::job::run_job;
use jerry_core::settings::Settings;
use jerry_core::storage::Storage;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::ipc::Json;
use crate::jobs;

/// Where the database lives: `%APPDATA%\Jerry\jerry.db`.
pub fn db_path() -> Result<PathBuf, JerryError> {
    Ok(jerry_win::folders::roaming_app_data()?
        .join("Jerry")
        .join("jerry.db"))
}

/// The database, once it is open. Clones share it.
#[derive(Clone, Default)]
pub struct Db(Arc<Shared>);

#[derive(Default)]
struct Shared {
    storage: OnceLock<Result<Mutex<Storage>, JerryError>>,
    /// Signalled once `storage` is set, for [`Db::settings_within`].
    opened: (Mutex<()>, Condvar),
}

impl Db {
    /// Starts opening the database at `path` on its own thread and returns at once. If the file
    /// can't be used, Jerry runs on the defaults in memory, saves nothing and logs why.
    pub fn open(path: Result<PathBuf, JerryError>) -> Self {
        let db = Self::default();
        let shared = Arc::clone(&db.0);
        let opening = std::thread::Builder::new()
            .name("jerry-storage".into())
            .spawn(move || {
                let started = Instant::now();
                let storage = run_job("open storage", || open_or_in_memory(path));
                tracing::info!(ms = started.elapsed().as_millis(), "storage ready");
                shared.set(storage.map(Mutex::new));
            });
        if let Err(error) = opening {
            tracing::error!(%error, "can't start the storage thread");
            db.0.set(Err(JerryError::internal()));
        }
        db
    }

    /// Runs `job` with the database locked, once it is open. Blocks: call it from a background
    /// job.
    pub fn with<T>(
        &self,
        job: impl FnOnce(&mut Storage) -> Result<T, JerryError>,
    ) -> Result<T, JerryError> {
        let storage = self.0.storage.wait().as_ref().map_err(JerryError::clone)?;
        job(&mut storage.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The settings, if the database opens within `timeout`; `None` (and a log line) if not.
    /// For the window's start, which needs the theme but must not hang on a slow disk.
    pub fn settings_within(&self, timeout: Duration) -> Option<Settings> {
        let (lock, opened) = &self.0.opened;
        let guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        let (_guard, wait) = opened
            .wait_timeout_while(guard, timeout, |()| self.0.storage.get().is_none())
            .unwrap_or_else(PoisonError::into_inner);
        if wait.timed_out() {
            tracing::warn!(
                ms = timeout.as_millis(),
                "the database isn't open yet; using the default settings"
            );
            return None;
        }
        self.with(|storage| Ok(storage.settings().clone())).ok()
    }
}

impl Shared {
    fn set(&self, storage: Result<Mutex<Storage>, JerryError>) {
        let _ = self.storage.set(storage);
        // Taking the lock orders the set before a waiter's check, so no wake-up is lost.
        drop(self.opened.0.lock().unwrap_or_else(PoisonError::into_inner));
        self.opened.1.notify_all();
    }
}

fn open_or_in_memory(path: Result<PathBuf, JerryError>) -> Result<Storage, JerryError> {
    path.and_then(|path| Storage::open(&path)).or_else(|error| {
        tracing::error!(%error, "can't use jerry.db; running on the defaults, saving nothing");
        Storage::in_memory()
    })
}

/// Sent to every window when a setting changes, with all the settings. Rust hears it too
/// (`window::apply`).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "settings:changed")]
pub struct SettingsChanged(pub Settings);

/// All settings, defaults filled in.
#[tauri::command]
#[specta::specta]
pub async fn settings_get(db: State<'_, Db>) -> Result<Settings, JerryError> {
    let db = Db::clone(&db);
    jobs::background("settings get", move || {
        db.with(|storage| Ok(storage.settings().clone()))
    })
    .await
}

/// Sets the setting `key` (a key of `Settings`) to `value` and saves it. If that changes
/// anything, every window gets `settings:changed`, this one included.
#[tauri::command]
#[specta::specta]
pub async fn settings_set(
    app: AppHandle,
    db: State<'_, Db>,
    key: String,
    value: Json,
) -> Result<(), JerryError> {
    let db = Db::clone(&db);
    jobs::background("settings set", move || {
        db.with(|storage| {
            if storage.set_setting(&key, value.0)? {
                // Still locked, so windows hear the changes in the order they were saved.
                if let Err(error) = SettingsChanged(storage.settings().clone()).emit(&app) {
                    tracing::error!(%error, "can't send settings:changed");
                }
            }
            Ok(())
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use jerry_core::settings::Theme;
    use serde_json::json;

    use super::*;

    #[test]
    fn settings_are_saved_once_the_database_is_open() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("Jerry").join("jerry.db");

        let db = Db::open(Ok(path.clone()));
        db.with(|storage| storage.set_setting("theme", json!("dark")))
            .expect("waits for the database, then saves");
        drop(db);

        let theme = Db::open(Ok(path)).with(|storage| Ok(storage.settings().theme));
        assert_eq!(theme, Ok(Theme::Dark));
    }

    #[test]
    fn settings_within_waits_for_the_database_but_not_forever() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = Db::open(Ok(dir.path().join("jerry.db")));
        let settings = db.settings_within(Duration::from_secs(10));
        assert_eq!(settings, Some(Settings::default()));

        // A database that never opens: the wait ends with the timeout.
        let never = Db::default();
        let started = Instant::now();
        assert_eq!(never.settings_within(Duration::from_millis(50)), None);
        assert!(started.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn without_a_usable_file_jerry_runs_on_the_defaults() {
        let db = Db::open(Err(JerryError::from_win32(5)));
        let changed = db.with(|storage| storage.set_setting("theme", json!("light")));
        assert_eq!(changed, Ok(true));
        assert_eq!(
            db.with(|storage| Ok(storage.settings().theme)),
            Ok(Theme::Light)
        );
    }
}
