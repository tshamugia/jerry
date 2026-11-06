//! The space on the volume a folder is on, for the status bar (#41) where the drive monitor has
//! none: a network drive or a share, which [`crate::drives`] never reads.
//!
//! Asking a network volume asks its server, which can take seconds to answer, or never. So each
//! read runs on a thread of its own and is waited for only so long ([`SPACE_WAIT`]); one that is
//! late goes on alone, and the next ask for that folder gets its answer. A folder has at most one
//! read at a time: asks while one runs wait for that one, so a server that doesn't answer never
//! piles up threads.

use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::drives::{Space, quiet_errors, space};
use crate::job::run_job;
use crate::path::long_path;

/// How long the status bar waits for a folder's free space.
pub const SPACE_WAIT: Duration = Duration::from_secs(3);

/// How a folder's space is read: [`folder_space`], or a fake in tests.
type Read = dyn Fn(&Path) -> Option<Space> + Send + Sync;

/// Reads folders' free space with a deadline. Clones share the reads that run.
#[derive(Clone)]
pub struct SpaceReader {
    read: Arc<Read>,
    state: Arc<Mutex<State>>,
}

/// Folders by [`key`].
#[derive(Default)]
struct State {
    /// The folders being read, with whoever waits for each.
    running: HashMap<String, Vec<Sender<Option<Space>>>>,
    /// Reads that finished after everyone had stopped waiting: the next ask gets the answer.
    late: HashMap<String, Space>,
}

impl Default for SpaceReader {
    /// Reads the volumes themselves ([`folder_space`]).
    fn default() -> Self {
        Self::with(folder_space)
    }
}

impl SpaceReader {
    /// Reads with `read` in place of the volumes (tests).
    fn with(read: impl Fn(&Path) -> Option<Space> + Send + Sync + 'static) -> Self {
        Self {
            read: Arc::new(read),
            state: Arc::default(),
        }
    }

    /// The size and free space of the volume `folder` is on, read now. If a read finished late
    /// since the last ask, its answer comes at once and a new read starts for the next ask.
    /// None if the volume can't be read, or doesn't answer within `wait`. Blocks up to `wait`:
    /// call it on a background thread.
    pub fn read(&self, folder: &Path, wait: Duration) -> Option<Space> {
        let (sender, answer) = mpsc::channel();
        let id = key(folder);
        let (late, start) = {
            let mut state = lock(&self.state);
            let late = state.late.remove(&id);
            let start = !state.running.contains_key(&id);
            let waiting = state.running.entry(id.clone()).or_default();
            if late.is_none() {
                waiting.push(sender);
            }
            (late, start)
        };
        if start && !self.start(folder, id) {
            return late;
        }
        late.or_else(|| answer.recv_timeout(wait).ok().flatten())
    }

    /// Reads `folder` on a thread of its own, for whoever waits on `id`. False if the thread
    /// didn't start.
    fn start(&self, folder: &Path, id: String) -> bool {
        let reader = self.clone();
        let folder = folder.to_owned();
        let done = id.clone();
        let started = std::thread::Builder::new()
            .name("folder space".into())
            .spawn(move || {
                // A panicking read fails alone: whoever waits gets none.
                let space = run_job("folder space", || Ok((reader.read)(&folder)))
                    .ok()
                    .flatten();
                let mut state = lock(&reader.state);
                let mut heard = false;
                for sender in state.running.remove(&done).unwrap_or_default() {
                    // An error: that ask has stopped waiting.
                    heard |= sender.send(space).is_ok();
                }
                if let (false, Some(space)) = (heard, space) {
                    state.late.insert(done, space);
                }
            });
        if let Err(error) = started {
            tracing::error!(%error, "can't start a thread to read free space");
            lock(&self.state).running.remove(&id);
            return false;
        }
        true
    }
}

/// Asks the volume `folder` is on for its size and the space free on it for this user. For a
/// network drive or share this asks the server, and can block for as long as Windows waits for
/// it.
pub fn folder_space(folder: &Path) -> Option<Space> {
    quiet_errors();
    let long = long_path(folder).ok()?;
    let mut wide: Vec<u16> = long.as_os_str().encode_wide().collect();
    // A share's root needs its trailing backslash.
    if wide.last() != Some(&u16::from(b'\\')) {
        wide.push(u16::from(b'\\'));
    }
    wide.push(0);
    space(&wide)
}

/// Folders are the same folder whatever their case.
fn key(folder: &Path) -> String {
    folder.to_string_lossy().to_lowercase()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    use super::*;

    const SPACE: Space = Space {
        total: 100.0,
        free: 40.0,
    };

    /// A reader whose reads each wait for a message on the returned sender, and count themselves.
    fn gated() -> (SpaceReader, Sender<()>, Arc<AtomicUsize>) {
        let (open, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let reads = Arc::new(AtomicUsize::new(0));
        let counted = reads.clone();
        let reader = SpaceReader::with(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            lock(&gate).recv_timeout(Duration::from_secs(10)).ok()?;
            Some(SPACE)
        });
        (reader, open, reads)
    }

    /// Waits until `done` holds, for up to 5 s.
    fn until(done: impl Fn() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn idle(reader: &SpaceReader) -> bool {
        lock(&reader.state).running.is_empty()
    }

    #[test]
    fn asks_while_a_read_runs_wait_for_it() {
        let (reader, open, reads) = gated();
        let folder = Path::new(r"\\?\UNC\server\share\Projects");

        let start = Instant::now();
        assert_eq!(reader.read(folder, Duration::from_millis(50)), None);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "waited only so long"
        );

        // The server hasn't answered: asking again (the case may differ) starts no second read.
        let again = reader.clone();
        let waiter = std::thread::spawn(move || {
            again.read(
                Path::new(r"\\?\UNC\SERVER\share\projects"),
                Duration::from_secs(5),
            )
        });
        until(|| {
            lock(&reader.state)
                .running
                .values()
                .any(|waiting| waiting.len() == 2)
        });
        open.send(()).unwrap();
        assert_eq!(waiter.join().unwrap(), Some(SPACE));
        assert_eq!(reads.load(Ordering::SeqCst), 1);

        // Done, and heard: the next ask reads again.
        until(|| idle(&reader));
        open.send(()).unwrap();
        assert_eq!(reader.read(folder, Duration::from_secs(5)), Some(SPACE));
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_late_answer_goes_to_the_next_ask() {
        let (reader, open, reads) = gated();
        let folder = Path::new(r"\\?\UNC\slow\share");
        assert_eq!(reader.read(folder, Duration::from_millis(20)), None);
        open.send(()).unwrap();
        until(|| idle(&reader));

        // The answer nobody waited for comes at once, and a new read starts for the next ask.
        let start = Instant::now();
        assert_eq!(reader.read(folder, Duration::from_secs(5)), Some(SPACE));
        assert!(start.elapsed() < Duration::from_secs(2));
        until(|| reads.load(Ordering::SeqCst) == 2);
        open.send(()).unwrap();
        until(|| idle(&reader));
        assert_eq!(reader.read(folder, Duration::from_millis(20)), Some(SPACE));
    }

    #[test]
    fn other_folders_read_apart() {
        let (reader, open, reads) = gated();
        assert_eq!(
            reader.read(Path::new(r"\\?\UNC\a\share"), Duration::from_millis(20)),
            None
        );
        // The first read still waits for its server; another folder doesn't wait behind it.
        let other = reader.clone();
        let waiter = std::thread::spawn(move || {
            other.read(Path::new(r"\\?\UNC\b\share"), Duration::from_secs(5))
        });
        until(|| reads.load(Ordering::SeqCst) == 2);
        open.send(()).unwrap();
        open.send(()).unwrap();
        assert_eq!(waiter.join().unwrap(), Some(SPACE));
        until(|| idle(&reader));
    }

    #[test]
    fn a_panicking_read_fails_alone() {
        let fail = Arc::new(Mutex::new(true));
        let failing = fail.clone();
        let reader = SpaceReader::with(move |_| {
            assert!(!*lock(&failing), "deliberate panic");
            Some(SPACE)
        });
        let folder = Path::new(r"C:\Work");
        assert_eq!(reader.read(folder, Duration::from_secs(5)), None);
        *lock(&fail) = false;
        until(|| idle(&reader));
        assert_eq!(reader.read(folder, Duration::from_secs(5)), Some(SPACE));
    }

    #[test]
    fn a_folder_has_the_space_of_its_volume() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("ქართული");
        std::fs::create_dir(&folder).unwrap();
        let space = folder_space(&folder).expect("a local folder's volume answers");
        assert!(space.total > 0.0 && space.free > 0.0 && space.free <= space.total);
        // Its long path too, with a trailing backslash or without.
        let long = long_path(&folder).unwrap();
        assert!(folder_space(&long).is_some());
        assert!(folder_space(&long.join("")).is_some());
        assert_eq!(folder_space(&folder.join("gone")), None);
    }
}
