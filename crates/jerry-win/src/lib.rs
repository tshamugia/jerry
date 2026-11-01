//! Jerry's Windows integration: icons, thumbnails, Recycle Bin, clipboard, drag & drop,
//! context menu, OneDrive, drives, registry and toast.
//!
//! This is the only crate that touches Shell, COM or OLE. OLE, clipboard, drag & drop and the
//! native context menu run on the main STA thread; other Shell calls run on the Shell pool with
//! timeouts.

pub mod process;

use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::core::w;

/// First Windows 11 build. Snap Layouts and Mica need it.
pub const WINDOWS_11_BUILD: u32 = 22_000;

/// Whether Windows' app mode is dark ("Choose your app mode" in Settings → Colors), read from
/// HKCU so a window can get its theme background before any web content loads.
pub fn apps_use_dark_theme() -> bool {
    let mut value = 1u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `value` is a writable u32 and `size` holds its size, as RRF_RT_REG_DWORD needs.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    status.is_ok() && value == 0
}

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
