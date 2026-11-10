//! The transfer journal in `jerry.db` (#50): the `transfer_journal` table. What it is for is in
//! [`crate::transfer::journal`].

use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use rusqlite::{Row, params};

use super::{Storage, blob_path, path_blob};
use crate::error::JerryError;
use crate::transfer::journal::{Counts, Unfinished, Writing};
use crate::transfer::{Job, Kind};

impl Storage {
    /// Writes down `job`, queued by the Jerry started at `run`, and returns its record.
    pub fn journal_queue(&mut self, run: i64, job: &Job, at: i64) -> Result<i64, JerryError> {
        self.conn.execute(
            "INSERT INTO transfer_journal (run, kind, sources, destination, state, updated)
             VALUES (?1, ?2, ?3, ?4, 'queued', ?5)",
            params![
                run,
                kind_text(job.kind),
                paths_blob(&job.sources),
                path_blob(&job.destination),
                at
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// A turn of job `record` starts, from `counts`.
    pub fn journal_start(
        &mut self,
        record: i64,
        counts: &Counts,
        at: i64,
    ) -> Result<(), JerryError> {
        self.conn.execute(
            "UPDATE transfer_journal SET state = 'running', files_done = ?2, files_total = ?3,
                 bytes_done = ?4, bytes_total = ?5, updated = ?6
             WHERE id = ?1",
            params![
                record,
                counts.files_done as i64,
                counts.files_total as i64,
                counts.bytes_done as i64,
                counts.bytes_total as i64,
                at
            ],
        )?;
        Ok(())
    }

    /// Job `record` begins to write a file, having come as far as `counts`. A link has no
    /// temporary file: the one before stays named, in case it is still there.
    pub fn journal_write(
        &mut self,
        record: i64,
        writing: &Writing<'_>,
        counts: &Counts,
        at: i64,
    ) -> Result<(), JerryError> {
        self.conn.execute(
            "UPDATE transfer_journal SET writing_source = ?2, writing_target = ?3,
                 part = COALESCE(?4, part),
                 files_done = ?5, files_total = ?6, bytes_done = ?7, bytes_total = ?8, updated = ?9
             WHERE id = ?1",
            params![
                record,
                path_blob(writing.source),
                path_blob(writing.target),
                writing.part.map(path_blob),
                counts.files_done as i64,
                counts.files_total as i64,
                counts.bytes_done as i64,
                counts.bytes_total as i64,
                at
            ],
        )?;
        Ok(())
    }

    /// The temporary file of job `record` isn't there any more.
    pub fn journal_part_gone(&mut self, record: i64) -> Result<(), JerryError> {
        self.conn.execute(
            "UPDATE transfer_journal SET part = NULL WHERE id = ?1",
            [record],
        )?;
        Ok(())
    }

    /// Job `record` ended.
    pub fn journal_end(&mut self, record: i64, at: i64) -> Result<(), JerryError> {
        self.conn.execute(
            "UPDATE transfer_journal SET state = 'ended', updated = ?2 WHERE id = ?1",
            params![record, at],
        )?;
        Ok(())
    }

    /// The jobs of other Jerrys than `run` that didn't end, oldest first.
    pub fn journal_unfinished(&self, run: i64) -> Result<Vec<Unfinished>, JerryError> {
        let rows = self
            .conn
            .prepare(
                "SELECT id, kind, sources, destination, state, files_done, files_total,
                     bytes_done, bytes_total, writing_source, writing_target, part
                 FROM transfer_journal WHERE state <> 'ended' AND run <> ?1 ORDER BY id",
            )?
            .query_map([run], unfinished)?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// The temporary files that records of jobs other Jerrys than `run` ended name: `(record,
    /// part)`. A job's record keeps the last one it wrote until it is seen gone; one of a job
    /// that didn't end goes with Resume or Discard.
    pub fn journal_parts(&self, run: i64) -> Result<Vec<(i64, PathBuf)>, JerryError> {
        let rows = self
            .conn
            .prepare(
                "SELECT id, part FROM transfer_journal
                 WHERE part IS NOT NULL AND state = 'ended' AND run <> ?1 ORDER BY id",
            )?
            .query_map([run], |row| {
                Ok((row.get(0)?, blob_path(&row.get::<_, Vec<u8>>(1)?)))
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Forgets the jobs that ended before `before` (ms since the Unix epoch) and name no
    /// temporary file. Returns how many.
    pub fn journal_forget(&mut self, before: i64) -> Result<usize, JerryError> {
        let forgotten = self.conn.execute(
            "DELETE FROM transfer_journal WHERE state = 'ended' AND part IS NULL AND updated < ?1",
            [before],
        )?;
        Ok(forgotten)
    }
}

fn unfinished(row: &Row<'_>) -> rusqlite::Result<Unfinished> {
    let path = |index: usize| -> rusqlite::Result<Option<PathBuf>> {
        Ok(row
            .get::<_, Option<Vec<u8>>>(index)?
            .map(|blob| blob_path(&blob)))
    };
    let count = |index: usize| -> rusqlite::Result<u64> { Ok(row.get::<_, i64>(index)? as u64) };
    let kind = match row.get_ref(1)?.as_str()? {
        "move" => Kind::Move,
        _ => Kind::Copy,
    };
    let writing = path(9)?.zip(path(10)?);
    Ok(Unfinished {
        record: row.get(0)?,
        job: Job {
            kind,
            sources: blob_paths(&row.get::<_, Vec<u8>>(2)?),
            destination: blob_path(&row.get::<_, Vec<u8>>(3)?),
        },
        started: row.get_ref(4)?.as_str()? == "running",
        counts: Counts {
            files_done: count(5)?,
            files_total: count(6)?,
            bytes_done: count(7)?,
            bytes_total: count(8)?,
        },
        writing,
        part: path(11)?,
    })
}

fn kind_text(kind: Kind) -> &'static str {
    match kind {
        Kind::Copy => "copy",
        Kind::Move => "move",
    }
}

/// Paths one after another, as `path_blob` keeps one, each ended by a 0 unit: no path has one.
fn paths_blob(paths: &[PathBuf]) -> Vec<u8> {
    paths
        .iter()
        .flat_map(|path| path.as_os_str().encode_wide().chain([0]))
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn blob_paths(blob: &[u8]) -> Vec<PathBuf> {
    let (pairs, _) = blob.as_chunks::<2>();
    pairs
        .split(|pair| *pair == [0, 0])
        .filter(|units| !units.is_empty())
        .map(|units| blob_path(units.as_flattened()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn job(kind: Kind) -> Job {
        Job {
            kind,
            sources: vec![
                PathBuf::from(r"\\?\C:\Users\თენგო\ნახაზი.dwg"),
                PathBuf::from(r"\\?\C:\Users\თენგო\პროექტები"),
            ],
            destination: PathBuf::from(r"\\?\UNC\nas\share\არქივი"),
        }
    }

    const RUN: i64 = 100;
    const NEXT: i64 = 200;

    #[test]
    fn a_job_is_written_down_before_it_starts_and_found_by_the_next_jerry() {
        let mut storage = Storage::in_memory().unwrap();
        let record = storage.journal_queue(RUN, &job(Kind::Move), 1).unwrap();
        assert_eq!(
            storage.journal_unfinished(RUN).unwrap(),
            vec![],
            "its own run"
        );
        let found = storage.journal_unfinished(NEXT).unwrap();
        assert_eq!(
            found,
            vec![Unfinished {
                record,
                job: job(Kind::Move),
                started: false,
                counts: Counts::default(),
                writing: None,
                part: None,
            }]
        );
    }

    #[test]
    fn the_file_being_written_is_kept_until_the_next_one() {
        let mut storage = Storage::in_memory().unwrap();
        let record = storage.journal_queue(RUN, &job(Kind::Copy), 1).unwrap();
        let counts = Counts {
            files_done: 3,
            files_total: 10,
            bytes_done: 300,
            bytes_total: 1_000,
        };
        storage
            .journal_start(record, &Counts::default(), 2)
            .unwrap();
        let (source, target) = (
            Path::new(r"\\?\C:\a\ბ.txt"),
            Path::new(r"\\?\UNC\nas\share\ბ.txt"),
        );
        let part = Path::new(r"\\?\UNC\nas\share\ბ.txt.jerrypart");
        let writing = Writing {
            source,
            target,
            part: Some(part),
        };
        storage.journal_write(record, &writing, &counts, 3).unwrap();
        let found = storage.journal_unfinished(NEXT).unwrap();
        assert!(found[0].started);
        assert_eq!(found[0].counts, counts);
        assert_eq!(
            found[0].writing,
            Some((source.to_owned(), target.to_owned()))
        );
        assert_eq!(found[0].part.as_deref(), Some(part));
        assert_eq!(
            storage.journal_parts(NEXT).unwrap(),
            vec![],
            "an unfinished job's part waits for Resume or Discard"
        );

        // A link has no temporary file: the last one stays named until it is seen gone.
        let link = Writing {
            source: Path::new(r"\\?\C:\a\link"),
            target: Path::new(r"\\?\UNC\nas\share\link"),
            part: None,
        };
        storage.journal_write(record, &link, &counts, 4).unwrap();
        assert_eq!(
            storage.journal_unfinished(NEXT).unwrap()[0].part.as_deref(),
            Some(part)
        );
        storage.journal_part_gone(record).unwrap();
        assert_eq!(storage.journal_unfinished(NEXT).unwrap()[0].part, None);
    }

    #[test]
    fn an_ended_job_is_not_unfinished_and_goes_after_a_while() {
        let mut storage = Storage::in_memory().unwrap();
        let ended = storage.journal_queue(RUN, &job(Kind::Copy), 1).unwrap();
        let kept = storage.journal_queue(RUN, &job(Kind::Copy), 1).unwrap();
        storage.journal_end(ended, 5).unwrap();
        storage.journal_end(kept, 5).unwrap();
        let part = Path::new(r"\\?\E:\x.jerrypart");
        let writing = Writing {
            source: Path::new(r"\\?\C:\x"),
            target: Path::new(r"\\?\E:\x"),
            part: Some(part),
        };
        storage
            .journal_write(kept, &writing, &Counts::default(), 5)
            .unwrap();
        assert_eq!(storage.journal_unfinished(NEXT).unwrap(), vec![]);
        assert_eq!(storage.journal_forget(5).unwrap(), 0, "not before 5");
        assert_eq!(
            storage.journal_forget(6).unwrap(),
            1,
            "the one without a part"
        );
        assert_eq!(
            storage.journal_parts(NEXT).unwrap(),
            vec![(kept, part.to_owned())]
        );
    }

    #[test]
    fn a_list_of_paths_comes_back_as_it_was() {
        let paths = vec![
            PathBuf::from(r"\\?\C:\ა"),
            PathBuf::from(r"\\?\C:\b c\😀.txt"),
            PathBuf::from(r"\\?\UNC\nas\share"),
        ];
        assert_eq!(blob_paths(&paths_blob(&paths)), paths);
        assert_eq!(blob_paths(&paths_blob(&[])), Vec::<PathBuf>::new());
    }
}
