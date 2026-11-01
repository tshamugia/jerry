//! Jerry's Windows integration: icons, thumbnails, Recycle Bin, clipboard, drag & drop,
//! context menu, OneDrive, drives, registry and toast.
//!
//! This is the only crate that touches Shell, COM or OLE. OLE, clipboard, drag & drop and the
//! native context menu run on the main STA thread; other Shell calls run on the Shell pool with
//! timeouts.

use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

/// First Windows 11 build. Snap Layouts and Mica need it.
pub const WINDOWS_11_BUILD: u32 = 22_000;

/// The Windows build number (for example 26100), read with `RtlGetVersion`, which unlike
/// `GetVersionExW` doesn't depend on the calling executable's compatibility manifest.
/// Returns `None` if the call fails.
pub fn os_build() -> Option<u32> {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid, writable OSVERSIONINFOW with its size field set, as required.
    let status = unsafe { RtlGetVersion(&mut info) };
    status.is_ok().then_some(info.dwBuildNumber)
}

/// Opts the calling thread out of execution-speed throttling (EcoQoS), so Windows keeps it on fast
/// cores at full clock even while Jerry's window is in the background. Spike #12 measures whether
/// the IO and CPU pools need this.
pub fn high_qos_thread() -> bool {
    use windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadInformation, THREAD_POWER_THROTTLING_CURRENT_VERSION,
        THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
        ThreadPowerThrottling,
    };
    let state = THREAD_POWER_THROTTLING_STATE {
        Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    // SAFETY: `state` is a valid THREAD_POWER_THROTTLING_STATE and the size passed matches it.
    unsafe {
        SetThreadInformation(
            GetCurrentThread(),
            ThreadPowerThrottling,
            (&raw const state).cast(),
            size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_build_is_windows_10_22h2_or_later() {
        // Windows 10 22H2 is build 19045; CI runners are newer.
        let build = os_build().expect("RtlGetVersion failed");
        assert!(build >= 19_045, "unexpected build {build}");
    }
}
