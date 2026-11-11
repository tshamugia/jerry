//! The search window's history in `jerry.db` (#66): `search_history`, the last
//! [`SEARCH_HISTORY`] searches, newest first. A search run again moves to the top instead of
//! showing twice.

use rusqlite::params;
use serde_json::Value;

use super::Storage;
use crate::error::JerryError;
use crate::search::SavedSearch;

/// Searches `search_history` keeps (PRD «ღრმა ძებნა»); past it the ones used longest ago go.
pub const SEARCH_HISTORY: usize = 20;

impl Storage {
    /// The last searches, newest first. A row that can't be read (a newer Jerry's filter) is
    /// left out.
    pub fn search_history(&self) -> Result<Vec<SavedSearch>, JerryError> {
        let rows: Vec<(String, String)> = self
            .conn
            .prepare("SELECT query, filters FROM search_history ORDER BY used_at DESC, rowid DESC")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(query, filters)| match saved(query, &filters) {
                Ok(search) => Some(search),
                Err(_) => {
                    tracing::warn!("a search history row doesn't read");
                    None
                }
            })
            .take(SEARCH_HISTORY)
            .collect())
    }

    /// Notes that `search` was run at `at` (ms since the Unix epoch): it goes to the top of the
    /// history, once. Past [`SEARCH_HISTORY`] searches, the ones run longest ago go.
    pub fn add_search(&mut self, search: &SavedSearch, at: i64) -> Result<(), JerryError> {
        let filters = filters_json(search)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO search_history (query, filters, used_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (query, filters) DO UPDATE SET used_at = excluded.used_at",
            params![search.name, filters, at],
        )?;
        tx.execute(
            "DELETE FROM search_history WHERE rowid NOT IN
             (SELECT rowid FROM search_history ORDER BY used_at DESC, rowid DESC LIMIT ?1)",
            [SEARCH_HISTORY as i64],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Takes `search` out of the history. Returns whether it was there.
    pub fn remove_search(&mut self, search: &SavedSearch) -> Result<bool, JerryError> {
        let filters = filters_json(search)?;
        let removed = self.conn.execute(
            "DELETE FROM search_history WHERE query = ?1 AND filters = ?2",
            params![search.name, filters],
        )?;
        Ok(removed > 0)
    }
}

/// Everything in `search` but its name, as JSON: equal filters give equal text, field by field
/// in declaration order.
fn filters_json(search: &SavedSearch) -> Result<String, JerryError> {
    let mut value = serde_json::to_value(search).map_err(|_| JerryError::internal())?;
    if let Value::Object(fields) = &mut value {
        fields.remove("name");
    }
    serde_json::to_string(&value).map_err(|_| JerryError::internal())
}

fn saved(query: String, filters: &str) -> Result<SavedSearch, JerryError> {
    let mut value: Value = serde_json::from_str(filters).map_err(|_| JerryError::internal())?;
    let Value::Object(fields) = &mut value else {
        return Err(JerryError::internal());
    };
    fields.insert("name".to_owned(), Value::String(query));
    serde_json::from_value(value).map_err(|_| JerryError::internal())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{DateFilter, FileType, SearchItems};

    fn search(name: &str) -> SavedSearch {
        SavedSearch {
            name: name.to_owned(),
            ..SavedSearch::default()
        }
    }

    #[test]
    fn searches_run_again_move_to_the_top_once() {
        let mut storage = Storage::in_memory().expect("opens");
        assert_eq!(storage.search_history(), Ok(vec![]));
        let dwg = SavedSearch {
            name: "*.dwg".to_owned(),
            modified: Some(DateFilter::ThisWeek),
            items: SearchItems::Files,
            ..SavedSearch::default()
        };
        storage.add_search(&dwg, 1).expect("adds");
        storage.add_search(&search("ნახაზი"), 2).expect("adds");
        assert_eq!(
            storage.search_history(),
            Ok(vec![search("ნახაზი"), dwg.clone()])
        );

        storage.add_search(&dwg, 3).expect("runs again");
        assert_eq!(
            storage.search_history(),
            Ok(vec![dwg.clone(), search("ნახაზი")])
        );

        // The same name with other filters is another search.
        let drawings = SavedSearch {
            file_type: Some(FileType::Drawings),
            ..dwg.clone()
        };
        storage.add_search(&drawings, 4).expect("adds");
        assert_eq!(storage.search_history().map(|all| all.len()), Ok(3));

        assert_eq!(storage.remove_search(&dwg), Ok(true));
        assert_eq!(storage.remove_search(&dwg), Ok(false));
        assert_eq!(
            storage.search_history(),
            Ok(vec![drawings, search("ნახაზი")])
        );
    }

    #[test]
    fn the_history_keeps_the_last_twenty() {
        let mut storage = Storage::in_memory().expect("opens");
        for n in 0..25 {
            storage
                .add_search(&search(&format!("search {n}")), n)
                .expect("adds");
        }
        let history = storage.search_history().expect("reads");
        assert_eq!(history.len(), SEARCH_HISTORY);
        assert_eq!(history.first(), Some(&search("search 24")));
        assert_eq!(history.last(), Some(&search("search 5")));
        let rows: i64 = storage
            .conn
            .query_row("SELECT COUNT(*) FROM search_history", [], |row| row.get(0))
            .expect("counts");
        assert_eq!(rows, SEARCH_HISTORY as i64);
    }

    #[test]
    fn rows_that_dont_read_are_left_out() {
        let mut storage = Storage::in_memory().expect("opens");
        storage.add_search(&search("a"), 1).expect("adds");
        storage
            .conn
            .execute(
                "INSERT INTO search_history (query, filters, used_at)
                 VALUES ('b', '{\"fileType\":{\"kind\":\"holograms\"}}', 2)",
                [],
            )
            .expect("a newer Jerry's row");
        assert_eq!(storage.search_history(), Ok(vec![search("a")]));
    }
}
