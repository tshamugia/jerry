//! Thumbnails as Explorer shows them (Tech PRD «აიკონები და thumbnail-ები»), as PNGs for the
//! `thumb://` protocol.
//!
//! A thumbnail comes from `IShellItemImageFactory::GetImage`: Windows' thumbcache and the type's
//! thumbnail handler, the ones Explorer uses (images, PDF, DWG where AutoCAD or DWG TrueView set
//! one up). Only thumbnails are asked for: a file whose type has none fails with `NotFound`, and
//! the UI keeps showing its type icon. Folders and paths longer than `MAX_PATH` have none.
//!
//! A cloud-only file (OneDrive "online only") is asked with `SIIGBF_INCACHEONLY`: only a
//! thumbnail Windows already has is shown, and the file's content is never read, which would
//! download it. That goes by the listing's attributes and by the file's attributes now, which
//! reading never downloads.
//!
//! Thumbnails run on a [`ShellPool`] of their own, started on first use, so slow ones never hold
//! up icons, each with a [`TIMEOUT`]. A hung handler is left behind on its thread, and the pool
//! replaces the worker. Each timeout leaves a thread behind, so a type whose thumbnails time out
//! [`HUNG_AFTER`] times is not asked again while Jerry runs: its files show their icon.
//!
//! PNGs are kept in memory only (Jerry has no disk cache; Windows' thumbcache is the one on
//! disk), by path, modified time, cloud state and size: WebView2 asks again for rows that come
//! back into view, and those are answered without a Shell call.

mod extract;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use jerry_core::{ErrorCode, JerryError};
use windows::Win32::Foundation::ERROR_TIMEOUT;

use crate::icons::keys::CLOUD_ONLY;
use crate::icons::shell::{is_cloud_only, shell_path};
use crate::shell_pool::ShellPool;

/// Smallest and largest thumbnails served, in device pixels (the box they fit in).
pub const MIN_PX: u32 = 16;
pub const MAX_PX: u32 = 1024;

/// How long one thumbnail may take. A large PDF or video can take a few seconds the first
/// time; after that it comes from the thumbcache. Past this the handler counts as hung.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Timeouts after which a type isn't asked for thumbnails again while Jerry runs. More than one,
/// so that one huge file on a slow share doesn't cost every file of its type its thumbnail.
pub const HUNG_AFTER: u32 = 2;

/// PNGs kept in memory; past this the cache starts over. A 96 px photo is ~20 KB, so this holds
/// a few screens of a grid.
const CACHE_BYTES: usize = 32 << 20;

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// The file to show a thumbnail of: its exact path, attributes and last write time (FILETIME),
/// as a listing or a conflict has them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbSource {
    pub path: PathBuf,
    pub attributes: u32,
    pub modified: u64,
}

impl ThumbSource {
    fn cloud_only(&self) -> bool {
        self.attributes & CLOUD_ONLY != 0
    }

    fn folder(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
    }
}

/// What a PNG in memory was made from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    path: PathBuf,
    modified: u64,
    cloud_only: bool,
    px: u32,
}

impl Key {
    fn of(source: &ThumbSource, px: u32) -> Self {
        Self {
            path: source.path.clone(),
            modified: source.modified,
            cloud_only: source.cloud_only(),
            px,
        }
    }
}

/// What the Shell is asked for one thumbnail.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    /// The path as the Shell takes it, NUL-terminated.
    path: Vec<u16>,
    /// The box the thumbnail fits in, in device pixels.
    px: u32,
    /// Only a thumbnail Windows already has: the file is cloud-only.
    cache_only: bool,
}

/// Makes one thumbnail on a Shell pool worker: the Shell's in Jerry, a fake in tests.
type Extract = dyn Fn(&Request) -> Result<Vec<u8>, JerryError> + Send + Sync;

/// Jerry's thumbnails: their Shell pool and the PNGs in memory.
pub struct Thumbnails {
    pool: OnceLock<ShellPool>,
    timeout: Duration,
    shared: Arc<Shared>,
}

struct Shared {
    extract: Box<Extract>,
    images: Mutex<Images>,
    /// Lowercase extension → how many of its thumbnails timed out.
    timeouts: Mutex<HashMap<String, u32>>,
}

#[derive(Default)]
struct Images {
    png: HashMap<Key, Arc<[u8]>>,
    bytes: usize,
}

impl Thumbnails {
    /// Nothing starts until the first thumbnail is asked for.
    pub fn new() -> Self {
        Self::with(TIMEOUT, extract::thumbnail)
    }

    fn with(
        timeout: Duration,
        extract: impl Fn(&Request) -> Result<Vec<u8>, JerryError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            pool: OnceLock::new(),
            timeout,
            shared: Arc::new(Shared {
                extract: Box::new(extract),
                images: Mutex::default(),
                timeouts: Mutex::default(),
            }),
        }
    }

    /// The PNG for `source` at `px` if it is in memory, with no Shell call. Safe on any thread,
    /// the main thread included.
    pub fn cached(&self, source: &ThumbSource, px: u32) -> Option<Arc<[u8]>> {
        lock(&self.shared.images)
            .png
            .get(&Key::of(source, clamp(px)))
            .cloned()
    }

    /// Makes the thumbnail of `source` to fit `px` × `px` on the Shell pool and calls `done`
    /// there with the PNG, or with the error: `NotFound` when there is none (a folder, a type
    /// with no thumbnail, a cloud-only file whose thumbnail Windows doesn't have, a long path),
    /// `ERROR_TIMEOUT` (as `Os`) for a hung handler or a type that hung before. `done` is called
    /// at once, on this thread, when there is nothing to ask the Shell.
    pub fn render(
        &self,
        source: ThumbSource,
        px: u32,
        done: impl FnOnce(Result<Arc<[u8]>, JerryError>) + Send + 'static,
    ) {
        let px = clamp(px);
        if source.folder() {
            done(Err(JerryError::new(ErrorCode::NotFound)));
            return;
        }
        let kind = kind_of(&source.path);
        if self.shared.hung(&kind) {
            done(Err(JerryError::from_win32(ERROR_TIMEOUT.0)));
            return;
        }
        let (shared, failures) = (self.shared.clone(), self.shared.clone());
        self.pool().submit(
            "thumbnail",
            self.timeout,
            move || shared.render(&source, px),
            move |result| {
                if let Err(error) = &result {
                    if error.code
                        == (ErrorCode::Os {
                            os: ERROR_TIMEOUT.0,
                        })
                    {
                        failures.timed_out(kind);
                    }
                    tracing::debug!(code = ?error.code, "no thumbnail");
                }
                done(result);
            },
        );
    }

    fn pool(&self) -> &ShellPool {
        self.pool.get_or_init(ShellPool::new)
    }
}

impl Default for Thumbnails {
    fn default() -> Self {
        Self::new()
    }
}

impl Shared {
    /// Runs on a Shell pool worker.
    fn render(&self, source: &ThumbSource, px: u32) -> Result<Arc<[u8]>, JerryError> {
        let path = shell_path(&source.path).ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
        // The listing's attributes can be older than the file: one made online-only since is
        // asked from the cache only too.
        let cache_only = source.cloud_only() || is_cloud_only(&source.path);
        let png: Arc<[u8]> = (self.extract)(&Request {
            path,
            px,
            cache_only,
        })?
        .into();
        let mut images = lock(&self.images);
        if images.bytes + png.len() > CACHE_BYTES {
            *images = Images::default();
        }
        images.bytes += png.len();
        images.png.insert(Key::of(source, px), png.clone());
        Ok(png)
    }

    fn hung(&self, kind: &str) -> bool {
        lock(&self.timeouts)
            .get(kind)
            .is_some_and(|&count| count >= HUNG_AFTER)
    }

    fn timed_out(&self, kind: String) {
        let mut timeouts = lock(&self.timeouts);
        let count = timeouts.entry(kind).or_default();
        *count += 1;
        if *count == HUNG_AFTER {
            tracing::warn!(
                count = *count,
                "thumbnails of a type keep timing out; its files show their icon from now on"
            );
        }
    }
}

/// What a hung handler is counted by: the file's extension, lowercase ("" for none).
fn kind_of(path: &Path) -> String {
    path.extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn clamp(px: u32) -> u32 {
    px.clamp(MIN_PX, MAX_PX)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
