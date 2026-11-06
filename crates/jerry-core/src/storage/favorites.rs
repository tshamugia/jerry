//! The sidebar's favorites in `jerry.db` (#35): `favorites` and `favorite_groups`. Positions are
//! kept 0, 1, 2, ... within the top level, within each group and among the groups.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use rusqlite::{OptionalExtension, Transaction, params};
use windows::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};

use super::{Storage, blob_path, path_blob};
use crate::error::{ErrorCode, JerryError};
use crate::favorites::{Favorite, FavoriteGroup, Favorites};

impl Storage {
    /// All favorites and their groups, in sidebar order.
    pub fn favorites(&self) -> Result<Favorites, JerryError> {
        let groups = self
            .conn
            .prepare("SELECT id, name FROM favorite_groups ORDER BY position, id")?
            .query_map([], |row| {
                Ok(FavoriteGroup {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        let items = self
            .conn
            .prepare(
                "SELECT f.id, f.path, f.name, f.group_id FROM favorites AS f
                 LEFT JOIN favorite_groups AS g ON g.id = f.group_id
                 ORDER BY f.group_id IS NOT NULL, g.position, g.id, f.position, f.id",
            )?
            .query_map([], |row| {
                Ok(Favorite {
                    id: row.get(0)?,
                    path: blob_path(&row.get::<_, Vec<u8>>(1)?),
                    name: row.get(2)?,
                    group: row.get(3)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(Favorites { groups, items })
    }

    /// The favorite `id`, if there is one.
    pub fn favorite(&self, id: u32) -> Result<Option<Favorite>, JerryError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, path, name, group_id FROM favorites WHERE id = ?1",
                [id],
                |row| {
                    Ok(Favorite {
                        id: row.get(0)?,
                        path: blob_path(&row.get::<_, Vec<u8>>(1)?),
                        name: row.get(2)?,
                        group: row.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Adds the folder `path` (an exact path, as core keeps it) at the end of the top level.
    /// Returns its id, and false if it was a favorite already (compared as Windows compares
    /// paths, ignoring case): it stays where it is.
    pub fn add_favorite(&mut self, path: &Path) -> Result<(u32, bool), JerryError> {
        let tx = self.conn.transaction()?;
        let existing = tx
            .prepare("SELECT id, path FROM favorites")?
            .query_map([], |row| Ok((row.get(0)?, row.get::<_, Vec<u8>>(1)?)))?
            .collect::<Result<Vec<(u32, Vec<u8>)>, _>>()?
            .into_iter()
            .find(|(_, blob)| same_path(&blob_path(blob), path));
        if let Some((id, _)) = existing {
            return Ok((id, false));
        }
        tx.execute(
            "INSERT INTO favorites (path, name, group_id, position)
             VALUES (?1, NULL, NULL, (SELECT COUNT(*) FROM favorites WHERE group_id IS NULL))",
            params![path_blob(path)],
        )?;
        let id = id_of(tx.last_insert_rowid())?;
        tx.commit()?;
        Ok((id, true))
    }

    /// Names the favorite `id` `name` (trimmed) in the sidebar; empty or none: its folder's own
    /// name again. The folder itself isn't renamed. False if nothing changed.
    pub fn rename_favorite(&mut self, id: u32, name: Option<&str>) -> Result<bool, JerryError> {
        let name = name.map(str::trim).filter(|name| !name.is_empty());
        let changed = self.conn.execute(
            "UPDATE favorites SET name = ?2 WHERE id = ?1 AND name IS NOT ?2",
            params![id, name],
        )?;
        Ok(changed > 0)
    }

    /// Removes the favorite `id`; its folder is untouched. False if it wasn't there.
    pub fn remove_favorite(&mut self, id: u32) -> Result<bool, JerryError> {
        let tx = self.conn.transaction()?;
        let Some(group) = group_of(&tx, id)? else {
            return Ok(false);
        };
        tx.execute("DELETE FROM favorites WHERE id = ?1", [id])?;
        let rest = members(&tx, group)?;
        place(&tx, group, &rest)?;
        tx.commit()?;
        Ok(true)
    }

    /// Moves the favorite `id` into `group` (none: the top level) at `index` there (past the end:
    /// last). False if it was there already. `NotFound` for a favorite or group that isn't there.
    pub fn move_favorite(
        &mut self,
        id: u32,
        group: Option<u32>,
        index: usize,
    ) -> Result<bool, JerryError> {
        let tx = self.conn.transaction()?;
        let from = group_of(&tx, id)?.ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
        if let Some(group) = group
            && !group_exists(&tx, group)?
        {
            return Err(JerryError::new(ErrorCode::NotFound));
        }
        let before = members(&tx, group)?;
        let mut after: Vec<u32> = before.iter().copied().filter(|&x| x != id).collect();
        after.insert(index.min(after.len()), id);
        if from == group && after == before {
            return Ok(false);
        }
        place(&tx, group, &after)?;
        if from != group {
            let rest = members(&tx, from)?;
            place(&tx, from, &rest)?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Adds a group named `name` (trimmed) after the others and returns its id. `InvalidName`
    /// for an empty name.
    pub fn add_favorite_group(&mut self, name: &str) -> Result<u32, JerryError> {
        let name = group_name(name)?;
        self.conn.execute(
            "INSERT INTO favorite_groups (name, position)
             VALUES (?1, (SELECT COUNT(*) FROM favorite_groups))",
            [name],
        )?;
        id_of(self.conn.last_insert_rowid())
    }

    /// Renames the group `id` to `name` (trimmed). False if nothing changed; `InvalidName` for an
    /// empty name.
    pub fn rename_favorite_group(&mut self, id: u32, name: &str) -> Result<bool, JerryError> {
        let name = group_name(name)?;
        let changed = self.conn.execute(
            "UPDATE favorite_groups SET name = ?2 WHERE id = ?1 AND name IS NOT ?2",
            params![id, name],
        )?;
        Ok(changed > 0)
    }

    /// Removes the group `id`. Its favorites stay: they go to the end of the top level, in their
    /// order. False if it wasn't there.
    pub fn remove_favorite_group(&mut self, id: u32) -> Result<bool, JerryError> {
        let tx = self.conn.transaction()?;
        if !group_exists(&tx, id)? {
            return Ok(false);
        }
        let mut top = members(&tx, None)?;
        top.extend(members(&tx, Some(id))?);
        place(&tx, None, &top)?;
        tx.execute("DELETE FROM favorite_groups WHERE id = ?1", [id])?;
        let rest = groups(&tx)?;
        place_groups(&tx, &rest)?;
        tx.commit()?;
        Ok(true)
    }

    /// Moves the group `id`, with its favorites, to `index` among the groups (past the end:
    /// last). False if it was there already; `NotFound` for a group that isn't there.
    pub fn move_favorite_group(&mut self, id: u32, index: usize) -> Result<bool, JerryError> {
        let tx = self.conn.transaction()?;
        let before = groups(&tx)?;
        if !before.contains(&id) {
            return Err(JerryError::new(ErrorCode::NotFound));
        }
        let mut after: Vec<u32> = before.iter().copied().filter(|&x| x != id).collect();
        after.insert(index.min(after.len()), id);
        if after == before {
            return Ok(false);
        }
        place_groups(&tx, &after)?;
        tx.commit()?;
        Ok(true)
    }
}

/// The group the favorite `id` is in (`Some(None)`: the top level); none if there is no such
/// favorite.
fn group_of(tx: &Transaction<'_>, id: u32) -> Result<Option<Option<u32>>, JerryError> {
    Ok(tx
        .query_row(
            "SELECT group_id FROM favorites WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?)
}

fn group_exists(tx: &Transaction<'_>, id: u32) -> Result<bool, JerryError> {
    Ok(tx
        .query_row("SELECT 1 FROM favorite_groups WHERE id = ?1", [id], |_| {
            Ok(())
        })
        .optional()?
        .is_some())
}

/// The favorites in `group` (none: the top level), in order.
fn members(tx: &Transaction<'_>, group: Option<u32>) -> Result<Vec<u32>, JerryError> {
    Ok(tx
        .prepare("SELECT id FROM favorites WHERE group_id IS ?1 ORDER BY position, id")?
        .query_map([group], |row| row.get(0))?
        .collect::<Result<_, _>>()?)
}

/// Puts the favorites `ids` in `group` (none: the top level), in this order.
fn place(tx: &Transaction<'_>, group: Option<u32>, ids: &[u32]) -> Result<(), JerryError> {
    let mut update =
        tx.prepare("UPDATE favorites SET group_id = ?2, position = ?3 WHERE id = ?1")?;
    for (position, id) in ids.iter().enumerate() {
        update.execute(params![id, group, position as i64])?;
    }
    Ok(())
}

/// The groups, in order.
fn groups(tx: &Transaction<'_>) -> Result<Vec<u32>, JerryError> {
    Ok(tx
        .prepare("SELECT id FROM favorite_groups ORDER BY position, id")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?)
}

fn place_groups(tx: &Transaction<'_>, ids: &[u32]) -> Result<(), JerryError> {
    let mut update = tx.prepare("UPDATE favorite_groups SET position = ?2 WHERE id = ?1")?;
    for (position, id) in ids.iter().enumerate() {
        update.execute(params![id, position as i64])?;
    }
    Ok(())
}

/// A group's name: `name` trimmed; `InvalidName` if nothing is left.
fn group_name(name: &str) -> Result<&str, JerryError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(JerryError::new(ErrorCode::InvalidName));
    }
    Ok(name)
}

/// A row id as the UI gets it. SQLite numbers rows from 1 up, so ids stay far below `u32::MAX`.
fn id_of(rowid: i64) -> Result<u32, JerryError> {
    u32::try_from(rowid).map_err(|_| JerryError::internal())
}

/// Whether `a` and `b` are the same path as Windows compares them: ordinal, ignoring case.
fn same_path(a: &Path, b: &Path) -> bool {
    let a: Vec<u16> = a.as_os_str().encode_wide().collect();
    let b: Vec<u16> = b.as_os_str().encode_wide().collect();
    // SAFETY: both are valid slices.
    unsafe { CompareStringOrdinal(&a, &b, true) == CSTR_EQUAL }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// The sidebar as text: `name@group` in order, the top level without `@`.
    fn shown(storage: &Storage) -> Vec<String> {
        let favorites = storage.favorites().expect("favorites");
        let group = |id: u32| {
            favorites
                .groups
                .iter()
                .find(|group| group.id == id)
                .map(|group| group.name.clone())
                .expect("its group is listed")
        };
        favorites
            .items
            .iter()
            .map(|item| {
                let name = item.name.clone().unwrap_or_else(|| {
                    item.path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default()
                });
                match item.group {
                    Some(id) => format!("{name}@{}", group(id)),
                    None => name,
                }
            })
            .collect()
    }

    fn add(storage: &mut Storage, path: &str) -> u32 {
        let (id, added) = storage.add_favorite(Path::new(path)).expect("adds");
        assert!(added, "{path} is new");
        id
    }

    #[test]
    fn favorites_keep_their_path_and_order() {
        let mut storage = Storage::in_memory().expect("opens");
        assert_eq!(storage.favorites(), Ok(Favorites::default()));
        let drawings = add(&mut storage, r"\\?\D:\Projects\2026\BMW\Drawings");
        add(&mut storage, r"\\?\UNC\INSTA-NAS01\Projects\ნახაზები");
        assert_eq!(shown(&storage), ["Drawings", "ნახაზები"]);
        assert_eq!(
            storage.favorite(drawings).expect("reads"),
            Some(Favorite {
                id: drawings,
                path: PathBuf::from(r"\\?\D:\Projects\2026\BMW\Drawings"),
                name: None,
                group: None,
            })
        );
        assert_eq!(storage.favorite(9999), Ok(None));
    }

    #[test]
    fn a_folder_is_a_favorite_once() {
        let mut storage = Storage::in_memory().expect("opens");
        let id = add(&mut storage, r"\\?\D:\Projects\BMW");
        add(&mut storage, r"\\?\D:\Projects\Audi");
        assert_eq!(
            storage.add_favorite(Path::new(r"\\?\d:\projects\bmw")),
            Ok((id, false)),
            "Windows compares paths ignoring case"
        );
        assert_eq!(shown(&storage), ["BMW", "Audi"], "it kept its place");
    }

    #[test]
    fn renaming_names_only_the_favorite() {
        let mut storage = Storage::in_memory().expect("opens");
        let id = add(&mut storage, r"\\?\D:\Projects\2026\BMW\Drawings");
        assert_eq!(
            storage.rename_favorite(id, Some("  BMW ნახაზები ")),
            Ok(true)
        );
        assert_eq!(storage.rename_favorite(id, Some("BMW ნახაზები")), Ok(false));
        assert_eq!(shown(&storage), ["BMW ნახაზები"]);
        assert_eq!(storage.rename_favorite(id, Some("   ")), Ok(true));
        assert_eq!(shown(&storage), ["Drawings"], "the folder's own name again");
        assert_eq!(storage.rename_favorite(id, None), Ok(false));
        assert_eq!(storage.rename_favorite(9999, Some("x")), Ok(false));
    }

    #[test]
    fn favorites_move_within_and_across_groups() {
        let mut storage = Storage::in_memory().expect("opens");
        let a = add(&mut storage, r"\\?\D:\A");
        let b = add(&mut storage, r"\\?\D:\B");
        let c = add(&mut storage, r"\\?\D:\C");
        let insta = storage.add_favorite_group("Insta").expect("group");
        let ts = storage.add_favorite_group(" TSGroup ").expect("group");

        assert_eq!(storage.move_favorite(c, None, 0), Ok(true));
        assert_eq!(shown(&storage), ["C", "A", "B"]);
        assert_eq!(
            storage.move_favorite(c, None, 0),
            Ok(false),
            "there already"
        );

        assert_eq!(storage.move_favorite(a, Some(insta), 0), Ok(true));
        assert_eq!(storage.move_favorite(b, Some(insta), 99), Ok(true));
        assert_eq!(storage.move_favorite(c, Some(ts), 0), Ok(true));
        assert_eq!(shown(&storage), ["A@Insta", "B@Insta", "C@TSGroup"]);

        assert_eq!(storage.move_favorite(c, Some(insta), 1), Ok(true));
        assert_eq!(shown(&storage), ["A@Insta", "C@Insta", "B@Insta"]);
        assert_eq!(storage.move_favorite(a, None, 0), Ok(true));
        assert_eq!(shown(&storage), ["A", "C@Insta", "B@Insta"]);

        let missing = storage.move_favorite(a, Some(9999), 0).map_err(|e| e.code);
        assert_eq!(missing, Err(ErrorCode::NotFound));
        let gone = storage.move_favorite(9999, None, 0).map_err(|e| e.code);
        assert_eq!(gone, Err(ErrorCode::NotFound));

        // A new favorite goes to the end of the top level.
        add(&mut storage, r"\\?\D:\D");
        assert_eq!(shown(&storage), ["A", "D", "C@Insta", "B@Insta"]);
    }

    #[test]
    fn removing_a_favorite_closes_the_gap() {
        let mut storage = Storage::in_memory().expect("opens");
        let a = add(&mut storage, r"\\?\D:\A");
        add(&mut storage, r"\\?\D:\B");
        add(&mut storage, r"\\?\D:\C");
        assert_eq!(storage.remove_favorite(a), Ok(true));
        assert_eq!(storage.remove_favorite(a), Ok(false));
        let d = add(&mut storage, r"\\?\D:\D");
        assert_eq!(storage.move_favorite(d, None, 1), Ok(true));
        assert_eq!(shown(&storage), ["B", "D", "C"]);
    }

    #[test]
    fn groups_are_named_moved_and_removed_without_their_favorites() {
        let mut storage = Storage::in_memory().expect("opens");
        let a = add(&mut storage, r"\\?\D:\A");
        let b = add(&mut storage, r"\\?\D:\B");
        let c = add(&mut storage, r"\\?\D:\C");
        let insta = storage.add_favorite_group("Insta").expect("group");
        let ts = storage.add_favorite_group("TSGroup").expect("group");
        let own = storage.add_favorite_group("პირადი").expect("group");
        storage.move_favorite(a, Some(insta), 0).expect("moves");
        storage.move_favorite(b, Some(ts), 0).expect("moves");

        let empty = storage.add_favorite_group("  ").map_err(|e| e.code);
        assert_eq!(empty, Err(ErrorCode::InvalidName));
        let unnamed = storage.rename_favorite_group(ts, "").map_err(|e| e.code);
        assert_eq!(unnamed, Err(ErrorCode::InvalidName));
        assert_eq!(storage.rename_favorite_group(ts, "TS Group"), Ok(true));
        assert_eq!(storage.rename_favorite_group(ts, "TS Group"), Ok(false));

        assert_eq!(storage.move_favorite_group(own, 0), Ok(true));
        assert_eq!(storage.move_favorite_group(own, 0), Ok(false));
        assert_eq!(storage.move_favorite_group(ts, 1), Ok(true));
        let names: Vec<String> = storage
            .favorites()
            .expect("favorites")
            .groups
            .into_iter()
            .map(|group| group.name)
            .collect();
        assert_eq!(names, ["პირადი", "TS Group", "Insta"]);
        assert_eq!(shown(&storage), ["C", "B@TS Group", "A@Insta"]);

        assert_eq!(storage.remove_favorite_group(ts), Ok(true));
        assert_eq!(storage.remove_favorite_group(ts), Ok(false));
        assert_eq!(shown(&storage), ["C", "B", "A@Insta"], "B kept, at the end");
        assert_eq!(storage.move_favorite(c, None, 9), Ok(true));
        assert_eq!(shown(&storage), ["B", "C", "A@Insta"]);
        let gone = storage.move_favorite_group(ts, 0).map_err(|e| e.code);
        assert_eq!(gone, Err(ErrorCode::NotFound));
    }

    #[test]
    fn favorites_survive_reopening_the_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("jerry.db");
        let mut storage = Storage::open(&path).expect("opens");
        let a = add(&mut storage, r"\\?\D:\Projects\BMW");
        let insta = storage.add_favorite_group("Insta").expect("group");
        storage.move_favorite(a, Some(insta), 0).expect("moves");
        storage
            .rename_favorite(a, Some("BMW ნახაზები"))
            .expect("renames");
        drop(storage);

        let reopened = Storage::open(&path).expect("reopens");
        assert_eq!(shown(&reopened), ["BMW ნახაზები@Insta"]);
    }
}
