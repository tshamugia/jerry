//! `jerry.db` on temp files: a new database, settings across a restart, damaged files and a
//! database from a newer Jerry.

// Test code throughout, helpers included (clippy allows `expect` in tests).
#![cfg(test)]

use std::path::{Path, PathBuf};

use jerry_core::settings::{Density, Settings, Theme};
use jerry_core::storage::{RECENT_FOLDERS, RecentFolder, SCHEMA_VERSION, Storage};
use jerry_core::{ErrorCode, JerryError};
use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;

/// A temp folder with a Georgian name, and `jerry.db` in it (not created yet).
fn db_in_temp() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("ჯერი").join("jerry.db");
    (dir, path)
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn stored_keys(path: &Path) -> Vec<String> {
    Connection::open(path)
        .expect("opens")
        .prepare("SELECT key FROM settings ORDER BY key")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
}

#[test]
fn a_new_database_has_every_table_in_wal_mode() {
    let (_dir, path) = db_in_temp();
    let storage = Storage::open(&path).expect("opens");
    assert_eq!(storage.settings(), &Settings::default());

    let conn = Connection::open(&path).expect("opens again");
    let mode: String = conn
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .expect("journal mode");
    assert_eq!(mode, "wal");
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("user version");
    assert_eq!(usize::try_from(version), Ok(SCHEMA_VERSION));
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(
        tables,
        [
            "favorite_groups",
            "favorites",
            "folder_views",
            "keybindings",
            "network_places",
            "recent_folders",
            "search_history",
            "session",
            "settings",
            "transfer_journal",
        ]
    );
}

#[test]
fn settings_survive_a_restart() {
    let (_dir, path) = db_in_temp();
    let mut storage = Storage::open(&path).expect("opens");
    assert_eq!(storage.set_setting("theme", json!("dark")), Ok(true));
    assert_eq!(storage.set_setting("density", json!("compact")), Ok(true));
    assert_eq!(storage.set_setting("density", json!("compact")), Ok(false));
    drop(storage);

    let mut storage = Storage::open(&path).expect("reopens");
    assert_eq!(storage.settings().theme, Theme::Dark);
    assert_eq!(storage.settings().density, Density::Compact);
    assert_eq!(stored_keys(&path), ["density", "theme"]);

    // Back to the default: the row goes, so a future default applies.
    assert_eq!(
        storage.set_setting("density", json!("comfortable")),
        Ok(true)
    );
    assert_eq!(stored_keys(&path), ["theme"]);
}

#[test]
fn bad_settings_change_nothing() {
    let (_dir, path) = db_in_temp();
    let mut storage = Storage::open(&path).expect("opens");
    let code = |result: Result<bool, JerryError>| result.map_err(|error| error.code);
    assert_eq!(
        code(storage.set_setting("theme", json!(7))),
        Err(ErrorCode::Internal)
    );
    assert_eq!(
        code(storage.set_setting("noSuchSetting", json!(true))),
        Err(ErrorCode::Internal)
    );
    assert_eq!(storage.settings(), &Settings::default());
    assert!(stored_keys(&path).is_empty());
}

#[test]
fn a_file_that_isnt_a_database_is_moved_aside() {
    let (dir, path) = db_in_temp();
    let folder = path.parent().expect("folder").to_path_buf();
    std::fs::create_dir_all(&folder).expect("folder");
    std::fs::write(&path, "not a database, just text").expect("write");

    let mut storage = Storage::open(&path).expect("opens a fresh database");
    assert_eq!(storage.settings(), &Settings::default());
    assert_eq!(storage.set_setting("showHidden", json!(true)), Ok(true));
    drop(storage);

    let moved = folder.join("jerry.db.corrupt");
    assert_eq!(
        std::fs::read_to_string(&moved).expect("kept"),
        "not a database, just text"
    );
    assert!(
        Storage::open(&path)
            .expect("reopens")
            .settings()
            .show_hidden
    );

    // Damaged again: the first copy stays, the second gets the next name.
    std::fs::write(&path, "garbage again").expect("write");
    drop(Storage::open(&path).expect("opens a fresh database"));
    assert_eq!(
        std::fs::read_to_string(folder.join("jerry.db.corrupt.2")).expect("kept"),
        "garbage again"
    );
    assert_eq!(
        std::fs::read_to_string(&moved).expect("still there"),
        "not a database, just text"
    );
    drop(dir);
}

#[test]
fn a_database_with_damaged_pages_is_moved_aside() {
    let (_dir, path) = db_in_temp();
    let mut storage = Storage::open(&path).expect("opens");
    storage.set_setting("theme", json!("dark")).expect("saves");
    drop(storage);

    // Overwrite everything after the first page (the schema) with garbage.
    let mut bytes = std::fs::read(&path).expect("read");
    let page = usize::from(u16::from_be_bytes([bytes[16], bytes[17]]));
    assert!(bytes.len() > page, "more than one page");
    bytes[page..].fill(0xA5);
    std::fs::write(&path, &bytes).expect("write");

    let storage = Storage::open(&path).expect("opens a fresh database");
    assert_eq!(storage.settings(), &Settings::default());
    let folder = path.parent().expect("folder");
    assert_eq!(
        std::fs::read(folder.join("jerry.db.corrupt")).expect("kept"),
        bytes
    );
}

#[test]
fn a_database_from_a_newer_jerry_is_left_alone() {
    let (_dir, path) = db_in_temp();
    drop(Storage::open(&path).expect("opens"));
    let newer = i64::try_from(SCHEMA_VERSION).expect("small") + 1;
    Connection::open(&path)
        .expect("opens")
        .pragma_update(None, "user_version", newer)
        .expect("sets the version");
    let before = std::fs::read(&path).expect("read");

    let error = Storage::open(&path).err().expect("refuses to open it");
    assert_eq!(error.code, ErrorCode::Internal);
    assert_eq!(std::fs::read(&path).expect("read"), before);
    let folder = path.parent().expect("folder");
    assert_eq!(file_names(folder), ["jerry.db"]);
}

#[test]
fn in_memory_storage_runs_on_defaults() {
    let mut storage = Storage::in_memory().expect("opens");
    assert_eq!(storage.settings(), &Settings::default());
    assert_eq!(storage.set_setting("theme", json!("light")), Ok(true));
    assert_eq!(storage.settings().theme, Theme::Light);
}

#[test]
fn recent_folders_count_visits_latest_first() {
    let (_dir, path) = db_in_temp();
    let projects = PathBuf::from(r"\?\C:\Projects\ნახაზები");
    let downloads = PathBuf::from(r"\?\UNC\server\share\Downloads");
    {
        let mut storage = Storage::open(&path).expect("opens");
        storage.visit_folder(&projects, 1_000).expect("saves");
        storage.visit_folder(&downloads, 2_000).expect("saves");
        storage.visit_folder(&projects, 3_000).expect("saves");
    }

    let storage = Storage::open(&path).expect("reopens");
    let recent = storage.recent_folders(10).expect("reads");
    assert_eq!(
        recent,
        [
            RecentFolder {
                path: projects,
                last_opened: 3_000,
                open_count: 2
            },
            RecentFolder {
                path: downloads,
                last_opened: 2_000,
                open_count: 1
            },
        ]
    );
    assert_eq!(storage.recent_folders(1).expect("reads").len(), 1);
}

#[test]
fn recent_folders_keep_names_that_are_not_unicode() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    let mut wide: Vec<u16> = r"\?\C:\a".encode_utf16().collect();
    wide.push(0xDC00);
    let folder = PathBuf::from(OsString::from_wide(&wide));
    let mut storage = Storage::in_memory().expect("opens");
    storage.visit_folder(&folder, 1).expect("saves");
    assert_eq!(storage.recent_folders(1).expect("reads")[0].path, folder);
}

#[test]
fn only_the_latest_recent_folders_stay() {
    let mut storage = Storage::in_memory().expect("opens");
    for n in 0..RECENT_FOLDERS + 20 {
        let folder = PathBuf::from(format!(r"\?\C:\f{n}"));
        storage.visit_folder(&folder, n as i64).expect("saves");
    }
    let recent = storage.recent_folders(RECENT_FOLDERS * 2).expect("reads");
    assert_eq!(recent.len(), RECENT_FOLDERS);
    assert_eq!(
        recent.first().map(|folder| folder.last_opened),
        Some((RECENT_FOLDERS + 19) as i64)
    );
    assert_eq!(recent.last().map(|folder| folder.last_opened), Some(20));
}
