//! Drives for the sidebar (Tech PRD «დისკები», #34): which drive letters exist, what each one is,
//! its label and free space, kept current by a [`DriveMonitor`].
//!
//! The work comes in two kinds, kept apart so that a slow drive never holds up the others, the
//! monitor or the UI:
//! - The scan ([`Source::scan`]) asks local tables only: the drive letters (`GetLogicalDrives`),
//!   each one's type (`GetDriveTypeW`) and, for a network drive, its share from the local
//!   connection table (`WNetGetConnectionW`). It touches no disk and no server, and runs every
//!   [`SCAN_EVERY`] and, more often for a moment, after Windows reports a device change
//!   ([`DriveMonitor::refresh`]): a USB drive gets its letter shortly after its volume arrives.
//! - A probe ([`Source::probe`]) reads a local volume: label, size, free space, and whether it
//!   can be ejected. An optical drive spinning up or a stuck virtual drive can take seconds, so
//!   each probe runs on a thread of its own and a drive has at most one at a time. Probes repeat
//!   every [`PROBE_EVERY`]. Network drives are never probed: the sidebar shows their share and
//!   server, which the connection table already has, so a slow server can't freeze anything.

use std::collections::HashMap;
use std::mem::offset_of;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use specta::Type;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_CONNECTION_UNAVAIL, ERROR_MORE_DATA, NO_ERROR,
};
use windows::Win32::NetworkManagement::WNet::WNetGetConnectionW;
use windows::Win32::Storage::FileSystem::{
    BusType1394, BusTypeMmc, BusTypeSd, BusTypeUsb, CreateFileW, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_SHARE_READ, FILE_SHARE_WRITE, GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives,
    GetVolumeInformationW, OPEN_EXISTING,
};
use windows::Win32::System::Diagnostics::Debug::{
    SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX, SetThreadErrorMode,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR,
    STORAGE_PROPERTY_QUERY, StorageDeviceProperty,
};
use windows::core::{PCWSTR, PWSTR};

use crate::error::{ErrorCode, JerryError};
use crate::job::run_job;
use crate::path::long_path;

/// How often the drive letters are read again without a device notice: often enough to see a
/// network drive mapped or disconnected elsewhere (Windows sends Jerry's window no notice for
/// those), and cheap, since a scan reads local tables only.
pub const SCAN_EVERY: Duration = Duration::from_secs(2);

/// How often a local drive's label and free space are read again.
pub const PROBE_EVERY: Duration = Duration::from_secs(10);

/// After a device notice: how many quick scans, and how far apart. A new volume gets its letter
/// a moment after Windows reports it.
const BURST_SCANS: u32 = 4;
const BURST_EVERY: Duration = Duration::from_millis(400);

/// What kind of drive a letter is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DriveKind {
    /// A local disk.
    Fixed,
    /// Removable media: a USB stick, a memory card.
    Removable,
    /// A CD, DVD or Blu-ray drive.
    Optical,
    /// A mapped network share.
    Network,
    /// A RAM disk.
    Ram,
}

/// A drive as the sidebar shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Drive {
    /// The drive letter, upper case: `"C"`.
    pub letter: String,
    pub kind: DriveKind,
    /// The volume label; for a network drive, its share's name. None without one, or before the
    /// drive has been read.
    pub label: Option<String>,
    /// A network drive's share as display text (`\\server\share`).
    pub remote: Option<String>,
    /// False for a network drive Windows remembers but isn't connected to now.
    pub connected: bool,
    /// The drive can be ejected: removable media, an optical drive, a USB disk.
    pub ejectable: bool,
    /// Size and free space, once a local drive has been read with media in it.
    pub space: Option<Space>,
}

/// A volume's size and the space free on it for the user, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Type)]
pub struct Space {
    pub total: f64,
    pub free: f64,
}

/// The drives, numbered: a later list has a higher generation, so a window that hears of the
/// drives two ways (`drives_list` and `drives:changed`) keeps the newest.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct DriveList {
    pub generation: u32,
    pub drives: Vec<Drive>,
}

/// A drive letter as the scan finds it, from local tables only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scanned {
    /// `b'A'` to `b'Z'`.
    pub letter: u8,
    pub kind: DriveKind,
    /// A network drive's share (`\\server\share`).
    pub remote: Option<String>,
    /// False for a network drive that is remembered but not connected.
    pub connected: bool,
}

/// What a probe read from a local volume.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Probed {
    /// The volume answered: media in the drive.
    pub ready: bool,
    pub label: Option<String>,
    pub space: Option<Space>,
    pub ejectable: bool,
}

/// Where the drives come from: [`System`] in Jerry, a fake in tests.
pub trait Source: Send + Sync + 'static {
    /// The drive letters there are now, in letter order. Fast: local tables only.
    fn scan(&self) -> Vec<Scanned>;

    /// Reads a local drive. Can block for seconds. `before` is the drive's previous probe, if
    /// any: what doesn't change (whether it can be ejected) needn't be asked again.
    fn probe(&self, drive: &Scanned, before: Option<&Probed>) -> Probed;
}

/// Keeps the drive list current and reports every change. Dropping it stops it: its thread
/// exits, and probes still running finish into nothing.
pub struct DriveMonitor {
    shared: Arc<Shared>,
}

struct Shared {
    source: Box<dyn Source>,
    state: Mutex<State>,
    wake: Condvar,
    /// The list last reported. Held while reporting, so reports go out in order.
    reported: Mutex<DriveList>,
    on_change: Box<dyn Fn(&DriveList) + Send + Sync>,
    timing: Timing,
}

#[derive(Clone, Copy)]
struct Timing {
    scan: Duration,
    probe: Duration,
    burst: Duration,
}

#[derive(Default)]
struct State {
    /// The last scan; None before the first.
    scanned: Option<Vec<Scanned>>,
    /// Each local drive's probe, by letter.
    slots: HashMap<u8, Slot>,
    /// Quick scans still to come after a device notice.
    burst: u32,
    /// Scan now, then probe every local drive again.
    refresh: bool,
    stopped: bool,
}

/// One drive's probes. A new slot (a drive that came back, or changed kind) has a new number:
/// a probe still running for the old one can't write into it.
struct Slot {
    number: u64,
    kind: DriveKind,
    probed: Option<Probed>,
    at: Option<Instant>,
    running: bool,
    stale: bool,
}

static SLOT_NUMBERS: AtomicU64 = AtomicU64::new(0);

impl DriveMonitor {
    /// Starts following the drives of `source`. `on_change` gets every new list, in order, on
    /// the monitor's or a probe's thread; it must not call back into the monitor.
    pub fn start(
        source: impl Source,
        on_change: impl Fn(&DriveList) + Send + Sync + 'static,
    ) -> Self {
        Self::with_timing(
            source,
            on_change,
            Timing {
                scan: SCAN_EVERY,
                probe: PROBE_EVERY,
                burst: BURST_EVERY,
            },
        )
    }

    fn with_timing(
        source: impl Source,
        on_change: impl Fn(&DriveList) + Send + Sync + 'static,
        timing: Timing,
    ) -> Self {
        let shared = Arc::new(Shared {
            source: Box::new(source),
            state: Mutex::default(),
            wake: Condvar::new(),
            reported: Mutex::default(),
            on_change: Box::new(on_change),
            timing,
        });
        let monitor = shared.clone();
        let started = std::thread::Builder::new()
            .name("drives".into())
            .spawn(move || watch(&monitor));
        if let Err(error) = started {
            tracing::error!(%error, "the drive monitor didn't start: the sidebar shows no drives");
        }
        Self { shared }
    }

    /// The drives as last reported; generation 0 and no drives before the first scan.
    pub fn drives(&self) -> DriveList {
        lock(&self.shared.reported).clone()
    }

    /// Windows reported a device change, or a drive was ejected: scan now and a few times more
    /// shortly, and read every local drive again.
    pub fn refresh(&self) {
        let mut state = lock(&self.shared.state);
        state.refresh = true;
        state.burst = BURST_SCANS;
        self.shared.wake.notify_all();
    }
}

impl Drop for DriveMonitor {
    fn drop(&mut self) {
        lock(&self.shared.state).stopped = true;
        self.shared.wake.notify_all();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The monitor's thread: scan, start the probes that are due, report, wait.
fn watch(shared: &Arc<Shared>) {
    loop {
        // A panicking scan fails alone: the last scan stays.
        let scanned = run_job("drive scan", || Ok(shared.source.scan())).ok();
        let due = {
            let mut state = lock(&shared.state);
            if state.stopped {
                return;
            }
            let refresh = std::mem::take(&mut state.refresh);
            if let Some(scanned) = scanned {
                state.scanned = Some(scanned);
            }
            due_probes(&mut state, refresh, shared.timing.probe)
        };
        for (drive, before, number) in due {
            probe(shared, drive, before, number);
        }
        report(shared);

        let mut state = lock(&shared.state);
        if !state.refresh && !state.stopped {
            let wait = if state.burst > 0 {
                state.burst -= 1;
                shared.timing.burst
            } else {
                shared.timing.scan
            };
            state = shared
                .wake
                .wait_timeout(state, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        if state.stopped {
            return;
        }
    }
}

/// Brings the probe slots in line with the scan and returns the probes to start: each local
/// drive without a probe running whose last one is older than `every`, or all of them on
/// `refresh`.
fn due_probes(
    state: &mut State,
    refresh: bool,
    every: Duration,
) -> Vec<(Scanned, Option<Probed>, u64)> {
    let State { scanned, slots, .. } = state;
    let Some(scanned) = scanned else {
        return Vec::new();
    };
    slots.retain(|letter, slot| {
        scanned
            .iter()
            .any(|drive| drive.letter == *letter && drive.kind == slot.kind)
    });
    let mut due = Vec::new();
    for drive in scanned
        .iter()
        .filter(|drive| drive.kind != DriveKind::Network)
    {
        let slot = slots.entry(drive.letter).or_insert_with(|| Slot {
            number: SLOT_NUMBERS.fetch_add(1, Ordering::Relaxed),
            kind: drive.kind,
            probed: None,
            at: None,
            running: false,
            stale: false,
        });
        slot.stale |= refresh;
        let old = slot.at.is_none_or(|at| at.elapsed() >= every);
        if !slot.running && (slot.stale || old) {
            slot.running = true;
            slot.stale = false;
            due.push((drive.clone(), slot.probed.clone(), slot.number));
        }
    }
    due
}

/// Reads `drive` on a thread of its own and reports what changed.
fn probe(shared: &Arc<Shared>, drive: Scanned, before: Option<Probed>, number: u64) {
    let letter = drive.letter;
    let monitor = shared.clone();
    let started = std::thread::Builder::new()
        .name("drive-probe".into())
        .spawn(move || {
            let probed = run_job("drive probe", || {
                Ok(monitor.source.probe(&drive, before.as_ref()))
            });
            {
                let mut state = lock(&monitor.state);
                if let Some(slot) = state.slots.get_mut(&letter)
                    && slot.number == number
                {
                    slot.running = false;
                    slot.at = Some(Instant::now());
                    if let Ok(probed) = probed {
                        slot.probed = Some(probed);
                    }
                }
            }
            report(&monitor);
        });
    if let Err(error) = started {
        tracing::warn!(%error, "a drive probe thread didn't start");
        if let Some(slot) = lock(&shared.state).slots.get_mut(&letter)
            && slot.number == number
        {
            slot.running = false;
            slot.at = Some(Instant::now());
        }
    }
}

/// Reports the drives if they changed since the last report (the first scan always reports).
fn report(shared: &Shared) {
    let mut reported = lock(&shared.reported);
    let drives = {
        let state = lock(&shared.state);
        match &state.scanned {
            Some(scanned) => list(scanned, &state.slots),
            None => return,
        }
    };
    if reported.generation > 0 && reported.drives == drives {
        return;
    }
    reported.generation += 1;
    reported.drives = drives;
    (shared.on_change)(&reported);
}

/// The drives to show. Removable media shows once read and ready: an empty card reader is hidden,
/// as in Explorer.
fn list(scanned: &[Scanned], slots: &HashMap<u8, Slot>) -> Vec<Drive> {
    scanned
        .iter()
        .filter_map(|drive| {
            let probed = slots
                .get(&drive.letter)
                .filter(|slot| slot.kind == drive.kind)
                .and_then(|slot| slot.probed.as_ref());
            let ready = probed.is_some_and(|probed| probed.ready);
            if drive.kind == DriveKind::Removable && !ready {
                return None;
            }
            let label = match drive.kind {
                DriveKind::Network => drive.remote.as_deref().and_then(share_name),
                _ => probed.and_then(|probed| probed.label.clone()),
            };
            Some(Drive {
                letter: char::from(drive.letter).to_string(),
                kind: drive.kind,
                label,
                remote: drive.remote.clone(),
                connected: drive.connected,
                ejectable: probed.is_some_and(|probed| probed.ejectable),
                space: probed.and_then(|probed| probed.space),
            })
        })
        .collect()
}

/// The share's name in a network path: `Projects` for `\\server\Projects` (and for
/// `\\server\Projects\2026`, its last part: `2026`, as Explorer names such a drive).
pub fn share_name(remote: &str) -> Option<String> {
    let rest = remote.strip_prefix(r"\\")?;
    let (_, path) = rest.split_once('\\')?;
    path.trim_end_matches('\\')
        .rsplit('\\')
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// The server in a network path: `server` for `\\server\share`.
pub fn server_name(remote: &str) -> Option<String> {
    let rest = remote.strip_prefix(r"\\")?;
    let server = rest.split('\\').next()?;
    (!server.is_empty()).then(|| server.to_owned())
}

/// A network location the user typed for the sidebar (`\\server\share`, also with `/`, quotes,
/// a trailing backslash or a folder below the share) as the exact path core keeps:
/// `\\?\UNC\server\share`. No network access: whether it answers shows when it is opened.
/// [`ErrorCode::InvalidName`] for anything that isn't a share.
pub fn network_place(text: &str) -> Result<PathBuf, JerryError> {
    let invalid = || JerryError::new(ErrorCode::InvalidName);
    let cleaned = text.trim().trim_matches('"').trim().replace('/', "\\");
    let unc = match cleaned.strip_prefix(r"\\?\UNC\") {
        Some(rest) => rest,
        None => cleaned.strip_prefix(r"\\").ok_or_else(invalid)?,
    };
    let parts: Vec<&str> = unc.trim_end_matches('\\').split('\\').collect();
    let bad = |part: &&str| {
        part.is_empty()
            || *part == "."
            || *part == ".."
            || part
                .chars()
                .any(|c| c.is_control() || r#"<>:"|?*"#.contains(c))
    };
    if parts.len() < 2 || parts.iter().any(bad) {
        return Err(invalid());
    }
    long_path(Path::new(&format!(r"\\{}", parts.join("\\"))))
}

/// The drives of this computer.
pub struct System;

/// `GetDriveTypeW` results.
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;
const DRIVE_RAMDISK: u32 = 6;

impl Source for System {
    fn scan(&self) -> Vec<Scanned> {
        // SAFETY: no arguments; returns the bitmask of drive letters.
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|bit| mask & (1 << bit) != 0)
            .filter_map(|bit| scan_letter(b'A' + bit))
            .collect()
    }

    fn probe(&self, drive: &Scanned, before: Option<&Probed>) -> Probed {
        quiet_errors();
        let root = wide(&format!("{}:\\", char::from(drive.letter)));
        let mut name = [0u16; 261];
        // SAFETY: a NUL-terminated root; the label buffer is writable for its length.
        let ready = unsafe {
            GetVolumeInformationW(
                PCWSTR(root.as_ptr()),
                Some(&mut name),
                None,
                None,
                None,
                None,
            )
        }
        .is_ok();
        let ejectable = match drive.kind {
            DriveKind::Removable | DriveKind::Optical => true,
            DriveKind::Fixed => {
                before.map_or_else(|| on_external_bus(drive.letter), |b| b.ejectable)
            }
            DriveKind::Network | DriveKind::Ram => false,
        };
        Probed {
            ready,
            label: if ready { text(&name) } else { None },
            space: if ready { space(&root) } else { None },
            ejectable,
        }
    }
}

fn scan_letter(letter: u8) -> Option<Scanned> {
    let root = wide(&format!("{}:\\", char::from(letter)));
    // SAFETY: a NUL-terminated root path.
    let kind = match unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } {
        DRIVE_FIXED => DriveKind::Fixed,
        DRIVE_REMOVABLE => DriveKind::Removable,
        DRIVE_CDROM => DriveKind::Optical,
        DRIVE_REMOTE => DriveKind::Network,
        DRIVE_RAMDISK => DriveKind::Ram,
        // Unknown, or no volume behind the letter.
        _ => return None,
    };
    let (remote, connected) = match kind {
        DriveKind::Network => connection(letter),
        _ => (None, true),
    };
    Some(Scanned {
        letter,
        kind,
        remote,
        connected,
    })
}

/// A network drive's share from the local connection table, and whether it is connected. No
/// network access.
pub(crate) fn connection(letter: u8) -> (Option<String>, bool) {
    let device = wide(&format!("{}:", char::from(letter)));
    let mut buffer = vec![0u16; 300];
    loop {
        let mut length = buffer.len() as u32;
        // SAFETY: a NUL-terminated device name; `buffer` is writable for `length` units.
        let status = unsafe {
            WNetGetConnectionW(
                PCWSTR(device.as_ptr()),
                Some(PWSTR(buffer.as_mut_ptr())),
                &mut length,
            )
        };
        match status {
            NO_ERROR => return (text(&buffer), true),
            ERROR_CONNECTION_UNAVAIL => return (text(&buffer), false),
            ERROR_MORE_DATA if length as usize > buffer.len() => {
                buffer = vec![0; length as usize];
            }
            _ => return (None, true),
        }
    }
}

/// The volume's size and the space free on it for this user.
pub(crate) fn space(root: &[u16]) -> Option<Space> {
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: a NUL-terminated root; two valid out pointers, the third not wanted.
    unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(root.as_ptr()),
            Some(&mut free),
            Some(&mut total),
            None,
        )
    }
    .ok()?;
    Some(Space {
        total: total as f64,
        free: free as f64,
    })
}

/// Whether a local disk is on a bus it can be unplugged from (USB, FireWire, SD, MMC): a USB disk,
/// a card. Explorer offers Eject for these. Hot-plug flags alone would also take in virtual
/// drives (Google Drive's reports them, and Explorer has no Eject for it). Asks the volume's
/// device; the handle is closed right away.
fn on_external_bus(letter: u8) -> bool {
    let device = wide(&format!(r"\.\{}:", char::from(letter)));
    // SAFETY: a NUL-terminated device path; no access asked, so nothing is read or locked.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(device.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    };
    let Ok(handle) = handle else {
        return false;
    };
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        ..Default::default()
    };
    // The descriptor and the strings after it; aligned for the descriptor.
    let mut answer = [0u64; 128];
    let mut returned = 0u32;
    // SAFETY: an open device handle; the query is read and the answer written within their sizes.
    let asked = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some((&raw const query).cast()),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(answer.as_mut_ptr().cast()),
            size_of_val(&answer) as u32,
            Some(&mut returned),
            None,
        )
    };
    // SAFETY: the handle came from CreateFileW above and is closed only here.
    let _ = unsafe { CloseHandle(handle) };
    let has_bus = returned as usize >= offset_of!(STORAGE_DEVICE_DESCRIPTOR, BusType) + 4;
    if asked.is_err() || !has_bus {
        return false;
    }
    // SAFETY: the buffer is aligned and large enough for the descriptor, and Windows wrote at
    // least up to its bus type; the fields after it aren't read.
    let bus = unsafe { (*answer.as_ptr().cast::<STORAGE_DEVICE_DESCRIPTOR>()).BusType };
    [BusTypeUsb, BusType1394, BusTypeSd, BusTypeMmc].contains(&bus)
}

/// No "There is no disk in the drive" box from Windows on this thread: a drive without media
/// just fails.
pub(crate) fn quiet_errors() {
    // SAFETY: sets this thread's own error mode; the old one isn't wanted.
    let _ = unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX, None) };
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// The text in a NUL-terminated buffer; None when empty.
fn text(buffer: &[u16]) -> Option<String> {
    let len = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    (len > 0).then(|| String::from_utf16_lossy(&buffer[..len]))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    /// Drives that tests change while the monitor runs. A drive in `blocked` doesn't finish its
    /// probe until the test sends on its channel.
    #[derive(Default)]
    struct Fake {
        drives: Mutex<Vec<Scanned>>,
        probes: Mutex<HashMap<u8, Probed>>,
        blocked: Mutex<HashMap<u8, mpsc::Receiver<()>>>,
        probed: Mutex<Vec<u8>>,
    }

    impl Source for Arc<Fake> {
        fn scan(&self) -> Vec<Scanned> {
            lock(&self.drives).clone()
        }

        fn probe(&self, drive: &Scanned, _before: Option<&Probed>) -> Probed {
            lock(&self.probed).push(drive.letter);
            let gate = lock(&self.blocked).remove(&drive.letter);
            if let Some(gate) = gate {
                let _ = gate.recv_timeout(Duration::from_secs(10));
            }
            lock(&self.probes)
                .get(&drive.letter)
                .cloned()
                .unwrap_or_default()
        }
    }

    fn local(letter: u8, kind: DriveKind) -> Scanned {
        Scanned {
            letter,
            kind,
            remote: None,
            connected: true,
        }
    }

    fn ready(label: &str, free: f64) -> Probed {
        Probed {
            ready: true,
            label: Some(label.into()),
            space: Some(Space { total: 100.0, free }),
            ejectable: false,
        }
    }

    const FAST: Timing = Timing {
        scan: Duration::from_millis(30),
        probe: Duration::from_secs(60),
        burst: Duration::from_millis(10),
    };

    fn start(fake: &Arc<Fake>) -> (DriveMonitor, mpsc::Receiver<DriveList>) {
        let (sender, lists) = mpsc::channel();
        let sender = Mutex::new(sender);
        let monitor = DriveMonitor::with_timing(
            fake.clone(),
            move |list| {
                let _ = lock(&sender).send(list.clone());
            },
            FAST,
        );
        (monitor, lists)
    }

    /// The next list for which `want` holds.
    fn until(lists: &mpsc::Receiver<DriveList>, want: impl Fn(&DriveList) -> bool) -> DriveList {
        loop {
            let list = lists
                .recv_timeout(Duration::from_secs(5))
                .expect("a drive list in time");
            if want(&list) {
                return list;
            }
        }
    }

    fn letters(list: &DriveList) -> String {
        list.drives
            .iter()
            .map(|drive| drive.letter.as_str())
            .collect()
    }

    #[test]
    fn drives_come_and_go_and_lists_are_numbered() {
        let fake = Arc::new(Fake::default());
        *lock(&fake.drives) = vec![local(b'C', DriveKind::Fixed)];
        lock(&fake.probes).insert(b'C', ready("Windows", 40.0));
        let (monitor, lists) = start(&fake);

        let first = until(&lists, |list| {
            list.drives.first().is_some_and(|c| c.space.is_some())
        });
        assert_eq!(letters(&first), "C");
        assert_eq!(first.drives[0].label.as_deref(), Some("Windows"));
        assert_eq!(monitor.drives(), first);

        lock(&fake.probes).insert(b'E', ready("USB", 2.0));
        lock(&fake.drives).push(local(b'E', DriveKind::Removable));
        monitor.refresh();
        let plugged = until(&lists, |list| letters(list) == "CE");
        assert!(plugged.generation > first.generation);

        lock(&fake.drives).retain(|drive| drive.letter != b'E');
        let unplugged = until(&lists, |list| letters(list) == "C");
        assert!(unplugged.generation > plugged.generation);
    }

    #[test]
    fn a_slow_drive_holds_up_nothing_else() {
        let fake = Arc::new(Fake::default());
        let (open, gate) = mpsc::channel();
        lock(&fake.blocked).insert(b'F', gate);
        *lock(&fake.drives) = vec![
            local(b'C', DriveKind::Fixed),
            local(b'F', DriveKind::Optical),
        ];
        lock(&fake.probes).insert(b'C', ready("Windows", 40.0));
        lock(&fake.probes).insert(b'F', ready("Disc", 0.0));
        let (monitor, lists) = start(&fake);

        // F's probe hangs: C is read and reported anyway, F shows without a label meanwhile.
        let waiting = until(&lists, |list| {
            list.drives.first().is_some_and(|c| c.space.is_some())
        });
        assert_eq!(letters(&waiting), "CF");
        assert_eq!(waiting.drives[1].label, None);
        // And the monitor keeps scanning: a new drive shows while F still hangs.
        lock(&fake.drives).push(local(b'D', DriveKind::Fixed));
        until(&lists, |list| letters(list) == "CFD");

        let _ = open.send(());
        let done = until(&lists, |list| {
            list.drives
                .iter()
                .any(|drive| drive.label.as_deref() == Some("Disc"))
        });
        assert_eq!(letters(&done), "CFD");
        // F was probed once: a probe still running isn't started again.
        assert_eq!(lock(&fake.probed).iter().filter(|&&l| l == b'F').count(), 1);
        drop(monitor);
    }

    #[test]
    fn network_drives_are_never_probed_and_empty_card_readers_hide() {
        let fake = Arc::new(Fake::default());
        *lock(&fake.drives) = vec![
            local(b'C', DriveKind::Fixed),
            local(b'R', DriveKind::Removable),
            Scanned {
                letter: b'Z',
                kind: DriveKind::Network,
                remote: Some(r"\\INSTA-NAS01\Projects".into()),
                connected: false,
            },
        ];
        // R has no media: its probe isn't ready.
        lock(&fake.probes).insert(b'C', ready("Windows", 40.0));
        let (monitor, lists) = start(&fake);

        let list = until(&lists, |list| {
            list.drives.first().is_some_and(|c| c.space.is_some())
        });
        assert_eq!(letters(&list), "CZ");
        let z = &list.drives[1];
        assert_eq!(z.label.as_deref(), Some("Projects"));
        assert!(!z.connected);
        assert!(!lock(&fake.probed).contains(&b'Z'));
        drop(monitor);
    }

    #[test]
    fn unchanged_scans_report_nothing_and_refresh_reads_again() {
        let fake = Arc::new(Fake::default());
        *lock(&fake.drives) = vec![local(b'C', DriveKind::Fixed)];
        lock(&fake.probes).insert(b'C', ready("Windows", 40.0));
        let (monitor, lists) = start(&fake);
        let first = until(&lists, |list| {
            list.drives.first().is_some_and(|c| c.space.is_some())
        });

        // Several scans later, nothing new was reported.
        std::thread::sleep(FAST.scan * 5);
        assert!(lists.try_recv().is_err());

        // Free space changed: a refresh reads the drive again before `PROBE_EVERY`.
        lock(&fake.probes).insert(b'C', ready("Windows", 30.0));
        monitor.refresh();
        let after = until(&lists, |list| {
            list.drives[0].space.map(|space| space.free) == Some(30.0)
        });
        assert_eq!(after.generation, first.generation + 1);
    }

    #[test]
    fn share_and_server_names() {
        assert_eq!(share_name(r"\\nas\Projects").as_deref(), Some("Projects"));
        assert_eq!(share_name(r"\\nas\Projects\2026\").as_deref(), Some("2026"));
        assert_eq!(share_name(r"\\nas"), None);
        assert_eq!(share_name("C:\\"), None);
        assert_eq!(
            server_name(r"\\INSTA-NAS01\Projects").as_deref(),
            Some("INSTA-NAS01")
        );
        assert_eq!(server_name(r"C:\x"), None);
    }

    #[test]
    fn network_places_are_shares_in_the_long_form() {
        let unc = |text: &str| network_place(text).map(|path| path.display().to_string());
        let projects = Ok(r"\\?\UNC\INSTA-NAS01\Projects".to_owned());
        assert_eq!(unc(r"\\INSTA-NAS01\Projects"), projects);
        assert_eq!(unc(r#"  "\\INSTA-NAS01\Projects\" "#), projects);
        assert_eq!(unc("//INSTA-NAS01/Projects"), projects);
        assert_eq!(unc(r"\\?\UNC\INSTA-NAS01\Projects"), projects);
        assert_eq!(
            unc(r"\\nas\პროექტები\2026"),
            Ok(r"\\?\UNC\nas\პროექტები\2026".to_owned())
        );
        for bad in [
            "",
            r"\\nas",
            r"\\nas\",
            r"\\\share",
            r"C:\Projects",
            r"\\.\C:",
            r"\\nas\share\..",
            r"\\nas\sha*re",
            "Projects",
        ] {
            assert_eq!(
                network_place(bad).map_err(|error| error.code),
                Err(ErrorCode::InvalidName),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn this_computer_has_its_system_drive() {
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let letter = system.as_bytes()[0].to_ascii_uppercase();
        let drive = System
            .scan()
            .into_iter()
            .find(|drive| drive.letter == letter)
            .expect("the system drive is listed");
        assert_eq!(drive.kind, DriveKind::Fixed);
        let probed = System.probe(&drive, None);
        assert!(probed.ready);
        assert!(!probed.ejectable, "an internal disk has no Eject");
        let space = probed.space.expect("the system drive has a size");
        assert!(space.total > 0.0 && space.free > 0.0 && space.free <= space.total);
    }
}
