//! File, drive and standard-folder icons, as Explorer shows them (Tech PRD «აიკონები და
//! thumbnail-ები»), rendered as PNGs for the `icon://` protocol.
//!
//! A file's icon comes from its type: `SHGetFileInfoW` with `SHGFI_USEFILEATTRIBUTES`, by
//! extension, with no disk access. Only .exe, .lnk, .ico and .url files have their own, read
//! from the file (see [`keys`] for the icon key the listing sends). Lookups and rendering run on
//! the [`ShellPool`] with a timeout. Results are cached in memory only, by system image list
//! index and size: every type that shares an icon shares the PNG, and a type icon rendered once
//! is served again without any Shell call.

pub mod keys;
mod render;
mod shell;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use jerry_core::{ErrorCode, JerryError};

pub use self::shell::Place;
use self::shell::{Glyph, drive_glyph, file_glyph, place_glyph, type_glyph};
use crate::shell_pool::ShellPool;

/// Smallest and largest icon sizes served, in device pixels.
pub const MIN_PX: u32 = 8;
pub const MAX_PX: u32 = 256;

/// How long one icon may take on the Shell pool: a type icon takes a few ms, a program on a slow
/// share can take a while. Past this a shell extension counts as hung.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Rendered icons kept in memory. Past this the cache starts over: a few MB holds every icon a
/// session shows at one size.
const CACHE_BYTES: usize = 32 << 20;

/// Files whose own icon is kept in memory; past this, that cache starts over.
const CACHE_FILES: usize = 16_384;

/// Which icon to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconSource {
    /// A type icon, by the key from the listing batch ([`keys::key`]).
    Type(u32),
    /// An entry with its own icon ([`keys::OWN`]): its exact path, attributes and last write
    /// time (a FILETIME). A file whose time or cloud state changed is looked up anew.
    File {
        path: PathBuf,
        attributes: u32,
        modified: u64,
    },
    /// A drive, by letter (`b'C'`).
    Drive(u8),
    /// A standard place for the sidebar.
    Place(Place),
}

/// Counters for the dev panel: whether scrolling is served from memory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IconStats {
    /// Icons served from memory, with no Shell call.
    pub from_memory: u64,
    /// Icons sent to the Shell pool.
    pub from_shell: u64,
    /// PNGs rendered (the rest of `from_shell` found theirs in memory).
    pub rendered: u64,
    pub failed: u64,
}

/// Jerry's icons: the Shell pool and the in-memory cache.
pub struct Icons {
    pool: ShellPool,
    cache: Arc<Cache>,
}

#[derive(Default)]
struct Cache {
    /// Type key → its icon. Types don't change their icon while Jerry runs.
    types: Mutex<HashMap<u32, Glyph>>,
    /// Path → the file's icon, for the version of the file it was read from. WebView2 doesn't
    /// keep icons of rows that scrolled away, so rows that come back ask again.
    files: Mutex<HashMap<PathBuf, (FileVersion, Glyph)>>,
    images: Mutex<Images>,
    from_memory: AtomicU64,
    from_shell: AtomicU64,
    rendered: AtomicU64,
    failed: AtomicU64,
}

/// What a file's own icon depends on besides its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileVersion {
    modified: u64,
    cloud_only: bool,
}

impl FileVersion {
    fn of(attributes: u32, modified: u64) -> Self {
        Self {
            modified,
            cloud_only: attributes & keys::CLOUD_ONLY != 0,
        }
    }
}

#[derive(Default)]
struct Images {
    png: HashMap<(Glyph, u32), Arc<[u8]>>,
    bytes: usize,
}

impl Icons {
    /// Starts the Shell pool. Nothing is rendered until asked.
    pub fn new() -> Self {
        Self {
            pool: ShellPool::new(),
            cache: Arc::default(),
        }
    }

    /// The PNG for `source` at `px` if it is in memory, with no Shell call: a type or file icon
    /// rendered before. Safe on any thread, the main thread included.
    pub fn cached(&self, source: &IconSource, px: u32) -> Option<Arc<[u8]>> {
        let glyph = match source {
            IconSource::Type(key) => *lock(&self.cache.types).get(key)?,
            IconSource::File {
                path,
                attributes,
                modified,
            } => {
                let files = lock(&self.cache.files);
                let (version, glyph) = files.get(path)?;
                (*version == FileVersion::of(*attributes, *modified)).then_some(*glyph)?
            }
            IconSource::Drive(_) | IconSource::Place(_) => return None,
        };
        let png = self.cache.image(glyph, clamp(px))?;
        self.cache.from_memory.fetch_add(1, Ordering::Relaxed);
        Some(png)
    }

    /// Renders `source` at `px` on the Shell pool and calls `done` there with the PNG, or with
    /// the error: `NotFound` for a key, drive or place that doesn't exist, `ERROR_TIMEOUT` (as
    /// `Os`) for a hung shell extension.
    pub fn render(
        &self,
        source: IconSource,
        px: u32,
        done: impl FnOnce(Result<Arc<[u8]>, JerryError>) + Send + 'static,
    ) {
        self.cache.from_shell.fetch_add(1, Ordering::Relaxed);
        let cache = self.cache.clone();
        let failures = self.cache.clone();
        let px = clamp(px);
        self.pool.submit(
            "icon",
            TIMEOUT,
            move || cache.render(&source, px),
            move |result| {
                if let Err(error) = &result {
                    failures.failed.fetch_add(1, Ordering::Relaxed);
                    tracing::debug!(code = ?error.code, "icon not rendered");
                }
                done(result);
            },
        );
    }

    pub fn stats(&self) -> IconStats {
        let count = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        IconStats {
            from_memory: count(&self.cache.from_memory),
            from_shell: count(&self.cache.from_shell),
            rendered: count(&self.cache.rendered),
            failed: count(&self.cache.failed),
        }
    }
}

impl Default for Icons {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    /// Runs on a Shell pool worker.
    fn render(&self, source: &IconSource, px: u32) -> Result<Arc<[u8]>, JerryError> {
        shell::init();
        let glyph = match source {
            IconSource::Type(key) => self.type_glyph(*key)?,
            IconSource::File {
                path,
                attributes,
                modified,
            } => {
                let glyph = file_glyph(path, *attributes)?;
                let mut files = lock(&self.files);
                if files.len() >= CACHE_FILES {
                    files.clear();
                }
                files.insert(
                    path.clone(),
                    (FileVersion::of(*attributes, *modified), glyph),
                );
                glyph
            }
            IconSource::Drive(letter) => drive_glyph(*letter)?,
            IconSource::Place(place) => place_glyph(*place)?,
        };
        if let Some(png) = self.image(glyph, px) {
            return Ok(png);
        }
        let png: Arc<[u8]> = render::png(glyph, px)?.into();
        self.rendered.fetch_add(1, Ordering::Relaxed);
        let mut images = lock(&self.images);
        if images.bytes + png.len() > CACHE_BYTES {
            *images = Images::default();
        }
        images.bytes += png.len();
        images.png.insert((glyph, px), png.clone());
        Ok(png)
    }

    fn type_glyph(&self, key: u32) -> Result<Glyph, JerryError> {
        if let Some(&glyph) = lock(&self.types).get(&key) {
            return Ok(glyph);
        }
        let icon = keys::icon_type(key).ok_or_else(|| JerryError::new(ErrorCode::NotFound))?;
        let glyph = type_glyph(&icon)?;
        lock(&self.types).insert(key, glyph);
        Ok(glyph)
    }

    fn image(&self, glyph: Glyph, px: u32) -> Option<Arc<[u8]>> {
        lock(&self.images).png.get(&(glyph, px)).cloned()
    }
}

fn clamp(px: u32) -> u32 {
    px.clamp(MIN_PX, MAX_PX)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::mpsc;

    use super::keys::{FILE, FOLDER, OWN, key};
    use super::*;

    fn render(icons: &Icons, source: IconSource, px: u32) -> Result<Arc<[u8]>, JerryError> {
        let (sender, result) = mpsc::channel();
        icons.render(source, px, move |png| {
            let _ = sender.send(png);
        });
        result
            .recv_timeout(Duration::from_secs(30))
            .expect("rendered")
    }

    /// Width and height from a PNG's IHDR chunk.
    fn png_size(png: &[u8]) -> (u32, u32) {
        assert_eq!(png[..8], [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        let word = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().expect("4 bytes"));
        (word(16), word(20))
    }

    fn key_of(name: &str) -> u32 {
        key(&name.encode_utf16().collect::<Vec<_>>(), 0x20)
    }

    fn glyph_of(path: &Path, attributes: u32) -> Glyph {
        let pool = ShellPool::new();
        let path = path.to_path_buf();
        pool.run("glyph", TIMEOUT, move || file_glyph(&path, attributes))
            .expect("a glyph")
    }

    #[test]
    fn first_requests_on_every_worker_at_once_all_succeed() {
        // nextest runs each test in its own process: these are the process's first Shell calls,
        // on every worker at once, as when Jerry's window first shows its sidebar.
        let icons = Icons::new();
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let drive = IconSource::Drive(system.as_bytes()[0]);
        let mut sources = vec![
            drive.clone(),
            IconSource::Place(Place::RecycleBin),
            IconSource::Place(Place::NetworkShare),
            IconSource::Type(key_of("a.pdf")),
            IconSource::Type(FOLDER),
        ];
        sources.extend(Place::ALL.map(IconSource::Place));
        sources.push(drive);
        let (sender, results) = mpsc::channel();
        for source in sources {
            let sender = sender.clone();
            let asked = source.clone();
            icons.render(source, 16, move |png| {
                let _ = sender.send((asked, png.map(|_| ()).map_err(|error| error.code)));
            });
        }
        drop(sender);
        let failed: Vec<_> = results
            .iter()
            .filter(|(_, result)| result.is_err())
            .collect();
        assert!(failed.is_empty(), "{failed:?}");
    }

    fn type_glyph_of(name: &str) -> Glyph {
        let pool = ShellPool::new();
        let name: Vec<u16> = name.encode_utf16().collect();
        pool.run("glyph", TIMEOUT, move || {
            type_glyph(&keys::icon_type_of(&name, 0x20))
        })
        .expect("a glyph")
    }

    #[test]
    fn type_icons_render_at_the_asked_size_and_then_come_from_memory() {
        let icons = Icons::new();
        let txt = IconSource::Type(key_of("ჩანაწერი.txt"));
        for px in [16, 20, 24, 32, 48, 64, 256] {
            let png = render(&icons, txt.clone(), px).expect("a PNG");
            assert_eq!(png_size(&png), (px, px));
        }
        let folder = render(&icons, IconSource::Type(FOLDER), 16).expect("folder");
        let txt16 = icons.cached(&txt, 16).expect("in memory");
        assert_ne!(folder, txt16);
        assert!(
            icons.cached(&IconSource::Type(FILE), 16).is_none(),
            "not rendered yet"
        );

        let stats = icons.stats();
        assert_eq!(stats.from_shell, 8);
        assert_eq!(stats.from_memory, 1);
        assert_eq!(stats.failed, 0);
    }

    #[test]
    fn unknown_keys_drives_and_places_fail_with_not_found() {
        let icons = Icons::new();
        let code = |source| {
            render(&icons, source, 16)
                .map(|_| ())
                .map_err(|error| error.code)
        };
        assert_eq!(code(IconSource::Type(OWN)), Err(ErrorCode::NotFound));
        assert_eq!(code(IconSource::Type(u32::MAX)), Err(ErrorCode::NotFound));
        // SAFETY: no arguments.
        let drives = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        let missing = (0..26)
            .rev()
            .find(|bit| drives & (1 << bit) == 0)
            .expect("a free letter");
        assert_eq!(
            code(IconSource::Drive(b'A' + missing as u8)),
            Err(ErrorCode::NotFound)
        );
        assert_eq!(icons.stats().failed, 3);
    }

    #[test]
    fn the_system_drive_and_standard_places_have_icons() {
        let icons = Icons::new();
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let letter = system.as_bytes()[0];
        let drive = render(&icons, IconSource::Drive(letter), 32).expect("system drive");
        assert_eq!(png_size(&drive), (32, 32));
        for place in [
            Place::Desktop,
            Place::Documents,
            Place::Downloads,
            Place::Pictures,
            Place::RecycleBin,
            Place::NetworkShare,
        ] {
            let png = render(&icons, IconSource::Place(place), 24);
            assert_eq!(png_size(&png.expect("an icon")), (24, 24), "{place:?}");
        }
        assert!(
            Place::ALL
                .iter()
                .all(|&place| Place::from_name(place.name()) == Some(place))
        );
    }

    #[test]
    fn a_url_shortcut_has_its_own_icon_unless_it_would_come_from_the_network() {
        let dir = tempfile::tempdir().expect("temp folder");
        let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let write = |name: &str, icon: &str| {
            let path = dir.path().join(name);
            let text = format!(
                "[InternetShortcut]\r\nURL=https://example.com/\r\nIconFile={icon}\r\nIconIndex=3\r\n"
            );
            std::fs::write(&path, text).expect("write");
            path
        };
        let by_type = type_glyph_of("x.url");
        assert!(by_type.link, ".url files get the shortcut arrow");

        let local = glyph_of(
            &write(
                "ადგილობრივი.url",
                &format!(r"{system}\System32\shell32.dll"),
            ),
            0x20,
        );
        assert_ne!(local.index, by_type.index, "its own icon");
        assert!(local.link);

        for (name, icon) in [
            ("share.url", r"\\jerry-test-server\share\icon.ico"),
            ("long-unc.url", r"\\?\UNC\jerry-test-server\share\icon.ico"),
            ("web.url", "https://example.com/favicon.ico"),
        ] {
            assert_eq!(glyph_of(&write(name, icon), 0x20), by_type, "{name}");
        }
    }

    #[test]
    fn a_file_icon_comes_from_memory_until_the_file_changes() {
        let dir = tempfile::tempdir().expect("temp folder");
        let path = dir.path().join("ნახაზები.url");
        std::fs::write(&path, "[InternetShortcut]\r\nURL=https://example.com/\r\n").expect("write");
        let source = |attributes, modified| IconSource::File {
            path: path.clone(),
            attributes,
            modified,
        };
        let icons = Icons::new();
        assert!(icons.cached(&source(0x20, 1), 16).is_none());

        render(&icons, source(0x20, 1), 16).expect("rendered");

        assert!(
            icons.cached(&source(0x20, 1), 16).is_some(),
            "again, from memory"
        );
        assert!(icons.cached(&source(0x20, 1), 32).is_none(), "another size");
        assert!(
            icons.cached(&source(0x20, 2), 16).is_none(),
            "the file changed"
        );
        assert!(
            icons.cached(&source(0x40_0020, 1), 16).is_none(),
            "now cloud-only"
        );
    }

    #[test]
    fn a_cloud_only_program_shows_its_type_icon() {
        // The file doesn't even exist: with the cloud-only attribute it is never opened.
        let path = Path::new(r"C:\jerry-test-missing\setup.exe");
        let recall_on_data_access = 0x40_0000;
        assert_eq!(
            glyph_of(path, 0x20 | recall_on_data_access),
            type_glyph_of("x.exe")
        );
    }

    #[test]
    fn long_paths_show_their_type_icon() {
        let long = format!(r"\\?\C:\{}\setup.exe", "ა".repeat(300));
        let wide: Vec<u16> = Path::new(&long).as_os_str().encode_wide().collect();
        assert!(wide.len() > 260);
        assert_eq!(glyph_of(Path::new(&long), 0x20), type_glyph_of("x.exe"));
    }

    #[test]
    fn shortcut_icons_get_the_arrow() {
        let icons = Icons::new();
        let lnk = IconSource::Type(key(
            &"x.lnk".encode_utf16().collect::<Vec<_>>(),
            0x20 | 0x40_0000,
        ));
        let plain = IconSource::Type(key_of("x.txt"));
        for px in [16, 32, 48] {
            let arrow = render(&icons, lnk.clone(), px).expect("lnk");
            assert_eq!(png_size(&arrow), (px, px));
            assert_ne!(arrow, render(&icons, plain.clone(), px).expect("txt"));
        }
    }
}
