//! Jerry's database: one SQLite file (`%APPDATA%\Jerry\jerry.db`) for everything that persists.
//!
//! - WAL journal; the schema version is SQLite's `user_version`, moved forward by
//!   `rusqlite_migration`.
//! - Before a migration the file is copied to `jerry.db.v{N}.bak` (`N` = the version it had).
//! - A file that isn't a database or fails SQLite's quick check is moved aside to
//!   `jerry.db.corrupt` and Jerry starts with a fresh one.
//! - A file from a newer Jerry (a higher schema version) is left as it is and opening fails.
//!
//! Jerry never overwrites or deletes these files: a name that is taken gets `.2`, `.3`, ...

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, params};
use rusqlite_migration::{M, Migrations};
use serde_json::Value;

use crate::error::{ErrorCode, JerryError};
use crate::settings::Settings;

/// The schema, one entry per version. Append only: a released migration never changes.
const MIGRATIONS: &[M<'static>] = &[M::up(include_str!("storage/migrations/0001-v0.1.sql"))];

/// The schema version this Jerry writes.
pub const SCHEMA_VERSION: usize = MIGRATIONS.len();

/// How long a statement waits for another connection's lock (a second Jerry) before failing.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The open database, with the settings read from it.
pub struct Storage {
    conn: Connection,
    settings: Settings,
}

impl Storage {
    /// Opens the database at `path`, creating its folder and the file if needed, and migrates it
    /// to [`SCHEMA_VERSION`]. Blocking IO: call it from a background job.
    pub fn open(path: &Path) -> Result<Self, JerryError> {
        Self::open_with(path, &Migrations::from_slice(MIGRATIONS))
    }

    /// A database in memory, for when the file can't be used: Jerry runs on the defaults and
    /// saves nothing.
    pub fn in_memory() -> Result<Self, JerryError> {
        let mut conn = Connection::open_in_memory()?;
        Migrations::from_slice(MIGRATIONS).to_latest(&mut conn)?;
        Self::with_connection(conn)
    }

    /// The settings, defaults filled in.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Sets the setting `key` to `value` and saves it. Returns whether the settings changed.
    ///
    /// Fails with `Internal` for an unknown key or a value that doesn't fit it. A value equal to
    /// the default removes the key's row, so a later Jerry with a new default applies it.
    pub fn set_setting(&mut self, key: &str, value: Value) -> Result<bool, JerryError> {
        let next = self.settings.with(key, value)?;
        if next == self.settings {
            return Ok(false);
        }
        let value = next.get(key).ok_or_else(JerryError::internal)?;
        if Settings::default().get(key).as_ref() == Some(&value) {
            self.conn
                .execute("DELETE FROM settings WHERE key = ?1", [key])?;
        } else {
            self.conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value.to_string()],
            )?;
        }
        self.settings = next;
        Ok(true)
    }

    pub(crate) fn open_with(path: &Path, migrations: &Migrations<'_>) -> Result<Self, JerryError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|error| JerryError::from(error).at(dir))?;
        }
        let mut existed = path.metadata().is_ok_and(|metadata| metadata.len() > 0);
        let mut conn = connect(path)?;
        if let Some(problem) = damage(&conn)? {
            drop(conn);
            let moved = move_aside(path)?;
            tracing::error!(
                %problem,
                moved = %moved.display(),
                "jerry.db is damaged; moved it aside, starting with a fresh one"
            );
            existed = false;
            conn = connect(path)?;
        }

        let mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            tracing::warn!(mode, "jerry.db isn't in WAL mode");
        }
        conn.pragma_update(None, "foreign_keys", true)?;

        let version = user_version(&conn)?;
        let pending = migrations.pending_migrations(&conn)?;
        if pending < 0 {
            tracing::error!(
                version,
                "jerry.db is from a newer Jerry; leaving it as it is"
            );
            return Err(JerryError::internal().at(path));
        }
        if existed && pending > 0 {
            let backup = back_up(&conn, path, version)?;
            tracing::info!(version, backup = %backup.display(), "copied jerry.db before migrating");
        }
        migrations.to_latest(&mut conn)?;
        tracing::info!(
            path = %path.display(),
            version = user_version(&conn)?,
            "storage open"
        );
        Self::with_connection(conn)
    }

    fn with_connection(conn: Connection) -> Result<Self, JerryError> {
        let rows = conn
            .prepare("SELECT key, value FROM settings")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<(String, String)>, _>>()?;
        let settings = Settings::from_rows(rows);
        Ok(Self { conn, settings })
    }
}

fn connect(path: &Path) -> Result<Connection, JerryError> {
    let conn = Connection::open(path).map_err(|error| JerryError::from(error).at(path))?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(conn)
}

/// What is wrong with the file, if it isn't a database or fails SQLite's quick check. Other
/// errors (a lock, say) aren't damage and fail the open.
fn damage(conn: &Connection) -> Result<Option<String>, JerryError> {
    match conn.query_row("PRAGMA quick_check(1)", [], |row| row.get::<_, String>(0)) {
        Ok(result) if result == "ok" => Ok(None),
        Ok(problem) => Ok(Some(problem)),
        Err(error)
            if matches!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt)
            ) =>
        {
            Ok(Some(error.to_string()))
        }
        Err(error) => Err(error.into()),
    }
}

fn user_version(conn: &Connection) -> Result<usize, JerryError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    usize::try_from(version).map_err(|_| JerryError::internal())
}

/// Copies the database to `jerry.db.v{version}.bak` (consistent, WAL included).
fn back_up(conn: &Connection, path: &Path, version: usize) -> Result<PathBuf, JerryError> {
    let backup = free_path(&with_suffix(path, &format!(".v{version}.bak")));
    let target = backup
        .to_str()
        .ok_or_else(|| JerryError::internal().at(&backup))?;
    conn.execute("VACUUM INTO ?1", [target])
        .map_err(|error| JerryError::from(error).at(&backup))?;
    Ok(backup)
}

/// Moves a damaged database and its WAL files to `jerry.db.corrupt` (`-wal`, `-shm`).
fn move_aside(path: &Path) -> Result<PathBuf, JerryError> {
    let moved = free_path(&with_suffix(path, ".corrupt"));
    for suffix in ["", "-wal", "-shm"] {
        let from = with_suffix(path, suffix);
        if suffix.is_empty() || from.exists() {
            std::fs::rename(&from, with_suffix(&moved, suffix))
                .map_err(|error| JerryError::from(error).at(&from))?;
        }
    }
    Ok(moved)
}

/// `path`, or `path.2`, `path.3`, ...: the first one that is free along with its `-wal` and
/// `-shm` names. `std::fs::rename` replaces an existing file, so taken names are never used.
fn free_path(path: &Path) -> PathBuf {
    let taken = |path: &Path| {
        ["", "-wal", "-shm"]
            .iter()
            .any(|s| with_suffix(path, s).exists())
    };
    let mut candidate = path.to_path_buf();
    let mut n = 2;
    while taken(&candidate) {
        candidate = with_suffix(path, &format!(".{n}"));
        n += 1;
    }
    candidate
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

impl From<rusqlite::Error> for JerryError {
    fn from(error: rusqlite::Error) -> Self {
        use rusqlite::ErrorCode as Sqlite;
        let code = match error.sqlite_error_code() {
            Some(Sqlite::DatabaseBusy | Sqlite::DatabaseLocked) => ErrorCode::InUse { app: None },
            Some(Sqlite::DiskFull) => ErrorCode::NoSpace {
                needed: None,
                available: None,
            },
            Some(Sqlite::PermissionDenied | Sqlite::ReadOnly) => ErrorCode::AccessDenied,
            _ => ErrorCode::Internal,
        };
        tracing::warn!(%error, "database error");
        Self::new(code)
    }
}

impl From<rusqlite_migration::Error> for JerryError {
    fn from(error: rusqlite_migration::Error) -> Self {
        match error {
            rusqlite_migration::Error::RusqliteError { err, query } => {
                tracing::warn!(query, "migration failed");
                err.into()
            }
            other => {
                tracing::warn!(error = %other, "migration failed");
                Self::internal()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const EXTRA_TABLE: M<'static> = M::up("CREATE TABLE extra (x INTEGER) STRICT;");

    fn tables(conn: &Connection) -> Vec<String> {
        conn.prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    }

    #[test]
    fn migrating_copies_the_file_first() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("jerry.db");
        let mut v1 = Storage::open_with(&path, &Migrations::from_slice(&MIGRATIONS[..1]))
            .expect("opens at v1");
        v1.set_setting("theme", json!("dark")).expect("saves");
        drop(v1);

        let v2 = [MIGRATIONS[0].clone(), EXTRA_TABLE];
        let migrated = Storage::open_with(&path, &Migrations::from_slice(&v2)).expect("migrates");
        assert_eq!(migrated.settings().theme, crate::settings::Theme::Dark);
        assert_eq!(user_version(&migrated.conn).expect("version"), 2);
        assert!(tables(&migrated.conn).contains(&"extra".to_owned()));

        let backup = Connection::open(dir.path().join("jerry.db.v1.bak")).expect("backup");
        assert_eq!(user_version(&backup).expect("version"), 1);
        assert!(!tables(&backup).contains(&"extra".to_owned()));
        let theme: String = backup
            .query_row(
                "SELECT value FROM settings WHERE key = 'theme'",
                [],
                |row| row.get(0),
            )
            .expect("the setting is in the copy");
        assert_eq!(theme, r#""dark""#);
    }

    #[test]
    fn a_failed_migration_leaves_the_file_as_it_was() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("jerry.db");
        drop(Storage::open_with(&path, &Migrations::from_slice(&MIGRATIONS[..1])).expect("v1"));

        let broken = [MIGRATIONS[0].clone(), EXTRA_TABLE, M::up("NOT SQL;")];
        let failed = Storage::open_with(&path, &Migrations::from_slice(&broken));
        assert!(failed.is_err());

        let conn = Connection::open(&path).expect("reopens");
        assert_eq!(user_version(&conn).expect("version"), 1);
        assert!(!tables(&conn).contains(&"extra".to_owned()), "rolled back");
        assert!(dir.path().join("jerry.db.v1.bak").exists());
    }

    #[test]
    fn a_new_file_needs_no_copy() {
        let dir = tempfile::tempdir().expect("temp dir");
        drop(Storage::open(&dir.path().join("jerry.db")).expect("opens"));
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .expect("dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert!(
            names
                .iter()
                .all(|name| !name.to_string_lossy().contains(".bak")),
            "{names:?}"
        );
    }

    #[test]
    fn free_path_skips_taken_names() {
        let dir = tempfile::tempdir().expect("temp dir");
        let base = dir.path().join("jerry.db.corrupt");
        assert_eq!(free_path(&base), base);
        std::fs::write(&base, "").expect("write");
        assert_eq!(free_path(&base), dir.path().join("jerry.db.corrupt.2"));
        std::fs::write(dir.path().join("jerry.db.corrupt.2-wal"), "").expect("write");
        assert_eq!(free_path(&base), dir.path().join("jerry.db.corrupt.3"));
    }
}
