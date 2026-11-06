//! Windows' device notifications, received on Jerry's own message-only window (ADR-0002,
//! [`main_thread`](crate::main_thread)).
//!
//! Now: a volume Jerry holds folders open on is about to be removed or locked (#126). Each such
//! volume gets a handle registration ([`watch_volume`]: `RegisterDeviceNotificationW` with
//! `DBT_DEVTYP_HANDLE`). Windows asks before it removes the volume (`DBT_DEVICEQUERYREMOVE`,
//! "Safely remove") or locks it (`GUID_IO_VOLUME_LOCK`: Explorer's Eject, chkdsk), and goes ahead
//! only if nothing on it is open once the window procedure returns. Then it says how that went.
//!
//! Drive arrival and removal (#34) come through the same window procedure: one device-interface
//! registration for every volume (`GUID_DEVINTERFACE_VOLUME`), made when the thread starts, tells
//! [`on_drives_change`]'s callback that a volume came or went. A message-only window gets no
//! broadcasts, so a mapped network drive brings no notice: the drive monitor scans for those.
//!
//! Registering and unregistering run on a thread of their own ([`start`]), never on the main
//! thread and never waited for by it: Windows delivers a notification from a thread of its own
//! that waits for the window procedure, and unregistering waits for a notification in flight.

use std::collections::{HashMap, VecDeque};
use std::mem::offset_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use jerry_core::JerryError;
use jerry_core::job::run_job;
use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, LRESULT, WPARAM};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::GUID_DEVINTERFACE_VOLUME;
use windows::Win32::UI::WindowsAndMessaging::{
    DBT_CUSTOMEVENT, DBT_DEVICEARRIVAL, DBT_DEVICEQUERYREMOVE, DBT_DEVICEQUERYREMOVEFAILED,
    DBT_DEVICEREMOVECOMPLETE, DBT_DEVICEREMOVEPENDING, DBT_DEVTYP_DEVICEINTERFACE,
    DBT_DEVTYP_HANDLE, DEV_BROADCAST_DEVICEINTERFACE_W, DEV_BROADCAST_HANDLE, DEV_BROADCAST_HDR,
    DEVICE_NOTIFY_WINDOW_HANDLE, GUID_IO_VOLUME_LOCK, GUID_IO_VOLUME_LOCK_FAILED,
    GUID_IO_VOLUME_UNLOCK, HDEVNOTIFY, RegisterDeviceNotificationW, UnregisterDeviceNotification,
};
use windows::core::{GUID, PCWSTR};

/// What Windows says about a volume (see [`watch_volume`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeEvent {
    /// Windows wants to remove or lock the volume: close everything open on it before the
    /// callback returns.
    Release,
    /// The removal or lock failed, or the lock is over: the volume stays.
    Keep,
    /// The volume is gone.
    Removed,
}

/// Windows' notices about one volume. Dropping it stops them.
pub struct VolumeNotices {
    id: u64,
}

/// Has Windows tell `on_event` what happens to the volume at `root` (a volume root, as
/// `jerry_core::volume::root` gives it). `on_event` runs on the main thread, inside the window
/// procedure: for [`VolumeEvent::Release`], whatever is open on the volume must be closed when
/// it returns, so it waits for that, and only for that.
///
/// Registers in the background and doesn't fail: notices start once
/// [`main_thread::init`](crate::main_thread::init) has run and Windows has the registration. A
/// root that can't be registered (a drive that is gone) is logged and gets none.
pub fn watch_volume(
    root: &Path,
    on_event: impl Fn(VolumeEvent) + Send + Sync + 'static,
) -> VolumeNotices {
    let mut state = state();
    state.next_id += 1;
    let id = state.next_id;
    state.registrations.insert(
        id,
        Registration {
            root: root.to_path_buf(),
            notify: None,
            on_event: Arc::new(on_event),
        },
    );
    state.work.push_back(Work::Register(id));
    WORK_READY.notify_one();
    VolumeNotices { id }
}

impl Drop for VolumeNotices {
    fn drop(&mut self) {
        let mut state = state();
        if let Some(Registration {
            notify: Some(notify),
            ..
        }) = state.registrations.remove(&self.id)
        {
            state.work.push_back(Work::Unregister(notify));
            WORK_READY.notify_one();
        }
    }
}

/// Has Windows tell `on_change` whenever a volume arrives or goes (a USB drive plugged in or
/// pulled out), on the main thread, inside the window procedure: it must return at once. One
/// callback for the process; a second call replaces the first.
pub fn on_drives_change(on_change: impl Fn() + Send + Sync + 'static) {
    *DRIVES_CHANGED
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(on_change));
}

static DRIVES_CHANGED: Mutex<Option<Arc<dyn Fn() + Send + Sync>>> = Mutex::new(None);

type OnEvent = Arc<dyn Fn(VolumeEvent) + Send + Sync>;

struct Registration {
    root: PathBuf,
    /// The `HDEVNOTIFY` once Windows has it, as an integer so it can cross threads.
    notify: Option<isize>,
    on_event: OnEvent,
}

enum Work {
    Register(u64),
    Unregister(isize),
}

#[derive(Default)]
struct State {
    next_id: u64,
    registrations: HashMap<u64, Registration>,
    /// For the registering thread, in order: a registration is never unregistered before it is
    /// registered.
    work: VecDeque<Work>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Mutex::default);
static WORK_READY: Condvar = Condvar::new();

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts the thread that registers with Windows for `window` (Jerry's message-only window, as
/// an integer). Called once, by `main_thread::init`; work queued before that waits for it.
pub(crate) fn start(window: isize) {
    let spawned = std::thread::Builder::new()
        .name("device-notices".into())
        .spawn(move || {
            // Kept for the life of the process.
            let _ = run_job("volume arrival notices", || {
                register_volumes(window)
                    .inspect_err(|error| {
                        tracing::warn!(code = ?error.code, "no notices of drives plugged in")
                    })
                    .map(|_| ())
            });
            loop {
                let work = {
                    let mut state = state();
                    loop {
                        if let Some(work) = state.work.pop_front() {
                            break work;
                        }
                        state = WORK_READY
                            .wait(state)
                            .unwrap_or_else(PoisonError::into_inner);
                    }
                };
                // A panic fails only this registration.
                let _ = run_job("device notices", || {
                    do_work(window, work);
                    Ok(())
                });
            }
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "the device-notice thread didn't start: no volume notices");
    }
}

fn do_work(window: isize, work: Work) {
    match work {
        Work::Register(id) => {
            let Some(root) = state().registrations.get(&id).map(|r| r.root.clone()) else {
                return; // dropped meanwhile
            };
            let notify = match register(window, &root) {
                Ok(notify) => notify,
                Err(error) => {
                    tracing::warn!(code = ?error.code, "volume notices not registered");
                    return;
                }
            };
            let mut state = state();
            match state.registrations.get_mut(&id) {
                Some(registration) => registration.notify = Some(notify),
                None => {
                    drop(state);
                    unregister(notify);
                }
            }
        }
        Work::Unregister(notify) => unregister(notify),
    }
}

/// Registers `window` for Windows' notices about the volume at `root`. The registration is the
/// volume's, not the handle's: the handle is closed right away, so it holds nothing open.
fn register(window: isize, root: &Path) -> Result<isize, JerryError> {
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call; no security attributes, no
    // template. Every share mode and no data access: it only names the volume.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }?;
    let filter = DEV_BROADCAST_HANDLE {
        dbch_size: size_of::<DEV_BROADCAST_HANDLE>() as u32,
        dbch_devicetype: DBT_DEVTYP_HANDLE.0,
        dbch_handle: handle,
        ..Default::default()
    };
    // SAFETY: `filter` is a valid DEV_BROADCAST_HANDLE for the call; `window` is Jerry's own
    // window, which lives as long as the process.
    let notify = unsafe {
        RegisterDeviceNotificationW(
            HANDLE(window as *mut core::ffi::c_void),
            (&raw const filter).cast(),
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    };
    // SAFETY: the handle came from CreateFileW above and is closed only here.
    if let Err(error) = unsafe { CloseHandle(handle) } {
        tracing::warn!(%error, "volume root didn't close");
    }
    Ok(notify?.0 as isize)
}

/// Registers `window` for the arrival and removal of every volume.
fn register_volumes(window: isize) -> Result<isize, JerryError> {
    let filter = DEV_BROADCAST_DEVICEINTERFACE_W {
        dbcc_size: size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
        dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
        dbcc_classguid: GUID_DEVINTERFACE_VOLUME,
        ..Default::default()
    };
    // SAFETY: `filter` is a valid DEV_BROADCAST_DEVICEINTERFACE_W for the call; `window` is
    // Jerry's own window, which lives as long as the process.
    let notify = unsafe {
        RegisterDeviceNotificationW(
            HANDLE(window as *mut core::ffi::c_void),
            (&raw const filter).cast(),
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    }?;
    Ok(notify.0 as isize)
}

fn unregister(notify: isize) {
    // SAFETY: a registration from RegisterDeviceNotificationW, unregistered only once.
    let result = unsafe { UnregisterDeviceNotification(HDEVNOTIFY(notify as *mut _)) };
    if let Err(error) = result {
        tracing::warn!(%error, "volume notices didn't unregister");
    }
}

/// `WM_DEVICECHANGE` on Jerry's window: hands a volume's notice to its [`watch_volume`] callback
/// and returns `TRUE`. Jerry never refuses a removal: it lets go of the volume instead.
pub(crate) fn on_device_change(wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: as below.
    if unsafe { is_volume_arrival_or_removal(wparam, lparam) } {
        let on_change = DRIVES_CHANGED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(on_change) = on_change {
            let _ = run_job("drives changed", || {
                on_change();
                Ok(())
            });
        }
        return LRESULT(1);
    }
    // SAFETY: Windows sends WM_DEVICECHANGE with an LPARAM that is null or points to a
    // DEV_BROADCAST_HDR, alive for the call.
    if let Some((notify, event)) = unsafe { volume_event(wparam, lparam) } {
        let on_event = state()
            .registrations
            .values()
            .find(|registration| registration.notify == Some(notify))
            .map(|registration| registration.on_event.clone());
        if let Some(on_event) = on_event {
            let started = Instant::now();
            // Unwinding out of a window procedure would abort the process.
            let _ = run_job("volume notice", || {
                on_event(event);
                Ok(())
            });
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            tracing::info!(?event, ms, "volume notice handled");
        }
    }
    LRESULT(1)
}

/// Whether a `WM_DEVICECHANGE` says that a volume arrived or went ([`register_volumes`]).
///
/// # Safety
/// As for [`volume_event`].
unsafe fn is_volume_arrival_or_removal(wparam: WPARAM, lparam: LPARAM) -> bool {
    let header = lparam.0 as *const DEV_BROADCAST_HDR;
    let event = wparam.0 as u32;
    if header.is_null() || (event != DBT_DEVICEARRIVAL && event != DBT_DEVICEREMOVECOMPLETE) {
        return false;
    }
    // SAFETY: not null, so a DEV_BROADCAST_HDR (the caller's promise).
    let (size, kind) = unsafe { ((*header).dbch_size as usize, (*header).dbch_devicetype) };
    if kind != DBT_DEVTYP_DEVICEINTERFACE
        || size < offset_of!(DEV_BROADCAST_DEVICEINTERFACE_W, dbcc_name)
    {
        return false;
    }
    // SAFETY: a device-interface notice at least up to its name, so its class is in it.
    let class = unsafe { (*header.cast::<DEV_BROADCAST_DEVICEINTERFACE_W>()).dbcc_classguid };
    class == GUID_DEVINTERFACE_VOLUME
}

/// The volume notice in a `WM_DEVICECHANGE`: the registration it is for, and what it says.
///
/// # Safety
/// `lparam` is null or points to a `DEV_BROADCAST_HDR` at least `dbch_size` bytes long, as
/// Windows sends it.
unsafe fn volume_event(wparam: WPARAM, lparam: LPARAM) -> Option<(isize, VolumeEvent)> {
    let header = lparam.0 as *const DEV_BROADCAST_HDR;
    if header.is_null() {
        return None;
    }
    // SAFETY: not null, so a DEV_BROADCAST_HDR (the caller's promise).
    let (size, kind) = unsafe { ((*header).dbch_size as usize, (*header).dbch_devicetype) };
    if kind != DBT_DEVTYP_HANDLE || size < offset_of!(DEV_BROADCAST_HANDLE, dbch_eventguid) {
        return None;
    }
    let handle = header.cast::<DEV_BROADCAST_HANDLE>();
    // SAFETY: a handle notice at least up to its event GUID, so its registration is in it.
    let notify = unsafe { (*handle).dbch_hdevnotify }.0 as isize;
    let event = match wparam.0 as u32 {
        DBT_DEVICEQUERYREMOVE => VolumeEvent::Release,
        DBT_DEVICEQUERYREMOVEFAILED => VolumeEvent::Keep,
        DBT_DEVICEREMOVEPENDING | DBT_DEVICEREMOVECOMPLETE => VolumeEvent::Removed,
        DBT_CUSTOMEVENT if size >= offset_of!(DEV_BROADCAST_HANDLE, dbch_nameoffset) => {
            // SAFETY: a custom handle notice, long enough to hold its event GUID.
            lock_event(unsafe { (*handle).dbch_eventguid })?
        }
        _ => return None,
    };
    Some((notify, event))
}

/// What a custom volume event means for what Jerry holds open: a lock wants it closed.
fn lock_event(guid: GUID) -> Option<VolumeEvent> {
    if guid == GUID_IO_VOLUME_LOCK {
        Some(VolumeEvent::Release)
    } else if guid == GUID_IO_VOLUME_LOCK_FAILED || guid == GUID_IO_VOLUME_UNLOCK {
        Some(VolumeEvent::Keep)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use windows::Win32::UI::WindowsAndMessaging::DBT_DEVNODES_CHANGED;

    use super::*;

    /// A handle notice for registration `notify`, as Windows sends it.
    fn notice(notify: isize, guid: GUID) -> DEV_BROADCAST_HANDLE {
        DEV_BROADCAST_HANDLE {
            dbch_size: size_of::<DEV_BROADCAST_HANDLE>() as u32,
            dbch_devicetype: DBT_DEVTYP_HANDLE.0,
            dbch_hdevnotify: HDEVNOTIFY(notify as *mut _),
            dbch_eventguid: guid,
            ..Default::default()
        }
    }

    fn send(event: u32, notice: Option<&DEV_BROADCAST_HANDLE>) -> LRESULT {
        let lparam = notice.map_or(0, |notice| std::ptr::from_ref(notice) as isize);
        on_device_change(WPARAM(event as usize), LPARAM(lparam))
    }

    #[test]
    fn notices_reach_the_callback_of_their_registration() {
        let (sender, told) = mpsc::channel();
        // A path nobody can open: the registering thread, if one runs, leaves it alone.
        let missing = Path::new(r"\\?\C:\jerry-no-such-folder-7e570126\");
        let notices = watch_volume(missing, move |event| {
            let _ = sender.send(event);
        });
        let fake = 0x7e57_0126;
        state()
            .registrations
            .get_mut(&notices.id)
            .expect("registered")
            .notify = Some(fake);

        let none = GUID::zeroed();
        assert_eq!(send(DBT_DEVICEQUERYREMOVE, Some(&notice(fake, none))).0, 1);
        send(DBT_DEVICEQUERYREMOVEFAILED, Some(&notice(fake, none)));
        send(DBT_CUSTOMEVENT, Some(&notice(fake, GUID_IO_VOLUME_LOCK)));
        send(DBT_CUSTOMEVENT, Some(&notice(fake, GUID_IO_VOLUME_UNLOCK)));
        send(
            DBT_CUSTOMEVENT,
            Some(&notice(fake, GUID_IO_VOLUME_LOCK_FAILED)),
        );
        send(DBT_DEVICEREMOVECOMPLETE, Some(&notice(fake, none)));
        // Not for this registration, not about a volume handle, or not a volume event.
        send(DBT_DEVICEQUERYREMOVE, Some(&notice(fake + 1, none)));
        send(DBT_CUSTOMEVENT, Some(&notice(fake, GUID::from_u128(1))));
        assert_eq!(send(DBT_DEVNODES_CHANGED, None).0, 1);
        let mut short = notice(fake, none);
        short.dbch_size = 12;
        send(DBT_DEVICEQUERYREMOVE, Some(&short));

        let told: Vec<VolumeEvent> = told.try_iter().collect();
        assert_eq!(
            told,
            [
                VolumeEvent::Release,
                VolumeEvent::Keep,
                VolumeEvent::Release,
                VolumeEvent::Keep,
                VolumeEvent::Keep,
                VolumeEvent::Removed
            ]
        );

        // Dropped: nothing more reaches it.
        drop(notices);
        assert!(
            !state()
                .registrations
                .values()
                .any(|r| r.notify == Some(fake))
        );
    }

    #[test]
    fn volumes_coming_and_going_reach_the_drives_callback() {
        let (sender, told) = mpsc::channel();
        let sender = Mutex::new(sender);
        on_drives_change(move || {
            let _ = sender.lock().map(|sender| sender.send(()));
        });
        let notice = |class: GUID| DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
            dbcc_classguid: class,
            ..Default::default()
        };
        let send = |event: u32, notice: &DEV_BROADCAST_DEVICEINTERFACE_W| {
            on_device_change(
                WPARAM(event as usize),
                LPARAM(std::ptr::from_ref(notice) as isize),
            )
        };
        let volume = notice(GUID_DEVINTERFACE_VOLUME);
        assert_eq!(send(DBT_DEVICEARRIVAL, &volume).0, 1);
        send(DBT_DEVICEREMOVECOMPLETE, &volume);
        // Another device class, or a volume event that isn't an arrival or removal.
        send(DBT_DEVICEARRIVAL, &notice(GUID::from_u128(7)));
        send(DBT_DEVICEQUERYREMOVE, &volume);
        assert_eq!(told.try_iter().count(), 2);
    }

    #[test]
    fn the_volume_class_registers() {
        let window = crate::main_thread::create_window().expect("a message-only window");
        let notify = register_volumes(window.0 as isize).expect("Windows registers volumes");
        unregister(notify);
    }

    #[test]
    fn a_volume_registers_from_another_thread_and_holds_nothing_open() {
        let window = crate::main_thread::create_window().expect("a message-only window");
        let window = window.0 as isize;
        let dir = tempfile::tempdir().expect("temp folder");
        let held = dir.path().join("USB");
        std::fs::create_dir(&held).expect("mkdir");
        let path = jerry_core::path::long_path(&held).expect("long path");

        // Any folder names its volume. Not the window's thread, as in Jerry.
        let notify = std::thread::spawn(move || register(window, &path))
            .join()
            .expect("the registering thread")
            .expect("Windows registers the volume");
        // The handle it registered with is closed: nothing holds the folder.
        std::fs::rename(&held, dir.path().join("moved")).expect("nothing holds it");
        unregister(notify);
    }
}
